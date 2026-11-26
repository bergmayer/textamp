//! Playlist-to-player transfers. Files are written directly to the selected volume.
use crate::{
    library::{models::Track, track::TrackOrigin},
    navidrome::Client,
    services::downloads,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Component, Path, PathBuf},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub mod mtp;
mod scrobbles;
pub mod volumes;
mod webdav;
pub use volumes::Volume;
pub use webdav::{WebdavSelection, review_webdav, sync_webdav, sync_webdav_device};
const INDEX: &str = ".textamp-sync.json";
const MARKER: &str = ".textamp-volume";
struct DownloadCopies<'a> {
    store: &'a downloads::Store,
    entries: HashMap<&'a TrackOrigin, Vec<(&'a str, &'a downloads::Entry)>>,
}
impl<'a> DownloadCopies<'a> {
    fn new(store: &'a downloads::Store) -> Self {
        let mut entries: HashMap<_, Vec<_>> = HashMap::new();
        for (id, entry) in &store.entries {
            if !matches!(entry.track.origin, TrackOrigin::Unavailable) && entry.bytes > 0 {
                entries
                    .entry(&entry.track.origin)
                    .or_default()
                    .push((id.as_str(), entry));
            }
        }
        Self { store, entries }
    }

    async fn open(
        &self,
        origin: &TrackOrigin,
        expected: Option<u64>,
        metadata: Option<&str>,
    ) -> Option<(tokio::fs::File, u64, String)> {
        for &(id, entry) in self.entries.get(origin)? {
            let bytes = entry.bytes;
            if metadata
                .is_some_and(|wanted| track_signature(&entry.track).ok().as_deref() != Some(wanted))
            {
                continue;
            }
            if expected.is_some_and(|expected| expected != bytes) {
                continue;
            }
            let Some(path) = self.store.audio(id) else {
                continue;
            };
            let Ok(file) = tokio::fs::File::open(path).await else {
                continue;
            };
            if let Ok(meta) = file.metadata().await
                && meta.is_file()
                && meta.len() == bytes
            {
                let signature = meta
                    .modified()
                    .ok()
                    .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|modified| format!("download:{id}:{bytes}:{}", modified.as_nanos()))
                    .unwrap_or_default();
                return Some((file, bytes, signature));
            }
        }
        None
    }
}

async fn copy_download(
    mut source: tokio::fs::File,
    bytes: u64,
    path: &Path,
    digest: &mut md5::Context,
) -> Result<Option<u64>> {
    let mut output = tokio::fs::File::create(path).await?;
    let mut input = (&mut source).take(bytes.saturating_add(1));
    let mut buffer = vec![0; 128 * 1024];
    let mut copied = 0;
    loop {
        let n = input.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        digest.consume(&buffer[..n]);
        output.write_all(&buffer[..n]).await?;
        copied += n as u64;
    }
    if copied != bytes {
        return Ok(None);
    }
    output.sync_all().await?;
    Ok(Some(bytes))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Player {
    pub volume_uuid: String,
    pub name: String,
    pub source_id: String,
    #[serde(default)]
    pub playlists: Vec<String>,
    #[serde(default = "enabled")]
    pub auto_sync: bool,
}
fn enabled() -> bool {
    true
}
#[derive(Clone, Debug)]
pub struct Song {
    pub id: String,
    pub track: Track,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub completed: usize,
    pub total: usize,
    pub copied: usize,
    pub reused: usize,
    pub received: u64,
    pub expected: Option<u64>,
    pub title: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub listens: usize,
    pub unconfirmed_listens: usize,
    pub copied: usize,
    pub reused: usize,
    pub removed: usize,
    pub bytes: u64,
    pub playlist: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    path: String,
    bytes: u64,
    signature: String,
}
fn playlist_path(path: &str) -> bool {
    relative(path) && !path.contains('/') && path.ends_with(".m3u")
}
fn audio_manifest_path(path: &str) -> bool {
    relative(path) && Path::new(path).components().count() == 3
}
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Manifest {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    listening_tags: BTreeMap<String, scrobbles::Tags>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    marker_paths: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_file: Option<PendingFile>,
    version: u8,
    volume_uuid: String,
    source_id: String,
    playlists: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    nomedia: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    synced_folders: Option<BTreeSet<String>>,
    files: BTreeMap<String, Entry>,
    #[serde(default)]
    retired: Vec<String>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PendingFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    song_id: Option<String>,
    entry: Entry,
    digest: String,
}
async fn digest(path: &Path) -> Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hash = md5::Context::new();
    let mut buffer = vec![0; 128 * 1024];
    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            return Ok(format!("{:x}", hash.finalize()));
        }
        hash.consume(&buffer[..n]);
    }
}
enum Layout {
    Playlist,
    Webdav,
}

