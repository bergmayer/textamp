//! Folder navigation dispatch handlers: LoadFolderRoot, NavigateIntoFolder,
//! NavigateUpFolder, RefreshSubfolder, PlayFolderTracks.

use crate::app::event::*;
use crate::app::event::LibraryEventSender;
use crate::app::{Action, AppState, Event};
use crate::app::action::{AsyncError, FolderAction};
use crate::plex::PlexClient;
use crate::audio::AudioPlayer;
use crate::plex::CachedFolder;
use crate::services::{FolderService, FolderColumn, FolderNavigationState};

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Resolve folder column track items to full Track objects using the all_tracks cache.
/// Returns tracks in column display order (only track items, skipping folders).
/// Returns empty vec if all_tracks cache is not loaded.
fn resolve_folder_column_tracks(state: &AppState) -> Vec<crate::plex::models::Track> {
    if state.library.all_tracks.is_empty() {
        return vec![];
    }
    let col = match state.folder_state.as_ref().and_then(|fs| fs.focused()) {
        Some(c) => c,
        None => return vec![],
    };

    let track_map: std::collections::HashMap<&str, &crate::plex::models::Track> = state.library.all_tracks.iter()
        .map(|t| (t.rating_key.as_str(), t))
        .collect();

    col.items.iter()
        .filter_map(|item| item.rating_key.as_deref())
        .filter_map(|key| track_map.get(key).map(|t| (*t).clone()))
        .collect()
}

/// Derive a folder's filesystem path from its child folders' cached paths.
/// If a child folder has a cached path like `D:\music\10cm\4.0`, the parent is `D:\music\10cm`.
pub(crate) fn derive_path_from_children(
    items: &[crate::plex::models::FolderItem],
    folder_cache: &std::collections::HashMap<String, CachedFolder>,
) -> Option<String> {
    for item in items {
        if !item.is_folder() { continue; }
        // Check if this child folder has a cached entry with a known path
        if let Some(cached) = folder_cache.get(&item.key) {
            if let Some(ref child_path) = cached.path {
                // Take the parent of the child's path
                if let Some(pos) = child_path.rfind(|c: char| c == '/' || c == '\\') {
                    let parent = &child_path[..pos];
                    if !parent.is_empty() {
                        return Some(parent.to_string());
                    }
                }
            }
        }
        // Check if the child item itself has a path from FolderDirectory.path
        if let Some(ref child_path) = item.path {
            if let Some(pos) = child_path.rfind(|c: char| c == '/' || c == '\\') {
                let parent = &child_path[..pos];
                if !parent.is_empty() {
                    return Some(parent.to_string());
                }
            }
        }
    }
    None
}

