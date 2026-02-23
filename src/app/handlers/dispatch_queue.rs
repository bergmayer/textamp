//! Queue dispatch handlers: PlayTrack, PlayTrackFromCategory, PlayAlbum, EnqueueAlbum,
//! ClearQueue, RemoveFromQueue, JumpToQueueIndex,
//! EnqueueSelection, PromptSavePlaylist, SaveQueueAsPlaylist.

use crate::app::{Action, AppState, Event};
use crate::app::state::{BrowseCategory, BrowseItem, PlayStatus, PlaybackMode, QueueSortMode, SimilarMode, View};
use crate::plex::PlexClient;
use crate::plex::models::Track;
use crate::audio::AudioPlayer;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Dispatch queue actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: Action,
    state: &mut AppState,
    client: &mut PlexClient,
    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    let mut follow_ups = vec![];

    match action {
        Action::PlayTrack(track) => {
            helpers::play_track(event_tx, track, state, client, audio).await;
        }
        Action::PlayTrackFromCategory(idx) => {
            if idx < state.selected_album_tracks.len() {
                // Report stop for currently playing track before switching
                if let Some(current) = state.current_track().cloned() {
                    helpers::report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
                }

                // Generate new session ID for this playback context
                state.plex_session_id = Some(helpers::generate_plex_session_id());

                // Clear radio state if switching from radio mode
                if state.playback_mode == PlaybackMode::Radio {
                    state.radio.clear();
                }

                // Prepend new tracks at front of queue
                let new_tracks: Vec<Track> = state.selected_album_tracks[idx..].to_vec();
                state.queue.splice(0..0, new_tracks);
                state.queue_index = Some(0);
                state.queue_original.clear();
                state.queue_sort_mode = QueueSortMode::QueueOrder;
                state.playback_mode = PlaybackMode::Queue;
                state.list_state.queue_index = 0;
                audio.track_cache.flush();
                helpers::play_current_track(event_tx, state, client, audio).await;
            }
        }
        Action::PlayAlbum { rating_key } => {
            // Load album tracks and play them (Shift+Enter on album)
            match client.get_album_tracks(&rating_key).await {
                Ok(tracks) => {
                    if !tracks.is_empty() {
                        helpers::queue_and_play(event_tx, state, client, audio, tracks, 0).await;
                    }
                }
                Err(e) => {
                    state.set_error(format!("Failed to load album: {}", e));
                }
            }
        }
        Action::PlayArtistTracks { artist_key } => {
            // Load all tracks by artist and play them (Shift+Enter on artist)
            match client.get_artist_all_tracks(&artist_key).await {
                Ok(tracks) => {
                    if !tracks.is_empty() {
                        helpers::queue_and_play(event_tx, state, client, audio, tracks, 0).await;
                    }
                }
                Err(e) => {
                    state.set_error(format!("Failed to load artist tracks: {}", e));
                }
            }
        }
        Action::PlaySearchResult => {
            // Play the selected search result (Shift+Enter in search)
            let play_actions = play_search_result(state);
            follow_ups.extend(play_actions);
        }
        Action::EnqueueAlbum { rating_key, title } => {
            // Load album tracks and append to end of queue
            match client.get_album_tracks(&rating_key).await {
                Ok(tracks) => {
                    if !tracks.is_empty() {
                        // If radio is playing, convert to queue mode first
                        if state.playback_mode == PlaybackMode::Radio {
                            state.queue = state.radio.tracks.clone();
                            state.queue_index = state.radio.track_index;
                            state.playback_mode = PlaybackMode::Queue;
                            state.radio.clear();
                            if let Some(idx) = state.queue_index {
                                state.list_state.queue_index = idx;
                            }
                        }

                        let added = tracks.len();
                        let insert_pos = state.queue.len();
                        state.queue.splice(insert_pos..insert_pos, tracks);
                        state.set_status(format!("Added {} tracks from \"{}\" to queue", added, title));
                    }
                }
                Err(e) => {
                    state.set_error(format!("Failed to load album: {}", e));
                }
            }
        }
        Action::EnqueueArtistTracks { artist_key, artist_name } => {
            // Load all tracks by artist and append to end of queue
            match client.get_artist_all_tracks(&artist_key).await {
                Ok(tracks) => {
                    if !tracks.is_empty() {
                        if state.playback_mode == PlaybackMode::Radio {
                            state.queue = state.radio.tracks.clone();
                            state.queue_index = state.radio.track_index;
                            state.playback_mode = PlaybackMode::Queue;
                            state.radio.clear();
                            if let Some(idx) = state.queue_index {
                                state.list_state.queue_index = idx;
                            }
                        }
                        let added = tracks.len();
                        let insert_pos = state.queue.len();
                        state.queue.splice(insert_pos..insert_pos, tracks);
                        state.set_status(format!("Added {} tracks by \"{}\" to queue", added, artist_name));
                    }
                }
                Err(e) => {
                    state.set_error(format!("Failed to load artist tracks: {}", e));
                }
            }
        }
        Action::EnqueueTrack(track) => {
            // Append a single track to end of queue
            if state.playback_mode == PlaybackMode::Radio {
                state.queue = state.radio.tracks.clone();
                state.queue_index = state.radio.track_index;
                state.playback_mode = PlaybackMode::Queue;
                state.radio.clear();
                if let Some(idx) = state.queue_index {
                    state.list_state.queue_index = idx;
                }
            }
            let title = track.title.clone();
            state.queue.push(track);
            state.set_status(format!("Added \"{}\" to queue", title));
        }
        Action::EnqueueSearchResult => {
            // Enqueue the selected search result (at end of queue)
            let follow_up = enqueue_search_result(state);
            follow_ups.extend(follow_up);
            // Close search popup and navigate to queue
            state.popups.search_active = false;
            state.set_view(View::Queue);
        }
        Action::EnqueueSearchResultNext => {
            // Ctrl+E: enqueue search result at TOP of queue and start playing
            let follow_up = enqueue_search_result_next(state);
            follow_ups.extend(follow_up);
            // Close search popup and navigate to queue
            state.popups.search_active = false;
            state.set_view(View::Queue);
        }
        Action::EnqueueAlbumNext { rating_key, title } => {
            // Shift+Enter: Insert album tracks at TOP of queue and start playing
            match client.get_album_tracks(&rating_key).await {
                Ok(tracks) => {
                    if !tracks.is_empty() {
                        let added = tracks.len();
                        helpers::queue_and_play(event_tx, state, client, audio, tracks, 0).await;
                        state.set_status(format!("Playing {} tracks from \"{}\"", added, title));
                    }
                }
                Err(e) => {
                    state.set_error(format!("Failed to load album: {}", e));
                }
            }
        }
        Action::EnqueueArtistTracksNext { artist_key, artist_name } => {
            // Shift+Enter: Insert artist tracks at TOP of queue and start playing
            match client.get_artist_all_tracks(&artist_key).await {
                Ok(tracks) => {
                    if !tracks.is_empty() {
                        let added = tracks.len();
                        helpers::queue_and_play(event_tx, state, client, audio, tracks, 0).await;
                        state.set_status(format!("Playing {} tracks by \"{}\"", added, artist_name));
                    }
                }
                Err(e) => {
                    state.set_error(format!("Failed to load artist tracks: {}", e));
                }
            }
        }
        Action::EnqueueTracksNext(tracks) => {
            // Shift+Enter: Insert tracks at TOP of queue and start playing
            if !tracks.is_empty() {
                let added = tracks.len();
                let title = tracks.first().map(|t| t.title.clone()).unwrap_or_default();
                helpers::queue_and_play(event_tx, state, client, audio, tracks, 0).await;
                if added == 1 {
                    state.set_status(format!("Playing \"{}\"", title));
                } else {
                    state.set_status(format!("Playing {} tracks starting with \"{}\"", added, title));
                }
            }
        }
        Action::ClearQueue => {
            // Clear the appropriate queue based on playback mode
            match state.playback_mode {
                PlaybackMode::Radio => {
                    state.radio.clear();
                }
                PlaybackMode::Queue | PlaybackMode::None => {
                    state.queue.clear();
                    state.queue_index = None;
                    state.queue_original.clear();
                    state.queue_sort_mode = QueueSortMode::QueueOrder;
                }
            }
            state.list_state.queue_index = 0;
            audio.stop();
            audio.track_cache.flush();
            state.playback.status = PlayStatus::Stopped;
        }
        Action::ToggleQueueShuffle => {
            use crate::audio::cache;
            use crate::services::PlaybackService;

            if state.playback_mode == PlaybackMode::Radio {
                // Shuffle/unshuffle radio tracks
                match state.queue_sort_mode {
                    QueueSortMode::QueueOrder => {
                        state.queue_original = state.radio.tracks.clone();
                        let (shuffled, new_idx) = PlaybackService::shuffle_queue(
                            state.radio.tracks.clone(), state.radio.track_index,
                        );
                        state.radio.tracks = shuffled;
                        state.radio.track_index = new_idx;
                        state.queue_sort_mode = QueueSortMode::Shuffle;
                    }
                    QueueSortMode::Shuffle => {
                        let current_key = state.current_track().map(|t| t.rating_key.clone());
                        state.radio.tracks = std::mem::take(&mut state.queue_original);
                        state.queue_sort_mode = QueueSortMode::QueueOrder;
                        if let Some(key) = current_key {
                            state.radio.track_index = state.radio.tracks.iter().position(|t| t.rating_key == key);
                        }
                    }
                }
            } else {
                // Shuffle/unshuffle queue tracks
                match state.queue_sort_mode {
                    QueueSortMode::QueueOrder => {
                        state.queue_original = state.queue.clone();
                        let (shuffled, new_idx) = PlaybackService::shuffle_queue(
                            state.queue.clone(), state.queue_index,
                        );
                        state.queue = shuffled;
                        state.queue_index = new_idx; // always Some(0)
                        state.queue_sort_mode = QueueSortMode::Shuffle;
                        state.list_state.queue_index = 0;
                    }
                    QueueSortMode::Shuffle => {
                        let current_key = state.current_track().map(|t| t.rating_key.clone());
                        state.queue = std::mem::take(&mut state.queue_original);
                        state.queue_sort_mode = QueueSortMode::QueueOrder;
                        if let Some(key) = current_key {
                            state.queue_index = state.queue.iter().position(|t| t.rating_key == key);
                        }
                        if let Some(idx) = state.queue_index {
                            state.list_state.queue_index = idx;
                        }
                    }
                }
            }
            // Flush and re-prefetch based on new order
            audio.track_cache.flush();
            let upcoming = helpers::get_upcoming_tracks(state);
            cache::trigger_prefetch(&audio.track_cache, &upcoming, client);
        }
        Action::RemoveFromQueue(idx) => {
            if idx < state.queue.len() {
                state.queue.remove(idx);
                // Adjust queue_index if needed
                if let Some(current) = state.queue_index {
                    if idx < current {
                        state.queue_index = Some(current - 1);
                    } else if idx == current && current >= state.queue.len() {
                        state.queue_index = if state.queue.is_empty() {
                            None
                        } else {
                            Some(state.queue.len() - 1)
                        };
                    }
                }
                // Adjust list selection
                let max_visual = state.queue.len().saturating_sub(1);
                if state.list_state.queue_index > max_visual {
                    state.list_state.queue_index = max_visual;
                }
            }
        }
        Action::JumpToQueueIndex(idx) => {
            // Jump to and play a specific track in the queue (without modifying queue order)
            if idx < state.queue.len() {
                // Report stop for currently playing track before switching
                if let Some(current) = state.current_track().cloned() {
                    helpers::report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
                }

                // Generate new session ID for this playback context
                state.plex_session_id = Some(helpers::generate_plex_session_id());

                state.queue_index = Some(idx);
                state.list_state.queue_index = idx;
                state.playback_mode = PlaybackMode::Queue;
                audio.track_cache.flush();
                helpers::play_current_track(event_tx, state, client, audio).await;

                // Trigger DJ mode processing after jump (all modes are continuous)
                if !state.dj.inserting && state.dj.active_mode.is_some() {
                    follow_ups.push(Action::DjModeProcess);
                }
            }
        }
        Action::EnqueueSelection => {
            // Ctrl+Shift+E: Add selected item + all following items to END of queue
            // For tracks: enqueue selected track + all following tracks in the view
            // For albums: enqueue the album

            // Check for album selection first
            let album_to_enqueue: Option<(String, String)> = match state.view {
                View::Browse => {
                    let nav = match state.browse_category {
                        BrowseCategory::Library => Some(&state.artist_nav),
                        BrowseCategory::Genres => Some(&state.genre_nav),
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
                View::Similar => {
                    match state.similar.mode {
                        SimilarMode::Albums => {
                            state.similar.albums.get(state.list_state.similar_index)
                                .map(|a| (a.rating_key.clone(), a.title.clone()))
                        }
                        _ => None,
                    }
                }
                View::Related => {
                    let idx = state.list_state.related_index;
                    let resolved = super::helpers::navigation::related_flat_resolve(&state.related.groups, idx);
                    resolved.and_then(|(gi, is_header, ai)| {
                        if is_header { return None; }
                        state.related.groups.get(gi)
                            .and_then(|g| g.albums.get(ai))
                            .map(|a| (a.rating_key.clone(), a.title.clone()))
                    })
                }
                _ => None,
            };

            if let Some((rating_key, title)) = album_to_enqueue {
                return Ok(vec![Action::EnqueueAlbum { rating_key, title }]);
            }

            // Get tracks from selected index to end
            let tracks_to_add: Vec<Track> = match state.view {
                View::Browse => {
                    // Miller columns: get selected track + all following
                    let nav = match state.browse_category {
                        BrowseCategory::Library => Some(&state.artist_nav),
                        BrowseCategory::Genres => Some(&state.genre_nav),
                        BrowseCategory::Playlists => Some(&state.playlist_nav),
                        _ => None,
                    };
                    if let Some(nav) = nav {
                        if let Some(col) = nav.columns.get(nav.focused_column) {
                            if let Some(BrowseItem::Track { .. }) = col.items.get(col.selected_index) {
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
                            let idx = state.list_state.similar_index;
                            state.similar.tracks[idx..].to_vec()
                        }
                        _ => vec![],
                    }
                }
                View::Search => {
                    use crate::app::state::SearchTab;
                    if let Some(ref results) = state.search_results {
                        let idx = state.list_state.search_item_index;
                        let (section, local_idx) = if state.search_tab == SearchTab::Global {
                            super::dispatch_search::resolve_global_index(results, idx)
                        } else {
                            (state.search_tab, idx)
                        };
                        match section {
                            SearchTab::Tracks => {
                                results.tracks[local_idx..].to_vec()
                            }
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
                    state.queue = state.radio.tracks.clone();
                    state.queue_index = state.radio.track_index;
                    state.playback_mode = PlaybackMode::Queue;
                    state.radio.clear();
                    if let Some(idx) = state.queue_index {
                        state.list_state.queue_index = idx;
                    }
                }

                state.queue_original.clear();
                state.queue_sort_mode = QueueSortMode::QueueOrder;

                let added = tracks_to_add.len();
                state.queue.extend(tracks_to_add);
                state.set_status(format!("Added {} to queue ({} total)", added, state.queue.len()));
            }
        }
        Action::EnqueueSelectionNext => {
            // Ctrl+E: Add selected item + all following items to TOP of queue and start playing
            // For tracks: enqueue selected track + all following tracks in the view
            // For albums: enqueue the album

            // Check for album selection first
            let album_to_enqueue: Option<(String, String)> = match state.view {
                View::Browse => {
                    let nav = match state.browse_category {
                        BrowseCategory::Library => Some(&state.artist_nav),
                        BrowseCategory::Genres => Some(&state.genre_nav),
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
                View::Similar => {
                    match state.similar.mode {
                        SimilarMode::Albums => {
                            state.similar.albums.get(state.list_state.similar_index)
                                .map(|a| (a.rating_key.clone(), a.title.clone()))
                        }
                        _ => None,
                    }
                }
                View::Related => {
                    let idx = state.list_state.related_index;
                    let resolved = super::helpers::navigation::related_flat_resolve(&state.related.groups, idx);
                    resolved.and_then(|(gi, is_header, ai)| {
                        if is_header { return None; }
                        state.related.groups.get(gi)
                            .and_then(|g| g.albums.get(ai))
                            .map(|a| (a.rating_key.clone(), a.title.clone()))
                    })
                }
                _ => None,
            };

            if let Some((rating_key, title)) = album_to_enqueue {
                return Ok(vec![Action::EnqueueAlbumNext { rating_key, title }]);
            }

            // Get tracks from selected index to end
            let tracks_to_add: Vec<Track> = match state.view {
                View::Browse => {
                    // Miller columns: get selected track + all following
                    let nav = match state.browse_category {
                        BrowseCategory::Library => Some(&state.artist_nav),
                        BrowseCategory::Genres => Some(&state.genre_nav),
                        BrowseCategory::Playlists => Some(&state.playlist_nav),
                        _ => None,
                    };
                    if let Some(nav) = nav {
                        if let Some(col) = nav.columns.get(nav.focused_column) {
                            if let Some(BrowseItem::Track { .. }) = col.items.get(col.selected_index) {
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
                    if let Some(ref results) = state.search_results {
                        let idx = state.list_state.search_item_index;
                        let (section, local_idx) = if state.search_tab == SearchTab::Global {
                            super::dispatch_search::resolve_global_index(results, idx)
                        } else {
                            (state.search_tab, idx)
                        };
                        match section {
                            SearchTab::Tracks => {
                                results.tracks[local_idx..].to_vec()
                            }
                            _ => vec![],
                        }
                    } else {
                        vec![]
                    }
                }
                _ => vec![],
            };

            if !tracks_to_add.is_empty() {
                return Ok(vec![Action::EnqueueTracksNext(tracks_to_add)]);
            }
        }
        Action::PromptSavePlaylist => {
            // Show input dialog for playlist name
            // Use queue if available, otherwise use radio tracks
            let has_tracks = !state.queue.is_empty() || !state.radio.tracks.is_empty();
            if !has_tracks {
                state.set_error("No tracks to save".to_string());
            } else {
                let title = if !state.queue.is_empty() {
                    "Save Queue as Playlist"
                } else {
                    "Save Station as Playlist"
                };
                state.popups.input_dialog = Some(crate::app::state::InputDialog {
                    title: title.to_string(),
                    input: String::new(),
                    action_type: crate::app::state::InputDialogAction::SavePlaylist,
                });
            }
        }
        Action::SaveQueueAsPlaylist(name) => {
            // Create playlist on Plex server
            // Use queue if available, otherwise use radio tracks
            let tracks: Vec<&Track> = if !state.queue.is_empty() {
                state.queue.iter().collect()
            } else {
                state.radio.tracks.iter().collect()
            };

            tracing::info!("SaveQueueAsPlaylist: name='{}', track_count={}", name, tracks.len());

            if tracks.is_empty() {
                state.set_error("No tracks to save".to_string());
            } else if name.trim().is_empty() {
                state.set_error("Playlist name cannot be empty".to_string());
            } else if let Some(ref library_key) = state.active_library {
                let track_keys: Vec<String> = tracks.iter()
                    .map(|t| t.rating_key.clone())
                    .filter(|k| !k.is_empty())  // Filter out any empty keys
                    .collect();

                if track_keys.is_empty() {
                    state.set_error("No valid track keys to save".to_string());
                    tracing::error!("SaveQueueAsPlaylist: All track keys were empty!");
                } else {
                    let track_count = track_keys.len();
                    let name_clone = name.clone();
                    let library_key_clone = library_key.clone();

                    tracing::info!("SaveQueueAsPlaylist: Saving {} tracks with keys: {:?}",
                        track_count, &track_keys[..track_keys.len().min(5)]);

                    state.set_status(format!("Saving playlist \"{}\"...", name));

                    match client.create_playlist(&name_clone, &track_keys, &library_key_clone).await {
                        Ok(()) => {
                            state.set_status(format!("Saved \"{}\" ({} tracks)", name_clone, track_count));
                            // Refresh playlists so the new one appears
                            return Ok(vec![Action::LoadPlaylists]);
                        }
                        Err(e) => {
                            state.set_error(format!("Failed to save playlist: {}", e));
                        }
                    }
                }
            } else {
                state.set_error("No library selected".to_string());
            }
        }
        Action::RemixGemini | Action::RemixTwofer | Action::RemixStretch | Action::RemixDoppelganger => {
            if state.playback_mode == PlaybackMode::Radio {
                let desc = match action {
                    Action::RemixGemini => "Remix: Gemini (from radio)",
                    Action::RemixTwofer => "Remix: Twofer (from radio)",
                    Action::RemixStretch => "Remix: Stretch (from radio)",
                    Action::RemixDoppelganger => "Remix: Doppelganger (from radio)",
                    _ => "Remix (from radio)",
                };
                let snapshot = state.convert_radio_to_queue(desc);
                state.queue_undo_snapshot = Some(snapshot);
            }
            if state.queue.len() < 2 {
                state.set_error("Need at least 2 tracks to remix".to_string());
                return Ok(vec![]);
            }

            // Save snapshot for undo (skip if already set by radio conversion)
            if state.queue_undo_snapshot.is_none() {
                state.queue_undo_snapshot = Some(crate::app::state::QueueSnapshot {
                    queue: state.queue.clone(),
                    queue_index: state.queue_index,
                    description: match action {
                        Action::RemixGemini => "Remix: Gemini".to_string(),
                        Action::RemixTwofer => "Remix: Twofer".to_string(),
                        Action::RemixStretch => "Remix: Stretch".to_string(),
                        Action::RemixDoppelganger => "Remix: Doppelganger".to_string(),
                        _ => "Remix".to_string(),
                    },
                    radio_snapshot: None,
                    radio_state_snapshot: None,
                });
            }

            let mode_name = match action {
                Action::RemixGemini => "Gemini",
                Action::RemixTwofer => "Twofer",
                Action::RemixStretch => "Stretch",
                Action::RemixDoppelganger => "Doppelganger",
                _ => "Remix",
            };
            state.set_status(format!("Remix: {} processing...", mode_name));

            spawn_remix_batch(event_tx, state, client, action);
        }
        Action::RemixShuffle => {
            if state.playback_mode == PlaybackMode::Radio {
                let snapshot = state.convert_radio_to_queue("Remix: Shuffle (from radio)");
                state.queue_undo_snapshot = Some(snapshot);
            }

            use crate::services::PlaybackService;

            // Save snapshot for Ctrl+Z undo (skip if already set by radio conversion)
            if state.queue_undo_snapshot.is_none() {
                state.queue_undo_snapshot = Some(crate::app::state::QueueSnapshot {
                    queue: state.queue.clone(),
                    queue_index: state.queue_index,
                    description: "Remix: Shuffle".to_string(),
                    radio_snapshot: None,
                    radio_state_snapshot: None,
                });
            }

            // Save shuffle-specific undo state (for toggle)
            state.shuffle_undo_queue = Some(state.queue.clone());
            state.shuffle_undo_index = state.queue_index;

            let (shuffled, new_idx) = PlaybackService::shuffle_queue(
                state.queue.clone(), state.queue_index,
            );
            state.queue = shuffled;
            state.queue_index = new_idx;
            state.list_state.queue_index = 0;
            state.set_status("Queue shuffled".to_string());
        }
        Action::RemixUndoShuffle => {
            if let Some(original) = state.shuffle_undo_queue.take() {
                let current_key = state.current_track().map(|t| t.rating_key.clone());
                state.queue = original;
                state.queue_index = state.shuffle_undo_index;
                // Try to keep the currently playing track as the index
                if let Some(key) = current_key {
                    if let Some(idx) = state.queue.iter().position(|t| t.rating_key == key) {
                        state.queue_index = Some(idx);
                    }
                }
                if let Some(idx) = state.queue_index {
                    state.list_state.queue_index = idx;
                }
                state.set_status("Shuffle undone".to_string());
            } else {
                state.set_error("No shuffle to undo".to_string());
            }
        }
        Action::UndoLastRemix => {
            if let Some(snapshot) = state.queue_undo_snapshot.take() {
                if let Some(radio_snap) = snapshot.radio_snapshot {
                    // Restore radio mode
                    state.radio = radio_snap;
                    state.playback_mode = PlaybackMode::Radio;
                    if let Some(rs) = snapshot.radio_state_snapshot {
                        state.radio_state = rs;
                    }
                    state.queue.clear();
                    state.queue_index = None;
                    state.queue_original.clear();
                    if let Some(idx) = state.radio.track_index {
                        state.list_state.queue_index = idx;
                    }
                    state.set_status(format!("Undid {} — resumed radio", snapshot.description));
                } else {
                    // Normal queue undo
                    state.queue = snapshot.queue;
                    state.queue_index = snapshot.queue_index;
                    if let Some(idx) = state.queue_index {
                        state.list_state.queue_index = idx;
                    }
                    state.set_status(format!("Undid {}", snapshot.description));
                }
                state.shuffle_undo_queue = None;
                state.shuffle_undo_index = None;
            } else {
                state.set_error("Nothing to undo".to_string());
            }
        }
        Action::MoveQueueTrackUp => {
            if state.playback_mode == PlaybackMode::Radio {
                let snapshot = state.convert_radio_to_queue("Move track (from radio)");
                state.queue_undo_snapshot = Some(snapshot);
            }
            let idx = state.list_state.queue_index;
            if idx == 0 || idx >= state.queue.len() { return Ok(vec![]); }

            state.queue.swap(idx, idx - 1);
            state.list_state.queue_index -= 1;

            // Adjust queue_index if current track was moved
            if let Some(qi) = state.queue_index {
                if qi == idx {
                    state.queue_index = Some(idx - 1);
                } else if qi == idx - 1 {
                    state.queue_index = Some(idx);
                }
            }
        }
        Action::MoveSelectedTracksUp => {
            if state.playback_mode == PlaybackMode::Radio {
                let snapshot = state.convert_radio_to_queue("Move tracks (from radio)");
                state.queue_undo_snapshot = Some(snapshot);
            }
            // Move all selected tracks up by one position (process from top to bottom)
            let selected: Vec<usize> = state.queue_selected.iter().copied().collect();
            if selected.is_empty() || selected[0] == 0 { return Ok(vec![]); }
            let mut new_selected = std::collections::BTreeSet::new();
            for &idx in &selected {
                if idx > 0 && idx < state.queue.len() {
                    state.queue.swap(idx, idx - 1);
                    new_selected.insert(idx - 1);
                    // Adjust queue_index if current track was moved
                    if let Some(qi) = state.queue_index {
                        if qi == idx { state.queue_index = Some(idx - 1); }
                        else if qi == idx - 1 { state.queue_index = Some(idx); }
                    }
                } else {
                    new_selected.insert(idx);
                }
            }
            state.queue_selected = new_selected;
            if state.list_state.queue_index > 0 {
                state.list_state.queue_index -= 1;
            }
        }
        Action::MoveSelectedTracksDown => {
            if state.playback_mode == PlaybackMode::Radio {
                let snapshot = state.convert_radio_to_queue("Move tracks (from radio)");
                state.queue_undo_snapshot = Some(snapshot);
            }
            // Move all selected tracks down by one position (process from bottom to top)
            let selected: Vec<usize> = state.queue_selected.iter().copied().rev().collect();
            if selected.is_empty() || *selected.first().unwrap() >= state.queue.len().saturating_sub(1) { return Ok(vec![]); }
            let mut new_selected = std::collections::BTreeSet::new();
            for &idx in &selected {
                if idx + 1 < state.queue.len() {
                    state.queue.swap(idx, idx + 1);
                    new_selected.insert(idx + 1);
                    if let Some(qi) = state.queue_index {
                        if qi == idx { state.queue_index = Some(idx + 1); }
                        else if qi == idx + 1 { state.queue_index = Some(idx); }
                    }
                } else {
                    new_selected.insert(idx);
                }
            }
            state.queue_selected = new_selected;
            let max = state.queue.len().saturating_sub(1);
            state.list_state.queue_index = (state.list_state.queue_index + 1).min(max);
        }
        Action::RemoveSelectedFromQueue => {
            if state.playback_mode == PlaybackMode::Radio {
                let snapshot = state.convert_radio_to_queue("Delete tracks (from radio)");
                state.queue_undo_snapshot = Some(snapshot);
            }
            // Remove all selected tracks (process from highest index down)
            let selected: Vec<usize> = state.queue_selected.iter().copied().rev().collect();
            for &idx in &selected {
                if idx < state.queue.len() {
                    state.queue.remove(idx);
                    if let Some(qi) = state.queue_index {
                        if idx < qi {
                            state.queue_index = Some(qi - 1);
                        } else if idx == qi && qi >= state.queue.len() {
                            state.queue_index = if state.queue.is_empty() { None } else { Some(state.queue.len() - 1) };
                        }
                    }
                }
            }
            state.queue_selected.clear();
            // Adjust visual index
            let max = state.queue.len().saturating_sub(1);
            if state.list_state.queue_index > max {
                state.list_state.queue_index = max;
            }
        }
        Action::MoveQueueTrackDown => {
            if state.playback_mode == PlaybackMode::Radio {
                let snapshot = state.convert_radio_to_queue("Move track (from radio)");
                state.queue_undo_snapshot = Some(snapshot);
            }
            let idx = state.list_state.queue_index;
            if idx + 1 >= state.queue.len() { return Ok(vec![]); }

            state.queue.swap(idx, idx + 1);
            state.list_state.queue_index += 1;

            // Adjust queue_index if current track was moved
            if let Some(qi) = state.queue_index {
                if qi == idx {
                    state.queue_index = Some(idx + 1);
                } else if qi == idx + 1 {
                    state.queue_index = Some(idx);
                }
            }
        }
        Action::RemixBatchReady(inserts) => {
            if inserts.is_empty() {
                state.set_status("Remix: no changes made".to_string());
                return Ok(vec![]);
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
                    let insert_at = (pos + 1).min(state.queue.len());
                    total_inserted += insert_tracks.len();
                    state.queue.splice(insert_at..insert_at, insert_tracks.iter().cloned());
                }
            }

            state.set_status(format!("Remix complete: {} tracks added", total_inserted));

            // Pre-cache upcoming tracks
            let upcoming = helpers::get_upcoming_tracks(state);
            crate::audio::cache::trigger_prefetch(&audio.track_cache, &upcoming, client);
        }
        Action::RemixDoppelgangerReady(replacements) => {
            if replacements.is_empty() {
                state.set_status("Remix: Doppelganger — no replacements found".to_string());
                return Ok(vec![]);
            }

            let count = replacements.len();

            // Replace tracks in reverse index order to preserve positions
            let mut sorted = replacements;
            sorted.sort_unstable_by(|a, b| b.0.cmp(&a.0));
            for (idx, track) in sorted {
                if idx < state.queue.len() {
                    state.queue[idx] = track;
                }
            }

            state.set_status(format!("Remix: Doppelganger — {} tracks replaced", count));

            // Stop currently playing track since it was replaced
            follow_ups.push(Action::Stop);

            // Pre-cache upcoming tracks
            let upcoming = helpers::get_upcoming_tracks(state);
            crate::audio::cache::trigger_prefetch(&audio.track_cache, &upcoming, client);
        }
        _ => unreachable!("dispatch_queue called with non-queue action: {:?}", action),
    }
    Ok(follow_ups)
}

// ---------------------------------------------------------------------------
// Remix batch processing
// ---------------------------------------------------------------------------

/// Maximum concurrent API requests for remix batch processing.
const REMIX_BATCH_CONCURRENCY: usize = 4;

/// Number of similar tracks to fetch for remix operations.
const REMIX_SIMILAR_FETCH_LIMIT: u32 = 50;

/// Maximum bridge tracks to insert between each pair (Remix Stretch).
const REMIX_STRETCH_BRIDGE_MAX: usize = 3;

/// Spawn a batch remix task based on the action type.
fn spawn_remix_batch(
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,
    client: &mut PlexClient,
    action: Action,
) {
    // Collect tracks to process: from current position to end
    // Doppelganger includes the currently playing track (replaces all from current)
    let base_idx = state.queue_index.unwrap_or(0).min(state.queue.len());
    let idx = base_idx;
    let tracks_with_indices: Vec<(usize, Track)> = state.queue[idx..]
        .iter()
        .enumerate()
        .map(|(i, t)| (idx + i, t.clone()))
        .collect();

    if tracks_with_indices.is_empty() {
        return;
    }

    let tx = event_tx.clone();
    let client_clone = client.clone();
    let library_key = state.active_library.clone();

    // For Doppelganger: collect artist aliases to exclude same artist and all their aliases
    let artist_aliases = state.artist_aliases.clone();

    tokio::spawn(async move {
        let mut inserts: Vec<(usize, Vec<Track>)> = Vec::new();
        let mut history: Vec<String> = Vec::new();

        match action {
            Action::RemixGemini => {
                remix_gemini_batch(&client_clone, tracks_with_indices, &mut history, &mut inserts).await;
                let _ = tx.send(Event::RemixBatchReady { inserts }).await;
            }
            Action::RemixTwofer => {
                remix_twofer_batch(&client_clone, tracks_with_indices, &mut history, &mut inserts).await;
                let _ = tx.send(Event::RemixBatchReady { inserts }).await;
            }
            Action::RemixStretch => {
                remix_stretch_batch(&client_clone, tracks_with_indices, &mut history, &mut inserts, library_key.as_deref()).await;
                let _ = tx.send(Event::RemixBatchReady { inserts }).await;
            }
            Action::RemixDoppelganger => {
                let replacements = remix_doppelganger_batch(&client_clone, tracks_with_indices, &artist_aliases).await;
                let _ = tx.send(Event::RemixDoppelgangerReady { replacements }).await;
            }
            _ => {
                let _ = tx.send(Event::RemixBatchReady { inserts }).await;
            }
        }
    });
}

/// Remix Gemini batch: insert the most sonically similar track after each queue track.
async fn remix_gemini_batch(
    client: &PlexClient,
    tracks_with_indices: Vec<(usize, Track)>,
    history: &mut Vec<String>,
    inserts: &mut Vec<(usize, Vec<Track>)>,
) {
    use futures::stream::{self, StreamExt};

    let mut used_artists: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut used_albums: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Seed with all queue track artists/albums for initial diversity
    for (_, t) in &tracks_with_indices {
        if let Some(a) = t.grandparent_rating_key.as_deref() {
            used_artists.insert(a.to_string());
        }
        if let Some(a) = t.parent_rating_key.as_deref() {
            used_albums.insert(a.to_string());
        }
    }

    // Fire all similar-track lookups concurrently
    let fetch_results: Vec<_> = stream::iter(tracks_with_indices.into_iter().map(|(idx, track)| {
        let client = client.clone();
        async move {
            let result = client.get_similar_tracks(&track.rating_key, REMIX_SIMILAR_FETCH_LIMIT).await;
            (idx, result)
        }
    }))
    .buffer_unordered(REMIX_BATCH_CONCURRENCY)
    .collect()
    .await;

    // Sort by original index to preserve queue order
    let mut fetch_results = fetch_results;
    fetch_results.sort_by_key(|(idx, _)| *idx);

    for (original_idx, result) in fetch_results {
        match result {
            Ok(similar) => {
                let picks = pick_diverse_remix(similar, 1, history, &used_artists, &used_albums);
                if let Some(pick) = picks.into_iter().next() {
                    if let Some(a) = pick.grandparent_rating_key.as_deref() {
                        used_artists.insert(a.to_string());
                    }
                    if let Some(a) = pick.parent_rating_key.as_deref() {
                        used_albums.insert(a.to_string());
                    }
                    history.push(pick.rating_key.clone());
                    inserts.push((original_idx, vec![pick]));
                }
            }
            Err(e) => {
                tracing::warn!("Remix Gemini: similar tracks failed for index {}: {}", original_idx, e);
            }
        }
    }
}

/// Remix Twofer batch: insert another track by the same artist after each queue track.
async fn remix_twofer_batch(
    client: &PlexClient,
    tracks_with_indices: Vec<(usize, Track)>,
    history: &mut Vec<String>,
    inserts: &mut Vec<(usize, Vec<Track>)>,
) {
    use futures::stream::{self, StreamExt};

    // Collect unique artist keys to fetch
    let unique_artists: std::collections::HashSet<String> = tracks_with_indices.iter()
        .filter_map(|(_, t)| t.grandparent_rating_key.clone())
        .collect();

    // Fetch all artist track lists in parallel
    let artist_results: Vec<_> = stream::iter(unique_artists.into_iter().map(|artist_key| {
        let client = client.clone();
        async move {
            let result = client.get_artist_all_tracks(&artist_key).await;
            (artist_key, result)
        }
    }))
    .buffer_unordered(REMIX_BATCH_CONCURRENCY)
    .collect()
    .await;

    let mut artist_cache: std::collections::HashMap<String, Vec<Track>> =
        std::collections::HashMap::new();
    for (key, result) in artist_results {
        match result {
            Ok(tracks) => { artist_cache.insert(key, tracks); }
            Err(e) => { tracing::warn!("Remix Twofer: artist tracks failed for {}: {}", key, e); }
        }
    }

    for (original_idx, track) in &tracks_with_indices {
        let artist_key = track.grandparent_rating_key.as_deref().unwrap_or("");
        if artist_key.is_empty() { continue; }

        let Some(artist_tracks) = artist_cache.get(artist_key) else { continue };

        let candidates: Vec<_> = artist_tracks.iter()
            .filter(|t| !history.contains(&t.rating_key) && t.rating_key != track.rating_key)
            .collect();

        if !candidates.is_empty() {
            use rand::prelude::IndexedRandom;
            let mut rng = rand::rng();
            if let Some(pick) = candidates.choose(&mut rng) {
                history.push(pick.rating_key.clone());
                inserts.push((*original_idx, vec![(*pick).clone()]));
            }
        }
    }
}

/// Remix Stretch batch: generate a mini sonic adventure bridge between each pair.
async fn remix_stretch_batch(
    client: &PlexClient,
    tracks_with_indices: Vec<(usize, Track)>,
    history: &mut Vec<String>,
    inserts: &mut Vec<(usize, Vec<Track>)>,
    section_id: Option<&str>,
) {
    use futures::stream::{self, StreamExt};

    let pairs: Vec<_> = tracks_with_indices.windows(2)
        .map(|w| (w[0].0, w[0].1.clone(), w[1].1.clone()))
        .collect();

    let adventure_results: Vec<_> = stream::iter(
        pairs.into_iter().map(|(original_idx, track, next_track)| {
            let client = client.clone();
            async move {
                let result = crate::services::generate_adventure_for_library(&client, &track, &next_track, 5, section_id).await;
                (original_idx, track.rating_key.clone(), next_track.rating_key.clone(), result)
            }
        })
    )
    .buffer_unordered(REMIX_BATCH_CONCURRENCY)
    .collect()
    .await;

    let mut adventure_results = adventure_results;
    adventure_results.sort_by_key(|(idx, _, _, _)| *idx);

    for (original_idx, track_key, next_key, result) in adventure_results {
        match result {
            Ok(bridge) => {
                let bridge_tracks: Vec<_> = bridge.into_iter()
                    .filter(|t| {
                        t.rating_key != track_key
                            && t.rating_key != next_key
                            && !history.contains(&t.rating_key)
                    })
                    .take(REMIX_STRETCH_BRIDGE_MAX)
                    .collect();
                if !bridge_tracks.is_empty() {
                    for t in &bridge_tracks {
                        history.push(t.rating_key.clone());
                    }
                    inserts.push((original_idx, bridge_tracks));
                }
            }
            Err(e) => {
                tracing::warn!("Remix Stretch: adventure failed: {}", e);
            }
        }
    }
}

/// Pick up to `count` diverse tracks from candidates for remix operations.
fn pick_diverse_remix(
    candidates: Vec<Track>,
    count: usize,
    history: &[String],
    used_artists: &std::collections::HashSet<String>,
    used_albums: &std::collections::HashSet<String>,
) -> Vec<Track> {
    let mut result = Vec::with_capacity(count);
    let mut picked_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut picked_artists: std::collections::HashSet<String> = used_artists.clone();
    let mut picked_albums: std::collections::HashSet<String> = used_albums.clone();

    // Phase 1: Diverse — different track, different artist, different album
    for t in &candidates {
        if result.len() >= count { break; }
        if history.contains(&t.rating_key) || picked_keys.contains(&t.rating_key) { continue; }
        let artist = t.grandparent_rating_key.as_deref().unwrap_or("");
        let album = t.parent_rating_key.as_deref().unwrap_or("");
        if !artist.is_empty() && picked_artists.contains(artist) { continue; }
        if !album.is_empty() && picked_albums.contains(album) { continue; }
        picked_keys.insert(t.rating_key.clone());
        if !artist.is_empty() { picked_artists.insert(artist.to_string()); }
        if !album.is_empty() { picked_albums.insert(album.to_string()); }
        result.push(t.clone());
    }

    // Phase 2: Relax — just avoid history and already-picked
    if result.len() < count {
        for t in &candidates {
            if result.len() >= count { break; }
            if history.contains(&t.rating_key) || picked_keys.contains(&t.rating_key) { continue; }
            picked_keys.insert(t.rating_key.clone());
            result.push(t.clone());
        }
    }

    // Phase 3: Last resort — any candidate not yet picked
    if result.len() < count {
        for t in &candidates {
            if result.len() >= count { break; }
            if picked_keys.contains(&t.rating_key) { continue; }
            picked_keys.insert(t.rating_key.clone());
            result.push(t.clone());
        }
    }

    result
}

/// Remix Doppelganger: replace each track with the most sonically similar track by a different artist.
/// Excludes tracks by the same album artist (grandparent_rating_key) AND same track artist name.
/// For compilation tracks, the track artist (original_title) is checked separately from album artist.
async fn remix_doppelganger_batch(
    client: &PlexClient,
    tracks_with_indices: Vec<(usize, Track)>,
    artist_aliases: &std::collections::HashMap<String, std::collections::HashSet<String>>,
) -> Vec<(usize, Track)> {
    use futures::stream::{self, StreamExt};

    // Build a map of original track artists for filtering (before consuming the vec)
    let original_artists: std::collections::HashMap<usize, String> = tracks_with_indices.iter()
        .filter_map(|(idx, t)| {
            t.grandparent_rating_key.as_ref().map(|a| (*idx, a.clone()))
        })
        .collect();

    // Collect all unique album artist keys in the queue
    let queue_artist_keys: std::collections::HashSet<String> = original_artists.values().cloned().collect();

    // Collect all unique track artist NAMES (lowercase for case-insensitive matching)
    // This catches compilation tracks where original_title differs from album artist
    let queue_artist_names: std::collections::HashSet<String> = tracks_with_indices.iter()
        .map(|(_, t)| t.track_artist().to_lowercase())
        .collect();

    // Build set of ALL artist KEYS to exclude (queue artists + all their aliases)
    let mut excluded_artist_keys: std::collections::HashSet<String> = queue_artist_keys.clone();
    for artist_key in &queue_artist_keys {
        // Add aliases of this artist
        if let Some(aliases) = artist_aliases.get(artist_key) {
            excluded_artist_keys.extend(aliases.iter().cloned());
        }
        // Also check if this artist IS an alias of someone else
        for (main_artist, aliases) in artist_aliases {
            if aliases.contains(artist_key) {
                excluded_artist_keys.insert(main_artist.clone());
                excluded_artist_keys.extend(aliases.iter().cloned());
            }
        }
    }

    // Collect original track keys to avoid picking them
    let mut used_keys: std::collections::HashSet<String> = tracks_with_indices.iter()
        .map(|(_, t)| t.rating_key.clone())
        .collect();

    // Fire all similar-track lookups concurrently
    let fetch_results: Vec<_> = stream::iter(tracks_with_indices.into_iter().map(|(idx, track)| {
        let client = client.clone();
        async move {
            let result = client.get_similar_tracks(&track.rating_key, REMIX_SIMILAR_FETCH_LIMIT).await;
            (idx, result)
        }
    }))
    .buffer_unordered(REMIX_BATCH_CONCURRENCY)
    .collect()
    .await;

    // Sort by original index to preserve order
    let mut fetch_results = fetch_results;
    fetch_results.sort_by_key(|(idx, _)| *idx);

    let mut replacements: Vec<(usize, Track)> = Vec::new();

    // Helper to check if a candidate track should be excluded
    let is_excluded = |t: &Track| -> bool {
        // Check album artist key
        let album_artist_key = t.grandparent_rating_key.as_deref().unwrap_or("");
        if !album_artist_key.is_empty() && excluded_artist_keys.contains(album_artist_key) {
            return true;
        }
        // Check track artist name (handles compilations)
        let track_artist_name = t.track_artist().to_lowercase();
        if queue_artist_names.contains(&track_artist_name) {
            return true;
        }
        false
    };

    for (original_idx, result) in fetch_results {
        match result {
            Ok(similar) => {
                // Phase 1: Pick the most similar track NOT by any excluded artist, not already used
                let pick = similar.iter().find(|t| {
                    !used_keys.contains(&t.rating_key) && !is_excluded(t)
                });

                // Phase 2: Relax — any track not by excluded artist (allow reuse)
                let pick = pick.or_else(|| {
                    similar.iter().find(|t| !is_excluded(t))
                });

                if let Some(pick) = pick {
                    used_keys.insert(pick.rating_key.clone());
                    replacements.push((original_idx, pick.clone()));
                }
            }
            Err(e) => {
                tracing::warn!("Remix Doppelganger: similar tracks failed for index {}: {}", original_idx, e);
            }
        }
    }

    // Post-processing: enforce consecutive-artist diversity (max 2 in a row)
    // Use track_artist() for consistency with compilation handling
    replacements.sort_by_key(|(idx, _)| *idx);
    if replacements.len() > 2 {
        for i in 2..replacements.len() {
            let artist_i = replacements[i].1.track_artist().to_lowercase();
            let artist_prev = replacements[i - 1].1.track_artist().to_lowercase();
            let artist_prev2 = replacements[i - 2].1.track_artist().to_lowercase();

            if !artist_i.is_empty() && artist_i == artist_prev && artist_i == artist_prev2 {
                // Try to swap with next non-same-artist entry
                if let Some(swap_idx) = (i + 1..replacements.len()).find(|&j| {
                    replacements[j].1.track_artist().to_lowercase() != artist_i
                }) {
                    replacements.swap(i, swap_idx);
                    // Also swap the queue indices so the right tracks go to the right positions
                    let idx_i = replacements[i].0;
                    let idx_j = replacements[swap_idx].0;
                    replacements[i].0 = idx_j;
                    replacements[swap_idx].0 = idx_i;
                }
            }
        }
    }

    replacements
}

/// Enqueue the currently selected search result + following items to END of queue.
fn enqueue_search_result(state: &mut AppState) -> Vec<Action> {
    use crate::app::state::SearchTab;

    let Some(ref results) = state.search_results else { return vec![] };
    let idx = state.list_state.search_item_index;

    let (section, local_idx) = if state.search_tab == SearchTab::Global {
        super::dispatch_search::resolve_global_index(results, idx)
    } else {
        (state.search_tab, idx)
    };

    match section {
        SearchTab::Artists => {
            if let Some(artist) = results.artists.get(local_idx) {
                return vec![Action::EnqueueArtistTracks {
                    artist_key: artist.rating_key.clone(),
                    artist_name: artist.title.clone(),
                }];
            }
        }
        SearchTab::Albums => {
            if let Some(album) = results.albums.get(local_idx) {
                return vec![Action::EnqueueAlbum {
                    rating_key: album.rating_key.clone(),
                    title: album.title.clone(),
                }];
            }
        }
        SearchTab::Tracks => {
            // Get selected track + all following tracks, add directly to queue
            let tracks: Vec<Track> = results.tracks[local_idx..].to_vec();
            if !tracks.is_empty() {
                // If radio is playing, convert to queue mode
                if state.playback_mode == PlaybackMode::Radio {
                    state.queue = state.radio.tracks.clone();
                    state.queue_index = state.radio.track_index;
                    state.playback_mode = PlaybackMode::Queue;
                    state.radio.clear();
                    if let Some(idx) = state.queue_index {
                        state.list_state.queue_index = idx;
                    }
                }
                state.queue_original.clear();
                state.queue_sort_mode = QueueSortMode::QueueOrder;
                let added = tracks.len();
                state.queue.extend(tracks);
                state.set_status(format!("Added {} to queue ({} total)", added, state.queue.len()));
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

    let Some(ref results) = state.search_results else { return vec![] };
    let idx = state.list_state.search_item_index;

    let (section, local_idx) = if state.search_tab == SearchTab::Global {
        super::dispatch_search::resolve_global_index(results, idx)
    } else {
        (state.search_tab, idx)
    };

    match section {
        SearchTab::Artists => {
            if let Some(artist) = results.artists.get(local_idx) {
                return vec![Action::PlayArtistTracks {
                    artist_key: artist.rating_key.clone(),
                }];
            }
        }
        SearchTab::Albums => {
            if let Some(album) = results.albums.get(local_idx) {
                return vec![Action::PlayAlbum {
                    rating_key: album.rating_key.clone(),
                }];
            }
        }
        SearchTab::Tracks => {
            if let Some(track) = results.tracks.get(local_idx) {
                return vec![Action::PlayTrack(track.clone())];
            }
        }
        SearchTab::Playlists | SearchTab::Genres | SearchTab::Global => {}
    }

    vec![]
}

/// Ctrl+E in search: add search result + following to TOP of queue and start playing.
fn enqueue_search_result_next(state: &AppState) -> Vec<Action> {
    use crate::app::state::SearchTab;

    let Some(ref results) = state.search_results else { return vec![] };
    let idx = state.list_state.search_item_index;

    let (section, local_idx) = if state.search_tab == SearchTab::Global {
        super::dispatch_search::resolve_global_index(results, idx)
    } else {
        (state.search_tab, idx)
    };

    match section {
        SearchTab::Artists => {
            if let Some(artist) = results.artists.get(local_idx) {
                return vec![Action::EnqueueArtistTracksNext {
                    artist_key: artist.rating_key.clone(),
                    artist_name: artist.title.clone(),
                }];
            }
        }
        SearchTab::Albums => {
            if let Some(album) = results.albums.get(local_idx) {
                return vec![Action::EnqueueAlbumNext {
                    rating_key: album.rating_key.clone(),
                    title: album.title.clone(),
                }];
            }
        }
        SearchTab::Tracks => {
            // Get selected track + all following tracks
            let tracks: Vec<Track> = results.tracks[local_idx..].to_vec();
            if !tracks.is_empty() {
                return vec![Action::EnqueueTracksNext(tracks)];
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

    fn make_track_with_artist(key: &str, title: &str, artist_key: &str, album_key: &str) -> Track {
        Track {
            rating_key: key.to_string(),
            title: title.to_string(),
            grandparent_rating_key: Some(artist_key.to_string()),
            parent_rating_key: Some(album_key.to_string()),
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
        state.queue = sample_queue();
        state.queue_index = Some(2); // playing Track 3

        // Simulate RemoveFromQueue(0) state mutation
        let idx = 0;
        state.queue.remove(idx);
        if let Some(current) = state.queue_index {
            if idx < current {
                state.queue_index = Some(current - 1);
            }
        }

        assert_eq!(state.queue_index, Some(1));
        assert_eq!(state.queue.len(), 3);
        assert_eq!(state.queue[1].rating_key, "t3"); // Track 3 is still current
    }

    #[test]
    fn remove_after_current_no_change() {
        let mut state = AppState::new();
        state.queue = sample_queue();
        state.queue_index = Some(1); // playing Track 2

        let idx = 3;
        state.queue.remove(idx);
        if let Some(current) = state.queue_index {
            if idx < current {
                state.queue_index = Some(current - 1);
            }
        }

        assert_eq!(state.queue_index, Some(1));
        assert_eq!(state.queue.len(), 3);
    }

    #[test]
    fn remove_current_at_end_wraps_back() {
        let mut state = AppState::new();
        state.queue = sample_queue();
        state.queue_index = Some(3); // playing last track

        let idx = 3;
        state.queue.remove(idx);
        if let Some(current) = state.queue_index {
            if idx == current && current >= state.queue.len() {
                state.queue_index = if state.queue.is_empty() {
                    None
                } else {
                    Some(state.queue.len() - 1)
                };
            }
        }

        assert_eq!(state.queue_index, Some(2)); // wraps to new last
    }

    #[test]
    fn remove_last_element_gives_none() {
        let mut state = AppState::new();
        state.queue = vec![make_track("t1", "Track 1")];
        state.queue_index = Some(0);

        let idx = 0;
        state.queue.remove(idx);
        if let Some(current) = state.queue_index {
            if idx == current && current >= state.queue.len() {
                state.queue_index = if state.queue.is_empty() {
                    None
                } else {
                    Some(state.queue.len() - 1)
                };
            }
        }

        assert_eq!(state.queue_index, None);
        assert!(state.queue.is_empty());
    }

    // --- MoveQueueTrack state logic tests ---

    #[test]
    fn move_track_up_swaps_and_adjusts() {
        let mut state = AppState::new();
        state.queue = sample_queue();
        state.queue_index = Some(2);
        state.list_state.queue_index = 2;

        // Simulate MoveQueueTrackUp
        let idx = state.list_state.queue_index;
        state.queue.swap(idx, idx - 1);
        state.list_state.queue_index -= 1;
        if let Some(qi) = state.queue_index {
            if qi == idx {
                state.queue_index = Some(idx - 1);
            } else if qi == idx - 1 {
                state.queue_index = Some(idx);
            }
        }

        assert_eq!(state.list_state.queue_index, 1);
        assert_eq!(state.queue_index, Some(1)); // current track moved up
        assert_eq!(state.queue[1].rating_key, "t3"); // t3 moved from 2 to 1
        assert_eq!(state.queue[2].rating_key, "t2"); // t2 moved from 1 to 2
    }

    #[test]
    fn move_track_up_at_zero_is_noop() {
        let mut state = AppState::new();
        state.queue = sample_queue();
        state.list_state.queue_index = 0;

        // MoveQueueTrackUp should be no-op at idx 0
        let idx = state.list_state.queue_index;
        if idx > 0 {
            state.queue.swap(idx, idx - 1);
        }

        assert_eq!(state.queue[0].rating_key, "t1"); // unchanged
    }

    #[test]
    fn move_track_down_swaps() {
        let mut state = AppState::new();
        state.queue = sample_queue();
        state.queue_index = Some(1);
        state.list_state.queue_index = 1;

        // Simulate MoveQueueTrackDown
        let idx = state.list_state.queue_index;
        state.queue.swap(idx, idx + 1);
        state.list_state.queue_index += 1;
        if let Some(qi) = state.queue_index {
            if qi == idx {
                state.queue_index = Some(idx + 1);
            } else if qi == idx + 1 {
                state.queue_index = Some(idx);
            }
        }

        assert_eq!(state.list_state.queue_index, 2);
        assert_eq!(state.queue_index, Some(2)); // current track moved down
        assert_eq!(state.queue[1].rating_key, "t3");
        assert_eq!(state.queue[2].rating_key, "t2");
    }

    #[test]
    fn move_track_down_at_end_is_noop() {
        let mut state = AppState::new();
        state.queue = sample_queue();
        state.list_state.queue_index = 3; // last index

        let idx = state.list_state.queue_index;
        if idx + 1 < state.queue.len() {
            state.queue.swap(idx, idx + 1);
        }

        assert_eq!(state.queue[3].rating_key, "t4"); // unchanged
    }

    // --- pick_diverse_remix tests ---

    #[test]
    fn pick_diverse_avoids_artists_and_albums() {
        let used_artists: std::collections::HashSet<String> = ["artist1"].iter().map(|s| s.to_string()).collect();
        let used_albums: std::collections::HashSet<String> = ["album1"].iter().map(|s| s.to_string()).collect();

        let candidates = vec![
            make_track_with_artist("c1", "By artist1", "artist1", "album2"),
            make_track_with_artist("c2", "By artist2 album1", "artist2", "album1"),
            make_track_with_artist("c3", "By artist3", "artist3", "album3"), // this one passes
        ];

        let result = pick_diverse_remix(candidates, 1, &[], &used_artists, &used_albums);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].rating_key, "c3");
    }

    #[test]
    fn pick_diverse_relaxes_when_no_diverse_match() {
        let used_artists: std::collections::HashSet<String> = ["artist1", "artist2"].iter().map(|s| s.to_string()).collect();
        let used_albums = std::collections::HashSet::new();

        let candidates = vec![
            make_track_with_artist("c1", "By artist1", "artist1", "a1"),
            make_track_with_artist("c2", "By artist2", "artist2", "a2"),
            make_track_with_artist("c3", "By artist1 again", "artist1", "a3"),
        ];

        // Phase 1 finds nothing diverse, Phase 2 takes any non-history track
        let result = pick_diverse_remix(candidates, 1, &[], &used_artists, &used_albums);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].rating_key, "c1"); // first candidate in relaxed phase
    }

    #[test]
    fn pick_diverse_respects_history() {
        let used_artists = std::collections::HashSet::new();
        let used_albums = std::collections::HashSet::new();
        let history = vec!["c1".to_string()];

        let candidates = vec![
            make_track_with_artist("c1", "Already used", "a1", "al1"),
            make_track_with_artist("c2", "Fresh", "a2", "al2"),
        ];

        let result = pick_diverse_remix(candidates, 1, &history, &used_artists, &used_albums);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].rating_key, "c2");
    }

    #[test]
    fn pick_diverse_empty_candidates() {
        let result = pick_diverse_remix(
            vec![],
            3,
            &[],
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        );
        assert!(result.is_empty());
    }

    #[test]
    fn pick_diverse_multiple() {
        let candidates = vec![
            make_track_with_artist("c1", "T1", "a1", "al1"),
            make_track_with_artist("c2", "T2", "a2", "al2"),
            make_track_with_artist("c3", "T3", "a3", "al3"),
        ];

        let result = pick_diverse_remix(
            candidates,
            2,
            &[],
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        );
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].rating_key, "c1");
        assert_eq!(result[1].rating_key, "c2");
    }
}
