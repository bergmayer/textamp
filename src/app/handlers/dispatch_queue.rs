//! Queue dispatch handlers: PlayTrack, PlayTrackFromCategory, PlayAlbum, EnqueueAlbum,
//! ClearQueue, RemoveFromQueue, JumpToQueueIndex,
//! EnqueueSelection, PromptSavePlaylist, SaveQueueAsPlaylist.

use crate::app::action::{DataAction, QueueAction, QueueLoadIntent};
use crate::app::state::{
    BrowseCategory, BrowseItem, PlayStatus, PlaybackMode, QueueSortMode, SimilarMode, View,
};
use crate::app::{Action, AppState, Event};
use crate::audio::AudioPlayer;
use crate::library::models::Track;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Get tracks from a folder column in display order, starting from `start_index`.
///
/// Uses the preloaded `all_tracks` cache for instant lookup (no API call).
/// Returns a background fetch plan if `all_tracks` is empty.
enum FolderTrackPlan {
    Ready(Vec<Track>),
    MissingCatalogTracks,
}

fn get_folder_tracks_from_index(state: &AppState, start_index: usize) -> FolderTrackPlan {
    let folder_state = match state.folder_state.as_ref() {
        Some(fs) => fs,
        None => return FolderTrackPlan::Ready(vec![]),
    };
    let col = match folder_state.focused() {
        Some(c) => c,
        None => return FolderTrackPlan::Ready(vec![]),
    };

    let start_index = start_index.min(col.items.len());
    if let Some(id) = state.sources.active.folder() {
        return FolderTrackPlan::Ready(
            col.items[start_index..]
                .iter()
                .filter(|i| i.is_track())
                .map(|i| Track::from_folder(id, &i.key))
                .collect(),
        );
    }
    // Collect rating_keys from the column in display order, starting at start_index
    let column_keys: Vec<String> = col.items[start_index..]
        .iter()
        .filter_map(|item| item.rating_key.clone())
        .collect();
    if column_keys.is_empty() {
        return FolderTrackPlan::Ready(vec![]);
    }

    // Fast path: look up from preloaded all_tracks cache (no network call)
    if !state.library.all_tracks.is_empty() {
        let track_map: std::collections::HashMap<&str, &Track> = state
            .library
            .all_tracks
            .iter()
            .map(|t| (t.rating_key.as_str(), t))
            .collect();
        let result: Vec<Track> = column_keys
            .iter()
            .filter_map(|key| track_map.get(key.as_str()).map(|t| (*t).clone()))
            .collect();
        if !result.is_empty() {
            return FolderTrackPlan::Ready(result);
        }
    }

    // Navidrome virtual folders are catalog-backed, never server folder URLs.
    {
        FolderTrackPlan::MissingCatalogTracks
    }
}

