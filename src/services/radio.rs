//! Provider-independent radio selection from catalog metadata. No network,
//! application state, authentication, or assumptions about a complete scan.
use crate::library::catalog::Album;
use crate::library::track::Track;
use anyhow::{Result, ensure};

use rand::seq::SliceRandom;
use std::collections::{BTreeSet, HashMap, HashSet};

pub const MIN_RANDOM_ARTIST_TRACKS: usize = 25;

pub const MIN_STATION_ARTISTS: usize = 20;
pub const SPARSE_STATION: &str = "Radio requires 20 playable albums by 20 distinct album artists";

#[derive(Debug, Default, Clone)]
pub struct Availability {
    pub moods: BTreeSet<String>,
    pub genres: BTreeSet<String>,
    pub decades: BTreeSet<u16>,
    anniversaries: HashSet<String>,
    owners: HashMap<String, String>,
    library: bool,
    deep_cuts: bool,
    random_artists: bool,
    artist_counts: HashMap<String, usize>,
    deep_cut_limit: Option<u64>,
}

fn cutoff(tracks: &[Track]) -> Option<u64> {
    let mut counts: Vec<_> = tracks.iter().filter_map(|t| t.view_count).collect();
    let index = counts.len() / 4;
    (!counts.is_empty()).then(|| *counts.select_nth_unstable(index).1)
}

impl Availability {
    pub fn build(tracks: &[Track], albums: &[Album]) -> Self {
        let mut result = Self::default();
        let mut playable = HashSet::new();
        let mut low = HashSet::new();
        let mut artist_counts = HashMap::new();
        let mut seen = HashSet::new();
        let cutoff = cutoff(tracks);
        for t in tracks {
            let Some(album) = t.parent_rating_key.as_deref().filter(|s| !s.is_empty()) else {
                continue;
            };
            if t.rating_key.is_empty() || !seen.insert(&t.rating_key) {
                continue;
            }
            playable.insert(album);
            if t.view_count.zip(cutoff).is_some_and(|(n, c)| n <= c) {
                low.insert(album);
            }
            if let Some(artist) = t.grandparent_rating_key.as_ref().filter(|s| !s.is_empty()) {
                *artist_counts.entry(artist).or_insert(0usize) += 1;
            }
        }
        let mut genres = HashMap::new();
        let mut moods = HashMap::new();
        let mut decades = HashMap::new();
        let mut days = HashMap::new();
        let mut distinct = HashSet::new();
        let mut deep = HashSet::new();
        let mut named_ids = HashMap::new();
        for a in albums {
            if let (Some(name), Some(id)) = (
                a.parent_title.as_deref(),
                a.parent_rating_key
                    .as_deref()
                    .filter(|id| !id.trim().is_empty()),
            ) {
                named_ids.entry(name.trim().to_lowercase()).or_insert(id);
            }
        }
        let owners = &mut result.owners;
        for album in albums {
            if !playable.remove(album.rating_key.as_str()) {
                continue;
            }
            let owner = album
                .parent_rating_key
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .map(|id| format!("id:{id}"))
                .or_else(|| {
                    album
                        .parent_title
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|s| {
                            let name = s.to_lowercase();
                            named_ids
                                .get(&name)
                                .map_or_else(|| format!("name:{name}"), |id| format!("id:{id}"))
                        })
                });
            let Some(owner) = owner else { continue };
            distinct.insert(owner.clone());
            if low.contains(album.rating_key.as_str()) {
                deep.insert(owner.clone());
            }
            for (tags, groups) in [(&album.genre, &mut genres), (&album.mood, &mut moods)] {
                for tag in tags.iter().filter(|g| !g.tag.trim().is_empty()) {
                    groups
                        .entry(tag.tag.clone())
                        .or_insert_with(HashSet::new)
                        .insert(owner.clone());
                }
            }
            if let Some(y) = album.year.filter(|y| *y > 0) {
                decades
                    .entry(y / 10 * 10)
                    .or_insert_with(HashSet::new)
                    .insert(owner.clone());
            }
            if let Some(d) = album
                .originally_available_at
                .as_deref()
                .and_then(|s| s.get(s.len().checked_sub(5)?..))
            {
                days.entry(d.to_owned())
                    .or_insert_with(HashSet::new)
                    .insert(owner.clone());
            }
            owners.insert(album.rating_key.clone(), owner);
        }
        fn enough<K>(sets: HashMap<K, HashSet<String>>) -> impl Iterator<Item = K> {
            sets.into_iter()
                .filter_map(|(k, v)| (v.len() >= MIN_STATION_ARTISTS).then_some(k))
        }
        result.moods = enough(moods).collect();
        result.genres = enough(genres).collect();
        result.decades = enough(decades).collect();
        result.anniversaries = enough(days).collect();
        result.library = distinct.len() >= MIN_STATION_ARTISTS;
        result.deep_cuts = deep.len() >= MIN_STATION_ARTISTS;
        result.random_artists = result.supports(tracks.iter().filter(|t| {
            t.grandparent_rating_key
                .as_ref()
                .and_then(|k| artist_counts.get(k))
                .is_some_and(|n| *n >= MIN_RANDOM_ARTIST_TRACKS)
        }));
        result.artist_counts = artist_counts
            .into_iter()
            .map(|(k, v)| (k.clone(), v))
            .collect();
        result.deep_cut_limit = cutoff;
        result
    }

    pub fn supports<'a>(&self, tracks: impl IntoIterator<Item = &'a Track>) -> bool {
        let mut artists = HashSet::new();
        tracks.into_iter().any(|t| {
            !t.rating_key.is_empty()
                && t.parent_rating_key
                    .as_ref()
                    .and_then(|a| self.owners.get(a))
                    .is_some_and(|owner| {
                        artists.insert(owner) && artists.len() >= MIN_STATION_ARTISTS
                    })
        })
    }

    pub fn offers(&self, id: &str, day: time::Date, sonic: bool, audio_moods: bool) -> bool {
        match id {
            "randomAlbum" | "library" | "artist-picker" => self.library,
            "randomArtist" => self.random_artists,
            "deepCuts" => self.deep_cuts,
            "timeTravel" | "decade" => !self.decades.is_empty(),
            "onThisDay" => {
                self.anniversaries
                    .contains(&format!("{:02}-{:02}", day.month() as u8, day.day()))
            }
            "mood" => !self.moods.is_empty() || (self.library && audio_moods),
            "style" => !self.genres.is_empty(),
            "sonic" | "stayClose" => self.library && sonic,
            _ => false,
        }
    }

    /// Input order supplies the random or recommendation ranking.
    pub fn batch<'a>(
        &self,
        tracks: impl IntoIterator<Item = &'a Track>,
        excluded: &HashSet<String>,
    ) -> Vec<Track> {
        let mut albums = HashSet::new();
        let mut artists = HashSet::new();
        let mut first = Vec::new();
        let mut rest = Vec::new();
        for t in tracks {
            let Some(album) = t.parent_rating_key.as_ref() else {
                continue;
            };
            let Some(owner) = self.owners.get(album) else {
                continue;
            };
            if t.rating_key.is_empty() || excluded.contains(album) || !albums.insert(album) {
                continue;
            }
            if artists.insert(owner) {
                first.push(t);
            } else if rest.len() < 100 {
                rest.push(t);
            }
            if first.len() == 100 {
                break;
            }
        }
        first.into_iter().chain(rest).take(100).cloned().collect()
    }
}

