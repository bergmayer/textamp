//! High-level audio player that wraps an AudioBackend.
//!
//! This module provides `AudioPlayer`, which:
//! - Handles async URL fetching with streaming support
//! - Delegates actual playback to an `AudioBackend` implementation
//! - Provides a convenient API for the rest of the application
//!
//! The separation allows the backend to be synchronous (important for FFI)
//! while the player handles the async network operations.

use super::cache::TrackAudioCache;
use super::rodio_backend::RodioBackend;
use super::streaming::{BlockingReader, StreamingBuffer};
use super::traits::AudioBackend;
use anyhow::{anyhow, Result};
use futures::StreamExt;
use reqwest::header::HeaderMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

/// Events emitted by the audio player to signal state changes.
///
/// This decouples the audio module from the app event system,
/// making it portable to other platforms.
#[derive(Debug, Clone)]
pub enum AudioEvent {
    /// Streaming buffer has enough data to start playback.
    BufferingReady,
    /// An error occurred during audio fetching or playback.
    Error(String),
}

/// Shared pending playback buffer: spawned tasks write here, main thread reads.
type PendingPlayback = Arc<Mutex<Option<(Arc<StreamingBuffer>, u64)>>>;

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
    backend: Option<RodioBackend>,
    /// Current streaming buffer (kept alive during playback)
    _stream_buffer: Option<Arc<StreamingBuffer>>,
    /// Shared inbox for pending playback — spawned tasks deliver buffers here,
    /// and start_pending_playback() reads from it on the main thread.
    pending_playback: PendingPlayback,
    /// Cache for pre-fetched track audio data.
    pub track_cache: Arc<TrackAudioCache>,
    /// Generation counter to detect stale background fetch tasks.
    /// Incremented on each play_url_with_headers call; spawned tasks check
    /// this before delivering results to discard stale fetches.
    playback_generation: Arc<AtomicU64>,
}