/// Dispatch queue actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: QueueAction,
    state: &mut AppState,

    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    let mut follow_ups = vec![];

    match action {
        QueueAction::PlayTrack(track) => {
            helpers::play_track(event_tx, *track, state, audio);
        }
        QueueAction::PlayTracksNow(tracks) => {
            // Play tracks immediately, replacing queue
            if !tracks.is_empty() {
                helpers::queue_and_play(event_tx, state, audio, tracks, 0);
            }
        }
        QueueAction::PlayTrackFromCategory(idx) => {
            if idx < state.library.selected_album_tracks.len() {
                // Report stop for currently playing track before switching

                // Generate new session ID for this playback context

                // Clear radio state if switching from radio mode
                if state.playback_mode == PlaybackMode::Radio {
                    state.radio.clear();
                }

                // Prepend new tracks at front of queue
                let new_tracks: Vec<Track> = state.library.selected_album_tracks[idx..].to_vec();
                state.queue.tracks.splice(0..0, new_tracks);
                state.queue.index = Some(0);
                state.queue.original.clear();
                state.queue.sort_mode = QueueSortMode::QueueOrder;
                state.set_playback_mode(PlaybackMode::Queue);
                state.list_state.queue_index = 0;
                helpers::play_current_track(event_tx, state, audio);
            }
        }

        QueueAction::PlaySearchResult => {
            // Play the selected search result (Shift+Enter in search)
            let play_actions = play_search_result(state);
            follow_ups.extend(play_actions);
        }

        QueueAction::EnqueueTrack(track) => {
            // Append a single track to end of queue
            if state.playback_mode == PlaybackMode::Radio {
                state.queue.tracks = state.radio.tracks.clone();
                state.queue.index = state.radio.track_index;
                state.set_playback_mode(PlaybackMode::Queue);
                state.radio.clear();
                if let Some(idx) = state.queue.index {
                    state.list_state.queue_index = idx;
                }
            }
            let title = track.title.clone();
            state.queue.tracks.push(*track);
            state.set_status(format!("Added \"{}\" to queue", title));
        }

        QueueAction::EnqueueSearchResult => {
            // Enqueue the selected search result (at end of queue)
            let follow_up = enqueue_search_result(state);
            follow_ups.extend(follow_up);
            // Close search popup and navigate to queue
            state.popups.search_active = false;
            state.set_view(View::Queue);
        }
        QueueAction::EnqueueSearchResultNext => {
            // Ctrl+Shift+E: insert search result NEXT in queue (after current track)
            let follow_up = enqueue_search_result_next(state);
            follow_ups.extend(follow_up);
            // Close search popup and navigate to queue
            state.popups.search_active = false;
            state.set_view(View::Queue);
        }

        QueueAction::EnqueueTracksNext(tracks) => {
            // Ctrl+Shift+E: Insert tracks NEXT in queue (after current track)
            if !tracks.is_empty() {
                let title = tracks.first().map(|t| t.title.clone()).unwrap_or_default();
                let added = helpers::insert_tracks_next(state, tracks);
                if added == 1 {
                    state.set_status(format!(
                        "Inserted \"{}\" next ({} total)",
                        title,
                        state.queue.tracks.len()
                    ));
                } else {
                    state.set_status(format!(
                        "Inserted {} tracks next, starting with \"{}\" ({} total)",
                        added,
                        title,
                        state.queue.tracks.len()
                    ));
                }
            }
        }
        QueueAction::TracksLoaded { intent, result } => {
            if let QueueLoadIntent::ReplaceAndPlay { request_id, .. } = &intent {
                if *request_id != state.queue_play_request_id {
                    tracing::debug!("Ignoring stale queue play load");
                    return Ok(follow_ups);
                }
            }
            let tracks = match result {
                Ok(tracks) => tracks,
                Err(error) => {
                    state.set_error(error.message);
                    return Ok(follow_ups);
                }
            };
            if tracks.is_empty() {
                state.set_error("No tracks were returned".to_string());
                return Ok(follow_ups);
            }

            match intent {
                QueueLoadIntent::ReplaceAndPlay { label, .. } => {
                    let count = tracks.len();
                    helpers::queue_and_play(event_tx, state, audio, tracks, 0);
                    if let Some(label) = label {
                        state.set_status(format!("Playing {count} tracks {label}"));
                    }
                }
                QueueLoadIntent::Append { label } => {
                    if state.playback_mode == PlaybackMode::Radio {
                        state.queue.tracks = state.radio.tracks.clone();
                        state.queue.index = state.radio.track_index;
                        state.set_playback_mode(PlaybackMode::Queue);
                        state.radio.clear();
                        if let Some(index) = state.queue.index {
                            state.list_state.queue_index = index;
                        }
                    }
                    let added = tracks.len();
                    state.queue.tracks.extend(tracks);
                    state.queue.original.clear();
                    state.queue.sort_mode = QueueSortMode::QueueOrder;
                    state.set_status(format!("Added {added} tracks {label} to queue"));
                }
                QueueLoadIntent::InsertNext { label } => {
                    let added = helpers::insert_tracks_next(state, tracks);
                    state.set_status(format!(
                        "Inserted {added} tracks {label} next ({} total)",
                        state.queue.tracks.len()
                    ));
                }
            }
        }
        QueueAction::ClearQueue => {
            // Clear the appropriate queue based on playback mode
            match state.playback_mode {
                PlaybackMode::Radio => {
                    state.radio.clear();
                }
                PlaybackMode::Queue | PlaybackMode::None => {
                    state.queue.tracks.clear();
                    state.queue.index = None;
                    state.queue.original.clear();
                    state.queue.sort_mode = QueueSortMode::QueueOrder;
                }
            }
            state.list_state.queue_index = 0;
            audio.stop();
            state.playback.request_id = audio.playback_id();
            state.playback.status = PlayStatus::Stopped;
            // Clear artwork so stale cover art doesn't linger
            state.artwork.current_thumb = None;
            state.artwork.current_data = None;
            state.artwork.loading = false;
            state.artwork.pending_thumb = None;
        }
        QueueAction::ToggleQueueShuffle => {
            use crate::services::shuffle_queue;

            if state.playback_mode == PlaybackMode::Radio {
                // Shuffle/unshuffle radio tracks
                match state.queue.sort_mode {
                    QueueSortMode::QueueOrder => {
                        state.queue.original = state.radio.tracks.clone();
                        let (shuffled, new_idx) =
                            shuffle_queue(state.radio.tracks.clone(), state.radio.track_index);
                        state.radio.tracks = shuffled;
                        state.radio.track_index = new_idx;
                        state.queue.sort_mode = QueueSortMode::Shuffle;
                    }
                    QueueSortMode::Shuffle => {
                        let current_key = state.current_track().map(|t| t.rating_key.clone());
                        state.radio.tracks = std::mem::take(&mut state.queue.original);
                        state.queue.sort_mode = QueueSortMode::QueueOrder;
                        if let Some(key) = current_key {
                            state.radio.track_index =
                                state.radio.tracks.iter().position(|t| t.rating_key == key);
                        }
                    }
                }
            } else {
                // Shuffle/unshuffle queue tracks
                match state.queue.sort_mode {
                    QueueSortMode::QueueOrder => {
                        state.queue.original = state.queue.tracks.clone();
                        let (shuffled, new_idx) = shuffle_queue(
                            std::mem::take(&mut state.queue.tracks),
                            state.queue.index,
                        );
                        state.queue.tracks = shuffled;
                        state.queue.index = new_idx; // always Some(0)
                        state.queue.sort_mode = QueueSortMode::Shuffle;
                        state.list_state.queue_index = 0;
                    }
                    QueueSortMode::Shuffle => {
                        let current_key = state.current_track().map(|t| t.rating_key.clone());
                        state.queue.tracks = std::mem::take(&mut state.queue.original);
                        state.queue.sort_mode = QueueSortMode::QueueOrder;
                        if let Some(key) = current_key {
                            state.queue.index =
                                state.queue.tracks.iter().position(|t| t.rating_key == key);
                        }
                        if let Some(idx) = state.queue.index {
                            state.list_state.queue_index = idx;
                        }
                    }
                }
            }
            // Flush and re-prefetch based on new order
        }
        QueueAction::RemoveFromQueue(idx) => {
            // Promote Radio → Queue before the bounds check — same
            // reasoning as MoveQueueTrack, otherwise the index lookup
            // hits an empty `state.queue.tracks` and silently no-ops.
            if state.playback_mode == PlaybackMode::Radio {
                let snapshot = state.convert_radio_to_queue("Remove track (from radio)");
                state.queue.undo_snapshot = Some(snapshot);
            }
            if idx < state.queue.tracks.len() {
                state.queue.tracks.remove(idx);
                // Adjust queue_index if needed
                if let Some(current) = state.queue.index {
                    if idx < current {
                        state.queue.index = Some(current - 1);
                    } else if idx == current && current >= state.queue.tracks.len() {
                        state.queue.index = if state.queue.tracks.is_empty() {
                            None
                        } else {
                            Some(state.queue.tracks.len() - 1)
                        };
                    }
                }
                // Adjust list selection
                let max_visual = state.queue.tracks.len().saturating_sub(1);
                if state.list_state.queue_index > max_visual {
                    state.list_state.queue_index = max_visual;
                }
            }
        }
        QueueAction::JumpToQueueIndex(idx) => {
            // Jump to and play a specific track in the queue (without modifying queue order)
            if idx < state.queue.tracks.len() {
                // Report stop for currently playing track before switching

                // Generate new session ID for this playback context

                state.queue.index = Some(idx);
                state.list_state.queue_index = idx;
                state.set_playback_mode(PlaybackMode::Queue);
                helpers::play_current_track(event_tx, state, audio);

                // Trigger DJ mode processing after jump (all modes are continuous)
                if !state.dj.inserting && state.dj.active_mode.is_some() {
                    follow_ups.push(crate::app::action::RadioAction::DjModeProcess.into());
                }
            }
        }
        QueueAction::EnqueueSelection => {
            // Ctrl+E: Add selected item + all following items to END of queue
            // For tracks: enqueue selected track + all following tracks in the view
            // For albums: enqueue the album

            // Check for album selection first
            let album_to_enqueue: Option<(String, String)> = match state.view {
                View::Browse => {
                    let nav = match state.browse_category {
                        BrowseCategory::Library => Some(&state.artist_nav),
                        cat if cat.is_tag_section() => Some(&state.tag_nav),
                        BrowseCategory::Playlists => Some(&state.playlist_nav),
                        _ => None,
                    };
                    nav.and_then(|n| n.selected_item()).and_then(|item| {
                        if let BrowseItem::Album { key, title, .. } = item {
                            Some((key.clone(), title.clone()))
                        } else {
                            None
                        }
                    })
                }
                View::Similar => match state.similar.mode {
                    SimilarMode::Albums => state
                        .similar
                        .albums
                        .get(state.list_state.similar_index)
                        .map(|a| (a.rating_key.clone(), a.title.clone())),
                    _ => None,
                },
                View::Related => {
                    let idx = state.list_state.related_index;
                    let resolved = super::helpers::navigation::related_flat_resolve(
                        &state.related.groups,
                        idx,
                    );
                    resolved.and_then(|(gi, is_header, ai)| {
                        if is_header {
                            return None;
                        }
                        state
                            .related
                            .groups
                            .get(gi)
                            .and_then(|g| g.albums.get(ai))
                            .map(|a| (a.rating_key.clone(), a.title.clone()))
                    })
                }
                _ => None,
            };

            if let Some((rating_key, title)) = album_to_enqueue {
                return Ok(vec![QueueAction::EnqueueAlbum { rating_key, title }.into()]);
            }

            // Folder tracks: fetch from API, ordered by column display order
            if state.view == View::Browse && state.browse_category == BrowseCategory::Folders {
                let start = state
                    .folder_state
                    .as_ref()
                    .and_then(|fs| fs.focused())
                    .map(|col| col.selected_index)
                    .unwrap_or(0);
                let intent = QueueLoadIntent::Append {
                    label: "from folder".to_string(),
                };
                match get_folder_tracks_from_index(state, start) {
                    FolderTrackPlan::Ready(tracks) if !tracks.is_empty() => {
                        return Ok(vec![QueueAction::TracksLoaded {
                            intent,
                            result: Ok(tracks),
                        }
                        .into()]);
                    }
                    FolderTrackPlan::Ready(_) => {}
                    FolderTrackPlan::MissingCatalogTracks => {
                        state.set_error(
                            "Folder tracks are no longer in the catalog; refresh with F5".into(),
                        );
                        return Ok(vec![]);
                    }
                }
            }

            // Get tracks from selected index to end
            let tracks_to_add: Vec<Track> = match state.view {
                View::Browse => {
                    // Miller columns: get selected track + all following
                    let nav = match state.browse_category {
                        BrowseCategory::Library => Some(&state.artist_nav),
                        cat if cat.is_tag_section() => Some(&state.tag_nav),
                        BrowseCategory::Playlists => Some(&state.playlist_nav),
                        _ => None,
                    };
                    if let Some(nav) = nav {
                        if let Some(col) = nav.columns.get(nav.focused_column) {
                            if let Some(BrowseItem::Track { .. }) =
                                col.items.get(col.selected_index)
                            {
                                col.tracks[col.selected_index..].to_vec()
                            } else {
                                vec![]
                            }
                        } else {
                            vec![]
                        }
                    } else {
                        vec![]
                    }
                }
                View::Similar => match state.similar.mode {
                    SimilarMode::Tracks => {
                        let idx = state.list_state.similar_index;
                        state.similar.tracks[idx..].to_vec()
                    }
                    _ => vec![],
                },
                View::Search => {
                    use crate::app::state::SearchTab;
                    if let Some(ref results) = state.search.results {
                        let idx = state.list_state.search_item_index;
                        let (section, local_idx) = if state.search.tab == SearchTab::Global {
                            super::dispatch_search::resolve_global_index(results, idx)
                        } else {
                            (state.search.tab, idx)
                        };
                        match section {
                            SearchTab::Tracks => results.tracks[local_idx..].to_vec(),
                            _ => vec![],
                        }
                    } else {
                        vec![]
                    }
                }
                _ => vec![],
            };

            if !tracks_to_add.is_empty() {
                // If radio is playing, convert to queue mode
                if state.playback_mode == PlaybackMode::Radio {
                    state.queue.tracks = state.radio.tracks.clone();
                    state.queue.index = state.radio.track_index;
                    state.set_playback_mode(PlaybackMode::Queue);
                    state.radio.clear();
                    if let Some(idx) = state.queue.index {
                        state.list_state.queue_index = idx;
                    }
                }

                state.queue.original.clear();
                state.queue.sort_mode = QueueSortMode::QueueOrder;

                let added = tracks_to_add.len();
                state.queue.tracks.extend(tracks_to_add);
                state.set_status(format!(
                    "Added {} to queue ({} total)",
                    added,
                    state.queue.tracks.len()
                ));
            }
        }
        QueueAction::EnqueueSelectionNext => {
            // Ctrl+Shift+E: Insert selected item + all following items NEXT in queue (after current track)
            // For tracks: insert selected track + all following tracks
            // For albums: insert the album's tracks

            // Check for album selection first
            let album_to_enqueue: Option<(String, String)> = match state.view {
                View::Browse => {
                    let nav = match state.browse_category {
                        BrowseCategory::Library => Some(&state.artist_nav),
                        cat if cat.is_tag_section() => Some(&state.tag_nav),
                        BrowseCategory::Playlists => Some(&state.playlist_nav),
                        _ => None,
                    };
                    nav.and_then(|n| n.selected_item()).and_then(|item| {
                        if let BrowseItem::Album { key, title, .. } = item {
                            Some((key.clone(), title.clone()))
                        } else {
                            None
                        }
                    })
                }
                View::Similar => match state.similar.mode {
                    SimilarMode::Albums => state
                        .similar
                        .albums
                        .get(state.list_state.similar_index)
                        .map(|a| (a.rating_key.clone(), a.title.clone())),
                    _ => None,
                },
                View::Related => {
                    let idx = state.list_state.related_index;
                    let resolved = super::helpers::navigation::related_flat_resolve(
                        &state.related.groups,
                        idx,
                    );
                    resolved.and_then(|(gi, is_header, ai)| {
                        if is_header {
                            return None;
                        }
                        state
                            .related
                            .groups
                            .get(gi)
                            .and_then(|g| g.albums.get(ai))
                            .map(|a| (a.rating_key.clone(), a.title.clone()))
                    })
                }
                _ => None,
            };

            if let Some((rating_key, title)) = album_to_enqueue {
                return Ok(vec![
                    QueueAction::EnqueueAlbumNext { rating_key, title }.into()
                ]);
            }

            // Folder tracks: fetch from API, ordered by column display order
            if state.view == View::Browse && state.browse_category == BrowseCategory::Folders {
                let start = state
                    .folder_state
                    .as_ref()
                    .and_then(|fs| fs.focused())
                    .map(|col| col.selected_index)
                    .unwrap_or(0);
                let intent = QueueLoadIntent::InsertNext {
                    label: "from folder".to_string(),
                };
                match get_folder_tracks_from_index(state, start) {
                    FolderTrackPlan::Ready(tracks) if !tracks.is_empty() => {
                        return Ok(vec![QueueAction::TracksLoaded {
                            intent,
                            result: Ok(tracks),
                        }
                        .into()]);
                    }
                    FolderTrackPlan::Ready(_) => {}
                    FolderTrackPlan::MissingCatalogTracks => {
                        state.set_error(
                            "Folder tracks are no longer in the catalog; refresh with F5".into(),
                        );
                        return Ok(vec![]);
                    }
                }
            }

            // Get tracks from selected index to end
            let tracks_to_add: Vec<Track> = match state.view {
                View::Browse => {
                    // Miller columns: get selected track + all following
                    let nav = match state.browse_category {
                        BrowseCategory::Library => Some(&state.artist_nav),
                        cat if cat.is_tag_section() => Some(&state.tag_nav),
                        BrowseCategory::Playlists => Some(&state.playlist_nav),
                        _ => None,
                    };
                    if let Some(nav) = nav {
                        if let Some(col) = nav.columns.get(nav.focused_column) {
                            if let Some(BrowseItem::Track { .. }) =
                                col.items.get(col.selected_index)
                            {
                                // Get all tracks from selected index to end
                                col.tracks[col.selected_index..].to_vec()
                            } else {
                                vec![]
                            }
                        } else {
                            vec![]
                        }
                    } else {
                        vec![]
                    }
                }
                View::Similar => {
                    match state.similar.mode {
                        SimilarMode::Tracks => {
                            // Get selected track + all following
                            let idx = state.list_state.similar_index;
                            state.similar.tracks[idx..].to_vec()
                        }
                        _ => vec![],
                    }
                }
                View::Search => {
                    // Search results: get selected track + all following in tracks tab
                    use crate::app::state::SearchTab;
                    if let Some(ref results) = state.search.results {
                        let idx = state.list_state.search_item_index;
                        let (section, local_idx) = if state.search.tab == SearchTab::Global {
                            super::dispatch_search::resolve_global_index(results, idx)
                        } else {
                            (state.search.tab, idx)
                        };
                        match section {
                            SearchTab::Tracks => results.tracks[local_idx..].to_vec(),
                            _ => vec![],
                        }
                    } else {
                        vec![]
                    }
                }
                _ => vec![],
            };

            if !tracks_to_add.is_empty() {
                return Ok(vec![QueueAction::EnqueueTracksNext(tracks_to_add).into()]);
            }
        }
        QueueAction::PromptSavePlaylist => {
            // Show input dialog for playlist name
            let has_tracks = !state.playback_tracks().is_empty();
            if !has_tracks {
                state.set_error("No tracks to save".to_string());
            } else {
                let title = if state.playback_mode == PlaybackMode::Radio {
                    "Save Station as Playlist"
                } else {
                    "Save Queue as Playlist"
                };
                state.popups.close_all();
                state.popups.input_dialog = Some(crate::app::state::InputDialog {
                    title: title.to_string(),
                    input: Default::default(),
                    action_type: crate::app::state::InputDialogAction::SavePlaylist,
                });
            }
        }

        QueueAction::QueuePlaylistSaved {
            name,
            track_count,
            result,
        } => match result {
            Ok(()) => {
                state.set_status(format!("Saved \"{name}\" ({track_count} tracks)"));
                return Ok(vec![DataAction::LoadPlaylists.into()]);
            }
            Err(error) => {
                state.set_error(error.message);
            }
        },

        QueueAction::RemixShuffle => {
            if state.playback_mode == PlaybackMode::Radio {
                let snapshot = state.convert_radio_to_queue("Remix: Shuffle (from radio)");
                state.queue.undo_snapshot = Some(snapshot);
            }

            use crate::services::shuffle_queue;

            // Save snapshot for Ctrl+Z undo (skip if already set by radio conversion)
            if state.queue.undo_snapshot.is_none() {
                state.queue.undo_snapshot = Some(crate::app::state::QueueSnapshot {
                    contents: crate::app::state::QueueContents::Queue {
                        tracks: state.queue.tracks.clone(),
                        index: state.queue.index,
                    },
                    description: "Remix: Shuffle".to_string(),
                });
            }

            // Save shuffle-specific undo state (for toggle)
            state.queue.shuffle_undo_queue = Some(state.queue.tracks.clone());
            state.queue.shuffle_undo_index = state.queue.index;

            let (shuffled, new_idx) = shuffle_queue(state.queue.tracks.clone(), state.queue.index);
            state.queue.tracks = shuffled;
            state.queue.index = new_idx;
            state.list_state.queue_index = 0;
            state.set_status("Queue shuffled".to_string());
        }
        QueueAction::RemixUndoShuffle => {
            if let Some(original) = state.queue.shuffle_undo_queue.take() {
                let current_key = state.current_track().map(|t| t.rating_key.clone());
                state.queue.tracks = original;
                state.queue.index = state.queue.shuffle_undo_index;
                // Try to keep the currently playing track as the index
                if let Some(key) = current_key {
                    if let Some(idx) = state.queue.tracks.iter().position(|t| t.rating_key == key) {
                        state.queue.index = Some(idx);
                    }
                }
                if let Some(idx) = state.queue.index {
                    state.list_state.queue_index = idx;
                }
                state.set_status("Shuffle undone".to_string());
            } else {
                state.set_error("No shuffle to undo".to_string());
            }
        }
        QueueAction::UndoLastRemix => {
            if let Some(snapshot) = state.queue.undo_snapshot.take() {
                match snapshot.contents {
                    crate::app::state::QueueContents::Radio(radio_snap) => {
                        // Restore radio mode
                        state.radio = *radio_snap;
                        state.set_playback_mode(PlaybackMode::Radio);
                        state.queue.tracks.clear();
                        state.queue.index = None;
                        state.queue.original.clear();
                        if let Some(idx) = state.radio.track_index {
                            state.list_state.queue_index = idx;
                        }
                        state.set_status(format!("Undid {} — resumed radio", snapshot.description));
                    }
                    crate::app::state::QueueContents::Queue { tracks, index } => {
                        // Normal queue undo
                        state.queue.tracks = tracks;
                        state.queue.index = index;
                        if let Some(idx) = state.queue.index {
                            state.list_state.queue_index = idx;
                        }
                        state.set_status(format!("Undid {}", snapshot.description));
                    }
                }
                state.queue.shuffle_undo_queue = None;
                state.queue.shuffle_undo_index = None;
            } else {
                state.set_error("Nothing to undo".to_string());
            }
        }
        QueueAction::MoveQueueTrackUp
        | QueueAction::MoveQueueTrackDown
        | QueueAction::MoveSelectedTracksUp
        | QueueAction::MoveSelectedTracksDown => {
            let multiple = matches!(
                action,
                QueueAction::MoveSelectedTracksUp | QueueAction::MoveSelectedTracksDown
            );
            let down = matches!(
                action,
                QueueAction::MoveQueueTrackDown | QueueAction::MoveSelectedTracksDown
            );
            let delta = if down { 1 } else { -1 };
            if state.playback_mode == PlaybackMode::Radio {
                let description = if multiple {
                    "Move tracks (from radio)"
                } else {
                    "Move track (from radio)"
                };
                state.queue.undo_snapshot = Some(state.convert_radio_to_queue(description));
            }
            let mut selected: Vec<_> = if multiple {
                state.queue.selected.iter().copied().collect()
            } else {
                vec![state.list_state.queue_index]
            };
            let len = state.queue.tracks.len();
            // A block touching the boundary cannot move; preserve its order.
            if selected.is_empty()
                || selected.iter().any(|&index| {
                    index >= len
                        || index
                            .checked_add_signed(delta)
                            .is_none_or(|next| next >= len)
                })
            {
                return Ok(vec![]);
            }
            if down {
                selected.reverse();
            }
            for &from in &selected {
                state
                    .queue
                    .move_track(from, from.checked_add_signed(delta).unwrap());
            }
            if multiple {
                state.queue.selected = selected
                    .into_iter()
                    .map(|index| index.checked_add_signed(delta).unwrap())
                    .collect();
            }
            state.list_state.queue_index = state
                .list_state
                .queue_index
                .saturating_add_signed(delta)
                .min(len.saturating_sub(1));
        }
        QueueAction::RemoveSelectedFromQueue => {
            if state.playback_mode == PlaybackMode::Radio {
                let snapshot = state.convert_radio_to_queue("Delete tracks (from radio)");
                state.queue.undo_snapshot = Some(snapshot);
            }
            // Remove all selected tracks (process from highest index down)
            let selected: Vec<usize> = state.queue.selected.iter().copied().rev().collect();
            for &idx in &selected {
                if idx < state.queue.tracks.len() {
                    state.queue.tracks.remove(idx);
                    if let Some(qi) = state.queue.index {
                        if idx < qi {
                            state.queue.index = Some(qi - 1);
                        } else if idx == qi && qi >= state.queue.tracks.len() {
                            state.queue.index = if state.queue.tracks.is_empty() {
                                None
                            } else {
                                Some(state.queue.tracks.len() - 1)
                            };
                        }
                    }
                }
            }
            state.queue.selected.clear();
            // Adjust visual index
            let max = state.queue.tracks.len().saturating_sub(1);
            if state.list_state.queue_index > max {
                state.list_state.queue_index = max;
            }
        }
        QueueAction::MoveQueueTrack { from, to } => {
            // Drag-and-drop reorder. `from` and `to` are absolute queue
            // indices; `to` is the position the dragged row should occupy
            // after the move (i.e. `Vec::remove(from)` then
            // `Vec::insert(adjusted_to, track)`).
            //
            // Promote a Radio session to a Queue *before* the bounds
            // check — in Radio mode `state.queue.tracks` is empty
            // (tracks live in `state.radio.tracks`) and reading its
            // length would short-circuit the move.
            if state.playback_mode == PlaybackMode::Radio {
                let snapshot = state.convert_radio_to_queue("Reorder track (from radio)");
                state.queue.undo_snapshot = Some(snapshot);
            }
            if let Some(destination) = state.queue.move_track(from, to) {
                state.list_state.queue_index = destination;
            }
        }
        QueueAction::RemixBatchReady(mut outcome) => {
            let inserts = std::mem::take(&mut outcome.items);
            if inserts.is_empty() {
                if outcome.failed > 0 {
                    let detail = outcome
                        .first_error
                        .as_ref()
                        .map(|error| error.message.as_str())
                        .unwrap_or("request failed");
                    state.set_error(format!(
                        "Remix could not make changes; {} of {} lookups failed: {}",
                        outcome.failed, outcome.attempted, detail
                    ));
                } else {
                    state.set_status("Remix: no matching changes found".to_string());
                }
                return Ok(follow_ups);
            }

            // Collect inserts into a map
            let inserts_map: std::collections::HashMap<usize, Vec<Track>> =
                inserts.into_iter().collect();

            // Process inserts in reverse index order so earlier splices don't shift later indices
            let mut positions: Vec<usize> = inserts_map.keys().copied().collect();
            positions.sort_unstable_by(|a, b| b.cmp(a));

            let mut total_inserted = 0usize;
            for pos in positions {
                if let Some(insert_tracks) = inserts_map.get(&pos) {
                    let insert_at = (pos + 1).min(state.queue.tracks.len());
                    total_inserted += insert_tracks.len();
                    state
                        .queue
                        .tracks
                        .splice(insert_at..insert_at, insert_tracks.iter().cloned());
                }
            }

            if outcome.failed > 0 {
                state.set_status(format!(
                    "Remix: {} tracks added; {} of {} lookups failed",
                    total_inserted, outcome.failed, outcome.attempted
                ));
            } else {
                state.set_status(format!("Remix complete: {} tracks added", total_inserted));
            }

            // Pre-cache upcoming tracks
        }
        QueueAction::RemixDoppelgangerReady(mut outcome) => {
            let replacements = std::mem::take(&mut outcome.items);
            if replacements.is_empty() {
                if outcome.failed > 0 {
                    let detail = outcome
                        .first_error
                        .as_ref()
                        .map(|error| error.message.as_str())
                        .unwrap_or("request failed");
                    state.set_error(format!(
                        "Remix Doppelganger could not replace tracks; {} of {} lookups failed: {}",
                        outcome.failed, outcome.attempted, detail
                    ));
                } else {
                    state.set_status("Remix: Doppelganger — no replacements found".to_string());
                }
                return Ok(follow_ups);
            }

            let count = replacements.len();
            let replaced_current = replacements
                .iter()
                .any(|(index, _)| state.queue.index == Some(*index));

            // Replace tracks in reverse index order to preserve positions
            let mut sorted = replacements;
            sorted.sort_unstable_by_key(|item| std::cmp::Reverse(item.0));
            for (idx, track) in sorted {
                if idx < state.queue.tracks.len() {
                    state.queue.tracks[idx] = track;
                }
            }

            if outcome.failed > 0 {
                state.set_status(format!(
                    "Remix: Doppelganger — {} replaced; {} of {} lookups failed",
                    count, outcome.failed, outcome.attempted
                ));
            } else {
                state.set_status(format!("Remix: Doppelganger — {} tracks replaced", count));
            }

            // Replacing only later tracks must not interrupt the current song.
            if replaced_current {
                follow_ups.push(crate::app::action::PlaybackAction::Stop.into());
            }

            // Pre-cache upcoming tracks
        }
        _ => anyhow::bail!("Unsupported library operation reached shared handler"),
    }
    Ok(follow_ups)
}

