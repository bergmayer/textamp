//! Data caching for faster startup.
//!
//! Caches library data to disk so the app can display content immediately
//! on startup, then refresh from API in background.
//!
//! IMPORTANT: Cache writes should happen once (on quit or periodically),
//! not from background tasks, to avoid file contention.
//!
//! # Subfolder Caching
//!
//! Subfolders have different caching behavior than other library data:
//! - **Lazy caching**: Only cached when navigated to (not preloaded)
//! - **No auto-refresh**: Stale subfolders are NOT automatically refreshed
//! - **Manual refresh**: F5 refreshes the currently focused subfolder
//! - **Warm cache (32+ days)**: Entries older than 32 days are served from cache
//!   but re-fetched in background on access
//!
//! This design provides fast navigation for frequently-accessed folders
//! while keeping data reasonably fresh.

use super::models::{Album, Artist, FolderItem, Genre, Playlist, Station, Track};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Cached subfolder with timestamp for staleness tracking.
///
/// Each subfolder is cached individually with its own timestamp,
/// allowing fine-grained staleness control. Subfolders older than
/// 32 days are served from cache but re-fetched in background on access.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedFolder {
    /// The folder's contents (subfolders and tracks).
    pub items: Vec<FolderItem>,
    /// Unix timestamp when this folder was cached.
    pub timestamp: u64,
    /// Filesystem path of this folder (for column headers).
    #[serde(default)]
    pub path: Option<String>,
}

impl CachedFolder {
    /// Create a new cached folder with current timestamp.
    pub fn new(items: Vec<FolderItem>) -> Self {
        Self {
            items,
            timestamp: current_timestamp(),
            path: None,
        }
    }

    /// Create a new cached folder with path and current timestamp.
    pub fn with_path(items: Vec<FolderItem>, path: Option<String>) -> Self {
        Self {
            items,
            timestamp: current_timestamp(),
            path,
        }
    }

    /// Check if this folder cache is older than the given threshold (in seconds).
    pub fn is_older_than(&self, threshold_secs: u64) -> bool {
        let now = current_timestamp();
        now.saturating_sub(self.timestamp) > threshold_secs
    }
}

/// Cached playlist tracks with timestamp for staleness tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedPlaylistTracks {
    pub tracks: Vec<Track>,
    pub timestamp: u64,
}

impl CachedPlaylistTracks {
    /// Create a new cached playlist tracks entry with current timestamp.
    pub fn new(tracks: Vec<Track>) -> Self {
        Self {
            tracks,
            timestamp: current_timestamp(),
        }
    }

    /// Check if this cache entry is older than the given threshold (in seconds).
    pub fn is_older_than(&self, threshold_secs: u64) -> bool {
        let now = current_timestamp();
        now.saturating_sub(self.timestamp) > threshold_secs
    }
}

/// Cache data structure with timestamp.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CacheData {
    #[serde(default)]
    pub timestamp: u64,
    #[serde(default)]
    pub playlist_timestamp: u64,
    #[serde(default)]
    pub library_key: String,

    // Core library data
    #[serde(default)]
    pub artists: Vec<Artist>,
    #[serde(default)]
    pub albums: Vec<Album>,
    #[serde(default)]
    pub playlists: Vec<Playlist>,

    // Folder data
    #[serde(default)]
    pub root_folders: Vec<FolderItem>,
    /// Cached subfolder contents: folder_key -> CachedFolder with timestamp.
    /// Each entry has its own timestamp for individual staleness tracking.
    /// Entries older than 32 days are served from cache but re-fetched on access.
    #[serde(default)]
    pub folder_contents: HashMap<String, CachedFolder>,

    // Genre/mood/style data
    #[serde(default)]
    pub genres: Vec<Genre>,
    #[serde(default, alias = "normalized_genres")]
    pub artist_genres: Vec<Genre>,
    #[serde(default)]
    pub album_genres: Vec<Genre>,
    #[serde(default)]
    pub moods: Vec<Genre>,
    #[serde(default)]
    pub styles: Vec<Genre>,

    // Playlist tracks (per-playlist, excludes smart playlists)
    #[serde(default)]
    pub playlist_tracks: HashMap<String, CachedPlaylistTracks>,

    // Stations
    #[serde(default)]
    pub stations: Vec<Station>,

    // Recent content (from hubs)
    #[serde(default)]
    pub recently_added_albums: Vec<Album>,
    // Per-category refresh timestamps (category display_name -> epoch secs)
    #[serde(default)]
    pub category_timestamps: HashMap<String, u64>,
}

