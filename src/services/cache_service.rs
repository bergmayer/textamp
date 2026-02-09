//! Cache service for library data caching.
//!
//! Provides utilities for building cache data from application state
//! and determining when cache saves should occur.
//!
//! # Cross-Platform Design
//!
//! This service is UI-agnostic and helps manage cache operations
//! without coupling to the event loop or terminal-specific code.
//!
//! # Subfolder Caching
//!
//! Subfolders have different caching behavior than other library data:
//! - **Lazy caching**: Only cached when navigated to (not preloaded on startup)
//! - **No auto-refresh**: Stale subfolders are NOT automatically refreshed
//! - **Manual refresh**: F5 refreshes the currently focused subfolder
//! - **Warm cache (32+ days)**: Very stale entries are served from cache but
//!   re-fetched in background when accessed ("warm cache" behavior)
//! - **Manual crawl**: Users can start a subfolder crawl from Settings > Libraries

use crate::plex::{CacheData, CachedFolder, LibraryCache};
use crate::plex::constants::CACHE_VERY_STALE_THRESHOLD_SECS;
use crate::plex::models::{Album, Artist, FolderItem, Genre, Playlist, Station};
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Default idle time before cache save (30 seconds).
pub const CACHE_IDLE_THRESHOLD_SECS: u64 = 30;

/// Default minimum interval between cache saves (2 minutes).
pub const CACHE_SAVE_INTERVAL_SECS: u64 = 120;

