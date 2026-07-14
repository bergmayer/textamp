//! Track audio pre-fetch cache.
//!
//! Downloads upcoming tracks in the background so playback starts instantly.
//! Uses an in-memory LRU cache bounded by entry count and total bytes.

use crate::plex::PlexClient;
use crate::plex::models::Track;

use futures::StreamExt;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Maximum number of cached tracks (10 upcoming + 3 recently played).
const MAX_ENTRIES: usize = 13;

/// Maximum total cache size in bytes. This is deliberately conservative:
/// decoded playback already consumes a separate PCM ring, and an 800 MiB
/// compressed cache caused severe memory pressure on ordinary laptops.
const MAX_BYTES: usize = 128 * 1024 * 1024;

/// Per-track prefetch ceiling. With one active download this also caps memory
/// not yet admitted to the LRU at roughly 32 MiB; oversized tracks fall
/// back to the incremental streaming path.
const MAX_PREFETCH_TRACK_BYTES: usize = 32 * 1024 * 1024;

/// Keep prefetch subordinate to the active incremental playback stream. One
/// download still fills the immediate next-track cache without opening three
/// competing transfers on a slow Plex connection.
const MAX_CONCURRENT_DOWNLOADS: usize = 1;

/// Maximum retry attempts per URL.
const MAX_RETRIES: u32 = 3;

/// A cached audio track with access timestamp for LRU eviction.
struct CachedTrack {
    data: Arc<Vec<u8>>,
    accessed: Instant,
}

/// Thread-safe cache for pre-fetched track audio data.
pub struct TrackAudioCache {
    entries: Mutex<HashMap<String, CachedTrack>>,
    in_flight: Mutex<HashMap<String, u64>>,
    generation: AtomicU64,
    semaphore: Arc<tokio::sync::Semaphore>,
}

impl TrackAudioCache {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(HashMap::new()),
            generation: AtomicU64::new(0),
            semaphore: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_DOWNLOADS)),
        }
    }

    /// Get cached audio data, updating the access timestamp.
    /// Returns an Arc clone (cheap pointer copy, not a full data copy).
    pub fn get(&self, key: &str) -> Option<Arc<Vec<u8>>> {
        let mut entries = super::lock_or_recover(&self.entries);
        if let Some(entry) = entries.get_mut(key) {
            entry.accessed = Instant::now();
            Some(entry.data.clone())
        } else {
            None
        }
    }

    /// Insert audio data into the cache, evicting LRU entries if limits exceeded.
    pub fn insert(&self, key: String, data: Vec<u8>) {
        if data.len() > MAX_BYTES {
            tracing::debug!(
                "Track cache: skipped {}-byte item larger than cache budget",
                data.len()
            );
            return;
        }
        let mut entries = super::lock_or_recover(&self.entries);

        let data_size = data.len();
        entries.insert(key, CachedTrack {
            data: Arc::new(data),
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
        super::lock_or_recover(&self.entries).contains_key(key)
    }

    /// Mark a key as currently being downloaded.
    /// Returns the cache generation if the fetch was admitted.
    pub fn start_fetch(&self, key: &str) -> Option<u64> {
        if self.contains(key) {
            return None;
        }
        let generation = self.generation.load(Ordering::Acquire);
        let mut in_flight = super::lock_or_recover(&self.in_flight);
        if in_flight.contains_key(key) {
            None
        } else {
            in_flight.insert(key.to_string(), generation);
            Some(generation)
        }
    }

    /// Remove a key from the in-flight set (download finished or failed).
    pub fn finish_fetch(&self, key: &str, generation: u64) {
        let mut in_flight = super::lock_or_recover(&self.in_flight);
        if in_flight.get(key) == Some(&generation) {
            in_flight.remove(key);
        }
    }

    fn generation_is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Acquire) == generation
    }

    /// Remove a specific entry (e.g., corrupt data fallback).
    pub fn remove(&self, key: &str) {
        let removed = super::lock_or_recover(&self.entries).remove(key);
        if let Some(removed) = removed {
            let _ = std::thread::Builder::new()
                .name("textamp-audio-cache-drop".to_string())
                .spawn(move || drop(removed));
        }
    }

    /// Clear all entries and in-flight state.
    pub fn flush(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        let previous = {
            let mut entries = super::lock_or_recover(&self.entries);
            std::mem::take(&mut *entries)
        };
        super::lock_or_recover(&self.in_flight).clear();
        if !previous.is_empty() {
            let _ = std::thread::Builder::new()
                .name("textamp-audio-cache-drop".to_string())
                .spawn(move || drop(previous));
        }
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
        let entries = super::lock_or_recover(&self.entries);
        let in_flight = super::lock_or_recover(&self.in_flight);
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
pub async fn download_track_audio(url: &str, fallback_url: Option<&str>, headers: reqwest::header::HeaderMap, http_client: reqwest::Client) -> Result<Vec<u8>, String> {
    download_track_audio_inner(url, fallback_url, headers, http_client, None).await
}

type PrefetchGeneration<'a> = Option<(&'a TrackAudioCache, u64)>;

