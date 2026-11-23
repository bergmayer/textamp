//! Private, disposable metadata snapshots for all supported library sources.
//! No credentials or media URLs are stored here. Call disk methods on a worker.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const VERSION: u32 = 1;
/// Library metadata shares one freshness policy across providers. Media caches
/// have separate size/eviction policies and are not periodically downloaded.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(7 * 24 * 3600);
pub fn refresh_due(timestamp: u64, current: u64) -> bool {
    timestamp == 0
        || timestamp > current
        || current.saturating_sub(timestamp) >= REFRESH_INTERVAL.as_secs()
}
/// Demand-loaded folder listings expire independently of the directory index.
pub const LISTING_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_FILE: u64 = 512 * 1024 * 1024;
const MAX_EXPANDED: u64 = 8 * 1024 * 1024 * 1024;
const MAX_TOTAL: u64 = 512 * 1024 * 1024;
static REVISION: AtomicU64 = AtomicU64::new(1);
static WRITES: Mutex<Option<Writes>> = Mutex::new(None);
#[derive(Default)]
struct Writes {
    cleared: HashMap<PathBuf, u64>,
    scopes: HashMap<(PathBuf, String), u64>,
    revisions: HashMap<PathBuf, u64>,
}

#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
    identity: String,
    scope: String,
}
#[derive(Clone, Debug)]
pub struct Ticket {
    store: Store,
    key: String,
    revision: u64,
    checkpoint: Option<usize>,
}
#[derive(Serialize, Deserialize)]
struct Snapshot<T> {
    version: u32,
    identity: String,
    key: String,
    timestamp: u64,
    value: T,
}
pub struct Hit<T> {
    pub value: T,
    pub stale: bool,
    pub timestamp: u64,
}

