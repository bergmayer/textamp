//! Navidrome stations use the selected catalog; only sonic/related discovery
//! needs a server request. Startup and refill share the same selection rules.
use super::effects::{browse, failure};
use super::recommendations::Recommendations;
use super::*;
use crate::app::action::*;
use crate::app::state::StationColumn;
use crate::library::station::{RadioSource, Station};
use rand::seq::SliceRandom;
use std::collections::{BTreeSet, HashSet};

fn station(kind: &str, title: &str, category: bool) -> Station {
    Station {
        key: format!("nav-radio/{kind}"),
        title: crate::util::sanitize_display_text(title).into_owned(),
        station_type: if category {
            "station.category"
        } else {
            "station"
        }
        .into(),
        identifier: Some(kind.into()),
        thumb: None,
        art: None,
        description: None,
    }
}
pub fn stations() -> Vec<Station> {
    crate::library::station::standard_stations("nav-radio", crate::library::capabilities::NAVIDROME)
}
pub fn load(state: &mut AppState) {
    let mut stations = stations();
    helpers::append_station_action_items(&mut stations, state.queue.shuffle_undo_queue.is_some());
    state.station_nav.columns = vec![StationColumn::new(
        None,
        "stations".into(),
        stations.clone(),
    )];
    state.station_nav.focused_column = 0;
    state.station_nav.loading = false;
    state.stations_loading = false;
    state.stations = stations;
}
pub fn children(state: &mut AppState, tx: &mpsc::Sender<Event>, key: String, title: String) {
    state.station_navigation_generation = state.station_navigation_generation.wrapping_add(1);
    state.sources.nav_tasks.remove("station-children");
    state.stations_loading = false;
    state.station_nav.loading = false;
    let kind = key.strip_prefix("nav-radio/").unwrap_or("");
    let values: BTreeSet<String> = match kind {
        "mood" => state
            .library
            .albums
            .iter()
            .flat_map(|a| a.mood.iter().map(|g| g.tag.clone()))
            .collect(),
        "style" => state
            .library
            .albums
            .iter()
            .flat_map(|a| a.genre.iter().map(|g| g.tag.clone()))
            .collect(),
        "decade" => state
            .library
            .albums
            .iter()
            .filter_map(|a| {
                a.year
                    .filter(|y| *y > 0)
                    .map(|y| ((y / 10) * 10).to_string())
            })
            .collect(),
        _ => BTreeSet::new(),
    };
    let audio_moods = kind == "mood"
        && crate::app::sources::sonic::enabled(state)
        && crate::app::sources::audiomuse::connection(state).is_some();
    if values.is_empty() && !audio_moods {
        state.set_status(format!("No {kind} tags are available in this library"));
        return;
    }
    let children: Vec<_> = values
        .into_iter()
        .map(|value| {
            let mut child = station(kind, &value, false);
            child.key = format!("{key}?id={}", urlencoding::encode(&value));
            child
        })
        .collect();
    if audio_moods {
        state.stations_loading = true;
        state.station_nav.loading = true;
        let connection = crate::app::sources::audiomuse::connection(state)
            .unwrap()
            .clone();
        let binding =
            crate::app::sources::audiomuse::key(state.sources.active.navidrome().unwrap());
        let navigation_generation = state.station_navigation_generation;
        super::effects::spawn(state, tx, "station-children", async move {
            let result = async {
                let api = crate::app::sources::audiomuse::client(&connection, &binding).await?;
                api.verify().await?;
                api.moods().await
            }
            .await;
            let event = match merge_moods(children, result) {
                Ok(children) => crate::app::event::RadioEvent::StationChildrenLoaded {
                    station_key: key,
                    station_title: title,
                    children,
                },
                Err(error) => crate::app::event::RadioEvent::StationChildrenFailed(failure(error)),
            };
            Ok(vec![super::effects::event(Event::RadioResult {
                generation: 0,
                navigation_generation: Some(navigation_generation),
                event: Box::new(event.into()),
            })])
        });
        return;
    }
    state
        .station_nav
        .push_column(StationColumn::new(Some(key), title, children.clone()));
    state.stations = children;
    if state.palette.open {
        crate::app::command_palette::refresh_matches(state);
    }
}