async fn recover_publication(
    target: &volumes::Target,
    root: &Path,
    manifest: &mut Manifest,
    layout: Layout,
) -> Result<Option<Manifest>> {
    if manifest.pending_file.is_none() {
        return Ok(None);
    }
    let original = manifest.clone();
    let pending = manifest.pending_file.take().expect("pending publication");
    let entry = &pending.entry;
    ensure!(
        entry.bytes > 0
            && match (&pending.song_id, layout) {
                (Some(id), Layout::Playlist) =>
                    !id.is_empty()
                        && audio_manifest_path(&entry.path)
                        && !manifest.files.iter().any(|(other, file)| other != id
                            && file.path.eq_ignore_ascii_case(&entry.path)),
                (None, Layout::Webdav) =>
                    webdav::valid_path(&entry.path) && crate::library::is_audio(&entry.path),
                _ => false,
            },
        "Invalid pending player file"
    );
    check_volume(target, root, &manifest.volume_uuid).await?;
    let dest = safe_path(root, &entry.path).await?;
    if tokio::fs::try_exists(&dest).await?
        && tokio::fs::metadata(&dest).await?.len() == entry.bytes
        && {
            target.hydrate(&dest).await?;
            digest(&dest).await? == pending.digest
        }
    {
        pending.commit(manifest);
    }
    Ok(Some(original))
}

impl PendingFile {
    fn commit(self, manifest: &mut Manifest) {
        let key = self.song_id.unwrap_or_else(|| self.entry.path.clone());
        if let Some(previous) = manifest.files.get(&key)
            && previous.path != self.entry.path
        {
            manifest.retired.push(previous.path.clone());
        }
        manifest.files.insert(key, self.entry);
    }
}

/// Register the destination before renaming. Recovery checks the published bytes
/// before adopting a file after a disconnect between the rename and index save.
async fn publish_file(
    target: &volumes::Target,
    root: &Path,
    manifest: &mut Manifest,
    pending: PendingFile,
    staged: &Path,
) -> Result<()> {
    check_volume(target, root, &manifest.volume_uuid).await?;
    let destination = safe_path(root, &pending.entry.path).await?;
    manifest.pending_file = Some(pending);
    save(target, root, manifest).await?;
    check_volume(target, root, &manifest.volume_uuid).await?;
    target.publish(staged, &destination).await?;
    tokio::fs::rename(staged, destination).await?;
    manifest
        .pending_file
        .take()
        .expect("prepared publication")
        .commit(manifest);
    save(target, root, manifest).await
}

/// Recover roots from older file indexes before a user changes their selection.
pub fn synced_folders(
    volume: &Volume,
    source: &str,
    selected: &BTreeSet<String>,
    remember_roots: bool,
) -> Result<BTreeSet<String>> {
    let directory = volume.mount.join(format!("Textamp {}", plain_name(source)));
    let path = directory.join(INDEX);
    for candidate in [&directory, &path] {
        match std::fs::symlink_metadata(candidate) {
            Ok(meta) => ensure!(!meta.is_symlink(), "Unexpected link in player library"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
            Err(e) => return Err(e.into()),
        }
    }
    let file = std::fs::File::open(&path)?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= 32 * 1024 * 1024,
        "Invalid player index"
    );
    let mut manifest: Manifest = serde_json::from_reader(std::io::BufReader::new(
        std::io::Read::take(file, 32 * 1024 * 1024),
    ))?;
    ensure!(
        manifest.version == 2
            && manifest.volume_uuid == volume.uuid
            && manifest.source_id == source,
        "Wrong player index"
    );
    if let Some(roots) = manifest.synced_folders {
        return Ok(roots);
    }
    let mut roots = BTreeSet::new();
    for path in manifest.files.keys() {
        let parent = path.rsplit_once('/').map_or("", |(p, _)| p);
        if let Some(root) = selected.iter().find(|root| contains_folder(root, parent)) {
            roots.insert(root.clone());
        } else {
            roots.insert(parent.to_owned());
        }
    }
    if remember_roots {
        manifest.synced_folders = Some(roots.clone());
        if volume.is_mtp() {
            mtp::session::update_metadata(
                volume,
                path.strip_prefix(&volume.mount)?,
                &std::fs::read(&path)?,
                &manifest,
            )?;
        }
        let mut staged = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer_pretty(&mut staged, &manifest)?;
        staged.as_file().sync_all()?;
        staged.persist(path)?;
    }
    Ok(roots)
}

