//! Audio playback engine.
//!
//! This module provides a cross-platform audio interface:
//!
//! - `AudioBackend` trait: The interface any platform audio implementation must fulfill
//! - `RodioBackend`: Default implementation using rodio (for TUI on Linux/macOS/Windows)
//! - `AudioPlayer`: High-level wrapper that handles URL fetching and uses an AudioBackend
//! - `StreamingBuffer`: Buffer for progressive download playback
//!
//! # Cross-Platform Design
//!
//! When porting to other platforms:
//! - iOS: Implement `AudioBackend` using AVFoundation
//! - Web: Implement `AudioBackend` using Web Audio API
//! - The rest of the app interacts only with the trait, not concrete implementations

pub mod cache;
mod player;
mod rodio_backend;
mod streaming;
mod traits;

/// Lock a mutex, recovering from poisoning.
///
/// If another thread panicked while holding the lock, this ignores the poison
/// and returns the inner data. This is appropriate for audio caches/buffers
/// where a panic in another thread shouldn't crash the whole app.
pub(crate) fn lock_or_recover<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

pub use cache::TrackAudioCache;
pub use player::{AudioEvent, AudioPlayer};
pub use rodio_backend::{RodioBackend, SampleTap};
pub use streaming::StreamingBuffer;
pub use traits::{AudioBackend, AudioError};