pub fn root() -> PathBuf {
    crate::config::XdgPaths::new("textamp")
        .cache_dir
        .join("sources")
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
impl Store {
    pub fn new(identity: impl Serialize) -> Result<Self> {
        Self::at(root(), identity)
    }
    pub fn at(root: PathBuf, identity: impl Serialize) -> Result<Self> {
        let identity = serde_json::to_string(&identity)?;
        let scope = format!("{:x}", md5::compute(identity.as_bytes()));
        Ok(Self {
            root,
            identity,
            scope,
        })
    }
    pub fn folder(source: &super::FolderSource) -> Result<Self> {
        // Display names are not identity; location changes must not reuse listings.
        Self::new(("folder", &source.id, &source.location))
    }
    /// Keep demand-loaded file listings in a separate bounded cache directory.
    pub fn listings(&self) -> Self {
        Self {
            root: self.root.join("temporary-listings"),
            identity: self.identity.clone(),
            scope: self.scope.clone(),
        }
    }
    pub fn ticket(&self, key: &str) -> Ticket {
        Ticket {
            checkpoint: None,
            store: self.clone(),
            key: key.into(),
            revision: REVISION.fetch_add(1, Ordering::SeqCst),
        }
    }
    pub fn scope_key(&self) -> &str {
        &self.scope
    }
    pub fn same_scope(&self, other: &Self) -> bool {
        self.root == other.root && self.scope == other.scope
    }
    fn owns(&self, path: &Path) -> bool {
        path.file_name().is_some_and(|name| {
            name.to_string_lossy()
                .starts_with(&format!("{}-", self.scope))
        })
    }
    /// Invalidate all writes issued before this clear, including absent files.
    pub fn clear(&self) -> Result<usize> {
        Ok(self.clear_files()? + self.listings().clear_files()?)
    }
    fn clear_files(&self) -> Result<usize> {
        let mut guard = WRITES.lock().unwrap_or_else(|p| p.into_inner());
        let writes = guard.get_or_insert_with(Writes::default);
        writes.scopes.insert(
            (self.root.clone(), self.scope.clone()),
            REVISION.fetch_add(1, Ordering::SeqCst),
        );
        let entries = files(&self.root)?
            .into_iter()
            .filter(|(path, _, _)| self.owns(path))
            .collect::<Vec<_>>();
        for (path, _, _) in &entries {
            std::fs::remove_file(path)?;
            writes.revisions.remove(path);
        }
        Ok(entries.len())
    }
    pub fn bytes(&self) -> Result<u64> {
        Ok(self.file_bytes()? + self.listings().file_bytes()?)
    }
    fn file_bytes(&self) -> Result<u64> {
        Ok(files(&self.root)?
            .into_iter()
            .filter(|(path, _, _)| self.owns(path))
            .map(|(_, size, _)| size)
            .sum())
    }
}
impl Ticket {
    fn path(&self) -> PathBuf {
        self.path_for(self.checkpoint)
    }

    fn path_for(&self, checkpoint: Option<usize>) -> PathBuf {
        let suffix = checkpoint.map(|n| format!(".part-{n}")).unwrap_or_default();
        self.store.root.join(format!(
            "{}-{:x}{suffix}.json.gz",
            self.store.scope,
            md5::compute(self.key.as_bytes())
        ))
    }
    pub(crate) fn checkpoint(&self, number: usize) -> Self {
        Self {
            checkpoint: Some(number),
            ..self.clone()
        }
    }
    pub fn write_finishing_checkpoints<T: Serialize>(&self, value: &T) -> Result<()> {
        self.write(value)?;
        let mut guard = WRITES.lock().unwrap_or_else(|p| p.into_inner());
        let writes = guard.get_or_insert_with(Writes::default);
        if writes.revisions.get(&self.path()) != Some(&self.revision) {
            return Ok(());
        }
        let prefix = format!(
            "{}-{:x}.part-",
            self.store.scope,
            md5::compute(self.key.as_bytes())
        );
        for (path, _, _) in files(&self.store.root)? {
            if path
                .file_name()
                .is_some_and(|s| s.to_string_lossy().starts_with(&prefix))
                && writes
                    .revisions
                    .get(&path)
                    .is_none_or(|r| *r <= self.revision)
            {
                std::fs::remove_file(&path)?;
                writes.revisions.remove(&path);
            }
        }
        Ok(())
    }
    /// Check whether budget eviction removed a snapshot since its batch completed.
    pub(crate) fn exists(&self) -> bool {
        let path = self.path();
        path.is_file() || path.with_extension("").is_file()
    }
    pub fn read<T: DeserializeOwned>(&self, ttl: Duration) -> Result<Option<Hit<T>>> {
        let path = self.path();
        let (file, compressed) = match std::fs::File::open(&path) {
            Ok(file) => (file, true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::File::open(path.with_extension("")) {
                    Ok(file) => (file, false),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    Err(e) => return Err(e).context("Read legacy library cache"),
                }
            }
            Err(e) => return Err(e).context("Read library cache"),
        };
        anyhow::ensure!(
            file.metadata()?.len() <= if compressed { MAX_FILE } else { MAX_EXPANDED },
            "Library cache is too large"
        );
        let reader = std::io::BufReader::new(file);
        let snapshot: Snapshot<T> = if compressed {
            decode(
                std::io::BufReader::new(flate2::read::GzDecoder::new(reader)),
                MAX_EXPANDED,
            )?
        } else {
            decode(reader, MAX_EXPANDED)?
        };
        if snapshot.version != VERSION
            || snapshot.identity != self.store.identity
            || snapshot.key != self.key
        {
            return Ok(None);
        }
        let current = now();
        Ok(Some(Hit {
            value: snapshot.value,
            timestamp: snapshot.timestamp,
            stale: snapshot.timestamp > current
                || current.saturating_sub(snapshot.timestamp) >= ttl.as_secs(),
        }))
    }
    pub fn write<T: Serialize>(&self, value: &T) -> Result<()> {
        self.write_at(value, now())
    }
    pub(crate) fn write_at<T: Serialize>(&self, value: &T, timestamp: u64) -> Result<()> {
        std::fs::create_dir_all(&self.store.root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.store.root, std::fs::Permissions::from_mode(0o700))?;
        }
        let mut temporary = tempfile::NamedTempFile::new_in(&self.store.root)?;
        {
            let encoder = flate2::write::GzEncoder::new(
                std::io::BufWriter::new(temporary.as_file_mut()),
                flate2::Compression::fast(),
            );
            let mut writer = BoundedWriter {
                inner: encoder,
                remaining: MAX_EXPANDED,
            };
            serde_json::to_writer(
                &mut writer,
                &Snapshot {
                    version: VERSION,
                    identity: self.store.identity.clone(),
                    key: self.key.clone(),
                    timestamp,
                    value,
                },
            )?;
            writer.inner.finish()?.flush()?;
        }
        temporary.as_file().sync_all()?;
        anyhow::ensure!(
            temporary.as_file().metadata()?.len() <= MAX_FILE,
            "Compressed library cache exceeds 512 MiB"
        );
        let mut guard = WRITES.lock().unwrap_or_else(|p| p.into_inner());
        let writes = guard.get_or_insert_with(Writes::default);
        let path = self.path();
        if self.checkpoint.is_some()
            && writes
                .revisions
                .get(&self.path_for(None))
                .is_some_and(|r| *r >= self.revision)
            || writes
                .cleared
                .get(&self.store.root)
                .is_some_and(|revision| self.revision <= *revision)
            || writes
                .scopes
                .get(&(self.store.root.clone(), self.store.scope.clone()))
                .is_some_and(|revision| self.revision <= *revision)
            || writes
                .revisions
                .get(&path)
                .is_some_and(|revision| *revision > self.revision)
        {
            return Ok(());
        }
        temporary.persist(&path).context("Save library cache")?;
        writes.revisions.insert(path.clone(), self.revision);
        let legacy = path.with_extension("");
        match std::fs::remove_file(legacy) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("Remove superseded uncompressed cache"),
        }
        trim(&self.store.root, &path, MAX_TOTAL, writes)
    }
}