pub fn contains_folder(parent: &str, path: &str) -> bool {
    parent.is_empty()
        || parent == path
        || path
            .strip_prefix(parent)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn plain_name(text: &str) -> String {
    let mut name = String::new();
    for ch in text.chars() {
        let ch = if ch.is_control() || "/\\:*?\"<>|".contains(ch) {
            '_'
        } else {
            ch
        };
        if name.len() + ch.len_utf8() > 70 {
            break;
        }
        name.push(ch);
    }
    let name = name.trim_matches(|ch: char| ch == '.' || ch.is_whitespace());
    if name.is_empty() {
        "Untitled".into()
    } else {
        name.into()
    }
}
fn track_signature(track: &Track) -> Result<String> {
    let mut fields = serde_json::json!({
        "media": track.media,
        "title": track.title,
        "artist": track.track_artist(),
        "albumArtist": track.artist_name(),
        "album": track.parent_title,
        "index": track.index,
        "disc": track.parent_index,
        "genres": track.genres,
        "year": track.year,
        "duration": track.duration,
        "artwork": track.thumb,
    });
    // Desktop enables serde_json preserve_order; device signatures must match the TUI.
    fields.sort_all_objects();
    Ok(format!("{:x}", md5::compute(serde_json::to_vec(&fields)?)))
}

async fn same_contents(left: &Path, right: &Path, bytes: u64) -> Result<bool> {
    let mut left = tokio::fs::File::open(left).await?;
    if left.metadata().await?.len() != bytes {
        return Ok(false);
    }
    let mut right = tokio::fs::File::open(right).await?;
    let mut a = vec![0; 128 * 1024];
    let mut b = vec![0; a.len()];
    let mut remaining = bytes;
    while remaining > 0 {
        let n = remaining.min(a.len() as u64) as usize;
        left.read_exact(&mut a[..n]).await?;
        right.read_exact(&mut b[..n]).await?;
        if a[..n] != b[..n] {
            return Ok(false);
        }
        remaining -= n as u64;
    }
    Ok(true)
}

fn relative(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}
async fn safe_path(root: &Path, path: &str) -> Result<PathBuf> {
    ensure!(relative(path), "Invalid player manifest path");
    let metadata = tokio::fs::symlink_metadata(root).await?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "The player sync folder was replaced"
    );
    let mut full = root.to_owned();
    for part in Path::new(path).components() {
        full.push(part);
        match tokio::fs::symlink_metadata(&full).await {
            Ok(meta) => ensure!(
                !meta.file_type().is_symlink(),
                "A player sync path is a symbolic link"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(full)
}
async fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut input = tokio::fs::File::open(path).await?.take(limit + 1);
    let mut data = vec![];
    input.read_to_end(&mut data).await?;
    ensure!(
        (data.len() as u64) <= limit,
        "The player sync index is too large"
    );
    Ok(data)
}
async fn save(target: &volumes::Target, root: &Path, manifest: &Manifest) -> Result<()> {
    let path = safe_path(root, INDEX).await?;
    atomic_write(target, &path, &serde_json::to_vec_pretty(manifest)?).await
}
async fn atomic_write(target: &volumes::Target, path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing player folder")?.to_owned();
    let temporary = crate::app::tasks::spawn_blocking(move || {
        tempfile::NamedTempFile::new_in(parent).map(|file| file.into_temp_path())
    })
    .await??;
    let mut output = tokio::fs::File::create(&temporary).await?;
    output.write_all(bytes).await?;
    output.sync_all().await?;
    drop(output);
    target.publish(&temporary, path).await?;
    tokio::fs::rename(&temporary, path).await?;
    Ok(())
}
async fn check_volume(target: &volumes::Target, root: &Path, uuid: &str) -> Result<()> {
    target.check().await?;
    let marker = safe_path(root, MARKER).await?;
    let data = read_limited(&marker, 128)
        .await
        .context("The player sync folder disappeared")?;
    ensure!(
        data == uuid.as_bytes(),
        "The player was disconnected or replaced"
    );
    Ok(())
}
async fn open_index(
    target: &volumes::Target,
    volume: &Volume,
    uuid: &str,
    source_id: &str,
    initialize: bool,
) -> Result<(PathBuf, Manifest)> {
    ensure!(volume.uuid == uuid, "The selected player changed");
    target.check().await?;
    ensure!(
        volume.is_mtp() || target.path == tokio::fs::canonicalize(&volume.mount).await?,
        "The selected player changed"
    );
    let root = target
        .path
        .join(format!("Textamp {}", plain_name(source_id)));
    let empty = || Manifest {
        version: 2,
        volume_uuid: uuid.into(),
        source_id: source_id.into(),
        ..Default::default()
    };
    match tokio::fs::symlink_metadata(&root).await {
        Ok(meta) => ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "Textamp on the player is not a regular folder"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if !initialize {
                return Ok((root, empty()));
            }
            tokio::fs::create_dir(&root).await?
        }
        Err(error) => return Err(error.into()),
    }
    let index = safe_path(&root, INDEX).await?;
    let manifest = match read_limited(&index, 32 * 1024 * 1024).await {
        Ok(bytes) => {
            let manifest: Manifest = serde_json::from_slice(&bytes)
                .context("The player sync index is unreadable; its files were left unchanged")?;
            ensure!(
                manifest.version == 2
                    && manifest.volume_uuid == uuid
                    && manifest.source_id == source_id,
                "This Textamp folder belongs to another player or library"
            );
            manifest
        }
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            ensure!(
                tokio::fs::read_dir(&root)
                    .await?
                    .next_entry()
                    .await?
                    .is_none(),
                "The player already has an unmanaged Textamp folder"
            );
            let manifest = empty();
            if initialize {
                save(target, &root, &manifest).await?;
            }
            manifest
        }
        Err(error) => return Err(error),
    };
    let marker = safe_path(&root, MARKER).await?;
    if !tokio::fs::try_exists(&marker).await? {
        if !initialize {
            return Ok((root, manifest));
        }
        atomic_write(target, &marker, uuid.as_bytes()).await?;
    }
    check_volume(target, &root, uuid).await?;
    Ok((root, manifest))
}