fn prefetch_cancelled(cancellation: PrefetchGeneration<'_>) -> bool {
    cancellation.is_some_and(|(cache, generation)| !cache.generation_is_current(generation))
}

async fn download_track_audio_inner(
    url: &str,
    fallback_url: Option<&str>,
    headers: reqwest::header::HeaderMap,
    http_client: reqwest::Client,
    cancellation: PrefetchGeneration<'_>,
) -> Result<Vec<u8>, String> {
    // Try primary URL
    match download_with_retry(url, &headers, &http_client, cancellation).await {
        Ok(data) => return Ok(data),
        Err(primary_err) => {
            if prefetch_cancelled(cancellation) {
                return Err("Prefetch cancelled".to_string());
            }
            tracing::warn!("Pre-fetch primary download failed: {}", primary_err);
            // Try fallback if available
            if let Some(fb_url) = fallback_url {
                match download_with_retry(fb_url, &headers, &http_client, cancellation).await {
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
async fn download_with_retry(
    url: &str,
    headers: &reqwest::header::HeaderMap,
    client: &reqwest::Client,
    cancellation: PrefetchGeneration<'_>,
) -> Result<Vec<u8>, String> {
    let backoff_secs = [1, 2, 4];

    for attempt in 0..MAX_RETRIES {
        if prefetch_cancelled(cancellation) {
            return Err("Prefetch cancelled".to_string());
        }
        match client.get(url).headers(headers.clone()).send().await {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    if response.content_length().is_some_and(|length| {
                        length > MAX_PREFETCH_TRACK_BYTES as u64
                    }) {
                        return Err(format!(
                            "Track exceeds prefetch limit of {} MiB",
                            MAX_PREFETCH_TRACK_BYTES / (1024 * 1024)
                        ));
                    }
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

                    let initial_capacity = response
                        .content_length()
                        .and_then(|length| usize::try_from(length).ok())
                        .unwrap_or(256 * 1024)
                        .min(MAX_PREFETCH_TRACK_BYTES);
                    let mut data = Vec::with_capacity(initial_capacity);
                    let mut body = response.bytes_stream();
                    let mut body_error = None;
                    while let Some(chunk) = body.next().await {
                        if prefetch_cancelled(cancellation) {
                            return Err("Prefetch cancelled".to_string());
                        }
                        match chunk {
                            Ok(chunk) => {
                                if data.len().saturating_add(chunk.len())
                                    > MAX_PREFETCH_TRACK_BYTES
                                {
                                    return Err(format!(
                                        "Track exceeds prefetch limit of {} MiB",
                                        MAX_PREFETCH_TRACK_BYTES / (1024 * 1024)
                                    ));
                                }
                                data.extend_from_slice(&chunk);
                            }
                            Err(error) => {
                                body_error = Some(error);
                                break;
                            }
                        }
                    }
                    match body_error {
                        None => {
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
                        Some(_error) => {
                            if attempt + 1 < MAX_RETRIES {
                                let delay = backoff_secs[attempt as usize];
                                tracing::debug!("Download body error (attempt {}), retrying in {}s", attempt + 1, delay);
                                tokio::time::sleep(Duration::from_secs(delay)).await;
                                continue;
                            }
                            return Err("Audio transfer was interrupted".to_string());
                        }
                    }
                }

                // Retry on 5xx and 429
                if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    if attempt + 1 < MAX_RETRIES {
                        let delay = response
                            .headers()
                            .get(reqwest::header::RETRY_AFTER)
                            .and_then(|value| value.to_str().ok())
                            .and_then(|value| value.parse::<u64>().ok())
                            .unwrap_or(backoff_secs[attempt as usize])
                            .min(30);
                        tracing::debug!("HTTP {} (attempt {}), retrying in {}s", status, attempt + 1, delay);
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                        continue;
                    }
                }

                // 4xx (except 429) - don't retry
                return Err(format!("HTTP {}", status));
            }
            Err(error) => {
                // A reqwest transport error may embed the request URL. Plex
                // transcode URLs contain the token in their query string, so
                // never copy that error verbatim into logs or UI state.
                let reason = if error.is_timeout() {
                    "request timed out"
                } else if error.is_connect() {
                    "connection failed"
                } else {
                    "request failed"
                };
                // Retry on timeout and connection errors
                if attempt + 1 < MAX_RETRIES {
                    let delay = backoff_secs[attempt as usize];
                    tracing::debug!("Request error (attempt {}), retrying in {}s", attempt + 1, delay);
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                    continue;
                }
                return Err(reason.to_string());
            }
        }
    }

    Err("Max retries exceeded".to_string())
}

/// Spawn background tasks to pre-fetch upcoming tracks.
///
/// Non-blocking: spawns tokio tasks and returns immediately.
/// Limits concurrent downloads via semaphore.
pub fn trigger_prefetch(
    cache: &Arc<TrackAudioCache>,
    upcoming_tracks: &[Track],
    client: &PlexClient,
    transcode_kbps: u32,
) {
    for track in upcoming_tracks {
        // Skip if already cached or being downloaded
        let Some(fetch_generation) = cache.start_fetch(&track.rating_key) else {
            continue;
        };

        // For direct play, build URL synchronously. For transcode, defer to async task.
        let direct_url = if transcode_kbps == 0 {
            match client.get_stream_url(track) {
                Ok(url) => Some(url),
                Err(_) => {
                    cache.finish_fetch(&track.rating_key, fetch_generation);
                    continue;
                }
            }
        } else {
            None
        };

        let cache = Arc::clone(cache);
        let rating_key = track.rating_key.clone();
        let title = track.title.clone();
        let track_clone = track.clone();
        let semaphore = Arc::clone(&cache.semaphore);
        // Transcode URLs have all auth in the query string — headers would duplicate and cause 400
        let stream_headers = if transcode_kbps > 0 {
            reqwest::header::HeaderMap::new()
        } else {
            client.stream_headers()
        };
        let http_client = client.http_client().clone();
        let plex_client = client.clone();

        tokio::spawn(async move {
            struct InFlightGuard {
                cache: Arc<TrackAudioCache>,
                key: String,
                generation: u64,
            }

            impl Drop for InFlightGuard {
                fn drop(&mut self) {
                    self.cache.finish_fetch(&self.key, self.generation);
                }
            }

            let _in_flight = InFlightGuard {
                cache: cache.clone(),
                key: rating_key.clone(),
                generation: fetch_generation,
            };

            // Acquire semaphore permit (limits concurrent downloads)
            let _permit = match semaphore.acquire().await {
                Ok(permit) => permit,
                Err(_) => return,
            };

            // Resolve URL (transcode requires async HLS playlist fetch)
            let primary_url = if let Some(url) = direct_url {
                url
            } else {
                match plex_client.get_transcoded_stream_url(&track_clone, transcode_kbps).await {
                    Ok(url) => url,
                    Err(e) => {
                        tracing::warn!("Pre-fetch transcode URL failed for {}: {}", title, e);
                        return;
                    }
                }
            };

            tracing::debug!("Pre-fetching: {}", title);
            match download_track_audio_inner(
                &primary_url,
                None,
                stream_headers,
                http_client,
                Some((&cache, fetch_generation)),
            )
            .await
            {
                Ok(data) => {
                    let size = data.len();
                    if cache.generation_is_current(fetch_generation) {
                        cache.insert(rating_key.clone(), data);
                        tracing::debug!("Pre-fetched: {} ({} bytes)", title, size);
                    } else {
                        tracing::debug!("Discarded stale prefetch for {}", title);
                    }
                }
                Err(e) => {
                    if cache.generation_is_current(fetch_generation) {
                        tracing::warn!("Pre-fetch failed for {}: {}", title, e);
                    } else {
                        tracing::debug!("Cancelled stale prefetch for {}", title);
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flush_invalidates_in_flight_download_without_removing_new_generation() {
        let cache = Arc::new(TrackAudioCache::new());
        let old_generation = cache.start_fetch("same-key").unwrap();
        cache.flush();
        let new_generation = cache.start_fetch("same-key").unwrap();
        assert_ne!(old_generation, new_generation);

        cache.finish_fetch("same-key", old_generation);
        assert!(cache.start_fetch("same-key").is_none());

        cache.finish_fetch("same-key", new_generation);
        assert!(cache.start_fetch("same-key").is_some());
    }

    #[tokio::test]
    async fn stale_prefetch_is_cancelled_before_opening_network_request() {
        let cache = TrackAudioCache::new();
        let generation = cache.start_fetch("track").unwrap();
        cache.flush();

        let error = download_with_retry(
            "http://127.0.0.1:9/should-not-be-opened",
            &reqwest::header::HeaderMap::new(),
            &reqwest::Client::new(),
            Some((&cache, generation)),
        )
        .await
        .unwrap_err();

        assert_eq!(error, "Prefetch cancelled");
    }
}
