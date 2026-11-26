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
    FolderTrackPlan::MissingCatalogTracks
}

/// Dispatch queue actions. Returns follow-up actions.
pub fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: QueueAction,
    state: &mut AppState,

    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    let mut follow_ups = vec![];

    let enemies = matches!(&action, QueueAction::RemixEnemiesReady(_));
    let replacement_name = if matches!(&action, QueueAction::RemixOppositeDayReady(_)) {
        "Opposite Day"
    } else {
        "Doppelganger"
    };
    match action {
        QueueAction::ReorderSelection { from, to } => {
            crate::app::queue_edit::reorder(state, from, to);
        }
        QueueAction::SetRepeat(mode) => {
            state.queue.repeat = mode;
        }
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
                crate::app::queue_edit::record(state, "Play tracks");
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

        QueueAction::EnqueueTrack(track) => {
            crate::app::queue_edit::record(state, "Add track to queue");
            let title = track.title.clone();
            state.playback_buffer_mut().0.push(*track);
            state.set_status(format!("Added \"{}\" to queue", title));
        }

        QueueAction::EnqueueSearchResult => {
            // Enqueue the selected search result (at end of queue)
            let follow_up = enqueue_search_result(state);
            follow_ups.extend(follow_up);
            // Close search popup and navigate to queue
            state.popups.set_search_active(false);
            state.set_view(View::Queue);
        }
        QueueAction::EnqueueSearchResultNext => {
            // Ctrl+Shift+E: insert search result NEXT in queue (after current track)
            let follow_up = enqueue_search_result_next(state);
            follow_ups.extend(follow_up);
            // Close search popup and navigate to queue
            state.popups.set_search_active(false);
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
                        state.playback_tracks().len()
                    ));
                } else {
                    state.set_status(format!(
                        "Inserted {} tracks next, starting with \"{}\" ({} total)",
                        added,
                        title,
                        state.playback_tracks().len()
                    ));
                }
            }
        }
        QueueAction::TracksLoaded { intent, result } => {
            if let QueueLoadIntent::ReplaceAndPlay { request_id, .. } = &intent
                && *request_id != state.queue_play_request_id
            {
                tracing::debug!("Ignoring stale queue play load");
                return Ok(follow_ups);
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
                    crate::app::queue_edit::record(state, "Add tracks to queue");
                    let added = tracks.len();
                    state.playback_buffer_mut().0.extend(tracks);
                    state.queue.original.clear();
                    state.queue.sort_mode = QueueSortMode::QueueOrder;
                    state.set_status(format!("Added {added} tracks {label} to queue"));
                }
                QueueLoadIntent::InsertNext { label } => {
                    let added = helpers::insert_tracks_next(state, tracks);
                    state.set_status(format!(
                        "Inserted {added} tracks {label} next ({} total)",
                        state.playback_tracks().len()
                    ));
                }
            }
        }
        QueueAction::SelectHistory(id) => {
            if let Some(index) = state.listening_history.iter().position(|p| p.id == id) {
                state.history_cursor = index;
            }
        }
        QueueAction::HistoryTrack(id, command) => {
            if let Some(play) = state.listening_history.iter().find(|p| p.id == id) {
                if let Some(track) = play.resolve(state) {
                    follow_ups.extend(crate::app::command_palette::run(
                        crate::app::state::PaletteCommandKind::FromTrackContext {
                            kind: command,
                            track: Box::new(track),
                        },
                        state,
                    ));
                } else {
                    state.set_status("Track unavailable in the current library".into());
                }
            }
        }
        QueueAction::ShowHistory(show) => {
            state.scroll.history_offset = None;
            state.scroll.history_click = None;
            state.showing_queue_history = show;
            state.history_cursor = 0;
        }
        QueueAction::ClearHistory => {
            state.listening_history = crate::app::listening_history::save(None)?;
            state.history_cursor = 0;
        }
        QueueAction::ClearQueue => {
            // Preserve the complete session before clearing; undo restores the queue
            // without unexpectedly restarting stopped playback.
            if !state.playback_tracks().is_empty() {
                state.queue.undo_snapshot = Some(crate::app::state::QueueSnapshot {
                    recorded_at: std::time::Instant::now(),
                    contents: if state.playback_mode == PlaybackMode::Radio {
                        crate::app::state::QueueContents::Radio(Box::new(state.radio.clone()))
                    } else {
                        crate::app::state::QueueContents::Queue {
                            tracks: state.queue.tracks.clone(),
                            index: state.queue.index,
                        }
                    },
                    description: "Clear Queue".into(),
                });
            }
            state.set_playback_mode(PlaybackMode::None);
            state.radio.clear();
            state.queue.tracks.clear();
            state.queue.index = None;
            state.queue.original.clear();
            state.queue.sort_mode = QueueSortMode::QueueOrder;
            state.queue.shuffle_undo_queue = None;
            state.queue.shuffle_undo_index = None;
            state.queue_play_request_id = state.queue_play_request_id.wrapping_add(1);
            state.sources.media = crate::app::sources::MediaPreparation::Idle;
            state.playback.preparation_id = state.playback.preparation_id.wrapping_add(1);
            state.list_state.queue_index = 0;
            audio.stop();
            state.playback.request_id = audio.playback_id();
            state.playback.status = PlayStatus::Stopped;
            helpers::reset_current_track(state);
        }
        QueueAction::ToggleQueueShuffle => {
            use crate::services::shuffle_queue;

            if state.queue.sort_mode == QueueSortMode::Shuffle && state.queue.original.is_empty() {
                state.queue.sort_mode = QueueSortMode::QueueOrder;
            }

            match state.queue.sort_mode {
                QueueSortMode::QueueOrder => {
                    let (buffer, index) = state.playback_buffer_mut();
                    let original = std::mem::take(buffer);
                    (*buffer, *index) = shuffle_queue(original.clone(), *index);
                    state.queue.original = original;
                    state.queue.sort_mode = QueueSortMode::Shuffle;
                    state.list_state.queue_index = 0;
                }
                QueueSortMode::Shuffle => {
                    let original = std::mem::take(&mut state.queue.original);
                    let current_key = state.current_track().map(|track| track.rating_key.clone());
                    let (buffer, index) = state.playback_buffer_mut();
                    *buffer = original;
                    if let Some(key) = current_key {
                        *index = buffer.iter().position(|track| track.rating_key == key);
                    }
                    if let Some(index) = *index {
                        state.list_state.queue_index = index;
                    }
                    state.queue.sort_mode = QueueSortMode::QueueOrder;
                }
            }
        }
        QueueAction::RemoveFromQueue(idx) => {
            follow_ups.extend(remove_tracks(
                event_tx,
                state,
                audio,
                &std::collections::BTreeSet::from([idx]),
            ));
        }
        QueueAction::RemoveQueueSide { row, above } => {
            let len = state.playback_tracks().len();
            if row < len {
                let range = if above { 0..row } else { row + 1..len };
                follow_ups.extend(remove_tracks(event_tx, state, audio, &range.collect()));
            }
        }
        QueueAction::JumpToQueueIndex(idx) => {
            if idx < state.queue.tracks.len() {
                state.set_playback_mode(PlaybackMode::Queue);
                let idx = crate::app::queue_edit::before_jump(state, idx);
                state.queue.index = Some(idx);
                state.list_state.queue_index = idx;
                helpers::play_current_track(event_tx, state, audio);
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
                        return Ok(vec![
                            QueueAction::TracksLoaded {
                                intent,
                                result: Ok(tracks),
                            }
                            .into(),
                        ]);
                    }
                    FolderTrackPlan::Ready(_) => {}
                    FolderTrackPlan::MissingCatalogTracks => {
                        state.set_error(
                            "Folder tracks are no longer in the catalog; choose Refresh Library"
                                .into(),
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
                crate::app::queue_edit::record(state, "Add tracks to queue");

                state.queue.original.clear();
                state.queue.sort_mode = QueueSortMode::QueueOrder;

                let added = tracks_to_add.len();
                state.playback_buffer_mut().0.extend(tracks_to_add);
                state.set_status(format!(
                    "Added {} to queue ({} total)",
                    added,
                    state.playback_tracks().len()
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
                    QueueAction::EnqueueAlbumNext { rating_key, title }.into(),
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
                        return Ok(vec![
                            QueueAction::TracksLoaded {
                                intent,
                                result: Ok(tracks),
                            }
                            .into(),
                        ]);
                    }
                    FolderTrackPlan::Ready(_) => {}
                    FolderTrackPlan::MissingCatalogTracks => {
                        state.set_error(
                            "Folder tracks are no longer in the catalog; choose Refresh Library"
                                .into(),
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
        QueueAction::PromptImportPlaylist => {
            if state.sources.active.navidrome().is_none() || state.downloads.is_offline() {
                state.set_error("Connect to a writable playlist library to import".into());
                return Ok(vec![]);
            }
            state.popups.close_all();
            state
                .popups
                .set_input_dialog(Some(crate::app::state::InputDialog {
                    title: "Import Playlist".into(),
                    input: Default::default(),
                    action_type: crate::app::state::InputDialogAction::ImportPlaylist {
                        generation: state.library_generation,
                    },
                }));
        }
        QueueAction::ImportPlaylist { generation, path } => {
            if generation != state.library_generation || state.downloads.is_offline() {
                return Ok(vec![]);
            }
            if state.sources.active.navidrome().is_none() {
                return Ok(vec![]);
            }
            if !matches!(
                state.library.catalog_loading,
                crate::app::catalog_loading::State::Ready
            ) {
                state.set_error("The full track catalog is required to import a playlist".into());
                return Ok(vec![]);
            }
            let catalog = state.library.all_tracks.clone();
            crate::app::sources::navidrome::effects::spawn(
                state,
                event_tx,
                "playlist-import",
                async move {
                    let (name, keys) = crate::app::tasks::spawn_blocking(move || {
                        crate::services::playlist_export::read_playlist(&path, &catalog)
                    })
                    .await??;
                    let addition = crate::app::sources::navidrome::playlist_edit::Addition::Tracks(
                        keys.into(),
                    );
                    Ok(vec![
                        crate::app::sources::navidrome::NavAction::Command(
                            crate::app::sources::navidrome::commands::Command::CreatePlaylist {
                                generation,
                                name,
                                addition: Some(addition),
                            },
                        )
                        .into(),
                    ])
                },
            );
        }
        QueueAction::PromptExportPlaylist { format, queue } => {
            let (title, tracks) = if queue {
                ("Queue".to_owned(), state.playback_tracks())
            } else if let Some(column) = state.playlist_nav.columns.get(1).filter(|column| {
                !state.playlist_nav.loading
                    && state.playlist_nav.columns.first().and_then(|root| root.selected_item()).is_some_and(|selected| matches!(&column.play_all_row, Some(crate::app::state::PlayAllRow::Playlist { rating_key, .. }) if rating_key == selected.key()))
                    && column.lazy.as_ref().is_none_or(|lazy| {
                    lazy.total
                        .is_none_or(|total| column.tracks.len() >= total as usize)
                })
            }) {
                (
                    state
                        .playlist_nav
                        .columns
                        .first()
                        .and_then(|root| root.selected_item())
                        .map(|item| item.title().to_owned())
                        .unwrap_or_else(|| column.title.clone()),
                    column.tracks.as_slice(),
                )
            } else {
                state.set_error("Open a complete playlist before exporting".into());
                return Ok(vec![]);
            };
            if tracks.is_empty() {
                state.set_error("No tracks to export".into());
                return Ok(vec![]);
            }
            let contents = match crate::services::playlist_export::encode(tracks, format) {
                Ok(text) => std::sync::Arc::new(text.into_bytes()),
                Err(error) => {
                    state.set_error(error.to_string());
                    return Ok(vec![]);
                }
            };
            let name: String = title
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || " -_".contains(c) {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let directory = dirs::download_dir()
                .or_else(dirs::home_dir)
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let path = directory.join(format!("{}.{}", name.trim(), format.extension()));
            state.popups.close_all();
            state
                .popups
                .set_input_dialog(Some(crate::app::state::InputDialog {
                    title: "Export Playlist".into(),
                    input: path.to_string_lossy().as_ref().into(),
                    action_type: crate::app::state::InputDialogAction::ExportPlaylist {
                        generation: state.library_generation,
                        contents,
                        format,
                    },
                }));
        }
        QueueAction::ExportPlaylist {
            generation,
            contents,
            path,
        } => {
            if generation != state.library_generation {
                return Ok(vec![]);
            }
            anyhow::ensure!(!path.as_os_str().is_empty(), "Choose an export path");
            let tx = event_tx.clone();
            let target = path.clone();
            crate::app::tasks::spawn(async move {
                let result = crate::app::tasks::spawn_blocking(move || {
                    crate::services::playlist_export::write(&target, &contents)
                })
                .await
                .map_err(|error| crate::app::action::AsyncError {
                    message: error.to_string(),
                })
                .and_then(|result| {
                    result.map_err(|error| crate::app::action::AsyncError {
                        message: error.to_string(),
                    })
                });
                let _ = tx
                    .send(Event::Effect(
                        QueueAction::PlaylistExported {
                            generation,
                            path,
                            result,
                        }
                        .into(),
                    ))
                    .await;
            });
        }
        QueueAction::PlaylistExported {
            generation,
            path,
            result,
        } => {
            if generation != state.library_generation {
                return Ok(vec![]);
            }
            match result {
                Ok(()) => state.set_status(format!("Exported {}", path.display())),
                Err(error) => state.set_error(error.message),
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
                state
                    .popups
                    .set_input_dialog(Some(crate::app::state::InputDialog {
                        title: title.to_string(),
                        input: Default::default(),
                        action_type: crate::app::state::InputDialogAction::SavePlaylist,
                    }));
            }
        }

        QueueAction::QueuePlaylistSaved {
            name,
            track_count,
            result,
        } => match result {
            Ok(playlist) => {
                let mut playlists = state.library.playlists.as_ref().clone();
                playlists.retain(|existing| existing.rating_key != playlist.rating_key);
                playlists.push(playlist);
                state.replace_playlists(playlists);
                state
                    .desktop_collapsed_sections
                    .retain(|section| section != "Playlists");
                state.set_status(format!("Saved \"{name}\" ({track_count} tracks)"));
                return Ok(vec![DataAction::LoadPlaylists.into()]);
            }
            Err(error) => {
                state.set_error(error.message);
            }
        },

        QueueAction::RemixShuffle => {
            crate::app::queue_edit::record(state, "Remix: Shuffle");
            let (buffer, playing) = state.playback_buffer_mut();
            let original = std::mem::take(buffer);
            let index = *playing;
            (*buffer, *playing) = crate::services::shuffle_queue(original.clone(), index);
            state.queue.shuffle_undo_queue = Some(original);
            state.queue.shuffle_undo_index = index;
            state.list_state.queue_index = 0;
            state.set_status("Queue shuffled".to_string());
        }
        QueueAction::RemixUndoShuffle => {
            if let Some(original) = state.queue.shuffle_undo_queue.take() {
                let current_key = state.current_track().map(|t| t.rating_key.clone());
                let original_index = state.queue.shuffle_undo_index;
                let (buffer, playing) = state.playback_buffer_mut();
                *buffer = original;
                *playing = current_key
                    .and_then(|key| buffer.iter().position(|track| track.rating_key == key))
                    .or(original_index);
                if let Some(idx) = *playing {
                    state.list_state.queue_index = idx;
                }
                state.set_status("Shuffle undone".to_string());
            } else {
                state.set_error("No shuffle to undo".to_string());
            }
        }
        QueueAction::UndoLastRemix => {
            if let Some(snapshot) = state.queue.undo_snapshot.take() {
                let playing = state
                    .current_track()
                    .map(|track| (track.rating_key.clone(), track.origin.clone()));
                let was_playing = matches!(
                    state.playback.status,
                    PlayStatus::Playing | PlayStatus::Buffering
                );
                let was_paused = state.playback.status == PlayStatus::Paused;
                state.sources.nav_tasks.remove("remix");
                state.queue_play_request_id = state.queue_play_request_id.wrapping_add(1);
                match snapshot.contents {
                    crate::app::state::QueueContents::Radio(radio_snap) => {
                        if state.playback_mode == PlaybackMode::Radio {
                            state.invalidate_radio_refill();
                        } else {
                            state.queue.tracks.clear();
                            state.queue.index = None;
                            state.set_playback_mode(PlaybackMode::Radio);
                        }
                        state.radio = *radio_snap;
                        state.radio.refill = crate::app::state::RadioRefill::Idle;
                        state.queue.original.clear();
                        state.set_status(format!("Undid {} — resumed radio", snapshot.description));
                    }
                    crate::app::state::QueueContents::Queue { tracks, index } => {
                        // Normal queue undo
                        state.queue.tracks = tracks;
                        state.queue.index = index;
                        state.queue.original.clear();
                        state.queue.sort_mode = QueueSortMode::QueueOrder;
                        state.set_playback_mode(PlaybackMode::Queue);
                        state.set_status(format!("Undid {}", snapshot.description));
                    }
                }
                state.queue.shuffle_undo_queue = None;
                state.queue.shuffle_undo_index = None;
                state.queue.selected.clear();
                state.queue.selection_anchor = None;
                let restored_index = if state.playback_mode == PlaybackMode::Radio {
                    state.radio.track_index
                } else {
                    state.queue.index
                };
                state.list_state.queue_index = restored_index.unwrap_or(0);
                let retained = playing.as_ref().and_then(|(key, origin)| {
                    let matches =
                        |track: &Track| track.rating_key == *key && track.origin == *origin;
                    let tracks = state.playback_tracks();
                    restored_index
                        .filter(|&index| tracks.get(index).is_some_and(matches))
                        .or_else(|| tracks.iter().position(matches))
                });
                if let Some(index) = retained.filter(|_| was_playing || was_paused) {
                    if state.playback_mode == PlaybackMode::Radio {
                        state.radio.track_index = Some(index);
                    } else {
                        state.queue.index = Some(index);
                    }
                    state.list_state.queue_index = index;
                } else if retained.is_none() || retained != restored_index {
                    audio.stop();
                    state.playback.request_id = audio.playback_id();
                    state.playback.preparation_id = state.playback.preparation_id.wrapping_add(1);
                    state.sources.media = crate::app::sources::MediaPreparation::Idle;
                    state.playback.status = PlayStatus::Stopped;
                    helpers::reset_current_track(state);
                    if was_playing {
                        helpers::play_current_track(event_tx, state, audio);
                    }
                }
                if state.current_track().is_some() {
                    follow_ups.push(crate::app::action::SystemAction::LoadArtwork.into());
                }
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
            let mut selected: Vec<_> = if multiple {
                state.queue.selected.iter().copied().collect()
            } else {
                vec![state.list_state.queue_index]
            };
            let len = state.playback_tracks().len();
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
            crate::app::queue_edit::record(state, "Move tracks");
            let (buffer, playing) = state.playback_buffer_mut();
            for &from in &selected {
                crate::app::state::move_queue_occurrence(
                    buffer,
                    playing,
                    from,
                    from.saturating_add_signed(delta),
                );
            }
            if multiple {
                state.queue.selected = selected
                    .into_iter()
                    .map(|index| index.saturating_add_signed(delta))
                    .collect();
            }
            state.list_state.queue_index = state
                .list_state
                .queue_index
                .saturating_add_signed(delta)
                .min(len.saturating_sub(1));
        }
        QueueAction::RemoveSelectedFromQueue => {
            follow_ups.extend(remove_tracks(
                event_tx,
                state,
                audio,
                &state.queue.selected.clone(),
            ));
        }
        QueueAction::MoveQueueTrack { from, to } => {
            let len = state.playback_tracks().len();
            if from >= len || from == to.min(len - 1) {
                return Ok(follow_ups);
            }
            crate::app::queue_edit::record(state, "Reorder track");
            let (buffer, playing) = state.playback_buffer_mut();
            if let Some(destination) =
                crate::app::state::move_queue_occurrence(buffer, playing, from, to)
            {
                state.list_state.queue_index = destination;
            }
        }
        QueueAction::RemixBatchReady(mut outcome) | QueueAction::RemixEnemiesReady(mut outcome) => {
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
                } else if enemies {
                    state.set_status("Enemies: no eligible tracks in this library".into());
                } else {
                    state.set_status("Remix: no eligible tracks found in this library after excluding queued albums and disallowed artists".to_string());
                }
                return Ok(follow_ups);
            }

            state.queue.undo_snapshot = Some(crate::app::queue_edit::snapshot(
                state,
                if enemies {
                    "Enemies"
                } else {
                    "Insert related tracks"
                },
            ));
            let inserts_map: std::collections::HashMap<usize, Vec<Track>> =
                inserts.into_iter().collect();

            // Process inserts in reverse index order so earlier splices don't shift later indices
            let mut positions: Vec<usize> = inserts_map.keys().copied().collect();
            positions.sort_unstable_by(|a, b| b.cmp(a));

            let mut total_inserted = 0usize;
            let mut focused = state.list_state.queue_index;
            let (buffer, playing) = state.playback_buffer_mut();
            for pos in positions {
                if let Some(insert_tracks) = inserts_map.get(&pos) {
                    let insert_at = (pos + 1).min(buffer.len());
                    total_inserted += insert_tracks.len();
                    if let Some(index) = playing.as_mut()
                        && *index >= insert_at
                    {
                        *index += insert_tracks.len();
                    }
                    if focused >= insert_at {
                        focused += insert_tracks.len();
                    }
                    buffer.splice(insert_at..insert_at, insert_tracks.iter().cloned());
                }
            }

            state.list_state.queue_index = focused;
            if outcome.failed > 0 {
                state.set_status(format!(
                    "Remix: {} tracks added; {} of {} lookups failed",
                    total_inserted, outcome.failed, outcome.attempted
                ));
            } else {
                state.set_status(format!("Remix complete: {} tracks added", total_inserted));
            }
        }
        QueueAction::RemixDoppelgangerReady(mut outcome)
        | QueueAction::RemixOppositeDayReady(mut outcome) => {
            let replacements = std::mem::take(&mut outcome.items);
            if replacements.is_empty() {
                if outcome.failed > 0 {
                    let detail = outcome
                        .first_error
                        .as_ref()
                        .map(|error| error.message.as_str())
                        .unwrap_or("request failed");
                    state.set_error(format!(
                        "Remix {replacement_name} could not replace tracks; {} of {} lookups failed: {}",
                        outcome.failed, outcome.attempted, detail
                    ));
                } else {
                    state.set_status(format!("Remix: {replacement_name} — no replacements found"));
                }
                return Ok(follow_ups);
            }

            state.queue.undo_snapshot =
                Some(crate::app::queue_edit::snapshot(state, replacement_name));
            let count = replacements.len();
            let (buffer, playing) = state.playback_buffer_mut();
            let replaced_current = replacements
                .iter()
                .any(|(index, _)| *playing == Some(*index));

            for (idx, track) in replacements {
                if idx < buffer.len() {
                    buffer[idx] = track;
                }
            }

            if outcome.failed > 0 {
                state.set_status(format!(
                    "Remix: {replacement_name} — {} replaced; {} of {} lookups failed",
                    count, outcome.failed, outcome.attempted
                ));
            } else {
                state.set_status(format!(
                    "Remix: {replacement_name} — {} tracks replaced",
                    count
                ));
            }

            // Replacing only later tracks must not interrupt the current song.
            if replaced_current {
                helpers::reset_current_track(state);
                follow_ups.push(crate::app::action::PlaybackAction::Stop.into());
                follow_ups.push(crate::app::action::SystemAction::LoadArtwork.into());
            }
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
                return vec![
                    QueueAction::EnqueueArtistTracks {
                        artist_key: artist.rating_key.clone(),
                        artist_name: artist.title.clone(),
                    }
                    .into(),
                ];
            }
        }
        SearchTab::Albums => {
            if let Some(album) = results.albums.get(local_idx) {
                return vec![
                    QueueAction::EnqueueAlbum {
                        rating_key: album.rating_key.clone(),
                        title: album.title.clone(),
                    }
                    .into(),
                ];
            }
        }
        SearchTab::Tracks => {
            // Get selected track + all following tracks, add directly to queue
            let tracks: Vec<Track> = results.tracks[local_idx..].to_vec();
            if !tracks.is_empty() {
                crate::app::queue_edit::record(state, "Add tracks to queue");
                state.queue.original.clear();
                state.queue.sort_mode = QueueSortMode::QueueOrder;
                let added = tracks.len();
                state.playback_buffer_mut().0.extend(tracks);
                state.set_status(format!(
                    "Added {} to queue ({} total)",
                    added,
                    state.playback_tracks().len()
                ));
            }
        }
        SearchTab::Playlists | SearchTab::Genres | SearchTab::Global => {
            // Playlists and genres can't be directly enqueued
        }
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
                return vec![
                    QueueAction::EnqueueArtistTracksNext {
                        artist_key: artist.rating_key.clone(),
                        artist_name: artist.title.clone(),
                    }
                    .into(),
                ];
            }
        }
        SearchTab::Albums => {
            if let Some(album) = results.albums.get(local_idx) {
                return vec![
                    QueueAction::EnqueueAlbumNext {
                        rating_key: album.rating_key.clone(),
                        title: album.title.clone(),
                    }
                    .into(),
                ];
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

    #[test]
    fn queue_undo_restores_album_replacement_after_successful_load() {
        let (events, _) = mpsc::channel(8);
        let mut audio = AudioPlayer::new_without_audio();
        for previous in [vec![], sample_queue()] {
            let mut state = AppState::new();
            state.queue.index = (!previous.is_empty()).then_some(2);
            let index = state.queue.index;
            state.queue.tracks = previous.clone();
            state.queue_play_request_id = 7;
            let load = |request_id| QueueAction::TracksLoaded {
                intent: QueueLoadIntent::ReplaceAndPlay {
                    request_id,
                    label: None,
                },
                result: Ok(vec![make_track("album-track", "Album track")]),
            };
            dispatch(&events, load(7), &mut state, &mut audio).unwrap();
            assert_eq!(state.queue.tracks[0].rating_key, "album-track");
            dispatch(&events, load(6), &mut state, &mut audio).unwrap();
            dispatch(&events, load(7), &mut state, &mut audio).unwrap();
            dispatch(&events, QueueAction::UndoLastRemix, &mut state, &mut audio).unwrap();
            assert!(
                state
                    .queue
                    .tracks
                    .iter()
                    .map(|track| &track.rating_key)
                    .eq(previous.iter().map(|track| &track.rating_key))
            );
            assert_eq!(state.queue.index, index);
            assert!(state.queue.undo_snapshot.is_none());
        }
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

fn remove_tracks(
    tx: &mpsc::Sender<Event>,
    state: &mut AppState,
    audio: &mut AudioPlayer,
    selected: &std::collections::BTreeSet<usize>,
) -> Option<Action> {
    let status = state.playback.status;
    if crate::app::queue_edit::remove(state, selected)
        != crate::app::queue_edit::Removal::RemovedPlayback
    {
        return None;
    }
    audio.stop();
    state.playback.request_id = audio.playback_id();
    state.playback.preparation_id = state.playback.preparation_id.wrapping_add(1);
    state.sources.media = crate::app::sources::MediaPreparation::Idle;
    state.playback.status = PlayStatus::Stopped;
    helpers::reset_current_track(state);
    if state.current_track().is_some() && status != PlayStatus::Stopped {
        helpers::play_current_track(tx, state, audio);
        if status == PlayStatus::Paused && state.playback.status != PlayStatus::Stopped {
            audio.pause();
            state.playback.status = PlayStatus::Paused;
        }
    }
    state
        .current_track()
        .map(|_| crate::app::action::SystemAction::LoadArtwork.into())
}