impl CacheData {
    /// Create a new cache data structure with current timestamp.
    pub fn new(library_key: &str) -> Self {
        Self {
            timestamp: current_timestamp(),
            library_key: library_key.to_string(),
            ..Default::default()
        }
    }

    /// Create a new cache data structure preserving an existing timestamp.
    ///
    /// Use this when re-saving existing data to disk so the timestamp
    /// reflects when the data was last refreshed from the server,
    /// not when the cache file was last written.
    pub fn with_timestamp(library_key: &str, timestamp: u64) -> Self {
        Self {
            timestamp,
            library_key: library_key.to_string(),
            ..Default::default()
        }
    }

    /// Update the timestamp to now.
    pub fn touch(&mut self) {
        self.timestamp = current_timestamp();
    }

    /// Get the current Unix timestamp (public for use by event handlers).
    pub fn now() -> u64 {
        current_timestamp()
    }
}

/// Library data cache manager.
pub struct LibraryCache {
    cache_dir: PathBuf,
}

impl LibraryCache {
    /// Create a new cache manager.
    pub fn new() -> Option<Self> {
        let cache_dir = get_cache_dir()?;

        // Ensure cache directory exists
        if !cache_dir.exists() {
            if let Err(e) = fs::create_dir_all(&cache_dir) {
                tracing::warn!("Failed to create cache directory: {}", e);
                return None;
            }
        }

        Some(Self { cache_dir })
    }

    /// Get the cache file path for a library.
    pub fn cache_path(&self, library_key: &str) -> PathBuf {
        self.cache_dir.join(format!("library_{}.json", library_key))
    }

    /// Load cache data from disk.
    pub fn load(&self, library_key: &str) -> Option<CacheData> {
        let path = self.cache_path(library_key);

        if !path.exists() {
            tracing::debug!("No cache file found: {:?}", path);
            return None;
        }

        match fs::read_to_string(&path) {
            Ok(contents) => {
                match serde_json::from_str::<CacheData>(&contents) {
                    Ok(data) => {
                        tracing::info!(
                            "Loaded cache: {} artists, {} albums, {} root folders, {} cached subfolders, {} playlist track lists",
                            data.artists.len(),
                            data.albums.len(),
                            data.root_folders.len(),
                            data.folder_contents.len(),
                            data.playlist_tracks.len()
                        );
                        Some(data)
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse cache file: {}", e);
                        // Delete corrupted cache
                        let _ = fs::remove_file(&path);
                        None
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to read cache file: {}", e);
                None
            }
        }
    }

    /// Save complete cache data to disk (call once, not per-field).
    pub fn save(&self, data: &CacheData) -> bool {
        let path = self.cache_path(&data.library_key);

        match serde_json::to_string(data) {
            Ok(contents) => {
                // Write atomically
                let temp_path = path.with_extension("json.tmp");
                if let Err(e) = fs::write(&temp_path, &contents) {
                    tracing::warn!("Failed to write cache temp file: {}", e);
                    return false;
                }
                if let Err(e) = fs::rename(&temp_path, &path) {
                    tracing::warn!("Failed to rename cache file: {}", e);
                    let _ = fs::remove_file(&temp_path);
                    return false;
                }
                tracing::debug!("Cache saved: {:?}", path);
                true
            }
            Err(e) => {
                tracing::warn!("Failed to serialize cache: {}", e);
                false
            }
        }
    }

    /// Clear all cache files.
    pub fn clear_all(&self) -> Result<usize, std::io::Error> {
        let mut count = 0;

        if self.cache_dir.exists() {
            for entry in fs::read_dir(&self.cache_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() && path.extension().map_or(false, |e| e == "json") {
                    if fs::remove_file(&path).is_ok() {
                        tracing::info!("Removed cache file: {:?}", path);
                        count += 1;
                    }
                }
            }
        }

        Ok(count)
    }

    /// Get total cache size in bytes.
    pub fn total_size(&self) -> u64 {
        if !self.cache_dir.exists() {
            return 0;
        }

        let mut total = 0u64;
        if let Ok(entries) = fs::read_dir(&self.cache_dir) {
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

impl Default for LibraryCache {
    fn default() -> Self {
        Self::new().unwrap_or_else(|| Self {
            cache_dir: PathBuf::from("/tmp/textamp_cache"),
        })
    }
}

/// Get the cache directory path using shared utility.
fn get_cache_dir() -> Option<PathBuf> {
    crate::util::paths::get_cache_dir("textamp")
}

/// Get the current Unix timestamp.
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
