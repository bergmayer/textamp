use super::super::recommendations::{self, Recommendations};
use super::*;
use crate::app::event::DataEvent;
use crate::app::state::{AdventureDrillLevel, RelatedArtistGroup, RelatedSource, SimilarMode};
use crate::navidrome::{array, object, Song};
use std::collections::HashSet;

pub(in crate::app::sources::navidrome) async fn similar(
    session: &Session,
    id: &str,
    sonic: bool,
) -> anyhow::Result<Vec<Track>> {
    anyhow::ensure!(!sonic || session.extensions.contains("sonicSimilarity"), "Sonic analysis unavailable: enable the AudioMuse sonicSimilarity plugin in Navidrome, then refresh with F5");
    let params = [("id", id.into()), ("count", "50".into())];
    let songs: Vec<Song> = if sonic {
        let response = session
            .client
            .call("getSonicSimilarTracks", &params)
            .await?;
        sonic_matches(&response)?
    } else {
        array(
            &session.client.call("getSimilarSongs2", &params).await?["similarSongs2"],
            "song",
        )?
    };
    Ok(songs.into_iter().map(|t| session.track(t)).collect())
}
pub(in crate::app::sources::navidrome) fn sonic_matches(
    response: &serde_json::Value,
) -> anyhow::Result<Vec<Song>> {
    array::<serde_json::Value>(response, "sonicMatch")?
        .iter()
        .map(|v| object(v, "entry"))
        .collect()
}
pub fn intercept(
    action: &Action,
    state: &mut AppState,
    tx: &mpsc::Sender<Event>,
    session: &Session,
) -> Option<Vec<Action>> {
    match action.clone() {
        Action::Data(DataAction::LoadSimilarTracks { rating_key, title }) => {
            load_similar(state, tx, session, rating_key, title, SimilarMode::Tracks)
        }
        Action::Data(DataAction::LoadSimilarAlbums { rating_key, title }) => {
            load_similar(state, tx, session, rating_key, title, SimilarMode::Albums)
        }
        Action::Data(DataAction::LoadSimilarArtists { artist_key, title }) => {
            load_similar(state, tx, session, artist_key, title, SimilarMode::Artists)
        }
        Action::Data(DataAction::LoadRelated { artist_key, title }) => {
            state.related.source_title = title;
            state.related.source_key = artist_key.clone();
            state.related.loading = true;
            state.related.groups.clear();
            state.set_view(View::Related);
            let albums = state.library.albums.clone();
            let artists = state.library.artists.clone();
            let session = session.clone();
            spawn(state, tx, "related", async move {
                let result: anyhow::Result<_> = async {
                    let id = session.id(&artist_key)?;
                    let response = session
                        .client
                        .call(
                            "getArtistInfo2",
                            &[
                                ("id", id),
                                ("count", "50".into()),
                                ("includeNotPresent", "false".into()),
                            ],
                        )
                        .await?;
                    let related: Vec<crate::navidrome::Artist> =
                        array(&response["artistInfo2"], "similarArtist")?;
                    Ok(related
                        .into_iter()
                        .filter_map(|a| {
                            artists
                                .iter()
                                .find(|known| known.rating_key == session.key(&a.id))
                                .cloned()
                        })
                        .map(|artist| RelatedArtistGroup {
                            albums: albums
                                .iter()
                                .filter(|a| {
                                    a.parent_rating_key.as_ref() == Some(&artist.rating_key)
                                })
                                .cloned()
                                .collect(),
                            artist,
                            source: RelatedSource::Navidrome,
                        })
                        .collect())
                }
                .await;
                Ok(vec![event(match result {
                    Ok(groups) => DataEvent::RelatedDataLoaded {
                        request_key: artist_key,
                        groups,
                    },
                    Err(e) => DataEvent::ScopedLoadError {
                        request_key: artist_key,
                        message: format!("{e:#}"),
                    },
                })])
            });
        }
        Action::Data(DataAction::LoadTrackPaneSimilar { rating_key }) => {
            if state.track_pane_similar.contains_key(&rating_key)
                || state.track_pane_similar_loading.contains(&rating_key)
            {
                return Some(vec![]);
            }
            state.track_pane_similar_loading.clear();
            state.track_pane_similar_loading.insert(rating_key.clone());
            let api = Recommendations::new(state, session);
            let sonic = recommendations::available(state);
            let session = session.clone();
            spawn(state, tx, "similar-pane", async move {
                let result = async {
                    Ok(api
                        .similar(&session.id(&rating_key)?, sonic)
                        .await?
                        .into_iter()
                        .filter(|t| t.rating_key != rating_key)
                        .collect())
                }
                .await
                .map_err(failure);
                Ok(vec![event(DataEvent::TrackPaneSimilarLoaded {
                    server_url: None,
                    rating_key,
                    result,
                })])
            });
        }
        Action::Playback(PlaybackAction::Stop) => {
            for slot in ["dj", "remix"] {
                state.sources.nav_tasks.remove(slot);
            }
            return None;
        }
        Action::Search(SearchAction::ArtistRadioPickerLaunch) => {
            if let Some(picker) = state.popups.artist_radio_picker.take() {
                let artists: Vec<_> = picker
                    .selected_artists
                    .iter()
                    .map(|a| &a.rating_key)
                    .collect();
                let encoded = serde_json::to_string(&artists).expect("string array");
                return Some(vec![RadioAction::StartStation(
                    crate::app::state::ActiveStation {
                        source: crate::library::models::RadioSource::Station(format!(
                            "nav-radio/artists?id={}",
                            urlencoding::encode(&encoded)
                        )),
                        title: "Artist Radio".into(),
                    },
                )
                .into()]);
            }
        }
        Action::Search(
            SearchAction::OpenAdventureLauncher
            | SearchAction::OpenAdventureLauncherWithStart { .. },
        ) if !recommendations::available(state) => {
            state.set_status("This server does not provide sonic paths".into())
        }
        Action::Search(SearchAction::AdventureLauncherDrillArtist { key, name }) => {
            let albums = state
                .library
                .albums
                .iter()
                .filter(|a| a.parent_rating_key.as_deref() == Some(&key))
                .cloned()
                .collect();
            if let Some(l) = &mut state.popups.adventure_launcher {
                l.drill = AdventureDrillLevel::ArtistAlbums {
                    artist_key: key,
                    artist_name: name,
                    albums,
                };
                l.item_index = 0;
                l.loading = false;
            }
        }
        Action::Search(SearchAction::AdventureLauncherDrillAlbum {
            key,
            title,
            artist_name,
        }) => {
            let tracks = browse::album_tracks(state, &key);
            if let Some(l) = &mut state.popups.adventure_launcher {
                l.drill = AdventureDrillLevel::AlbumTracks {
                    album_key: key,
                    album_title: title,
                    artist_name,
                    tracks,
                };
                l.item_index = 0;
                l.loading = false;
            }
        }
        Action::Search(SearchAction::AdventureLauncherGenerate) => {
            if let Some(l) = &state.popups.adventure_launcher {
                if let (Some(start), Some(end)) = (l.start_track.clone(), l.end_track.clone()) {
                    let count = l
                        .track_count_input
                        .parse::<usize>()
                        .unwrap_or(20)
                        .clamp(5, 100);
                    state.popups.adventure_launcher = None;
                    adventure(state, tx, session, start, end, count);
                } else {
                    state.set_error("Choose a start and end track".into());
                }
            }
        }
        Action::Settings(SettingsAction::SetAdventureLength(count)) => {
            if let (Some(start), Some(end)) = (
                state.adventure.start_track.clone(),
                state.adventure.end_track.clone(),
            ) {
                adventure(state, tx, session, start, end, count.clamp(5, 100));
            }
        }
        _ => return None,
    }
    Some(vec![])
}

