//! High-level audio player that wraps an AudioBackend.
//!
//! This module provides `AudioPlayer`, which:
//! - Handles async URL fetching with streaming support
//! - Delegates actual playback to an `AudioBackend` implementation
//! - Provides a convenient API for the rest of the application
//!
//! The separation allows the backend to be synchronous (important for FFI)
//! while the player handles the async network operations.

use super::rodio_backend::RodioBackend;
use super::streaming::{BlockingReader, StreamingBuffer};
use super::traits::AudioBackend;
use anyhow::{anyhow, Result};
use futures::StreamExt;
use std::sync::Arc;
use std::time::Duration;


/// High-level audio player with streaming support.
///
/// Uses RodioBackend directly to enable streaming playback.
/// For other backends, use the trait-based approach with full download.
///
/// # Example
///
/// ```ignore
/// let mut player = AudioPlayer::new()?;
/// player.play_url("http://example.com/track.mp3").await?;
/// player.pause();
/// player.resume();
/// player.stop();
/// ```
pub struct AudioPlayer {
    backend: RodioBackend,
    /// Current streaming buffer (kept alive during playback)
    _stream_buffer: Option<Arc<StreamingBuffer>>,
}

impl AudioPlayer {
    /// Create a new audio player with the default backend (rodio).
    ///
    /// Returns an error if no audio device is available.
    pub fn new() -> Result<Self> {
        let backend = RodioBackend::new()
            .map_err(|e| anyhow!("Failed to create audio backend: {}", e))?;

        Ok(Self {
            backend,
            _stream_buffer: None,
        })
    }

    /// Play audio from a URL with streaming support.
    ///
    /// This method:
    /// 1. Starts downloading the audio file
    /// 2. Buffers a minimum amount (256KB) for format detection
    /// 3. Begins playback while download continues in background
    ///
    /// This significantly reduces time-to-first-audio compared to
    /// downloading the entire file first.
    pub async fn play_url(&mut self, url: &str) -> Result<()> {
        // Stop any existing playback
        self.stop();

        let client = reqwest::Client::new();
        let response = client.get(url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow!("Failed to fetch audio: HTTP {}", response.status()));
        }

        // Get content length for progress tracking
        let content_length = response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<usize>().ok());

        let total_size = content_length.unwrap_or(0);

        // For small files (< 1MB) with known size, use simple buffered approach
        // This is safe because the download is quick
        if total_size > 0 && total_size < 1024 * 1024 {
            return self.play_url_simple(response).await;
        }

        // For unknown size or large files, use streaming to avoid blocking
        // This prevents the event loop from freezing during download
        let buffer = if total_size > 0 {
            Arc::new(StreamingBuffer::with_size_hint(total_size))
        } else {
            // Unknown size - use streaming without size hint
            // Use a reasonable default allocation (10MB)
            Arc::new(StreamingBuffer::with_size_hint(10 * 1024 * 1024))
        };

        // For byte_len hint to decoder, use actual size or a large value for unknown
        let byte_len_hint = if total_size > 0 { total_size as u64 } else { u64::MAX };

        // Start download task
        let buffer_clone = buffer.clone();
        let _download_handle = tokio::spawn(async move {
            let mut stream = response.bytes_stream();
            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(chunk) => {
                        buffer_clone.append(&chunk);
                    }
                    Err(e) => {
                        buffer_clone.set_error(format!("Download error: {}", e));
                        return;
                    }
                }
            }
            buffer_clone.set_complete();
        });

        // Wait for minimum buffer before starting playback
        let mut wait_count = 0;
        while !buffer.has_min_buffer() && wait_count < 100 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            wait_count += 1;

            // Check for early error
            if let Some(err) = buffer.error() {
                return Err(anyhow!("Download failed: {}", err));
            }
        }

        if !buffer.has_min_buffer() {
            return Err(anyhow!("Timeout waiting for initial buffer"));
        }

        // Start playback from streaming buffer
        let reader = BlockingReader::new(&buffer);

        self.backend
            .play_streaming(reader, byte_len_hint)
            .map_err(|e| anyhow!("Playback error: {}", e))?;

        // Keep buffer alive during playback
        self._stream_buffer = Some(buffer);

        Ok(())
    }

    /// Simple playback for small files - downloads fully before playing.
    async fn play_url_simple(&mut self, response: reqwest::Response) -> Result<()> {
        let bytes = response.bytes().await?;
        let data = bytes.to_vec();

        self.backend
            .play_data(data)
            .map_err(|e| anyhow!("Playback error: {}", e))
    }

    /// Play audio from raw bytes.
    ///
    /// Use this when you already have the audio data (e.g., from cache).
    pub fn play_data(&mut self, data: Vec<u8>) -> Result<()> {
        self._stream_buffer = None;
        self.backend
            .play_data(data)
            .map_err(|e| anyhow!("Playback error: {}", e))
    }

    /// Pause playback.
    pub fn pause(&mut self) {
        self.backend.pause();
    }

    /// Resume playback.
    pub fn resume(&mut self) {
        self.backend.resume();
    }

    /// Stop playback.
    pub fn stop(&mut self) {
        self._stream_buffer = None;
        self.backend.stop();
    }

    /// Set volume (0.0 to 1.0).
    pub fn set_volume(&mut self, volume: f32) {
        self.backend.set_volume(volume);
    }

    /// Get current volume.
    pub fn volume(&self) -> f32 {
        self.backend.volume()
    }

    /// Check if playback is finished.
    pub fn is_finished(&self) -> bool {
        self.backend.is_finished()
    }

    /// Check if currently playing.
    pub fn is_playing(&self) -> bool {
        self.backend.is_playing()
    }

    /// Check if paused.
    pub fn is_paused(&self) -> bool {
        self.backend.is_paused()
    }

    /// Seek to a position in the current track.
    ///
    /// Returns true if seeking was successful.
    pub fn try_seek(&mut self, position: Duration) -> bool {
        self.backend.seek(position)
    }

    /// Get the current playback position if available.
    pub fn position(&self) -> Option<Duration> {
        self.backend.position()
    }
}
