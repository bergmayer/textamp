//! Audio backend trait for cross-platform audio playback.
//!
//! This trait defines the interface that any platform-specific audio
//! implementation must fulfill:
//! - TUI (Linux/macOS/Windows): RodioBackend using rodio
//! - iOS: Would implement using AVFoundation
//! - Web: Would implement using Web Audio API
//!
//! The trait is designed to be:
//! - Synchronous (no async) for easy FFI binding
//! - Data-driven (accepts bytes, not URLs) to separate fetching from playback
//! - Minimal (only essential playback controls)

use std::time::Duration;

/// Error type for audio operations.
#[derive(Debug, Clone)]
pub enum AudioError {
    /// No audio device available.
    NoDevice,
    /// Failed to decode audio data.
    DecodeError(String),
    /// Playback error.
    PlaybackError(String),
    /// Seek not supported or failed.
    SeekError(String),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::NoDevice => write!(f, "No audio device available"),
            AudioError::DecodeError(msg) => write!(f, "Decode error: {}", msg),
            AudioError::PlaybackError(msg) => write!(f, "Playback error: {}", msg),
            AudioError::SeekError(msg) => write!(f, "Seek error: {}", msg),
        }
    }
}

impl std::error::Error for AudioError {}

/// Trait defining the audio player interface.
///
/// Implementations provide platform-specific audio playback while
/// exposing a consistent interface to the rest of the application.
///
/// # Design Principles
///
/// 1. **Synchronous**: Methods are synchronous to enable easy FFI binding
///    and avoid async runtime dependencies in platform implementations.
///
/// 2. **Data-driven**: `play_data` accepts raw audio bytes rather than URLs.
///    The application layer handles fetching; the audio layer handles playback.
///    This cleanly separates concerns and allows the fetch strategy to vary
///    (streaming, caching, etc.) independently of the audio backend.
///
/// 3. **Stateful**: The backend tracks its own playback state (playing,
///    paused, volume, position) rather than requiring the caller to manage it.
pub trait AudioBackend {
    /// Play audio from raw bytes.
    ///
    /// The caller is responsible for fetching/downloading the audio data.
    /// This method decodes and begins playback immediately.
    ///
    /// If audio is already playing, it should be stopped first.
    fn play_data(&mut self, data: Vec<u8>) -> Result<(), AudioError>;

    /// Check if this backend supports streaming playback.
    ///
    /// If true, the player will use streaming for large files.
    fn supports_streaming(&self) -> bool {
        false
    }

    /// Pause playback.
    ///
    /// Has no effect if already paused or stopped.
    fn pause(&mut self);

    /// Resume playback after pause.
    ///
    /// Has no effect if already playing or stopped.
    fn resume(&mut self);

    /// Stop playback and release resources.
    ///
    /// After stop, `is_finished()` should return true.
    fn stop(&mut self);

    /// Set volume level.
    ///
    /// # Arguments
    /// * `volume` - Volume level from 0.0 (mute) to 1.0 (full).
    ///   Values outside this range should be clamped.
    fn set_volume(&mut self, volume: f32);

    /// Get current volume level (0.0 to 1.0).
    fn volume(&self) -> f32;

    /// Check if playback has finished naturally (reached end of audio).
    ///
    /// Returns true if:
    /// - No audio is loaded
    /// - Audio finished playing
    /// - Audio was stopped
    fn is_finished(&self) -> bool;

    /// Check if audio is currently playing.
    ///
    /// Returns true only if audio is actively playing (not paused, not stopped).
    fn is_playing(&self) -> bool;

    /// Check if playback is paused.
    ///
    /// Returns true if audio was playing and then paused.
    fn is_paused(&self) -> bool;

    /// Seek to a position in the current track.
    ///
    /// # Arguments
    /// * `position` - Target position from start of track.
    ///
    /// # Returns
    /// * `true` if seek was successful
    /// * `false` if seeking is not supported or failed
    fn seek(&mut self, position: Duration) -> bool;

    /// Get current playback position.
    ///
    /// # Returns
    /// * `Some(duration)` - Current position from start of track
    /// * `None` - If position tracking is not available or no audio is loaded
    fn position(&self) -> Option<Duration>;
}
