//! Catalog-derived compilation browsing. No provider calls or independent task state.

use std::collections::{HashMap, HashSet};

use super::artist_alias_service::{build_artist_lookup, compute_aliases, normalize_artist_name};
use crate::library::models::{Album, Artist, Track};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CompilationIndex {
    pub albums: Vec<Album>,
    /// Album excerpts mapped to the complete release after matching song identities.
    #[serde(default)]
    pub complete_releases: HashMap<String, String>,
    /// Provider album records that jointly contain an explicitly named compilation.
    #[serde(default)]
    pub release_members: HashMap<String, Vec<String>>,
    /// Artists with compilation appearances but no ordinary albums.
    pub artist_keys: HashSet<String>,
    pub artist_map: HashMap<String, Vec<String>>,
    pub single_artist: HashMap<String, Vec<Album>>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
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

fn song_identity(track: &Track) -> Option<(String, String)> {
    let performer = performer_name(track)?.to_lowercase();
    let title = track.title.trim().to_lowercase();
    let prefix = format!("{performer} - ");
    let title = title.strip_prefix(&prefix).unwrap_or(&title);
    Some((
        normalize_artist_name(&performer),
        normalize_artist_name(title),
    ))
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
            .sort_by_cached_key(|a| normalize_artist_name(&a.title));

        // A split release is recognized by both its title and contained songs;
        // unrelated collections with a common title remain independent albums.
        let mut compilation_songs: HashMap<String, HashSet<(String, String)>> = HashMap::new();
        for album in albums
            .iter()
            .filter(|album| album.is_compilation_candidate())
        {
            if let Some(tracks) = album_tracks
                .get(album.rating_key.as_str())
                .filter(|tracks| {
                    tracks
                        .iter()
                        .filter_map(|track| performer_name(track))
                        .map(normalize_artist_name)
                        .collect::<HashSet<_>>()
                        .len()
                        > 1
                })
            {
                compilation_songs
                    .entry(album.title.trim().to_lowercase())
                    .or_default()
                    .extend(tracks.iter().filter_map(|track| song_identity(track)));
            }
        }
        let mut releases: HashMap<String, Vec<&Album>> = HashMap::new();
        for album in albums
            .iter()
            .filter(|album| album.is_compilation_candidate())
        {
            releases
                .entry(album.title.trim().to_lowercase())
                .or_default()
                .push(album);
        }
        for group in releases.values() {
            for excerpt in group {
                let Some(parts) = album_tracks
                    .get(excerpt.rating_key.as_str())
                    .filter(|parts| !parts.is_empty())
                else {
                    continue;
                };
                let canonical = group
                    .iter()
                    .filter(|candidate| {
                        album_tracks
                            .get(candidate.rating_key.as_str())
                            .is_some_and(|whole| {
                                whole.len() > parts.len()
                                    && parts.iter().all(|track| {
                                        song_identity(track).is_some_and(|identity| {
                                            whole.iter().any(|song| {
                                                song_identity(song).as_ref() == Some(&identity)
                                            })
                                        })
                                    })
                            })
                    })
                    .max_by_key(|candidate| {
                        album_tracks
                            .get(candidate.rating_key.as_str())
                            .map_or(0, Vec::len)
                    });
                if let Some(canonical) = canonical {
                    result
                        .compilations
                        .complete_releases
                        .insert(excerpt.rating_key.clone(), canonical.rating_key.clone());
                }
            }
        }
        for group in releases.values().filter(|group| group.len() > 1) {
            if !group.iter().all(|album| album.has_compilation_title()) {
                continue;
            }
            let years: HashSet<_> = group.iter().filter_map(|album| album.year).collect();
            if years.len() > 1 {
                continue;
            }
            let canonical = group
                .iter()
                .max_by_key(|album| {
                    (
                        matches!(
                            album.artist_name().trim().to_lowercase().as_str(),
                            "various artists" | "various"
                        ),
                        album.leaf_count.unwrap_or(0),
                        &album.rating_key,
                    )
                })
                .unwrap();
            let mut members: Vec<_> = group.iter().map(|album| album.rating_key.clone()).collect();
            members.sort();
            result
                .compilations
                .release_members
                .insert(canonical.rating_key.clone(), members);
            for album in group {
                if album.rating_key != canonical.rating_key {
                    result
                        .compilations
                        .complete_releases
                        .insert(album.rating_key.clone(), canonical.rating_key.clone());
                }
            }
        }
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
            // A single cached performer does not establish a solo release:
            // Various Artists albums can contain only one song from that artist.
            // Reassign a collection only when its album credit agrees as well.
            let sole_artist = if !album.has_compilation_title() && complete && names.len() == 1 {
                names
                    .iter()
                    .next()
                    .filter(|name| **name == normalize_artist_name(album.artist_name()))
                    .and_then(|name| lookup.get(name))
            } else {
                None
            };
            if sole_artist.is_none()
                || compilation_songs
                    .get(&album.title.trim().to_lowercase())
                    .is_some_and(|songs| {
                        tracks.is_some_and(|tracks| {
                            !tracks.is_empty()
                                && tracks.iter().all(|track| {
                                    song_identity(track)
                                        .is_some_and(|identity| songs.contains(&identity))
                                })
                        })
                    })
            {
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
    fn single_performer_excerpt_of_a_multi_artist_release_remains_a_compilation() {
        let artists = [artist("Kind"), artist("B"), artist("Various Artists")];
        let mut whole = album("whole", "Various Artists", true, 2);
        whole.title = "the Compilation CD".into();
        let mut excerpt = album("excerpt", "Various Artists", true, 1);
        excerpt.title = "The compilation cd".into();
        let result = ArtistIndex::build(
            &artists,
            &[whole, excerpt],
            &[
                track("whole", "Kind"),
                track("whole", "B"),
                track("excerpt", "Kind"),
            ],
        );
        assert_eq!(result.compilations.albums.len(), 2);
        assert!(result.compilations.artist_keys.contains("Kind"));
        assert!(!result.compilations.single_artist.contains_key("Kind"));
        assert_eq!(result.compilations.artist_map["Kind"], ["whole", "excerpt"]);
        assert_eq!(result.compilations.complete_releases["excerpt"], "whole");
    }
    #[test]
    fn unrelated_single_artist_collection_with_the_same_title_is_not_an_excerpt() {
        let artists = [artist("A"), artist("B"), artist("Various Artists")];
        let mut compilation = album("comp", "Various Artists", true, 2);
        compilation.title = "Greatest Hits".into();
        let mut solo = album("solo", "A", true, 1);
        solo.title = "Greatest Hits".into();
        let mut a = track("comp", "A");
        a.title = "First song".into();
        let mut b = track("comp", "B");
        b.title = "Second song".into();
        let mut c = track("solo", "A");
        c.title = "Different song".into();
        let result = ArtistIndex::build(&artists, &[compilation, solo], &[a, b, c]);
        assert_eq!(result.compilations.albums.len(), 1);
        assert!(result.compilations.complete_releases.is_empty());
        assert_eq!(result.compilations.single_artist["A"][0].rating_key, "solo");
        assert!(!result.compilations.artist_keys.contains("A"));
    }
    #[test]
    fn explicitly_named_compilation_joins_disjoint_artist_records() {
        let title = "VA - Mutiny on the Bowery (Mystic Sounds of USA Cities Vol. 6) 1986";
        let mut albums = vec![
            album("a", "A", false, 1),
            album("b", "B", false, 1),
            album("va", "Various Artists", true, 1),
        ];
        for album in &mut albums {
            album.title = title.into();
        }
        let tracks = [track("a", "A"), track("b", "B"), track("va", "C")];
        let result = ArtistIndex::build(&[artist("A"), artist("B"), artist("C")], &albums, &tracks);
        assert_eq!(result.compilations.release_members["va"], ["a", "b", "va"]);
        assert_eq!(result.compilations.complete_releases["a"], "va");
        assert_eq!(result.compilations.complete_releases["b"], "va");
        assert!(result.compilations.single_artist.is_empty());
        for album in &mut albums {
            album.title = "Split".into();
        }
        let independent = ArtistIndex::build(&[], &albums, &tracks);
        assert!(independent.compilations.release_members.is_empty());
        albums[0].title = title.into();
        albums[1].title = title.into();
        albums[0].year = Some(1986);
        albums[1].year = Some(2001);
        assert!(
            ArtistIndex::build(&[], &albums, &tracks)
                .compilations
                .release_members
                .is_empty()
        );
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
        let albums = [album("hits", "The A", true, 2)];
        let tracks = [track("hits", "A"), track("hits", "The A")];
        let result = ArtistIndex::build(&artists, &albums, &tracks);
        assert_eq!(
            result.compilations.single_artist["The A"][0].rating_key,
            "hits"
        );
        assert!(result.compilations.albums.is_empty());
        assert!(!result.compilations.artist_keys.contains("The A"));
        assert_eq!(result.album_display["hits"], "A");

        for tracks in [
            vec![tracks[0].clone()],
            vec![tracks[0].clone(), track("hits", "")],
            vec![tracks[0].clone(), track("hits", "Unknown Artist")],
            vec![tracks[0].clone(), track("hits", "Various Artists")],
        ] {
            let result = ArtistIndex::build(&artists, &albums, &tracks);
            assert!(result.compilations.single_artist.is_empty());
            assert_eq!(result.compilations.albums.len(), 1);
            assert!(result.compilations.artist_keys.contains("The A"));
        }
    }

    #[test]
    fn various_artist_credits_survive_single_performer_and_incomplete_track_lists() {
        let artists = [artist("2.3"), artist("Various Artists")];
        for credited_artist in ["Various Artists", "Various"] {
            for tagged in [false, true] {
                for count in [Some(1), Some(27), None] {
                    let mut release = album("mutant-pop", "Various Artists", tagged, 1);
                    release.title = "Mutant Pop: 78/79".into();
                    release.parent_title = Some(credited_artist.into());
                    release.leaf_count = count;
                    let result =
                        ArtistIndex::build(&artists, &[release], &[track("mutant-pop", "2.3")]);
                    assert_eq!(result.compilations.albums.len(), 1);
                    assert!(result.compilations.single_artist.is_empty());
                    assert!(result.compilations.artist_keys.contains("2.3"));
                    assert!(result.compilations.artist_keys.contains("Various Artists"));
                    assert_eq!(result.compilations.artist_map["2.3"], ["mutant-pop"]);
                    assert!(!result.album_display.contains_key("mutant-pop"));
                }
            }
        }
    }

    #[test]
    fn compilation_ownership_does_not_require_loaded_performer_metadata() {
        let artists = [artist("Various Artists")];
        let release = album("teenline", "Various Artists", false, 27);
        for tracks in [vec![], vec![track("teenline", "Unknown Artist")]] {
            let result = ArtistIndex::build(&artists, std::slice::from_ref(&release), &tracks);
            assert_eq!(result.compilations.albums.len(), 1);
            assert!(result.compilations.artist_keys.contains("Various Artists"));
            assert!(result.compilations.artist_map.is_empty());
            assert!(result.compilations.single_artist.is_empty());
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
        assert!(
            result
                .compilations
                .artist_map
                .contains_key("track_artist:a")
        );
        assert!(
            result
                .compilations
                .artist_map
                .contains_key("track_artist:b")
        );
        assert!(
            !result
                .compilations
                .artist_map
                .contains_key("Various Artists")
        );
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
