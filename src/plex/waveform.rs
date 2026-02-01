//! Waveform generation and caching service.
//!
//! Computes amplitude profiles from audio files for visualization.

use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Default number of amplitude bins.
pub const DEFAULT_BIN_COUNT: usize = 1000;

/// Waveform data version for cache invalidation.
pub const WAVEFORM_VERSION: u8 = 1;

/// Computed waveform data for a track.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveformData {
    /// Track rating key (unique identifier).
    pub track_key: String,
    /// Duration in milliseconds (for validation).
    pub duration_ms: u64,
    /// Normalized amplitude bins (0.0-1.0).
    pub bins: Vec<f32>,
    /// Version marker for cache invalidation.
    pub version: u8,
    /// Timestamp when this waveform was generated.
    #[serde(default)]
    pub created_at: u64,
}

impl WaveformData {
    /// Resample bins to fit a specific width.
    pub fn resample(&self, target_width: usize) -> Vec<f32> {
        if target_width == 0 || self.bins.is_empty() {
            return vec![0.0; target_width];
        }

        if target_width == self.bins.len() {
            return self.bins.clone();
        }

        let mut result = Vec::with_capacity(target_width);
        let bins_per_output = self.bins.len() as f32 / target_width as f32;

        for i in 0..target_width {
            let start = (i as f32 * bins_per_output) as usize;
            let end = ((i + 1) as f32 * bins_per_output) as usize;
            let end = end.min(self.bins.len());

            if start < end {
                // Take max amplitude in range (preserves peaks)
                let max_amp = self.bins[start..end]
                    .iter()
                    .fold(0.0f32, |acc, &x| acc.max(x));
                result.push(max_amp);
            } else {
                result.push(self.bins.get(start).copied().unwrap_or(0.0));
            }
        }
        result
    }
}

/// Errors that can occur during waveform generation.
#[derive(Debug, Error)]
pub enum WaveformError {
    #[error("Failed to probe audio format: {0}")]
    Probe(String),
    #[error("No audio track found")]
    NoTrack,
    #[error("Missing sample rate")]
    NoSampleRate,
    #[error("Decoder creation failed: {0}")]
    Decoder(String),
    #[error("Decode error: {0}")]
    Decode(String),
    #[error("Download failed: {0}")]
    Download(String),
    #[error("No samples decoded")]
    NoSamples,
}

/// Decode audio data to mono PCM f32 samples.
pub fn decode_to_pcm(audio_data: Vec<u8>) -> Result<(Vec<f32>, u32), WaveformError> {
    let byte_len = audio_data.len() as u64;
    let cursor = Cursor::new(audio_data);
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let hint = Hint::new();
    let format_opts = FormatOptions::default();
    let meta_opts = MetadataOptions::default();
    let decoder_opts = DecoderOptions::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &format_opts, &meta_opts)
        .map_err(|e| WaveformError::Probe(e.to_string()))?;

    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or(WaveformError::NoTrack)?;

    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or(WaveformError::NoSampleRate)?;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &decoder_opts)
        .map_err(|e| WaveformError::Decoder(e.to_string()))?;

    let track_id = track.id;
    let mut samples = Vec::new();

    // Reserve approximate space based on file size
    // Rough estimate: 1 byte ~= 0.5 samples for compressed audio
    samples.reserve((byte_len / 2) as usize);

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(symphonia::core::errors::Error::ResetRequired) => {
                // Handle reset by reinitializing decoder
                decoder.reset();
                continue;
            }
            Err(_) => break, // Other errors, stop gracefully
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(_) => continue, // Skip bad packets
        };

        let spec = *decoded.spec();
        let mut sample_buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        sample_buf.copy_interleaved_ref(decoded);

        // Mix to mono if stereo
        let channel_count = spec.channels.count();
        let buf_samples = sample_buf.samples();

        if channel_count == 1 {
            samples.extend_from_slice(buf_samples);
        } else {
            // Average channels to mono
            for chunk in buf_samples.chunks(channel_count) {
                let avg = chunk.iter().sum::<f32>() / channel_count as f32;
                samples.push(avg);
            }
        }
    }

    if samples.is_empty() {
        return Err(WaveformError::NoSamples);
    }

    Ok((samples, sample_rate))
}

