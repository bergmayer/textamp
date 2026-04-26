//! Spectrogram generation and caching service.
//!
//! Computes FFT-based spectrograms from audio files for visualization.
//! Uses the same PCM decode pipeline as waveform generation.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

use super::waveform::{decode_to_pcm, WaveformError};

/// FFT window size (number of samples per frame). 2048 samples at 44.1 kHz
/// ≈ 46 ms — a standard music STFT frame size with frequency resolution
/// of ~21 Hz, fine enough to resolve musical pitch above ~A1.
const FFT_SIZE: usize = 2048;

/// Hop size between frames. 50% overlap (FFT_SIZE / 2) is the minimum
/// that gives perfect Hann-window reconstruction (COLA condition) and
/// ensures every input sample contributes meaningfully to the output —
/// without overlap, samples at frame edges are zeroed by the window
/// and effectively lost.
const HOP_SIZE: usize = FFT_SIZE / 2;

/// Number of frequency bins in output (log-scaled from FFT bins).
const NUM_BINS: usize = 96;

/// Spectrogram data version for cache invalidation.
/// Bumped to 6 when overlap + windowed normalisation + power averaging
/// + extended dB range were introduced (v5 caches are not reusable).
pub const SPECTROGRAM_VERSION: u8 = 6;

/// Computed spectrogram data for a track.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpectrogramData {
    /// Track rating key (unique identifier).
    pub track_key: String,
    /// Duration in milliseconds (for validation).
    pub duration_ms: u64,
    /// Number of frequency bins per frame.
    pub bins_per_frame: usize,
    /// Total number of time frames.
    pub frame_count: usize,
    /// Frames per second (sample_rate / HOP_SIZE).
    pub frames_per_second: f32,
    /// Sample rate of the source audio.
    pub sample_rate: u32,
    /// Flat array of quantized magnitude values: frame_count * bins_per_frame.
    /// Each value is 0-255 (u8 quantized from dB scale).
    #[serde(with = "base64_bytes")]
    pub frames: Vec<u8>,
    /// Version marker for cache invalidation.
    pub version: u8,
    /// Timestamp when this spectrogram was generated.
    #[serde(default)]
    pub created_at: u64,
}

impl SpectrogramData {
    /// Get the frame index for a given playback position in milliseconds.
    pub fn frame_at_position(&self, position_ms: u64) -> usize {
        if self.frame_count == 0 || self.duration_ms == 0 {
            return 0;
        }
        let progress = position_ms as f64 / self.duration_ms as f64;
        let frame = (progress * self.frame_count as f64) as usize;
        frame.min(self.frame_count.saturating_sub(1))
    }

    /// Get the spectrum (frequency bins) at a given frame index.
    pub fn spectrum_at(&self, frame: usize) -> &[u8] {
        let start = frame * self.bins_per_frame;
        let end = start + self.bins_per_frame;
        if end <= self.frames.len() {
            &self.frames[start..end]
        } else {
            &[]
        }
    }

    /// Resample frequency bins to a target width for a given frame.
    pub fn resample_spectrum(&self, frame: usize, target_width: usize) -> Vec<u8> {
        let spectrum = self.spectrum_at(frame);
        if spectrum.is_empty() || target_width == 0 {
            return vec![0; target_width];
        }
        if target_width == spectrum.len() {
            return spectrum.to_vec();
        }

        let mut result = Vec::with_capacity(target_width);
        let bins_per_output = spectrum.len() as f32 / target_width as f32;

        for i in 0..target_width {
            let start = (i as f32 * bins_per_output) as usize;
            let end = ((i + 1) as f32 * bins_per_output) as usize;
            let end = end.min(spectrum.len());

            if start < end {
                let max_val = spectrum[start..end].iter().copied().max().unwrap_or(0);
                result.push(max_val);
            } else {
                result.push(spectrum.get(start).copied().unwrap_or(0));
            }
        }
        result
    }
}

/// Serde helper for base64-encoding Vec<u8>.
mod base64_bytes {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        serializer.serialize_str(&encoded)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(&s)
            .map_err(serde::de::Error::custom)
    }
}

