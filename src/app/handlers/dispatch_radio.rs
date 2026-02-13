//! Radio dispatch handlers: StopRadio, JumpToRadioTrack, StartPlexRadio,
//! PlayStation, DrillIntoStation, NavigateStationsBack.

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
        Action::StartPlexRadio { key, title } => {
            // Start radio using Plex playQueue API (full heuristics)
            if let Some(current) = state.current_track().cloned() {
                helpers::report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
            }

            audio.stop();
            state.playback.status = crate::app::state::PlayStatus::Stopped;
            state.plex_session_id = Some(helpers::generate_plex_session_id());

            state.radio_state.mode = RadioMode::Active; // Plex decides the actual mix
            state.radio_state.seed_track_key = Some(key.clone());
            state.radio_state.seed_title = title.clone();
            state.radio_state.history.clear();
            state.radio_state.fetching = true;
            state.view = View::NowPlaying;
            state.playback_mode = PlaybackMode::Radio;
            state.set_status(format!("Starting radio: {}...", title));

            let tx = event_tx.clone();
            let mut client_clone = client.clone();
            let rk = key.clone();
            let rt = title.clone();
            tokio::spawn(async move {
                let timeout_duration = std::time::Duration::from_secs(30);
                match tokio::time::timeout(timeout_duration, client_clone.create_radio_from_metadata(&rk)).await {
                    Ok(Ok(tracks)) => {
                        let _ = tx.send(Event::StationTracksLoaded {
                            station_key: format!("plex_radio:{}", rk),
                            station_title: rt,
                            tracks,
                            time_travel_decades: vec![],
                        }).await;
                    }
                    Ok(Err(e)) => {
                        let _ = tx.send(Event::StationLoadFailed {
                            station_key: format!("plex_radio:{}", rk),
                            error: format!("Failed to start radio: {}", e),
                        }).await;
                    }
                    Err(_) => {
                        let _ = tx.send(Event::StationLoadFailed {
                            station_key: format!("plex_radio:{}", rk),
                            error: "Radio creation timed out".into(),
                        }).await;
                    }
                }
            });
        }
        Action::PlayStation(station_key) => {
            // Report stop for currently playing track before starting station
            // continuing=true because we're starting new content
            if let Some(current) = state.current_track().cloned() {
                helpers::report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
            }

            // Stop audio immediately to prevent stale TrackEnded events from the
            // old track being processed after the new station starts playing.
            audio.stop();
            state.playback.status = crate::app::state::PlayStatus::Stopped;

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
            let mut client_clone = client.clone();
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
            let mut client_clone = client.clone();
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
