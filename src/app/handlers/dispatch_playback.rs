//! Playback dispatch handlers: TogglePlayPause, Pause, Play, Stop, Next, Previous,
//! VolumeUp, VolumeDown, ToggleMute, Seek, SeekRelative, ToggleShuffle.

use crate::app::event::*;
use crate::app::{Action, AppState, Event};
use crate::app::action::{PlaybackAction, RadioAction};
use crate::app::state::{PlayStatus, PlaybackMode};
use crate::plex::PlexClient;
use crate::audio::AudioPlayer;

use anyhow::Result;
use std::future::Future;
use tokio::sync::mpsc;

use super::helpers;

fn spawn_remote_command<F>(
    event_tx: &mpsc::Sender<Event>,
    library_generation: u64,
    player_id: String,
    command: RemoteCommandKind,
    operation: F,
)
where
    F: Future<Output = Result<(), crate::plex::ApiError>> + Send + 'static,
{
    let event_tx = event_tx.clone();
    tokio::spawn(async move {
        let error = operation.await.err().map(|error| error.to_string());
        let _ = event_tx
            .send(Event::for_library(
                library_generation,
                RemoteEvent::RemoteCommandResult {
                    player_id,
                    command,
                    error,
                },
            ))
            .await;
    });
}

/// Dispatch playback actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: PlaybackAction,
    state: &mut AppState,
    client: &mut PlexClient,
    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    // Credential replacement is deliberately handled before the remote-output
    // branch.  The old remote target belongs to the old account, while the
    // client already contains the new credentials by the time this action is
    // dispatched.  Sending either a Plex timeline or remote stop here would
    // therefore cross account boundaries.
    if matches!(&action, PlaybackAction::ResetForAccountChange) {
        audio.stop();
        audio.track_cache.flush();
        state.playback.request_id = audio.playback_id();
        state.playback.preparation_id = state.playback.preparation_id.wrapping_add(1);
        state.playback.status = PlayStatus::Stopped;
        state.playback.position_ms = 0;
        state.playback.duration_ms = 0;
        state.playback.scrobble_reported = false;
        state.playback.playback_started_at = None;
        state.playback_mode = PlaybackMode::None;
        state.plex_session_id = None;
        state.last_progress_report = None;
        state.consecutive_playback_errors = 0;
        state.seeking_drag = false;
        state.volume_drag = false;

        let old_queue = std::mem::take(&mut state.queue);
        let old_radio = std::mem::take(&mut state.radio);
        let old_radio_state = std::mem::take(&mut state.radio_state);
        let old_dj = std::mem::take(&mut state.dj);
        tokio::task::spawn_blocking(move || {
            drop(old_queue);
            drop(old_radio);
            drop(old_radio_state);
            drop(old_dj);
        });
        return Ok(vec![]);
    }

    if matches!(&action, PlaybackAction::PrefetchUpcoming) {
        let upcoming = helpers::get_upcoming_tracks(state);
        crate::audio::cache::trigger_prefetch(
            &audio.track_cache,
            &upcoming,
            client,
            state.transcode_kbps,
        );
        return Ok(vec![]);
    }

    // Remote playback guard: when output is Remote, branch to remote handlers
    if let crate::app::state::OutputTarget::Remote { ref player_id, ref player_uri, .. } = state.remote.output_target {
        return dispatch_remote(event_tx, action, state, client, audio, player_id.clone(), player_uri.clone());
    }

    match action {
        PlaybackAction::TogglePlayPause => {
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
                        helpers::play_current_track(event_tx, state, client, audio);
                    }
                }
                _ => {}
            }
        }
        PlaybackAction::Stop => {
            // Report stop to Plex before stopping
            // continuing=false because playback is truly stopping
            if let Some(track) = state.current_track().cloned() {
                helpers::report_playback_stop_to_plex(&track, state.playback.position_ms, false, state.plex_session_id.clone(), client);
            }
            audio.stop();
            state.playback.request_id = audio.playback_id();
            state.playback.status = PlayStatus::Stopped;
            state.playback.position_ms = 0;
            // Clear session ID when playback truly stops
            state.plex_session_id = None;
        }
        PlaybackAction::ResetForAccountChange => unreachable!(
            "account reset is handled before local/remote playback dispatch"
        ),
        PlaybackAction::Next => {
            // Report stop for current track before switching
            // continuing=true because we're moving to the next track
            if let Some(track) = state.current_track().cloned() {
                helpers::report_playback_stop_to_plex(&track, state.playback.position_ms, true, state.plex_session_id.clone(), client);
            }

            let mut track_advanced = false;

            match state.playback_mode {
                PlaybackMode::Radio => {
                    // Radio mode: use radio.tracks and auto-fetch more
                    if let Some(idx) = state.radio.track_index {
                        if idx + 1 < state.radio.tracks.len() {
                            state.radio.track_index = Some(idx + 1);
                            helpers::play_current_track(event_tx, state, client, audio);
                            track_advanced = true;

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
                    if let Some(idx) = state.queue.index {
                        if idx + 1 < state.queue.tracks.len() {
                            state.queue.index = Some(idx + 1);
                            helpers::play_current_track(event_tx, state, client, audio);
                            track_advanced = true;
                        } else {
                            // End of queue: report final stop to Plex (not continuing)
                            if let Some(track) = state.current_track().cloned() {
                                helpers::report_playback_stop_to_plex(&track, state.playback.position_ms, false, state.plex_session_id.clone(), client);
                            }
                            audio.stop();
                            state.playback.request_id = audio.playback_id();
                            state.playback.status = PlayStatus::Stopped;
                            state.plex_session_id = None;
                        }
                    }
                }
            }

            // Trigger DJ mode processing after every track transition.
            // All DJ modes are continuous: insert tracks after current position.
            if track_advanced && !state.dj.inserting {
                if state.dj.active_mode.is_some() {
                    return Ok(vec![RadioAction::DjModeProcess.into()]);
                }
            }
        }
        PlaybackAction::Previous => {
            // If more than 3 seconds in, restart current track (no stop report needed)
            if state.playback.position_ms > 3000 {
                state.playback.position_ms = 0;
                helpers::play_current_track(event_tx, state, client, audio);
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
                                helpers::play_current_track(event_tx, state, client, audio);
                            }
                        }
                    }
                    PlaybackMode::Queue | PlaybackMode::None => {
                        if let Some(idx) = state.queue.index {
                            if idx > 0 {
                                state.queue.index = Some(idx - 1);
                                helpers::play_current_track(event_tx, state, client, audio);
                            }
                        }
                    }
                }
            }
        }
        PlaybackAction::VolumeUp => {
            state.playback.volume = (state.playback.volume + 0.05).min(1.0);
            audio.set_volume(state.playback.volume);
        }
        PlaybackAction::VolumeDown => {
            state.playback.volume = (state.playback.volume - 0.05).max(0.0);
            audio.set_volume(state.playback.volume);
        }
        PlaybackAction::SetVolume(vol) => {
            state.playback.volume = vol.clamp(0.0, 1.0);
            state.playback.muted = false;
            audio.set_volume(state.playback.volume);
        }
        PlaybackAction::ToggleMute => {
            state.playback.muted = !state.playback.muted;
            audio.set_volume(if state.playback.muted { 0.0 } else { state.playback.volume });
        }
        PlaybackAction::Seek(position_ms) => {
            // Seek to absolute position
            let position = std::time::Duration::from_millis(position_ms);
            if audio.try_seek(position) {
                state.playback.position_ms = position_ms;
            }
        }
        PlaybackAction::SeekRelative(delta_ms) => {
            // Seek relative to current position
            let current = state.playback.position_ms as i64;
            let duration = state.playback.duration_ms as i64;
            let new_pos = (current + delta_ms).clamp(0, duration) as u64;
            let position = std::time::Duration::from_millis(new_pos);
            if audio.try_seek(position) {
                state.playback.position_ms = new_pos;
            }
        }
        PlaybackAction::StartResolvedStream {
            preparation_id,
            track_key,
            url,
        } => {
            let current = state.current_track().cloned();
            if state.playback.preparation_id != preparation_id
                || current.as_ref().map(|track| track.rating_key.as_str())
                    != Some(track_key.as_str())
                || state.playback.status != PlayStatus::Buffering
            {
                tracing::debug!("Ignoring stale resolved stream action");
                return Ok(vec![]);
            }
            if let Some(track) = current {
                helpers::start_resolved_stream(
                    event_tx,
                    state,
                    client,
                    audio,
                    &track,
                    &url,
                    true,
                );
            }
        }
        PlaybackAction::RetryCurrentTrack => {
            // Replay the current track without resetting the error counter.
            // Used by PlaybackError handler to retry before skipping.
            helpers::play_current_track(event_tx, state, client, audio);
        }
        PlaybackAction::PrefetchUpcoming => unreachable!(
            "prefetch is handled before local/remote playback dispatch"
        ),
    }
    Ok(vec![])
}

