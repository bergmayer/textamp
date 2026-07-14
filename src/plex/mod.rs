//! Plex integration layer.
//!
//! This module provides a unified interface for all Plex-related functionality:
//! - HTTP API client
//! - Authentication
//! - Data caching
//! - Waveform generation and caching
//! - Background preloading
//!
//! # Cross-Platform Design
//!
//! This module is designed to be portable. It has no dependencies on:
//! - UI frameworks (ratatui, SwiftUI, etc.)
//! - Audio playback (rodio, AVFoundation, etc.)
//!
//! When porting to other platforms (iOS, macOS native, Web), this module
//! can be compiled as a library and accessed via FFI or as a Rust dependency.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        PlexService                              │
//! │  High-level facade combining client + cache + preloading        │
//! └────────────────────────────┬────────────────────────────────────┘
//!                              │
//!        ┌─────────────────────┼─────────────────────┐
//!        ▼                     ▼                     ▼
//! ┌─────────────┐      ┌─────────────┐       ┌─────────────┐
//! │ PlexClient  │      │ LibraryCache│       │WaveformCache│
//! │ (HTTP API)  │      │ (disk cache)│       │ (disk cache)│
//! └─────────────┘      └─────────────┘       └─────────────┘
//! ```

mod artwork_cache;
mod auth;
mod cache;
mod client;
pub mod constants;
mod error;
pub mod models;
pub mod remote;
mod spectrogram;
mod waveform;

pub use artwork_cache::ArtworkCache;
pub use auth::{AccountMarker, PlexAuth, PlexClientInfo, ServerInfo, StoredAuth};
pub use cache::{CacheData, CachedFolder, CachedPlaylistTracks, LibraryCache};
pub use client::{PlexClient, test_connection};
pub use error::ApiError;
pub use remote::RemotePlayerClient;
pub use spectrogram::{SpectrogramCache, SpectrogramData, generate_spectrogram, generate_spectrogram_from_pcm};
pub use waveform::{
    WaveformCache, WaveformData, WaveformError, decode_to_pcm, generate_waveform,
    generate_waveform_from_pcm,
};

use models::{Album, Artist, Genre, Playlist, Station, Track};
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Default cache expiration for library data (72 hours).
const LIBRARY_CACHE_TTL_SECS: u64 = 72 * 60 * 60;

/// Default cache expiration for waveforms (7 days).
/// User indicated they don't replay songs often, so waveforms can expire faster.
const WAVEFORM_CACHE_TTL_SECS: u64 = 7 * 24 * 60 * 60;

/// Maximum waveform cache size in bytes (100 MB).
const WAVEFORM_CACHE_MAX_BYTES: u64 = 100 * 1024 * 1024;
const MAX_WAVEFORM_SOURCE_BYTES: usize = 128 * 1024 * 1024;

/// Unified Plex service combining API client with caching.
///
/// This is the primary interface for Plex data access. It handles:
/// - Transparent caching of library data
/// - Cache-first loading for fast startup
/// - Background refresh of stale data
/// - Waveform generation and caching
pub struct PlexService {
    client: PlexClient,
    library_cache: LibraryCache,
    waveform_cache: WaveformCache,
    /// Cached library data (in-memory for fast access).
    cache_data: Arc<RwLock<Option<CacheData>>>,
}

impl PlexService {
    /// Create a new PlexService with default cache locations.
    pub fn new(client_info: PlexClientInfo) -> Result<Self, ApiError> {
        let client = PlexClient::new(client_info)?;
        let library_cache = LibraryCache::default();
        let waveform_cache = WaveformCache::default();

        Ok(Self {
            client,
            library_cache,
            waveform_cache,
            cache_data: Arc::new(RwLock::new(None)),
        })
    }

    /// Create a PlexService with a pre-configured client.
    pub fn with_client(client: PlexClient) -> Self {
        let library_cache = LibraryCache::default();
        let waveform_cache = WaveformCache::default();

        Self {
            client,
            library_cache,
            waveform_cache,
            cache_data: Arc::new(RwLock::new(None)),
        }
    }

