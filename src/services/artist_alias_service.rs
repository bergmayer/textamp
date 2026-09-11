//! Artist alias resolution service.
//!
//! Pure functions for name normalization, alias computation, and artist key resolution.
//! Used to match track-level artist names (e.g. "Ramones") to server album artists
//! (e.g. "The Ramones") via normalized matching.

use std::collections::{HashMap, HashSet};

use crate::library::models::{Album, Artist, Track};

/// Normalize artist name for matching: lowercase + strip leading "The ".
pub fn normalize_artist_name(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    if lower.starts_with("the ") && lower.len() > 4 {
        lower[4..].to_string()
    } else {
        lower
    }
}

/// Compute artist aliases from bulk track data.
///
/// For each complete non-compilation album, if all tracks have `original_title` set and
/// agree on a single name that differs from the album artist, that name
/// is an alias of the album artist.
///
/// Returns (artist_key → set of alias names, album_key → display artist name).
pub fn compute_aliases(
    all_tracks: &[Track],
    albums: &[Album],
) -> (HashMap<String, HashSet<String>>, HashMap<String, String>) {
    let mut artist_aliases: HashMap<String, HashSet<String>> = HashMap::new();
    let mut album_display_artist: HashMap<String, String> = HashMap::new();

    if all_tracks.is_empty() {
        return (artist_aliases, album_display_artist);
    }

    // Build album key → Album lookup for compilation check
    let album_by_key: HashMap<&str, &Album> =
        albums.iter().map(|a| (a.rating_key.as_str(), a)).collect();

    // Group tracks by album (parent_rating_key)
    let mut album_tracks: HashMap<String, Vec<&Track>> = HashMap::new();
    for track in all_tracks {
        if let Some(ref album_key) = track.parent_rating_key {
            album_tracks
                .entry(album_key.clone())
                .or_default()
                .push(track);
        }
    }

    for (album_key, tracks) in &album_tracks {
        // Skip compilation candidates
        let Some(album) = album_by_key.get(album_key.as_str()) else {
            continue;
        };
        if album.is_compilation_candidate()
            || album
                .leaf_count
                .is_some_and(|count| count as usize != tracks.len())
        {
            continue;
        }

        // Collect unique original_title values (track artists).
        let mut track_artist_names: HashSet<&str> = HashSet::new();
        let mut with_original_title = 0usize;
        for track in tracks {
            if let Some(ref s) = track.original_title {
                if !s.is_empty() {
                    track_artist_names.insert(s.as_str());
                    with_original_title += 1;
                }
            }
        }

        // Need at least one track with original_title, and all must agree
        if with_original_title != tracks.len() || track_artist_names.len() != 1 {
            continue;
        }

        let uniform_name = track_artist_names.into_iter().next().unwrap();

        // Song artist IDs identify performers on OpenSubsonic, not album owners.
        let album_artist_name = album.artist_name();
        let Some(artist_key) = album.parent_rating_key.clone() else {
            continue;
        };

        // Only create alias if the track artist differs from the album artist
        // Use normalized comparison so "Ramones" vs "The Ramones" are treated as same
        if normalize_artist_name(uniform_name) == normalize_artist_name(album_artist_name) {
            continue;
        }

        // "Various Artists" is never a real alias — skip it
        if uniform_name.eq_ignore_ascii_case("Various Artists") {
            continue;
        }

        // Record the alias
        artist_aliases
            .entry(artist_key)
            .or_default()
            .insert(uniform_name.to_string());
        album_display_artist.insert(album_key.clone(), uniform_name.to_string());
    }

    (artist_aliases, album_display_artist)
}

/// Build a normalized artist name → artist key lookup from the artist list
/// AND from alias reverse mappings.
///
/// This allows resolving "Ramones" to the key for "The Ramones" because both
/// normalize to "ramones".
pub fn build_artist_lookup(
    artists: &[Artist],
    aliases: &HashMap<String, HashSet<String>>,
) -> HashMap<String, String> {
    let mut lookup: HashMap<String, String> = HashMap::new();

    // All artists: normalize(artist.title) → artist.rating_key
    for artist in artists {
        lookup.insert(
            normalize_artist_name(&artist.title),
            artist.rating_key.clone(),
        );
    }

    // All aliases: for each (artist_key, alias_names), normalize(alias) → artist_key
    for (artist_key, alias_names) in aliases {
        for alias in alias_names {
            lookup
                .entry(normalize_artist_name(alias))
                .or_insert_with(|| artist_key.clone());
        }
    }

    lookup
}

