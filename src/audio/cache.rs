//! Track audio pre-fetch cache.
//!
//! Downloads upcoming tracks in the background so playback starts instantly.
//! Uses an in-memory LRU cache bounded by entry count and total bytes.

use crate::api::PlexClient;
use crate::api::models::Track;
use crate::app::AppState;
use crate::app::state::PlaybackMode;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Maximum number of cached tracks (10 upcoming + 3 recently played).
const MAX_ENTRIES: usize = 13;

/// Maximum total cache size in bytes (800 MB).
const MAX_BYTES: usize = 800 * 1024 * 1024;

/// Maximum concurrent background downloads.
const MAX_CONCURRENT_DOWNLOADS: usize = 3;

/// Maximum retry attempts per URL.
const MAX_RETRIES: u32 = 3;

/// A cached audio track with access timestamp for LRU eviction.
struct CachedTrack {
    data: Vec<u8>,
    accessed: Instant,
}

/// Thread-safe cache for pre-fetched track audio data.
pub struct TrackAudioCache {
    entries: Mutex<HashMap<String, CachedTrack>>,
    in_flight: Mutex<HashSet<String>>,
    semaphore: Arc<tokio::sync::Semaphore>,
}

impl TrackAudioCache {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(HashSet::new()),
            semaphore: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_DOWNLOADS)),
        }
    }

    /// Get cached audio data, updating the access timestamp.
    /// Returns a clone of the data.
    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        let mut entries = self.entries.lock().unwrap();
        if let Some(entry) = entries.get_mut(key) {
            entry.accessed = Instant::now();
            Some(entry.data.clone())
        } else {
            None
        }
    }

    /// Insert audio data into the cache, evicting LRU entries if limits exceeded.
    pub fn insert(&self, key: String, data: Vec<u8>) {
        let mut entries = self.entries.lock().unwrap();

        let data_size = data.len();
        entries.insert(key, CachedTrack {
            data,
            accessed: Instant::now(),
        });

        // Evict by count
        while entries.len() > MAX_ENTRIES {
            if let Some(oldest_key) = Self::find_oldest(&entries) {
                entries.remove(&oldest_key);
            } else {
                break;
            }
        }

        // Evict by total size
        let mut total: usize = entries.values().map(|e| e.data.len()).sum();
        while total > MAX_BYTES {
            if let Some(oldest_key) = Self::find_oldest(&entries) {
                if let Some(removed) = entries.remove(&oldest_key) {
                    total -= removed.data.len();
                }
            } else {
                break;
            }
        }

        if data_size > 0 {
            tracing::debug!(
                "Track cache: inserted ({} bytes), {} entries, {:.1} MB total",
                data_size,
                entries.len(),
                total as f64 / (1024.0 * 1024.0),
            );
        }
    }

    /// Check if a key is cached (without cloning data).
    pub fn contains(&self, key: &str) -> bool {
        self.entries.lock().unwrap().contains_key(key)
    }

    /// Mark a key as currently being downloaded.
    /// Returns false if already in-flight or already cached.
    pub fn start_fetch(&self, key: &str) -> bool {
        if self.contains(key) {
            return false;
        }
        let mut in_flight = self.in_flight.lock().unwrap();
        in_flight.insert(key.to_string())
    }

    /// Remove a key from the in-flight set (download finished or failed).
    pub fn finish_fetch(&self, key: &str) {
        self.in_flight.lock().unwrap().remove(key);
    }

    /// Remove a specific entry (e.g., corrupt data fallback).
    pub fn remove(&self, key: &str) {
        self.entries.lock().unwrap().remove(key);
    }

    /// Clear all entries and in-flight state.
    pub fn flush(&self) {
        self.entries.lock().unwrap().clear();
        self.in_flight.lock().unwrap().clear();
        tracing::debug!("Track cache flushed");
    }

    /// Find the key with the oldest access timestamp.
    fn find_oldest(entries: &HashMap<String, CachedTrack>) -> Option<String> {
        entries
            .iter()
            .min_by_key(|(_, v)| v.accessed)
            .map(|(k, _)| k.clone())
    }
}

impl std::fmt::Debug for TrackAudioCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let entries = self.entries.lock().unwrap();
        let in_flight = self.in_flight.lock().unwrap();
        f.debug_struct("TrackAudioCache")
            .field("entries", &entries.len())
            .field("in_flight", &in_flight.len())
            .finish()
    }
}

/// Download track audio with retry and optional fallback URL.
///
/// Retries on 5xx, 429, timeouts, and connection errors.
/// Does NOT retry on 4xx client errors (except 429).
pub async fn download_track_audio(url: &str, fallback_url: Option<&str>) -> Result<Vec<u8>, String> {
    // Try primary URL
    match download_with_retry(url).await {
        Ok(data) => return Ok(data),
        Err(primary_err) => {
            tracing::warn!("Pre-fetch primary download failed: {}", primary_err);
            // Try fallback if available
            if let Some(fb_url) = fallback_url {
                match download_with_retry(fb_url).await {
                    Ok(data) => return Ok(data),
                    Err(fb_err) => {
                        return Err(format!("Both URLs failed: primary={}, fallback={}", primary_err, fb_err));
                    }
                }
            }
            Err(primary_err)
        }
    }
}

