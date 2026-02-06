//! Radio dispatch handlers: StartTrackRadio, StartAlbumRadio, StartArtistRadio,
//! StopRadio, JumpToRadioTrack, FetchMoreRadioTracks, PlayStation, DrillIntoStation,
//! NavigateStationsBack.

use crate::app::{Action, AppState, Event};
use crate::app::state::{PlaybackMode, RadioMode, View};
use crate::api::PlexClient;
use crate::audio::AudioPlayer;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Dispatch radio and station actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: Action,
    state: &mut AppState,
    client: &mut PlexClient,
    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    match action {
        Action::StartTrackRadio { track_key, title } => {
            use rand::seq::SliceRandom;

            // Report stop for currently playing track before starting radio
            // continuing=true because we're starting new content
            if let Some(current) = state.current_track().cloned() {
                helpers::report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
            }

            // Generate new session ID for this playback context
            state.plex_session_id = Some(helpers::generate_plex_session_id());

            // Start track radio - fetch similar tracks, shuffle to avoid album clustering
            state.radio_state.mode = RadioMode::Track;
            state.radio_state.seed_track_key = Some(track_key.clone());
            state.radio_state.seed_title = title.clone();
            state.radio_state.history.clear();
            state.radio_state.fetching = true;
            state.view = View::NowPlaying;
            state.playback_mode = PlaybackMode::Radio;

            // Get the seed track first (to start playing immediately)
            let seed_track = if let Some(track) = state.selected_album_tracks.iter()
                .find(|t| t.rating_key == track_key)
                .cloned() {
                Some(track)
            } else if let Ok(tracks) = client.get_album_tracks(&track_key).await {
                tracks.into_iter().find(|t| t.rating_key == track_key)
            } else {
                None
            };

            // Clear queue and track cache, start with seed track if found
            audio.track_cache.flush();
            state.queue.clear();
            if let Some(track) = seed_track {
                state.queue.push(track);
                state.radio_state.history.push(track_key.clone());
            }
            state.queue_index = Some(0);

            // Start playback immediately
            if !state.queue.is_empty() {
                helpers::play_current_track(event_tx, state, client, audio).await;
            }

            // Fetch similar tracks
            match client.get_similar_tracks(&track_key, 50).await {
                Ok(mut tracks) => {
                    // Shuffle to break up album blocks and add diversity
                    let mut rng = rand::rng();
                    tracks.shuffle(&mut rng);

                    // Filter out seed track and duplicates
                    let new_tracks: Vec<_> = tracks.into_iter()
                        .filter(|t| !state.radio_state.history.contains(&t.rating_key))
                        .take(25)
                        .collect();

                    if !new_tracks.is_empty() {
                        // Add to history to avoid repeats
                        for track in &new_tracks {
                            state.radio_state.history.push(track.rating_key.clone());
                        }

                        // Extend queue with shuffled similar tracks
                        state.queue.extend(new_tracks.clone());

                        state.set_status(format!("{} radio: {} tracks", title, state.queue.len()));
                    } else if state.queue.is_empty() {
                        state.set_error("No similar tracks found".to_string());
                    }
                    state.radio_state.fetching = false;
                }
                Err(e) => {
                    state.set_error(format!("Failed to fetch similar tracks: {}", e));
                    state.radio_state.fetching = false;
                }
            }
        }
        Action::StartAlbumRadio { album_key, title } => {
            // Report stop for currently playing track before starting radio
            // continuing=true because we're starting new content
            if let Some(current) = state.current_track().cloned() {
                helpers::report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
            }

            // Generate new session ID for this playback context
            state.plex_session_id = Some(helpers::generate_plex_session_id());

            // Start album radio - play album then similar albums
            state.radio_state.mode = RadioMode::Album;
            state.radio_state.seed_track_key = Some(album_key.clone());
            state.radio_state.seed_title = title;
            state.radio_state.history.clear();
            state.radio_state.fetching = true;
            state.view = View::NowPlaying;

            // First, load the album's tracks
            match client.get_album_tracks(&album_key).await {
                Ok(tracks) => {
                    state.queue = tracks;
                    state.queue_index = Some(0);
                    helpers::play_current_track(event_tx, state, client, audio).await;
                }
                Err(e) => {
                    state.set_error(format!("Failed to load album tracks: {}", e));
                }
            }

            // Then fetch similar albums
            match client.get_similar_albums(&album_key, 10).await {
                Ok(albums) => {
                    for album in albums {
                        if let Ok(tracks) = client.get_album_tracks(&album.rating_key).await {
                            state.queue.extend(tracks);
                        }
                    }
                    state.radio_state.fetching = false;
                }
                Err(e) => {
                    tracing::warn!("Failed to fetch similar albums: {}", e);
                    state.radio_state.fetching = false;
                }
            }
        }
        Action::StartArtistRadio { artist_key, title } => {
            // Report stop for currently playing track before starting radio
            // continuing=true because we're starting new content
            if let Some(current) = state.current_track().cloned() {
                helpers::report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
            }

            // Generate new session ID for this playback context
            state.plex_session_id = Some(helpers::generate_plex_session_id());

            // Start artist radio - play artist's tracks then similar
            state.radio_state.mode = RadioMode::Artist;
            state.radio_state.seed_track_key = Some(artist_key.clone());
            state.radio_state.seed_title = title;
            state.radio_state.history.clear();
            state.radio_state.fetching = true;
            state.view = View::NowPlaying;

            // Load artist's tracks
            match client.get_artist_all_tracks(&artist_key).await {
                Ok(tracks) => {
                    state.queue = tracks;
                    state.queue_index = Some(0);
                    helpers::play_current_track(event_tx, state, client, audio).await;
                }
                Err(e) => {
                    state.set_error(format!("Failed to load artist tracks: {}", e));
                }
            }
            state.radio_state.fetching = false;
        }
        Action::StopRadio => {
            state.radio_state.mode = RadioMode::Off;
            state.radio_state.seed_track_key = None;
            state.radio_state.seed_title.clear();
            state.radio_state.fetching = false;
            state.radio_state.history.clear();
        }
        Action::JumpToRadioTrack(idx) => {
            // Report stop for current track before jumping
            // continuing=true because we're jumping to another track
            if let Some(track) = state.current_track().cloned() {
                helpers::report_playback_stop_to_plex(&track, state.playback.position_ms, true, state.plex_session_id.clone(), client);
            }

            // Jump to track in radio queue without clearing radio state
            if idx < state.radio.tracks.len() {
                state.radio.track_index = Some(idx);
                state.list_state.queue_index = idx;
                helpers::play_current_track(event_tx, state, client, audio).await;
            }
        }
        Action::FetchMoreRadioTracks => {
            use rand::seq::SliceRandom;

            // Only fetch if in track radio mode and not already fetching
            if state.radio_state.mode == RadioMode::Track && !state.radio_state.fetching {
                if let Some(ref seed_key) = state.radio_state.seed_track_key.clone() {
                    state.radio_state.fetching = true;

                    match client.get_similar_tracks(&seed_key, 30).await {
                        Ok(mut tracks) => {
                            // Shuffle to maintain diversity
                            let mut rng = rand::rng();
                            tracks.shuffle(&mut rng);

                            let new_tracks: Vec<_> = tracks.into_iter()
                                .filter(|t| !state.radio_state.history.contains(&t.rating_key))
                                .take(15)
                                .collect();

                            for track in &new_tracks {
                                state.radio_state.history.push(track.rating_key.clone());
                            }

                            state.queue.extend(new_tracks);
                            state.radio_state.fetching = false;
                        }
                        Err(e) => {
                            tracing::warn!("Failed to fetch more radio tracks: {}", e);
                            state.radio_state.fetching = false;
                        }
                    }
                }
            }
        }
        Action::PlayStation(station_key) => {
            // Report stop for currently playing track before starting station
            // continuing=true because we're starting new content
            if let Some(current) = state.current_track().cloned() {
                helpers::report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
            }

            // Generate new session ID for this playback context
            state.plex_session_id = Some(helpers::generate_plex_session_id());

            // Find station title from station_nav (Miller columns) or fall back to legacy state.stations
            let station_title = state.station_nav.selected_station()
                .filter(|s| s.key == station_key)
                .map(|s| s.title.clone())
                .or_else(|| {
                    // Search all columns
                    state.station_nav.columns.iter()
                        .flat_map(|col| col.stations.iter())
                        .find(|s| s.key == station_key)
                        .map(|s| s.title.clone())
                })
                .or_else(|| {
                    // Fall back to legacy state.stations
                    state.stations.iter()
                        .find(|s| s.key == station_key)
                        .map(|s| s.title.clone())
                })
                .unwrap_or_else(|| "Radio".to_string());

            // Set loading state immediately - UI stays responsive
            state.stations_loading = true;
            state.station_nav.loading = true;
            state.set_status(format!("Loading {}...", station_title));

            // Spawn background task for station queue creation (non-blocking)
            let tx = event_tx.clone();
            let client_clone = client.clone();
            let sk = station_key.clone();
            let st = station_title.clone();
            let lib_key = state.active_library.clone();
            tokio::spawn(async move {
                let queue_future = client_clone.create_station_queue(&sk);
                let timeout_duration = std::time::Duration::from_secs(30);

                match tokio::time::timeout(timeout_duration, queue_future).await {
                    Ok(Ok(tracks)) => {
                        // For Time Travel Radio: fetch decades in same background task
                        let time_travel_decades = if sk.contains("timeTravel") {
                            if let Some(ref lk) = lib_key {
                                client_clone.get_time_travel_decades(lk).await.unwrap_or_default()
                            } else {
                                vec![]
                            }
                        } else {
                            vec![]
                        };

                        let _ = tx.send(Event::StationTracksLoaded {
                            station_key: sk,
                            station_title: st,
                            tracks,
                            time_travel_decades,
                        }).await;
                    }
                    Ok(Err(e)) => {
                        let _ = tx.send(Event::StationLoadFailed {
                            station_key: sk,
                            error: format!("Failed to start station: {}", e),
                        }).await;
                    }
                    Err(_) => {
                        tracing::warn!("Station queue creation timed out after 30 seconds: {}", sk);
                        let _ = tx.send(Event::StationLoadFailed {
                            station_key: sk,
                            error: "Station timed out - try a different station".into(),
                        }).await;
                    }
                }
            });
        }
        Action::DrillIntoStation(station_key, station_title) => {
            // Drill into a station category (e.g., Mood Radio -> sub-moods)
            state.stations_loading = true;
            state.station_nav.loading = true;
            state.set_status(format!("Loading {}...", station_title));

            // Spawn background task for child loading (non-blocking)
            let tx = event_tx.clone();
            let client_clone = client.clone();
            let sk = station_key.clone();
            let st = station_title.clone();
            tokio::spawn(async move {
                match client_clone.get_station_children(&sk).await {
                    Ok(children) => {
                        if children.is_empty() {
                            // No children - treat as playable station
                            let queue_future = client_clone.create_station_queue(&sk);
                            let timeout_duration = std::time::Duration::from_secs(30);

                            match tokio::time::timeout(timeout_duration, queue_future).await {
                                Ok(Ok(tracks)) => {
                                    let _ = tx.send(Event::StationTracksLoaded {
                                        station_key: sk,
                                        station_title: st,
                                        tracks,
                                        time_travel_decades: vec![],
                                    }).await;
                                }
                                Ok(Err(e)) => {
                                    let _ = tx.send(Event::StationLoadFailed {
                                        station_key: sk,
                                        error: format!("Failed to start station: {}", e),
                                    }).await;
                                }
                                Err(_) => {
                                    tracing::warn!("Station queue creation timed out after 30 seconds: {}", sk);
                                    let _ = tx.send(Event::StationLoadFailed {
                                        station_key: sk,
                                        error: "Station timed out - try a different station".into(),
                                    }).await;
                                }
                            }
                        } else {
                            let _ = tx.send(Event::StationChildrenLoaded {
                                station_key: sk,
                                station_title: st,
                                children,
                            }).await;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Event::StationLoadFailed {
                            station_key: sk,
                            error: format!("Failed to load station children: {}", e),
                        }).await;
                    }
                }
            });
        }
        Action::NavigateStationsBack => {
            // Go back in Miller columns (just move focus left - data already in memory)
            if state.station_nav.can_go_left() {
                state.station_nav.focus_left();
                // Update legacy state to match focused column
                if let Some(col) = state.station_nav.focused() {
                    state.stations = col.stations.clone();
                    state.stations_index = col.selected_index;
                }
            }
        }
        Action::PlayCurrentRadioTrack => {
            // Play the current track in radio mode (stays in Radio playback mode)
            state.consecutive_playback_errors = 0;
            helpers::play_current_track(event_tx, state, client, audio).await;
        }
        _ => unreachable!("dispatch_radio called with non-radio action: {:?}", action),
    }
    Ok(vec![])
}
