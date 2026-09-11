//! Library-scoped cache jobs. They never activate a source or touch playback.
use super::*;
use crate::app::action::SettingsAction;
use crate::library::cache::{Store, REFRESH_INTERVAL};
use crate::library::tree::{Tree, KEY};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub updated: u64,
    pub warning: Option<String>,
}
impl ScanResult {
    fn fresh(warning: Option<String>) -> Self {
        Self {
            updated: crate::library::cache::now(),
            warning,
        }
    }
}
#[derive(Debug, Clone)]
pub enum Work {
    Scan(TaskLease),
    Clear(TaskLease),
    Idle,
    Cleared,
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
                "Library catalog and enabled AudioMuse analysis".into()
            }
            _ => "Folder tree; album contents load when opened".into(),
        })
}
pub fn running(choice: &LibraryChoice, state: &AppState) -> bool {
    scope(choice)
        .and_then(|key| state.settings_state.cache_scans.get(&key))
        .is_some_and(|scan| matches!(scan.work, Work::Scan(_)))
}
pub fn cancel(choice: &LibraryChoice, state: &mut AppState) {
    if let Some(key) = scope(choice) {
        if let Some(scan) = state.settings_state.cache_scans.get_mut(&key) {
            if let Work::Scan(task) = &scan.work {
                task.cancel();
            } else {
                return;
            }
            scan.work = Work::Idle;
            scan.message = "Re-scan cancelled".into();
            scan.request = scan.request.wrapping_add(1);
        }
    }
}
pub fn start(choice: LibraryChoice, manual: bool, state: &mut AppState, tx: &mpsc::Sender<Event>) {
    let Some(key) = scope(&choice) else { return };
    if let Some(scan) = state.settings_state.cache_scans.get(&key) {
        if matches!(scan.work, Work::Scan(_) | Work::Clear(_)) {
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
    state.settings_state.scan_request = state.settings_state.scan_request.wrapping_add(1);
    let request = state.settings_state.scan_request;
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
    let tx = tx.clone();
    let task = crate::app::tasks::spawn(async move {
        let result = rescan(&choice, analysis, manual)
            .await
            .map_err(|e| format!("{e:#}"));
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
            work: Work::Scan(TaskLease::new(&task)),
            message: "Scanning metadata… (select Re-scan to cancel)".into(),
            checked: crate::library::cache::now(),
        },
    );
}
pub fn clear(choice: LibraryChoice, state: &mut AppState, tx: &mpsc::Sender<Event>) {
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
    if scan.request != request || !matches!(scan.work, Work::Scan(_)) {
        return false;
    }
    scan.work = Work::Idle;
    scan.checked = result
        .as_ref()
        .map_or_else(|_| crate::library::cache::now(), |r| r.updated);
    scan.message = match result {
        Ok(ScanResult { warning: None, .. }) => "Cache up to date".into(),
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
    if let Some(key) = scope(choice) {
        if state
            .settings_state
            .cache_scans
            .get(&key)
            .is_some_and(|scan| matches!(scan.work, Work::Cleared))
        {
            state.settings_state.cache_scans.remove(&key);
        }
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
                        .filter(|hit| !hit.stale)
                {
                    return Ok(ScanResult {
                        updated: hit.timestamp,
                        warning: None,
                    });
                }
            }
            let tree = crate::library::tree::scan(source).await?;
            crate::app::tasks::spawn_blocking(move || ticket.write(&tree)).await??;
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
            let catalog = navidrome::catalog::fetch(&mut session)
                .await
                .map_err(anyhow::Error::msg)?;
            let ids = catalog.track_ids(&session);
            crate::app::tasks::spawn_blocking(move || ticket.write(&catalog)).await??;
            if let (Some((key, connection)), Some(ticket)) = (analysis, analysis_ticket) {
                let sync = async {
                    let read = ticket.clone();
                    let previous = crate::app::tasks::spawn_blocking(move || {
                        read.read::<crate::audiomuse::Snapshot>(REFRESH_INTERVAL)
                    })
                    .await??
                    .map(|hit| hit.value)
                    .unwrap_or_default();
                    let api = audiomuse::client(&connection, &key).await?;
                    api.verify().await?;
                    let snapshot = api.refresh(&previous, &ids).await?;
                    crate::app::tasks::spawn_blocking(move || ticket.write(&snapshot)).await??;
                    Ok::<_, anyhow::Error>(())
                };
                match tokio::time::timeout(Duration::from_secs(600), sync).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => return Ok(ScanResult::fresh(Some(format!("AudioMuse: {e:#}")))),
                    Err(_) => {
                        return Ok(ScanResult::fresh(Some(
                            "AudioMuse refresh timed out".into(),
                        )))
                    }
                }
            }
        }
        _ => anyhow::bail!("Select a saved library"),
    }
    Ok(ScanResult::fresh(None))
}

/// Cached branch lookup, shared with lazy browsing. Never serves an incomplete
/// listing for a leaf or mixed folder.
pub fn read_folder(
    store: &Store,
    path: &str,
) -> anyhow::Result<Option<crate::library::cache::Hit<Vec<FolderEntry>>>> {
    let mut visited = store
        .ticket(path)
        .read::<Vec<FolderEntry>>(REFRESH_INTERVAL)?;
    let tree = store.ticket(KEY).read::<Tree>(REFRESH_INTERVAL)?;
    if let (Some(visited), Some(tree)) = (&mut visited, &tree) {
        // Re-scanning directories invalidates older demand-loaded file listings
        // without eagerly loading leaf tracks.
        visited.stale |= tree.timestamp > visited.timestamp;
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
