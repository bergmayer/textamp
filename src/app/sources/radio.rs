//! One station lifecycle for every library: prepare, fetch, commit, and refill.
//! Providers return tracks; only the shared event reducer changes playback.
use super::{ActiveSource, navidrome};
use crate::app::action::{AsyncError, SystemAction};
use crate::app::event::{LibraryEventSender, RadioEvent};
use crate::app::state::{ActiveStation, PlayStatus, RadioRefill, View};
use crate::app::tasks::TaskLease;
use crate::app::{Action, AppState, Event};
use crate::library::station::RadioSource;
use crate::library::track::Track;

use rand::prelude::IndexedRandom;
use std::collections::HashSet;
use tokio::sync::mpsc;

pub struct Batch {
    pub tracks: Vec<Track>,
    pub decades: Vec<String>,
    pub next_decade: Option<usize>,
}
impl Batch {
    pub fn tracks(tracks: Vec<Track>) -> Self {
        Self {
            tracks,
            decades: vec![],
            next_decade: None,
        }
    }
}

enum Request {
    Navidrome(navidrome::radio::Request),
    Folder(Vec<Track>),
}
impl Request {
    fn prepare(
        state: &AppState,

        station: &ActiveStation,
        refill: bool,
    ) -> Result<Self, AsyncError> {
        // Sonic Mix follows the buffer tail; fixed-seed modes retain their chosen seed.
        let source = match (&station.source, refill, state.radio.tracks.last()) {
            (RadioSource::Sonic(_), true, Some(track)) => {
                RadioSource::Sonic(track.rating_key.clone())
            }
            _ => station.source.clone(),
        };
        Ok(match &state.sources.active {
            ActiveSource::None => {
                return Err(AsyncError {
                    message: "Select a library first".into(),
                });
            }

            ActiveSource::Navidrome(session) => Self::Navidrome(
                navidrome::radio::Request::prepare(state, session, &source, refill)?,
            ),
            ActiveSource::Folder(_) => {
                let seed = match &source {
                    RadioSource::Station(k)
                        if matches!(
                            k.as_str(),
                            "folder-radio/randomAlbum" | "folder-radio/library"
                        ) =>
                    {
                        None
                    }
                    RadioSource::Related(key) => {
                        state.radio.tracks.iter().find(|t| t.rating_key == *key)
                    }
                    _ => return Err(failure("This station is unavailable for folder libraries")),
                };
                let empty = HashSet::new();
                let excluded = if refill {
                    &state.radio.introduced_albums
                } else {
                    &empty
                };
                Self::Folder(
                    state
                        .library
                        .folder_radio
                        .select(excluded, seed)
                        .map_err(|e| failure(&e.to_string()))?,
                )
            }
        })
    }
    async fn fetch(self) -> Result<Batch, AsyncError> {
        match self {
            Self::Navidrome(request) => request.fetch().await,
            Self::Folder(tracks) => Ok(Batch::tracks(tracks)),
        }
    }
}
fn failure(message: &str) -> AsyncError {
    AsyncError {
        message: message.into(),
    }
}

pub fn start(tx: &mpsc::Sender<Event>, state: &mut AppState, station: ActiveStation) {
    if matches!(station.source, RadioSource::SonicFixed(_)) {
        if !super::sonic::enabled(state) {
            state.set_error("Stay Close Mix requires enabled sonic analysis".into());
            return;
        }
        let seed = match &station.source {
            RadioSource::SonicFixed(key) => state
                .current_track()
                .filter(|track| &track.rating_key == key)
                .or_else(|| {
                    state
                        .library
                        .all_tracks
                        .iter()
                        .find(|track| &track.rating_key == key)
                })
                .cloned(),
            _ => None,
        };
        if seed.is_none() {
            state.set_error("Choose a track from the active library".into());
            return;
        }
        begin(tx, state, station, seed);
        return;
    }
    let sonic_seed = match &station.source {
        RadioSource::Sonic(key) => Some(
            state
                .current_track()
                .filter(|t| &t.rating_key == key)
                .or_else(|| {
                    state
                        .library
                        .all_tracks
                        .iter()
                        .find(|t| &t.rating_key == key)
                })
                .cloned(),
        ),
        RadioSource::Station(key)
            if key == "nav-radio/sonic" || key.ends_with("/stations/sonic") =>
        {
            Some(
                state
                    .current_track()
                    .or_else(|| state.library.all_tracks.choose(&mut rand::rng()))
                    .cloned(),
            )
        }
        _ => None,
    };
    if let Some(seed) = sonic_seed {
        if let Some(seed) = seed {
            start_sonic(tx, state, seed);
        } else {
            state.set_error("Choose a track to start Sonic Mix".into());
        }
        return;
    }
    begin(tx, state, station, None);
}

