//! Radio dispatch handlers: JumpToRadioTrack, StartArtistRadio,
//! PlayStation, DrillIntoStation, NavigateStationsBack, PlayCurrentRadioTrack,
//! ToggleDjMode, DjModeProcess, DjModeTracksReady, DjModeBatchReady.

use crate::app::action::RadioAction;
use crate::app::state::{DjMode, PlaybackMode};
use crate::app::{Action, AppState, Event};
use crate::audio::AudioPlayer;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Dispatch radio and station actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: RadioAction,
    state: &mut AppState,

    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    match action {
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
            state.stations_loading = false;
            state.station_nav.loading = false;
            // Go back in Miller columns (just move focus left - data already in memory)
            if state.station_nav.can_go_left() {
                state.station_nav.focus_left();
                // Update legacy state to match focused column
                if let Some(col) = state.station_nav.focused() {
                    state.stations = col.stations.clone();
                }
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
        RadioAction::ToggleDjMode(mode) => {
            if state.dj.active_mode != Some(mode)
                && !crate::app::sources::radio::dj_available(state, mode)
            {
                state.set_status(
                    "This DJ mode needs artist metadata or enabled sonic analysis".into(),
                );
                return Ok(vec![]);
            }
            state.sources.nav_tasks.remove("dj");
            tracing::info!("ToggleDjMode: {:?}, current_mode={:?}, playback_mode={:?}, queue_len={}, queue_index={:?}, current_track={}",
                mode, state.dj.active_mode, state.playback_mode,
                state.queue.tracks.len(), state.queue.index,
                state.current_track().map(|t| t.title.as_str()).unwrap_or("None"));

            if state.dj.active_mode == Some(mode) {
                // Same mode active → deactivate
                state.dj.active_mode = None;
                state.dj.history.clear();
                state.dj.inserting = false;
                state.dj.last_was_inserted = false;
                state.set_status(format!("{} off", mode.name()));
            } else {
                // DJ + Station mutual exclusivity: convert radio to queue if active
                if state.playback_mode == PlaybackMode::Radio {
                    tracing::info!("DJ mode: converting radio to queue (radio.tracks={}, radio.track_index={:?})",
                        state.radio.tracks.len(), state.radio.track_index);
                    state.queue.tracks = state.radio.tracks.clone();
                    state.queue.index = state.radio.track_index;
                    state.set_playback_mode(PlaybackMode::Queue);
                    state.radio.clear();
                }

                // Activate new mode (or switch from a different one)
                state.dj.active_mode = Some(mode);
                state.dj.history.clear();
                state.dj.inserting = false;
                state.dj.last_was_inserted = false;
                state.set_status(format!("{} on", mode.name()));

                // All DJ modes are continuous: insert DJ tracks after current position
                return Ok(vec![RadioAction::DjModeProcess.into()]);
            }
        }
        RadioAction::DjModeProcess => {
            // Only for continuous modes (Freeze, Contempo, Groupie)
            crate::app::sources::radio::process_dj(event_tx, state);
        }
        RadioAction::DjModeTracksReady(result, _insert_next) => {
            // Insert DJ-picked tracks right after the current track position.
            state.dj.inserting = false;

            let tracks = match result {
                Ok(tracks) => tracks,
                Err(error) => {
                    state.set_error(error.message);
                    return Ok(vec![]);
                }
            };

            if tracks.is_empty() {
                if let Some(mode) = state.dj.active_mode {
                    let hint = match mode {
                        DjMode::Freeze | DjMode::Gemini | DjMode::Stretch => format!(
                            "{}: no similar tracks found (requires Sonic Analysis)",
                            mode.name()
                        ),
                        _ => format!("{}: no matching tracks found", mode.name()),
                    };
                    state.set_status(hint);
                }
                return Ok(vec![]);
            }

            for track in &tracks {
                state.dj.history.push(track.rating_key.clone());
            }

            // Mark that the next track to play is a DJ insertion.
            // Interleaving modes use this to alternate: original → DJ → original.
            state.dj.last_was_inserted = true;

            insert_tracks_after_current(state, tracks);

            // Pre-cache the newly inserted DJ tracks (and other upcoming tracks)
        }
        RadioAction::DjModeBatchReady(inserts) => {
            // Inserter mode batch results: interleave into queue
            state.dj.inserting = false;

            if inserts.is_empty() {
                return Ok(vec![]);
            }

            // Collect inserts into a map: original_index -> tracks_to_insert_after
            let inserts_map: std::collections::HashMap<usize, Vec<crate::library::models::Track>> =
                inserts.into_iter().collect();

            // Add all inserted track keys to history
            for tracks in inserts_map.values() {
                for track in tracks {
                    state.dj.history.push(track.rating_key.clone());
                }
            }

            // Process inserts in reverse index order so earlier splices don't shift later indices
            let mut positions: Vec<usize> = inserts_map.keys().copied().collect();
            positions.sort_unstable_by(|a, b| b.cmp(a));

            match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => {
                    for pos in positions {
                        if let Some(insert_tracks) = inserts_map.get(&pos) {
                            let insert_at = (pos + 1).min(state.queue.tracks.len());
                            state
                                .queue
                                .tracks
                                .splice(insert_at..insert_at, insert_tracks.iter().cloned());
                        }
                    }
                }
                PlaybackMode::Radio => {
                    for pos in positions {
                        if let Some(insert_tracks) = inserts_map.get(&pos) {
                            let insert_at = (pos + 1).min(state.radio.tracks.len());
                            state
                                .radio
                                .tracks
                                .splice(insert_at..insert_at, insert_tracks.iter().cloned());
                        }
                    }
                }
            }

            // Pre-cache upcoming tracks (including newly interleaved DJ tracks)
        }
    }
    Ok(vec![])
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Insert tracks right after the current playback position.
/// Works for both Queue and Radio modes.
fn insert_tracks_after_current(state: &mut AppState, tracks: Vec<crate::library::models::Track>) {
    match state.playback_mode {
        PlaybackMode::Queue | PlaybackMode::None => {
            let insert_at = state.queue.index.unwrap_or(0) + 1;
            let insert_at = insert_at.min(state.queue.tracks.len());
            state.queue.tracks.splice(insert_at..insert_at, tracks);
        }
        PlaybackMode::Radio => {
            let insert_at = state.radio.track_index.unwrap_or(0) + 1;
            let insert_at = insert_at.min(state.radio.tracks.len());
            state.radio.tracks.splice(insert_at..insert_at, tracks);
        }
    }
}