/// Download from a single URL with exponential backoff retry.
async fn download_with_retry(url: &str) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let backoff_secs = [1, 2, 4];

    for attempt in 0..MAX_RETRIES {
        match client.get(url).send().await {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    // Check for HTML content-type (Plex can return HTML errors with 200)
                    let is_html = response.headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .map(|ct| ct.contains("text/html"))
                        .unwrap_or(false);
                    if is_html {
                        if attempt + 1 < MAX_RETRIES {
                            let delay = backoff_secs[attempt as usize];
                            tracing::debug!("Pre-fetch got HTML content-type (attempt {}), retrying in {}s", attempt + 1, delay);
                            tokio::time::sleep(Duration::from_secs(delay)).await;
                            continue;
                        }
                        return Err("Server returned HTML instead of audio".to_string());
                    }

                    match response.bytes().await {
                        Ok(bytes) => {
                            let data = bytes.to_vec();
                            // Check downloaded bytes for HTML markers (small responses only)
                            if data.len() < 1024 * 1024 {
                                let prefix = &data[..data.len().min(256)];
                                let text = String::from_utf8_lossy(prefix).to_lowercase();
                                if text.contains("<!doctype html") || text.contains("<html") || text.contains("<head") {
                                    if attempt + 1 < MAX_RETRIES {
                                        let delay = backoff_secs[attempt as usize];
                                        tracing::debug!("Pre-fetch got HTML body (attempt {}), retrying in {}s", attempt + 1, delay);
                                        tokio::time::sleep(Duration::from_secs(delay)).await;
                                        continue;
                                    }
                                    return Err("Server returned HTML instead of audio".to_string());
                                }
                            }
                            return Ok(data);
                        }
                        Err(e) => {
                            if attempt + 1 < MAX_RETRIES {
                                let delay = backoff_secs[attempt as usize];
                                tracing::debug!("Download body error (attempt {}), retrying in {}s: {}", attempt + 1, delay, e);
                                tokio::time::sleep(Duration::from_secs(delay)).await;
                                continue;
                            }
                            return Err(format!("Download body error: {}", e));
                        }
                    }
                }

                // Retry on 5xx and 429
                if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    if attempt + 1 < MAX_RETRIES {
                        let delay = backoff_secs[attempt as usize];
                        tracing::debug!("HTTP {} (attempt {}), retrying in {}s", status, attempt + 1, delay);
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                        continue;
                    }
                }

                // 4xx (except 429) - don't retry
                return Err(format!("HTTP {}", status));
            }
            Err(e) => {
                // Retry on timeout and connection errors
                if attempt + 1 < MAX_RETRIES {
                    let delay = backoff_secs[attempt as usize];
                    tracing::debug!("Request error (attempt {}), retrying in {}s: {}", attempt + 1, delay, e);
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                    continue;
                }
                return Err(format!("Request failed: {}", e));
            }
        }
    }

    Err("Max retries exceeded".to_string())
}

/// Compute the list of upcoming tracks to pre-fetch from current state.
pub fn get_upcoming_tracks(state: &AppState) -> Vec<Track> {
    match state.playback_mode {
        PlaybackMode::Queue | PlaybackMode::None => {
            if let Some(idx) = state.queue_index {
                let start = idx + 1;
                let end = (start + 10).min(state.queue.len());
                if start < state.queue.len() {
                    return state.queue[start..end].to_vec();
                }
            }
            vec![]
        }
        PlaybackMode::Radio => {
            if let Some(idx) = state.radio.track_index {
                let start = idx + 1;
                let end = (start + 10).min(state.radio.tracks.len());
                if start < state.radio.tracks.len() {
                    return state.radio.tracks[start..end].to_vec();
                }
            }
            vec![]
        }
    }
}

/// Spawn background tasks to pre-fetch upcoming tracks.
///
/// Non-blocking: spawns tokio tasks and returns immediately.
/// Limits concurrent downloads via semaphore.
pub fn trigger_prefetch(
    cache: &Arc<TrackAudioCache>,
    upcoming_tracks: &[Track],
    client: &PlexClient,
) {
    for track in upcoming_tracks {
        // Skip if already cached or being downloaded
        if !cache.start_fetch(&track.rating_key) {
            continue;
        }

        // Build URLs
        let primary_url = match client.get_stream_url(track) {
            Ok(url) => url,
            Err(_) => {
                cache.finish_fetch(&track.rating_key);
                continue;
            }
        };
        let fallback_url = client.get_transcoded_stream_url(track).ok();

        let cache = Arc::clone(cache);
        let rating_key = track.rating_key.clone();
        let title = track.title.clone();
        let semaphore = Arc::clone(&cache.semaphore);

        tokio::spawn(async move {
            // Acquire semaphore permit (limits concurrent downloads)
            let _permit = match semaphore.acquire().await {
                Ok(permit) => permit,
                Err(_) => {
                    cache.finish_fetch(&rating_key);
                    return;
                }
            };

            tracing::debug!("Pre-fetching: {}", title);
            match download_track_audio(&primary_url, fallback_url.as_deref()).await {
                Ok(data) => {
                    let size = data.len();
                    cache.insert(rating_key.clone(), data);
                    cache.finish_fetch(&rating_key);
                    tracing::debug!("Pre-fetched: {} ({} bytes)", title, size);
                }
                Err(e) => {
                    cache.finish_fetch(&rating_key);
                    tracing::warn!("Pre-fetch failed for {}: {}", title, e);
                }
            }
        });
    }
}