/// Optional analysis enriches real tag choices; its failure must not erase them.
fn merge_moods(
    mut tags: Vec<Station>,
    analysis: anyhow::Result<Vec<String>>,
) -> anyhow::Result<Vec<Station>> {
    match analysis {
        Ok(moods) => tags.extend(moods.into_iter().map(|mood| {
            let mut child = station("audioMood", &format!("{mood} · AudioMuse"), false);
            child.key = format!("nav-radio/audioMood?id={}", urlencoding::encode(&mood));
            child
        })),
        Err(error) if !tags.is_empty() => {
            tracing::warn!("AudioMuse moods unavailable; retaining catalog mood tags: {error:#}")
        }
        Err(error) => return Err(error),
    }
    Ok(tags)
}

struct Selection {
    tracks: Vec<Track>,
    related: Option<Recommendation>,
    next_decade: Option<usize>,
}
enum Recommendation {
    Seed(String, bool),
    Mood(String),
}
fn choose(state: &AppState, source: &RadioSource, refill: bool) -> anyhow::Result<Selection> {
    use crate::services::radio::{self, AlbumFilter, Recipe};
    let played: HashSet<&str> = if refill {
        state
            .radio
            .tracks
            .iter()
            .map(|t| t.rating_key.as_str())
            .collect()
    } else {
        HashSet::new()
    };
    if let RadioSource::Sonic(key) = source {
        return Ok(Selection {
            tracks: vec![],
            related: Some(Recommendation::Seed(key.clone(), true)),
            next_decade: None,
        });
    }
    if let RadioSource::Artist(key) = source {
        let mut tracks = browse::artist_tracks(state, key);
        tracks.retain(|t| !played.contains(t.rating_key.as_str()));
        tracks.shuffle(&mut rand::rng());
        tracks.truncate(100);
        return Ok(Selection {
            tracks,
            // A performer derived only from compilation tags may have no
            // server artist ID. Its cached tracks still support Artist Radio.
            related: state
                .sources
                .active
                .navidrome()
                .filter(|session| session.id(key).is_ok())
                .map(|_| Recommendation::Seed(key.into(), false)),
            next_decade: None,
        });
    }
    let RadioSource::Station(key) = source else {
        unreachable!()
    };
    let key = key
        .strip_prefix("nav-radio/")
        .ok_or_else(|| anyhow::anyhow!("Invalid Navidrome station"))?;
    let (kind, value) = key.split_once("?id=").unwrap_or((key, ""));
    let value = urlencoding::decode(value)?;
    if kind == "audioMood" {
        anyhow::ensure!(!value.is_empty(), "Choose an AudioMuse mood");
        return Ok(Selection {
            tracks: vec![],
            related: Some(Recommendation::Mood(value.into_owned())),
            next_decade: None,
        });
    }
    let recipe = match kind {
        "artists" => Recipe::Artists(serde_json::from_str(&value)?),
        "library" => Recipe::Library,
        "randomArtist" => Recipe::RandomArtist {
            minimum_tracks: radio::MIN_RANDOM_ARTIST_TRACKS,
        },
        "artistMix" => Recipe::RandomArtist { minimum_tracks: 1 },
        "randomAlbum" | "albumMix" => Recipe::RandomAlbum,
        "deepCuts" => Recipe::DeepCuts,
        "timeTravel" => Recipe::TimeTravel {
            next_decade: if refill {
                state.radio.time_travel_index
            } else {
                0
            },
        },
        "decade" => Recipe::Albums(AlbumFilter::Decade(value.parse()?)),
        "mood" => Recipe::Albums(AlbumFilter::Mood(value.into_owned())),
        "style" => Recipe::Albums(AlbumFilter::Genre(value.into_owned())),
        "onThisDay" => {
            let today = time::OffsetDateTime::now_local()
                .unwrap_or_else(|_| time::OffsetDateTime::now_utc());
            Recipe::Albums(AlbumFilter::Anniversary {
                month: today.month() as u8,
                day: today.day(),
            })
        }
        _ => anyhow::bail!("Unknown Navidrome station"),
    };
    let selected = radio::select(
        &state.library.all_tracks,
        &state.library.albums,
        recipe,
        &played,
    )?;
    let related = selected.tracks.first().and_then(|seed| match kind {
        "albumMix" => Some((seed.rating_key.clone(), true)),
        "artistMix" => seed.grandparent_rating_key.clone().map(|key| (key, false)),
        _ => None,
    });
    Ok(Selection {
        tracks: selected.tracks,
        related: related.map(|(key, sonic)| Recommendation::Seed(key, sonic)),
        next_decade: selected.next_decade,
    })
}

