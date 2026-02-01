//! Cache service for library data caching.
//!
//! Provides utilities for building cache data from application state
//! and determining when cache saves should occur.
//!
//! # Cross-Platform Design
//!
//! This service is UI-agnostic and helps manage cache operations
//! without coupling to the event loop or terminal-specific code.

use crate::plex::{CacheData, LibraryCache};
use crate::plex::models::{Album, Artist, Genre, Playlist, Station};
use crate::services::FolderItem;
use std::time::{Duration, Instant};

/// Default idle time before cache save (30 seconds).
pub const CACHE_IDLE_THRESHOLD_SECS: u64 = 30;

/// Default minimum interval between cache saves (2 minutes).
pub const CACHE_SAVE_INTERVAL_SECS: u64 = 120;

/// Parameters needed to decide if cache should be saved.
#[derive(Debug, Clone)]
pub struct CacheSaveConditions {
    /// Whether the cache has unsaved changes.
    pub is_dirty: bool,
    /// Whether a save operation is already in progress.
    pub save_in_progress: bool,
    /// Time of last user input.
    pub last_input_time: Instant,
    /// Time of last cache save.
    pub last_cache_save: Instant,
}

impl CacheSaveConditions {
    /// Check if all conditions for saving cache are met.
    ///
    /// Conditions:
    /// - Cache is dirty (has unsaved changes)
    /// - No save already in progress
    /// - User has been idle for `idle_threshold`
    /// - At least `save_interval` has passed since last save
    pub fn should_save(&self, idle_threshold: Duration, save_interval: Duration) -> bool {
        if !self.is_dirty || self.save_in_progress {
            return false;
        }

        let idle_enough = self.last_input_time.elapsed() >= idle_threshold;
        let interval_passed = self.last_cache_save.elapsed() >= save_interval;

        idle_enough && interval_passed
    }

    /// Check with default thresholds.
    pub fn should_save_default(&self) -> bool {
        self.should_save(
            Duration::from_secs(CACHE_IDLE_THRESHOLD_SECS),
            Duration::from_secs(CACHE_SAVE_INTERVAL_SECS),
        )
    }
}

/// Data sources for building a cache snapshot.
pub struct CacheDataSources<'a> {
    pub artists: &'a [Artist],
    pub albums: &'a [Album],
    pub playlists: &'a [Playlist],
    pub root_folders: Option<&'a [FolderItem]>,
    pub genres: &'a [Genre],
    pub artist_genres: &'a [Genre],
    pub album_genres: &'a [Genre],
    pub moods: &'a [Genre],
    pub styles: &'a [Genre],
    pub stations: &'a [Station],
    pub recently_added_albums: &'a [Album],
    pub recently_played_albums: &'a [Album],
    pub recent_playlists: &'a [Playlist],
}

/// Service for cache operations.
pub struct CacheService;

impl CacheService {
    /// Build a CacheData structure from data sources.
    ///
    /// This consolidates all the data needed for a cache save into
    /// a single CacheData instance ready for serialization.
    pub fn build_cache_data(library_key: &str, sources: &CacheDataSources) -> CacheData {
        let mut cache_data = CacheData::new(library_key);

        cache_data.artists = sources.artists.to_vec();
        cache_data.albums = sources.albums.to_vec();
        cache_data.playlists = sources.playlists.to_vec();

        if let Some(folders) = sources.root_folders {
            cache_data.root_folders = folders.to_vec();
        }

        cache_data.genres = sources.genres.to_vec();
        cache_data.artist_genres = sources.artist_genres.to_vec();
        cache_data.album_genres = sources.album_genres.to_vec();
        cache_data.moods = sources.moods.to_vec();
        cache_data.styles = sources.styles.to_vec();
        cache_data.stations = sources.stations.to_vec();
        cache_data.recently_added_albums = sources.recently_added_albums.to_vec();
        cache_data.recently_played_albums = sources.recently_played_albums.to_vec();
        cache_data.recent_playlists = sources.recent_playlists.to_vec();

        cache_data
    }

    /// Check if the cache is stale (older than TTL).
    ///
    /// Returns true if cache should be refreshed from API.
    pub fn is_cache_stale(cache: &CacheData, ttl_secs: u64) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        cache.timestamp + ttl_secs < now
    }

    /// Save cache data synchronously.
    ///
    /// For async saving, use `build_cache_data` and save in a tokio task.
    pub fn save_sync(cache_data: &CacheData) -> bool {
        if let Some(cache) = LibraryCache::new() {
            cache.save(cache_data)
        } else {
            false
        }
    }

    /// Load cache data for a library.
    pub fn load(library_key: &str) -> Option<CacheData> {
        let cache = LibraryCache::new()?;
        cache.load(library_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_save_conditions_dirty() {
        let now = Instant::now();
        let conditions = CacheSaveConditions {
            is_dirty: false,
            save_in_progress: false,
            last_input_time: now - Duration::from_secs(60),
            last_cache_save: now - Duration::from_secs(180),
        };

        // Not dirty, so shouldn't save
        assert!(!conditions.should_save_default());
    }

    #[test]
    fn test_cache_save_conditions_in_progress() {
        let now = Instant::now();
        let conditions = CacheSaveConditions {
            is_dirty: true,
            save_in_progress: true,
            last_input_time: now - Duration::from_secs(60),
            last_cache_save: now - Duration::from_secs(180),
        };

        // Save in progress, so shouldn't save
        assert!(!conditions.should_save_default());
    }

    #[test]
    fn test_cache_save_conditions_not_idle_enough() {
        let now = Instant::now();
        let conditions = CacheSaveConditions {
            is_dirty: true,
            save_in_progress: false,
            last_input_time: now - Duration::from_secs(10), // Only 10 seconds idle
            last_cache_save: now - Duration::from_secs(180),
        };

        // Not idle enough (need 30 seconds)
        assert!(!conditions.should_save_default());
    }

    #[test]
    fn test_cache_save_conditions_too_soon() {
        let now = Instant::now();
        let conditions = CacheSaveConditions {
            is_dirty: true,
            save_in_progress: false,
            last_input_time: now - Duration::from_secs(60),
            last_cache_save: now - Duration::from_secs(60), // Only 60 seconds since last save
        };

        // Too soon since last save (need 120 seconds)
        assert!(!conditions.should_save_default());
    }

    #[test]
    fn test_cache_save_conditions_all_met() {
        let now = Instant::now();
        let conditions = CacheSaveConditions {
            is_dirty: true,
            save_in_progress: false,
            last_input_time: now - Duration::from_secs(60),  // 60 seconds idle
            last_cache_save: now - Duration::from_secs(180), // 3 minutes since last save
        };

        // All conditions met
        assert!(conditions.should_save_default());
    }

    #[test]
    fn test_is_cache_stale() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let fresh_cache = CacheData {
            timestamp: now - 1000, // 1000 seconds ago
            ..Default::default()
        };

        let stale_cache = CacheData {
            timestamp: now - 100000, // 100000 seconds ago
            ..Default::default()
        };

        // TTL of 24 hours = 86400 seconds
        assert!(!CacheService::is_cache_stale(&fresh_cache, 86400));
        assert!(CacheService::is_cache_stale(&stale_cache, 86400));
    }
}