// ---------------------------------------------------------------------------
// Remix batch processing
// ---------------------------------------------------------------------------

/// Enqueue the currently selected search result + following items to END of queue.
fn enqueue_search_result(state: &mut AppState) -> Vec<Action> {
    use crate::app::state::SearchTab;

    let Some(ref results) = state.search.results else {
        return vec![];
    };
    let idx = state.list_state.search_item_index;

    let (section, local_idx) = if state.search.tab == SearchTab::Global {
        super::dispatch_search::resolve_global_index(results, idx)
    } else {
        (state.search.tab, idx)
    };

    match section {
        SearchTab::Artists => {
            if let Some(artist) = results.artists.get(local_idx) {
                return vec![QueueAction::EnqueueArtistTracks {
                    artist_key: artist.rating_key.clone(),
                    artist_name: artist.title.clone(),
                }
                .into()];
            }
        }
        SearchTab::Albums => {
            if let Some(album) = results.albums.get(local_idx) {
                return vec![QueueAction::EnqueueAlbum {
                    rating_key: album.rating_key.clone(),
                    title: album.title.clone(),
                }
                .into()];
            }
        }
        SearchTab::Tracks => {
            // Get selected track + all following tracks, add directly to queue
            let tracks: Vec<Track> = results.tracks[local_idx..].to_vec();
            if !tracks.is_empty() {
                // If radio is playing, convert to queue mode
                if state.playback_mode == PlaybackMode::Radio {
                    state.queue.tracks = state.radio.tracks.clone();
                    state.queue.index = state.radio.track_index;
                    state.set_playback_mode(PlaybackMode::Queue);
                    state.radio.clear();
                    if let Some(idx) = state.queue.index {
                        state.list_state.queue_index = idx;
                    }
                }
                state.queue.original.clear();
                state.queue.sort_mode = QueueSortMode::QueueOrder;
                let added = tracks.len();
                state.queue.tracks.extend(tracks);
                state.set_status(format!(
                    "Added {} to queue ({} total)",
                    added,
                    state.queue.tracks.len()
                ));
            }
        }
        SearchTab::Playlists | SearchTab::Genres | SearchTab::Global => {
            // Playlists and genres can't be directly enqueued
        }
    }

    vec![]
}