/// A provider-owned snapshot; shared radio orchestration owns cancellation and
/// completion delivery. Only recommendation requests need a catalog ID filter.
pub struct Request {
    selection: anyhow::Result<Selection>,
    recommendations: Option<Recommendations>,
}
impl Request {
    pub fn prepare(
        state: &AppState,
        session: &Session,
        source: &RadioSource,
        refill: bool,
    ) -> Self {
        let selection = choose(state, source, refill);
        let recommendations = selection
            .as_ref()
            .ok()
            .filter(|s| s.related.is_some())
            .map(|_| Recommendations::new(state, session));
        Self {
            selection,
            recommendations,
        }
    }
    pub async fn fetch(self) -> Result<crate::app::sources::radio::Batch, AsyncError> {
        let operation = async {
            let mut selection = self.selection?;
            if let Some(recommendation) = selection.related {
                let api = self
                    .recommendations
                    .as_ref()
                    .expect("recommendation request");
                let similar = match recommendation {
                    Recommendation::Seed(seed, true) => {
                        api.similar(&api.session.id(&seed)?, true).await?
                    }
                    Recommendation::Seed(artist, false) => {
                        let seed = selection
                            .tracks
                            .first()
                            .map(|t| api.session.id(&t.rating_key))
                            .transpose()?;
                        match api.artist(&api.session.id(&artist)?, seed.as_deref()).await {
                            Ok(tracks) => tracks,
                            Err(error) if !selection.tracks.is_empty() => {
                                tracing::warn!("Artist radio: related discovery failed; using selected artists' cached tracks: {error:#}");
                                vec![]
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    Recommendation::Mood(mood) => api.mood(&mood).await?,
                };
                selection.tracks.extend(similar);
                let mut seen = HashSet::new();
                selection
                    .tracks
                    .retain(|t| seen.insert(t.rating_key.clone()));
                selection.tracks.shuffle(&mut rand::rng());
                selection.tracks.truncate(100);
            }
            anyhow::ensure!(!selection.tracks.is_empty(), "No matching tracks");
            Ok(crate::app::sources::radio::Batch {
                tracks: selection.tracks,
                decades: vec![],
                next_decade: selection.next_decade,
            })
        };
        operation.await.map_err(failure)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::models::{Album, GenreTag};
    #[test]
    fn derived_performer_radio_uses_cached_tracks_without_a_server_artist_id() {
        let mut state = AppState::new();
        state
            .library
            .track_artists
            .push(crate::library::models::Artist {
                rating_key: "track_artist:c".into(),
                title: "C".into(),
                ..Default::default()
            });
        state.library.all_tracks.push(Track {
            rating_key: "song".into(),
            original_title: Some("C".into()),
            ..Default::default()
        });
        let selection =
            choose(&state, &RadioSource::Artist("track_artist:c".into()), false).unwrap();
        assert_eq!(selection.tracks[0].rating_key, "song");
        assert!(selection.related.is_none());
    }
    #[test]
    fn unavailable_analysis_preserves_real_mood_choices_but_is_not_empty_success() {
        let tag = station("mood", "Calm", false);
        let result = merge_moods(vec![tag], Err(anyhow::anyhow!("index not ready"))).unwrap();
        assert_eq!(result[0].title, "Calm");
        assert!(merge_moods(vec![], Err(anyhow::anyhow!("index not ready"))).is_err());
        let result = merge_moods(result, Ok(vec!["relaxed".into()])).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].key, "nav-radio/audioMood?id=relaxed");
        assert_eq!(result[1].title, "relaxed · AudioMuse");
    }
    fn fixture() -> AppState {
        let mut state = AppState::new();
        for (album, year, genre) in [("a", 1971, "Rock"), ("b", 1992, "Jazz")] {
            state.library.albums.push(Album {
                rating_key: album.into(),
                year: Some(year),
                genre: vec![GenreTag {
                    tag: genre.into(),
                    ..Default::default()
                }],
                mood: vec![GenreTag {
                    tag: "Calm".into(),
                    ..Default::default()
                }],
                ..Default::default()
            });
            for (disc, track) in [(2, 1), (1, 2), (1, 1)] {
                state.library.all_tracks.push(Track {
                    rating_key: format!("{album}-{disc}-{track}"),
                    parent_rating_key: Some(album.into()),
                    grandparent_rating_key: Some(format!("artist-{album}")),
                    parent_index: Some(disc),
                    index: Some(track),
                    year: Some(year),
                    view_count: Some(if album == "a" { 0 } else { 20 }),
                    ..Default::default()
                });
            }
        }
        state
    }
    fn select(state: &AppState, key: &str, refill: bool) -> Selection {
        choose(
            state,
            &RadioSource::Station(format!("nav-radio/{key}")),
            refill,
        )
        .unwrap()
    }
    #[test]
    fn random_album_is_complete_ordered_and_refill_chooses_another_album() {
        let mut state = fixture();
        let first = select(&state, "randomAlbum", false);
        assert!(first.related.is_none());
        assert_eq!(
            first
                .tracks
                .iter()
                .map(|t| (t.parent_index.unwrap(), t.index.unwrap()))
                .collect::<Vec<_>>(),
            [(1, 1), (1, 2), (2, 1)]
        );
        let album = first.tracks[0].parent_rating_key.clone();
        assert!(first.tracks.iter().all(|t| t.parent_rating_key == album));
        state.radio.tracks = first.tracks;
        let second = select(&state, "randomAlbum", true);
        assert!(second.tracks.iter().all(|t| t.parent_rating_key != album));
    }
    #[test]
    fn metadata_stations_need_neither_network_nor_analysis() {
        let state = fixture();
        for key in [
            "library",
            "deepCuts",
            "timeTravel",
            "decade?id=1990",
            "mood?id=Calm",
            "style?id=Rock",
        ] {
            let result = select(&state, key, false);
            assert!(!result.tracks.is_empty(), "{key}");
            assert!(result.related.is_none(), "{key}");
        }
        assert!(select(&state, "deepCuts", false)
            .tracks
            .iter()
            .all(|t| t.view_count == Some(0)));
        assert!(select(&state, "decade?id=1990", false)
            .tracks
            .iter()
            .all(|t| t.year == Some(1992)));
        assert!(select(&state, "style?id=Rock", false)
            .tracks
            .iter()
            .all(|t| t.parent_rating_key.as_deref() == Some("a")));
    }
    #[test]
    fn time_travel_advances_decades_and_empty_filters_are_errors() {
        let mut state = fixture();
        let first = select(&state, "timeTravel", false);
        assert!(first.tracks.iter().all(|t| t.year == Some(1971)));
        state.radio.time_travel_index = first.next_decade.unwrap();
        state.radio.tracks = first.tracks;
        assert!(select(&state, "timeTravel", true)
            .tracks
            .iter()
            .all(|t| t.year == Some(1992)));
        assert!(choose(
            &state,
            &RadioSource::Station("nav-radio/mood?id=missing".into()),
            false
        )
        .is_err());
        assert!(choose(
            &state,
            &RadioSource::Station("nav-radio/onThisDay".into()),
            false
        )
        .is_err());
    }
}
