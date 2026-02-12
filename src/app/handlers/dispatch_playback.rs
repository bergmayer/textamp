//! Playback dispatch handlers: TogglePlayPause, Pause, Play, Stop, Next, Previous,
//! VolumeUp, VolumeDown, ToggleMute, Seek, SeekRelative, ToggleShuffle.

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
    // Remote playback guard: when output is Remote, branch to remote handlers
    if let crate::app::state::OutputTarget::Remote { ref player_id, ref player_uri, .. } = state.output_target {
        return dispatch_remote(event_tx, action, state, client, audio, player_id.clone(), player_uri.clone()).await;
    }

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
                    if state.current_track().is_some() {
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
        Action::StartPendingPlayback => {
            match audio.start_pending_playback() {
                Ok(true) => {
                    state.playback.status = PlayStatus::Playing;
                    state.playback.playback_started_at = Some(std::time::Instant::now());
                    // Don't reset consecutive_playback_errors here — wait for sustained
                    // playback (5s) to confirm the track is actually playing successfully.
                }
                Ok(false) => {
                    // No pending data — stale BufferingEnd event, ignore
                    tracing::debug!("StartPendingPlayback: no pending data (stale event?)");
                }
                Err(e) => {
                    // Route through PlaybackError for retry/skip logic
                    let tx = event_tx.clone();
                    let msg = format!("{}", e);
                    tokio::spawn(async move {
                        let _ = tx.send(Event::PlaybackError(msg)).await;
                    });
                }
            }
        }
        Action::RetryCurrentTrack => {
            // Replay the current track without resetting the error counter.
            // Used by PlaybackError handler to retry before skipping.
            helpers::play_current_track(event_tx, state, client, audio).await;
        }
        _ => unreachable!("dispatch_playback called with non-playback action: {:?}", action),
    }
    Ok(vec![])
}