/// Build actions to play a search result (Shift+Enter/Shift+Click in search).
/// Uses PlayAlbum/PlayArtistTracks/PlayTrack to add to queue AND start playback.
fn play_search_result(state: &AppState) -> Vec<Action> {
    use crate::app::state::SearchTab;

    let Some(ref results) = state.search.results else {
        return vec![];
    };
    let idx = state.list_state.search_item_index;

    let (section, local_idx) = if state.search.tab == SearchTab::Global {
        super::dispatch_search::resolve_global_index(results, idx)
    } else {
        (state.search.tab, idx)
    };

    match section {
        SearchTab::Artists => {
            if let Some(artist) = results.artists.get(local_idx) {
                return vec![QueueAction::PlayArtistTracks {
                    artist_key: artist.rating_key.clone(),
                }
                .into()];
            }
        }
        SearchTab::Albums => {
            if let Some(album) = results.albums.get(local_idx) {
                return vec![QueueAction::PlayAlbum {
                    rating_key: album.rating_key.clone(),
                }
                .into()];
            }
        }
        SearchTab::Tracks => {
            if let Some(track) = results.tracks.get(local_idx) {
                return vec![QueueAction::PlayTrack(Box::new(track.clone())).into()];
            }
        }
        SearchTab::Playlists | SearchTab::Genres | SearchTab::Global => {}
    }

    vec![]
}

