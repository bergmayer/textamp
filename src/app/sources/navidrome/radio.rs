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
fn stations() -> Vec<Station> {
    crate::library::station::standard_stations("nav-radio", crate::library::capabilities::NAVIDROME)
}
pub fn load(state: &mut AppState) {
    let today = time::OffsetDateTime::now_local()
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
        .date();
    let sonic = super::recommendations::available(state);
    let mut options = stations();
    let audio = crate::app::sources::audiomuse::connection(state);
    let audio_moods = sonic
        && audio.is_some()
        && state
            .library
            .audio_mood_cache
            .as_ref()
            .filter(|(connection, _)| Some(connection) == audio)
            .and_then(|(_, cache)| cache.get())
            .is_none_or(|pools| !pools.is_empty());
    options.retain(|s| {
        state.library.radio_availability.offers(
            s.identifier.as_deref().unwrap_or(""),
            today,
            sonic,
            audio_moods,
        )
    });
    state.station_navigation_generation = state.station_navigation_generation.wrapping_add(1);
    state.sources.nav_tasks.remove("station-children");
    state.station_children_cache.clear();
    state.station_nav.columns = vec![StationColumn::new(None, "stations".into(), options)];
    state.station_nav.focused_column = 0;
    state.station_nav.loading = false;
}
pub fn children(state: &mut AppState, tx: &mpsc::Sender<Event>, key: String, title: String) {
    state.station_navigation_generation = state.station_navigation_generation.wrapping_add(1);
    state.sources.nav_tasks.remove("station-children");
    state.station_nav.loading = false;
    let kind = key.strip_prefix("nav-radio/").unwrap_or("");
    let values: BTreeSet<String> = match kind {
        "mood" => state.library.radio_availability.moods.clone(),
        "style" => state.library.radio_availability.genres.clone(),
        "decade" => state
            .library
            .radio_availability
            .decades
            .iter()
            .map(u16::to_string)
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
        state.station_nav.loading = true;
        let connection = crate::app::sources::audiomuse::connection(state)
            .unwrap()
            .clone();
        let api = Recommendations::new(state, state.sources.active.navidrome().unwrap());
        let cache = &mut state.library.audio_mood_cache;
        if cache.as_ref().is_none_or(|(bound, _)| bound != &connection) {
            *cache = Some((connection, Default::default()));
        }
        let pools = cache.as_ref().unwrap().1.clone();
        let availability = state.library.radio_availability.clone();
        let navigation_generation = state.station_navigation_generation;
        super::effects::spawn(state, tx, "station-children", async move {
            let result = pools
                .get_or_try_init(|| async {
                    use futures::{StreamExt, TryStreamExt, stream};
                    let client = api.direct().await?;
                    let names = client.moods().await?;
                    let api = &api;
                    let pools = stream::iter(names)
                        .map(|name| async move {
                            let ids = client.mood(&name).await?;
                            let mut seen = HashSet::new();
                            let tracks: Vec<_> = ids
                                .iter()
                                .filter(|id| seen.insert(id.as_str()))
                                .filter_map(|id| api.ranked_candidate(id))
                                .cloned()
                                .collect();
                            Ok::<_, anyhow::Error>((name, tracks))
                        })
                        .buffer_unordered(3)
                        .try_collect::<Vec<_>>()
                        .await?;
                    Ok::<_, anyhow::Error>(
                        pools
                            .into_iter()
                            .filter(|(_, tracks)| availability.supports(tracks))
                            .collect(),
                    )
                })
                .await
                .map(|pools| pools.keys().cloned().collect());
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
    if state.palette.open {
        crate::app::command_palette::refresh_matches(state);
    }
}

type MoodPools = std::collections::BTreeMap<String, Vec<Track>>;
pub(crate) type MoodCache = Option<(
    crate::audiomuse::Connection,
    std::sync::Arc<tokio::sync::OnceCell<MoodPools>>,
)>;
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
    Artist(String),
    SonicTrack(String),
    Mood(String),
    Related(String, bool),
}
fn choose(state: &AppState, source: &RadioSource, refill: bool) -> anyhow::Result<Selection> {
    use crate::services::radio::{self, AlbumFilter, Recipe};
    let empty = HashSet::new();
    let excluded = if refill {
        &state.radio.introduced_albums
    } else {
        &empty
    };
    if let RadioSource::Sonic(key) | RadioSource::SonicFixed(key) | RadioSource::Related(key) =
        source
    {
        return Ok(Selection {
            tracks: vec![],
            related: Some(if matches!(source, RadioSource::Related(_)) {
                Recommendation::Related(key.clone(), super::recommendations::available(state))
            } else {
                Recommendation::SonicTrack(key.clone())
            }),
            next_decade: None,
        });
    }
    if let RadioSource::Artist(key) = source {
        let mut tracks = browse::artist_tracks(state, key);
        tracks.shuffle(&mut rand::rng());
        return Ok(Selection {
            tracks,
            // Only server artist IDs can seed the recommendation endpoint.
            related: state
                .sources
                .active
                .navidrome()
                .filter(|session| session.id(key).is_ok())
                .map(|_| Recommendation::Artist(key.into())),
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
        anyhow::ensure!(
            crate::app::sources::sonic::enabled(state),
            "Sonic features are disabled for this library"
        );
        anyhow::ensure!(!value.is_empty(), "Choose an AudioMuse mood");
        let cached = state
            .library
            .audio_mood_cache
            .as_ref()
            .filter(|(c, _)| Some(c) == crate::app::sources::audiomuse::connection(state))
            .and_then(|(_, cache)| cache.get())
            .and_then(|pools| pools.get(value.as_ref()))
            .cloned();
        if let Some(mut tracks) = cached {
            tracks.shuffle(&mut rand::rng());
            return Ok(Selection {
                tracks,
                related: None,
                next_decade: None,
            });
        }
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
        &state.library.radio_availability,
        excluded,
    )?;
    let related = selected.tracks.first().and_then(|seed| match kind {
        "albumMix" => Some(Recommendation::SonicTrack(seed.rating_key.clone())),
        "artists" | "artistMix" => seed
            .grandparent_rating_key
            .clone()
            .map(Recommendation::Artist),
        _ => None,
    });
    Ok(Selection {
        tracks: selected.tracks,
        related,
        next_decade: selected.next_decade,
    })
}

/// A provider-owned snapshot; shared radio orchestration owns cancellation and
/// completion delivery. Only recommendation requests need a catalog ID filter.
pub struct Request {
    availability: std::sync::Arc<crate::services::radio::Availability>,
    excluded: HashSet<String>,
    work: Work,
    next_decade: Option<usize>,
}

enum Work {
    Cached(Vec<Track>),
    Discover {
        api: Box<Recommendations>,
        seed: Recommendation,
        tracks: Vec<Track>,
    },
}
impl Request {
    pub fn prepare(
        state: &AppState,
        session: &Session,
        source: &RadioSource,
        refill: bool,
    ) -> Result<Self, AsyncError> {
        let Selection {
            tracks,
            related,
            next_decade,
        } = choose(state, source, refill).map_err(failure)?;
        let work = match related {
            None => {
                if (matches!(source, RadioSource::Artist(_))
                    || matches!(source, RadioSource::Station(k) if k.starts_with("nav-radio/artists?")))
                    && !state.library.radio_availability.supports(&tracks)
                {
                    return Err(failure(anyhow::anyhow!(
                        crate::services::radio::SPARSE_STATION
                    )));
                }
                Work::Cached(tracks)
            }
            Some(seed) => Work::Discover {
                api: Box::new(Recommendations::new(state, session)),
                seed,
                tracks,
            },
        };
        Ok(Self {
            work,
            next_decade,
            availability: state.library.radio_availability.clone(),
            excluded: if refill {
                state.radio.introduced_albums.clone()
            } else {
                Default::default()
            },
        })
    }
    pub async fn fetch(self) -> Result<crate::app::sources::radio::Batch, AsyncError> {
        let operation = async {
            let tracks = match self.work {
                Work::Cached(tracks) => tracks,
                Work::Discover {
                    api,
                    seed: recommendation,
                    mut tracks,
                } => {
                    let artist_mix = matches!(recommendation, Recommendation::Artist(_));
                    let mut similar = match recommendation {
                        Recommendation::SonicTrack(seed) => {
                            api.similar(&api.session.id(&seed)?, true).await?
                        }
                        Recommendation::Artist(artist) => {
                            let seed = tracks.first().map(|t| t.rating_key.as_str());
                            match api.artist_mix(&artist, seed).await {
                                Ok(matches) => matches,
                                Err(error) if !tracks.is_empty() => {
                                    tracing::warn!(
                                        "Artist radio: related discovery failed; using selected artists' cached tracks: {error:#}"
                                    );
                                    vec![]
                                }
                                Err(error) => return Err(error),
                            }
                        }
                        Recommendation::Mood(mood) => api.mood(&mood).await?,
                        Recommendation::Related(seed, sonic) => {
                            api.similar(&api.session.id(&seed)?, sonic).await?
                        }
                    };
                    if artist_mix && !tracks.is_empty() {
                        similar.shuffle(&mut rand::rng());
                        tracks = crate::services::radio::artist_mix(tracks, similar);
                    } else {
                        tracks.extend(similar);
                        let mut seen = HashSet::new();
                        tracks.retain(|t| seen.insert(t.rating_key.clone()));
                        tracks.shuffle(&mut rand::rng());
                    }
                    anyhow::ensure!(
                        self.availability.supports(&tracks),
                        crate::services::radio::SPARSE_STATION
                    );
                    tracks
                }
            };
            let tracks = self.availability.batch(&tracks, &self.excluded);
            Ok(crate::app::sources::radio::Batch {
                tracks,
                decades: vec![],
                next_decade: self.next_decade,
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
        std::sync::Arc::make_mut(&mut state.library.all_tracks).push(Track {
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
    #[test]
    fn mood_needs_twenty_distinct_album_artists() {
        let mut state = fixture();
        let albums = std::sync::Arc::make_mut(&mut state.library.albums);
        albums.truncate(20);
        albums[19].parent_rating_key = albums[0].parent_rating_key.clone();
        let check = |albums: &[Album]| {
            crate::services::radio::Availability::build(&state.library.all_tracks, albums)
                .moods
                .contains("Calm")
        };
        assert!(!check(albums));
        albums[19].parent_rating_key = Some("twentieth".into());
        assert!(check(albums));
    }
    fn fixture() -> AppState {
        let mut state = AppState::new();
        for i in 0..240 {
            let album = format!("album-{i}");
            let (year, genre) = if i < 120 {
                (1971, "Rock")
            } else {
                (1992, "Jazz")
            };
            std::sync::Arc::make_mut(&mut state.library.albums).push(Album {
                rating_key: album.clone(),
                parent_rating_key: Some(format!("artist-{i}")),
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
                std::sync::Arc::make_mut(&mut state.library.all_tracks).push(Track {
                    rating_key: format!("{album}-{disc}-{track}"),
                    parent_rating_key: Some(album.clone()),
                    grandparent_rating_key: Some(format!("artist-{album}")),
                    parent_index: Some(disc),
                    index: Some(track),
                    year: Some(year),
                    view_count: Some(if year == 1971 { 0 } else { 20 }),
                    ..Default::default()
                });
            }
        }
        state.library.radio_availability = crate::services::radio::Availability::build(
            &state.library.all_tracks,
            &state.library.albums,
        )
        .into();
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
    fn random_album_refill_skips_every_previously_introduced_album() {
        let mut state = fixture();
        let first = select(&state, "randomAlbum", false);
        state.radio.introduced_albums = first
            .tracks
            .iter()
            .filter_map(|t| t.parent_rating_key.clone())
            .collect();
        assert_eq!(state.radio.introduced_albums.len(), 100);
        let second = select(&state, "randomAlbum", true);
        assert_eq!(second.tracks.len(), 100);
        assert!(second.tracks.iter().all(|t| {
            !state
                .radio
                .introduced_albums
                .contains(t.parent_rating_key.as_ref().unwrap())
        }));
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
        assert!(
            select(&state, "deepCuts", false)
                .tracks
                .iter()
                .all(|t| t.view_count == Some(0))
        );
        assert!(
            select(&state, "decade?id=1990", false)
                .tracks
                .iter()
                .all(|t| t.year == Some(1992))
        );
        assert!(
            select(&state, "style?id=Rock", false)
                .tracks
                .iter()
                .all(|t| t.year == Some(1971))
        );
    }
    #[test]
    fn time_travel_advances_decades_and_empty_filters_are_errors() {
        let mut state = fixture();
        let first = select(&state, "timeTravel", false);
        assert!(first.tracks.iter().all(|t| t.year == Some(1971)));
        state.radio.time_travel_index = first.next_decade.unwrap();
        state.radio.tracks = first.tracks;
        assert!(
            select(&state, "timeTravel", true)
                .tracks
                .iter()
                .all(|t| t.year == Some(1992))
        );
        assert!(
            choose(
                &state,
                &RadioSource::Station("nav-radio/mood?id=missing".into()),
                false
            )
            .is_err()
        );
        assert!(
            choose(
                &state,
                &RadioSource::Station("nav-radio/onThisDay".into()),
                false
            )
            .is_err()
        );
    }
}