fn is_checkpoint(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|s| s.to_string_lossy().contains(".part-"))
}

fn trim(root: &Path, saved: &Path, limit: u64, writes: &mut Writes) -> Result<()> {
    let mut entries = files(root)?;
    let mut total: u64 = entries.iter().map(|(_, size, _)| size).sum();
    if total <= limit {
        return Ok(());
    }
    // Keep the indexes needed to open a library ahead of individual item snapshots.
    let indexes = ["catalog", "browse-catalog-v1", super::tree::KEY]
        .map(|key| format!("{:x}", md5::compute(key)));
    entries.sort_by_cached_key(|(path, _, modified)| {
        let indexed = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.split_once('-'))
            .and_then(|(_, key)| key.split_once('.'))
            .is_some_and(|(key, _)| indexes.iter().any(|index| index == key));
        (!is_checkpoint(path), indexed, *modified)
    });
    for (old, size, _) in entries {
        if total <= limit {
            break;
        }
        if is_checkpoint(&old) || (old != saved && !is_checkpoint(saved)) {
            std::fs::remove_file(&old)?;
            writes.revisions.remove(&old);
            total = total.saturating_sub(size);
        }
    }
    Ok(())
}

fn files(root: &Path) -> Result<Vec<(PathBuf, u64, SystemTime)>> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };
    let mut files = vec![];
    for entry in entries {
        let entry = entry?;
        if entry
            .path()
            .extension()
            .is_some_and(|ext| ext == "json" || ext == "gz")
            && entry.file_type()?.is_file()
        {
            let meta = entry.metadata()?;
            files.push((entry.path(), meta.len(), meta.modified()?));
        }
    }
    Ok(files)
}
fn decode<T: DeserializeOwned>(reader: impl Read, limit: u64) -> Result<T> {
    let mut reader = reader.take(limit + 1);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .context("Read expanded library cache")?;
    anyhow::ensure!(reader.limit() > 0, "Expanded cache exceeds safety limit");
    serde_json::from_slice(&bytes).context("Invalid library cache")
}
struct BoundedWriter<W> {
    inner: W,
    remaining: u64,
}
impl<W: Write> Write for BoundedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() as u64 > self.remaining {
            return Err(std::io::Error::other(
                "Expanded cache exceeds 8 GiB safety limit",
            ));
        }
        let written = self.inner.write(bytes)?;
        self.remaining -= written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