pub type Report = BTreeMap<String, Vec<String>>;
pub const ADD: &str = "Add";
pub const UPDATE: &str = "Update";
pub const REMOVE: &str = "Remove";
pub const KEEP: &str = "Already on player";
pub const CHANGE_GROUPS: [&str; 4] = [ADD, UPDATE, REMOVE, KEEP];
fn change_report() -> Report {
    CHANGE_GROUPS
        .into_iter()
        .map(|key| (key.into(), Vec::new()))
        .collect()
}
fn report_line(path: &str, reason: &str) -> String {
    if reason.is_empty() {
        path.to_owned()
    } else {
        format!("{path} · {reason}")
    }
}
async fn validate_review(
    target: &volumes::Target,
    volume: &Volume,
    manifest: &Manifest,
    before_recovery: Option<&Manifest>,
) -> Result<()> {
    let (root, current) = open_index(
        target,
        volume,
        &manifest.volume_uuid,
        &manifest.source_id,
        true,
    )
    .await?;
    ensure!(
        before_recovery.unwrap_or(manifest) == &current,
        "Player contents changed. Review the sync again."
    );
    if before_recovery.is_some() {
        save(target, &root, manifest).await?;
    }
    Ok(())
}

async fn check_reviewed_copy(root: &Path, entry: &Entry) -> Result<()> {
    let path = safe_path(root, &entry.path).await?;
    ensure!(
        tokio::fs::metadata(path)
            .await
            .is_ok_and(|m| m.is_file() && m.len() == entry.bytes),
        "{} changed on the player. Review the sync again.",
        entry.path
    );
    Ok(())
}

async fn planned_path(root: &Path, path: &str) -> Result<PathBuf> {
    if tokio::fs::try_exists(root).await? {
        safe_path(root, path).await
    } else {
        ensure!(relative(path), "Invalid player path");
        Ok(root.join(path))
    }
}

pub async fn inventory(volume: &Volume, names: &BTreeMap<String, String>) -> Result<Report> {
    let target = volumes::Target::for_volume(volume, None).await?;
    let mut dirs = tokio::fs::read_dir(&target.path).await?;
    let mut result = Report::new();
    while let Some(dir) = dirs.next_entry().await? {
        let name = dir.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_prefix("Textamp ") else {
            continue;
        };
        if !dir.file_type().await?.is_dir() {
            continue;
        }
        let path = safe_path(&dir.path(), INDEX).await?;
        if !tokio::fs::try_exists(&path).await? {
            continue;
        }
        let index: Manifest =
            serde_json::from_slice(&read_limited(&path, 32 * 1024 * 1024).await?)?;
        ensure!(
            index.version == 2
                && index.volume_uuid == volume.uuid
                && plain_name(&index.source_id) == id,
            "Invalid sync index in {name}"
        );
        let label = names
            .get(&index.source_id)
            .map_or_else(|| name.clone(), |source| format!("{source} · {name}"));
        let mut membership: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for playlist in &index.playlists {
            ensure!(playlist_path(playlist), "Invalid indexed playlist path");
            let m3u = safe_path(&dir.path(), playlist).await?;
            if !tokio::fs::try_exists(&m3u).await? {
                continue;
            }
            let bytes = read_limited(&m3u, 32 * 1024 * 1024).await?;
            for line in std::str::from_utf8(&bytes)?
                .lines()
                .filter(|line| relative(line) && !line.starts_with('#'))
            {
                membership
                    .entry(line.into())
                    .or_default()
                    .insert(playlist.trim_end_matches(".m3u").into());
            }
        }
        let mut rows = Vec::new();
        for entry in index.files.into_values() {
            ensure!(relative(&entry.path), "Invalid indexed file path");
            if !tokio::fs::try_exists(safe_path(&dir.path(), &entry.path).await?).await? {
                continue;
            }
            let playlists = membership
                .remove(&entry.path)
                .unwrap_or_default()
                .into_iter()
                .collect::<Vec<_>>()
                .join(", ");
            rows.push(report_line(&entry.path, &playlists));
        }
        rows.sort();
        result.insert(label, rows);
    }
    match read_playback_history(&target.path).await {
        Ok(rows) if !rows.is_empty() => {
            result.insert("Player listening history".into(), rows);
        }
        Err(error) => {
            result.insert(
                "Player listening history".into(),
                vec![format!("Could not read playback log: {error}")],
            );
        }
        _ => {}
    }
    target.check().await?;
    Ok(result)
}