pub enum AlbumFilter {
    Decade(u16),
    Mood(String),
    Genre(String),
    Anniversary { month: u8, day: u8 },
}
impl AlbumFilter {
    fn matches(&self, album: &Album) -> bool {
        match self {
            Self::Decade(decade) => album.year.is_some_and(|y| y > 0 && y / 10 * 10 == *decade),
            Self::Mood(value) => album.mood.iter().any(|g| g.tag == *value),
            Self::Genre(value) => album.genre.iter().any(|g| g.tag == *value),
            Self::Anniversary { month, day } => album
                .originally_available_at
                .as_deref()
                .is_some_and(|date| date.ends_with(&format!("-{month:02}-{day:02}"))),
        }
    }
}
pub enum Recipe {
    Library,
    Artists(HashSet<String>),
    RandomArtist { minimum_tracks: usize },
    RandomAlbum,
    DeepCuts,
    TimeTravel { next_decade: usize },
    Albums(AlbumFilter),
}
pub struct Selection {
    pub tracks: Vec<Track>,
    pub next_decade: Option<usize>,
}

fn album_tracks<'a>(tracks: &[&'a Track], albums: &[Album], filter: AlbumFilter) -> Vec<&'a Track> {
    let ids: HashSet<_> = albums
        .iter()
        .filter(|a| filter.matches(a))
        .map(|a| a.rating_key.as_str())
        .collect();
    tracks
        .iter()
        .copied()
        .filter(|t| {
            t.parent_rating_key
                .as_deref()
                .is_some_and(|k| ids.contains(k))
        })
        .collect()
}
pub fn select(
    tracks: &[Track],
    albums: &[Album],
    recipe: Recipe,
    availability: &Availability,
    excluded: &HashSet<String>,
) -> Result<Selection> {
    let available: Vec<_> = tracks.iter().collect();
    let random_albums = matches!(recipe, Recipe::RandomAlbum);
    let artist_seed = matches!(recipe, Recipe::Artists(_));
    let mut next_decade = None;
    let mut candidates = match recipe {
        Recipe::Library | Recipe::RandomAlbum => available,
        Recipe::Artists(ids) => available
            .into_iter()
            .filter(|t| {
                t.grandparent_rating_key
                    .as_ref()
                    .is_some_and(|k| ids.contains(k))
            })
            .collect(),
        Recipe::RandomArtist { minimum_tracks } => {
            let eligible: Vec<_> = available
                .into_iter()
                .filter(|t| {
                    t.grandparent_rating_key
                        .as_deref()
                        .and_then(|id| availability.artist_counts.get(id))
                        .is_some_and(|count| *count >= minimum_tracks)
                })
                .collect();
            ensure!(
                !eligible.is_empty(),
                "No unplayed artists with at least {minimum_tracks} tracks in this library"
            );
            eligible
        }
        Recipe::DeepCuts => {
            let cutoff = availability
                .deep_cut_limit
                .ok_or_else(|| anyhow::anyhow!("Play counts unavailable"))?;
            available
                .into_iter()
                .filter(|t| t.view_count.is_some_and(|n| n <= cutoff))
                .collect()
        }
        Recipe::Albums(filter) => album_tracks(&available, albums, filter),
        Recipe::TimeTravel { next_decade: start } => {
            let decades: Vec<_> = availability.decades.iter().copied().collect();
            ensure!(!decades.is_empty(), "No release years in this library");
            let mut selected = Vec::new();
            // Keep indices in the full decade list, skipping exhausted decades.
            for offset in 0..decades.len() {
                let index = (start % decades.len() + offset) % decades.len();
                selected = album_tracks(&available, albums, AlbumFilter::Decade(decades[index]));
                if selected.iter().any(|t| {
                    t.parent_rating_key.as_ref().is_some_and(|a| {
                        availability.owners.contains_key(a) && !excluded.contains(a)
                    })
                }) {
                    next_decade = Some((index + 1) % decades.len());
                    break;
                }
            }
            selected
        }
    };
    ensure!(
        !candidates.is_empty(),
        "No matching music for this station (check library tags or choose Refresh Library)"
    );
    candidates.shuffle(&mut rand::rng());
    if random_albums {
        let mut albums = HashSet::new();
        candidates.retain(|t| {
            t.parent_rating_key
                .as_ref()
                .is_some_and(|k| albums.insert(k))
        });
        candidates.shuffle(&mut rand::rng());
    }
    if !artist_seed {
        ensure!(
            availability.supports(candidates.iter().copied()),
            SPARSE_STATION
        );
    }
    let tracks = if artist_seed {
        candidates.into_iter().cloned().collect()
    } else {
        availability.batch(candidates, excluded)
    };
    Ok(Selection {
        tracks,
        next_decade,
    })
}

