//! Explicit offline audio storage, independent of the evictable metadata and artwork caches.
use crate::library::models::Track;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// A committed local copy, scoped to its source library.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub library: String,
    pub library_name: String,
    pub track: Track,
    pub bytes: u64,
    pub artwork: bool,
}
/// Persistent index. File names are derived from source identities, never server paths.
#[derive(Clone, Debug)]
pub struct Store {
    pub root: PathBuf,
    pub entries: BTreeMap<String, Entry>,
}
impl Default for Store {
    fn default() -> Self {
        Self {
            root: crate::config::XdgPaths::new("textamp")
                .data_dir
                .join("downloads"),
            entries: BTreeMap::new(),
        }
    }
}
impl Store {
    /// Read the last durable index. Missing audio is not advertised as downloaded.
    /// # Errors
    /// Returns corrupt-index and filesystem errors without replacing the index.
    pub fn load(root: PathBuf) -> anyhow::Result<Self> {
        let path = root.join("index.json");
        let mut entries: BTreeMap<String, Entry> = match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error.into()),
        };
        entries.retain(|id, entry| {
            valid_id(id)
                && root
                    .join(id)
                    .join("audio")
                    .metadata()
                    .is_ok_and(|metadata| metadata.len() == entry.bytes && entry.bytes > 0)
        });
        Ok(Self { root, entries })
    }
    /// Stable source-and-track key; identical provider IDs in different libraries stay separate.
    pub fn id(library: &str, track: &Track) -> String {
        format!(
            "{:x}",
            md5::compute(format!("{library}\0{}", track.rating_key))
        )
    }
    /// Local path for one indexed audio item.
    pub fn audio(&self, id: &str) -> Option<PathBuf> {
        self.entries
            .contains_key(id)
            .then(|| self.root.join(id).join("audio"))
    }
    /// Find a saved copy by immutable source origin.
    pub fn find(&self, track: &Track) -> Option<(&str, &Entry)> {
        self.entries
            .iter()
            .find(|(_, entry)| {
                entry.track.origin == track.origin && entry.track.rating_key == track.rating_key
            })
            .map(|(id, entry)| (id.as_str(), entry))
    }
    /// Commit an index after its audio has been durably installed.
    /// # Errors
    /// Returns serialization, write, or atomic replacement errors.
    pub fn save(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let mut file = tempfile::NamedTempFile::new_in(&self.root)?;
        serde_json::to_writer(&mut file, &self.entries)?;
        file.as_file().sync_all()?;
        file.persist(self.root.join("index.json"))?;
        Ok(())
    }
    /// Remove only a verified, indexed download; original music is never a target.
    /// # Errors
    /// Returns filesystem errors, preserving a usable index on failure.
    pub fn remove(&mut self, id: &str) -> anyhow::Result<()> {
        if !valid_id(id) || !self.entries.contains_key(id) {
            return Ok(());
        }
        let entry = self.entries.remove(id);
        if let Err(error) = self.save() {
            if let Some(entry) = entry {
                self.entries.insert(id.into(), entry);
            }
            return Err(error);
        }
        match std::fs::remove_dir_all(self.root.join(id)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
    /// Remove all committed copies with a single index transaction.
    /// # Errors
    /// Returns index or filesystem errors. Original source files are never affected.
    pub fn clear(&mut self) -> anyhow::Result<()> {
        let entries = std::mem::take(&mut self.entries);
        if let Err(error) = self.save() {
            self.entries = entries;
            return Err(error);
        }
        let mut failure = None;
        for id in entries.keys().filter(|id| valid_id(id)) {
            match std::fs::remove_dir_all(self.root.join(id)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        failure.map_or(Ok(()), |error| Err(error.into()))
    }
    /// Sum only committed audio bytes for download management.
    pub fn size(&self) -> u64 {
        self.entries.values().map(|entry| entry.bytes).sum()
    }
}
fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Finish a temporary download in the store's filesystem with atomic replacement.
/// # Errors
/// Rejects incomplete files; an indexed copy is never overwritten.
pub fn install(
    store: &mut Store,
    id: String,
    entry: Entry,
    audio: &Path,
    artwork: Option<&[u8]>,
) -> anyhow::Result<()> {
    anyhow::ensure!(valid_id(&id) && entry.bytes > 0, "Invalid download");
    if store.entries.contains_key(&id) {
        return Ok(());
    }
    anyhow::ensure!(
        std::fs::metadata(audio)?.len() == entry.bytes,
        "Incomplete download"
    );
    let staged = tempfile::Builder::new()
        .prefix(".download-")
        .tempdir_in(&store.root)?;
    std::fs::rename(audio, staged.path().join("audio"))?;
    if let Some(artwork) = artwork {
        std::fs::write(staged.path().join("artwork"), artwork)?;
    }
    let directory = store.root.join(&id);
    if directory.exists() {
        std::fs::remove_dir_all(&directory)?;
    }
    std::fs::rename(staged.path(), &directory)?;
    store.entries.insert(id.clone(), entry);
    if let Err(error) = store.save() {
        store.entries.remove(&id);
        let _ = std::fs::remove_dir_all(directory);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clearing_downloads_continues_after_one_copy_cannot_be_removed() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = Store::load(directory.path().into()).unwrap();
        let ids = ["0".repeat(32), "1".repeat(32)];
        for id in &ids {
            store.entries.insert(
                id.clone(),
                Entry {
                    library: "fixture".into(),
                    library_name: "Fixture".into(),
                    track: Track::default(),
                    bytes: 1,
                    artwork: false,
                },
            );
        }
        let blocked = directory.path().join(&ids[0]);
        std::fs::write(&blocked, b"x").unwrap();
        let removable = directory.path().join(&ids[1]);
        std::fs::create_dir(&removable).unwrap();
        std::fs::write(removable.join("audio"), b"x").unwrap();

        assert!(store.clear().is_err());
        assert!(!removable.exists());
        assert!(store.entries.is_empty());
        assert!(
            Store::load(directory.path().into())
                .unwrap()
                .entries
                .is_empty()
        );
    }

    #[test]
    fn source_ids_are_isolated_and_removed_audio_never_touches_originals() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original.wav");
        std::fs::write(&original, b"audio").unwrap();
        let mut store = Store::load(directory.path().join("downloads")).unwrap();
        let track = Track {
            rating_key: "same".into(),
            ..Default::default()
        };
        let id = Store::id("one", &track);
        assert_ne!(id, Store::id("two", &track));
        std::fs::create_dir_all(&store.root).unwrap();
        let temporary = store.root.join("partial");
        std::fs::copy(&original, &temporary).unwrap();
        install(
            &mut store,
            id.clone(),
            Entry {
                library: "one".into(),
                library_name: "One".into(),
                track,
                bytes: 5,
                artwork: false,
            },
            &temporary,
            None,
        )
        .unwrap();
        assert_eq!(Store::load(store.root.clone()).unwrap().entries.len(), 1);
        store.remove(&id).unwrap();
        assert!(original.exists());
        assert!(store.entries.is_empty());
    }
    #[test]
    fn truncated_files_are_not_offered_as_downloads() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::load(root.path().to_path_buf()).unwrap();
        let track = Track {
            rating_key: "x".into(),
            ..Default::default()
        };
        let id = Store::id("source", &track);
        store.entries.insert(
            id,
            Entry {
                library: "source".into(),
                library_name: "Source".into(),
                track,
                bytes: 12,
                artwork: false,
            },
        );
        store.save().unwrap();
        assert!(Store::load(root.path().into()).unwrap().entries.is_empty());
    }
}