/// Compute RMS amplitude bins from PCM samples.
pub fn compute_rms_bins(samples: &[f32], bin_count: usize) -> Vec<f32> {
    if samples.is_empty() || bin_count == 0 {
        return vec![0.0; bin_count];
    }

    let samples_per_bin = samples.len() / bin_count;
    if samples_per_bin == 0 {
        // More bins than samples - just take what we have
        let mut bins: Vec<f32> = samples.iter().map(|s| s.abs()).collect();
        bins.resize(bin_count, 0.0);
        return normalize_bins(bins);
    }

    let mut bins = Vec::with_capacity(bin_count);

    // Compute RMS for each bin
    for i in 0..bin_count {
        let start = i * samples_per_bin;
        let end = if i == bin_count - 1 {
            samples.len() // Last bin gets remaining samples
        } else {
            ((i + 1) * samples_per_bin).min(samples.len())
        };

        if start < end {
            let sum_squares: f32 = samples[start..end].iter().map(|&s| s * s).sum();
            let rms = (sum_squares / (end - start) as f32).sqrt();
            bins.push(rms);
        } else {
            bins.push(0.0);
        }
    }

    normalize_bins(bins)
}

/// Normalize bins to 0.0-1.0 range.
fn normalize_bins(mut bins: Vec<f32>) -> Vec<f32> {
    let max_val = bins.iter().fold(0.0f32, |acc, &x| acc.max(x));
    if max_val > 0.0 {
        for bin in &mut bins {
            *bin /= max_val;
        }
    }
    bins
}

/// Generate waveform data from audio bytes.
pub fn generate_waveform(
    track_key: String,
    duration_ms: u64,
    audio_data: Vec<u8>,
) -> Result<WaveformData, WaveformError> {
    let (samples, _sample_rate) = decode_to_pcm(audio_data)?;
    let bins = compute_rms_bins(&samples, DEFAULT_BIN_COUNT);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    Ok(WaveformData {
        track_key,
        duration_ms,
        bins,
        version: WAVEFORM_VERSION,
        created_at: now,
    })
}

/// Waveform cache for persisting waveform data.
pub struct WaveformCache {
    cache_dir: PathBuf,
}

impl WaveformCache {
    /// Create a new waveform cache.
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

    /// Load waveform from cache.
    pub fn load(&self, track_key: &str) -> Option<WaveformData> {
        let path = self.cache_path(track_key);
        if !path.exists() {
            return None;
        }

        match std::fs::read_to_string(&path) {
            Ok(contents) => {
                match serde_json::from_str::<WaveformData>(&contents) {
                    Ok(data) => {
                        // Validate version and track key
                        if data.version == WAVEFORM_VERSION && data.track_key == track_key {
                            Some(data)
                        } else {
                            // Stale cache, remove it
                            let _ = std::fs::remove_file(&path);
                            None
                        }
                    }
                    Err(_) => {
                        // Corrupted cache, remove it
                        let _ = std::fs::remove_file(&path);
                        None
                    }
                }
            }
            Err(_) => None,
        }
    }

