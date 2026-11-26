//! Library-scoped cache jobs. They never activate a source or touch playback.
use super::*;
use crate::app::action::SettingsAction;
use crate::library::cache::{REFRESH_INTERVAL, Store};
use crate::library::tree::{KEY, Tree};

/// Measured cache usage and current work for one saved library.
#[derive(Clone)]
pub struct CacheLibrary {
    pub choice: crate::app::sources::LibraryChoice,
    pub metadata: String,
    pub artwork: String,
    pub status: String,
    pub running: bool,
    pub controls: [(super::options::OptionAction, &'static str); 3],
}
/// Project cache management from the shared library state.
pub fn summaries(state: &AppState) -> Vec<CacheLibrary> {
    crate::app::sources::library_choices(state)
        .into_iter()
        .map(|choice| {
            let key = choice.artwork_key();
            let metadata = state
                .settings_state
                .cache_entries
                .iter()
                .find(|(c, _)| c.artwork_key() == key)
                .map(|(_, size)| match size {
                    Ok(bytes) => crate::util::format_bytes(*bytes),
                    Err(error) => error.clone(),
                })
                .unwrap_or_else(|| "Measuring…".into());
            let artwork = key
                .as_ref()
                .and_then(|k| state.settings_state.artwork_cache_entries.get(k))
                .map(|(count, bytes)| {
                    format!("{count} images · {}", crate::util::format_bytes(*bytes))
                })
                .unwrap_or_else(|| "Measuring…".into());
            let status = crate::app::sources::cache::status(&choice, state);
            let running = crate::app::sources::cache::running(&choice, state);
            let controls = super::options::cache_controls(&choice, state);
            CacheLibrary {
                choice,
                metadata,
                artwork,
                status,
                running,
                controls,
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub folder_radio: Option<crate::library::tree::FolderRadio>,
    pub updated: u64,
    pub warning: Option<String>,
}
impl ScanResult {
    fn fresh(warning: Option<String>) -> Self {
        Self {
            folder_radio: None,
            updated: crate::library::cache::now(),
            warning,
        }
    }
}
#[derive(Debug, Clone)]
pub enum Work {
    Scan(TaskLease),
    Warm(TaskLease),
    Clear(TaskLease),
    Idle,
    Cleared,
}
impl Work {
    fn scan_task(&self) -> Option<&TaskLease> {
        match self {
            Self::Scan(task) | Self::Warm(task) => Some(task),
            Self::Clear(_) | Self::Idle | Self::Cleared => None,
        }
    }
}
#[derive(Debug, Clone)]
pub struct ScanState {
    pub request: u64,
    pub work: Work,
    pub message: String,
    pub checked: u64,
}
fn scope(choice: &LibraryChoice) -> Option<String> {
    choice
        .cache_store()
        .ok()
        .map(|store| store.scope_key().to_owned())
}
pub fn status(choice: &LibraryChoice, state: &AppState) -> String {
    scope(choice)
        .and_then(|key| state.settings_state.cache_scans.get(&key))
        .map(|scan| scan.message.clone())
        .unwrap_or_else(|| match choice {
            LibraryChoice::Navidrome { .. } => {
                "Artists and albums; all playlists and their track lists; album artwork; enabled AudioMuse analysis".into()
            }
            _ => "Folder tree; album contents load when opened".into(),
        })
}
pub fn running(choice: &LibraryChoice, state: &AppState) -> bool {
    scope(choice)
        .and_then(|key| state.settings_state.cache_scans.get(&key))
        .is_some_and(|scan| scan.work.scan_task().is_some())
}
pub fn cancel(choice: &LibraryChoice, state: &mut AppState) {
    if let Some(key) = scope(choice)
        && let Some(scan) = state.settings_state.cache_scans.get_mut(&key)
    {
        if let Some(task) = scan.work.scan_task() {
            task.cancel();
        } else {
            return;
        }
        scan.work = Work::Idle;
        scan.message = "Re-scan cancelled".into();
        scan.request = scan.request.wrapping_add(1);
    }
}
pub fn start(choice: LibraryChoice, manual: bool, state: &mut AppState, tx: &mpsc::Sender<Event>) {
    let Some(key) = scope(&choice) else { return };
    if let Some(scan) = state.settings_state.cache_scans.get(&key) {
        if scan.work.scan_task().is_some() || matches!(scan.work, Work::Clear(_)) {
            if manual {
                cancel(&choice, state);
            }
            return;
        }
        if !manual && matches!(scan.work, Work::Cleared) {
            return;
        }
        if !manual
            && !crate::library::cache::refresh_due(scan.checked, crate::library::cache::now())
        {
            return;
        }
    }
    let analysis = sonic::choice_key(&choice, state).and_then(|key| {
        (!state.sources.sonic_disabled_libraries.contains(&key))
            .then(|| {
                state
                    .sources
                    .audiomuse
                    .connections
                    .get(&key)
                    .cloned()
                    .map(|connection| (key, connection))
            })
            .flatten()
    });
    let target = choice.clone();
    launch_scan(choice, key, manual, state, tx, Work::Scan, async move {
        rescan(&target, analysis, manual).await
    });
}

/// Fill missing playlist and artwork caches from the installed catalog.
pub(super) fn warm_catalog(state: &mut AppState, tx: &mpsc::Sender<Event>) {
    let Some(session) = state.sources.active.navidrome().cloned() else {
        return;
    };
    let choice = LibraryChoice::Navidrome {
        source: session.source.clone(),
        folder: session.client.folder.clone(),
        name: String::new(),
    };
    let Some(key) = scope(&choice) else { return };
    if state
        .settings_state
        .cache_scans
        .get(&key)
        .is_some_and(|scan| matches!(scan.work, Work::Scan(_) | Work::Clear(_) | Work::Cleared))
    {
        return;
    }
    let Some(library) = choice.artwork_key() else {
        return;
    };
    let playlists = state.library.playlists.clone();
    let albums = state.library.albums.clone();
    launch_scan(choice, key, false, state, tx, Work::Warm, async move {
        navidrome::playlists::warm(&session, &library, playlists, albums, false).await?;
        Ok(ScanResult::fresh(None))
    });
}

fn launch_scan(
    choice: LibraryChoice,
    key: String,
    manual: bool,
    state: &mut AppState,
    tx: &mpsc::Sender<Event>,
    work: fn(TaskLease) -> Work,
    job: impl std::future::Future<Output = anyhow::Result<ScanResult>> + Send + 'static,
) {
    state.settings_state.scan_request = state.settings_state.scan_request.wrapping_add(1);
    let request = state.settings_state.scan_request;
    let tx = tx.clone();
    let task = crate::app::tasks::spawn(async move {
        let result = job.await.map_err(|e| format!("{e:#}"));
        let _ = tx
            .send(Event::Effect(
                SettingsAction::SourceCacheScanned {
                    choice,
                    request,
                    manual,
                    result,
                }
                .into(),
            ))
            .await;
    });
    state.settings_state.cache_scans.insert(
        key,
        ScanState {
            request,
            work: work(TaskLease::new(&task)),
            message: "Scanning library…".into(),
            checked: crate::library::cache::now(),
        },
    );
}
pub fn clear(choice: LibraryChoice, state: &mut AppState, tx: &mpsc::Sender<Event>) {
    cancel(&choice, state);
    let Some(key) = scope(&choice) else { return };
    if state
        .settings_state
        .cache_scans
        .get(&key)
        .is_some_and(|scan| matches!(scan.work, Work::Clear(_)))
    {
        return;
    }
    cancel(&choice, state);
    if active(&choice, state) {
        state.sources.listing = None;
        state.sources.list_id = state.sources.list_id.wrapping_add(1);
        state.library_loading = false;
        state.library.artists_loading = false;
        state.library.playlists_loading = false;
        state.library.albums_loading = false;
        state.artist_nav.loading = false;
        state.playlist_nav.loading = false;
        if let Some(folders) = &mut state.folder_state {
            folders.loading = false;
        }
        state.sources.nav_tasks.remove(audiomuse::SYNC_SLOT);
        // Keep visible in-memory data. A cleared cache is not immediately refilled.
        state.cache_mgmt.category_timestamps.insert(
            crate::app::state::RefreshCategory::Artists,
            crate::library::cache::now(),
        );
    }
    state.settings_state.scan_request = state.settings_state.scan_request.wrapping_add(1);
    let request = state.settings_state.scan_request;
    let tx = tx.clone();
    let task = crate::app::tasks::spawn_blocking(move || {
        let result = choice
            .cache_store()
            .and_then(|store| store.clear())
            .map_err(|e| e.to_string());
        let _ = tx.blocking_send(Event::Effect(
            SettingsAction::SourceCacheCleared {
                choice,
                request,
                result,
            }
            .into(),
        ));
    });
    state.settings_state.cache_scans.insert(
        key,
        ScanState {
            request,
            work: Work::Clear(TaskLease::new(&task)),
            checked: crate::library::cache::now(),
            message: "Clearing cache…".into(),
        },
    );
}
pub fn cleared(
    choice: &LibraryChoice,
    request: u64,
    result: &Result<usize, String>,
    state: &mut AppState,
) -> bool {
    let Some(scan) = scope(choice).and_then(|key| state.settings_state.cache_scans.get_mut(&key))
    else {
        return false;
    };
    if scan.request != request || !matches!(scan.work, Work::Clear(_)) {
        return false;
    }
    scan.work = if result.is_ok() {
        Work::Cleared
    } else {
        Work::Idle
    };
    scan.message = match result {
        Ok(_) => "Cache cleared; re-scan or reopen to rebuild".into(),
        Err(error) => format!("Clear failed: {error}"),
    };
    true
}
pub fn completed(
    choice: &LibraryChoice,
    request: u64,
    result: &Result<ScanResult, String>,
    state: &mut AppState,
) -> bool {
    let Some(scan) = scope(choice).and_then(|key| state.settings_state.cache_scans.get_mut(&key))
    else {
        return false;
    };
    if scan.request != request || scan.work.scan_task().is_none() {
        return false;
    }
    scan.work = Work::Idle;
    scan.checked = result
        .as_ref()
        .map_or_else(|_| crate::library::cache::now(), |r| r.updated);
    scan.message = match result {
        Ok(ScanResult { warning: None, .. }) => "Library up to date".into(),
        Ok(ScanResult {
            warning: Some(warning),
            ..
        }) => format!("Catalog refreshed; {warning}"),
        Err(error) => format!("Re-scan failed: {error}"),
    };
    true
}
/// A user-cleared cache stays empty until explicit refresh or reopening.
pub fn maintenance_paused(state: &AppState) -> bool {
    super::cache_store(state)
        .and_then(Result::ok)
        .and_then(|store| state.settings_state.cache_scans.get(store.scope_key()))
        .is_some_and(|scan| matches!(scan.work, Work::Clear(_) | Work::Cleared))
}
pub fn resume(choice: &LibraryChoice, state: &mut AppState) {
    if let Some(key) = scope(choice)
        && state
            .settings_state
            .cache_scans
            .get(&key)
            .is_some_and(|scan| matches!(scan.work, Work::Cleared))
    {
        state.settings_state.cache_scans.remove(&key);
    }
}
pub fn active(choice: &LibraryChoice, state: &AppState) -> bool {
    choice
        .cache_store()
        .ok()
        .zip(super::cache_store(state).and_then(Result::ok))
        .is_some_and(|(a, b)| a.same_scope(&b))
}

/// Build before replacing; each ticket predates network work so Clear invalidates
/// even a write already handed to a blocking worker.
async fn rescan(
    choice: &LibraryChoice,
    analysis: Option<(String, crate::audiomuse::Connection)>,
    manual: bool,
) -> anyhow::Result<ScanResult> {
    let store = choice.cache_store()?;
    match choice {
        LibraryChoice::Folder(source) => {
            let ticket = store.ticket(KEY);
            if !manual {
                let read = ticket.clone();
                if let Some(hit) =
                    crate::app::tasks::spawn_blocking(move || read.read::<Tree>(REFRESH_INTERVAL))
                        .await??
                        .filter(|hit| !hit.stale && hit.value.radio.is_some())
                {
                    return Ok(ScanResult {
                        folder_radio: hit.value.radio,
                        updated: hit.timestamp,
                        warning: None,
                    });
                }
            }
            let tree = crate::library::tree::scan(source).await?;
            let folder_radio = tree.radio.clone();
            crate::app::tasks::spawn_blocking(move || ticket.write(&tree)).await??;
            return Ok(ScanResult {
                folder_radio,
                ..ScanResult::fresh(None)
            });
        }
        LibraryChoice::Navidrome { source, folder, .. } => {
            let ticket = store.ticket("catalog");
            let analysis_ticket = analysis
                .as_ref()
                .map(|(_, connection)| {
                    serde_json::to_string(connection)
                        .map(|json| store.ticket(&format!("audiomuse:{json}")))
                })
                .transpose()?;
            let id = source.id.clone();
            let password = crate::app::tasks::spawn_blocking(move || {
                crate::library::credentials::load_source("navidrome", &id)
            })
            .await??
            .ok_or_else(|| anyhow::anyhow!("Set the library password in Edit connection"))?;
            let mut session = navidrome::Session {
                source: source.clone(),
                client: crate::navidrome::Client::new(
                    source,
                    crate::util::SecretString::from(password.as_str()),
                    source.canonical_folder(folder.clone()),
                )?,
                extensions: Default::default(),
            };
            let catalog = navidrome::catalog::fetch(&mut session, None)
                .await
                .map_err(anyhow::Error::msg)?;
            let ids = catalog.track_ids(&session);
            let playlists = catalog.playlists.clone();
            let albums = catalog.albums.clone();
            let browse = store.ticket("browse-catalog-v1");
            let save_catalog = crate::app::tasks::spawn_blocking(move || {
                let timestamp = crate::library::cache::now();
                ticket.write_at(&catalog, timestamp)?;
                catalog.prepare().save_browse(Some(browse), timestamp);
                Ok::<_, anyhow::Error>(())
            });
            let library_key = format!(
                "navidrome:{}:{}",
                session.source.id,
                session.client.folder.as_deref().unwrap_or("all")
            );
            let analysis_work = async {
                let (Some((key, connection)), Some(ticket)) = (analysis, analysis_ticket) else {
                    return None;
                };
                let sync = async {
                    let read = ticket.clone();
                    let (previous, api) = tokio::try_join!(
                        async move {
                            crate::app::tasks::spawn_blocking(move || {
                                read.read::<crate::audiomuse::Snapshot>(REFRESH_INTERVAL)
                            })
                            .await?
                        },
                        async {
                            let api = audiomuse::client(&connection, &key).await?;
                            api.verify().await?;
                            Ok::<_, anyhow::Error>(api)
                        }
                    )?;
                    let previous = previous.map(|hit| hit.value).unwrap_or_default();
                    let snapshot = api
                        .refresh_resumable(&previous, &ids, &ticket, |_| {})
                        .await?;
                    crate::app::tasks::spawn_blocking(move || {
                        ticket.write_finishing_checkpoints(&snapshot)
                    })
                    .await??;
                    Ok::<_, anyhow::Error>(())
                };
                sync.await
                    .err()
                    .map(|error| format!("AudioMuse: {error:#}"))
            };
            let (saved, playlists, analysis) = tokio::join!(
                save_catalog,
                navidrome::playlists::warm(
                    &session,
                    &library_key,
                    playlists.into(),
                    albums.into(),
                    manual
                ),
                analysis_work
            );
            saved??;
            let issues: Vec<_> = playlists
                .err()
                .map(|error| format!("Playlist cache: {error:#}"))
                .into_iter()
                .chain(analysis)
                .collect();
            Ok(ScanResult::fresh(
                (!issues.is_empty()).then(|| issues.join("; ")),
            ))
        }
        _ => anyhow::bail!("Select a saved library"),
    }
}

/// Cached branch lookup, shared with lazy browsing. Never serves an incomplete
/// listing for a leaf or mixed folder.
pub fn read_folder(
    store: &Store,
    path: &str,
) -> anyhow::Result<Option<crate::library::cache::Hit<Vec<FolderEntry>>>> {
    let mut visited = match store
        .listings()
        .ticket(path)
        .read::<Vec<FolderEntry>>(crate::library::cache::LISTING_TTL)
    {
        Ok(hit) => hit,
        Err(error) => {
            tracing::warn!("Read cached folder listing: {error:#}");
            None
        }
    };
    let tree = match store.ticket(KEY).read::<Tree>(REFRESH_INTERVAL) {
        Ok(tree) => tree,
        Err(error) if visited.is_some() => {
            tracing::warn!("Read folder index: {error:#}");
            return Ok(visited);
        }
        Err(error) => return Err(error),
    };
    if let (Some(visited), Some(tree)) = (&mut visited, &tree) {
        // Re-scanning directories invalidates older demand-loaded file listings
        // without eagerly loading leaf tracks.
        if tree.timestamp > visited.timestamp {
            let mut child = path;
            while !child.is_empty() {
                let parent = child.rsplit_once('/').map_or("", |(parent, _)| parent);
                if tree
                    .value
                    .directories
                    .get(parent)
                    .is_some_and(|entries| !entries.iter().any(|entry| entry.path == child))
                {
                    return Ok(None);
                }
                child = parent;
            }
            visited.stale = true;
        }
    }
    let branch = tree.and_then(|hit| {
        hit.value
            .listing(path)
            .map(|value| crate::library::cache::Hit {
                value,
                stale: hit.stale,
                timestamp: hit.timestamp,
            })
    });
    Ok(match (visited, branch) {
        (Some(visited), Some(branch)) if visited.timestamp > branch.timestamp => Some(visited),
        (_, Some(branch)) => Some(branch),
        (visited, None) => visited,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_folder_tracks_remain_available_while_directory_index_is_fresh() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::at(root.path().into(), "folder-fixture").unwrap();
        let album = FolderEntry {
            path: "artist/album".into(),
            name: "Album".into(),
            directory: true,
        };
        let song = FolderEntry {
            path: "artist/album/song.flac".into(),
            name: "Song".into(),
            directory: false,
        };
        let tree = Tree {
            directories: [
                ("artist".into(), vec![album.clone()]),
                (album.path.clone(), vec![]),
            ]
            .into(),
            ..Default::default()
        };
        store.ticket(KEY).write(&tree).unwrap();
        let ticket = store.listings().ticket(&album.path);
        ticket.write(&vec![song.clone()]).unwrap();
        assert_eq!(
            read_folder(&store, &album.path).unwrap().unwrap().value,
            vec![song.clone()]
        );
        ticket
            .write_at(
                &vec![song.clone()],
                crate::library::cache::now() - crate::library::cache::LISTING_TTL.as_secs() - 1,
            )
            .unwrap();
        let stale = read_folder(&store, &album.path).unwrap().unwrap();
        assert!(stale.stale);
        assert_eq!(stale.value, vec![song.clone()]);
        assert_eq!(
            read_folder(&store, "artist").unwrap().unwrap().value,
            vec![album]
        );
        let removed = "artist/removed/disc";
        store
            .listings()
            .ticket(removed)
            .write_at(&vec![song], 1)
            .unwrap();
        assert!(read_folder(&store, removed).unwrap().is_none());
        assert!(store.bytes().unwrap() > 0);
        assert_eq!(store.clear().unwrap(), 3);
        assert_eq!(store.bytes().unwrap(), 0);
        ticket.write(&Vec::<FolderEntry>::new()).unwrap();
        assert!(
            ticket
                .read::<Vec<FolderEntry>>(crate::library::cache::LISTING_TTL)
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn reading_a_fresh_tree_does_not_extend_its_weekly_refresh_deadline() {
        let root = tempfile::tempdir().unwrap();
        let source = FolderSource {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Offline".into(),
            location: FolderLocation::Local {
                path: root.path().join("missing"),
            },
        };
        let choice = LibraryChoice::Folder(source);
        let stored = crate::library::cache::now() - 6 * 24 * 3600;
        choice
            .cache_store()
            .unwrap()
            .ticket(KEY)
            .write_at(&Tree::default(), stored)
            .unwrap();
        let result = rescan(&choice, None, false).await.unwrap();
        assert_eq!(result.updated, stored);
        assert!(result.warning.is_none());
    }
}
