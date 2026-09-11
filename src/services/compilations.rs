//! Catalog-derived compilation browsing. No provider calls or independent task state.

use std::collections::{HashMap, HashSet};

use super::artist_alias_service::{build_artist_lookup, compute_aliases, normalize_artist_name};
use crate::library::models::{Album, Artist, Track};

#[derive(Debug, Clone, Default)]
pub struct CompilationIndex {
    pub albums: Vec<Album>,
    /// Artists with compilation appearances but no ordinary albums.
    pub artist_keys: HashSet<String>,
    pub artist_map: HashMap<String, Vec<String>>,
    pub single_artist: HashMap<String, Vec<Album>>,
}

#[derive(Debug, Clone, Default)]
pub struct ArtistIndex {
    pub compilations: CompilationIndex,
    pub track_artists: Vec<Artist>,
    pub aliases: HashMap<String, HashSet<String>>,
    pub album_display: HashMap<String, String>,
}

fn performer_name(track: &Track) -> Option<&str> {
    track
        .original_title
        .as_deref()
        .map(str::trim)
        .filter(|name| {
            !name.is_empty()
                && !["Unknown Artist", "Various Artists", "Various"]
                    .iter()
                    .any(|placeholder| name.eq_ignore_ascii_case(placeholder))
        })
}