    /// Save waveform to cache.
    pub fn save(&self, data: &WaveformData) -> bool {
        // Ensure directory exists
        if !self.cache_dir.exists() {
            if std::fs::create_dir_all(&self.cache_dir).is_err() {
                return false;
            }
        }

        let path = self.cache_path(&data.track_key);
        match serde_json::to_string(data) {
            Ok(contents) => {
                // Atomic write via temp file
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

    /// Clear all cached waveforms.
    pub fn clear(&self) {
        if self.cache_dir.exists() {
            let _ = std::fs::remove_dir_all(&self.cache_dir);
        }
    }

    /// Prune expired waveform cache entries.
    /// User doesn't replay songs often, so waveforms can expire faster.
    pub fn prune_expired(&self, ttl_secs: u64) {
        if !self.cache_dir.exists() {
            return;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let cutoff = now.saturating_sub(ttl_secs);

        if let Ok(entries) = std::fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() || path.extension().map_or(true, |e| e != "json") {
                    continue;
                }

                // Try to read the file and check created_at
                if let Ok(contents) = std::fs::read_to_string(&path) {
                    if let Ok(data) = serde_json::from_str::<WaveformData>(&contents) {
                        if data.created_at < cutoff {
                            tracing::debug!("Pruning expired waveform: {}", data.track_key);
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
            }
        }
    }

    /// Prune waveform cache to fit within size limit.
    /// Removes oldest entries first.
    pub fn prune_to_size(&self, max_bytes: u64) {
        if !self.cache_dir.exists() {
            return;
        }

        // Collect all cache entries with their size and age
        let mut entries: Vec<(PathBuf, u64, u64)> = Vec::new(); // (path, size, created_at)
        let mut total_size = 0u64;

        if let Ok(dir_entries) = std::fs::read_dir(&self.cache_dir) {
            for entry in dir_entries.flatten() {
                let path = entry.path();
                if !path.is_file() || path.extension().map_or(true, |e| e != "json") {
                    continue;
                }

                if let Ok(metadata) = entry.metadata() {
                    let size = metadata.len();
                    total_size += size;

                    // Try to read created_at from the file
                    let created_at = std::fs::read_to_string(&path)
                        .ok()
                        .and_then(|contents| serde_json::from_str::<WaveformData>(&contents).ok())
                        .map(|data| data.created_at)
                        .unwrap_or(0);

                    entries.push((path, size, created_at));
                }
            }
        }

        if total_size <= max_bytes {
            return;
        }

        // Sort by created_at ascending (oldest first)
        entries.sort_by_key(|(_, _, created_at)| *created_at);

        // Remove oldest until we're under the limit
        for (path, size, _) in entries {
            if total_size <= max_bytes {
                break;
            }
            tracing::debug!("Pruning waveform to fit size limit: {:?}", path);
            if std::fs::remove_file(&path).is_ok() {
                total_size = total_size.saturating_sub(size);
            }
        }
    }

    /// Get total waveform cache size in bytes.
    pub fn total_size(&self) -> u64 {
        if !self.cache_dir.exists() {
            return 0;
        }

        let mut total = 0u64;
        if let Ok(entries) = std::fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_file() {
                        total += metadata.len();
                    }
                }
            }
        }
        total
    }
}

impl Default for WaveformCache {
    fn default() -> Self {
        let cache_dir = get_waveform_cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp/textamp_waveforms"));

        Self { cache_dir }
    }
}

/// Get the waveform cache directory path.
///
/// Checks $XDG_CACHE_HOME first (on macOS and Linux), then falls back to platform defaults.
fn get_waveform_cache_dir() -> Option<PathBuf> {
    // Check XDG env var first (works on both macOS and Linux)
    if let Ok(xdg_cache) = std::env::var("XDG_CACHE_HOME") {
        return Some(PathBuf::from(xdg_cache).join("textamp/waveforms"));
    }

    // Fall back to platform default
    #[cfg(target_os = "linux")]
    {
        dirs::home_dir().map(|h| h.join(".cache/textamp/waveforms"))
    }

    #[cfg(target_os = "macos")]
    {
        dirs::cache_dir().map(|p| p.join("textamp/waveforms"))
    }

    #[cfg(target_os = "windows")]
    {
        dirs::cache_dir().map(|p| p.join("textamp/waveforms"))
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        dirs::cache_dir().map(|p| p.join("textamp/waveforms"))
    }
}