    /// Get the underlying PlexClient for direct API access.
    pub fn client(&self) -> &PlexClient {
        &self.client
    }

    /// Get mutable access to the underlying PlexClient.
    pub fn client_mut(&mut self) -> &mut PlexClient {
        &mut self.client
    }

    /// Get the library cache.
    pub fn library_cache(&self) -> &LibraryCache {
        &self.library_cache
    }

    /// Get the waveform cache.
    pub fn waveform_cache(&self) -> &WaveformCache {
        &self.waveform_cache
    }

    fn server_cache_scope(&self) -> Option<String> {
        self.client.server_url().map(str::to_owned)
    }

    // ========================================================================
    // Cache Management
    // ========================================================================

    /// Load cached data for a library.
    /// Returns the cached data if available and not expired.
    pub async fn load_cache(&self, library_key: &str) -> Option<CacheData> {
        let server_scope = self.server_cache_scope()?;
        let cache = self.library_cache.clone();
        let cache_library_key = library_key.to_string();
        let cache_server_scope = server_scope.clone();
        let data = match tokio::task::spawn_blocking(move || {
            cache.load_scoped(Some(&cache_server_scope), &cache_library_key)
        })
        .await
        {
            Ok(data) => data?,
            Err(error) => {
                tracing::warn!("Library cache worker failed: {}", error);
                return None;
            }
        };

        // Check if cache is expired
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        if now.saturating_sub(data.timestamp) > LIBRARY_CACHE_TTL_SECS {
            tracing::info!("Library cache expired, will refresh");
            // Still return stale data for fast startup - caller can refresh
        }

        // Store in memory for fast access
        {
            let mut data_lock = self.cache_data.write().await;
            *data_lock = Some(data.clone());
        }

        Some(data)
    }

    /// Save cache data to disk.
    pub async fn save_cache(&self, data: &CacheData) -> bool {
        let Some(server_scope) = self.server_cache_scope() else {
            return false;
        };
        let mut snapshot = data.clone();
        snapshot.server_id = Some(server_scope);

        // Update in-memory cache
        {
            let mut data_lock = self.cache_data.write().await;
            *data_lock = Some(snapshot.clone());
        }

        let cache = self.library_cache.clone();
        match tokio::task::spawn_blocking(move || cache.save(&snapshot)).await {
            Ok(saved) => saved,
            Err(error) => {
                tracing::warn!("Library cache worker failed: {}", error);
                false
            }
        }
    }

    /// Update specific fields in the cache without full reload.
    pub async fn update_cache<F>(&self, library_key: &str, updater: F) -> bool
    where
        F: FnOnce(&mut CacheData),
    {
        let Some(server_scope) = self.server_cache_scope() else {
            return false;
        };
        let snapshot = {
            let mut data_lock = self.cache_data.write().await;

            if let Some(ref mut data) = *data_lock {
                if data.library_key == library_key
                    && data.server_id.as_deref() == Some(server_scope.as_str())
                {
                    updater(data);
                    data.touch();
                    Some(data.clone())
                } else {
                    None
                }
            } else {
                None
            }
        };

        let Some(snapshot) = snapshot else {
            return false;
        };
        let cache = self.library_cache.clone();
        match tokio::task::spawn_blocking(move || cache.save(&snapshot)).await {
            Ok(saved) => saved,
            Err(error) => {
                tracing::warn!("Library cache worker failed: {}", error);
                false
            }
        }
    }

    // ========================================================================
    // Waveform Management
    // ========================================================================

    /// Load waveform from cache.
    pub fn load_waveform(&self, track_key: &str) -> Option<WaveformData> {
        self.server_cache_scope()
            .and_then(|scope| self.waveform_cache.load_scoped(&scope, track_key))
    }

    /// Save waveform to cache with expiration check.
    pub fn save_waveform(&self, data: &WaveformData) -> bool {
        let Some(scope) = self.server_cache_scope() else {
            return false;
        };
        // First, check if we need to prune old waveforms
        self.waveform_cache.prune_expired(WAVEFORM_CACHE_TTL_SECS);
        self.waveform_cache.prune_to_size(WAVEFORM_CACHE_MAX_BYTES);

        self.waveform_cache.save_scoped(&scope, data)
    }

