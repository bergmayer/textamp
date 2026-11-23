//! Syncs selected WebDAV folder trees using their original paths.
use super::*;
use crate::library::{FolderLocation, FolderSource, MAX_ENTRIES, is_audio, webdav};

const NOMEDIA: &str = ".nomedia";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebdavSelection {
    #[serde(default)]
    pub folders: BTreeSet<String>,
    #[serde(default = "enabled")]
    pub nomedia: bool,
}
impl Default for WebdavSelection {
    fn default() -> Self {
        Self {
            folders: BTreeSet::new(),
            nomedia: true,
        }
    }
}

pub(super) fn valid_path(path: &str) -> bool {
    relative(path)
        && path.split('/').all(|part| {
            !part.is_empty()
                && !matches!(part, "." | ".." | INDEX | MARKER | NOMEDIA)
                && !part.contains(['\\', ':', '*', '?', '"', '<', '>', '|', '\0'])
                && !part.ends_with(['.', ' '])
        })
}

/// A cached download's device record also retains the WebDAV validator when available.
struct DeviceCopy<'a> {
    version: Option<&'a str>,
    intact: bool,
}

impl<'a> DeviceCopy<'a> {
    fn from_entry(entry: &'a Entry, intact: bool) -> Self {
        let signature = entry.signature.as_str();
        let version = if signature.starts_with("download:") {
            signature.split_once('\n').map(|(_, version)| version)
        } else {
            Some(signature)
        };
        Self {
            version: version.filter(|v| !v.is_empty()),
            intact,
        }
    }

    fn validator(&self) -> Option<&'a str> {
        self.version.filter(|_| self.intact)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileDecision {
    Add,
    Keep,
    Repair,
    Changed,
    Verify,
}

impl FileDecision {
    fn compare(copy: Option<&DeviceCopy<'_>>, version: &str) -> Self {
        match copy {
            None => Self::Add,
            Some(copy) if !copy.intact => Self::Repair,
            Some(copy) if copy.version.is_none() || version.is_empty() => Self::Verify,
            Some(copy) if copy.version == Some(version) => Self::Keep,
            Some(_) => Self::Changed,
        }
    }

    fn report(self) -> (&'static str, &'static str) {
        match self {
            Self::Add => (ADD, ""),
            Self::Keep => (KEEP, ""),
            Self::Repair => (UPDATE, "Player file is missing or its size changed"),
            Self::Changed => (UPDATE, "WebDAV version changed"),
            Self::Verify => (UPDATE, "Version unavailable; verify file"),
        }
    }
}

struct PlannedFile<'a> {
    path: &'a String,
    decision: FileDecision,
    prior_version: Option<String>,
    observed_version: String,
}