async fn read_playback_history(root: &Path) -> Result<Vec<String>> {
    let log = safe_path(root, ".scrobbler.log").await?;
    match tokio::fs::symlink_metadata(&log).await {
        Ok(meta) => ensure!(meta.is_file(), "Invalid playback log"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    }
    let data = read_limited(&log, 32 * 1024 * 1024).await?;
    Ok(playback_history(std::str::from_utf8(&data)?))
}

fn playback_history(log: &str) -> Vec<String> {
    let mut lines = log.lines();
    if lines.next() != Some("#AUDIOSCROBBLER/1.1") {
        return Vec::new();
    }
    lines
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let artist = fields.next()?;
            let album = fields.next()?;
            let title = fields.next()?;
            let status = fields.nth(2)?;
            let timestamp: i64 = fields.next()?.parse().ok()?;
            if status != "L" || timestamp <= 0 || artist.is_empty() || title.is_empty() {
                return None;
            }
            let date = time::OffsetDateTime::from_unix_timestamp(timestamp).ok()?;
            Some(format!(
                "{artist} · {title} · {album} · {} {:02}:{:02} UTC",
                date.date(),
                date.hour(),
                date.minute()
            ))
        })
        .collect()
}

fn album_folder(song: &Song) -> (String, String) {
    let artist = if song
        .track
        .grandparent_title
        .as_deref()
        .is_some_and(|name| !name.trim().is_empty())
    {
        song.track.artist_name()
    } else {
        song.track.track_artist()
    };
    (plain_name(artist), plain_name(song.track.album_name()))
}

fn album_key(folder: &(String, String)) -> (String, String) {
    (folder.0.to_lowercase(), folder.1.to_lowercase())
}

fn audio_path(song: &Song, folder: &(String, String), number: u64, width: usize) -> Result<String> {
    let extension = song
        .track
        .media
        .first()
        .and_then(|media| {
            media
                .container
                .as_deref()
                .or_else(|| media.part.first()?.container.as_deref())
        })
        .context("The server did not identify the audio format")?
        .to_ascii_lowercase();
    ensure!(
        matches!(
            extension.as_str(),
            "mp3"
                | "wav"
                | "flac"
                | "ape"
                | "ogg"
                | "m4a"
                | "aac"
                | "wma"
                | "mp2"
                | "amr"
                | "mid"
                | "m4r"
                | "mmf"
                | "wv"
                | "aif"
                | "aiff"
                | "au"
                | "ra"
                | "mka"
                | "ac3"
                | "caf"
                | "3gpp"
                | "dff"
        ),
        "Unsupported player audio format: {extension}"
    );
    Ok(format!(
        "{}/{}/{number:0width$} {}.{extension}",
        folder.0,
        folder.1,
        plain_name(&song.track.title)
    ))
}
fn matches_layout(existing: &str, planned: &str) -> bool {
    if existing.eq_ignore_ascii_case(planned) {
        return true;
    }
    let (Some((stem, extension)), Some((existing_stem, existing_extension))) =
        (planned.rsplit_once('.'), existing.rsplit_once('.'))
    else {
        return false;
    };
    extension.eq_ignore_ascii_case(existing_extension)
        && existing_stem
            .rsplit_once(" [")
            .is_some_and(|(base, suffix)| {
                base.eq_ignore_ascii_case(stem)
                    && suffix.strip_suffix(']').is_some_and(|value| {
                        value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                    })
            })
}
fn playlist_bytes(songs: &[Song], files: &BTreeMap<String, (bool, String)>) -> Result<Vec<u8>> {
    let mut text = String::from("#EXTM3U\r\n");
    for song in songs {
        let entry = files
            .get(&song.id)
            .context("A playlist track was not transferred")?;
        text.push_str(&format!(
            "#EXTINF:{},{} - {}\r\n{}\r\n",
            song.track.duration.unwrap_or(0) / 1000,
            song.track.track_artist().replace(['\r', '\n'], " "),
            song.track.title.replace(['\r', '\n'], " "),
            entry.1
        ));
    }
    Ok(text.into_bytes())
}
#[derive(Debug)]
pub struct Playlist {
    pub title: String,
    pub songs: Vec<Song>,
}

/// Runs one serialized USB transfer. Cancelling the future drops its temporary file.
/// Existing playlists remain usable until every requested audio file is committed.
pub async fn sync(
    client: &Client,
    target: &volumes::Target,
    volume: &Volume,
    player: &Player,
    songs: &[Song],
    progress: impl FnMut(Progress),
) -> Result<Summary> {
    sync_playlists(
        client,
        target,
        volume,
        player,
        &[Playlist {
            title: player
                .playlists
                .first()
                .context("No playlist is selected")?
                .clone(),
            songs: songs.to_vec(),
        }],
        &downloads::Store::default(),
        progress,
    )
    .await
}

pub async fn sync_playlists(
    client: &Client,
    target: &volumes::Target,
    volume: &Volume,
    player: &Player,
    playlists: &[Playlist],
    downloads: &downloads::Store,
    progress: impl FnMut(Progress),
) -> Result<Summary> {
    review_playlists(
        client,
        target,
        volume,
        player,
        playlists,
        downloads,
        progress,
        |_| async { Ok(()) },
    )
    .await
}

