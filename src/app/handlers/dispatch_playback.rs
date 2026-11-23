//! Playback dispatch handlers: TogglePlayPause, Pause, Play, Stop, Next, Previous,
//! VolumeUp, VolumeDown, ToggleMute, Seek, SeekRelative, ToggleShuffle.

use crate::app::action::PlaybackAction;
use crate::app::state::{PlayStatus, PlaybackMode};
use crate::app::{Action, AppState, Event};
use crate::audio::AudioPlayer;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Dispatch playback actions. Returns follow-up actions.
pub fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: PlaybackAction,
    state: &mut AppState,

    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    if matches!(
        action,
        PlaybackAction::Stop
            | PlaybackAction::OutputInterrupted
            | PlaybackAction::ResetForAccountChange
    ) {
        state.sources.media = crate::app::sources::MediaPreparation::Idle;
        state.playback.preparation_id = state.playback.preparation_id.wrapping_add(1);
    }
    // Discard playback belonging to the previous account before using new credentials.
    if matches!(&action, PlaybackAction::ResetForAccountChange) {
        audio.stop();
        state.playback.request_id = audio.playback_id();
        state.playback.status = PlayStatus::Stopped;
        state.playback.position_ms = 0;
        state.playback.duration_ms = 0;
        state.playback.scrobble = crate::app::state::ScrobbleState::Pending;
        state.playback.playback_started_at = None;
        state.set_playback_mode(PlaybackMode::None);

        state.consecutive_playback_errors = 0;
        state.seek_drag = None;
        state.volume_drag = false;

        let old_queue = std::mem::take(&mut state.queue);
        let old_radio = std::mem::take(&mut state.radio);
        crate::app::tasks::spawn_blocking(move || {
            drop(old_queue);
            drop(old_radio);
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
        PlaybackAction::ShowLyrics => {
            if let Some(track) = state.current_track().cloned() {
                crate::app::lyrics::open(state, event_tx, track, true);
            }
        }
        PlaybackAction::RetryLyrics => {
            if let Some(panel) = state.popups.lyrics().cloned() {
                crate::app::lyrics::open(state, event_tx, panel.track, panel.playback_bound);
            }
        }
        PlaybackAction::ToggleLyricsSync => {
            if let Some(panel) = state.popups.lyrics_mut() {
                panel.follow = !panel.follow;
            }
        }
        PlaybackAction::SelectLyricsLanguage(index) => {
            if let Some(panel) = state.popups.lyrics_mut()
                && index < panel.documents().len()
            {
                panel.language = index;
                panel.scroll = 0;
            }
        }
        PlaybackAction::LyricsLoaded {
            generation,
            request,
            result,
        } => crate::app::lyrics::complete(state, generation, request, result),
        PlaybackAction::SearchLyrics(query) => {
            return Ok(vec![crate::app::lyrics::web_search(&query)]);
        }

        PlaybackAction::ToggleAutoPlay => {
            if crate::app::queue_modes::current(state) == crate::app::queue_modes::Mode::Related {
                return Ok(vec![]);
            }
            crate::app::queue_modes::continue_related(state, event_tx);
        }
        PlaybackAction::ToggleAutoMix => {
            state.playback.auto_mix = !state.playback.auto_mix;
        }
        PlaybackAction::MixNext { playback_id } => {
            if state.playback.auto_mix
                && state.playback.request_id == playback_id
                && state.playback.status == PlayStatus::Playing
                && state.next_track().is_some()
                && state.queue.repeat != crate::app::state::RepeatMode::Track
            {
                audio.prepare_transition(std::time::Duration::from_millis(
                    state
                        .playback
                        .duration_ms
                        .saturating_sub(state.playback.position_ms)
                        .clamp(20, 4000),
                ));
                if let Some(track) = state.current_track().cloned()
                    && state.playback.scrobble == crate::app::state::ScrobbleState::Pending
                {
                    state.playback.scrobble = crate::app::state::ScrobbleState::Queued;
                    crate::app::sources::navidrome::scrobble(state, event_tx, &track);
                }
                return Ok(vec![PlaybackAction::Next.into()]);
            }
        }
        PlaybackAction::OutputInterrupted => {
            if let Some(position) = audio.position() {
                state.playback.position_ms = position.as_millis() as u64;
            }
            audio.interrupt_output();
            state.playback.request_id = audio.playback_id();
            if state.playback.status != PlayStatus::Stopped {
                state.playback.status = PlayStatus::Paused;
            }
        }
        PlaybackAction::TogglePlayPause => match state.playback.status {
            PlayStatus::Playing => {
                audio.pause();
                state.playback.status = PlayStatus::Paused;
            }
            PlayStatus::Paused if !audio.has_audio() && !audio.has_resume_input() => {
                helpers::play_current_track(event_tx, state, audio);
            }
            PlayStatus::Paused => match audio
                .resume_at(std::time::Duration::from_millis(state.playback.position_ms))
            {
                Ok(()) => {
                    state.playback.request_id = audio.playback_id();
                    state.playback.status = PlayStatus::Playing;
                }
                Err(error) => state.set_error(format!("Audio is unavailable: {error}")),
            },
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
            match state.playback_mode {
                PlaybackMode::Radio => {
                    helpers::advance_radio(event_tx, state, audio);
                }
                PlaybackMode::Queue | PlaybackMode::None => {
                    // Queue mode: use state.queue
                    if let Some(idx) = state.queue.index {
                        if idx + 1 < state.queue.tracks.len() {
                            state.queue.index = Some(idx + 1);
                            helpers::play_current_track(event_tx, state, audio);
                        } else if state.queue.repeat == crate::app::state::RepeatMode::Queue
                            && !state.queue.tracks.is_empty()
                        {
                            state.queue.index = Some(0);
                            helpers::play_current_track(event_tx, state, audio);
                        } else {
                            // End of queue: report final stop to server (not continuing)

                            audio.stop();
                            state.playback.request_id = audio.playback_id();
                            state.playback.status = PlayStatus::Stopped;
                        }
                    }
                }
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
                        if let Some(idx) = state.radio.track_index
                            && idx > 0
                        {
                            state.radio.track_index = Some(idx - 1);
                            helpers::play_current_track(event_tx, state, audio);
                        }
                    }
                    PlaybackMode::Queue | PlaybackMode::None => {
                        if let Some(idx) = state.queue.index
                            && idx > 0
                        {
                            state.queue.index = Some(idx - 1);
                            helpers::play_current_track(event_tx, state, audio);
                        }
                    }
                }
            }
        }
        PlaybackAction::VolumeUp => {
            state.playback.muted = false;
            state.playback.volume = (state.playback.volume + 0.05).min(1.0);
            audio.set_volume(state.playback.volume);
        }
        PlaybackAction::VolumeDown => {
            state.playback.muted = false;
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

#[cfg(test)]
#[path = "continuous_playback_tests.rs"]
mod continuous_playback;