impl ArtistIndex {
    pub fn build(artists: &[Artist], albums: &[Album], tracks: &[Track]) -> Self {
        let (aliases, album_display) = compute_aliases(tracks, albums);
        let mut result = Self {
            aliases,
            album_display,
            ..Self::default()
        };
        let mut lookup = build_artist_lookup(artists, &result.aliases);
        let by_key: HashMap<_, _> = artists.iter().map(|a| (a.rating_key.as_str(), a)).collect();
        let mut performers = HashMap::new();
        let mut album_tracks: HashMap<&str, Vec<&Track>> = HashMap::new();
        for track in tracks {
            if let Some(key) = track.parent_rating_key.as_deref() {
                album_tracks.entry(key).or_default().push(track);
            }
            let name = track.track_artist().trim();
            if name.is_empty() || name == "Unknown Artist" {
                continue;
            }
            let normalized = normalize_artist_name(name);
            let key = lookup.entry(normalized).or_insert_with(|| {
                // A track's provider ID can refer to an album artist or the first
                // credited performer. Never reuse it for a different name.
                format!("track_artist:{}", name.to_lowercase())
            });
            performers.entry(key.clone()).or_insert_with(|| {
                by_key
                    .get(key.as_str())
                    .map(|a| (*a).clone())
                    .unwrap_or_else(|| Artist {
                        rating_key: key.clone(),
                        title: name.to_owned(),
                        ..Artist::default()
                    })
            });
        }
        result.track_artists = performers.into_values().collect();
        result
            .track_artists
            .sort_by_key(|a| normalize_artist_name(&a.title));

        let mut ordinary_owners = HashSet::new();
        for album in albums {
            // Guest credits alone do not make an album a compilation.
            if !album.is_compilation_candidate() {
                ordinary_owners.extend(album.parent_rating_key.iter().cloned());
                continue;
            }
            let tracks = album_tracks.get(album.rating_key.as_str());
            let names: HashSet<_> = tracks
                .into_iter()
                .flatten()
                .filter_map(|t| performer_name(t))
                .map(normalize_artist_name)
                .collect();
            let complete = tracks.is_some_and(|tracks| {
                !tracks.is_empty()
                    && album
                        .leaf_count
                        .is_none_or(|count| count as usize == tracks.len())
                    && tracks.iter().all(|t| performer_name(t).is_some())
            });
            let sole_artist = if complete && names.len() == 1 {
                names.iter().next().and_then(|name| lookup.get(name))
            } else {
                None
            };
            if names.len() > 1 {
                result.compilations.albums.push(album.clone());
                if let Some(owner) = &album.parent_rating_key {
                    result.compilations.artist_keys.insert(owner.clone());
                }
                for name in names {
                    if let Some(key) = lookup.get(&name) {
                        result.compilations.artist_keys.insert(key.clone());
                        result
                            .compilations
                            .artist_map
                            .entry(key.clone())
                            .or_default()
                            .push(album.rating_key.clone());
                    }
                }
            } else if let Some(key) = sole_artist {
                let key = key.clone();
                if let Some(track) = tracks.and_then(|tracks| tracks.first()) {
                    result.album_display.insert(
                        album.rating_key.clone(),
                        track.track_artist().trim().to_owned(),
                    );
                }
                ordinary_owners.insert(key.clone());
                result
                    .compilations
                    .single_artist
                    .entry(key)
                    .or_default()
                    .push(album.clone());
                if let Some(owner) = &album.parent_rating_key {
                    result.compilations.artist_keys.insert(owner.clone());
                }
            } else if let Some(owner) = &album.parent_rating_key {
                // Incomplete albums keep their server ownership.
                ordinary_owners.insert(owner.clone());
            }
        }
        result
            .compilations
            .artist_keys
            .retain(|key| !ordinary_owners.contains(key));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artist(name: &str) -> Artist {
        Artist {
            rating_key: name.into(),
            title: name.into(),
            ..Default::default()
        }
    }
    fn album(key: &str, owner: &str, compilation: bool, count: u32) -> Album {
        Album {
            rating_key: key.into(),
            parent_rating_key: Some(owner.into()),
            parent_title: Some(owner.into()),
            leaf_count: Some(count),
            subtype: compilation.then(|| "compilation".into()),
            ..Default::default()
        }
    }
    fn track(album: &str, name: &str) -> Track {
        Track {
            parent_rating_key: Some(album.into()),
            original_title: Some(name.into()),
            ..Default::default()
        }
    }

    #[test]
    fn tagged_and_various_artist_compilations_group_appearances_without_hiding_solo_artists() {
        let artists = [artist("A"), artist("B"), artist("Various Artists")];
        let albums = [
            album("solo", "A", false, 1),
            album("tagged", "A", true, 2),
            album("va", "Various Artists", false, 2),
        ];
        let tracks = [
            track("solo", "A"),
            track("tagged", "A"),
            track("tagged", "B"),
            track("va", "A"),
            track("va", "B"),
        ];
        let result = ArtistIndex::build(&artists, &albums, &tracks);
        assert_eq!(result.compilations.albums.len(), 2);
        assert_eq!(result.compilations.artist_map["B"], ["tagged", "va"]);
        assert_eq!(
            result.compilations.artist_keys,
            HashSet::from(["B".into(), "Various Artists".into()])
        );
        assert!(result.aliases.is_empty());
    }

    #[test]
    fn guest_credits_do_not_make_an_ordinary_album_a_compilation() {
        let result = ArtistIndex::build(
            &[artist("A"), artist("B")],
            &[album("duet", "A", false, 2)],
            &[track("duet", "A"), track("duet", "B")],
        );
        assert!(result.compilations.albums.is_empty());
        assert!(result.compilations.artist_keys.is_empty());
    }

    #[test]
    fn single_artist_collections_are_reassigned_only_with_complete_performer_data() {
        let artists = [artist("The A"), artist("Various Artists")];
        let albums = [album("hits", "Various Artists", true, 2)];
        let tracks = [track("hits", "A"), track("hits", "The A")];
        let result = ArtistIndex::build(&artists, &albums, &tracks);
        assert_eq!(
            result.compilations.single_artist["The A"][0].rating_key,
            "hits"
        );
        assert!(result.compilations.albums.is_empty());
        assert!(!result.compilations.artist_keys.contains("The A"));
        assert!(result.compilations.artist_keys.contains("Various Artists"));
        assert_eq!(result.album_display["hits"], "A");

        for tracks in [
            vec![tracks[0].clone()],
            vec![tracks[0].clone(), track("hits", "")],
            vec![tracks[0].clone(), track("hits", "Unknown Artist")],
            vec![tracks[0].clone(), track("hits", "Various Artists")],
        ] {
            let result = ArtistIndex::build(&artists, &albums, &tracks);
            assert!(result.compilations.single_artist.is_empty());
            assert!(result.compilations.artist_keys.is_empty());
        }
    }

    #[test]
    fn performer_names_never_borrow_another_artists_id() {
        let artists = [artist("Various Artists")];
        let mut tracks = [track("comp", "A"), track("comp", "B")];
        for t in &mut tracks {
            t.grandparent_rating_key = Some("Various Artists".into());
        }
        let result = ArtistIndex::build(
            &artists,
            &[album("comp", "Various Artists", true, 2)],
            &tracks,
        );
        assert_eq!(result.track_artists.len(), 2);
        assert!(result
            .compilations
            .artist_map
            .contains_key("track_artist:a"));
        assert!(result
            .compilations
            .artist_map
            .contains_key("track_artist:b"));
        assert!(!result
            .compilations
            .artist_map
            .contains_key("Various Artists"));
    }

    #[test]
    fn missing_or_placeholder_performers_do_not_panic_or_reassign() {
        for name in ["", "Unknown Artist", "Various Artists"] {
            let result = ArtistIndex::build(
                &[],
                &[album("x", "Various Artists", true, 1)],
                &[track("x", name)],
            );
            assert!(result.compilations.single_artist.is_empty());
        }
    }
}