fn load_similar(
    state: &mut AppState,
    tx: &mpsc::Sender<Event>,
    session: &Session,
    key: String,
    title: String,
    mode: SimilarMode,
) {
    state.similar.source_title = title;
    state.similar.mode = mode;
    state.similar.request_key = Some(key.clone());
    state.similar.loading = true;
    state.similar.tracks.clear();
    state.similar.artists.clear();
    state.similar.albums.clear();
    state.list_state.similar_index = 0;
    state.set_view(View::Similar);
    let albums = state.library.albums.clone();
    let artists = state.library.artists.clone();
    let seed = if mode == SimilarMode::Albums {
        browse::album_tracks(state, &key)
            .first()
            .map(|t| t.rating_key.clone())
            .unwrap_or_else(|| key.clone())
    } else {
        key.clone()
    };
    let api = Recommendations::new(state, session);
    let sonic = recommendations::available(state);
    let session = session.clone();
    spawn(state, tx, "similar", async move {
        let result = async {
            let id = session.id(&seed)?;
            if mode == SimilarMode::Artists {
                let response = session
                    .client
                    .call(
                        "getArtistInfo2",
                        &[
                            ("id", id),
                            ("count", "50".into()),
                            ("includeNotPresent", "false".into()),
                        ],
                    )
                    .await?;
                let related: Vec<crate::navidrome::Artist> =
                    array(&response["artistInfo2"], "similarArtist")?;
                let artists = related
                    .into_iter()
                    .filter_map(|a| {
                        artists
                            .iter()
                            .find(|known| known.rating_key == session.key(&a.id))
                            .cloned()
                    })
                    .collect();
                return Ok(DataEvent::SimilarArtistsLoaded {
                    request_key: key.clone(),
                    artists,
                });
            }
            let tracks: Vec<_> = api
                .similar(&id, sonic)
                .await?
                .into_iter()
                .filter(|t| t.rating_key != key)
                .collect();
            if mode == SimilarMode::Albums {
                let mut seen = HashSet::new();
                let albums = tracks
                    .iter()
                    .filter_map(|t| t.parent_rating_key.as_ref())
                    .filter(|id| *id != &key && seen.insert((*id).clone()))
                    .filter_map(|id| albums.iter().find(|a| &a.rating_key == id).cloned())
                    .collect();
                Ok(DataEvent::SimilarAlbumsLoaded {
                    request_key: key.clone(),
                    albums,
                })
            } else {
                Ok(DataEvent::SimilarTracksLoaded {
                    request_key: key.clone(),
                    tracks,
                })
            }
        }
        .await;
        Ok(vec![event(result.unwrap_or_else(|e: anyhow::Error| {
            DataEvent::ScopedLoadError {
                request_key: key,
                message: format!("Recommendations: {e:#}"),
            }
        }))])
    });
}
fn adventure(
    state: &mut AppState,
    tx: &mpsc::Sender<Event>,
    session: &Session,
    start: Track,
    end: Track,
    count: usize,
) {
    if !recommendations::available(state) {
        state.set_status("This server does not provide sonic paths".into());
        return;
    }
    state.adventure.generating = true;
    state.adventure_request_id = state.adventure_request_id.wrapping_add(1);
    let request_id = state.adventure_request_id;
    let api = Recommendations::new(state, session);
    let session = session.clone();
    spawn(state, tx, "adventure", async move {
        let result = async {
            let start_id = session.id(&start.rating_key)?;
            let end_id = session.id(&end.rating_key)?;
            api.path(&start_id, &end_id, count).await
        }
        .await
        .map_err(failure);
        Ok(vec![SettingsAction::AdventureGenerated {
            request_id,
            result,
        }
        .into()])
    });
}