impl AudioPlayer {
    /// Create a new audio player with the default backend (rodio).
    ///
    /// Returns an error if no audio device is available.
    pub fn new() -> Result<Self> {
        let backend = RodioBackend::new()
            .map_err(|e| anyhow!("Failed to create audio backend: {}", e))?;

        Ok(Self {
            backend: Some(backend),
            _stream_buffer: None,
            pending_playback: Arc::new(Mutex::new(None)),
            track_cache: Arc::new(TrackAudioCache::new()),
            playback_generation: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Create an audio player without a backend (no audio output).
    /// The app runs normally but playback operations are no-ops.
    /// Construct a player with no audio backend attached. This is
    /// infallible — the returned player is a stub that reports
    /// `has_audio() == false` and silently no-ops every play/seek call.
    /// Used by the GUI's audio-init retry loop when no output device
    /// is available, and by the test TUI binary that runs headless.
    pub fn new_without_audio() -> Self {
        Self {
            backend: None,
            _stream_buffer: None,
            pending_playback: Arc::new(Mutex::new(None)),
            track_cache: Arc::new(TrackAudioCache::new()),
            playback_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Play audio from a URL (non-blocking).
    ///
    /// Spawns the HTTP request and buffer setup in a background task.
    /// Returns immediately — the event loop stays responsive.
    /// Sends `AudioEvent::BufferingReady` when playback is ready to start.
    pub async fn play_url(&mut self, url: &str, event_tx: mpsc::Sender<AudioEvent>, http_client: reqwest::Client) -> Result<()> {
        self.play_url_with_headers(url, HeaderMap::new(), None, event_tx, http_client).await
    }

    /// Play audio from a URL with optional fallback (non-blocking).
    ///
    /// This method:
    /// 1. Stops current playback synchronously
    /// 2. Spawns a background task that fetches the audio
    /// 3. Returns immediately (no blocking network I/O)
    /// 4. The background task sends `AudioEvent::BufferingReady` when ready
    /// 5. `StartPendingPlayback` then starts actual audio output
    ///
    /// If `fallback_url` is provided and the primary URL fails,
    /// the background task automatically tries the fallback.
    ///
    /// `http_client`: use the PlexClient's HTTP client to share connection pool and settings.
    pub async fn play_url_with_headers(
        &mut self,
        url: &str,
        headers: HeaderMap,
        fallback_url: Option<String>,
        event_tx: mpsc::Sender<AudioEvent>,
        http_client: reqwest::Client,
    ) -> Result<()> {
        // Stop any existing playback synchronously
        self.stop();

        // Increment generation so any in-flight fetch tasks become stale
        let expected_gen = self.playback_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let generation = self.playback_generation.clone();
        let pending = self.pending_playback.clone();
        let url = url.to_string();

        tokio::spawn(async move {
            // Try primary URL
            match fetch_and_buffer(&url, &headers, &http_client, &pending, &event_tx, &generation, expected_gen).await {
                Ok(()) => {},
                Err(primary_err) => {
                    tracing::warn!("Primary stream failed: {}", primary_err);
                    // Try fallback if available
                    if let Some(ref fb_url) = fallback_url {
                        let redacted = fb_url.split("X-Plex-Token=").next().unwrap_or(fb_url);
                        tracing::info!("Trying fallback (transcode): {}...", redacted);
                        match fetch_and_buffer(fb_url, &headers, &http_client, &pending, &event_tx, &generation, expected_gen).await {
                            Ok(()) => {},
                            Err(fb_err) => {
                                tracing::error!("Fallback stream also failed: {}", fb_err);
                                let _ = event_tx.send(AudioEvent::Error(
                                    format!("Playback failed: {}", fb_err)
                                )).await;
                            }
                        }
                    } else {
                        let _ = event_tx.send(AudioEvent::Error(
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
    /// Returns `Ok(true)` if playback started, `Ok(false)` if no pending data
    /// (e.g., stale BufferingEnd event after audio was stopped).
    /// Whether audio output is available.
    pub fn has_audio(&self) -> bool {
        self.backend.is_some()
    }

    /// Clone of the rodio sample tap, if a backend is attached. Used
    /// by the vectorscope visualizer (TUI + GUI) to read recent
    /// stereo samples on each tick.
    pub fn sample_tap(&self) -> Option<super::rodio_backend::SampleTap> {
        self.backend.as_ref().map(|b| b.sample_tap())
    }

    /// Try to (re)attach a live audio backend. Used to recover from a
    /// startup where no device was available yet (e.g. Windows audio
    /// service still initializing right after login). Returns Ok(true)
    /// if the device is now usable, Ok(false) if we already had one,
    /// and Err with the cpal failure if retrying still fails.
    pub fn try_attach_backend(&mut self) -> Result<bool> {
        if self.backend.is_some() {
            return Ok(false);
        }
        let backend = RodioBackend::new()
            .map_err(|e| anyhow!("Failed to create audio backend: {}", e))?;
        self.backend = Some(backend);
        Ok(true)
    }

    pub fn start_pending_playback(&mut self) -> Result<bool> {
        let Some(ref mut backend) = self.backend else { return Ok(false) };
        let pending = super::lock_or_recover(&self.pending_playback).take();
        if let Some((buffer, byte_len_hint)) = pending {
            let reader = BlockingReader::new(&buffer);
            backend.play_streaming(reader, byte_len_hint)
                .map_err(|e| anyhow!("Decode error: {}", e))?;
            self._stream_buffer = Some(buffer);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn play_data(&mut self, data: Arc<Vec<u8>>) -> Result<()> {
        let Some(ref mut backend) = self.backend else { return Ok(()) };
        self._stream_buffer = None;
        backend.play_data(data).map_err(|e| anyhow!("Playback error: {}", e))
    }

    pub fn pause(&mut self) {
        if let Some(ref mut b) = self.backend { b.pause(); }
    }

    pub fn resume(&mut self) {
        if let Some(ref mut b) = self.backend { b.resume(); }
    }

    pub fn stop(&mut self) {
        *super::lock_or_recover(&self.pending_playback) = None;
        self._stream_buffer = None;
        if let Some(ref mut b) = self.backend { b.stop(); }
    }

    pub fn set_volume(&mut self, volume: f32) {
        if let Some(ref mut b) = self.backend { b.set_volume(volume); }
    }

    pub fn volume(&self) -> f32 {
        self.backend.as_ref().map_or(0.8, |b| b.volume())
    }

    pub fn is_finished(&self) -> bool {
        self.backend.as_ref().map_or(true, |b| b.is_finished())
    }

    pub fn is_playing(&self) -> bool {
        self.backend.as_ref().map_or(false, |b| b.is_playing())
    }

    pub fn is_paused(&self) -> bool {
        self.backend.as_ref().map_or(false, |b| b.is_paused())
    }

    pub fn try_seek(&mut self, position: Duration) -> bool {
        self.backend.as_mut().map_or(false, |b| b.seek(position))
    }

    pub fn position(&self) -> Option<Duration> {
        self.backend.as_ref().and_then(|b| b.position())
    }
}

/// Fetch audio from a URL, set up streaming buffer, and signal readiness.
///
/// This runs inside a spawned task. On success, sets the pending_playback
/// buffer and sends `BufferingEnd`. On failure, returns the error for
/// the caller to handle (e.g., try a fallback URL).
/// Check if a response has an HTML content-type header.
fn is_html_content_type(response: &reqwest::Response) -> bool {
    response.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.contains("text/html"))
        .unwrap_or(false)
}

/// Check if raw bytes look like HTML (starts with common HTML markers).
fn looks_like_html(data: &[u8]) -> bool {
    // Check first 256 bytes for HTML signatures
    let prefix = &data[..data.len().min(256)];
    let text = String::from_utf8_lossy(prefix);
    let lower = text.to_lowercase();
    lower.contains("<!doctype html") || lower.contains("<html") || lower.contains("<head")
}

async fn fetch_and_buffer(
    url: &str,
    headers: &HeaderMap,
    client: &reqwest::Client,
    pending: &PendingPlayback,
    event_tx: &mpsc::Sender<AudioEvent>,
    generation: &Arc<AtomicU64>,
    expected_gen: u64,
) -> Result<(), String> {
    tracing::debug!("Fetching audio from: {}", url);

    // Retry loop with exponential backoff for transient errors (including HTML responses)
    let backoff_secs = [1u64, 2, 4];
    let max_retries = 3u32;
    let mut last_error = String::new();

    let response = 'retry: {
        for attempt in 0..max_retries {
            match client.get(url).headers(headers.clone()).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        // Check for HTML content-type on successful responses
                        // Plex sometimes returns HTML error pages with 200 status
                        if is_html_content_type(&resp) {
                            last_error = "Server returned HTML instead of audio".to_string();
                            let body = resp.text().await.unwrap_or_default();
                            if attempt + 1 < max_retries {
                                let delay = backoff_secs[attempt as usize];
                                tracing::debug!("Audio fetch got HTML response (attempt {}), retrying in {}s - Body: {}",
                                    attempt + 1, delay, &body[..body.len().min(200)]);
                                tokio::time::sleep(Duration::from_secs(delay)).await;
                                continue;
                            }
                            tracing::warn!("Audio fetch got HTML after {} retries", max_retries);
                            return Err(last_error);
                        }
                        break 'retry resp;
                    }

                    // Retry on 5xx, 429, and HTML error bodies
                    if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        let body = resp.text().await.unwrap_or_default();
                        last_error = format!("HTTP {}", status);
                        if attempt + 1 < max_retries {
                            let delay = backoff_secs[attempt as usize];
                            tracing::debug!("Audio fetch HTTP {} (attempt {}), retrying in {}s", status, attempt + 1, delay);
                            tokio::time::sleep(Duration::from_secs(delay)).await;
                            continue;
                        }
                        tracing::error!("Audio fetch failed: HTTP {} - Body: {}", status, &body[..body.len().min(200)]);
                        return Err(last_error);
                    }

                    // 4xx (except 429) - don't retry
                    let body = resp.text().await.unwrap_or_default();
                    tracing::error!("Audio fetch failed: HTTP {} - Body: {}", status, &body[..body.len().min(200)]);
                    return Err(format!("HTTP {}", status));
                }
                Err(e) => {
                    last_error = format!("Failed to fetch audio: {}", e);
                    tracing::warn!("Audio fetch send error (attempt {}): {:?}", attempt + 1, e);
                    if attempt + 1 < max_retries {
                        let delay = backoff_secs[attempt as usize];
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                        continue;
                    }
                    return Err(last_error);
                }
            }
        }
        return Err(last_error);
    };

    // Log response headers for diagnostics
    {
        let h = response.headers();
        tracing::info!("Audio response: status={}, content-type={:?}, content-encoding={:?}, content-length={:?}, transfer-encoding={:?}",
            response.status(),
            h.get("content-type").map(|v| v.to_str().unwrap_or("?")),
            h.get("content-encoding").map(|v| v.to_str().unwrap_or("?")),
            h.get("content-length").map(|v| v.to_str().unwrap_or("?")),
            h.get("transfer-encoding").map(|v| v.to_str().unwrap_or("?")),
        );
    }

    // Get content length for progress tracking
    let content_length = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok());
    let total_size = content_length.unwrap_or(0);

    // Small files (< 1MB): download fully, check for HTML, wrap in complete buffer
    if total_size > 0 && total_size < 1024 * 1024 {
        let bytes = response.bytes().await
            .map_err(|e| { tracing::warn!("Small file bytes() error: {:?}", e); format!("Download failed: {}", e) })?;
        let data = bytes.to_vec();

        // Detect HTML that slipped through without a text/html content-type
        if looks_like_html(&data) {
            tracing::warn!("Downloaded data looks like HTML, not audio ({} bytes)", data.len());
            return Err("Server returned HTML instead of audio".to_string());
        }

        // Discard if a newer playback request has superseded this one
        if generation.load(Ordering::SeqCst) != expected_gen {
            tracing::debug!("Stale fetch (small file) discarded (gen {} != current {})", expected_gen, generation.load(Ordering::SeqCst));
            return Ok(());
        }
        let size = data.len();
        let buffer = Arc::new(StreamingBuffer::with_size_hint(size));
        buffer.append(&data);
        buffer.set_complete();
        *super::lock_or_recover(pending) = Some((buffer, size as u64));
        let _ = event_tx.send(AudioEvent::BufferingReady).await;
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
                    tracing::warn!("Streaming chunk error: {:?}", e);
                    dl_buffer.set_error(format!("Download error: {}", e));
                    return;
                }
            }
        }
        dl_buffer.set_complete();
    });

    // Wait for minimum buffer before signaling ready (10s timeout for remote servers)
    let wait_buffer = buffer.clone();
    let mut wait_count = 0;
    while !wait_buffer.has_min_buffer() && wait_count < 200 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        wait_count += 1;
        if let Some(err) = wait_buffer.error() {
            return Err(format!("Download failed: {}", err));
        }
    }
    if !wait_buffer.has_min_buffer() {
        return Err("Timeout waiting for initial buffer".to_string());
    }

    // Discard if a newer playback request has superseded this one
    if generation.load(Ordering::SeqCst) != expected_gen {
        tracing::debug!("Stale fetch (streaming) discarded (gen {} != current {})", expected_gen, generation.load(Ordering::SeqCst));
        return Ok(());
    }

    // Deliver buffer and signal ready
    *super::lock_or_recover(pending) = Some((buffer, byte_len_hint));
    let _ = event_tx.send(AudioEvent::BufferingReady).await;
    Ok(())
}