pub fn clear_all() -> Result<usize> {
    clear_at(&root())
}
fn clear_at(root: &Path) -> Result<usize> {
    let mut guard = WRITES.lock().unwrap_or_else(|p| p.into_inner());
    let writes = guard.get_or_insert_with(Writes::default);
    writes
        .cleared
        .insert(root.to_path_buf(), REVISION.fetch_add(1, Ordering::SeqCst));
    writes.revisions.retain(|path, _| !path.starts_with(root));
    let files = files(root)?;
    for (path, _, _) in &files {
        std::fs::remove_file(path)?;
    }
    Ok(files.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_evicts_item_snapshots_before_an_older_library_index() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::at(root.path().into(), "budget").unwrap();
        let catalog = store.ticket("catalog");
        let item = store.ticket("playlist:old");
        let saved = store.ticket("playlist:new");
        for entry in [&catalog, &item, &saved] {
            entry.write(&"metadata").unwrap();
        }
        let limit =
            catalog.path().metadata().unwrap().len() + saved.path().metadata().unwrap().len();
        trim(root.path(), &saved.path(), limit, &mut Writes::default()).unwrap();
        assert!(catalog.exists() && saved.exists());
        assert!(!item.exists());
        assert_eq!(store.bytes().unwrap(), limit);
    }

    #[test]
    fn checkpoints_yield_to_committed_data_and_obey_refresh_ownership() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::at(root.path().into(), "library").unwrap();
        let old = store.ticket("analysis");
        old.write(&"complete").unwrap();
        let new = store.ticket("analysis");
        let part = new.checkpoint(0);
        part.write(&"temporary").unwrap();
        let limit = old.path().metadata().unwrap().len();
        trim(root.path(), &part.path(), limit, &mut Writes::default()).unwrap();
        assert!(old.exists() && !part.exists());
        part.write(&"progress").unwrap();
        old.write_finishing_checkpoints(&"old").unwrap();
        assert!(part.exists());
        new.write_finishing_checkpoints(&"finished").unwrap();
        part.write(&"late worker").unwrap();
        assert!(!part.exists());
    }

    #[test]
    fn committed_revision_survives_legacy_cleanup_failure() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path().into(), "fixture").unwrap();
        let old = store.ticket("album");
        let latest = store.ticket("album");
        std::fs::create_dir(latest.path().with_extension("")).unwrap();

        assert!(latest.write(&"new").is_err());
        old.write(&"old").unwrap();
        let snapshot = latest.read::<String>(REFRESH_INTERVAL).unwrap().unwrap();
        assert_eq!(snapshot.value, "new");
    }

    #[test]
    fn clearing_one_library_preserves_others_and_blocks_pending_writes() {
        let dir = tempfile::tempdir().unwrap();
        let first = Store::at(dir.path().into(), "first").unwrap();
        let second = Store::at(dir.path().into(), "second").unwrap();
        first.ticket("catalog").write(&1).unwrap();
        second.ticket("catalog").write(&2).unwrap();
        let pending = first.ticket("analysis-not-written-yet");
        assert!(first.bytes().unwrap() > 0);
        assert_eq!(first.clear().unwrap(), 1);
        pending.write(&3).unwrap();
        assert_eq!(first.bytes().unwrap(), 0);
        assert_eq!(
            second
                .ticket("catalog")
                .read::<i32>(REFRESH_INTERVAL)
                .unwrap()
                .unwrap()
                .value,
            2
        );
        first.ticket("catalog").write(&4).unwrap();
        assert!(first.bytes().unwrap() > 0);
    }

    #[test]
    fn weekly_freshness_includes_boundary_missing_and_future_timestamps() {
        let current = 10 * REFRESH_INTERVAL.as_secs();
        assert!(!refresh_due(current, current));
        assert!(!refresh_due(
            current - REFRESH_INTERVAL.as_secs() + 1,
            current
        ));
        assert!(refresh_due(current - REFRESH_INTERVAL.as_secs(), current));
        assert!(refresh_due(0, current));
        assert!(refresh_due(current + 1, current));
        let dir = tempfile::tempdir().unwrap();
        let ticket = Store::at(dir.path().into(), "weekly")
            .unwrap()
            .ticket("metadata");
        let now = now();
        ticket
            .write_at(&vec!["cached"], now - REFRESH_INTERVAL.as_secs())
            .unwrap();
        let hit = ticket
            .read::<Vec<String>>(REFRESH_INTERVAL)
            .unwrap()
            .unwrap();
        assert!(hit.stale);
        assert_eq!(hit.value, ["cached"], "stale metadata remains available");
        ticket.write(&vec!["updated"]).unwrap();
        assert!(
            !ticket
                .read::<Vec<String>>(REFRESH_INTERVAL)
                .unwrap()
                .unwrap()
                .stale
        );
    }

    #[test]
    fn streams_catalog_larger_than_old_256_mib_limit() {
        use serde::ser::SerializeSeq;
        struct LargeCatalog;
        impl Serialize for LargeCatalog {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let row = "music metadata ".repeat(70000);
                let mut seq = serializer.serialize_seq(Some(270))?;
                for _ in 0..270 {
                    seq.serialize_element(&row)?;
                }
                seq.end()
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let ticket = Store::at(dir.path().into(), "large")
            .unwrap()
            .ticket("catalog");
        ticket.write(&LargeCatalog).unwrap();
        assert!(std::fs::metadata(ticket.path()).unwrap().len() < 8 * 1024 * 1024);
        assert!(
            ticket
                .read::<serde::de::IgnoredAny>(Duration::ZERO)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn legacy_cache_is_read_then_migrated_and_decode_limit_is_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let ticket = Store::at(dir.path().into(), "legacy")
            .unwrap()
            .ticket("root");
        let legacy = ticket.path().with_extension("");
        let snapshot = Snapshot {
            version: VERSION,
            identity: ticket.store.identity.clone(),
            key: ticket.key.clone(),
            timestamp: 1,
            value: vec!["old"],
        };
        std::fs::write(&legacy, serde_json::to_vec(&snapshot).unwrap()).unwrap();
        assert_eq!(
            ticket
                .read::<Vec<String>>(Duration::ZERO)
                .unwrap()
                .unwrap()
                .value,
            ["old"]
        );
        ticket.write(&vec!["new"]).unwrap();
        assert!(!legacy.exists());
        assert_eq!(
            ticket
                .read::<Vec<String>>(Duration::ZERO)
                .unwrap()
                .unwrap()
                .value,
            ["new"]
        );
        assert!(decode::<serde_json::Value>(&b"[1,2,3]"[..], 4).is_err());
    }
    #[test]
    fn snapshots_are_scoped_fresh_and_ordered_and_clear_rejects_old_writes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path().into(), ("account", "library")).unwrap();
        let old = store.ticket("root");
        let new = store.ticket("root");
        new.write(&vec!["new"]).unwrap();
        old.write(&vec!["old"]).unwrap();
        let hit = new
            .read::<Vec<String>>(Duration::from_secs(60))
            .unwrap()
            .unwrap();
        assert_eq!(hit.value, ["new"]);
        assert!(!hit.stale);
        assert!(
            new.read::<Vec<String>>(Duration::ZERO)
                .unwrap()
                .unwrap()
                .stale
        );
        assert!(
            Store::at(dir.path().into(), ("other", "library"))
                .unwrap()
                .ticket("root")
                .read::<Vec<String>>(Duration::ZERO)
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .ticket("other-folder")
                .read::<Vec<String>>(Duration::ZERO)
                .unwrap()
                .is_none()
        );
        assert_eq!(clear_at(dir.path()).unwrap(), 1);
        new.write(&vec!["resurrected"]).unwrap();
        assert!(new.read::<Vec<String>>(Duration::ZERO).unwrap().is_none());
        store.ticket("root").write(&Vec::<String>::new()).unwrap();
        assert!(
            new.read::<Vec<String>>(Duration::ZERO)
                .unwrap()
                .unwrap()
                .value
                .is_empty()
        );
    }
    #[test]
    fn corrupt_cache_is_an_error_and_a_valid_refresh_repairs_it() {
        let dir = tempfile::tempdir().unwrap();
        let ticket = Store::at(dir.path().into(), "source")
            .unwrap()
            .ticket("root");
        ticket.write(&vec!["track"]).unwrap();
        std::fs::write(ticket.path(), b"not json").unwrap();
        assert!(ticket.read::<Vec<String>>(Duration::ZERO).is_err());
        ticket.write(&vec!["repaired"]).unwrap();
        assert_eq!(
            ticket
                .read::<Vec<String>>(Duration::ZERO)
                .unwrap()
                .unwrap()
                .value,
            ["repaired"]
        );
        std::fs::OpenOptions::new()
            .write(true)
            .open(ticket.path())
            .unwrap()
            .set_len(MAX_FILE + 1)
            .unwrap();
        assert!(
            ticket.read::<Vec<String>>(Duration::ZERO).is_err(),
            "oversized files are rejected before allocation"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(ticket.path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}