/// Handle playback actions when output is a remote Plex player.
fn dispatch_remote(
    event_tx: &mpsc::Sender<Event>,
    action: PlaybackAction,
    state: &mut AppState,
    client: &mut PlexClient,
    audio: &mut AudioPlayer,
    target_player_id: String,
    player_uri: Option<String>,
) -> Result<Vec<Action>> {
    let token = client.shared_token_or_empty();
    let client_id = client.client_identifier().to_string();
    let server_url = client.server_url().unwrap_or("").to_string();
    let machine_id = state.active_server_id.clone()
        .or_else(|| state.available_servers.first()
            .map(|server| server.client_identifier.clone()))
        .unwrap_or_default();

    // Build remote client once — it's Clone so we can share it with spawned tasks
    let rc = match crate::plex::RemotePlayerClient::new(
        token, client_id, target_player_id.clone(), server_url, machine_id, player_uri,
    ) {
        Ok(client) => client,
        Err(error) => {
            state.set_error(format!("Remote player unavailable: {error}"));
            return Ok(vec![]);
        }
    };

    match action {
        PlaybackAction::TogglePlayPause => {
            match state.playback.status {
                PlayStatus::Playing => {
                    let rc = rc.clone();
                    spawn_remote_command(
                        event_tx,
                        state.library_generation,
                        target_player_id.clone(),
                        RemoteCommandKind::Pause,
                        async move { rc.pause().await },
                    );
                    let position = state.remote.playback.estimated_position();
                    state.remote.playback.anchor(position, false);
                    state.playback.position_ms = position;
                    state.playback.status = PlayStatus::Paused;
                }
                PlayStatus::Paused => {
                    let rc = rc.clone();
                    spawn_remote_command(
                        event_tx,
                        state.library_generation,
                        target_player_id.clone(),
                        RemoteCommandKind::Resume,
                        async move { rc.resume().await },
                    );
                    state.remote.playback.anchor(state.playback.position_ms, true);
                    state.playback.status = PlayStatus::Playing;
                }
                PlayStatus::Stopped => {
                    if state.current_track().is_some() {
                        helpers::play_current_track(event_tx, state, client, audio);
                    }
                }
                _ => {}
            }
        }
        PlaybackAction::Stop => {
            let was_playing = state.playback.status == PlayStatus::Playing;
            let position_ms = state.remote.playback.estimated_position();
            let rc = rc.clone();
            spawn_remote_command(
                event_tx,
                state.library_generation,
                target_player_id.clone(),
                RemoteCommandKind::Stop { was_playing, position_ms },
                async move { rc.stop().await },
            );
            state.remote.playback.anchor(0, false);
            state.playback.status = PlayStatus::Stopped;
            state.playback.position_ms = 0;
            state.plex_session_id = None;
        }
        PlaybackAction::ResetForAccountChange => unreachable!(
            "account reset is handled before local/remote playback dispatch"
        ),
        PlaybackAction::Next => {
            // Report stop for current track before switching
            if let Some(track) = state.current_track().cloned() {
                helpers::report_playback_stop_to_plex(&track, state.playback.position_ms, true, state.plex_session_id.clone(), client);
            }

            let mut track_advanced = false;

            match state.playback_mode {
                PlaybackMode::Radio => {
                    if let Some(idx) = state.radio.track_index {
                        if idx + 1 < state.radio.tracks.len() {
                            state.radio.track_index = Some(idx + 1);
                            helpers::play_current_track(event_tx, state, client, audio);
                            track_advanced = true;

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
                    if let Some(idx) = state.queue.index {
                        if idx + 1 < state.queue.tracks.len() {
                            state.queue.index = Some(idx + 1);
                            helpers::play_current_track(event_tx, state, client, audio);
                            track_advanced = true;
                        } else {
                            // End of queue — report final stop (not continuing)
                            if let Some(track) = state.current_track().cloned() {
                                helpers::report_playback_stop_to_plex(&track, state.playback.position_ms, false, state.plex_session_id.clone(), client);
                            }
                            let rc = rc.clone();
                            let position_ms = state.remote.playback.estimated_position();
                            spawn_remote_command(
                                event_tx,
                                state.library_generation,
                                target_player_id.clone(),
                                RemoteCommandKind::Stop {
                                    was_playing: true,
                                    position_ms,
                                },
                                async move { rc.stop().await },
                            );
                            state.playback.status = PlayStatus::Stopped;
                            state.plex_session_id = None;
                        }
                    }
                }
            }

            // Trigger DJ mode processing after every track transition
            if track_advanced && !state.dj.inserting {
                if state.dj.active_mode.is_some() {
                    return Ok(vec![RadioAction::DjModeProcess.into()]);
                }
            }
        }
        PlaybackAction::Previous => {
            if state.playback.position_ms > 3000 {
                state.playback.position_ms = 0;
                helpers::play_current_track(event_tx, state, client, audio);
            } else {
                // Report stop for current track before going to previous
                if let Some(track) = state.current_track().cloned() {
                    helpers::report_playback_stop_to_plex(&track, state.playback.position_ms, true, state.plex_session_id.clone(), client);
                }

                match state.playback_mode {
                    PlaybackMode::Radio => {
                        if let Some(idx) = state.radio.track_index {
                            if idx > 0 {
                                state.radio.track_index = Some(idx - 1);
                                helpers::play_current_track(event_tx, state, client, audio);
                            }
                        }
                    }
                    PlaybackMode::Queue | PlaybackMode::None => {
                        if let Some(idx) = state.queue.index {
                            if idx > 0 {
                                state.queue.index = Some(idx - 1);
                                helpers::play_current_track(event_tx, state, client, audio);
                            }
                        }
                    }
                }
            }
        }
        PlaybackAction::VolumeUp => {
            state.playback.volume = (state.playback.volume + 0.05).min(1.0);
            let volume_pct = (state.playback.volume * 100.0) as u32;
            let rc = rc.clone();
            spawn_remote_command(
                event_tx,
                state.library_generation,
                target_player_id.clone(),
                RemoteCommandKind::SetVolume,
                async move { rc.set_volume(volume_pct).await },
            );
        }
        PlaybackAction::VolumeDown => {
            state.playback.volume = (state.playback.volume - 0.05).max(0.0);
            let volume_pct = (state.playback.volume * 100.0) as u32;
            let rc = rc.clone();
            spawn_remote_command(
                event_tx,
                state.library_generation,
                target_player_id.clone(),
                RemoteCommandKind::SetVolume,
                async move { rc.set_volume(volume_pct).await },
            );
        }
        PlaybackAction::SetVolume(vol) => {
            state.playback.volume = vol.clamp(0.0, 1.0);
            state.playback.muted = false;
            let volume_pct = (state.playback.volume * 100.0) as u32;
            let rc = rc.clone();
            spawn_remote_command(
                event_tx,
                state.library_generation,
                target_player_id.clone(),
                RemoteCommandKind::SetVolume,
                async move { rc.set_volume(volume_pct).await },
            );
        }
        PlaybackAction::ToggleMute => {
            state.playback.muted = !state.playback.muted;
            let volume_pct = if state.playback.muted { 0 } else { (state.playback.volume * 100.0) as u32 };
            let rc = rc.clone();
            spawn_remote_command(
                event_tx,
                state.library_generation,
                target_player_id.clone(),
                RemoteCommandKind::SetVolume,
                async move { rc.set_volume(volume_pct).await },
            );
        }
        PlaybackAction::Seek(position_ms) => {
            state.playback.position_ms = position_ms;
            state.remote.playback.anchor(
                position_ms,
                state.playback.status == PlayStatus::Playing,
            );
            let rc = rc.clone();
            spawn_remote_command(
                event_tx,
                state.library_generation,
                target_player_id.clone(),
                RemoteCommandKind::Seek { position_ms },
                async move { rc.seek_to(position_ms).await },
            );
        }
        PlaybackAction::SeekRelative(delta_ms) => {
            let current = state.playback.position_ms as i64;
            let duration = state.playback.duration_ms as i64;
            let new_pos = (current + delta_ms).clamp(0, duration) as u64;
            state.playback.position_ms = new_pos;
            state.remote.playback.anchor(
                new_pos,
                state.playback.status == PlayStatus::Playing,
            );
            let rc = rc.clone();
            spawn_remote_command(
                event_tx,
                state.library_generation,
                target_player_id.clone(),
                RemoteCommandKind::Seek { position_ms: new_pos },
                async move { rc.seek_to(new_pos).await },
            );
        }
        PlaybackAction::StartResolvedStream { .. } => {
            // Remote playback performs its own server-side media negotiation.
        }
        PlaybackAction::RetryCurrentTrack => {
            helpers::play_current_track(event_tx, state, client, audio);
        }
        PlaybackAction::PrefetchUpcoming => unreachable!(
            "prefetch is handled before local/remote playback dispatch"
        ),
    }
    Ok(vec![])
}