/// Get the current Unix timestamp in seconds.
pub fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

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
    /// Cached subfolder contents: folder_key -> CachedFolder with timestamp.
    pub folder_contents: Option<&'a HashMap<String, CachedFolder>>,
    pub genres: &'a [Genre],
    pub artist_genres: &'a [Genre],
    pub album_genres: &'a [Genre],
    pub moods: &'a [Genre],
    pub styles: &'a [Genre],
    pub stations: &'a [Station],
    pub recently_added_albums: &'a [Album],
    pub recently_played_albums: &'a [Album],
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

        if let Some(folder_contents) = sources.folder_contents {
            cache_data.folder_contents = folder_contents.clone();
        }

        cache_data.genres = sources.genres.to_vec();
        cache_data.artist_genres = sources.artist_genres.to_vec();
        cache_data.album_genres = sources.album_genres.to_vec();
        cache_data.moods = sources.moods.to_vec();
        cache_data.styles = sources.styles.to_vec();
        cache_data.stations = sources.stations.to_vec();
        cache_data.recently_added_albums = sources.recently_added_albums.to_vec();
        cache_data.recently_played_albums = sources.recently_played_albums.to_vec();

        cache_data
    }

    /// Check if the cache is stale (older than TTL).
    ///
    /// Returns true if cache should be refreshed from API.
    pub fn is_cache_stale(cache: &CacheData, ttl_secs: u64) -> bool {
        let now = current_timestamp();
        cache.timestamp + ttl_secs < now
    }

    /// Filter out very stale subfolder cache entries.
    ///
    /// Entries older than `very_stale_threshold_secs` (default 32 days) are removed.
    /// This is different from other caches which get refreshed - subfolders are
    /// deleted to prevent accumulation of stale folder data.
    ///
    /// Returns the number of entries that were removed.
    pub fn filter_stale_subfolders(
        folder_contents: &mut HashMap<String, CachedFolder>,
        very_stale_threshold_secs: u64,
    ) -> usize {
        let now = current_timestamp();
        let initial_count = folder_contents.len();

        folder_contents.retain(|key, cached| {
            let age_secs = now.saturating_sub(cached.timestamp);
            let keep = age_secs < very_stale_threshold_secs;
            if !keep {
                tracing::debug!(
                    "Removing very stale subfolder cache: {} (age: {} days)",
                    key,
                    age_secs / (24 * 60 * 60)
                );
            }
            keep
        });

        initial_count - folder_contents.len()
    }

    /// Filter stale subfolders using the default threshold (32 days).
    pub fn filter_stale_subfolders_default(
        folder_contents: &mut HashMap<String, CachedFolder>,
    ) -> usize {
        Self::filter_stale_subfolders(folder_contents, CACHE_VERY_STALE_THRESHOLD_SECS)
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

    /// Load cache data for a library, filtering out very stale subfolders.
    ///
    /// Note: Stale entries are no longer deleted on load. They are kept as a
    /// warm cache and re-fetched in background when accessed. This function
    /// is retained for explicit manual cleanup only.
    pub fn load_with_subfolder_filtering(library_key: &str) -> Option<(CacheData, usize)> {
        let cache = LibraryCache::new()?;
        let data = cache.load(library_key)?;

        // No longer delete stale entries on load — serve as warm cache
        Some((data, 0))
    }

    /// Determine which root folder keys need re-fetching.
    ///
    /// Returns keys that are either missing from the cache or stale (older than
    /// `stale_threshold_secs`). Fresh entries are skipped.
    pub fn keys_needing_refresh(
        root_folder_keys: &[String],
        folder_contents: &HashMap<String, CachedFolder>,
        stale_threshold_secs: u64,
    ) -> Vec<String> {
        let now = current_timestamp();
        root_folder_keys
            .iter()
            .filter(|key| {
                match folder_contents.get(key.as_str()) {
                    Some(cached) => {
                        // Stale: older than threshold
                        now.saturating_sub(cached.timestamp) >= stale_threshold_secs
                    }
                    None => true, // Missing: always needs fetch
                }
            })
            .cloned()
            .collect()
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
        let now = current_timestamp();

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

    #[test]
    fn test_filter_stale_subfolders() {
        let now = current_timestamp();
        let one_day = 24 * 60 * 60;
        let threshold = 32 * one_day;

        let mut folder_contents = HashMap::new();

        // Fresh entry (1 day old)
        folder_contents.insert(
            "fresh".to_string(),
            CachedFolder {
                items: vec![],
                timestamp: now - one_day,
            },
        );

        // Old but not very stale (20 days old)
        folder_contents.insert(
            "old".to_string(),
            CachedFolder {
                items: vec![],
                timestamp: now - (20 * one_day),
            },
        );

        // Very stale (40 days old) - should be removed
        folder_contents.insert(
            "very_stale".to_string(),
            CachedFolder {
                items: vec![],
                timestamp: now - (40 * one_day),
            },
        );

        let removed = CacheService::filter_stale_subfolders(&mut folder_contents, threshold);

        assert_eq!(removed, 1);
        assert_eq!(folder_contents.len(), 2);
        assert!(folder_contents.contains_key("fresh"));
        assert!(folder_contents.contains_key("old"));
        assert!(!folder_contents.contains_key("very_stale"));
    }

    #[test]
    fn test_keys_needing_refresh() {
        let now = current_timestamp();
        let one_day = 24 * 60 * 60;
        let threshold = 32 * one_day;

        let mut folder_contents = HashMap::new();

        // Fresh entry (1 day old) — should NOT need refresh
        folder_contents.insert(
            "fresh_key".to_string(),
            CachedFolder {
                items: vec![],
                timestamp: now - one_day,
            },
        );

        // Stale entry (40 days old) — should need refresh
        folder_contents.insert(
            "stale_key".to_string(),
            CachedFolder {
                items: vec![],
                timestamp: now - (40 * one_day),
            },
        );

        let root_keys = vec![
            "fresh_key".to_string(),
            "stale_key".to_string(),
            "missing_key".to_string(),
        ];

        let needs_refresh = CacheService::keys_needing_refresh(&root_keys, &folder_contents, threshold);

        // stale_key and missing_key need refresh, fresh_key does not
        assert_eq!(needs_refresh.len(), 2);
        assert!(needs_refresh.contains(&"stale_key".to_string()));
        assert!(needs_refresh.contains(&"missing_key".to_string()));
        assert!(!needs_refresh.contains(&"fresh_key".to_string()));
    }

    #[test]
    fn test_keys_needing_refresh_all_fresh() {
        let now = current_timestamp();
        let one_day = 24 * 60 * 60;
        let threshold = 32 * one_day;

        let mut folder_contents = HashMap::new();
        folder_contents.insert(
            "a".to_string(),
            CachedFolder { items: vec![], timestamp: now - one_day },
        );
        folder_contents.insert(
            "b".to_string(),
            CachedFolder { items: vec![], timestamp: now - (10 * one_day) },
        );

        let root_keys = vec!["a".to_string(), "b".to_string()];
        let needs_refresh = CacheService::keys_needing_refresh(&root_keys, &folder_contents, threshold);

        assert!(needs_refresh.is_empty());
    }

    #[test]
    fn test_keys_needing_refresh_all_missing() {
        let folder_contents = HashMap::new();
        let root_keys = vec!["x".to_string(), "y".to_string()];
        let threshold = 32 * 24 * 60 * 60;

        let needs_refresh = CacheService::keys_needing_refresh(&root_keys, &folder_contents, threshold);

        assert_eq!(needs_refresh.len(), 2);
    }
}
