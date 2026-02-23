//! Compilation detection from cached track data.

use std::collections::{HashMap, HashSet};

use crate::plex::models::{Album, Artist, Track};
use crate::app::{AppState, Event};
use crate::plex::PlexClient;
use tokio::sync::mpsc;

/// Check preconditions and run compilation detection if needed.
///
/// Call this from any code path that finishes populating artists + albums + all_tracks
/// (cache load, preload events, etc.). It's idempotent: if detection has
/// already run or is not needed, it returns immediately.
pub fn maybe_detect(
    event_tx: &mpsc::Sender<Event>,
    state: &AppState,
    _client: &PlexClient,
) {
    // Already detected (or currently detecting)
    if state.compilations.detected {
        return;
    }
    // Need artists, albums, and all_tracks loaded
    if state.artists.is_empty() || state.albums.is_empty() || state.all_tracks.is_empty() {
        return;
    }
    // Need an active library
    let Some(lib_key) = state.active_library.clone() else {
        return;
    };

    tracing::info!(
        "Starting compilation detection: {} albums, {} artists, {} tracks",
        state.albums.len(),
        state.artists.len(),
        state.all_tracks.len()
    );

    // Pure function — no API calls needed
    let result = detect_compilations_from_tracks(
        &state.albums,
        &state.all_tracks,
        &state.artists,
        &state.artist_aliases,
    );

    let tx = event_tx.clone();
    // Send result via event (keeps same pattern, could be made sync but event
    // pattern ensures consistent state update path)
    tokio::spawn(async move {
        let _ = tx.send(Event::CompilationsDetected {
            library_key: lib_key,
            albums: result.confirmed_compilations,
            artist_only_keys: result.artist_only_keys,
            track_artist_keys: result.compilation_track_artist_keys,
            artist_compilation_map: result.artist_compilation_map,
            single_artist_compilations: result.single_artist_compilations,
        }).await;
    });
}

/// Result of compilation detection.
struct CompilationResult {
    confirmed_compilations: Vec<Album>,
    artist_only_keys: HashSet<String>,
    compilation_track_artist_keys: HashSet<String>,
    artist_compilation_map: HashMap<String, Vec<String>>,
    single_artist_compilations: HashMap<String, Vec<Album>>,
}

/// Detect compilation albums using the pre-cached track list.
///
/// Groups tracks by album, checks each compilation candidate for multi-artist content.
/// Pure function — no API calls, no async.
fn detect_compilations_from_tracks(
    albums: &[Album],
    all_tracks: &[Track],
    artists: &[Artist],
    artist_aliases: &HashMap<String, HashSet<String>>,
) -> CompilationResult {
    // Group tracks by parent_rating_key (album key)
    let mut tracks_by_album: HashMap<&str, Vec<&Track>> = HashMap::new();
    for track in all_tracks {
        if let Some(ref album_key) = track.parent_rating_key {
            tracks_by_album.entry(album_key.as_str()).or_default().push(track);
        }
    }

    // Build all_artist_keys and normalized artist lookup
    let all_artist_keys: HashSet<String> = artists.iter()
        .map(|a| a.rating_key.clone())
        .collect();
    let artist_lookup = crate::services::artist_alias_service::build_artist_lookup(artists, artist_aliases);

    let candidates: Vec<&Album> = albums.iter()
        .filter(|a| a.is_compilation_candidate())
        .collect();

    if candidates.is_empty() {
        return CompilationResult {
            confirmed_compilations: vec![],
            artist_only_keys: HashSet::new(),
            compilation_track_artist_keys: HashSet::new(),
            artist_compilation_map: HashMap::new(),
            single_artist_compilations: HashMap::new(),
        };
    }

    tracing::info!("Checking {} compilation candidates...", candidates.len());

    let mut confirmed: Vec<Album> = Vec::new();
    let mut compilation_track_artist_keys: HashSet<String> = HashSet::new();
    let mut artist_compilation_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut single_artist_compilations: HashMap<String, Vec<Album>> = HashMap::new();

    // Helper: resolve a track artist name to a real artist key using normalized matching
    let resolve_artist_key = |name: &str| -> Option<String> {
        crate::services::artist_alias_service::resolve_artist_key(name, &artist_lookup)
    };

    for album in &candidates {
        let tracks = match tracks_by_album.get(album.rating_key.as_str()) {
            Some(t) if !t.is_empty() => t,
            _ => continue,
        };

        // Collect distinct artist names from tracks.
        // Use original_title (track-level artist) when available,
        // falling back to grandparent_title (album-level artist).
        let mut track_artists: HashSet<String> = HashSet::new();
        let mut real_artist_keys: HashSet<String> = HashSet::new();
        for track in tracks {
            let artist = track.original_title.as_deref()
                .unwrap_or_else(|| track.artist_name());
            track_artists.insert(artist.to_lowercase());
            // Resolve track artist to a real artist key via name or alias
            if let Some(real_key) = resolve_artist_key(artist) {
                real_artist_keys.insert(real_key.clone());
                compilation_track_artist_keys.insert(real_key);
            }
            // Also include grandparent key for backward compat
            if let Some(ref key) = track.grandparent_rating_key {
                compilation_track_artist_keys.insert(key.clone());
            }
        }

        // Multi-artist = true compilation
        if track_artists.len() > 1 {
            confirmed.push((*album).clone());
            // Map each real track artist to this compilation album
            for artist_key in &real_artist_keys {
                artist_compilation_map
                    .entry(artist_key.clone())
                    .or_default()
                    .push(album.rating_key.clone());
            }
        } else if track_artists.len() == 1 {
            // Single-artist "compilation" — map to the actual artist
            let artist_name = track_artists.iter().next().unwrap(); // lowercase
            if let Some(real_key) = resolve_artist_key(artist_name) {
                single_artist_compilations
                    .entry(real_key.clone())
                    .or_default()
                    .push((*album).clone());
            }
        }
    }

    // Find artist keys that appear ONLY on compilations (no solo albums)
    let confirmed_keys: HashSet<&str> = confirmed.iter()
        .map(|a| a.rating_key.as_str())
        .collect();

    let mut artists_with_solo: HashSet<String> = HashSet::new();
    for album in albums {
        if confirmed_keys.contains(album.rating_key.as_str()) {
            continue;
        }
        if let Some(ref artist_key) = album.parent_rating_key {
            artists_with_solo.insert(artist_key.clone());
        }
    }

    let artist_only_keys: HashSet<String> = compilation_track_artist_keys.iter()
        .filter(|key| all_artist_keys.contains(*key) && !artists_with_solo.contains(*key))
        .cloned()
        .collect();

    tracing::info!(
        "Compilation detection complete: {} confirmed, {} single-artist, {} artist-only keys, {} track artist keys, {} artists on compilations",
        confirmed.len(), single_artist_compilations.values().map(|v| v.len()).sum::<usize>(),
        artist_only_keys.len(), compilation_track_artist_keys.len(), artist_compilation_map.len()
    );

    CompilationResult {
        confirmed_compilations: confirmed,
        artist_only_keys,
        compilation_track_artist_keys,
        artist_compilation_map,
        single_artist_compilations,
    }
}