pub async fn sync_webdav(
    source: &FolderSource,
    target: &volumes::Target,
    volume: &Volume,
    selection: &WebdavSelection,
    downloads: &downloads::Store,
    progress: impl FnMut(Progress),
) -> Result<Summary> {
    sync_webdav_device(
        source, &source.id, target, volume, selection, downloads, progress,
    )
    .await
}
pub async fn sync_webdav_device(
    source: &FolderSource,
    device_id: &str,
    target: &volumes::Target,
    volume: &Volume,
    selection: &WebdavSelection,
    downloads: &downloads::Store,
    progress: impl FnMut(Progress),
) -> Result<Summary> {
    review_webdav(
        source,
        device_id,
        target,
        volume,
        selection,
        downloads,
        progress,
        |_| async { Ok(()) },
    )
    .await
}
pub async fn review_webdav<F: Future<Output = Result<()>>>(
    source: &FolderSource,
    device_id: &str,
    target: &volumes::Target,
    volume: &Volume,
    selection: &WebdavSelection,
    downloads: &downloads::Store,
    mut progress: impl FnMut(Progress),
    review: impl FnOnce(Report) -> F,
) -> Result<Summary> {
    ensure!(
        matches!(source.location, FolderLocation::Webdav { .. }),
        "Expected a WebDAV library"
    );
    let downloads = DownloadCopies::new(downloads);
    let mut pending: Vec<_> = selection.folders.iter().cloned().collect();
    let mut visited = BTreeSet::new();
    source.validate()?;
    let mut files = BTreeMap::new();
    while let Some(folder) = pending.pop() {
        if !visited.insert(folder.clone()) {
            continue;
        }
        ensure!(
            folder.is_empty() || valid_path(&folder),
            "A WebDAV folder name cannot be stored on this player"
        );
        for (entry, version) in webdav::list_with_versions(source, &folder).await? {
            if !entry.directory && !is_audio(&entry.name) {
                continue;
            }
            ensure!(
                valid_path(&entry.path),
                "A WebDAV path cannot be stored as-is on this player: {}",
                entry.path
            );
            if entry.directory {
                pending.push(entry.path);
            } else {
                files.insert(entry.path, version);
            }
        }
        ensure!(
            visited.len() + pending.len() + files.len() <= MAX_ENTRIES,
            "WebDAV selection has too many entries"
        );
    }
    let mut folded = BTreeSet::new();
    ensure!(
        files.keys().all(|path| folded.insert(path.to_lowercase())),
        "Two WebDAV files have the same name on a case-insensitive player"
    );

    let (root, mut manifest) = open_index(target, volume, &volume.uuid, device_id, false).await?;
    ensure!(
        manifest.playlists.is_empty()
            && manifest.retired.is_empty()
            && manifest
                .files
                .iter()
                .all(|(path, entry)| path == &entry.path
                    && valid_path(path)
                    && is_audio(path)
                    && entry.bytes > 0),
        "Invalid WebDAV player sync index"
    );
    recover_publication(target, &root, &mut manifest, Layout::Webdav).await?;
    let mut report = change_report();
    let mut decisions = Vec::with_capacity(files.len());
    for (path, listed_version) in &files {
        let destination = planned_path(&root, path).await?;
        let metadata = match tokio::fs::metadata(&destination).await {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let copy = manifest.files.get(path).map(|entry| {
            DeviceCopy::from_entry(
                entry,
                metadata
                    .as_ref()
                    .is_some_and(|m| m.is_file() && m.len() == entry.bytes),
            )
        });
        ensure!(
            copy.is_some() || metadata.is_none(),
            "A file already uses this WebDAV path on the player: {path}"
        );
        let validator = copy.as_ref().and_then(DeviceCopy::validator);
        let remote = if validator.is_some() && validator != Some(listed_version.as_str()) {
            webdav::version(source, path).await?
        } else {
            None
        };
        let version = remote
            .as_ref()
            .map(|(version, _)| version.as_str())
            .filter(|v| !v.is_empty())
            .unwrap_or(listed_version);
        let decision = FileDecision::compare(copy.as_ref(), version);
        let (group, reason) = decision.report();
        report
            .get_mut(group)
            .unwrap()
            .push(report_line(path, reason));
        decisions.push(PlannedFile {
            path,
            decision,
            prior_version: copy.and_then(|c| c.version.map(str::to_owned)),
            observed_version: version.to_owned(),
        });
    }
    report.get_mut(REMOVE).unwrap().extend(
        manifest
            .files
            .keys()
            .filter(|path| !files.contains_key(*path))
            .cloned(),
    );
    let mut desired = BTreeSet::new();
    if selection.nomedia {
        desired.insert(NOMEDIA.to_owned());
        for folder in &visited {
            let mut parent = PathBuf::new();
            for component in Path::new(folder).components() {
                parent.push(component);
                desired.insert(parent.join(NOMEDIA).to_string_lossy().into_owned());
            }
        }
    }
    for path in &desired {
        if !tokio::fs::try_exists(planned_path(&root, path).await?).await? {
            report
                .entry("Media scanner".into())
                .or_default()
                .push(report_line(path, "Add marker"));
        }
    }
    for path in manifest
        .marker_paths
        .iter()
        .map(String::as_str)
        .chain(manifest.nomedia.then_some(NOMEDIA))
    {
        if !desired.contains(path) && tokio::fs::try_exists(safe_path(&root, path).await?).await? {
            report
                .entry("Media scanner".into())
                .or_default()
                .push(report_line(path, "Remove owned marker"));
        }
    }
    review(report).await?;
    validate_review(target, volume, &manifest).await?;
    for plan in &decisions {
        if plan.decision == FileDecision::Keep {
            check_reviewed_copy(&root, &manifest.files[plan.path]).await?;
        }
    }
    reconcile_markers(target, &root, &mut manifest, desired).await?;
    let mut status = Progress {
        total: files.len(),
        ..Default::default()
    };
    let mut summary = Summary::default();
    for PlannedFile {
        path,
        decision,
        prior_version,
        observed_version,
    } in decisions
    {
        let prior_version = prior_version.as_deref();
        let validator = prior_version.filter(|_| decision != FileDecision::Repair);
        status.title = path.rsplit('/').next().unwrap_or(path).to_owned();
        status.received = 0;
        status.expected = None;
        progress(status.clone());
        if decision == FileDecision::Keep {
            summary.reused += 1;
            status.reused += 1;
            status.completed += 1;
            progress(status.clone());
            continue;
        }
        check_volume(target, &root, &volume.uuid).await?;
        let destination = safe_path(&root, path).await?;
        ensure!(
            manifest.files.contains_key(path) || !tokio::fs::try_exists(&destination).await?,
            "Another file now uses {path}. Review the sync again."
        );
        let parent = destination
            .parent()
            .context("Missing WebDAV destination folder")?;
        tokio::fs::create_dir_all(parent).await?;
        let verify = decision == FileDecision::Verify;
        let parent = parent.to_owned();
        let temporary = crate::app::tasks::spawn_blocking(move || {
            (if verify {
                tempfile::NamedTempFile::new()
            } else {
                tempfile::NamedTempFile::new_in(parent)
            })
            .map(|file| file.into_temp_path())
        })
        .await??;
        let mut downloaded = None;
        let origin = TrackOrigin::Folder {
            source_id: source.id.clone(),
            path: path.clone(),
        };
        if matches!(decision, FileDecision::Add | FileDecision::Repair)
            && let Some((file, bytes, signature)) = downloads.open(&origin, None, None).await
        {
            let remote = webdav::version(source, path).await?;
            let current = remote
                .as_ref()
                .map(|(version, _)| version.as_str())
                .filter(|v| !v.is_empty())
                .unwrap_or(&observed_version);
            let expected = remote.as_ref().and_then(|(_, bytes)| *bytes);
            let known_change =
                !current.is_empty() && prior_version.is_some_and(|previous| previous != current);
            let local_signature = if signature.is_empty() {
                current.to_owned()
            } else {
                format!("{signature}\n{current}")
            };
            if !known_change && expected.is_none_or(|size| size == bytes) {
                let mut digest = md5::Context::new();
                downloaded = copy_download(file, bytes, &temporary, &mut digest)
                    .await?
                    .map(|bytes| (bytes, local_signature, format!("{:x}", digest.finalize())));
            }
        }
        let transfer = if downloaded.is_some() {
            downloaded
        } else {
            webdav::transfer(source, path, validator, &temporary, |received, expected| {
                status.received = received;
                status.expected = expected;
                progress(status.clone());
            })
            .await?
        };
        match transfer {
            Some((bytes, signature, digest)) => {
                check_volume(target, &root, &volume.uuid).await?;
                let entry = Entry {
                    path: path.clone(),
                    bytes,
                    signature: if signature.is_empty() {
                        observed_version
                    } else {
                        signature
                    },
                };
                if verify && same_contents(&destination, &temporary, bytes).await? {
                    if manifest.files[path].signature != entry.signature {
                        manifest.files.insert(path.clone(), entry);
                        save(&root, &manifest).await?;
                    }
                    summary.reused += 1;
                    status.reused += 1;
                } else {
                    let staged = if verify {
                        let staged = tempfile::NamedTempFile::new_in(
                            destination.parent().context("Missing WebDAV folder")?,
                        )?
                        .into_temp_path();
                        ensure!(
                            tokio::fs::copy(&temporary, &staged).await? == bytes,
                            "Incomplete player copy"
                        );
                        check_volume(target, &root, &volume.uuid).await?;
                        staged
                    } else {
                        temporary
                    };
                    publish_file(
                        target,
                        &root,
                        &mut manifest,
                        PendingFile {
                            song_id: None,
                            entry,
                            digest,
                        },
                        &staged,
                    )
                    .await?;
                    summary.copied += 1;
                    summary.bytes += bytes;
                    status.copied += 1;
                }
            }
            None => {
                summary.reused += 1;
                status.reused += 1;
            }
        }
        status.completed += 1;
        progress(status.clone());
    }
    let removed: Vec<_> = manifest
        .files
        .keys()
        .filter(|path| !files.contains_key(*path))
        .cloned()
        .collect();
    for path in removed {
        check_volume(target, &root, &volume.uuid).await?;
        match tokio::fs::remove_file(safe_path(&root, &path).await?).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        manifest.files.remove(&path);
        summary.removed += 1;
    }
    manifest.synced_folders = Some(selection.folders.clone());
    save(&root, &manifest).await?;
    Ok(summary)
}

async fn reconcile_markers(
    target: &volumes::Target,
    root: &Path,
    manifest: &mut Manifest,
    desired: BTreeSet<String>,
) -> Result<()> {
    if manifest.nomedia {
        manifest.marker_paths.insert(NOMEDIA.into());
        manifest.nomedia = false;
    }
    let obsolete: Vec<_> = manifest
        .marker_paths
        .difference(&desired)
        .cloned()
        .collect();
    for name in obsolete {
        ensure!(
            name == NOMEDIA || name.strip_suffix("/.nomedia").is_some_and(valid_path),
            "Invalid media marker path"
        );
        check_volume(target, root, &manifest.volume_uuid).await?;
        let path = safe_path(root, &name).await?;
        if tokio::fs::try_exists(&path).await? && tokio::fs::metadata(&path).await?.len() == 0 {
            tokio::fs::remove_file(path).await?;
        }
        manifest.marker_paths.remove(&name);
    }
    for name in desired {
        let dest = safe_path(root, &name).await?;
        if tokio::fs::try_exists(&dest).await? {
            continue;
        }
        check_volume(target, root, &manifest.volume_uuid).await?;
        tokio::fs::create_dir_all(dest.parent().context("Missing marker parent")?).await?;
        match tokio::fs::File::create_new(&dest).await {
            Ok(file) => {
                file.sync_all().await?;
                manifest.marker_paths.insert(name);
                save(root, manifest).await?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
    }
    save(root, manifest).await
}
