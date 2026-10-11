//! Playback dispatch handlers: TogglePlayPause, Pause, Play, Stop, Next, Previous,
//! VolumeUp, VolumeDown, ToggleMute, Seek, SeekRelative, ToggleShuffle.

use crate::app::action::{PlaybackAction, RadioAction};
use crate::app::state::{PlayStatus, PlaybackMode};
use crate::app::{Action, AppState, Event};
use crate::audio::AudioPlayer;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Dispatch playback actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: PlaybackAction,
    state: &mut AppState,

    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    if matches!(
        action,
        PlaybackAction::Stop | PlaybackAction::ResetForAccountChange
    ) {
        state.sources.preparing = None;
        state.sources.prepared = None;
        state.playback.preparation_id = state.playback.preparation_id.wrapping_add(1);
    }
    // Discard playback belonging to the previous account before using new credentials.
    if matches!(&action, PlaybackAction::ResetForAccountChange) {
        audio.stop();
        state.playback.request_id = audio.playback_id();
        state.playback.preparation_id = state.playback.preparation_id.wrapping_add(1);
        state.playback.status = PlayStatus::Stopped;
        state.playback.position_ms = 0;
        state.playback.duration_ms = 0;
        state.playback.scrobble_reported = false;
        state.playback.playback_started_at = None;
        state.set_playback_mode(PlaybackMode::None);

        state.consecutive_playback_errors = 0;
        state.seek_drag = None;
        state.volume_drag = false;

        let old_queue = std::mem::take(&mut state.queue);
        let old_radio = std::mem::take(&mut state.radio);
        let old_dj = std::mem::take(&mut state.dj);
        crate::app::tasks::spawn_blocking(move || {
            drop(old_queue);
            drop(old_radio);
            drop(old_dj);
        });
        return Ok(vec![]);
    }

    // Stop cancels station preparation and refills. Explicit transport input
    // supersedes an automatic advance, while preserving useful prefetched tracks.
    if matches!(action, PlaybackAction::Stop)
        || (matches!(action, PlaybackAction::TogglePlayPause)
            && state.station_starting.as_ref().is_some_and(|start| {
                start.continue_playback.is_none() || state.playback.status == PlayStatus::Stopped
            }))
    {
        state.radio_generation = state.radio_generation.wrapping_add(1);
        state.radio_task = None;
        state.station_starting = None;
        state.radio.refill = crate::app::state::RadioRefill::Idle;
    } else if matches!(
        action,
        PlaybackAction::TogglePlayPause
            | PlaybackAction::Previous
            | PlaybackAction::Seek(_)
            | PlaybackAction::SeekRelative(_)
    ) && state.radio.refill == crate::app::state::RadioRefill::Waiting
    {
        state.radio.refill = crate::app::state::RadioRefill::Prefetching;
    }

    match action {
        PlaybackAction::OpenOutputPicker => {
            state.popups.close_all();
            state.palette.close();
            state.popups.sound_output_active = true;
            state.sound_output.scroll_pin = None;
            state.sound_output.index = state
                .sound_output
                .choices()
                .iter()
                .position(|o| *o == state.sound_output.current)
                .unwrap_or(0);
            return Ok(vec![PlaybackAction::DiscoverAirplay.into()]);
        }
        PlaybackAction::DiscoverAirplay => {
            state.sound_output.request_id = state.sound_output.request_id.wrapping_add(1);
            let request_id = state.sound_output.request_id;
            state.sound_output.loading = true;
            state.sound_output.error = None;
            let tx = event_tx.clone();
            crate::app::tasks::spawn(async move {
                let result = tokio::task::spawn_blocking(crate::audio::airplay::discover_outputs)
                    .await
                    .unwrap_or_else(|_| Err("Sound output discovery worker failed".into()));
                let _ = tx
                    .send(Event::Effect(
                        PlaybackAction::AirplayDiscovered { request_id, result }.into(),
                    ))
                    .await;
            });
        }
        PlaybackAction::AirplayDiscovered { request_id, result } => {
            if request_id != state.sound_output.request_id {
                return Ok(vec![]);
            }
            state.sound_output.loading = false;
            match result {
                Ok(found) => {
                    let focused = state
                        .sound_output
                        .choices()
                        .get(state.sound_output.index)
                        .cloned();
                    let mut errors = Vec::new();
                    match found.local {
                        Ok(devices) => state.sound_output.local_devices = devices,
                        Err(error) => errors.push(error),
                    }
                    match found.airplay {
                        Ok(devices) => state.sound_output.destinations = devices,
                        Err(error) => errors.push(error),
                    }
                    state.sound_output.error = (!errors.is_empty()).then(|| errors.join("; "));
                    state.sound_output.index = focused
                        .and_then(|selected| {
                            state
                                .sound_output
                                .choices()
                                .iter()
                                .position(|o| *o == selected)
                        })
                        .unwrap_or(0);
                }
                Err(error) => state.sound_output.error = Some(error),
            }
        }
        PlaybackAction::SelectOutput(output) => {
            if output == state.sound_output.current && audio.has_audio() {
                state.popups.sound_output_active = false;
                state.sound_output.error = None;
                return Ok(vec![]);
            }
            if state.playback.status != PlayStatus::Stopped
                || state.sources.preparing.is_some()
                || state.station_starting.is_some()
            {
                state.sound_output.error =
                    Some("Stop playback before selecting an audio output".into());
                return Ok(vec![]);
            }
            let label = output.label();
            state.sound_output.current = output.clone();
            state.playback.preparation_id = state.playback.preparation_id.wrapping_add(1);
            match audio.set_output(output) {
                Ok(()) => {
                    audio.set_volume(state.playback.volume);
                    state.sound_output.error = None;
                    state.popups.sound_output_active = false;
                    state.set_status(format!("Output: {label}"));
                }
                Err(error) => {
                    state.sound_output.error = Some(format!("Output {label} unavailable: {error}"))
                }
            }
            state.vectorscope_tap = audio.sample_tap();
            state.playback.request_id = audio.playback_id();
        }
        PlaybackAction::TogglePlayPause => match state.playback.status {
            PlayStatus::Playing => {
                audio.pause();
                state.playback.status = PlayStatus::Paused;
            }
            PlayStatus::Paused => {
                audio.resume();
                state.playback.status = PlayStatus::Playing;
            }
            PlayStatus::Stopped if state.current_track().is_some() => {
                helpers::play_current_track(event_tx, state, audio);
            }
            _ => {}
        },
        PlaybackAction::Stop => {
            audio.stop();
            state.playback.request_id = audio.playback_id();
            state.playback.status = PlayStatus::Stopped;
            state.playback.position_ms = 0;
        }
        PlaybackAction::ResetForAccountChange => {
            unreachable!("account reset is handled before playback dispatch")
        }
        PlaybackAction::Next => {
            let mut track_advanced = false;

            match state.playback_mode {
                PlaybackMode::Radio => {
                    track_advanced = helpers::advance_radio(event_tx, state, audio);
                }
                PlaybackMode::Queue | PlaybackMode::None => {
                    // Queue mode: use state.queue
                    if let Some(idx) = state.queue.index {
                        if idx + 1 < state.queue.tracks.len() {
                            state.queue.index = Some(idx + 1);
                            helpers::play_current_track(event_tx, state, audio);
                            track_advanced = true;
                        } else {
                            // End of queue: report final stop to server (not continuing)

                            audio.stop();
                            state.playback.request_id = audio.playback_id();
                            state.playback.status = PlayStatus::Stopped;
                        }
                    }
                }
            }

            // Trigger DJ mode processing after every track transition.
            // All DJ modes are continuous: insert tracks after current position.
            if track_advanced && !state.dj.inserting && state.dj.active_mode.is_some() {
                return Ok(vec![RadioAction::DjModeProcess.into()]);
            }
        }
        PlaybackAction::Previous => {
            // If more than 3 seconds in, restart current track (no stop report needed)
            if state.playback.position_ms > 3000 {
                state.playback.position_ms = 0;
                helpers::play_current_track(event_tx, state, audio);
            } else {
                // Report stop for current track before going to previous
                // continuing=true because we're moving to the previous track

                // Go to previous track based on playback mode
                match state.playback_mode {
                    PlaybackMode::Radio => {
                        if let Some(idx) = state.radio.track_index {
                            if idx > 0 {
                                state.radio.track_index = Some(idx - 1);
                                helpers::play_current_track(event_tx, state, audio);
                            }
                        }
                    }
                    PlaybackMode::Queue | PlaybackMode::None => {
                        if let Some(idx) = state.queue.index {
                            if idx > 0 {
                                state.queue.index = Some(idx - 1);
                                helpers::play_current_track(event_tx, state, audio);
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
            audio.set_volume(if state.playback.muted {
                0.0
            } else {
                state.playback.volume
            });
        }
        PlaybackAction::Seek(position_ms) => {
            if state.playback.duration_ms == 0 || state.playback.status == PlayStatus::Stopped {
                return Ok(vec![]);
            }
            let position_ms = position_ms.min(state.playback.duration_ms);
            let previous_id = audio.playback_id();
            match audio.seek(std::time::Duration::from_millis(position_ms)) {
                Ok(()) => {
                    state.playback.position_ms = position_ms;
                    state.playback.request_id = audio.playback_id();
                    if previous_id != state.playback.request_id
                        && state.playback.status != PlayStatus::Paused
                    {
                        state.playback.status = PlayStatus::Buffering;
                    }
                }
                Err(error) => {
                    tracing::warn!("Could not seek: {error}");
                    state.set_status("Cannot seek".into());
                }
            }
        }
        PlaybackAction::SeekRelative(delta_ms) => {
            let position = state
                .playback
                .position_ms
                .saturating_add_signed(delta_ms)
                .min(state.playback.duration_ms);
            return Ok(vec![PlaybackAction::Seek(position).into()]);
        }

        PlaybackAction::RetryCurrentTrack => {
            // Replay the current track without resetting the error counter.
            // Used by PlaybackError handler to retry before skipping.
            helpers::play_current_track(event_tx, state, audio);
        }
    }
    Ok(vec![])
}
