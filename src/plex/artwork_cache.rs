//! Disk cache for album artwork images.
//!
//! Stores raw image bytes (PNG/JPEG from Plex transcoder) with hash-based
//! filenames. Uses file modification time for age checking.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::SystemTime;

/// Disk cache for album artwork.
pub struct ArtworkCache {
    cache_dir: PathBuf,
}

impl ArtworkCache {
    /// Create a new artwork cache with the given directory.
    pub fn new(cache_dir: PathBuf) -> Self {
        Self { cache_dir }
    }

    /// Get the cache file path for a key (album/artist rating key).
    fn cache_path(&self, key: &str) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        let hash = hasher.finish();
        self.cache_dir.join(format!("{:016x}.bin", hash))
    }

    /// Load artwork bytes from cache. Returns None if not cached or expired.
    pub fn load(&self, key: &str, ttl_secs: u64) -> Option<Vec<u8>> {
        let path = self.cache_path(key);
        if !path.exists() {
            return None;
        }

        // Check file age via modification time
        if ttl_secs > 0 {
            if let Ok(metadata) = std::fs::metadata(&path) {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(age) = SystemTime::now().duration_since(modified) {
                        if age.as_secs() > ttl_secs {
                            // Expired - remove and return None
                            let _ = std::fs::remove_file(&path);
                            return None;
                        }
                    }
                }
            }
        }

        let data = std::fs::read(&path).ok();
        // Touch mtime on successful read so LRU pruning reflects last-accessed time
        if data.is_some() {
            if let Ok(file) = std::fs::File::open(&path) {
                let times = std::fs::FileTimes::new().set_modified(SystemTime::now());
                let _ = file.set_times(times);
            }
        }
        data
    }

    /// Load with warm cache support. Returns (data, is_warm) where is_warm
    /// means the entry is older than `warm_threshold_secs` (e.g. 32 days).
    /// Unlike `load()`, this never deletes expired entries — they're served
    /// as warm cache and should be re-fetched in background by the caller.
    pub fn load_warm(&self, key: &str, warm_threshold_secs: u64) -> Option<(Vec<u8>, bool)> {
        let path = self.cache_path(key);
        if !path.exists() {
            return None;
        }

        let is_warm = if let Ok(metadata) = std::fs::metadata(&path) {
            if let Ok(modified) = metadata.modified() {
                if let Ok(age) = SystemTime::now().duration_since(modified) {
                    age.as_secs() >= warm_threshold_secs
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        std::fs::read(&path).ok().map(|data| {
            // Touch mtime on successful read so LRU pruning reflects last-accessed time
            if let Ok(file) = std::fs::File::open(&path) {
                let times = std::fs::FileTimes::new().set_modified(SystemTime::now());
                let _ = file.set_times(times);
            }
            (data, is_warm)
        })
    }

    /// Get cache statistics: (file_count, total_bytes).
    pub fn stats(&self) -> (usize, u64) {
        if !self.cache_dir.exists() {
            return (0, 0);
        }

        let mut count = 0usize;
        let mut total_bytes = 0u64;

        if let Ok(entries) = std::fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().map_or(false, |e| e == "bin") {
                    if let Ok(metadata) = entry.metadata() {
                        count += 1;
                        total_bytes += metadata.len();
                    }
                }
            }
        }

        (count, total_bytes)
    }

    /// Clear all cached artwork files. Returns the number of files removed.
    pub fn clear_all(&self) -> usize {
        if !self.cache_dir.exists() {
            return 0;
        }

        let mut removed = 0;
        if let Ok(entries) = std::fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().map_or(false, |e| e == "bin") {
                    if std::fs::remove_file(&path).is_ok() {
                        removed += 1;
                    }
                }
            }
        }
        removed
    }

    /// Save artwork bytes to cache. Uses atomic write via temp file.
    pub fn save(&self, key: &str, data: &[u8]) -> bool {
        // Ensure directory exists
        if !self.cache_dir.exists() {
            if std::fs::create_dir_all(&self.cache_dir).is_err() {
                return false;
            }
        }

        let path = self.cache_path(key);
        let temp_path = path.with_extension("bin.tmp");

        if std::fs::write(&temp_path, data).is_ok() {
            std::fs::rename(&temp_path, &path).is_ok()
        } else {
            false
        }
    }

    /// Prune entries older than the given TTL.
    pub fn prune_expired(&self, ttl_secs: u64) {
        if !self.cache_dir.exists() {
            return;
        }

        let entries = match std::fs::read_dir(&self.cache_dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().map_or(true, |e| e != "bin") {
                continue;
            }

            if let Ok(metadata) = entry.metadata() {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(age) = SystemTime::now().duration_since(modified) {
                        if age.as_secs() > ttl_secs {
                            tracing::debug!("Pruning expired artwork: {:?}", path.file_name());
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
            }
        }
    }

    /// Prune cache to fit within a size limit. Removes oldest files first.
    pub fn prune_to_size(&self, max_bytes: u64) {
        if !self.cache_dir.exists() {
            return;
        }

        let mut entries: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
        let mut total_size = 0u64;

        if let Ok(dir_entries) = std::fs::read_dir(&self.cache_dir) {
            for entry in dir_entries.flatten() {
                let path = entry.path();
                if !path.is_file() || path.extension().map_or(true, |e| e != "bin") {
                    continue;
                }

                if let Ok(metadata) = entry.metadata() {
                    let size = metadata.len();
                    total_size += size;
                    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    entries.push((path, size, modified));
                }
            }
        }

        if total_size <= max_bytes {
            return;
        }

        // Sort by modification time ascending (oldest first)
        entries.sort_by_key(|(_, _, modified)| *modified);

        for (path, size, _) in entries {
            if total_size <= max_bytes {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total_size = total_size.saturating_sub(size);
            }
        }
    }
}

impl Default for ArtworkCache {
    fn default() -> Self {
        let cache_dir = get_artwork_cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp/textamp_images"));
        Self { cache_dir }
    }
}

/// Get the artwork cache directory path.
fn get_artwork_cache_dir() -> Option<PathBuf> {
    // Check XDG env var first
    if let Ok(xdg_cache) = std::env::var("XDG_CACHE_HOME") {
        return Some(PathBuf::from(xdg_cache).join("textamp/images"));
    }

    #[cfg(target_os = "linux")]
    {
        dirs::home_dir().map(|h| h.join(".cache/textamp/images"))
    }

    #[cfg(target_os = "macos")]
    {
        dirs::cache_dir().map(|p| p.join("textamp/images"))
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        dirs::cache_dir().map(|p| p.join("textamp/images"))
    }
}
