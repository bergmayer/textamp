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
use crate::app::Event;
use anyhow::{anyhow, Result};
use futures::StreamExt;
use reqwest::header::HeaderMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;


/// High-level audio player with streaming support.
///
/// Uses RodioBackend directly to enable streaming playback.
/// For other backends, use the trait-based approach with full download.
///
/// # Example
///
/// ```ignore
/// let mut player = AudioPlayer::new()?;
/// player.play_url("http://example.com/track.mp3", event_tx).await?;
/// player.pause();
/// player.resume();
/// player.stop();
/// ```
pub struct AudioPlayer {
    backend: RodioBackend,
    /// Current streaming buffer (kept alive during playback)
    _stream_buffer: Option<Arc<StreamingBuffer>>,
    /// Shared inbox for pending playback — spawned tasks deliver buffers here,
    /// and start_pending_playback() reads from it on the main thread.
    pending_playback: Arc<Mutex<Option<(Arc<StreamingBuffer>, u64)>>>,
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
            pending_playback: Arc::new(Mutex::new(None)),
        })
    }

    /// Play audio from a URL (non-blocking).
    ///
    /// Spawns the HTTP request and buffer setup in a background task.
    /// Returns immediately — the event loop stays responsive.
    /// Sends `BufferingEnd` event when playback is ready to start.
    pub async fn play_url(&mut self, url: &str, event_tx: mpsc::Sender<Event>) -> Result<()> {
        self.play_url_with_headers(url, HeaderMap::new(), None, event_tx).await
    }

    /// Play audio from a URL with optional fallback (non-blocking).
    ///
    /// This method:
    /// 1. Stops current playback synchronously
    /// 2. Spawns a background task that fetches the audio
    /// 3. Returns immediately (no blocking network I/O)
    /// 4. The background task sends `BufferingEnd` when ready
    /// 5. `StartPendingPlayback` then starts actual audio output
    ///
    /// If `fallback_url` is provided and the primary URL fails,
    /// the background task automatically tries the fallback.
    pub async fn play_url_with_headers(
        &mut self,
        url: &str,
        headers: HeaderMap,
        fallback_url: Option<String>,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<()> {
        // Stop any existing playback synchronously
        self.stop();

        let pending = self.pending_playback.clone();
        let url = url.to_string();

        tokio::spawn(async move {
            // Try primary URL
            match fetch_and_buffer(&url, &headers, &pending, &event_tx).await {
                Ok(()) => return,
                Err(primary_err) => {
                    tracing::warn!("Primary stream failed: {}", primary_err);
                    // Try fallback if available
                    if let Some(ref fb_url) = fallback_url {
                        let redacted = fb_url.split("X-Plex-Token=").next().unwrap_or(fb_url);
                        tracing::info!("Trying fallback (transcode): {}...", redacted);
                        match fetch_and_buffer(fb_url, &headers, &pending, &event_tx).await {
                            Ok(()) => return,
                            Err(fb_err) => {
                                tracing::error!("Fallback stream also failed: {}", fb_err);
                                let _ = event_tx.send(Event::PlaybackError(
                                    format!("Playback failed: {}", fb_err)
                                )).await;
                            }
                        }
                    } else {
                        let _ = event_tx.send(Event::PlaybackError(
                            format!("Playback failed: {}", primary_err)
                        )).await;
                    }
                }
            }
        });

        Ok(())
    }

    /// Start playback from the pending buffer.
    ///
    /// Called when `BufferingEnd` event is received, indicating the streaming
    /// buffer has enough data for format detection and playback start.
    /// Safe to call when no pending playback exists (no-op).
    pub fn start_pending_playback(&mut self) -> Result<()> {
        let pending = self.pending_playback.lock().unwrap().take();
        if let Some((buffer, byte_len_hint)) = pending {
            let reader = BlockingReader::new(&buffer);
            self.backend.play_streaming(reader, byte_len_hint)
                .map_err(|e| anyhow!("Playback error: {}", e))?;
            self._stream_buffer = Some(buffer);
        }
        Ok(())
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

    /// Stop playback and clear pending state.
    pub fn stop(&mut self) {
        *self.pending_playback.lock().unwrap() = None;
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

/// Fetch audio from a URL, set up streaming buffer, and signal readiness.
///
/// This runs inside a spawned task. On success, sets the pending_playback
/// buffer and sends `BufferingEnd`. On failure, returns the error for
/// the caller to handle (e.g., try a fallback URL).
async fn fetch_and_buffer(
    url: &str,
    _headers: &HeaderMap,
    pending: &Arc<Mutex<Option<(Arc<StreamingBuffer>, u64)>>>,
    event_tx: &mpsc::Sender<Event>,
) -> Result<(), String> {
    tracing::debug!("Fetching audio from: {}", url);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let response = client.get(url).send().await
        .map_err(|e| format!("Failed to fetch audio: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::error!("Audio fetch failed: HTTP {} - Body: {}", status, body);
        return Err(format!("HTTP {}", status));
    }

    // Get content length for progress tracking
    let content_length = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok());
    let total_size = content_length.unwrap_or(0);

    // Small files (< 1MB): download fully, wrap in complete buffer
    if total_size > 0 && total_size < 1024 * 1024 {
        let bytes = response.bytes().await
            .map_err(|e| format!("Download failed: {}", e))?;
        let data = bytes.to_vec();
        let size = data.len();
        let buffer = Arc::new(StreamingBuffer::with_size_hint(size));
        buffer.append(&data);
        buffer.set_complete();
        *pending.lock().unwrap() = Some((buffer, size as u64));
        let _ = event_tx.send(Event::BufferingEnd).await;
        return Ok(());
    }

    // Large files: streaming buffer with progressive download
    let buffer = if total_size > 0 {
        Arc::new(StreamingBuffer::with_size_hint(total_size))
    } else {
        Arc::new(StreamingBuffer::with_size_hint(10 * 1024 * 1024))
    };
    let byte_len_hint = if total_size > 0 { total_size as u64 } else { u64::MAX };

    // Start download task
    let dl_buffer = buffer.clone();
    tokio::spawn(async move {
        let mut stream = response.bytes_stream();
        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(chunk) => dl_buffer.append(&chunk),
                Err(e) => {
                    dl_buffer.set_error(format!("Download error: {}", e));
                    return;
                }
            }
        }
        dl_buffer.set_complete();
    });

    // Wait for minimum buffer before signaling ready
    let wait_buffer = buffer.clone();
    let mut wait_count = 0;
    while !wait_buffer.has_min_buffer() && wait_count < 100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        wait_count += 1;
        if let Some(err) = wait_buffer.error() {
            return Err(format!("Download failed: {}", err));
        }
    }
    if !wait_buffer.has_min_buffer() {
        return Err("Timeout waiting for initial buffer".to_string());
    }

    // Deliver buffer and signal ready
    *pending.lock().unwrap() = Some((buffer, byte_len_hint));
    let _ = event_tx.send(Event::BufferingEnd).await;
    Ok(())
}
