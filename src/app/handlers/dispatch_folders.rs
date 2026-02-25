//! Folder navigation dispatch handlers: LoadFolderRoot, NavigateIntoFolder,
//! NavigateUpFolder, RefreshSubfolder, PlayFolderTracks.

use crate::app::{Action, AppState, Event};
use crate::plex::PlexClient;
use crate::audio::AudioPlayer;
use crate::plex::CachedFolder;
use crate::services::{FolderService, FolderColumn, FolderNavigationState};

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

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
    folder_key: &str,
    items: &[crate::plex::models::FolderItem],
    event_tx: &mpsc::Sender<Event>,
    client: &PlexClient,
) {
    // Find the first child folder to probe
    let child_key = items.iter()
        .find(|item| item.is_folder())
        .map(|item| item.key.clone());

    if let Some(child_key) = child_key {
        let event_tx = event_tx.clone();
        let client = client.clone();
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
                                let _ = event_tx.send(Event::FolderPathDiscovered {
                                    folder_key: fk,
                                    path: parent.to_string(),
                                }).await;
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
    action: Action,
    state: &mut AppState,
    client: &mut PlexClient,
    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    match action {
        Action::LoadFolderRoot => {
            if let Some(lib_key) = &state.active_library {
                let lib_title = state.libraries.iter()
                    .find(|l| &l.key == lib_key)
                    .map(|l| l.title.clone())
                    .unwrap_or_else(|| "Root".to_string());

                let event_tx = event_tx.clone();
                let client = client.clone();
                let lk = lib_key.clone();
                let lt = lib_title;
                tokio::spawn(async move {
                    match client.get_library_folders(&lk).await {
                        Ok(response) => {
                            let items = FolderService::from_response(&response);
                            let _ = event_tx.send(Event::FolderRootLoaded {
                                library_key: lk,
                                lib_title: lt,
                                items,
                            }).await;
                        }
                        Err(e) => {
                            let _ = event_tx.send(Event::FolderLoadFailed(
                                format!("Failed to load folders: {}", e)
                            )).await;
                        }
                    }
                });
            }
        }
        Action::NavigateIntoFolder(folder_key) => {
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
                    folder_state.push_column(new_column);
                    backfill_parent_path(folder_state);
                }

                // If we couldn't determine the path, probe a child folder in background
                if let Some(items) = items_for_discovery {
                    spawn_path_discovery(&folder_key, &items, event_tx, client);
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
                    let event_tx = event_tx.clone();
                    let client = client.clone();
                    let fk = folder_key;
                    tokio::spawn(async move {
                        match client.get_folder_contents(&fk).await {
                            Ok(response) => {
                                let items = FolderService::from_response(&response);
                                let folder_path = FolderService::folder_path(&response);
                                let _ = event_tx.send(Event::SubfolderRefreshed {
                                    folder_key: fk,
                                    cached_folder: CachedFolder::with_path(items, folder_path),
                                }).await;
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
                let event_tx = event_tx.clone();
                let client = client.clone();
                let fk = folder_key;
                let ip = item_path;
                tokio::spawn(async move {
                    match client.get_folder_contents(&fk).await {
                        Ok(response) => {
                            let items = FolderService::from_response(&response);
                            let folder_path = FolderService::folder_path(&response);
                            let _ = event_tx.send(Event::FolderContentsLoaded {
                                folder_key: fk,
                                items,
                                folder_path,
                                item_path: ip,
                            }).await;
                        }
                        Err(e) => {
                            let _ = event_tx.send(Event::FolderLoadFailed(
                                format!("Failed to load folder: {}", e)
                            )).await;
                        }
                    }
                });
            }
        }
        Action::RefreshSubfolder(folder_key) => {
            // Manual refresh of a specific subfolder (F5 when focused on subfolder)
            // This is the ONLY way subfolder caches get manually refreshed.
            state.set_status("Refreshing folder\u{2026}".to_string());
            let event_tx = event_tx.clone();
            let client = client.clone();
            let fk = folder_key;
            tokio::spawn(async move {
                match client.get_folder_contents(&fk).await {
                    Ok(response) => {
                        let items = FolderService::from_response(&response);
                        let folder_path = FolderService::folder_path(&response);
                        let _ = event_tx.send(Event::FolderRefreshLoaded {
                            folder_key: fk,
                            items,
                            folder_path,
                        }).await;
                    }
                    Err(e) => {
                        let _ = event_tx.send(Event::FolderLoadFailed(
                            format!("Failed to refresh folder: {}", e)
                        )).await;
                    }
                }
            });
        }
        Action::PlayFolderTracks => {
            // Play tracks in the focused column's folder, starting from selected item
            if let Some(ref folder_state) = state.folder_state {
                // Get the folder key and selected item from the focused column
                let selected_key = folder_state.selected_item().map(|item| item.key.clone());
                let selected_index = folder_state.focused().map(|col| col.selected_index).unwrap_or(0);
                let is_shuffled = folder_state.focused().map(|col| col.is_shuffled()).unwrap_or(false);
                // Capture track key order from column items for shuffle reordering
                let column_track_keys: Vec<String> = if is_shuffled {
                    folder_state.focused().map(|col| {
                        col.items.iter()
                            .filter_map(|item| item.rating_key.clone())
                            .collect()
                    }).unwrap_or_default()
                } else {
                    vec![]
                };

                if let Some(col) = folder_state.focused() {
                    if let Some(ref folder_key) = col.key {
                        match client.get_folder_tracks(folder_key).await {
                            Ok(mut tracks) => {
                                // Reorder tracks to match shuffled column order
                                if is_shuffled && !column_track_keys.is_empty() {
                                    use std::collections::HashMap;
                                    let pos_map: HashMap<&str, usize> = column_track_keys.iter()
                                        .enumerate()
                                        .map(|(i, k)| (k.as_str(), i))
                                        .collect();
                                    tracks.sort_by_key(|t| {
                                        pos_map.get(t.rating_key.as_str()).copied().unwrap_or(usize::MAX)
                                    });
                                }

                                // Find the index of the selected track
                                let start_idx = if let Some(ref sel_key) = selected_key {
                                    tracks.iter().position(|t| {
                                        t.rating_key == *sel_key || t.key == *sel_key
                                    }).unwrap_or(selected_index.min(tracks.len().saturating_sub(1)))
                                } else {
                                    0
                                };

                                if let Some(current) = state.current_track().cloned() {
                                    helpers::report_playback_stop_to_plex(
                                        &current, state.playback.position_ms, true,
                                        state.plex_session_id.clone(), client,
                                    );
                                }
                                state.plex_session_id = Some(helpers::generate_plex_session_id());
                                state.queue_original.clear();
                                state.queue_sort_mode = crate::app::state::QueueSortMode::QueueOrder;
                                helpers::queue_and_play(event_tx, state, client, audio, tracks, start_idx).await;
                            }
                            Err(e) => {
                                state.set_error(format!("Failed to load folder tracks: {}", e));
                            }
                        }
                    } else {
                        // Root folder - get all tracks from library root
                        if let Some(lib_key) = &state.active_library {
                            match client.get_library_root_tracks(lib_key).await {
                                Ok(mut tracks) => {
                                    if !tracks.is_empty() {
                                        // Reorder tracks to match shuffled column order
                                        if is_shuffled && !column_track_keys.is_empty() {
                                            use std::collections::HashMap;
                                            let pos_map: HashMap<&str, usize> = column_track_keys.iter()
                                                .enumerate()
                                                .map(|(i, k)| (k.as_str(), i))
                                                .collect();
                                            tracks.sort_by_key(|t| {
                                                pos_map.get(t.rating_key.as_str()).copied().unwrap_or(usize::MAX)
                                            });
                                        }

                                        // Find the index of the selected track
                                        let start_idx = if let Some(ref sel_key) = selected_key {
                                            tracks.iter().position(|t| {
                                                t.rating_key == *sel_key || t.key == *sel_key
                                            }).unwrap_or(selected_index.min(tracks.len().saturating_sub(1)))
                                        } else {
                                            0
                                        };

                                        if let Some(current) = state.current_track().cloned() {
                                            helpers::report_playback_stop_to_plex(
                                                &current, state.playback.position_ms, true,
                                                state.plex_session_id.clone(), client,
                                            );
                                        }
                                        state.plex_session_id = Some(helpers::generate_plex_session_id());
                                        state.queue_original.clear();
                                        state.queue_sort_mode = crate::app::state::QueueSortMode::QueueOrder;
                                        helpers::queue_and_play(event_tx, state, client, audio, tracks, start_idx).await;
                                    }
                                }
                                Err(e) => {
                                    state.set_error(format!("Failed to load tracks: {}", e));
                                }
                            }
                        }
                    }
                }
            }
        }
        Action::PlayFolderTrack { track_index } => {
            // Play a single track from the focused folder column
            if let Some(ref folder_state) = state.folder_state {
                let selected_key = folder_state.focused()
                    .and_then(|col| col.items.get(track_index))
                    .and_then(|item| item.rating_key.clone());

                if let Some(col) = folder_state.focused() {
                    if let Some(ref folder_key) = col.key {
                        match client.get_folder_tracks(folder_key).await {
                            Ok(tracks) => {
                                // Find the track matching the selected key
                                let track = if let Some(ref sel_key) = selected_key {
                                    tracks.into_iter().find(|t| t.rating_key == *sel_key || t.key == *sel_key)
                                } else {
                                    tracks.into_iter().nth(track_index)
                                };
                                if let Some(track) = track {
                                    state.plex_session_id = Some(helpers::generate_plex_session_id());
                                    state.queue_original.clear();
                                    state.queue_sort_mode = crate::app::state::QueueSortMode::QueueOrder;
                                    helpers::queue_and_play(event_tx, state, client, audio, vec![track], 0).await;
                                }
                            }
                            Err(e) => {
                                state.set_error(format!("Failed to load folder tracks: {}", e));
                            }
                        }
                    }
                }
            }
        }
        _ => unreachable!("dispatch_folders called with non-folder action: {:?}", action),
    }
    Ok(vec![])
}
