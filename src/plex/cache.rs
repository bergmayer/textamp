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
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static LIBRARY_CACHE_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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
    /// Plex server machine/client identifier. Library section keys are only
    /// unique within a server, so this must participate in disk identity.
    #[serde(default)]
    pub server_id: Option<String>,

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
    #[serde(default)]
    pub decades: Vec<Genre>,
    #[serde(default)]
    pub years: Vec<Genre>,
    #[serde(default)]
    pub collections: Vec<Genre>,
    #[serde(default)]
    pub countries: Vec<Genre>,
    #[serde(default)]
    pub labels: Vec<Genre>,
    #[serde(default)]
    pub formats: Vec<Genre>,
    #[serde(default)]
    pub studios: Vec<Genre>,

    // Playlist tracks (per-playlist, excludes smart playlists)
    #[serde(default)]
    pub playlist_tracks: HashMap<String, CachedPlaylistTracks>,

    // Stations
    #[serde(default)]
    pub stations: Vec<Station>,

    // Station children (mood/style/decade sub-lists, keyed by station key)
    #[serde(default)]
    pub station_children: HashMap<String, Vec<Station>>,

    // All tracks (for compilation detection + track-level artist derivation)
    #[serde(default)]
    pub all_tracks: Vec<Track>,

    // Track-level artist list (derived from all_tracks)
    #[serde(default)]
    pub track_artists: Vec<Artist>,

    // Compilation detection results
    #[serde(default)]
    pub compilation_albums: Vec<Album>,
    #[serde(default)]
    pub compilation_artist_keys: HashSet<String>,
    #[serde(default)]
    pub compilation_track_artist_keys: HashSet<String>,
    #[serde(default)]
    pub artist_compilation_map: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub single_artist_compilations: HashMap<String, Vec<Album>>,

    // Artist aliases (uniform track artists differing from album artist)
    #[serde(default)]
    pub artist_aliases: HashMap<String, HashSet<String>>,
    #[serde(default)]
    pub album_display_artist: HashMap<String, String>,

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

    /// Create cache data scoped to a specific Plex server.
    pub fn new_scoped(library_key: &str, server_id: Option<&str>) -> Self {
        Self {
            timestamp: current_timestamp(),
            library_key: library_key.to_string(),
            server_id: server_id.map(str::to_owned),
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

    /// Preserve categories that were not loaded in the snapshot being saved.
    /// A missing category timestamp means the corresponding preload never
    /// completed in this library context. This prevents an early quit from
    /// replacing a populated disk cache with empty vectors.
    fn preserve_unloaded_from(&mut self, mut previous: Self) {
        macro_rules! preserve_vec {
            ($category:literal, $field:ident) => {
                if !self.category_timestamps.contains_key($category)
                    && self.$field.is_empty()
                {
                    self.$field = std::mem::take(&mut previous.$field);
                }
            };
        }

        preserve_vec!("Artists", artists);
        preserve_vec!("Albums", albums);
        preserve_vec!("Playlists", playlists);
        preserve_vec!("Artist Genres", artist_genres);
        preserve_vec!("Album Genres", album_genres);
        preserve_vec!("Album Genres", genres);
        preserve_vec!("Moods", moods);
        preserve_vec!("Styles", styles);
        preserve_vec!("Decades", decades);
        preserve_vec!("Years", years);
        preserve_vec!("Collections", collections);
        preserve_vec!("Countries", countries);
        preserve_vec!("Labels", labels);
        preserve_vec!("Formats", formats);
        preserve_vec!("Studios", studios);
        preserve_vec!("Stations", stations);

        if !self.category_timestamps.contains_key("Folders") {
            if self.root_folders.is_empty() {
                self.root_folders = std::mem::take(&mut previous.root_folders);
            }
            for (key, value) in previous.folder_contents {
                self.folder_contents.entry(key).or_insert(value);
            }
        }

        if !self.category_timestamps.contains_key("Playlists") {
            for (key, value) in previous.playlist_tracks {
                self.playlist_tracks.entry(key).or_insert(value);
            }
        }

        if !self.category_timestamps.contains_key("Stations") {
            for (key, value) in previous.station_children {
                self.station_children.entry(key).or_insert(value);
            }
        }

        if !self.category_timestamps.contains_key("All Tracks")
            && self.all_tracks.is_empty()
        {
            self.all_tracks = previous.all_tracks;
            self.track_artists = previous.track_artists;
            self.compilation_albums = previous.compilation_albums;
            self.compilation_artist_keys = previous.compilation_artist_keys;
            self.compilation_track_artist_keys = previous.compilation_track_artist_keys;
            self.artist_compilation_map = previous.artist_compilation_map;
            self.single_artist_compilations = previous.single_artist_compilations;
            self.artist_aliases = previous.artist_aliases;
            self.album_display_artist = previous.album_display_artist;
        }
    }
}

/// Library data cache manager.
#[derive(Clone)]
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
        self.cache_dir.join(format!(
            "library_{}.json",
            safe_cache_filename_component(library_key)
        ))
    }

    /// Cache path whose identity includes both server and section key.
    pub fn scoped_cache_path(&self, server_id: &str, library_key: &str) -> PathBuf {
        self.cache_dir.join(format!(
            "library_v2_{}_{}.json",
            encode_cache_component(server_id),
            encode_cache_component(library_key)
        ))
    }

    /// Load cache data from disk.
    pub fn load(&self, library_key: &str) -> Option<CacheData> {
        let path = self.cache_path(library_key);

        self.load_path(&path)
    }

    /// Load a server-scoped cache. A legacy unscoped file is accepted once
    /// for migration and is tagged with the current server before its next
    /// save.
    pub fn load_scoped(&self, server_id: Option<&str>, library_key: &str) -> Option<CacheData> {
        let Some(server_id) = server_id else {
            return self.load(library_key);
        };
        let path = self.scoped_cache_path(server_id, library_key);
        if let Some(data) = self.load_path(&path) {
            if data.server_id.as_deref() == Some(server_id)
                && data.library_key == library_key
            {
                return Some(data);
            }
            tracing::warn!("Ignoring cache whose server/library identity does not match its path");
            return None;
        }

        let mut legacy = self.load(library_key)?;
        legacy.server_id = Some(server_id.to_string());
        Some(legacy)
    }

    fn load_path(&self, path: &std::path::Path) -> Option<CacheData> {

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
        let lock = LIBRARY_CACHE_WRITE_LOCK.get_or_init(|| Mutex::new(()));
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        self.save_unlocked(data)
    }

    /// Save a possibly partial snapshot without erasing categories whose
    /// preload had not completed. The read/merge/write transaction shares the
    /// same process-wide lock as ordinary saves.
    pub fn save_preserving_unloaded(&self, mut data: CacheData) -> bool {
        let lock = LIBRARY_CACHE_WRITE_LOCK.get_or_init(|| Mutex::new(()));
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(previous) =
            self.load_scoped(data.server_id.as_deref(), &data.library_key)
        {
            data.preserve_unloaded_from(previous);
        }
        self.save_unlocked(&data)
    }

    fn save_unlocked(&self, data: &CacheData) -> bool {
        let path = match data.server_id.as_deref() {
            Some(server_id) => self.scoped_cache_path(server_id, &data.library_key),
            None => self.cache_path(&data.library_key),
        };

        match serde_json::to_string(data) {
            Ok(contents) => {
                // Write atomically
                let temp_path = path.with_extension(format!(
                    "json.{}.tmp",
                    uuid::Uuid::new_v4(),
                ));
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
        let lock = LIBRARY_CACHE_WRITE_LOCK.get_or_init(|| Mutex::new(()));
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
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

    /// Get total cache size in bytes (all libraries).
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

    /// Get cache file size for a specific library.
    pub fn library_size(&self, library_key: &str) -> u64 {
        let path = self.cache_path(library_key);
        fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
    }

    pub fn library_size_scoped(&self, server_id: Option<&str>, library_key: &str) -> u64 {
        let path = match server_id {
            Some(server_id) => self.scoped_cache_path(server_id, library_key),
            None => self.cache_path(library_key),
        };
        fs::metadata(path).map(|metadata| metadata.len()).unwrap_or(0)
    }

    /// Get per-field size breakdown for a specific library's cache.
    /// Returns vec of (field_name, bytes) sorted by size descending.
    pub fn library_breakdown(&self, library_key: &str) -> Vec<(String, u64)> {
        let Some(data) = self.load(library_key) else { return vec![] };
        Self::breakdown(data)
    }

    pub fn library_breakdown_scoped(
        &self,
        server_id: Option<&str>,
        library_key: &str,
    ) -> Vec<(String, u64)> {
        let Some(data) = self.load_scoped(server_id, library_key) else {
            return vec![];
        };
        Self::breakdown(data)
    }

    fn breakdown(data: CacheData) -> Vec<(String, u64)> {
        fn measure(val: &(impl serde::Serialize + ?Sized)) -> u64 {
            serde_json::to_string(val).map(|s| s.len() as u64).unwrap_or(0)
        }
        let mut sizes = vec![
            ("tracks".into(), measure(&data.all_tracks)),
            ("albums".into(), measure(&data.albums)),
            ("artists".into(), measure(&data.artists)),
            ("playlist tracks".into(), measure(&data.playlist_tracks)),
            ("folders".into(), measure(&data.folder_contents)),
            ("compilations".into(),
                measure(&data.compilation_albums)
                + measure(&data.artist_compilation_map)
                + measure(&data.single_artist_compilations)),
            ("genres".into(),
                measure(&data.genres) + measure(&data.artist_genres)
                + measure(&data.album_genres) + measure(&data.moods)
                + measure(&data.styles) + measure(&data.decades)
                + measure(&data.years) + measure(&data.collections)
                + measure(&data.countries) + measure(&data.labels)
                + measure(&data.formats) + measure(&data.studios)),
            ("stations".into(),
                measure(&data.stations) + measure(&data.station_children)),
        ];
        sizes.sort_by(|a, b| b.1.cmp(&a.1));
        sizes
    }
}

fn encode_cache_component(value: &str) -> String {
    use std::fmt::Write;

    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn safe_cache_filename_component(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        value.to_string()
    } else {
        format!("h{}", encode_cache_component(value))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn track(key: &str) -> Track {
        Track {
            rating_key: key.to_string(),
            title: key.to_string(),
            ..Track::default()
        }
    }

    #[test]
    fn partial_snapshot_preserves_unloaded_tracks() {
        let mut current = CacheData::new_scoped("1", Some("server-a"));
        let mut previous = CacheData::new_scoped("1", Some("server-a"));
        previous.all_tracks.push(track("old"));

        current.preserve_unloaded_from(previous);

        assert_eq!(current.all_tracks.len(), 1);
        assert_eq!(current.all_tracks[0].rating_key, "old");
    }

    #[test]
    fn completed_empty_category_does_not_resurrect_old_tracks() {
        let mut current = CacheData::new_scoped("1", Some("server-a"));
        current
            .category_timestamps
            .insert("All Tracks".to_string(), CacheData::now());
        let mut previous = CacheData::new_scoped("1", Some("server-a"));
        previous.all_tracks.push(track("old"));

        current.preserve_unloaded_from(previous);

        assert!(current.all_tracks.is_empty());
    }

    #[test]
    fn server_scoped_paths_do_not_collide_or_escape_cache_directory() {
        let cache = LibraryCache {
            cache_dir: PathBuf::from("/tmp/textamp-cache-test"),
        };
        let first = cache.scoped_cache_path("server-a", "1");
        let second = cache.scoped_cache_path("server-b", "1");
        let hostile = cache.cache_path("../../outside");

        assert_ne!(first, second);
        assert_eq!(hostile.parent(), Some(cache.cache_dir.as_path()));
    }

    #[test]
    fn preserving_save_round_trips_previous_unloaded_category() {
        let directory = std::env::temp_dir().join(format!(
            "textamp-cache-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).unwrap();
        let cache = LibraryCache {
            cache_dir: directory.clone(),
        };

        let mut previous = CacheData::new_scoped("1", Some("server-a"));
        previous.all_tracks.push(track("old"));
        assert!(cache.save(&previous));

        let current = CacheData::new_scoped("1", Some("server-a"));
        assert!(cache.save_preserving_unloaded(current));

        let loaded = cache.load_scoped(Some("server-a"), "1").unwrap();
        assert_eq!(loaded.all_tracks[0].rating_key, "old");
        fs::remove_dir_all(directory).unwrap();
    }
}
