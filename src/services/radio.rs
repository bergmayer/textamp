//! Provider-independent radio selection from catalog metadata. No network,
//! application state, authentication, or assumptions about a complete scan.
use crate::library::catalog::Album;
use crate::library::track::Track;
use anyhow::{ensure, Result};
use rand::prelude::{IndexedRandom, IteratorRandom};
use rand::seq::SliceRandom;
use std::collections::{BTreeSet, HashMap, HashSet};

pub const MIN_RANDOM_ARTIST_TRACKS: usize = 25;

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
fn group<'a>(tracks: &[&'a Track], key: fn(&Track) -> Option<&str>) -> Result<Vec<&'a Track>> {
    let ids: HashSet<_> = tracks.iter().filter_map(|t| key(t)).collect();
    let id = ids
        .iter()
        .choose(&mut rand::rng())
        .ok_or_else(|| anyhow::anyhow!("No unplayed groups remain"))?;
    Ok(tracks
        .iter()
        .copied()
        .filter(|t| key(t) == Some(*id))
        .collect())
}

/// The caller supplies only its active library and exclusions. Random albums
/// remain complete and disc/track ordered; other recipes return at most 100 songs.
pub fn select(
    tracks: &[Track],
    albums: &[Album],
    recipe: Recipe,
    excluded: &HashSet<&str>,
) -> Result<Selection> {
    let available: Vec<_> = tracks
        .iter()
        .filter(|t| !excluded.contains(t.rating_key.as_str()))
        .collect();
    let ordered = matches!(recipe, Recipe::RandomAlbum);
    let mut next_decade = None;
    let mut candidates = match recipe {
        Recipe::Library => available,
        Recipe::Artists(ids) => available
            .into_iter()
            .filter(|t| {
                t.grandparent_rating_key
                    .as_ref()
                    .is_some_and(|k| ids.contains(k))
            })
            .collect(),
        Recipe::RandomArtist { minimum_tracks } => {
            // Count distinct catalog tracks, not just the unplayed remainder.
            // Otherwise a qualifying artist becomes ineligible during refill.
            let mut catalogs: HashMap<&str, HashSet<&str>> = HashMap::new();
            for track in tracks {
                if let Some(artist) = track
                    .grandparent_rating_key
                    .as_deref()
                    .filter(|k| !k.is_empty())
                {
                    if !track.rating_key.is_empty() {
                        catalogs
                            .entry(artist)
                            .or_default()
                            .insert(&track.rating_key);
                    }
                }
            }
            let eligible: Vec<_> = available
                .into_iter()
                .filter(|t| {
                    t.grandparent_rating_key
                        .as_deref()
                        .and_then(|id| catalogs.get(id))
                        .is_some_and(|catalog| catalog.len() >= minimum_tracks)
                })
                .collect();
            ensure!(
                !eligible.is_empty(),
                "No unplayed artists with at least {minimum_tracks} tracks in this library"
            );
            group(&eligible, |t| t.grandparent_rating_key.as_deref())?
        }
        Recipe::RandomAlbum => group(&available, |t| t.parent_rating_key.as_deref())?,
        Recipe::DeepCuts => {
            let mut counts: Vec<_> = available.iter().filter_map(|t| t.view_count).collect();
            ensure!(
                !counts.is_empty(),
                "Play counts unavailable; refresh the library with F5"
            );
            counts.sort_unstable();
            let cutoff = counts[counts.len() / 4];
            available
                .into_iter()
                .filter(|t| t.view_count.is_some_and(|n| n <= cutoff))
                .collect()
        }
        Recipe::Albums(filter) => album_tracks(&available, albums, filter),
        Recipe::TimeTravel { next_decade: start } => {
            let decades: BTreeSet<_> = albums
                .iter()
                .filter_map(|a| a.year.filter(|y| *y > 0).map(|y| y / 10 * 10))
                .collect();
            let decades: Vec<_> = decades.into_iter().collect();
            ensure!(!decades.is_empty(), "No release years in this library");
            let mut selected = Vec::new();
            // Keep indices in the full decade list, skipping exhausted decades.
            for offset in 0..decades.len() {
                let index = (start % decades.len() + offset) % decades.len();
                selected = album_tracks(&available, albums, AlbumFilter::Decade(decades[index]));
                if !selected.is_empty() {
                    next_decade = Some((index + 1) % decades.len());
                    break;
                }
            }
            selected
        }
    };
    ensure!(
        !candidates.is_empty(),
        "No matching music for this station (check library tags or refresh with F5)"
    );
    let tracks = if ordered {
        candidates.sort_by(|a, b| {
            (a.parent_index.unwrap_or(1), a.index.unwrap_or(0), &a.title).cmp(&(
                b.parent_index.unwrap_or(1),
                b.index.unwrap_or(0),
                &b.title,
            ))
        });
        candidates.into_iter().cloned().collect()
    } else {
        let mut rng = rand::rng();
        let mut selected: Vec<_> = candidates
            .choose_multiple(&mut rng, 100)
            .map(|t| (*t).clone())
            .collect();
        selected.shuffle(&mut rng);
        selected
    };
    Ok(Selection {
        tracks,
        next_decade,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn random_artist_counts_unique_catalog_tracks_before_excluding_played_tracks() {
        let mut tracks = Vec::new();
        for (artist, count) in [("small", 24), ("eligible", 25)] {
            tracks.extend((0..count).map(|i| Track {
                rating_key: format!("{artist}-{i}"),
                grandparent_rating_key: Some(artist.into()),
                ..Default::default()
            }));
        }
        tracks.extend(tracks[..24].to_vec()); // duplicates cannot qualify small artists
        let recipe = || Recipe::RandomArtist {
            minimum_tracks: MIN_RANDOM_ARTIST_TRACKS,
        };
        let result = select(&tracks, &[], recipe(), &HashSet::new()).unwrap();
        assert_eq!(result.tracks.len(), 25);
        assert!(result
            .tracks
            .iter()
            .all(|t| t.grandparent_rating_key.as_deref() == Some("eligible")));
        let played = tracks
            .iter()
            .filter(|t| t.rating_key != "eligible-24")
            .map(|t| t.rating_key.as_str())
            .collect();
        let result = select(&tracks, &[], recipe(), &played).unwrap();
        assert_eq!(result.tracks.len(), 1);
        assert_eq!(result.tracks[0].rating_key, "eligible-24");
        assert!(select(&tracks[..24], &[], recipe(), &HashSet::new())
            .err()
            .unwrap()
            .to_string()
            .contains("25"));
        // Explicit artist selection is not subject to the random-station minimum.
        assert_eq!(
            select(
                &tracks[..24],
                &[],
                Recipe::Artists(HashSet::from(["small".into()])),
                &HashSet::new()
            )
            .unwrap()
            .tracks
            .len(),
            24
        );
    }
    #[test]
    fn a_folder_track_can_participate_without_server_ids_or_tags() {
        let tracks: Vec<_> = ["1.flac", "2.flac"]
            .into_iter()
            .map(|p| Track::from_folder("local", p))
            .collect();
        let selected = select(&tracks, &[], Recipe::Library, &HashSet::new()).unwrap();
        assert_eq!(selected.tracks.len(), 2);
        assert!(select(
            &tracks,
            &[],
            Recipe::Albums(AlbumFilter::Mood("Calm".into())),
            &HashSet::new()
        )
        .is_err());
    }
    #[test]
    fn time_travel_skips_exhausted_decades_without_skipping_available_ones() {
        let albums: Vec<_> = [1970, 1980, 1990]
            .into_iter()
            .map(|year| Album {
                rating_key: year.to_string(),
                year: Some(year),
                ..Default::default()
            })
            .collect();
        let tracks: Vec<_> = albums
            .iter()
            .map(|a| Track {
                rating_key: a.rating_key.clone(),
                parent_rating_key: Some(a.rating_key.clone()),
                ..Default::default()
            })
            .collect();
        let excluded = HashSet::from(["1970", "1980"]);
        let picked = select(
            &tracks,
            &albums,
            Recipe::TimeTravel { next_decade: 1 },
            &excluded,
        )
        .unwrap();
        assert_eq!(picked.tracks[0].rating_key, "1990");
        assert_eq!(picked.next_decade, Some(0));
    }
}