    /// Generate and cache waveform for a track.
    pub async fn generate_and_cache_waveform(
        &self,
        track: &Track,
    ) -> Result<WaveformData, WaveformError> {
        // Check cache first
        let cache_scope = self.server_cache_scope().ok_or_else(|| {
            WaveformError::Download("no Plex server selected".to_string())
        })?;
        let cache = self.waveform_cache.clone();
        let cache_key = track.rating_key.clone();
        let read_scope = cache_scope.clone();
        let cached = tokio::task::spawn_blocking(move || {
            cache.load_scoped(&read_scope, &cache_key)
        })
        .await
        .map_err(|error| {
            WaveformError::Decode(format!("waveform cache worker failed: {}", error))
        })?;
        if let Some(data) = cached {
            return Ok(data);
        }

        // Get stream URL
        let url = self.client.get_stream_url(track)
            .map_err(|e| WaveformError::Download(e.to_string()))?;

        // Download audio through the shared pool with header-based auth. The
        // direct URL intentionally contains no credential query parameter.
        let response = self.client.http_client()
            .get(&url)
            .headers(self.client.stream_headers())
            .send()
            .await
            .map_err(|e| WaveformError::Download(e.to_string()))?;
        if !response.status().is_success() {
            return Err(WaveformError::Download(format!("HTTP {}", response.status())));
        }
        if response.content_length().is_some_and(|length| {
            length > MAX_WAVEFORM_SOURCE_BYTES as u64
        }) {
            return Err(WaveformError::Download("audio exceeds 128 MiB limit".to_string()));
        }
        let mut audio_data = Vec::with_capacity(
            response.content_length()
                .and_then(|length| usize::try_from(length).ok())
                .unwrap_or(256 * 1024)
                .min(MAX_WAVEFORM_SOURCE_BYTES)
        );
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| WaveformError::Download(error.to_string()))?;
            if audio_data.len().saturating_add(chunk.len()) > MAX_WAVEFORM_SOURCE_BYTES {
                return Err(WaveformError::Download("audio exceeds 128 MiB limit".to_string()));
            }
            audio_data.extend_from_slice(&chunk);
        }

        // Decoding and cache I/O are blocking/CPU-heavy and must not occupy a
        // Tokio worker used by API and UI-completion tasks.
        let track_key = track.rating_key.clone();
        let duration_ms = track.duration_ms();
        let waveform_cache = self.waveform_cache.clone();
        let waveform = tokio::task::spawn_blocking(move || {
            let waveform = generate_waveform(track_key, duration_ms, Arc::from(audio_data))?;
            waveform_cache.save_scoped(&cache_scope, &waveform);
            Ok::<_, WaveformError>(waveform)
        }).await
            .map_err(|error| WaveformError::Decode(format!("waveform worker failed: {}", error)))??;

        Ok(waveform)
    }

    // ========================================================================
    // Library Data Access (Cache-First)
    // ========================================================================

    /// Get artists with cache-first strategy.
    /// Returns cached data immediately, then optionally refreshes.
    pub async fn get_artists_cached(&self, library_key: &str) -> Vec<Artist> {
        let server_scope = self.server_cache_scope();
        // Try cache first
        {
            let data_lock = self.cache_data.read().await;
            if let Some(ref data) = *data_lock {
                if data.library_key == library_key
                    && data.server_id.as_deref() == server_scope.as_deref()
                    && !data.artists.is_empty()
                {
                    return data.artists.clone();
                }
            }
        }

        // Fallback to API
        match self.client.get_artists(library_key).await {
            Ok(artists) => {
                // Update cache
                let _ = self.update_cache(library_key, |data| {
                    data.artists = artists.clone();
                }).await;
                artists
            }
            Err(e) => {
                tracing::error!("Failed to fetch artists: {}", e);
                Vec::new()
            }
        }
    }

    /// Get playlists with cache-first strategy.
    pub async fn get_playlists_cached(&self, section_id: Option<&str>) -> Vec<Playlist> {
        let server_scope = self.server_cache_scope();
        // Try cache first
        {
            let data_lock = self.cache_data.read().await;
            if let Some(ref data) = *data_lock {
                if data.server_id.as_deref() == server_scope.as_deref()
                    && !data.playlists.is_empty()
                {
                    return data.playlists.clone();
                }
            }
        }

        // Fallback to API
        match self.client.get_playlists(section_id).await {
            Ok(playlists) => {
                // Update cache
                let snapshot = {
                    let mut data_lock = self.cache_data.write().await;
                    if let Some(ref mut data) = *data_lock {
                        if data.server_id.as_deref() == server_scope.as_deref() {
                            data.playlists = playlists.clone();
                            data.touch();
                            Some(data.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                if let Some(snapshot) = snapshot {
                    let cache = self.library_cache.clone();
                    let _ = tokio::task::spawn_blocking(move || cache.save(&snapshot)).await;
                }
                playlists
            }
            Err(e) => {
                tracing::error!("Failed to fetch playlists: {}", e);
                Vec::new()
            }
        }
    }

    /// Get genres with cache-first strategy.
    pub async fn get_genres_cached(&self, library_key: &str) -> Vec<Genre> {
        let server_scope = self.server_cache_scope();
        // Try cache first
        {
            let data_lock = self.cache_data.read().await;
            if let Some(ref data) = *data_lock {
                if data.library_key == library_key
                    && data.server_id.as_deref() == server_scope.as_deref()
                    && !data.genres.is_empty()
                {
                    return data.genres.clone();
                }
            }
        }

        // Fallback to API
        match self.client.get_genres(library_key).await {
            Ok(genres) => {
                // Update cache
                let _ = self.update_cache(library_key, |data| {
                    data.genres = genres.clone();
                }).await;
                genres
            }
            Err(e) => {
                tracing::error!("Failed to fetch genres: {}", e);
                Vec::new()
            }
        }
    }

    /// Get stations with cache-first strategy.
    pub async fn get_stations_cached(&self, library_key: &str) -> Vec<Station> {
        let server_scope = self.server_cache_scope();
        // Try cache first
        {
            let data_lock = self.cache_data.read().await;
            if let Some(ref data) = *data_lock {
                if data.library_key == library_key
                    && data.server_id.as_deref() == server_scope.as_deref()
                    && !data.stations.is_empty()
                {
                    return data.stations.clone();
                }
            }
        }

        // Fallback to API
        match self.client.get_stations(library_key).await {
            Ok(stations) => {
                // Update cache
                let _ = self.update_cache(library_key, |data| {
                    data.stations = stations.clone();
                }).await;
                stations
            }
            Err(e) => {
                tracing::error!("Failed to fetch stations: {}", e);
                Vec::new()
            }
        }
    }

    // ========================================================================
    // Direct API Passthrough
    // ========================================================================

    // These methods pass through to the client without caching,
    // for operations that need fresh data or don't benefit from caching.

    /// Get tracks for an album (fresh from API).
    pub async fn get_album_tracks(&self, album_key: &str) -> Result<Vec<Track>, ApiError> {
        self.client.get_album_tracks(album_key).await
    }

    /// Get tracks for a playlist (fresh from API).
    pub async fn get_playlist_tracks(&self, playlist_key: &str) -> Result<Vec<Track>, ApiError> {
        self.client.get_playlist_tracks(playlist_key).await
    }

    /// Search library (fresh from API).
    pub async fn search(&self, query: &str) -> Result<models::SearchResults, ApiError> {
        self.client.search(query).await
    }

    /// Get similar tracks (fresh from API).
    pub async fn get_similar_tracks(&self, rating_key: &str, limit: u32) -> Result<Vec<Track>, ApiError> {
        self.client.get_similar_tracks(rating_key, limit).await
    }

    /// Get similar albums (fresh from API).
    pub async fn get_similar_albums(&self, rating_key: &str, limit: u32) -> Result<Vec<Album>, ApiError> {
        self.client.get_similar_albums(rating_key, limit).await
    }

    /// Get stream URL for a track.
    pub fn get_stream_url(&self, track: &Track) -> Result<String, ApiError> {
        self.client.get_stream_url(track)
    }
}