/// Handle playback actions when output is a remote Plex player.
async fn dispatch_remote(
    event_tx: &mpsc::Sender<Event>,
    action: Action,
    state: &mut AppState,
    client: &mut PlexClient,
    audio: &mut AudioPlayer,
    target_player_id: String,
    player_uri: Option<String>,
) -> Result<Vec<Action>> {
    let token = client.token().map(|s| s.to_string()).unwrap_or_default();
    let client_id = client.client_identifier().to_string();
    let server_url = client.server_url().unwrap_or("").to_string();
    let machine_id = state.available_servers.first()
        .map(|s| s.client_identifier.clone()).unwrap_or_default();

    // Build remote client once — it's Clone so we can share it with spawned tasks
    let rc = crate::plex::RemotePlayerClient::new(
        token, client_id, target_player_id, server_url, machine_id, player_uri,
    );

    match action {
        Action::TogglePlayPause => {
            match state.playback.status {
                PlayStatus::Playing => {
                    let rc = rc.clone();
                    let event_tx = event_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = rc.pause().await {
                            let _ = event_tx.send(Event::RemotePlayerError(e.to_string())).await;
                        }
                    });
                    state.playback.status = PlayStatus::Paused;
                }
                PlayStatus::Paused => {
                    let rc = rc.clone();
                    let event_tx = event_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = rc.resume().await {
                            let _ = event_tx.send(Event::RemotePlayerError(e.to_string())).await;
                        }
                    });
                    state.playback.status = PlayStatus::Playing;
                }
                PlayStatus::Stopped => {
                    if state.current_track().is_some() {
                        helpers::play_current_track(event_tx, state, client, audio).await;
                    }
                }
                _ => {}
            }
        }
        Action::Pause => {
            let rc = rc.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = rc.pause().await {
                    let _ = event_tx.send(Event::RemotePlayerError(e.to_string())).await;
                }
            });
            state.playback.status = PlayStatus::Paused;
        }
        Action::Play => {
            let rc = rc.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = rc.resume().await {
                    let _ = event_tx.send(Event::RemotePlayerError(e.to_string())).await;
                }
            });
            state.playback.status = PlayStatus::Playing;
        }
        Action::Stop => {
            let rc = rc.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = rc.stop().await {
                    let _ = event_tx.send(Event::RemotePlayerError(e.to_string())).await;
                }
            });
            state.playback.status = PlayStatus::Stopped;
            state.playback.position_ms = 0;
            state.plex_session_id = None;
        }
        Action::Next => {
            match state.playback_mode {
                PlaybackMode::Radio => {
                    if let Some(idx) = state.radio.track_index {
                        if idx + 1 < state.radio.tracks.len() {
                            state.radio.track_index = Some(idx + 1);
                            helpers::play_current_track(event_tx, state, client, audio).await;

                            let remaining = state.radio.tracks.len().saturating_sub(idx + 1);
                            if remaining < 5 && !state.radio.fetching {
                                helpers::fetch_more_radio_tracks(event_tx, state, client);
                            }
                        } else if !state.radio.fetching {
                            helpers::fetch_more_radio_tracks(event_tx, state, client);
                        }
                    }
                }
                PlaybackMode::Queue | PlaybackMode::None => {
                    if let Some(idx) = state.queue_index {
                        if idx + 1 < state.queue.len() {
                            state.queue_index = Some(idx + 1);
                            helpers::play_current_track(event_tx, state, client, audio).await;
                        } else {
                            // End of queue
                            tokio::spawn(async move { let _ = rc.stop().await; });
                            state.playback.status = PlayStatus::Stopped;
                            state.plex_session_id = None;
                        }
                    }
                }
            }
        }
        Action::Previous => {
            if state.playback.position_ms > 3000 {
                state.playback.position_ms = 0;
                helpers::play_current_track(event_tx, state, client, audio).await;
            } else {
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
            let volume_pct = (state.playback.volume * 100.0) as u32;
            let rc = rc.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = rc.set_volume(volume_pct).await {
                    let _ = event_tx.send(Event::RemotePlayerError(e.to_string())).await;
                }
            });
        }
        Action::VolumeDown => {
            state.playback.volume = (state.playback.volume - 0.05).max(0.0);
            let volume_pct = (state.playback.volume * 100.0) as u32;
            let rc = rc.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = rc.set_volume(volume_pct).await {
                    let _ = event_tx.send(Event::RemotePlayerError(e.to_string())).await;
                }
            });
        }
        Action::ToggleMute => {
            state.playback.muted = !state.playback.muted;
            let volume_pct = if state.playback.muted { 0 } else { (state.playback.volume * 100.0) as u32 };
            let rc = rc.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = rc.set_volume(volume_pct).await {
                    let _ = event_tx.send(Event::RemotePlayerError(e.to_string())).await;
                }
            });
        }
        Action::Seek(position_ms) => {
            state.playback.position_ms = position_ms;
            // Recalibrate local clock so tick handler continues smoothly from the new position
            state.playback.playback_started_at = Some(
                std::time::Instant::now() - std::time::Duration::from_millis(position_ms)
            );
            let rc = rc.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = rc.seek_to(position_ms).await {
                    let _ = event_tx.send(Event::RemotePlayerError(e.to_string())).await;
                }
            });
        }
        Action::SeekRelative(delta_ms) => {
            let current = state.playback.position_ms as i64;
            let duration = state.playback.duration_ms as i64;
            let new_pos = (current + delta_ms).clamp(0, duration) as u64;
            state.playback.position_ms = new_pos;
            // Recalibrate local clock so tick handler continues smoothly from the new position
            state.playback.playback_started_at = Some(
                std::time::Instant::now() - std::time::Duration::from_millis(new_pos)
            );
            let rc = rc.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = rc.seek_to(new_pos).await {
                    let _ = event_tx.send(Event::RemotePlayerError(e.to_string())).await;
                }
            });
        }
        Action::StartPendingPlayback => {
            // No-op for remote — remote player handles its own buffering
        }
        Action::RetryCurrentTrack => {
            helpers::play_current_track(event_tx, state, client, audio).await;
        }
        _ => {}
    }
    Ok(vec![])
}
