//! Station results from all providers. Owns the atomic playback-state commit.
use crate::app::action::*;
use crate::app::event::RadioEvent;
use crate::app::state::{PlayStatus, PlaybackMode, View};
use crate::app::{Action, AppState};

fn empty_station_detail(source: &crate::library::station::RadioSource) -> &'static str {
    use crate::library::models::RadioSource;
    match source {
        RadioSource::Artist(_) => "this artist has no playable tracks",
        RadioSource::Sonic(_) => "no sonically similar tracks were found",
        RadioSource::Station(key) if key.ends_with("/randomAlbum") => {
            "no playable albums were found in this library"
        }
        RadioSource::Station(key) if key.ends_with("/onThisDay") => {
            "no albums in this library were released on today's date"
        }
        RadioSource::Station(key)
            if ["/stations/mood?", "/stations/style?", "/stations/decade?"]
                .iter()
                .any(|kind| key.contains(kind)) =>
        {
            "this category contains no playable tracks"
        }
        RadioSource::Station(_) => "no playable tracks were returned",
    }
}

pub fn handle(event: RadioEvent, state: &mut AppState) -> Vec<Action> {
    match event {
        RadioEvent::StationTracksLoaded {
            station,
            mut tracks,
            time_travel_decades,
            time_travel_index,
        } => {
            state.radio_task = None;
            let continue_playback = state
                .station_starting
                .take()
                .and_then(|start| start.continue_playback);
            if continue_playback.is_some_and(|id| id != state.playback.request_id) {
                state.set_status("Sonic Radio cancelled: playback changed while loading".into());
                return vec![];
            }
            let preserve_current =
                continue_playback.is_some() && state.playback.status != PlayStatus::Stopped;
            // If the final song ended while discovery was loading, start the
            // recommendations without replaying the song that just finished.
            if continue_playback.is_some() && !preserve_current && !tracks.is_empty() {
                tracks.remove(0);
            }
            let station_title = station.title.clone();
            if tracks.is_empty() {
                let detail = empty_station_detail(&station.source);
                state.set_error(format!(
                    "{}: {}. Previous queue unchanged.",
                    station_title, detail
                ));
            } else {
                // Commit only after loading succeeds. Report the old track before
                // replacing its state; PlayCurrentRadioTrack replaces its audio.

                state.dj.active_mode = None;
                state.dj.history.clear();
                state.dj.inserting = false;
                state.dj.last_was_inserted = false;
                state.clear_error();
                state.set_playback_mode(PlaybackMode::Radio);
                state.queue.selected.clear();
                state.radio.clear();
                state.radio.active_station = Some(station);
                state.radio.tracks = tracks;
                state.radio.track_index = Some(0);
                state.radio.time_travel_index = time_travel_index.unwrap_or(0);

                // Time Travel Radio initialization
                if !time_travel_decades.is_empty() {
                    state.radio.time_travel_decades = time_travel_decades;
                    state.radio.time_travel_index = time_travel_index.unwrap_or(0);
                    tracing::info!(
                        "Time Travel Radio: initialized with {} decades, next fetch from index {}",
                        state.radio.time_travel_decades.len(),
                        state.radio.time_travel_index
                    );
                }

                state.list_state.queue_index = 0;
                state.set_view(View::Queue);
                state.set_status(if preserve_current {
                    "Sonic Radio ready".into()
                } else {
                    format!(
                        "Playing {} ({} tracks)",
                        station_title,
                        state.radio.tracks.len()
                    )
                });
                return if preserve_current {
                    vec![]
                } else {
                    vec![RadioAction::PlayCurrentRadioTrack.into()]
                };
            }
            vec![]
        }
        RadioEvent::StationLoadFailed { error } => {
            state.radio_task = None;
            state.station_starting = None;
            state.set_error(error.message);
            vec![]
        }
        RadioEvent::StationChildrenFailed(error) => {
            state.stations_loading = false;
            state.station_nav.loading = false;
            state.set_error(error.message.clone());
            vec![]
        }
        RadioEvent::StationChildrenLoaded {
            station_key,
            station_title,
            children,
        } => {
            state.stations_loading = false;
            state.station_nav.loading = false;

            // Cache children for instant loading next time
            state
                .station_children_cache
                .insert(station_key.clone(), children.clone());
            state.cache_mgmt.dirty = true;

            // Push new column with children (Miller columns style)
            state
                .station_nav
                .push_column(crate::app::state::StationColumn::new(
                    Some(station_key),
                    station_title,
                    children.clone(),
                ));
            // Also update the legacy state for compatibility
            state.stations = children;
            state.clear_error();
            if state.palette.open {
                crate::app::command_palette::refresh_matches(state);
            }
            vec![]
        }
        // Radio track fetching completed (background)
        RadioEvent::RadioTracksLoaded {
            result,
            time_travel_index,
        } => {
            state.radio_task = None;
            let waiting =
                std::mem::take(&mut state.radio.refill) == crate::app::state::RadioRefill::Waiting;
            let tracks = match result {
                Ok(tracks) => tracks,
                Err(error) => {
                    state.set_error(error.message);
                    return vec![];
                }
            };
            // Deduplicate against existing tracks
            let mut existing_keys: std::collections::HashSet<_> = state
                .radio
                .tracks
                .iter()
                .map(|t| t.rating_key.clone())
                .collect();

            let unique_tracks: Vec<_> = tracks
                .into_iter()
                .filter(|t| existing_keys.insert(t.rating_key.clone()))
                .collect();

            let added = unique_tracks.len();
            if added > 0 {
                tracing::info!("Radio: adding {} new unique tracks", added);
                state.radio.tracks.extend(unique_tracks);
            }

            if let Some(idx) = time_travel_index {
                state.radio.time_travel_index = idx;
            }

            if added == 0 {
                state.set_error(
                    "Radio returned no new tracks. Try Next again or choose another station."
                        .into(),
                );
            } else if waiting && state.playback_mode == PlaybackMode::Radio {
                return vec![PlaybackAction::Next.into()];
            }

            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_station_messages_do_not_blame_sonic_analysis() {
        assert_eq!(
            empty_station_detail(&crate::library::models::RadioSource::Station(
                "/library/sections/5/stations/onThisDay".into()
            )),
            "no albums in this library were released on today's date"
        );
        assert_eq!(
            empty_station_detail(&crate::library::models::RadioSource::Station(
                "/library/sections/5/stations/decade?id=1990".into()
            )),
            "this category contains no playable tracks"
        );
        assert!(
            !empty_station_detail(&crate::library::models::RadioSource::Artist("123".into()))
                .contains("Sonic")
        );
    }
}
