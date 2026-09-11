//! Waveform generation and caching service.
//!
//! Computes amplitude profiles from audio files for visualization.

use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as DecodeError;
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
    /// Return one peak-preserving sample for a target display width.
    ///
    /// This is the allocation-free counterpart to [`Self::resample`], used by
    /// renderers that consume each output bin exactly once.
    pub fn resampled_peak_at(&self, target_width: usize, output_index: usize) -> f32 {
        if target_width == 0 || output_index >= target_width || self.bins.is_empty() {
            return 0.0;
        }

        if target_width == self.bins.len() {
            return self.bins.get(output_index).copied().unwrap_or(0.0);
        }

        let bins_per_output = self.bins.len() as f32 / target_width as f32;
        let start = (output_index as f32 * bins_per_output) as usize;
        let end = (((output_index + 1) as f32 * bins_per_output) as usize).min(self.bins.len());

        if start < end {
            self.bins[start..end]
                .iter()
                .fold(0.0f32, |acc, &value| acc.max(value))
        } else {
            self.bins.get(start).copied().unwrap_or(0.0)
        }
    }

    /// Resample bins to fit a specific width.
    pub fn resample(&self, target_width: usize) -> Vec<f32> {
        (0..target_width)
            .map(|index| self.resampled_peak_at(target_width, index))
            .collect()
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

/// Decode one packet at a time; callers retain summaries, never whole-track PCM.
/// Recoverable packet errors become timed gaps. Reject more than 5% missing
/// audio (allowing one second for short clips), and never accept padding alone.
pub(super) fn decode_audio(
    audio_data: Arc<[u8]>,
    mut consume: impl FnMut(&[f32], u32),
) -> Result<(), WaveformError> {
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
    let track = format.default_track().ok_or(WaveformError::NoTrack)?;

    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or(WaveformError::NoSampleRate)?;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &decoder_opts)
        .map_err(|e| WaveformError::Decoder(e.to_string()))?;

    let track_id = track.id;
    let time_base = track.codec_params.time_base;
    let mut total_samples = 0u64;
    let mut missing_samples = 0u64;
    let mut first_packet_error = None;
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
            Err(error) => return Err(WaveformError::Decode(error.to_string())),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(error @ (DecodeError::DecodeError(_) | DecodeError::IoError(_))) => {
                // Symphonia permits continuing after these packet errors. Keep
                // decoder state (including the MP3 reservoir) for the next frame.
                // Packet duration is in the track's time base, not always samples.
                let samples = time_base
                    .filter(|base| base.numer > 0 && base.denom > 0)
                    .map(|base| {
                        u128::from(packet.dur()) * u128::from(base.numer) * u128::from(sample_rate)
                            / u128::from(base.denom)
                    })
                    .unwrap_or(0);
                if samples == 0 || samples > u128::from(sample_rate) {
                    return Err(WaveformError::Decode(format!(
                        "cannot recover an audio packet with unknown or excessive duration: {error}"
                    )));
                }
                missing_samples += samples as u64;
                first_packet_error.get_or_insert_with(|| error.to_string());
                // Bounded chunks avoid an allocation derived from external metadata.
                let silence = [0.0; 1024];
                let mut remaining = samples as usize;
                while remaining > 0 {
                    let count = remaining.min(silence.len());
                    consume(&silence[..count], sample_rate);
                    remaining -= count;
                }
                continue;
            }
            Err(error) => return Err(WaveformError::Decode(error.to_string())),
        };

        let spec = *decoded.spec();
        let mut sample_buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        sample_buf.copy_interleaved_ref(decoded);

        // Mix to mono if stereo
        let channel_count = spec.channels.count();
        let buf_samples = sample_buf.samples();
        if channel_count == 0 {
            return Err(WaveformError::Decode(
                "decoded packet reported zero audio channels".to_string(),
            ));
        }
        if spec.rate != sample_rate || sample_rate == 0 {
            return Err(WaveformError::Decode(
                "Sample rate changed during analysis".into(),
            ));
        }
        let mono: Vec<f32> = buf_samples
            .chunks_exact(channel_count)
            .map(|frame| frame.iter().sum::<f32>() / channel_count as f32)
            .collect();
        total_samples += mono.len() as u64;
        consume(&mono, sample_rate);
    }
    if total_samples == 0 {
        return Err(WaveformError::NoSamples);
    }
    if let Some(error) = first_packet_error {
        // Use decoded duration, not the server's possibly inaccurate metadata.
        // Some encoders produce several seconds of rejected frames at a track's
        // edges; an absolute one-second cutoff incorrectly rejects those albums.
        if missing_samples > (total_samples / 19).max(u64::from(sample_rate)) {
            return Err(WaveformError::Decode(format!(
                "too much undecodable audio (over 5% and one second): {error}"
            )));
        }
        tracing::warn!(missing_samples, sample_rate, %error, "Audio analysis recovered undecodable packets as timed gaps");
    }
    Ok(())
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
    audio_data: Arc<[u8]>,
) -> Result<WaveformData, WaveformError> {
    super::analysis::analyze_audio(track_key, duration_ms, audio_data, false).map(|(wave, _)| wave)
}

