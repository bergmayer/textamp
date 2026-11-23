//! Radio dispatch handlers: JumpToRadioTrack, StartArtistRadio,
//! PlayStation, DrillIntoStation, NavigateStationsBack, PlayCurrentRadioTrack,
//! Continuous station playback and source-owned refills.

use crate::app::action::RadioAction;
use crate::app::{Action, AppState, Event};
use crate::audio::AudioPlayer;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Dispatch radio and station actions. Returns follow-up actions.
pub fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: RadioAction,
    state: &mut AppState,

    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    match action {
        RadioAction::StopStation => {
            state.radio_task = None;
            state.station_starting = None;
            state.radio_generation = state.radio_generation.wrapping_add(1);
            if state.playback_mode == crate::app::state::PlaybackMode::Radio {
                state.queue.tracks = state.radio.tracks.clone();
                state.queue.index = state.radio.track_index;
                state.queue.original.clear();
                state.queue.sort_mode = crate::app::state::QueueSortMode::QueueOrder;
                state.set_playback_mode(crate::app::state::PlaybackMode::Queue);
                state.radio.clear();
                state.set_status("Station stopped. Queued tracks will keep playing.".into());
            }
        }
        RadioAction::StartSonicRadio(track) => {
            crate::app::sources::radio::start_sonic(event_tx, state, *track);
        }
        RadioAction::StartStation(station) => {
            crate::app::sources::radio::start(event_tx, state, station);
        }
        RadioAction::JumpToRadioTrack(idx) => {
            // Report stop for current track before jumping
            // continuing=true because we're jumping to another track

            // Jump to track in radio queue without clearing radio state
            if idx < state.radio.tracks.len() {
                state.radio.track_index = Some(idx);
                state.list_state.queue_index = idx;
                helpers::play_current_track(event_tx, state, audio);
            }
        }
        RadioAction::StartArtistRadio { key, title } => {
            crate::app::sources::radio::start(
                event_tx,
                state,
                crate::app::state::ActiveStation {
                    source: crate::library::models::RadioSource::Artist(key),
                    title,
                },
            );
        }
        RadioAction::PlayStation(station_key) => {
            if state
                .station_by_key(&station_key)
                .is_some_and(|station| station.identifier.as_deref() == Some("stayClose"))
            {
                return Ok(vec![
                    crate::app::action::SearchAction::OpenStayCloseRadioPicker.into(),
                ]);
            }
            if state
                .station_by_key(&station_key)
                .is_some_and(|station| station.identifier.as_deref() == Some("sonic"))
            {
                return Ok(vec![
                    crate::app::action::SearchAction::OpenTrackRadioPicker.into(),
                ]);
            }
            let station_title = state
                .station_by_key(&station_key)
                .map(|station| station.title.clone())
                .unwrap_or_else(|| "Radio".to_string());

            crate::app::sources::radio::start(
                event_tx,
                state,
                crate::app::state::ActiveStation {
                    source: crate::library::models::RadioSource::Station(station_key),
                    title: station_title,
                },
            );
        }
        RadioAction::DrillIntoStation(station_key, station_title) => {
            return Ok(crate::app::sources::radio::children(
                event_tx,
                state,
                station_key,
                station_title,
            ));
        }
        RadioAction::NavigateStationsBack => {
            state.station_navigation_generation =
                state.station_navigation_generation.wrapping_add(1);
            state.station_nav.loading = false;
            // Go back in Miller columns (just move focus left - data already in memory)
            if state.station_nav.can_go_left() {
                state.station_nav.focus_left();
            }
            if state.palette.open {
                crate::app::command_palette::refresh_matches(state);
            }
        }
        RadioAction::PlayCurrentRadioTrack => {
            audio.stop();
            state.playback.request_id = audio.playback_id();
            // Play the current track in radio mode (stays in Radio playback mode)
            state.consecutive_playback_errors = 0;
            helpers::play_current_track(event_tx, state, audio);
        }
    }
    Ok(vec![])
}
