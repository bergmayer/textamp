//! Data caching for faster startup.
//!
//! Caches library data to disk so the app can display content immediately
//! on startup, then refresh from API in background.
//!
//! IMPORTANT: Cache writes should happen once (on quit or periodically),
//! not from background tasks, to avoid file contention.

use super::models::{Album, Artist, Genre, Playlist, Station};
use crate::services::FolderItem;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Cache data structure with timestamp.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CacheData {
    #[serde(default)]
    pub timestamp: u64,
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

    // Stations
    #[serde(default)]
    pub stations: Vec<Station>,

    // Recent content (from hubs)
    #[serde(default)]
    pub recently_added_albums: Vec<Album>,
    #[serde(default)]
    pub recently_played_albums: Vec<Album>,
    #[serde(default)]
    pub recent_playlists: Vec<Playlist>,
}

impl CacheData {
    /// Create a new cache data structure.
    pub fn new(library_key: &str) -> Self {
        Self {
            timestamp: current_timestamp(),
            library_key: library_key.to_string(),
            ..Default::default()
        }
    }

    /// Update the timestamp to now.
    pub fn touch(&mut self) {
        self.timestamp = current_timestamp();
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
                            "Loaded cache: {} artists, {} albums, {} folders",
                            data.artists.len(),
                            data.albums.len(),
                            data.root_folders.len()
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

/// Get the cache directory path.
///
/// Checks $XDG_CACHE_HOME first (on macOS and Linux), then falls back to platform defaults.
fn get_cache_dir() -> Option<PathBuf> {
    // Check XDG env var first (works on both macOS and Linux)
    if let Ok(xdg_cache) = std::env::var("XDG_CACHE_HOME") {
        return Some(PathBuf::from(xdg_cache).join("textamp"));
    }

    // Fall back to platform default
    #[cfg(target_os = "linux")]
    {
        dirs::home_dir().map(|h| h.join(".cache/textamp"))
    }

    #[cfg(target_os = "macos")]
    {
        dirs::cache_dir().map(|p| p.join("textamp"))
    }

    #[cfg(target_os = "windows")]
    {
        dirs::cache_dir().map(|p| p.join("textamp"))
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        dirs::cache_dir().map(|p| p.join("textamp"))
    }
}

/// Get the current Unix timestamp.
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
