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

pub use cache::TrackAudioCache;
pub use player::{AudioEvent, AudioPlayer};
pub use rodio_backend::RodioBackend;
pub use streaming::StreamingBuffer;
pub use traits::{AudioBackend, AudioError};
