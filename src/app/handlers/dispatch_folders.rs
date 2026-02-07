//! Folder navigation dispatch handlers: LoadFolderRoot, NavigateIntoFolder,
//! NavigateUpFolder, RefreshSubfolder, PlayFolderTracks.

use crate::app::{Action, AppState, Event};
use crate::app::state::PlaybackMode;
use crate::api::PlexClient;
use crate::audio::AudioPlayer;
use crate::plex::CachedFolder;
use crate::services::{FolderService, FolderColumn, FolderNavigationState};

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

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
                match client.get_library_folders(lib_key).await {
                    Ok(response) => {
                        let items = FolderService::from_response(&response);
                        let lib_title = state.libraries.iter()
                            .find(|l| &l.key == lib_key)
                            .map(|l| l.title.clone())
                            .unwrap_or_else(|| "Root".to_string());

                        let root_column = FolderColumn::new(None, lib_title, items);
                        state.folder_state = Some(FolderNavigationState {
                            library_key: lib_key.clone(),
                            columns: vec![root_column],
                            focused_column: 0,
                            loading: false,
                        });
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load folders: {}", e));
                    }
                }
            }
        }
        Action::NavigateIntoFolder(folder_key) => {
            // Check cache first for instant navigation
            if let Some(cached_folder) = state.folder_contents_cache.get(&folder_key) {
                tracing::debug!("Folder cache hit: {} ({} items)", folder_key, cached_folder.items.len());
                // Extract folder title from the key if possible, or use a generic title
                let folder_title = folder_key.split('/').last().unwrap_or("Folder").to_string();
                if let Some(ref mut folder_state) = state.folder_state {
                    let new_column = FolderColumn::new(Some(folder_key), folder_title, cached_folder.items.clone());
                    folder_state.push_column(new_column);
                }
            } else {
                // Not in cache - fetch from API
                match client.get_folder_contents(&folder_key).await {
                    Ok(response) => {
                        let items = FolderService::from_response(&response);
                        let folder_title = response.media_container.title2.clone().unwrap_or_default();

                        // Store in cache with timestamp for future use
                        state.folder_contents_cache.insert(folder_key.clone(), CachedFolder::new(items.clone()));
                        state.cache_dirty = true;
                        tracing::debug!("Cached folder: {} ({} items)", folder_key, items.len());

                        if let Some(ref mut folder_state) = state.folder_state {
                            let new_column = FolderColumn::new(Some(folder_key), folder_title, items);
                            folder_state.push_column(new_column);
                        }
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load folder: {}", e));
                    }
                }
            }
        }
        Action::NavigateUpFolder => {
            // In column view, going up just moves focus left
            if let Some(ref mut folder_state) = state.folder_state {
                folder_state.focus_left();
            }
        }
        Action::RefreshSubfolder(folder_key) => {
            // Manual refresh of a specific subfolder (F5 when focused on subfolder)
            // This is the ONLY way subfolder caches get manually refreshed.

            match client.get_folder_contents(&folder_key).await {
                Ok(response) => {
                    let items = FolderService::from_response(&response);
                    let folder_title = response.media_container.title2.clone().unwrap_or_default();

                    // Update the cache with fresh data and new timestamp
                    state.folder_contents_cache.insert(folder_key.clone(), CachedFolder::new(items.clone()));
                    state.cache_dirty = true;
                    tracing::info!("Refreshed subfolder cache: {} ({} items)", folder_key, items.len());

                    // Update the currently displayed column if it matches
                    if let Some(ref mut folder_state) = state.folder_state {
                        // Find the column that corresponds to this folder key and update it
                        for col in folder_state.columns.iter_mut() {
                            if col.key.as_ref() == Some(&folder_key) {
                                let old_selected = col.selected_index;
                                col.items = items.clone();
                                // Preserve selection position if possible
                                col.selected_index = old_selected.min(col.items.len().saturating_sub(1));
                                col.title = folder_title.clone();
                                break;
                            }
                        }
                    }

                    state.set_status("Folder refreshed".to_string());
                }
                Err(e) => {
                    state.set_error(format!("Failed to refresh folder: {}", e));
                }
            }
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

                                state.queue = tracks;
                                state.queue_index = Some(start_idx);
                                state.playback_mode = PlaybackMode::Queue;
                                if let Some(track) = state.queue.get(start_idx).cloned() {
                                    helpers::play_track(event_tx, track, state, client, audio).await;
                                }
                            }
                            Err(e) => {
                                state.set_error(format!("Failed to load folder tracks: {}", e));
                            }
                        }
                    } else {
                        // Root folder - get all tracks from library root
                        if let Some(lib_key) = &state.active_library {
                            match client.get_library_root_tracks(lib_key).await {
                                Ok(tracks) => {
                                    if !tracks.is_empty() {
                                        state.queue = tracks;
                                        state.queue_index = Some(0);
                                        state.playback_mode = PlaybackMode::Queue;
                                        if let Some(track) = state.queue.first().cloned() {
                                            helpers::play_track(event_tx, track, state, client, audio).await;
                                        }
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
        _ => unreachable!("dispatch_folders called with non-folder action: {:?}", action),
    }
    Ok(vec![])
}