/// Ctrl+Shift+E in search: insert search result + following NEXT in queue (after current track).
fn enqueue_search_result_next(state: &AppState) -> Vec<Action> {
    use crate::app::state::SearchTab;

    let Some(ref results) = state.search.results else {
        return vec![];
    };
    let idx = state.list_state.search_item_index;

    let (section, local_idx) = if state.search.tab == SearchTab::Global {
        super::dispatch_search::resolve_global_index(results, idx)
    } else {
        (state.search.tab, idx)
    };

    match section {
        SearchTab::Artists => {
            if let Some(artist) = results.artists.get(local_idx) {
                return vec![QueueAction::EnqueueArtistTracksNext {
                    artist_key: artist.rating_key.clone(),
                    artist_name: artist.title.clone(),
                }
                .into()];
            }
        }
        SearchTab::Albums => {
            if let Some(album) = results.albums.get(local_idx) {
                return vec![QueueAction::EnqueueAlbumNext {
                    rating_key: album.rating_key.clone(),
                    title: album.title.clone(),
                }
                .into()];
            }
        }
        SearchTab::Tracks => {
            // Get selected track + all following tracks
            let tracks: Vec<Track> = results.tracks[local_idx..].to_vec();
            if !tracks.is_empty() {
                return vec![QueueAction::EnqueueTracksNext(tracks).into()];
            }
        }
        SearchTab::Playlists | SearchTab::Genres | SearchTab::Global => {
            // Playlists and genres can't be directly enqueued
        }
    }

    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppState;

    fn make_track(key: &str, title: &str) -> Track {
        Track {
            rating_key: key.to_string(),
            title: title.to_string(),
            ..Default::default()
        }
    }

    fn sample_queue() -> Vec<Track> {
        vec![
            make_track("t1", "Track 1"),
            make_track("t2", "Track 2"),
            make_track("t3", "Track 3"),
            make_track("t4", "Track 4"),
        ]
    }

    // --- RemoveFromQueue state logic tests ---

    #[test]
    fn remove_before_current_adjusts_index() {
        let mut state = AppState::new();
        state.queue.tracks = sample_queue();
        state.queue.index = Some(2); // playing Track 3

        // Simulate RemoveFromQueue(0) state mutation
        let idx = 0;
        state.queue.tracks.remove(idx);
        if let Some(current) = state.queue.index {
            if idx < current {
                state.queue.index = Some(current - 1);
            }
        }

        assert_eq!(state.queue.index, Some(1));
        assert_eq!(state.queue.tracks.len(), 3);
        assert_eq!(state.queue.tracks[1].rating_key, "t3"); // Track 3 is still current
    }

    #[test]
    fn remove_after_current_no_change() {
        let mut state = AppState::new();
        state.queue.tracks = sample_queue();
        state.queue.index = Some(1); // playing Track 2

        let idx = 3;
        state.queue.tracks.remove(idx);
        if let Some(current) = state.queue.index {
            if idx < current {
                state.queue.index = Some(current - 1);
            }
        }

        assert_eq!(state.queue.index, Some(1));
        assert_eq!(state.queue.tracks.len(), 3);
    }

    #[test]
    fn remove_current_at_end_wraps_back() {
        let mut state = AppState::new();
        state.queue.tracks = sample_queue();
        state.queue.index = Some(3); // playing last track

        let idx = 3;
        state.queue.tracks.remove(idx);
        if let Some(current) = state.queue.index {
            if idx == current && current >= state.queue.tracks.len() {
                state.queue.index = if state.queue.tracks.is_empty() {
                    None
                } else {
                    Some(state.queue.tracks.len() - 1)
                };
            }
        }

        assert_eq!(state.queue.index, Some(2)); // wraps to new last
    }

    #[test]
    fn remove_last_element_gives_none() {
        let mut state = AppState::new();
        state.queue.tracks = vec![make_track("t1", "Track 1")];
        state.queue.index = Some(0);

        let idx = 0;
        state.queue.tracks.remove(idx);
        if let Some(current) = state.queue.index {
            if idx == current && current >= state.queue.tracks.len() {
                state.queue.index = if state.queue.tracks.is_empty() {
                    None
                } else {
                    Some(state.queue.tracks.len() - 1)
                };
            }
        }

        assert_eq!(state.queue.index, None);
        assert!(state.queue.tracks.is_empty());
    }

    // --- MoveQueueTrack state logic tests ---

    #[test]
    fn move_track_up_swaps_and_adjusts() {
        let mut state = AppState::new();
        state.queue.tracks = sample_queue();
        state.queue.index = Some(2);
        state.list_state.queue_index = 2;

        // Simulate MoveQueueTrackUp
        let idx = state.list_state.queue_index;
        state.queue.tracks.swap(idx, idx - 1);
        state.list_state.queue_index -= 1;
        if let Some(qi) = state.queue.index {
            if qi == idx {
                state.queue.index = Some(idx - 1);
            } else if qi == idx - 1 {
                state.queue.index = Some(idx);
            }
        }

        assert_eq!(state.list_state.queue_index, 1);
        assert_eq!(state.queue.index, Some(1)); // current track moved up
        assert_eq!(state.queue.tracks[1].rating_key, "t3"); // t3 moved from 2 to 1
        assert_eq!(state.queue.tracks[2].rating_key, "t2"); // t2 moved from 1 to 2
    }

    #[test]
    fn move_track_up_at_zero_is_noop() {
        let mut state = AppState::new();
        state.queue.tracks = sample_queue();
        state.list_state.queue_index = 0;

        // MoveQueueTrackUp should be no-op at idx 0
        let idx = state.list_state.queue_index;
        if idx > 0 {
            state.queue.tracks.swap(idx, idx - 1);
        }

        assert_eq!(state.queue.tracks[0].rating_key, "t1"); // unchanged
    }

    #[test]
    fn move_track_down_swaps() {
        let mut state = AppState::new();
        state.queue.tracks = sample_queue();
        state.queue.index = Some(1);
        state.list_state.queue_index = 1;

        // Simulate MoveQueueTrackDown
        let idx = state.list_state.queue_index;
        state.queue.tracks.swap(idx, idx + 1);
        state.list_state.queue_index += 1;
        if let Some(qi) = state.queue.index {
            if qi == idx {
                state.queue.index = Some(idx + 1);
            } else if qi == idx + 1 {
                state.queue.index = Some(idx);
            }
        }

        assert_eq!(state.list_state.queue_index, 2);
        assert_eq!(state.queue.index, Some(2)); // current track moved down
        assert_eq!(state.queue.tracks[1].rating_key, "t3");
        assert_eq!(state.queue.tracks[2].rating_key, "t2");
    }

    #[test]
    fn move_track_down_at_end_is_noop() {
        let mut state = AppState::new();
        state.queue.tracks = sample_queue();
        state.list_state.queue_index = 3; // last index

        let idx = state.list_state.queue_index;
        if idx + 1 < state.queue.tracks.len() {
            state.queue.tracks.swap(idx, idx + 1);
        }

        assert_eq!(state.queue.tracks[3].rating_key, "t4"); // unchanged
    }

    // --- pick_diverse_remix tests ---
}
