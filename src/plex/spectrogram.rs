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

/// FFT window size (number of samples per frame).
const FFT_SIZE: usize = 2048;

/// Hop size between frames (non-overlapping for speed).
const HOP_SIZE: usize = 2048;

/// Number of frequency bins in output (log-scaled from FFT bins).
const NUM_BINS: usize = 64;

/// Spectrogram data version for cache invalidation.
pub const SPECTROGRAM_VERSION: u8 = 5;

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
fn compute_spectrogram(samples: &[f32], sample_rate: u32) -> SpectrogramData {
    let frame_count = samples.len() / HOP_SIZE;
    if frame_count == 0 {
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

    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);

    // Pre-compute Hann window
    let window: Vec<f32> = (0..FFT_SIZE)
        .map(|i| {
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (FFT_SIZE - 1) as f32).cos())
        })
        .collect();

    // Pre-compute log-frequency bin mapping
    let bin_mapping = log_frequency_mapping(FFT_SIZE, sample_rate, NUM_BINS);

    let mut frames = vec![0u8; frame_count * NUM_BINS];
    let mut fft_buffer = vec![Complex::new(0.0f32, 0.0f32); FFT_SIZE];

    // Gamma > 1.0 pushes quiet bins down for Plexamp-style variation.
    const GAMMA: f32 = 1.5;
    // Individual frequency bins in music peak around -15 dBFS (energy spread across
    // many bins), so range of 35 dB (-50 to -15) makes peaks reach full height.
    const DB_FLOOR: f32 = -50.0;
    const DB_RANGE: f32 = 35.0;

    for frame_idx in 0..frame_count {
        let offset = frame_idx * HOP_SIZE;
        let available = samples.len() - offset;
        let len = available.min(FFT_SIZE);

        // Apply window and fill FFT buffer
        for i in 0..FFT_SIZE {
            if i < len {
                fft_buffer[i] = Complex::new(samples[offset + i] * window[i], 0.0);
            } else {
                fft_buffer[i] = Complex::new(0.0, 0.0);
            }
        }

        fft.process(&mut fft_buffer);

        // Map FFT bins to log-spaced output bins
        // Normalization factor: divide by N/2 to get amplitude relative to full scale (0 dBFS)
        let norm_factor = 2.0 / FFT_SIZE as f32;
        for (bin_idx, &(start, end)) in bin_mapping.iter().enumerate() {
            let mut max_mag: f32 = 0.0;
            for fft_bin in start..end {
                if fft_bin < fft_buffer.len() {
                    let mag = fft_buffer[fft_bin].norm() * norm_factor;
                    max_mag = max_mag.max(mag);
                }
            }
            // Convert normalized magnitude to dBFS (0 dBFS = full scale)
            let db = if max_mag > 1e-10 {
                20.0 * max_mag.log10()
            } else {
                DB_FLOOR - 1.0
            };
            // Map dB to 0.0-1.0 range (DB_FLOOR to 0dB), then apply gamma
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
