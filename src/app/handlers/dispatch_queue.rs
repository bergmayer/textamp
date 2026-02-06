//! Queue dispatch handlers: PlayTrack, PlayTrackFromCategory, PlayAlbum, EnqueueAlbum,
//! ClearQueue, RemoveFromQueue, JumpToQueueIndex, PlayRecentlyPlayedAlbum,
//! EnqueueSelection, PromptSavePlaylist, SaveQueueAsPlaylist.

use crate::app::{Action, AppState, Event};
use crate::app::state::{PlayStatus, PlaybackMode, QueueSortMode, RightPanelMode, SimilarMode, View};
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

                        // Add tracks to queue, respecting 500 track limit
                        const MAX_QUEUE_SIZE: usize = 500;
                        let mut added = 0;
                        for track in tracks {
                            if state.queue.len() < MAX_QUEUE_SIZE {
                                state.queue.push(track);
                                added += 1;
                            }
                        }
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
                }
            }
            audio.stop();
            state.playback.status = PlayStatus::Stopped;
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
            }
        }
        Action::PlayRecentlyPlayedAlbum(idx) => {
            // Play album from recently played list
            if let Some(album) = state.recently_played_albums.get(idx).cloned() {
                let rating_key = album.rating_key.clone();
                follow_ups.push(Action::PlayAlbum { rating_key });
            }
        }
        Action::EnqueueSelection => {
            // Check if we should enqueue an album instead of tracks
            let album_to_enqueue: Option<(String, String)> = match state.view {
                View::Browse => {
                    match state.focus {
                        crate::app::state::Focus::Left => {
                            // Check if we're in albums mode or have albums selected
                            match state.browse_category {
                                crate::app::state::BrowseCategory::Artists if state.artist_view_mode == crate::app::state::ArtistViewMode::Album => {
                                    // Albums view on left - enqueue selected album
                                    state.albums.get(state.list_state.albums_index)
                                        .map(|a| (a.rating_key.clone(), a.title.clone()))
                                }
                                crate::app::state::BrowseCategory::Playlists if state.playlists_mode == crate::app::state::PlaylistsMode::RecentlyAdded => {
                                    // Recently added albums - enqueue selected album
                                    state.recently_added_albums.get(state.list_state.playlists_index)
                                        .map(|a| (a.rating_key.clone(), a.title.clone()))
                                }
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

                // Add tracks to queue, respecting 500 track limit
                const MAX_QUEUE_SIZE: usize = 500;
                for track in tracks_to_add {
                    if state.queue.len() < MAX_QUEUE_SIZE {
                        state.queue.push(track);
                    }
                }
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
        _ => unreachable!("dispatch_queue called with non-queue action: {:?}", action),
    }
    Ok(follow_ups)
}
