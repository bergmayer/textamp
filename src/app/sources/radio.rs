//! One station lifecycle for every library: prepare, fetch, commit, and refill.
//! Providers return tracks; only the shared event reducer changes playback.
use super::{navidrome, ActiveSource};
use crate::app::action::{AsyncError, SystemAction};
use crate::app::event::{LibraryEventSender, RadioEvent};
use crate::app::state::{ActiveStation, PlayStatus, RadioRefill, View};
use crate::app::tasks::TaskLease;
use crate::app::{Action, AppState, Event};
use crate::library::station::{RadioSource, Station};
use crate::library::{radio::Recipe, track::Track, FolderSource};

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
    Navidrome(Box<navidrome::radio::Request>),
    Folder {
        source: FolderSource,
        recipe: Recipe,
        excluded: HashSet<String>,
    },
}
impl Request {
    fn prepare(
        state: &AppState,

        station: &ActiveStation,
        refill: bool,
    ) -> Result<Self, AsyncError> {
        // Refill follows the currently audible song, not the original seed forever.
        let source = match (&station.source, refill, state.current_track()) {
            (RadioSource::Sonic(_), true, Some(track)) => {
                RadioSource::Sonic(track.rating_key.clone())
            }
            _ => station.source.clone(),
        };
        Ok(match &state.sources.active {
            ActiveSource::None => {
                return Err(AsyncError {
                    message: "Select a library first".into(),
                })
            }

            ActiveSource::Navidrome(session) => Self::Navidrome(Box::new(
                navidrome::radio::Request::prepare(state, session, &source, refill),
            )),
            ActiveSource::Folder(id) => {
                let source = state
                    .sources
                    .folders
                    .iter()
                    .find(|s| &s.id == id)
                    .cloned()
                    .ok_or_else(|| failure("The folder library has been removed"))?;
                let recipe = match station.source.station_key() {
                    Some("folder-radio/randomAlbum") => Recipe::RandomAlbum,
                    Some("folder-radio/library") => Recipe::Library,
                    _ => {
                        return Err(failure(
                            "This station is not available for folder libraries",
                        ))
                    }
                };
                let excluded = if refill {
                    state
                        .radio
                        .tracks
                        .iter()
                        .map(|t| t.rating_key.clone())
                        .collect()
                } else {
                    HashSet::new()
                };
                Self::Folder {
                    source,
                    recipe,
                    excluded,
                }
            }
        })
    }
    async fn fetch(self) -> Result<Batch, AsyncError> {
        match self {
            Self::Navidrome(request) => request.fetch().await,
            Self::Folder {
                source,
                recipe,
                excluded,
            } => {
                let store = crate::library::cache::Store::folder(&source)
                    .map_err(|e| failure(&format!("{e:#}")))?;
                crate::library::radio::select(&source, &store, recipe, &excluded)
                    .await
                    .map(Batch::tracks)
                    .map_err(|e| failure(&format!("{e:#}")))
            }
        }
    }
}
fn failure(message: &str) -> AsyncError {
    AsyncError {
        message: message.into(),
    }
}

pub fn start(tx: &mpsc::Sender<Event>, state: &mut AppState, station: ActiveStation) {
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
            state.set_error("Choose a track to start Sonic Radio".into());
        }
        return;
    }
    begin(tx, state, station, None);
}

pub fn start_sonic(tx: &mpsc::Sender<Event>, state: &mut AppState, seed: Track) {
    if !super::sonic::enabled(state) {
        state.set_status("Sonic Radio requires enabled sonic analysis".into());
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
        title: "Sonic Radio".into(),
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
                    batch.tracks.retain(|t| t.rating_key != seed.rating_key);
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

/// Validate before shared DJ state changes, then let the provider supply tracks.
pub fn dj_available(state: &AppState, mode: crate::app::state::DjMode) -> bool {
    if !state
        .sources
        .active
        .capabilities()
        .supports(super::sonic::dj_feature(mode))
    {
        return false;
    }
    if !super::sonic::dj_requires_sonic(mode) {
        return true;
    }
    match &state.sources.active {
        ActiveSource::Navidrome(_) => navidrome::recommendations::available(state),
        ActiveSource::Folder(_) | ActiveSource::None => false,
    }
}
pub fn process_dj(tx: &mpsc::Sender<Event>, state: &mut AppState) {
    match state.sources.active {
        ActiveSource::Navidrome(_) => navidrome::dj::process(state, tx),
        ActiveSource::Folder(_) | ActiveSource::None => {
            state.dj.inserting = false;
            state.set_status("DJ modes need artist metadata or sonic analysis".into());
        }
    }
}

pub fn folder_stations() -> Vec<Station> {
    crate::library::station::standard_stations(
        "folder-radio",
        crate::library::capabilities::FOLDERS,
    )
}

pub fn load_folder_stations(state: &mut AppState) {
    state.stations = folder_stations();
    state.station_nav.columns = vec![crate::app::state::StationColumn::new(
        None,
        "stations".into(),
        state.stations.clone(),
    )];
    state.station_nav.focused_column = 0;
    state.station_nav.loading = false;
    state.stations_loading = false;
}