/// Pre-compute the mapping from linear FFT bins to log-spaced output bins.
/// Returns a Vec of (start_bin, end_bin) pairs for each output bin.
fn log_frequency_mapping(fft_size: usize, sample_rate: u32, num_bins: usize) -> Vec<(usize, usize)> {
    let max_freq_bin = fft_size / 2; // Nyquist
    let min_freq = 20.0f64; // 20 Hz
    let max_freq = (sample_rate as f64 / 2.0).min(20000.0); // Nyquist or 20kHz

    let log_min = min_freq.ln();
    let log_max = max_freq.ln();

    let mut mapping = Vec::with_capacity(num_bins);
    let bin_hz = sample_rate as f64 / fft_size as f64;

    for i in 0..num_bins {
        let freq_low = (log_min + (log_max - log_min) * i as f64 / num_bins as f64).exp();
        let freq_high = (log_min + (log_max - log_min) * (i + 1) as f64 / num_bins as f64).exp();

        let bin_low = (freq_low / bin_hz).floor() as usize;
        let bin_high = (freq_high / bin_hz).ceil() as usize;

        let bin_low = bin_low.max(1).min(max_freq_bin);
        let bin_high = bin_high.max(bin_low + 1).min(max_freq_bin);

        mapping.push((bin_low, bin_high));
    }

    mapping
}

/// Compute spectrogram from PCM samples.
///
/// Implements a standard short-time Fourier transform (STFT):
/// - 2048-sample Hann-windowed frames
/// - 50% overlap (HOP_SIZE = FFT_SIZE / 2) for COLA reconstruction
/// - Window-corrected magnitude normalisation (2 / sum(window)) so a
///   full-scale sine reads as 0 dBFS regardless of windowing
/// - Power averaging within each log-spaced output bin (preserves
///   broadband energy correctly; previous max-magnitude biased toward
///   isolated tones)
/// - dB scale with -80 dB floor and 80 dB dynamic range (typical for
///   music spectrograms; the old 35 dB range clipped the lowest 45 dB
///   into solid background)
/// - sqrt visual lift (gamma 0.5) so quieter spectral content is still
///   visible — gamma > 1.0 (the previous setting) pushed it BELOW the
///   colour ramp's perceptible threshold.
fn compute_spectrogram(samples: &[f32], sample_rate: u32) -> SpectrogramData {
    if samples.len() < FFT_SIZE {
        return SpectrogramData {
            track_key: String::new(),
            duration_ms: 0,
            bins_per_frame: NUM_BINS,
            frame_count: 0,
            frames_per_second: sample_rate as f32 / HOP_SIZE as f32,
            sample_rate,
            frames: Vec::new(),
            version: SPECTROGRAM_VERSION,
            created_at: 0,
        };
    }
    // With 50% overlap, the last frame must fit a full FFT window.
    let frame_count = (samples.len() - FFT_SIZE) / HOP_SIZE + 1;

    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);

    // Pre-compute periodic Hann window (the standard for STFT — using
    // FFT_SIZE in the denominator instead of FFT_SIZE-1 gives the
    // periodic version which is what FFT-based analysis assumes).
    let window: Vec<f32> = (0..FFT_SIZE)
        .map(|i| {
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / FFT_SIZE as f32).cos())
        })
        .collect();
    let window_sum: f32 = window.iter().sum();
    // 2 / sum(window) recovers the true peak amplitude of a sinusoid
    // through the FFT (the factor of 2 accounts for the negative-
    // frequency mirror image we discard).
    let amp_norm = 2.0 / window_sum;

    // Pre-compute log-frequency bin mapping
    let bin_mapping = log_frequency_mapping(FFT_SIZE, sample_rate, NUM_BINS);

    let mut frames = vec![0u8; frame_count * NUM_BINS];
    let mut fft_buffer = vec![Complex::new(0.0f32, 0.0f32); FFT_SIZE];

    // Visual contrast curve. < 1.0 LIFTS quiet bins so they're visible
    // alongside loud peaks; > 1.0 would do the opposite. 0.5 (sqrt)
    // is a common choice for monochrome spectrograms.
    const GAMMA: f32 = 0.5;
    // Wide musical dynamic range: -80 dBFS floor (room noise on a
    // 16-bit master) up to 0 dBFS (full scale). 80 dB total span.
    const DB_FLOOR: f32 = -80.0;
    const DB_RANGE: f32 = 80.0;

    for frame_idx in 0..frame_count {
        let offset = frame_idx * HOP_SIZE;

        // Apply window and fill FFT buffer (always exactly FFT_SIZE
        // samples now, guaranteed by the frame_count formula above).
        for i in 0..FFT_SIZE {
            fft_buffer[i] = Complex::new(samples[offset + i] * window[i], 0.0);
        }

        fft.process(&mut fft_buffer);

        // Map FFT bins to log-spaced output bins. Sum POWER (magnitude
        // squared) within each bin range, then convert back to RMS
        // amplitude — this preserves total energy correctly even when
        // a single output bin spans many FFT bins (high-frequency end).
        for (bin_idx, &(start, end)) in bin_mapping.iter().enumerate() {
            let mut sum_power: f32 = 0.0;
            let mut count: u32 = 0;
            for fft_bin in start..end {
                if fft_bin < fft_buffer.len() {
                    let mag = fft_buffer[fft_bin].norm() * amp_norm;
                    sum_power += mag * mag;
                    count += 1;
                }
            }
            let rms = if count > 0 { (sum_power / count as f32).sqrt() } else { 0.0 };
            let db = if rms > 1e-10 {
                20.0 * rms.log10()
            } else {
                DB_FLOOR - 1.0
            };
            let normalized = ((db - DB_FLOOR) / DB_RANGE).clamp(0.0, 1.0);
            let curved = normalized.powf(GAMMA);
            frames[frame_idx * NUM_BINS + bin_idx] = (curved * 255.0).round() as u8;
        }
    }

    let fps = sample_rate as f32 / HOP_SIZE as f32;

    SpectrogramData {
        track_key: String::new(),
        duration_ms: 0,
        bins_per_frame: NUM_BINS,
        frame_count,
        frames_per_second: fps,
        sample_rate,
        frames,
        version: SPECTROGRAM_VERSION,
        created_at: 0,
    }
}