/// Spawn an async task to discover the filesystem path of a folder by probing a child folder.
///
/// When a folder contains only subdirectories (no tracks), the Plex API doesn't return
/// filesystem paths. This probes the first child folder to find tracks and derive the
/// parent path from their file paths.
pub(crate) fn spawn_path_discovery(
    library_key: &str,
    folder_key: &str,
    items: &[crate::plex::models::FolderItem],
    event_tx: &mpsc::Sender<Event>,
    library_generation: u64,
    client: &PlexClient,
) {
    // Find the first child folder to probe
    let child_key = items.iter()
        .find(|item| item.is_folder())
        .map(|item| item.key.clone());

    if let Some(child_key) = child_key {
        let event_tx = LibraryEventSender::new(event_tx.clone(), library_generation);
        let client = client.clone();
        let lk = library_key.to_string();
        let fk = folder_key.to_string();
        tokio::spawn(async move {
            match client.get_folder_contents(&child_key).await {
                Ok(response) => {
                    // Try to get path from the child folder's contents
                    if let Some(child_path) = FolderService::folder_path(&response) {
                        // child_path is the child folder's path; parent is our folder's path
                        if let Some(pos) = child_path.rfind(|c: char| c == '/' || c == '\\') {
                            let parent = &child_path[..pos];
                            if !parent.is_empty() {
                                let _ = event_tx.send(FolderEvent::FolderPathDiscovered {
                                    library_key: lk,
                                    folder_key: fk,
                                    path: parent.to_string(),
                                }.into()).await;
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!("Path discovery failed for {}: {}", fk, e);
                }
            }
        });
    }
}

/// After pushing a column with a known path, backfill any parent column that's missing a path.
pub(crate) fn backfill_parent_path(folder_state: &mut FolderNavigationState) {
    let num_cols = folder_state.columns.len();
    if num_cols < 2 { return; }
    let child_title = folder_state.columns[num_cols - 1].title.clone();
    if child_title.is_empty() { return; }
    // Derive parent path from child path
    if let Some(pos) = child_title.rfind(|c: char| c == '/' || c == '\\') {
        let parent_path = &child_title[..pos];
        if !parent_path.is_empty() {
            let parent_col = &mut folder_state.columns[num_cols - 2];
            // Only backfill if parent doesn't already have a path-style title
            if parent_col.title.is_empty() {
                parent_col.title = parent_path.to_string();
            }
        }
    }
}

/// Dispatch folder navigation actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: FolderAction,
    state: &mut AppState,
    client: &mut PlexClient,
    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    match action {
        FolderAction::LoadFolderRoot => {
            if let Some(lib_key) = &state.active_library {
                let lib_title = state.libraries.iter()
                    .find(|l| &l.key == lib_key)
                    .map(|l| l.title.clone())
                    .unwrap_or_else(|| "Root".to_string());

                let event_tx =
                    LibraryEventSender::new(event_tx.clone(), state.library_generation);
                let client = client.clone();
                let lk = lib_key.clone();
                let lt = lib_title;
                tokio::spawn(async move {
                    match client.get_library_folders(&lk).await {
                        Ok(response) => {
                            let items = FolderService::from_response(&response);
                            let _ = event_tx.send(FolderEvent::FolderRootLoaded {
                                library_key: lk,
                                lib_title: lt,
                                items,
                            }.into()).await;
                        }
                        Err(e) => {
                            let _ = event_tx.send(FolderEvent::FolderLoadFailed {
                                library_key: lk,
                                pending_folder_key: None,
                                message: format!("Failed to load folders: {}", e),
                            }.into()).await;
                        }
                    }
                });
            }
        }
        FolderAction::NavigateIntoFolder { folder_key, replace_child } => {
            // Get the filesystem path from the selected folder item in the current column
            let item_path = state.folder_state.as_ref()
                .and_then(|fs| fs.focused())
                .and_then(|col| col.selected_item())
                .and_then(|item| item.path.clone());

            // Check cache first for instant navigation
            if let Some(cached_folder) = state.folder_contents_cache.get(&folder_key) {
                state.pending_folder_load = None; // cancel any in-flight load
                tracing::debug!("Folder cache hit: {} ({} items)", folder_key, cached_folder.items.len());
                let folder_title = item_path.clone()
                    .or_else(|| cached_folder.path.clone())
                    .or_else(|| derive_path_from_children(&cached_folder.items, &state.folder_contents_cache))
                    .unwrap_or_default();
                let needs_path_discovery = folder_title.is_empty();
                let items_for_discovery = if needs_path_discovery { Some(cached_folder.items.clone()) } else { None };
                if let Some(ref mut folder_state) = state.folder_state {
                    let new_column = FolderColumn::new(Some(folder_key.clone()), folder_title, cached_folder.items.clone());
                    if replace_child {
                        folder_state.replace_child_column(new_column);
                    } else {
                        folder_state.push_column(new_column);
                    }
                    backfill_parent_path(folder_state);
                }

                // If we couldn't determine the path, probe a child folder in background
                if let Some(items) = items_for_discovery {
                    if let Some(library_key) = state.active_library.as_deref() {
                        spawn_path_discovery(
                            library_key,
                            &folder_key,
                            &items,
                            event_tx,
                            state.library_generation,
                            client,
                        );
                    }
                }

                // If entry is >= 72h old, serve from cache (warm) but re-fetch in background
                let now_ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let age_secs = now_ts.saturating_sub(cached_folder.timestamp);
                if age_secs >= crate::plex::constants::CACHE_STALE_THRESHOLD_SECS {
                    tracing::info!("Warm subfolder cache: {} ({} days old), re-fetching in background",
                        folder_key, age_secs / (24 * 60 * 60));
                    let event_tx =
                        LibraryEventSender::new(event_tx.clone(), state.library_generation);
                    let client = client.clone();
                    let library_key = state.active_library.clone().unwrap_or_default();
                    let fk = folder_key;
                    tokio::spawn(async move {
                        match client.get_folder_contents(&fk).await {
                            Ok(response) => {
                                let items = FolderService::from_response(&response);
                                let folder_path = FolderService::folder_path(&response);
                                let _ = event_tx.send(FolderEvent::SubfolderRefreshed {
                                    library_key,
                                    folder_key: fk,
                                    cached_folder: CachedFolder::with_path(items, folder_path),
                                }.into()).await;
                            }
                            Err(e) => {
                                tracing::warn!("Warm subfolder re-fetch failed for {}: {}", fk, e);
                            }
                        }
                    });
                }
            } else if state.pending_folder_load.as_ref() != Some(&folder_key) {
                // Not in cache and not already loading - fetch from API in background
                state.pending_folder_load = Some(folder_key.clone());
                state.set_status("Loading folder\u{2026}".to_string());
                let event_tx =
                    LibraryEventSender::new(event_tx.clone(), state.library_generation);
                let client = client.clone();
                let library_key = state.active_library.clone().unwrap_or_default();
                let fk = folder_key;
                let ip = item_path;
                let rc = replace_child;
                tokio::spawn(async move {
                    match client.get_folder_contents(&fk).await {
                        Ok(response) => {
                            let items = FolderService::from_response(&response);
                            let folder_path = FolderService::folder_path(&response);
                            let _ = event_tx.send(FolderEvent::FolderContentsLoaded {
                                library_key,
                                folder_key: fk,
                                items,
                                folder_path,
                                item_path: ip,
                                replace_child: rc,
                            }.into()).await;
                        }
                        Err(e) => {
                            let _ = event_tx.send(FolderEvent::FolderLoadFailed {
                                library_key,
                                pending_folder_key: Some(fk),
                                message: format!("Failed to load folder: {}", e),
                            }.into()).await;
                        }
                    }
                });
            }
        }
        FolderAction::RefreshSubfolder(folder_key) => {
            // Manual refresh of a specific subfolder (F5 when focused on subfolder)
            // This is the ONLY way subfolder caches get manually refreshed.
            state.set_status("Refreshing folder\u{2026}".to_string());
            let event_tx =
                LibraryEventSender::new(event_tx.clone(), state.library_generation);
            let client = client.clone();
            let library_key = state.active_library.clone().unwrap_or_default();
            let fk = folder_key;
            tokio::spawn(async move {
                match client.get_folder_contents(&fk).await {
                    Ok(response) => {
                        let items = FolderService::from_response(&response);
                        let folder_path = FolderService::folder_path(&response);
                        let _ = event_tx.send(FolderEvent::FolderRefreshLoaded {
                            library_key,
                            folder_key: fk,
                            items,
                            folder_path,
                        }.into()).await;
                    }
                    Err(e) => {
                        let _ = event_tx.send(FolderEvent::FolderLoadFailed {
                            library_key,
                            pending_folder_key: None,
                            message: format!("Failed to refresh folder: {}", e),
                        }.into()).await;
                    }
                }
            });
        }
        FolderAction::PlayFolderTracks => {
            // Play tracks in the focused column's folder, starting from selected item
            if let Some(ref folder_state) = state.folder_state {
                let selected_index = folder_state.focused().map(|col| col.selected_index).unwrap_or(0);

                // Resolve tracks in column display order using all_tracks cache
                let tracks = resolve_folder_column_tracks(state);

                if !tracks.is_empty() {
                    // Find start position (selected item)
                    let start_idx = selected_index.min(tracks.len().saturating_sub(1));

                    if let Some(current) = state.current_track().cloned() {
                        helpers::report_playback_stop_to_plex(
                            &current, state.playback.position_ms, true,
                            state.plex_session_id.clone(), client,
                        );
                    }
                    state.plex_session_id = Some(helpers::generate_plex_session_id());
                    state.queue.original.clear();
                    state.queue.sort_mode = crate::app::state::QueueSortMode::QueueOrder;
                    helpers::queue_and_play(event_tx, state, client, audio, tracks, start_idx);
                } else {
                    // Fallback: fetch from API if all_tracks cache is empty
                    let selected_key = folder_state.selected_item().map(|item| item.key.clone());
                    let column_track_keys: Vec<String> = folder_state.focused().map(|col| {
                        col.items.iter()
                            .filter_map(|item| item.rating_key.clone())
                            .collect()
                    }).unwrap_or_default();

                    if let Some(col) = folder_state.focused() {
                        state.folder_play_request_id = state.folder_play_request_id.wrapping_add(1);
                        let request_id = state.folder_play_request_id;
                        let folder_key = col.key.clone();
                        let library_key = state.active_library.clone();
                        let tx = event_tx.clone();
                        let request_client = client.clone();
                        tokio::spawn(async move {
                            let result = if let Some(folder_key) = folder_key {
                                request_client.get_folder_tracks(&folder_key).await
                            } else if let Some(library_key) = library_key {
                                request_client.get_library_root_tracks(&library_key).await
                            } else {
                                Ok(vec![])
                            }
                            .map_err(|error| {
                                AsyncError::from_api("Failed to load folder tracks", &error)
                            });
                            let _ = tx
                                .send(Event::Effect(
                                    FolderAction::FolderTracksLoaded {
                                        request_id,
                                        selected_key,
                                        selected_index,
                                        ordered_keys: column_track_keys,
                                        result,
                                    }
                                    .into(),
                                ))
                                .await;
                        });
                    }
                }
            }
        }
        FolderAction::FolderTracksLoaded {
            request_id,
            selected_key,
            selected_index,
            ordered_keys,
            result,
        } => {
            if state.folder_play_request_id != request_id {
                return Ok(vec![]);
            }
            let mut tracks = match result {
                Ok(tracks) => {
                    state.connection.mark_healthy();
                    tracks
                }
                Err(error) => {
                    if error.connection_error {
                        state.connection.mark_degraded(error.message.clone());
                    }
                    state.set_error(error.message);
                    return Ok(vec![]);
                }
            };
            if !ordered_keys.is_empty() {
                let positions: std::collections::HashMap<&str, usize> = ordered_keys
                    .iter()
                    .enumerate()
                    .map(|(index, key)| (key.as_str(), index))
                    .collect();
                tracks.sort_by_key(|track| {
                    positions
                        .get(track.rating_key.as_str())
                        .copied()
                        .unwrap_or(usize::MAX)
                });
            }
            if tracks.is_empty() {
                return Ok(vec![]);
            }
            let start_index = selected_key
                .as_ref()
                .and_then(|key| {
                    tracks
                        .iter()
                        .position(|track| track.rating_key == *key || track.key == *key)
                })
                .unwrap_or(selected_index.min(tracks.len().saturating_sub(1)));
            if let Some(current) = state.current_track().cloned() {
                helpers::report_playback_stop_to_plex(
                    &current,
                    state.playback.position_ms,
                    true,
                    state.plex_session_id.clone(),
                    client,
                );
            }
            state.plex_session_id = Some(helpers::generate_plex_session_id());
            state.queue.original.clear();
            state.queue.sort_mode = crate::app::state::QueueSortMode::QueueOrder;
            helpers::queue_and_play(event_tx, state, client, audio, tracks, start_index);
        }
        FolderAction::PlayFolderTrack { track_index } => {
            // Play a single track from the focused folder column
            if let Some(ref folder_state) = state.folder_state {
                let selected_key = folder_state.focused()
                    .and_then(|col| col.items.get(track_index))
                    .and_then(|item| item.rating_key.clone());

                // Fast path: look up from all_tracks cache
                if let Some(ref sel_key) = selected_key {
                    if let Some(track) = state.library.all_tracks.iter().find(|t| t.rating_key == *sel_key) {
                        state.plex_session_id = Some(helpers::generate_plex_session_id());
                        state.queue.original.clear();
                        state.queue.sort_mode = crate::app::state::QueueSortMode::QueueOrder;
                        helpers::queue_and_play(event_tx, state, client, audio, vec![track.clone()], 0);
                        return Ok(vec![]);
                    }
                }

                // Slow path: fetch from API
                if let Some(col) = folder_state.focused() {
                    if let Some(ref folder_key) = col.key {
                        state.folder_play_request_id = state.folder_play_request_id.wrapping_add(1);
                        let request_id = state.folder_play_request_id;
                        let folder_key = folder_key.clone();
                        let tx = event_tx.clone();
                        let request_client = client.clone();
                        tokio::spawn(async move {
                            let result = request_client
                                .get_folder_tracks(&folder_key)
                                .await
                                .map_err(|error| {
                                    AsyncError::from_api("Failed to load folder tracks", &error)
                                });
                            let _ = tx
                                .send(Event::Effect(
                                    FolderAction::FolderTrackLoaded {
                                        request_id,
                                        selected_key,
                                        track_index,
                                        result,
                                    }
                                    .into(),
                                ))
                                .await;
                        });
                    }
                }
            }
        }
        FolderAction::FolderTrackLoaded {
            request_id,
            selected_key,
            track_index,
            result,
        } => {
            if state.folder_play_request_id != request_id {
                return Ok(vec![]);
            }
            let tracks = match result {
                Ok(tracks) => {
                    state.connection.mark_healthy();
                    tracks
                }
                Err(error) => {
                    if error.connection_error {
                        state.connection.mark_degraded(error.message.clone());
                    }
                    state.set_error(error.message);
                    return Ok(vec![]);
                }
            };
            let track = if let Some(selected_key) = selected_key {
                tracks
                    .into_iter()
                    .find(|track| track.rating_key == selected_key || track.key == selected_key)
            } else {
                tracks.into_iter().nth(track_index)
            };
            if let Some(track) = track {
                state.plex_session_id = Some(helpers::generate_plex_session_id());
                state.queue.original.clear();
                state.queue.sort_mode = crate::app::state::QueueSortMode::QueueOrder;
                helpers::queue_and_play(event_tx, state, client, audio, vec![track], 0);
            }
        }
    }
    Ok(vec![])
}
