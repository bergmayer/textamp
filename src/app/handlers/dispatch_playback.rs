//! Playback dispatch handlers: TogglePlayPause, Pause, Play, Stop, Next, Previous,
//! VolumeUp, VolumeDown, ToggleMute, Seek, SeekRelative, ToggleShuffle, CycleRepeat.

use crate::app::{Action, AppState, Event};
use crate::app::state::{PlayStatus, PlaybackMode};
use crate::api::PlexClient;
use crate::audio::AudioPlayer;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Dispatch playback actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: Action,
    state: &mut AppState,
    client: &mut PlexClient,
    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    match action {
        Action::TogglePlayPause => {
            match state.playback.status {
                PlayStatus::Playing => {
                    audio.pause();
                    state.playback.status = PlayStatus::Paused;
                }
                PlayStatus::Paused => {
                    audio.resume();
                    state.playback.status = PlayStatus::Playing;
                }
                PlayStatus::Stopped => {
                    if state.queue_index.is_some() {
                        helpers::play_current_track(event_tx, state, client, audio).await;
                    }
                }
                _ => {}
            }
        }
        Action::Pause => {
            // Report stop to Plex before pausing
            // continuing=false because playback is pausing (not moving to next track)
            if let Some(track) = state.current_track().cloned() {
                helpers::report_playback_stop_to_plex(&track, state.playback.position_ms, false, state.plex_session_id.clone(), client);
            }
            audio.pause();
            state.playback.status = PlayStatus::Paused;
        }
        Action::Play => {
            audio.resume();
            state.playback.status = PlayStatus::Playing;
        }
        Action::Stop => {
            // Report stop to Plex before stopping
            // continuing=false because playback is truly stopping
            if let Some(track) = state.current_track().cloned() {
                helpers::report_playback_stop_to_plex(&track, state.playback.position_ms, false, state.plex_session_id.clone(), client);
            }
            audio.stop();
            state.playback.status = PlayStatus::Stopped;
            state.playback.position_ms = 0;
            // Clear session ID when playback truly stops
            state.plex_session_id = None;
        }
        Action::Next => {
            // Report stop for current track before switching
            // continuing=true because we're moving to the next track
            if let Some(track) = state.current_track().cloned() {
                helpers::report_playback_stop_to_plex(&track, state.playback.position_ms, true, state.plex_session_id.clone(), client);
            }

            match state.playback_mode {
                PlaybackMode::Radio => {
                    // Radio mode: use radio.tracks and auto-fetch more
                    if let Some(idx) = state.radio.track_index {
                        if idx + 1 < state.radio.tracks.len() {
                            state.radio.track_index = Some(idx + 1);
                            helpers::play_current_track(event_tx, state, client, audio).await;

                            // Auto-fetch more tracks when running low
                            let remaining = state.radio.tracks.len().saturating_sub(idx + 1);
                            if remaining < 5 && !state.radio.fetching {
                                helpers::fetch_more_radio_tracks(event_tx, state, client);
                            }
                        } else if !state.radio.fetching {
                            // At end, fetch more — RadioTracksLoaded handler will auto-advance
                            helpers::fetch_more_radio_tracks(event_tx, state, client);
                        }
                    }
                }
                PlaybackMode::Queue | PlaybackMode::None => {
                    // Queue mode: use state.queue
                    if let Some(idx) = state.queue_index {
                        if idx + 1 < state.queue.len() {
                            state.queue_index = Some(idx + 1);
                            helpers::play_current_track(event_tx, state, client, audio).await;
                        } else if state.playback.repeat_mode == crate::app::state::RepeatMode::All {
                            state.queue_index = Some(0);
                            helpers::play_current_track(event_tx, state, client, audio).await;
                        } else {
                            // End of queue: report final stop to Plex (not continuing)
                            if let Some(track) = state.current_track().cloned() {
                                helpers::report_playback_stop_to_plex(&track, state.playback.position_ms, false, state.plex_session_id.clone(), client);
                            }
                            audio.stop();
                            state.playback.status = PlayStatus::Stopped;
                            state.plex_session_id = None;
                        }
                    }
                }
            }
        }
        Action::Previous => {
            // If more than 3 seconds in, restart current track (no stop report needed)
            if state.playback.position_ms > 3000 {
                state.playback.position_ms = 0;
                helpers::play_current_track(event_tx, state, client, audio).await;
            } else {
                // Report stop for current track before going to previous
                // continuing=true because we're moving to the previous track
                if let Some(track) = state.current_track().cloned() {
                    helpers::report_playback_stop_to_plex(&track, state.playback.position_ms, true, state.plex_session_id.clone(), client);
                }

                // Go to previous track based on playback mode
                match state.playback_mode {
                    PlaybackMode::Radio => {
                        if let Some(idx) = state.radio.track_index {
                            if idx > 0 {
                                state.radio.track_index = Some(idx - 1);
                                helpers::play_current_track(event_tx, state, client, audio).await;
                            }
                        }
                    }
                    PlaybackMode::Queue | PlaybackMode::None => {
                        if let Some(idx) = state.queue_index {
                            if idx > 0 {
                                state.queue_index = Some(idx - 1);
                                helpers::play_current_track(event_tx, state, client, audio).await;
                            }
                        }
                    }
                }
            }
        }
        Action::VolumeUp => {
            state.playback.volume = (state.playback.volume + 0.05).min(1.0);
            audio.set_volume(state.playback.volume);
        }
        Action::VolumeDown => {
            state.playback.volume = (state.playback.volume - 0.05).max(0.0);
            audio.set_volume(state.playback.volume);
        }
        Action::ToggleMute => {
            state.playback.muted = !state.playback.muted;
            audio.set_volume(if state.playback.muted { 0.0 } else { state.playback.volume });
        }
        Action::Seek(position_ms) => {
            // Seek to absolute position
            let position = std::time::Duration::from_millis(position_ms);
            if audio.try_seek(position) {
                state.playback.position_ms = position_ms;
            }
        }
        Action::SeekRelative(delta_ms) => {
            // Seek relative to current position
            let current = state.playback.position_ms as i64;
            let duration = state.playback.duration_ms as i64;
            let new_pos = (current + delta_ms).clamp(0, duration) as u64;
            let position = std::time::Duration::from_millis(new_pos);
            if audio.try_seek(position) {
                state.playback.position_ms = new_pos;
            }
        }
        Action::CycleRepeat => {
            state.playback.repeat_mode = state.playback.repeat_mode.next();
        }
        Action::StartPendingPlayback => {
            match audio.start_pending_playback() {
                Ok(()) => {
                    state.playback.status = PlayStatus::Playing;
                    state.consecutive_playback_errors = 0;
                }
                Err(e) => {
                    state.set_error(format!("Playback error: {}", e));
                    state.playback.status = PlayStatus::Stopped;
                }
            }
        }
        _ => unreachable!("dispatch_playback called with non-playback action: {:?}", action),
    }
    Ok(vec![])
}