/// Generate spectrogram data from audio bytes.
pub fn generate_spectrogram(
    track_key: String,
    duration_ms: u64,
    audio_data: Vec<u8>,
) -> Result<SpectrogramData, WaveformError> {
    let (samples, sample_rate) = decode_to_pcm(audio_data)?;
    let mut data = compute_spectrogram(&samples, sample_rate);

    data.track_key = track_key;
    data.duration_ms = duration_ms;
    data.created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    Ok(data)
}

/// Generate spectrogram data from already-decoded PCM samples.
pub fn generate_spectrogram_from_pcm(
    track_key: String,
    duration_ms: u64,
    samples: &[f32],
    sample_rate: u32,
) -> SpectrogramData {
    let mut data = compute_spectrogram(samples, sample_rate);

    data.track_key = track_key;
    data.duration_ms = duration_ms;
    data.created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    data
}

/// Spectrogram cache for persisting spectrogram data.
pub struct SpectrogramCache {
    cache_dir: PathBuf,
}

impl SpectrogramCache {
    /// Create a new spectrogram cache.
    pub fn new(cache_dir: PathBuf) -> Self {
        Self { cache_dir }
    }

    /// Get the cache file path for a track.
    fn cache_path(&self, track_key: &str) -> PathBuf {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        track_key.hash(&mut hasher);
        let hash = hasher.finish();

        self.cache_dir.join(format!("{:016x}.json", hash))
    }

    /// Load spectrogram from cache.
    pub fn load(&self, track_key: &str) -> Option<SpectrogramData> {
        let path = self.cache_path(track_key);
        if !path.exists() {
            return None;
        }

        match std::fs::read_to_string(&path) {
            Ok(contents) => {
                match serde_json::from_str::<SpectrogramData>(&contents) {
                    Ok(data) => {
                        if data.version == SPECTROGRAM_VERSION && data.track_key == track_key {
                            Some(data)
                        } else {
                            let _ = std::fs::remove_file(&path);
                            None
                        }
                    }
                    Err(_) => {
                        let _ = std::fs::remove_file(&path);
                        None
                    }
                }
            }
            Err(_) => None,
        }
    }

    /// Save spectrogram to cache.
    pub fn save(&self, data: &SpectrogramData) -> bool {
        if !self.cache_dir.exists() {
            if std::fs::create_dir_all(&self.cache_dir).is_err() {
                return false;
            }
        }

        let path = self.cache_path(&data.track_key);
        match serde_json::to_string(data) {
            Ok(contents) => {
                let temp_path = path.with_extension("json.tmp");
                if std::fs::write(&temp_path, &contents).is_ok() {
                    std::fs::rename(&temp_path, &path).is_ok()
                } else {
                    false
                }
            }
            Err(_) => false,
        }
    }
}