pub fn start_sonic(tx: &mpsc::Sender<Event>, state: &mut AppState, seed: Track) {
    if !super::sonic::enabled(state) {
        state.set_status("Sonic Mix requires enabled sonic analysis".into());
        return;
    }
    let valid = match &state.sources.active {
        ActiveSource::Navidrome(session) => session.id(&seed.rating_key).is_ok(),
        ActiveSource::Folder(_) | ActiveSource::None => false,
    };
    if !valid {
        state.set_error("This track does not belong to the active library".into());
        return;
    }
    let station = ActiveStation {
        source: RadioSource::Sonic(seed.rating_key.clone()),
        title: "Sonic Mix".into(),
    };
    begin(tx, state, station, Some(seed));
}

fn begin(
    tx: &mpsc::Sender<Event>,
    state: &mut AppState,

    station: ActiveStation,
    seed: Option<Track>,
) {
    state.radio_generation = state.radio_generation.wrapping_add(1);
    state.radio.refill = RadioRefill::Idle;
    let continue_playback = seed
        .as_ref()
        .filter(|seed| {
            state
                .current_track()
                .is_some_and(|t| t.rating_key == seed.rating_key)
                && matches!(
                    state.playback.status,
                    PlayStatus::Playing | PlayStatus::Paused | PlayStatus::Buffering
                )
        })
        .map(|_| state.playback.request_id);
    state.station_starting = Some(crate::app::state::StationStart {
        title: station.title.clone(),
        continue_playback,
    });
    state.set_view(View::Queue);
    launch(tx, state, station, false, seed);
}
pub fn refill(tx: &mpsc::Sender<Event>, state: &mut AppState) {
    if super::sonic::radio_blocked(state)
        || state.radio.refill != RadioRefill::Idle
        || state.station_starting.is_some()
    {
        return;
    }
    if let Some(station) = state.radio.active_station.clone() {
        state.radio.refill = RadioRefill::Prefetching;
        launch(tx, state, station, true, None);
    }
}
fn launch(
    tx: &mpsc::Sender<Event>,
    state: &mut AppState,

    station: ActiveStation,
    refill: bool,
    seed: Option<Track>,
) {
    state.radio_task = None;
    let request = Request::prepare(state, &station, refill);

    let tx = LibraryEventSender::new(tx.clone(), state.library_generation)
        .with_radio(state.radio_generation);
    let task = crate::app::tasks::spawn(async move {
        let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            request?.fetch().await
        })
        .await
        .unwrap_or_else(|_| {
            Err(AsyncError {
                message: "Radio request timed out; previous playback is unchanged. Try again."
                    .into(),
            })
        });
        let event = match (refill, result) {
            (false, Ok(mut batch)) => {
                if let Some(seed) = seed {
                    batch.tracks.retain(|t| {
                        t.rating_key != seed.rating_key
                            && t.parent_rating_key != seed.parent_rating_key
                    });
                    if batch.tracks.is_empty() {
                        let _ = tx
                            .send(
                                RadioEvent::StationLoadFailed {
                                    error: failure(
                                        "No sonically similar tracks are available for this song",
                                    ),
                                }
                                .into(),
                            )
                            .await;
                        return;
                    }
                    batch.tracks.insert(0, seed);
                }
                RadioEvent::StationTracksLoaded {
                    station,
                    tracks: batch.tracks,
                    time_travel_decades: batch.decades,
                    time_travel_index: batch.next_decade,
                }
            }
            (false, Err(error)) => RadioEvent::StationLoadFailed { error },
            (true, Ok(batch)) => RadioEvent::RadioTracksLoaded {
                result: Ok(batch.tracks),
                time_travel_index: batch.next_decade,
            },
            (true, Err(error)) => RadioEvent::RadioTracksLoaded {
                result: Err(error),
                time_travel_index: None,
            },
        };
        let _ = tx.send(event.into()).await;
    });
    state.radio_task = Some(TaskLease::new(&task));
}

pub fn children(
    tx: &mpsc::Sender<Event>,
    state: &mut AppState,

    key: String,
    title: String,
) -> Vec<Action> {
    match state.sources.active {
        ActiveSource::Navidrome(_) => {
            navidrome::radio::children(state, tx, key, title);
            vec![]
        }
        ActiveSource::Folder(_) | ActiveSource::None => {
            vec![SystemAction::SetStatus("Folder radio has no tag categories".into()).into()]
        }
    }
}

pub fn load_folder_stations(state: &mut AppState) {
    let stations = if state.library.folder_radio.available() {
        crate::library::station::standard_stations(
            "folder-radio",
            crate::library::capabilities::FOLDERS,
        )
    } else {
        vec![]
    };
    state.station_nav.columns = vec![crate::app::state::StationColumn::new(
        None,
        "stations".into(),
        stations,
    )];
    state.station_nav.focused_column = 0;
    state.station_nav.loading = false;
}