/// Resolve a track artist name to a real server artist key.
pub fn resolve_artist_key(track_artist: &str, lookup: &HashMap<String, String>) -> Option<String> {
    lookup.get(&normalize_artist_name(track_artist)).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_artist_name() {
        assert_eq!(normalize_artist_name("The Ramones"), "ramones");
        assert_eq!(normalize_artist_name("Ramones"), "ramones");
        assert_eq!(normalize_artist_name("the clash"), "clash");
        assert_eq!(normalize_artist_name("U2"), "u2");
        // "The The" should normalize to "the" (strip leading "The ", lowercase remainder)
        assert_eq!(normalize_artist_name("The The"), "the");
        // Don't strip if name is just "The"
        assert_eq!(normalize_artist_name("The"), "the");
    }

    #[test]
    fn aliases_use_album_owner_and_do_not_override_real_artist_identities() {
        let tracks = [make_track(
            "t",
            "album",
            "performer",
            "Owner",
            Some("Performer"),
        )];
        let albums = [make_album("album", "owner", "Owner")];
        let (aliases, _) = compute_aliases(&tracks, &albums);
        assert!(aliases["owner"].contains("Performer"));
        assert!(!aliases.contains_key("performer"));
        let artists = [Artist {
            rating_key: "performer".into(),
            title: "Performer".into(),
            ..Default::default()
        }];
        assert_eq!(
            build_artist_lookup(&artists, &aliases)["performer"],
            "performer"
        );
    }

    #[test]
    fn aliases_are_not_inferred_from_incomplete_albums() {
        let tracks = [make_track(
            "t",
            "album",
            "performer",
            "Owner",
            Some("Performer"),
        )];
        let mut album = make_album("album", "owner", "Owner");
        album.leaf_count = Some(2);
        assert!(compute_aliases(&tracks, &[album.clone()]).0.is_empty());
        let tracks = [
            tracks[0].clone(),
            make_track("missing", "album", "owner", "Owner", None),
        ];
        assert!(compute_aliases(&tracks, &[album]).0.is_empty());
    }

    fn make_track(
        rating_key: &str,
        album_key: &str,
        artist_key: &str,
        artist_name: &str,
        original_title: Option<&str>,
    ) -> Track {
        Track {
            rating_key: rating_key.to_string(),
            parent_rating_key: Some(album_key.to_string()),
            grandparent_rating_key: Some(artist_key.to_string()),
            grandparent_title: Some(artist_name.to_string()),
            original_title: original_title.map(|s| s.to_string()),
            ..Track::default()
        }
    }

    fn make_album(key: &str, artist_key: &str, artist_name: &str) -> Album {
        Album {
            rating_key: key.to_string(),
            parent_rating_key: Some(artist_key.to_string()),
            parent_title: Some(artist_name.to_string()),
            ..Album::default()
        }
    }

    #[test]
    fn test_compute_aliases() {
        // Album "Bee Thousand" by "Robert Pollard" where all tracks have original_title "Guided by Voices"
        let tracks = vec![
            make_track(
                "t1",
                "album1",
                "artist1",
                "Robert Pollard",
                Some("Guided by Voices"),
            ),
            make_track(
                "t2",
                "album1",
                "artist1",
                "Robert Pollard",
                Some("Guided by Voices"),
            ),
        ];
        let albums = vec![make_album("album1", "artist1", "Robert Pollard")];

        let (aliases, display) = compute_aliases(&tracks, &albums);

        assert!(aliases.contains_key("artist1"));
        assert!(aliases["artist1"].contains("Guided by Voices"));
        assert_eq!(display.get("album1").unwrap(), "Guided by Voices");
    }

    #[test]
    fn test_compute_aliases_mixed() {
        // Album with mixed original_title → no alias
        let tracks = vec![
            make_track(
                "t1",
                "album1",
                "artist1",
                "Robert Pollard",
                Some("Guided by Voices"),
            ),
            make_track(
                "t2",
                "album1",
                "artist1",
                "Robert Pollard",
                Some("Boston Spaceships"),
            ),
        ];
        let albums = vec![make_album("album1", "artist1", "Robert Pollard")];

        let (aliases, display) = compute_aliases(&tracks, &albums);

        assert!(aliases.is_empty());
        assert!(display.is_empty());
    }

    #[test]
    fn test_compute_aliases_normalized_same() {
        // "Ramones" as original_title should NOT create alias for "The Ramones" artist
        // because they normalize to the same name
        let tracks = vec![
            make_track("t1", "album1", "artist1", "The Ramones", Some("Ramones")),
            make_track("t2", "album1", "artist1", "The Ramones", Some("Ramones")),
        ];
        let albums = vec![make_album("album1", "artist1", "The Ramones")];

        let (aliases, display) = compute_aliases(&tracks, &albums);

        assert!(aliases.is_empty());
        assert!(display.is_empty());
    }

    #[test]
    fn test_build_artist_lookup() {
        let artists = vec![
            Artist {
                rating_key: "k1".to_string(),
                title: "The Ramones".to_string(),
                ..Artist::default()
            },
            Artist {
                rating_key: "k2".to_string(),
                title: "U2".to_string(),
                ..Artist::default()
            },
        ];
        let mut aliases: HashMap<String, HashSet<String>> = HashMap::new();
        aliases.insert(
            "k3".to_string(),
            HashSet::from(["Guided by Voices".to_string()]),
        );

        let lookup = build_artist_lookup(&artists, &aliases);

        // "The Ramones" and "Ramones" both normalize to "ramones" → k1
        assert_eq!(lookup.get("ramones").unwrap(), "k1");
        assert_eq!(lookup.get("u2").unwrap(), "k2");
        assert_eq!(lookup.get("guided by voices").unwrap(), "k3");
    }

    #[test]
    fn test_resolve_artist_key() {
        let artists = vec![Artist {
            rating_key: "k1".to_string(),
            title: "The Ramones".to_string(),
            ..Artist::default()
        }];
        let aliases = HashMap::new();
        let lookup = build_artist_lookup(&artists, &aliases);

        // "Ramones" resolves to "The Ramones" key because both normalize to "ramones"
        assert_eq!(
            resolve_artist_key("Ramones", &lookup),
            Some("k1".to_string())
        );
        assert_eq!(
            resolve_artist_key("The Ramones", &lookup),
            Some("k1".to_string())
        );
        assert_eq!(resolve_artist_key("Unknown Band", &lookup), None);
    }
}
