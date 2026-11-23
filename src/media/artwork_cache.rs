//! Library-scoped image storage with bounded background fills and LRU demand eviction.

use std::collections::{HashMap, HashSet, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

/// Maximum downloaded artwork retained for one library.
pub const ARTWORK_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
static CACHES: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<CacheState>>>>> = OnceLock::new();

#[derive(Default)]
struct CacheState {
    generation: u64,
    bytes: Option<u64>,
    reference_revision: u64,
}

enum WritePolicy {
    Demand,
    Prefetch,
}

/// Shared disk artwork cache. Perform filesystem operations on a worker.
#[derive(Clone)]
pub struct ArtworkCache {
    cache_dir: PathBuf,
    state: Arc<Mutex<CacheState>>,
    generation: u64,
}

impl ArtworkCache {
    /// Open an image cache, retaining its invalidation generation.
    pub fn new(cache_dir: PathBuf) -> Self {
        let state = CACHES
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(cache_dir.clone())
            .or_default()
            .clone();
        let generation = state.lock().unwrap_or_else(|p| p.into_inner()).generation;
        Self {
            cache_dir,
            state,
            generation,
        }
    }

    /// Open the cache belonging to an exact library identity.
    pub fn for_library(library_key: &str) -> Self {
        let root = get_artwork_cache_dir().unwrap_or_else(|| PathBuf::from("/tmp/textamp_images"));
        Self::new(root.join("by-library").join(Self::hash(library_key)))
    }

    fn hash(key: &str) -> String {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    fn cache_path(&self, key: &str) -> PathBuf {
        self.cache_dir.join(format!("{}.bin", Self::hash(key)))
    }

    fn resolved_path(&self, key: &str) -> PathBuf {
        let path = self.cache_path(key);
        if path.exists() {
            return path;
        }
        if let Ok(target) = std::fs::read_to_string(path.with_extension("alias"))
            && target.len() == 16
            && target.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return self.cache_dir.join(format!("{target}.bin"));
        }
        path
    }

    /// Check background coverage without reading image bytes or changing LRU order.
    pub(crate) fn contains(&self, key: &str) -> bool {
        std::fs::metadata(self.resolved_path(key))
            .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
    }

    /// A changed server cover ID invalidates an album's old image alias.
    pub(crate) fn contains_cover(&self, album: &str, cover: Option<&str>) -> bool {
        self.contains(album)
            && cover.is_none_or(|cover| self.resolved_path(album) == self.cache_path(cover))
    }

    /// Read an image and update its last-use time. Zero TTL disables expiry.
    pub fn load(&self, key: &str, ttl_secs: u64) -> Option<Vec<u8>> {
        let path = self.resolved_path(key);
        use std::io::Read;
        let mut file = std::fs::File::open(&path).ok()?;
        let metadata = file.metadata().ok()?;
        let age = metadata
            .modified()
            .ok()
            .and_then(|mtime| mtime.elapsed().ok());
        if ttl_secs > 0 && age.is_some_and(|age| age.as_secs() > ttl_secs) {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            if std::fs::remove_file(&path).is_ok() {
                state.bytes = None;
            }
            return None;
        }
        let mut data = Vec::with_capacity(metadata.len().try_into().ok()?);
        file.read_to_end(&mut data).ok()?;
        // An expiring entry uses sliding TTL. Permanent entries need only coarse LRU ordering.
        if ttl_secs > 0 || age.is_none_or(|age| age.as_secs() >= 60) {
            let _ = file.set_times(std::fs::FileTimes::new().set_modified(SystemTime::now()));
        }
        Some(data)
    }

    fn entries(&self) -> Vec<(PathBuf, u64, SystemTime)> {
        std::fs::read_dir(&self.cache_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if !path
                    .extension()
                    .is_some_and(|ext| ext == "bin" || ext == "alias")
                {
                    return None;
                }
                let meta = entry.metadata().ok()?;
                meta.is_file().then(|| {
                    (
                        path,
                        meta.len(),
                        meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    )
                })
            })
            .collect()
    }

    /// Return the number of image files and their total stored bytes, including aliases.
    pub fn stats(&self) -> (usize, u64) {
        let entries = self.entries();
        (
            entries
                .iter()
                .filter(|(p, _, _)| p.extension().is_some_and(|ext| ext == "bin"))
                .count(),
            entries.iter().map(|(_, size, _)| size).sum(),
        )
    }

    /// Remove images and their reference index, fencing every previously issued writer.
    pub fn clear_all(&self) -> usize {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.generation = state.generation.wrapping_add(1);
        state.reference_revision = state.reference_revision.wrapping_add(1);
        let _ = std::fs::remove_file(self.cache_dir.join("warm-summary.json"));
        let mut removed = 0;
        for (path, _, _) in self.entries() {
            if std::fs::remove_file(&path).is_ok()
                && path.extension().is_some_and(|ext| ext == "bin")
            {
                removed += 1;
            }
        }
        let _ = std::fs::remove_file(self.cache_dir.join("playlist-references.json"));
        state.bytes = None;
        removed
    }

    fn atomic_write(&self, path: &std::path::Path, data: &[u8]) -> anyhow::Result<()> {
        use std::io::Write;
        std::fs::create_dir_all(&self.cache_dir)?;
        let mut temporary = tempfile::NamedTempFile::new_in(&self.cache_dir)?;
        temporary.write_all(data)?;
        temporary.persist(path)?;
        Ok(())
    }

    /// Save requested artwork, evicting least recently used files when necessary.
    pub fn save(&self, key: &str, data: &[u8]) -> bool {
        self.store(key, data, ARTWORK_LIMIT, WritePolicy::Demand)
            .unwrap_or_else(|error| {
                tracing::warn!("Save artwork: {error:#}");
                false
            })
    }

    /// Fill available space without evicting images used by interactive browsing.
    /// Returns false when the library's artwork budget is full.
    ///
    /// # Errors
    /// Returns an error for failed writes or an invalidated cache generation.
    pub(crate) fn prefetch(&self, key: &str, data: &[u8]) -> anyhow::Result<bool> {
        self.store(
            key,
            data,
            ARTWORK_LIMIT.saturating_sub(16),
            WritePolicy::Prefetch,
        )
    }

    fn store(
        &self,
        key: &str,
        data: &[u8],
        limit: u64,
        policy: WritePolicy,
    ) -> anyhow::Result<bool> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        anyhow::ensure!(
            state.generation == self.generation,
            "Artwork cache was invalidated"
        );
        if data.len() as u64 > limit {
            return Ok(false);
        }
        let total = *state.bytes.get_or_insert_with(|| self.stats().1);
        let path = self.cache_path(key);
        let old = std::fs::metadata(&path).map_or(0, |m| m.len());
        let required = (data.len() as u64).saturating_sub(old);
        if total.saturating_add(required) > limit {
            if matches!(policy, WritePolicy::Prefetch) {
                return Ok(false);
            }
            self.prune_locked(&mut state, (limit * 9 / 10).min(limit - data.len() as u64));
        }
        let old = std::fs::metadata(&path).map_or(0, |m| m.len());
        self.atomic_write(&path, data)?;
        state.bytes = Some(state.bytes.unwrap_or_default().saturating_sub(old) + data.len() as u64);
        let alias = path.with_extension("alias");
        if let Ok(meta) = std::fs::metadata(&alias)
            && std::fs::remove_file(alias).is_ok()
        {
            state.bytes = state.bytes.map(|total| total.saturating_sub(meta.len()));
        }
        Ok(true)
    }

    /// Associate an album identity with an already stored cover without copying image bytes.
    pub(crate) fn alias(&self, key: &str, target: &str) -> anyhow::Result<()> {
        if key == target {
            return Ok(());
        }
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        anyhow::ensure!(
            state.generation == self.generation,
            "Artwork cache was invalidated"
        );
        anyhow::ensure!(
            self.cache_path(target).exists(),
            "Artwork alias target is missing"
        );
        let path = self.cache_path(key);
        let alias = path.with_extension("alias");
        let total = *state.bytes.get_or_insert_with(|| self.stats().1);
        let old_alias = std::fs::metadata(&alias).map_or(0, |meta| meta.len());
        let old_image = std::fs::metadata(&path).map_or(0, |meta| meta.len());
        anyhow::ensure!(
            total.saturating_sub(old_alias).saturating_sub(old_image) + 16 <= ARTWORK_LIMIT,
            "Artwork budget is full"
        );
        self.atomic_write(&alias, Self::hash(target).as_bytes())?;
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                state.bytes = None;
                return Err(error.into());
            }
        }
        state.bytes = Some(total.saturating_sub(old_alias).saturating_sub(old_image) + 16);
        Ok(())
    }

    fn prune_locked(&self, state: &mut CacheState, max_bytes: u64) {
        let mut entries = self.entries();
        let mut total: u64 = entries.iter().map(|(_, size, _)| size).sum();
        if total <= max_bytes {
            state.bytes = Some(total);
            return;
        }
        let mut aliases: HashMap<PathBuf, Vec<(PathBuf, u64)>> = HashMap::new();
        for (path, size, _) in &entries {
            if path.extension().is_some_and(|ext| ext == "alias")
                && let Ok(target) = std::fs::read_to_string(path)
                && target.len() == 16
                && target.bytes().all(|c| c.is_ascii_hexdigit())
            {
                let image = self.cache_dir.join(format!("{target}.bin"));
                if image.exists() {
                    aliases
                        .entry(image)
                        .or_default()
                        .push((path.clone(), *size));
                } else if std::fs::remove_file(path).is_ok() {
                    total = total.saturating_sub(*size);
                }
            }
        }
        entries.retain(|(path, _, _)| path.extension().is_some_and(|ext| ext == "bin"));
        entries.sort_by_key(|(_, _, modified)| *modified);
        for (path, size, _) in entries {
            if total <= max_bytes {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(size);
                for (alias, size) in aliases.remove(&path).unwrap_or_default() {
                    if std::fs::remove_file(alias).is_ok() {
                        total = total.saturating_sub(size);
                    }
                }
            }
        }
        state.bytes = Some(total);
    }

    /// Enforce a disk budget, removing least recently used entries first.
    pub fn prune_to_size(&self, max_bytes: u64) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.generation == self.generation {
            self.prune_locked(&mut state, max_bytes);
        }
    }

    /// A completed, fresh warm job can be reused until its catalog changes or this cache is cleared.
    pub(crate) fn warm_summary_matches(&self, fingerprint: &str) -> bool {
        let summary = std::fs::read(self.cache_dir.join("warm-summary.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<(String, u64)>(&bytes).ok());
        summary.is_some_and(|(saved, timestamp)| {
            saved == fingerprint
                && !crate::library::cache::refresh_due(timestamp, crate::library::cache::now())
        })
    }
    pub(crate) fn finish_warm_summary(
        &self,
        fingerprint: &str,
        revision: u64,
    ) -> anyhow::Result<()> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        anyhow::ensure!(
            state.reference_revision == revision,
            "Artwork refresh was invalidated"
        );
        self.atomic_write(
            &self.cache_dir.join("warm-summary.json"),
            &serde_json::to_vec(&(fingerprint, crate::library::cache::now()))?,
        )
    }

    /// Begin a reference refresh so an older completion cannot replace a newer index.
    pub(crate) fn begin_refresh(&self) -> u64 {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.reference_revision = state.reference_revision.wrapping_add(1);
        state.reference_revision
    }

    /// Retire catalog and playlist images only after a complete successful refresh.
    /// Catalog and current playlist references both protect shared covers.
    ///
    /// # Errors
    /// Returns filesystem errors or an invalidated cache generation.
    pub(crate) fn reconcile_playlists(
        &self,
        catalog: &HashSet<String>,
        playlists: &HashSet<String>,
        revision: u64,
    ) -> anyhow::Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        anyhow::ensure!(
            state.generation == self.generation,
            "Artwork cache was invalidated"
        );
        anyhow::ensure!(
            state.reference_revision == revision,
            "Artwork reference refresh was superseded"
        );
        let paths = |keys: &HashSet<String>| -> HashSet<String> {
            keys.iter()
                .flat_map(|key| {
                    [
                        self.cache_path(key).with_extension("alias"),
                        self.resolved_path(key),
                    ]
                })
                .filter_map(|path| path.file_name().map(|n| n.to_string_lossy().into_owned()))
                .collect()
        };
        let mut retained = paths(catalog);
        retained.extend(paths(playlists));
        let index = self.cache_dir.join("playlist-references.json");
        let previous: HashSet<String> = match std::fs::read(&index) {
            Ok(bytes) => {
                anyhow::ensure!(
                    bytes.len() <= 16 * 1024 * 1024,
                    "Artwork reference index is too large"
                );
                serde_json::from_slice(&bytes)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashSet::new(),
            Err(error) => return Err(error.into()),
        };
        let mut removed = false;
        for name in previous.difference(&retained) {
            if name.len() <= 22
                && !name.contains(['/', '\\'])
                && (name.ends_with(".bin") || name.ends_with(".alias"))
            {
                match std::fs::remove_file(self.cache_dir.join(name)) {
                    Ok(()) => {
                        removed = true;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        self.atomic_write(&index, &serde_json::to_vec(&retained)?)?;
        state.bytes = None;
        if removed {
            state.generation = state.generation.wrapping_add(1);
        }
        Ok(())
    }
}

impl Default for ArtworkCache {
    fn default() -> Self {
        Self::new(get_artwork_cache_dir().unwrap_or_else(|| PathBuf::from("/tmp/textamp_images")))
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budget_preserves_recent_images_and_prefetch_never_evicts() {
        let root = tempfile::tempdir().unwrap();
        let cache = ArtworkCache::new(root.path().into());
        assert!(
            cache
                .store("old", &[1; 40], 100, WritePolicy::Demand)
                .unwrap()
        );
        cache.alias("album", "old").unwrap();
        let file = std::fs::File::open(cache.cache_path("old")).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
            .unwrap();
        assert!(
            cache
                .store("recent", &[2; 40], 100, WritePolicy::Demand)
                .unwrap()
        );
        assert!(
            !cache
                .store("background", &[3; 40], 100, WritePolicy::Prefetch)
                .unwrap()
        );
        assert!(cache.contains("album"));
        assert!(
            cache
                .store("requested", &[4; 40], 100, WritePolicy::Demand)
                .unwrap()
        );
        assert!(!cache.contains("old"));
        assert!(!cache.contains("album"));
        assert!(cache.contains("recent"));
        assert!(cache.contains("requested"));
        assert_eq!(cache.stats(), (2, 80));
        assert!(
            !cache
                .store("oversized", &[5; 101], 100, WritePolicy::Demand)
                .unwrap()
        );
    }

    #[test]
    fn growing_an_existing_image_stays_within_budget_when_it_is_evicted_first() {
        let root = tempfile::tempdir().unwrap();
        let cache = ArtworkCache::new(root.path().into());
        assert!(
            cache
                .store("growing", &[1; 40], 100, WritePolicy::Demand)
                .unwrap()
        );
        std::fs::File::open(cache.cache_path("growing"))
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
            .unwrap();
        assert!(
            cache
                .store("other", &[2; 60], 100, WritePolicy::Demand)
                .unwrap()
        );
        assert!(
            cache
                .store("growing", &[3; 80], 100, WritePolicy::Demand)
                .unwrap()
        );
        assert!(cache.stats().1 <= 100);
        assert_eq!(cache.load("growing", 0).unwrap(), vec![3; 80]);
    }

    #[test]
    fn playlist_cleanup_protects_catalog_covers_and_rejects_superseded_refreshes() {
        let root = tempfile::tempdir().unwrap();
        let cache = ArtworkCache::new(root.path().into());
        assert!(cache.save("shared-cover", b"shared"));
        cache.alias("catalog-album", "shared-cover").unwrap();
        assert!(cache.contains_cover("catalog-album", Some("shared-cover")));
        assert!(!cache.contains_cover("catalog-album", Some("replacement-cover")));
        cache.alias("playlist-album", "shared-cover").unwrap();
        assert!(cache.save("removed-cover", b"removed"));
        cache.alias("removed-album", "removed-cover").unwrap();
        assert_eq!(cache.stats(), (2, 61));
        let catalog = HashSet::from(["catalog-album".into()]);
        let mut previous_catalog = catalog.clone();
        previous_catalog.insert("retired-catalog-album".into());
        assert!(cache.save("retired-catalog-album", b"retired"));
        let playlists = HashSet::from(["playlist-album".into(), "removed-album".into()]);
        cache
            .reconcile_playlists(&previous_catalog, &playlists, cache.begin_refresh())
            .unwrap();
        let old = cache.begin_refresh();
        let current = cache.begin_refresh();
        assert!(
            cache
                .reconcile_playlists(&catalog, &HashSet::new(), old)
                .is_err()
        );
        assert!(cache.contains("removed-album"));
        cache
            .reconcile_playlists(&catalog, &HashSet::new(), current)
            .unwrap();
        assert_eq!(cache.load("catalog-album", 0).unwrap(), b"shared");
        assert!(!cache.contains("retired-catalog-album"));
        assert!(!cache.contains("removed-album"));
        assert!(!cache.contains("playlist-album"));
        assert!(!cache.save("removed-cover", b"late completion"));
        assert!(ArtworkCache::new(root.path().into()).save("new", b"new"));
    }

    #[test]
    fn clearing_artwork_fences_old_writers_and_preserves_other_libraries() {
        let root = std::env::temp_dir().join(format!("textamp-art-{}", uuid::Uuid::new_v4()));
        let first = ArtworkCache::new(root.join("first"));
        let pending = first.clone();
        let second = ArtworkCache::new(root.join("second"));
        assert!(first.save("album", b"first"));
        assert!(second.save("album", b"second"));
        assert_eq!(first.clear_all(), 1);
        assert!(!pending.save("album", b"late"));
        assert!(first.load("album", 0).is_none());
        assert_eq!(second.load("album", 0).unwrap(), b"second");
        assert!(ArtworkCache::new(root.join("first")).save("album", b"fresh"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
