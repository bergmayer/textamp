//! Queue dispatch handlers: PlayTrack, PlayTrackFromCategory, PlayAlbum, EnqueueAlbum,
//! ClearQueue, RemoveFromQueue, JumpToQueueIndex,
//! EnqueueSelection, PromptSavePlaylist, SaveQueueAsPlaylist.

use crate::app::{Action, AppState, Event};
use crate::app::state::{BrowseCategory, BrowseItem, PlayStatus, PlaybackMode, QueueSortMode, RightPanelMode, SimilarMode, View};
use crate::api::PlexClient;
use crate::api::models::Track;
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
                // continuing=true because we're switching to another track
                if let Some(current) = state.current_track().cloned() {
                    helpers::report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
                }

                // Generate new session ID for this playback context
                state.plex_session_id = Some(helpers::generate_plex_session_id());

                // Clear radio state if switching from radio mode
                if state.playback_mode == PlaybackMode::Radio {
                    state.radio.clear();
                }
                // Queue all tracks from current position
                audio.track_cache.flush();
                state.queue = state.selected_album_tracks[idx..].to_vec();
                state.queue_index = Some(0);
                state.queue_original.clear();
                state.queue_sort_mode = QueueSortMode::QueueOrder;
                state.playback_mode = PlaybackMode::Queue;
                helpers::play_current_track(event_tx, state, client, audio).await;
            }
        }
        Action::PlayAlbum { rating_key } => {
            // Load album tracks and play them
            match client.get_album_tracks(&rating_key).await {
                Ok(tracks) => {
                    if !tracks.is_empty() {
                        // Report stop for currently playing track before switching
                        // continuing=true because we're switching to another album
                        if let Some(current) = state.current_track().cloned() {
                            helpers::report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
                        }

                        // Generate new session ID for this playback context
                        state.plex_session_id = Some(helpers::generate_plex_session_id());

                        // Clear radio state if switching from radio mode
                        if state.playback_mode == PlaybackMode::Radio {
                            state.radio.clear();
                        }
                        audio.track_cache.flush();
                        state.queue = tracks;
                        state.queue_index = Some(0);
                        state.queue_original.clear();
                        state.queue_sort_mode = QueueSortMode::QueueOrder;
                        state.playback_mode = PlaybackMode::Queue;
                        helpers::play_current_track(event_tx, state, client, audio).await;
                    }
                }
                Err(e) => {
                    state.set_error(format!("Failed to load album: {}", e));
                }
            }
        }
        Action::EnqueueAlbum { rating_key, title } => {
            // Load album tracks and add to queue
            match client.get_album_tracks(&rating_key).await {
                Ok(tracks) => {
                    if !tracks.is_empty() {
                        // If radio is playing, convert to queue mode first
                        if state.playback_mode == PlaybackMode::Radio {
                            state.queue = state.radio.tracks.clone();
                            state.queue_index = state.radio.track_index;
                            state.playback_mode = PlaybackMode::Queue;
                            state.radio.clear();
                        }

                        let added = tracks.len();
                        state.queue.extend(tracks);
                        state.set_status(format!("Added {} tracks from \"{}\" to queue", added, title));
                    }
                }
                Err(e) => {
                    state.set_error(format!("Failed to load album: {}", e));
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
                        state.list_state.queue_index = state.play_history.len();
                    }
                    QueueSortMode::Shuffle => {
                        let current_key = state.current_track().map(|t| t.rating_key.clone());
                        state.queue = std::mem::take(&mut state.queue_original);
                        state.queue_sort_mode = QueueSortMode::QueueOrder;
                        if let Some(key) = current_key {
                            state.queue_index = state.queue.iter().position(|t| t.rating_key == key);
                        }
                        if let Some(idx) = state.queue_index {
                            state.list_state.queue_index = state.play_history.len() + idx;
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
                if state.list_state.queue_index >= state.queue.len() && !state.queue.is_empty() {
                    state.list_state.queue_index = state.queue.len() - 1;
                }
            }
        }
        Action::JumpToQueueIndex(idx) => {
            // Jump to and play a specific track in the queue
            if idx < state.queue.len() {
                state.queue_index = Some(idx);
                state.list_state.queue_index = state.play_history.len() + idx;
                if let Some(track) = state.queue.get(idx).cloned() {
                    follow_ups.push(Action::PlayTrack(track));
                }
                // Trigger DJ mode processing after jump (all modes are continuous)
                if !state.dj_inserting && state.active_dj_mode.is_some() {
                    follow_ups.push(Action::DjModeProcess);
                }
            }
        }
        Action::EnqueueSelection => {
            // Check if we should enqueue an album instead of tracks
            let album_to_enqueue: Option<(String, String)> = match state.view {
                View::Browse => {
                    // Check Miller columns first — selected album in any browse category
                    let miller_album = {
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
                    };
                    if miller_album.is_some() {
                        miller_album
                    } else {
                    match state.focus {
                        crate::app::state::Focus::Left => {
                            match state.browse_category {
                                _ => None,
                            }
                        }
                        crate::app::state::Focus::Right => {
                            // Check if we're viewing albums (not tracks)
                            match state.right_panel_mode {
                                RightPanelMode::ArtistAlbums => {
                                    // Artist's albums - enqueue selected album
                                    // Note: right_albums_index 0 is "All tracks", so actual albums start at 1
                                    if state.list_state.right_albums_index > 0 {
                                        let album_idx = state.list_state.right_albums_index - 1;
                                        state.selected_artist_albums.get(album_idx)
                                            .map(|a| (a.rating_key.clone(), a.title.clone()))
                                    } else {
                                        None
                                    }
                                }
                                RightPanelMode::CategoryAlbums => {
                                    // Genre/mood albums - enqueue selected album
                                    state.genre_albums.get(state.genre_albums_index)
                                        .map(|a| (a.rating_key.clone(), a.title.clone()))
                                }
                                _ => None,
                            }
                        }
                    }
                    } // else (no Miller album)
                }
                View::Similar => {
                    match state.similar_mode {
                        SimilarMode::Albums => {
                            // Similar albums - enqueue selected album
                            state.similar_albums.get(state.list_state.similar_index)
                                .map(|a| (a.rating_key.clone(), a.title.clone()))
                        }
                        _ => None,
                    }
                }
                _ => None,
            };

            // If we found an album to enqueue, do that instead
            if let Some((rating_key, title)) = album_to_enqueue {
                return Ok(vec![Action::EnqueueAlbum { rating_key, title }]);
            }

            // Otherwise, try to enqueue individual tracks
            let tracks_to_add: Vec<Track> = match state.view {
                View::Browse => {
                    match state.focus {
                        crate::app::state::Focus::Right => {
                            // Enqueue selected track
                            if !state.selected_album_tracks.is_empty() {
                                vec![state.selected_album_tracks[state.list_state.tracks_index].clone()]
                            } else {
                                vec![]
                            }
                        }
                        crate::app::state::Focus::Left => {
                            // Left panel with no album selected - nothing to enqueue
                            vec![]
                        }
                    }
                }
                View::Similar => {
                    match state.similar_mode {
                        SimilarMode::Tracks => {
                            if let Some(track) = state.similar_tracks.get(state.list_state.similar_index) {
                                vec![track.clone()]
                            } else {
                                vec![]
                            }
                        }
                        _ => vec![],
                    }
                }
                View::NowPlaying => {
                    // Already in queue view - can't enqueue from here
                    vec![]
                }
                _ => vec![],
            };

            if !tracks_to_add.is_empty() {
                // If radio is playing, convert to queue mode
                if state.playback_mode == PlaybackMode::Radio {
                    // Convert current radio tracks to queue
                    state.queue = state.radio.tracks.clone();
                    state.queue_index = state.radio.track_index;
                    state.playback_mode = PlaybackMode::Queue;
                    state.radio.clear();
                }

                state.queue.extend(tracks_to_add);
                state.set_status(format!("Added to queue ({} tracks)", state.queue.len()));
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
                state.input_dialog = Some(crate::app::state::InputDialog {
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

            if tracks.is_empty() {
                state.set_error("No tracks to save".to_string());
            } else if name.trim().is_empty() {
                state.set_error("Playlist name cannot be empty".to_string());
            } else if let Some(ref library_key) = state.active_library {
                let track_keys: Vec<String> = tracks.iter()
                    .map(|t| t.rating_key.clone())
                    .collect();
                let track_count = track_keys.len();
                let name_clone = name.clone();
                let library_key_clone = library_key.clone();

                state.set_status(format!("Saving playlist \"{}\"...", name));

                match client.create_playlist(&name_clone, &track_keys, &library_key_clone).await {
                    Ok(()) => {
                        state.set_status(format!("Saved \"{}\" ({} tracks)", name_clone, track_count));
                        // Refresh playlists so the new one appears
                        state.playlists_loading = true;
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to save playlist: {}", e));
                    }
                }
            } else {
                state.set_error("No library selected".to_string());
            }
        }
        Action::RemixGemini | Action::RemixTwofer | Action::RemixStretch => {
            if state.playback_mode != PlaybackMode::Queue && state.playback_mode != PlaybackMode::None {
                state.set_error("Remix only works in queue mode".to_string());
                return Ok(vec![]);
            }
            if state.queue.len() < 2 {
                state.set_error("Need at least 2 tracks to remix".to_string());
                return Ok(vec![]);
            }

            // Save snapshot for undo
            state.queue_undo_snapshot = Some(crate::app::state::QueueSnapshot {
                queue: state.queue.clone(),
                queue_index: state.queue_index,
                description: match action {
                    Action::RemixGemini => "Remix: Gemini".to_string(),
                    Action::RemixTwofer => "Remix: Twofer".to_string(),
                    Action::RemixStretch => "Remix: Stretch".to_string(),
                    _ => "Remix".to_string(),
                },
            });

            let mode_name = match action {
                Action::RemixGemini => "Gemini",
                Action::RemixTwofer => "Twofer",
                Action::RemixStretch => "Stretch",
                _ => "Remix",
            };
            state.set_status(format!("Remix: {} processing...", mode_name));

            spawn_remix_batch(event_tx, state, client, action);
        }
        Action::RemixShuffle => {
            if state.playback_mode != PlaybackMode::Queue && state.playback_mode != PlaybackMode::None {
                state.set_error("Remix only works in queue mode".to_string());
                return Ok(vec![]);
            }

            use crate::services::PlaybackService;

            // Save snapshot for Ctrl+Z undo
            state.queue_undo_snapshot = Some(crate::app::state::QueueSnapshot {
                queue: state.queue.clone(),
                queue_index: state.queue_index,
                description: "Remix: Shuffle".to_string(),
            });

            // Save shuffle-specific undo state (for toggle)
            state.shuffle_undo_queue = Some(state.queue.clone());
            state.shuffle_undo_index = state.queue_index;

            let (shuffled, new_idx) = PlaybackService::shuffle_queue(
                state.queue.clone(), state.queue_index,
            );
            state.queue = shuffled;
            state.queue_index = new_idx;
            state.list_state.queue_index = state.play_history.len();
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
                    state.list_state.queue_index = state.play_history.len() + idx;
                }
                state.set_status("Shuffle undone".to_string());
            } else {
                state.set_error("No shuffle to undo".to_string());
            }
        }
        Action::UndoLastRemix => {
            if let Some(snapshot) = state.queue_undo_snapshot.take() {
                state.queue = snapshot.queue;
                state.queue_index = snapshot.queue_index;
                state.shuffle_undo_queue = None;
                state.shuffle_undo_index = None;
                if let Some(idx) = state.queue_index {
                    state.list_state.queue_index = state.play_history.len() + idx;
                }
                state.set_status(format!("Undid {}", snapshot.description));
            } else {
                state.set_error("Nothing to undo".to_string());
            }
        }
        Action::MoveQueueTrackUp => {
            if state.playback_mode != PlaybackMode::Queue && state.playback_mode != PlaybackMode::None {
                return Ok(vec![]);
            }
            let visual = state.list_state.queue_index;
            let history_len = state.play_history.len();
            if visual < history_len { return Ok(vec![]); }
            let idx = visual - history_len;
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
        Action::MoveQueueTrackDown => {
            if state.playback_mode != PlaybackMode::Queue && state.playback_mode != PlaybackMode::None {
                return Ok(vec![]);
            }
            let visual = state.list_state.queue_index;
            let history_len = state.play_history.len();
            if visual < history_len { return Ok(vec![]); }
            let idx = visual - history_len;
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
    let idx = state.queue_index.unwrap_or(0).min(state.queue.len());
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

    tokio::spawn(async move {
        let mut inserts: Vec<(usize, Vec<Track>)> = Vec::new();
        let mut history: Vec<String> = Vec::new();

        match action {
            Action::RemixGemini => {
                remix_gemini_batch(&client_clone, tracks_with_indices, &mut history, &mut inserts).await;
            }
            Action::RemixTwofer => {
                remix_twofer_batch(&client_clone, tracks_with_indices, &mut history, &mut inserts).await;
            }
            Action::RemixStretch => {
                remix_stretch_batch(&client_clone, tracks_with_indices, &mut history, &mut inserts, library_key.as_deref()).await;
            }
            _ => {}
        }

        let _ = tx.send(Event::RemixBatchReady { inserts }).await;
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