/// Insert one discovery after every two seed tracks. Input order represents
/// the provider's shuffled candidates. A batch always starts with its seed.
pub(crate) fn artist_mix(primary: Vec<Track>, related: Vec<Track>) -> Vec<Track> {
    let mut seen = HashSet::new();
    let primary: Vec<_> = primary
        .into_iter()
        .filter(|track| seen.insert(track.rating_key.clone()))
        .collect();
    let mut related = related
        .into_iter()
        .filter(|track| seen.insert(track.rating_key.clone()));
    let mut mix = Vec::new();
    for (index, track) in primary.into_iter().enumerate() {
        mix.push(track);
        if index % 2 == 1
            && let Some(track) = related.next()
        {
            mix.push(track);
        }
    }
    mix.extend(related);
    mix
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artist_mix_starts_with_the_chosen_artist_and_keeps_a_unique_majority() {
        let track = |id: &str| Track {
            rating_key: id.into(),
            ..Default::default()
        };
        let primary = ["a", "b", "c", "c", "d"].map(track).to_vec();
        let related = ["b", "e", "e", "f", "g"].map(track).to_vec();
        let ids: Vec<_> = artist_mix(primary, related)
            .into_iter()
            .map(|t| t.rating_key)
            .collect();
        assert_eq!(ids, ["a", "b", "e", "c", "d", "f", "g"]);
    }
}