/// Generate waveform data from already-decoded mono PCM samples.
pub fn generate_waveform_from_pcm(
    track_key: String,
    duration_ms: u64,
    samples: &[f32],
) -> WaveformData {
    let bins = compute_rms_bins(samples, DEFAULT_BIN_COUNT);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    WaveformData {
        track_key,
        duration_ms,
        bins,
        version: WAVEFORM_VERSION,
        created_at: now,
    }
}

/// Waveform cache for persisting waveform data.
#[derive(Clone)]
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

    fn scoped_cache_path(&self, scope: &str, track_key: &str) -> PathBuf {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        scope.hash(&mut hasher);
        track_key.hash(&mut hasher);
        self.cache_dir
            .join(format!("{:016x}.json", hasher.finish()))
    }

    /// Load waveform from cache.
    pub fn load(&self, track_key: &str) -> Option<WaveformData> {
        let path = self.cache_path(track_key);
        self.load_path(path, track_key)
    }

    /// Load a waveform whose server rating key is namespaced by server/library.
    pub fn load_scoped(&self, scope: &str, track_key: &str) -> Option<WaveformData> {
        let path = self.scoped_cache_path(scope, track_key);
        self.load_path(path, track_key)
    }

    fn load_path(&self, path: PathBuf, track_key: &str) -> Option<WaveformData> {
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
        let path = self.cache_path(&data.track_key);
        self.save_path(path, data)
    }

    /// Save a waveform under a server/library namespace while retaining the
    /// original rating key inside the serialized data for UI validation.
    pub fn save_scoped(&self, scope: &str, data: &WaveformData) -> bool {
        let path = self.scoped_cache_path(scope, &data.track_key);
        self.save_path(path, data)
    }

    fn save_path(&self, path: PathBuf, data: &WaveformData) -> bool {
        // Ensure directory exists
        if !self.cache_dir.exists() && std::fs::create_dir_all(&self.cache_dir).is_err() {
            return false;
        }

        match serde_json::to_string(data) {
            Ok(contents) => {
                // Atomic write via temp file
                let temp_path = path.with_extension(format!("json.{}.tmp", uuid::Uuid::new_v4(),));
                if std::fs::write(&temp_path, &contents).is_ok() {
                    let saved = std::fs::rename(&temp_path, &path).is_ok();
                    if !saved {
                        let _ = std::fs::remove_file(temp_path);
                    }
                    saved
                } else {
                    let _ = std::fs::remove_file(temp_path);
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
                if !path.is_file() || path.extension().is_none_or(|e| e != "json") {
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
                if !path.is_file() || path.extension().is_none_or(|e| e != "json") {
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

    /// Get cache statistics: (file_count, total_bytes).
    pub fn stats(&self) -> (usize, u64) {
        if !self.cache_dir.exists() {
            return (0, 0);
        }

        let mut count = 0usize;
        let mut total = 0u64;
        if let Ok(entries) = std::fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_file() {
                        count += 1;
                        total += metadata.len();
                    }
                }
            }
        }
        (count, total)
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
        let cache_dir =
            get_waveform_cache_dir().unwrap_or_else(|| PathBuf::from("/tmp/textamp_waveforms"));

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

#[cfg(test)]
mod tests {
    use super::*;

    // Hand-built MPEG-1 Layer III, 128 kbps / 44.1 kHz stereo silence. No
    // copyrighted audio or external encoder is needed for these regression tests.
    fn mp3_frame(damaged: bool) -> Vec<u8> {
        let mut frame = vec![0; 417];
        frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0]);
        if damaged {
            // First granule/channel consumes all 381 main-data bytes. The next
            // channel then references the end of the reservoir (invalid offset).
            frame[6] = 0x0b;
            frame[7] = 0xe8;
        }
        frame
    }

    #[test]
    fn mp3_fixture_reproduces_invalid_main_data_offset() {
        use symphonia::core::{codecs, errors::Error, formats::Packet};
        let mut params = codecs::CodecParameters::new();
        params.for_codec(codecs::CODEC_TYPE_MP3);
        let mut decoder = symphonia::default::get_codecs()
            .make(&params, &DecoderOptions::default())
            .unwrap();
        let packet = Packet::new_from_slice(0, 0, 1152, &mp3_frame(true));
        assert!(matches!(
            decoder.decode(&packet),
            Err(Error::DecodeError("mpa: invalid main_data offset"))
        ));
    }

    #[test]
    fn mp3_analysis_recovers_bad_frames_without_shortening_timeline() {
        for damaged_index in [None, Some(0), Some(5), Some(11)] {
            let audio: Vec<_> = (0..12)
                .flat_map(|i| mp3_frame(Some(i) == damaged_index))
                .collect();
            let mut samples = 0;
            decode_audio(audio.into(), |pcm, rate| {
                assert_eq!(rate, 44100);
                samples += pcm.len();
                assert!(pcm.iter().all(|sample| sample.is_finite()));
            })
            .unwrap();
            assert_eq!(samples, 12 * 1152);
        }
    }

    #[test]
    fn mp3_analysis_rejects_entirely_undecodable_audio() {
        let audio: Vec<_> = (0..4).flat_map(|_| mp3_frame(true)).collect();
        assert!(matches!(
            decode_audio(audio.into(), |_, _| {}),
            Err(WaveformError::NoSamples)
        ));
    }

    #[test]
    fn mp3_analysis_rejects_excessive_damage() {
        let audio: Vec<_> = (0..100).flat_map(|i| mp3_frame(i % 2 == 1)).collect();
        let error = decode_audio(audio.into(), |_, _| {}).unwrap_err();
        assert!(error.to_string().contains("too much undecodable audio"));
        assert!(error.to_string().contains("invalid main_data offset"));
    }

    #[test]
    fn mp3_analysis_tolerates_short_runs_relative_to_track_duration() {
        // Real encoder output can have a few seconds of rejected frames in an
        // otherwise decodable track. Cover leading, internal and trailing runs.
        for start in [0, 1000, 1920] {
            let audio: Vec<_> = (0..2000)
                .flat_map(|i| mp3_frame((start..start + 80).contains(&i)))
                .collect();
            let mut samples = 0;
            decode_audio(audio.into(), |pcm, _| samples += pcm.len()).unwrap();
            assert_eq!(samples, 2000 * 1152);
        }
    }

    #[test]
    fn mp3_waveform_and_spectrogram_keep_timed_gaps() {
        let analyze = |damaged| {
            let audio: Vec<_> = (0..30)
                .flat_map(|i| mp3_frame(damaged && i % 3 == 1))
                .collect();
            super::super::analysis::analyze_audio("test".into(), 784, audio.into(), true).unwrap()
        };
        let (clean_wave, clean_spectrum) = analyze(false);
        let (recovered_wave, recovered_spectrum) = analyze(true);
        assert_eq!(recovered_wave.bins, clean_wave.bins);
        let clean_spectrum = clean_spectrum.unwrap();
        let recovered_spectrum = recovered_spectrum.unwrap();
        assert_eq!(recovered_spectrum.frame_count, clean_spectrum.frame_count);
        assert_eq!(
            recovered_spectrum.frames_per_second,
            clean_spectrum.frames_per_second
        );
        assert_eq!(recovered_spectrum.frames, clean_spectrum.frames);
    }

    #[test]
    fn indexed_resampling_matches_allocating_resample() {
        let data = WaveformData {
            track_key: "track".to_string(),
            duration_ms: 1_000,
            bins: vec![0.1, 0.7, 0.2, 0.9],
            version: WAVEFORM_VERSION,
            created_at: 1,
        };

        let allocating = data.resample(2);
        let indexed = (0..2)
            .map(|index| data.resampled_peak_at(2, index))
            .collect::<Vec<_>>();

        assert_eq!(indexed, allocating);
        assert_eq!(indexed, vec![0.7, 0.9]);
        assert_eq!(data.resampled_peak_at(2, 2), 0.0);
    }

    #[test]
    fn scoped_cache_does_not_reuse_same_rating_key_across_servers() {
        let directory =
            std::env::temp_dir().join(format!("textamp-waveform-test-{}", uuid::Uuid::new_v4(),));
        let cache = WaveformCache::new(directory.clone());
        let data = WaveformData {
            track_key: "123".to_string(),
            duration_ms: 1_000,
            bins: vec![0.25, 0.75],
            version: WAVEFORM_VERSION,
            created_at: 1,
        };

        assert!(cache.save_scoped("server-a\0library-1", &data));
        assert!(cache.load_scoped("server-a\0library-1", "123").is_some());
        assert!(cache.load_scoped("server-b\0library-1", "123").is_none());

        let _ = std::fs::remove_dir_all(directory);
    }
}