pub async fn review_playlists<F: Future<Output = Result<()>>>(
    client: &Client,
    target: &volumes::Target,
    volume: &Volume,
    player: &Player,
    playlists: &[Playlist],
    downloads: &downloads::Store,
    mut progress: impl FnMut(Progress),
    review: impl FnOnce(Report) -> F,
) -> Result<Summary> {
    let songs = || playlists.iter().flat_map(|playlist| &playlist.songs);
    let downloads = DownloadCopies::new(downloads);
    let (root, mut manifest) = open_index(
        target,
        volume,
        &player.volume_uuid,
        &player.source_id,
        false,
    )
    .await?;
    ensure!(
        manifest
            .files
            .values()
            .all(|entry| audio_manifest_path(&entry.path) && entry.bytes > 0)
            && manifest
                .retired
                .iter()
                .all(|path| audio_manifest_path(path) || playlist_path(path))
            && manifest.playlists.iter().all(|path| playlist_path(path)),
        "Invalid player manifest paths; files were left unchanged"
    );
    let before_recovery =
        recover_publication(target, &root, &mut manifest, Layout::Playlist).await?;
    let desired: BTreeSet<_> = songs().map(|song| song.id.clone()).collect();
    let mut albums = BTreeMap::new();
    let mut unique = BTreeSet::new();
    for song in songs().filter(|song| unique.insert(&song.id)) {
        let folder = album_folder(song);
        let (_, discs) = albums
            .entry(album_key(&folder))
            .or_insert_with(|| (folder, BTreeMap::new()));
        let (last, unnumbered) = discs
            .entry(song.track.parent_index.unwrap_or(1).max(1))
            .or_insert((0u64, 0u64));
        match song.track.index.filter(|number| *number > 0) {
            Some(number) => *last = (*last).max(u64::from(number)),
            None => *unnumbered += 1,
        }
    }
    let mut layouts = BTreeMap::new();
    for (key, (folder, discs)) in albums {
        let mut offset = 0u64;
        let positions: BTreeMap<_, _> = discs
            .into_iter()
            .map(|(disc, (last, unnumbered))| {
                let position = (disc, (offset, last));
                offset += last + unnumbered;
                position
            })
            .collect();
        layouts.insert(key, (folder, offset.to_string().len().max(2), positions));
    }
    let mut paths = BTreeMap::new();
    let mut required = 0u64;
    let mut planned = BTreeSet::new();
    for song in songs() {
        if planned.insert(&song.id) {
            let folder = album_folder(song);
            let (folder, width, discs) = layouts
                .get_mut(&album_key(&folder))
                .expect("album was indexed");
            let (offset, last) = discs
                .get_mut(&song.track.parent_index.unwrap_or(1).max(1))
                .expect("disc was indexed");
            let number = match song.track.index.filter(|number| *number > 0) {
                Some(number) => u64::from(number),
                None => {
                    *last += 1;
                    *last
                }
            };
            let path = audio_path(song, folder, *offset + number, *width)?;
            if manifest
                .files
                .get(&song.id)
                .is_none_or(|entry| !matches_layout(&entry.path, &path))
            {
                required = required
                    .checked_add(
                        song.track
                            .media
                            .first()
                            .and_then(|media| media.part.first()?.size)
                            .or_else(|| manifest.files.get(&song.id).map(|entry| entry.bytes))
                            .unwrap_or(0),
                    )
                    .context("The playlist is too large")?;
            }
            paths.insert(&song.id, path);
        }
    }
    ensure!(
        volume.available == 0 || required <= volume.available.saturating_sub(1024 * 1024),
        "The player does not have enough free space for this playlist"
    );
    let mut report = change_report();
    let mut reserved = BTreeSet::new();
    let mut reusable_files = BTreeMap::new();
    for song in songs() {
        if reusable_files.contains_key(&song.id) {
            continue;
        }
        let metadata = track_signature(&song.track)?;
        let entry = manifest.files.get(&song.id);
        let same_file = if let Some(old) = entry.filter(|old| old.signature == metadata) {
            let path = safe_path(&root, &old.path).await?;
            tokio::fs::metadata(&path)
                .await
                .is_ok_and(|m| m.is_file() && m.len() == old.bytes)
        } else {
            false
        };
        let (group, reason) = match entry {
            None => (ADD, ""),
            Some(old) if !matches_layout(&old.path, &paths[&song.id]) => {
                (UPDATE, "Album path or track order changed")
            }
            Some(old) if old.signature != metadata => {
                (UPDATE, "Server catalog changed; compare audio")
            }
            Some(_) if !same_file => (UPDATE, "Player file is missing or its size changed"),
            _ => (KEEP, ""),
        };
        let base = &paths[&song.id];
        let mut name = base.clone();
        if let Some(old) = entry.filter(|old| matches_layout(&old.path, base)) {
            name = old.path.clone();
        } else if reserved.contains(&name.to_lowercase())
            || tokio::fs::try_exists(planned_path(&root, &name).await?).await?
        {
            let (stem, extension) = base.rsplit_once('.').context("Missing file extension")?;
            let hash = format!("{:x}", md5::compute(song.id.as_bytes()));
            name = format!("{stem} [{}].{extension}", &hash[..8]);
            while reserved.contains(&name.to_lowercase())
                || tokio::fs::try_exists(planned_path(&root, &name).await?).await?
            {
                let suffix = uuid::Uuid::new_v4().simple().to_string();
                name = format!("{stem} [{}].{extension}", &suffix[..8]);
            }
        }
        reserved.insert(name.to_lowercase());
        report
            .get_mut(group)
            .unwrap()
            .push(report_line(&name, reason));
        reusable_files.insert(song.id.clone(), (same_file, name));
    }
    for (id, entry) in &manifest.files {
        if !desired.contains(id) {
            report.get_mut(REMOVE).unwrap().push(entry.path.clone());
        }
    }
    let mut names = BTreeSet::new();
    let mut publications = Vec::new();
    for playlist in playlists {
        let stem = plain_name(&playlist.title);
        let mut name = format!("{stem}.m3u");
        if !names.insert(name.to_lowercase()) {
            let hash = format!("{:x}", md5::compute(playlist.title.as_bytes()));
            name = format!("{stem} [{}].m3u", &hash[..8]);
            ensure!(
                names.insert(name.to_lowercase()),
                "Two playlists use the same filename"
            );
        }
        let path = planned_path(&root, &name).await?;
        let exists = tokio::fs::try_exists(&path).await?;
        ensure!(
            manifest.playlists.contains(&name) || !exists,
            "An unmanaged playlist already uses that filename"
        );
        let contents = playlist_bytes(&playlist.songs, &reusable_files)?;
        let changed = !exists || read_limited(&path, 32 * 1024 * 1024).await? != contents;
        if changed {
            report
                .entry("Playlists".into())
                .or_default()
                .push(report_line(
                    &name,
                    if exists {
                        "Update track list"
                    } else {
                        "Add playlist"
                    },
                ));
        }
        publications.push((name, path, contents, changed));
    }
    let desired_playlists: BTreeSet<_> = publications
        .iter()
        .map(|(name, _, _, _)| name.clone())
        .collect();
    for name in manifest.playlists.difference(&desired_playlists) {
        report
            .entry("Playlists".into())
            .or_default()
            .push(report_line(name, "Remove playlist"));
    }
    review(report).await?;
    validate_review(target, volume, &manifest, before_recovery.as_ref()).await?;
    for (id, (reusable, _)) in &reusable_files {
        if *reusable {
            check_reviewed_copy(&root, &manifest.files[id]).await?;
        }
    }
    let mut status = Progress {
        total: desired.len(),
        ..Default::default()
    };
    for song in songs() {
        manifest
            .listening_tags
            .insert(song.id.clone(), (&song.track).into());
    }
    let tracks: std::collections::HashMap<_, _> = songs()
        .map(|song| (song.id.as_str(), &song.track))
        .collect();
    let (listens, unconfirmed_listens) =
        scrobbles::import(target, &root, &mut manifest, Some(client), |id| {
            tracks
                .get(id)
                .map(|track| (*track).clone())
                .unwrap_or_else(|| {
                    let key = format!("navidrome:{}:{}", player.source_id, urlencoding::encode(id));
                    Track {
                        origin: TrackOrigin::Navidrome {
                            source_id: player.source_id.clone(),
                            song_id: id.to_owned(),
                        },
                        rating_key: key.clone(),
                        key,
                        ..Default::default()
                    }
                })
        })
        .await?;
    let mut summary = Summary {
        listens,
        unconfirmed_listens,
        ..Default::default()
    };
    let mut done = BTreeSet::new();
    for song in songs() {
        if !done.insert(song.id.clone()) {
            continue;
        }
        check_volume(target, &root, &player.volume_uuid).await?;
        let metadata = track_signature(&song.track)?;
        let existing = manifest.files.get(&song.id);
        let reusable = if let Some(entry) = existing.filter(|_| reusable_files[&song.id].0) {
            Some((entry, safe_path(&root, &entry.path).await?))
        } else {
            None
        };
        status.title = song.track.title.clone();
        status.received = 0;
        status.expected = song
            .track
            .media
            .first()
            .and_then(|media| media.part.first()?.size);
        progress(status.clone());
        if reusable
            .as_ref()
            .is_some_and(|(entry, _)| matches_layout(&entry.path, &paths[&song.id]))
        {
            status.reused += 1;
            summary.reused += 1;
        } else {
            let name = reusable_files[&song.id].1.clone();
            let old = existing.cloned();
            let owned = old.as_ref().filter(|entry| entry.path == name);
            let destination = safe_path(&root, &name).await?;
            ensure!(
                owned.is_some() || !tokio::fs::try_exists(&destination).await?,
                "Another file now uses {name}. Review the sync again."
            );
            let parent = destination.parent().context("Missing audio folder")?;
            tokio::fs::create_dir_all(parent).await?;
            let parent = parent.to_owned();
            let verifying = owned.is_some() && tokio::fs::try_exists(&destination).await?;
            if verifying {
                target.hydrate(&destination).await?;
            }
            let temporary = crate::app::tasks::spawn_blocking(move || {
                if verifying {
                    tempfile::NamedTempFile::new()
                } else {
                    tempfile::NamedTempFile::new_in(parent)
                }
                .map(|file| file.into_temp_path())
            })
            .await??;
            let mut digest = md5::Context::new();
            let local = if let Some((entry, previous)) = reusable {
                target.hydrate(&previous).await?;
                let copied = copy_download(
                    tokio::fs::File::open(previous).await?,
                    entry.bytes,
                    &temporary,
                    &mut digest,
                )
                .await?;
                Some(copied.context("The previous player copy changed during sync")?)
            } else if let Some((file, bytes, _)) = downloads
                .open(
                    &TrackOrigin::Navidrome {
                        source_id: match &song.track.origin {
                            TrackOrigin::Navidrome { source_id, .. } => source_id.clone(),
                            _ => player.source_id.clone(),
                        },
                        song_id: song.id.clone(),
                    },
                    status.expected,
                    Some(&metadata),
                )
                .await
            {
                copy_download(file, bytes, &temporary, &mut digest).await?
            } else {
                None
            };
            let received = match local {
                Some(bytes) => bytes,
                None => {
                    digest = md5::Context::new();
                    client
                        .download_to(
                            &song.id,
                            &temporary,
                            |bytes, total| {
                                status.received = bytes;
                                status.expected = total;
                                progress(status.clone());
                            },
                            Some(&mut digest),
                        )
                        .await?
                }
            };
            ensure!(received > 0, "The server returned an empty audio file");
            check_volume(target, &root, &player.volume_uuid).await?;
            let temporary = if verifying {
                if same_contents(&destination, &temporary, received).await? {
                    manifest.files.insert(
                        song.id.clone(),
                        Entry {
                            path: name,
                            bytes: received,
                            signature: metadata,
                        },
                    );
                    save(target, &root, &manifest).await?;
                    summary.reused += 1;
                    status.reused += 1;
                    status.completed += 1;
                    progress(status.clone());
                    continue;
                }
                let staged = tempfile::NamedTempFile::new_in(destination.parent().unwrap())?
                    .into_temp_path();
                ensure!(
                    tokio::fs::copy(&temporary, &staged).await? == received,
                    "Incomplete player copy"
                );
                check_volume(target, &root, &player.volume_uuid).await?;
                staged
            } else {
                temporary
            };
            publish_file(
                target,
                &root,
                &mut manifest,
                PendingFile {
                    song_id: Some(song.id.clone()),
                    entry: Entry {
                        path: name,
                        bytes: received,
                        signature: metadata,
                    },
                    digest: format!("{:x}", digest.finalize()),
                },
                &temporary,
            )
            .await?;
            status.copied += 1;
            summary.copied += 1;
            summary.bytes += received;
        }
        status.completed += 1;
        progress(status.clone());
    }
    check_volume(target, &root, &player.volume_uuid).await?;
    for (name, path, _, _) in &publications {
        ensure!(
            manifest.playlists.contains(name) || !tokio::fs::try_exists(path).await?,
            "Another file now uses {name}. Review the sync again."
        );
    }
    manifest.playlists.extend(desired_playlists.iter().cloned());
    save(target, &root, &manifest).await?;
    for (_, path, contents, changed) in &publications {
        if !changed {
            continue;
        }
        check_volume(target, &root, &player.volume_uuid).await?;
        atomic_write(target, path, contents).await?;
    }
    manifest
        .retired
        .extend(manifest.playlists.difference(&desired_playlists).cloned());
    let active_paths: BTreeSet<_> = desired_playlists
        .iter()
        .chain(manifest.files.values().map(|entry| &entry.path))
        .map(|path| path.to_lowercase())
        .collect();
    manifest
        .retired
        .retain(|path| !active_paths.contains(&path.to_lowercase()));
    manifest.playlists = desired_playlists;
    save(target, &root, &manifest).await?;
    let remove: Vec<_> = manifest
        .files
        .keys()
        .filter(|id| !desired.contains(*id))
        .cloned()
        .collect();
    for id in remove {
        check_volume(target, &root, &player.volume_uuid).await?;
        let path = safe_path(&root, &manifest.files[&id].path).await?;
        target.remove(&path).await?;
        manifest.files.remove(&id);
        summary.removed += 1;
    }
    for path in &manifest.retired {
        check_volume(target, &root, &player.volume_uuid).await?;
        target.remove(&safe_path(&root, path).await?).await?;
    }
    manifest
        .listening_tags
        .retain(|id, _| manifest.files.contains_key(id));
    manifest.retired.clear();
    save(target, &root, &manifest).await?;
    summary.playlist = publications
        .into_iter()
        .next()
        .map(|(_, path, _, _)| path)
        .unwrap_or_default();
    Ok(summary)
}
