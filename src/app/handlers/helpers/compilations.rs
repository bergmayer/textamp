//! Background compilation detection.

use std::collections::{HashMap, HashSet};

use crate::api::models::Album;
use crate::api::PlexClient;
use crate::app::{AppState, Event};
use tokio::sync::mpsc;

/// Check preconditions and spawn compilation detection if needed.
///
/// Call this from any code path that finishes populating artists + albums
/// (cache load, preload events, etc.). It's idempotent: if detection has
/// already run or is not needed, it returns immediately.
pub fn maybe_detect(
    event_tx: &mpsc::Sender<Event>,
    state: &AppState,
    client: &PlexClient,
) {
    // Already detected (or currently detecting)
    if state.compilations_detected {
        return;
    }
    // Need both artists and albums loaded
    if state.artists.is_empty() || state.albums.is_empty() {
        return;
    }
    // Need an active library
    let Some(lib_key) = state.active_library.clone() else {
        return;
    };

    tracing::info!(
        "Starting compilation detection: {} albums, {} artists",
        state.albums.len(),
        state.artists.len()
    );

    let tx = event_tx.clone();
    let client_clone = client.clone();
    let albums_clone = state.albums.clone();
    let all_artist_keys: HashSet<String> = state.artists.iter()
        .map(|a| a.rating_key.clone())
        .collect();
    // Build lowercase name → key map for looking up actual artist from track artist name
    let artist_name_to_key: HashMap<String, String> = state.artists.iter()
        .map(|a| (a.title.to_lowercase(), a.rating_key.clone()))
        .collect();

    tokio::spawn(async move {
        detect_compilations(tx, lib_key, albums_clone, all_artist_keys, artist_name_to_key, client_clone).await;
    });
}

/// Detect compilation albums from the full album list.
///
/// A compilation candidate (subtype="compilation" or "Various Artists") is confirmed
/// as a true compilation if its tracks come from multiple distinct artists.
/// Same-artist compilations (e.g., greatest hits) are excluded.
///
/// Also computes the set of artist keys that appear ONLY on compilations
/// (i.e., they have no solo albums), so they can be hidden from the artist list.
///
/// Builds `artist_compilation_map`: artist_key → Vec<album_rating_key> so we
/// can show compilation appearances in an artist's album view.
async fn detect_compilations(
    event_tx: mpsc::Sender<Event>,
    library_key: String,
    albums: Vec<Album>,
    all_artist_keys: HashSet<String>,
    artist_name_to_key: HashMap<String, String>,
    client: PlexClient,
) {
    let candidates: Vec<&Album> = albums.iter()
        .filter(|a| a.is_compilation_candidate())
        .collect();

    if candidates.is_empty() {
        let _ = event_tx.send(Event::CompilationsDetected {
            library_key,
            albums: vec![],
            artist_only_keys: HashSet::new(),
            track_artist_keys: HashSet::new(),
            artist_compilation_map: HashMap::new(),
            single_artist_compilations: HashMap::new(),
        }).await;
        return;
    }

    tracing::info!("Checking {} compilation candidates...", candidates.len());

    let mut confirmed: Vec<Album> = Vec::new();
    // Track artist keys that appear on compilations (track-level grandparent_rating_key)
    let mut compilation_track_artist_keys: HashSet<String> = HashSet::new();
    // Map: artist_key → Vec<album_rating_key> for compilation appearances
    let mut artist_compilation_map: HashMap<String, Vec<String>> = HashMap::new();
    // Single-artist "compilations" mapped to the actual artist key
    let mut single_artist_compilations: HashMap<String, Vec<Album>> = HashMap::new();

    for album in &candidates {
        // Fetch tracks for each candidate to check if multi-artist
        match client.get_album_tracks(&album.rating_key).await {
            Ok(tracks) => {
                if tracks.is_empty() {
                    continue;
                }

                // Collect distinct artist names from tracks.
                // Use original_title (track-level artist) when available,
                // falling back to grandparent_title (album-level artist).
                let mut track_artists: HashSet<String> = HashSet::new();
                let mut album_artist_keys: HashSet<String> = HashSet::new();
                for track in &tracks {
                    let artist = track.original_title.as_deref()
                        .unwrap_or_else(|| track.artist_name());
                    track_artists.insert(artist.to_lowercase());
                    if let Some(ref key) = track.grandparent_rating_key {
                        compilation_track_artist_keys.insert(key.clone());
                        album_artist_keys.insert(key.clone());
                    }
                }

                // Multi-artist = true compilation
                if track_artists.len() > 1 {
                    confirmed.push((*album).clone());
                    // Record which artists appear on this compilation
                    for artist_key in &album_artist_keys {
                        artist_compilation_map
                            .entry(artist_key.clone())
                            .or_default()
                            .push(album.rating_key.clone());
                    }
                } else if track_artists.len() == 1 {
                    // Single-artist "compilation" — map to the actual artist
                    // so it appears as a normal album under that artist.
                    // Look up artist by name since grandparent_rating_key points
                    // to the album-level parent (e.g. "Various Artists"), not the
                    // actual track artist.
                    let artist_name = track_artists.iter().next().unwrap(); // lowercase
                    if let Some(real_key) = artist_name_to_key.get(artist_name) {
                        single_artist_compilations
                            .entry(real_key.clone())
                            .or_default()
                            .push((*album).clone());
                    }
                }
            }
            Err(e) => {
                tracing::debug!("Failed to fetch tracks for compilation candidate {}: {}", album.rating_key, e);
            }
        }
    }

    // Find artist keys that appear ONLY on compilations (no solo albums).
    // An artist's solo albums are those in `albums` where:
    //   - parent_rating_key matches the artist key
    //   - the album is NOT a confirmed compilation
    let confirmed_keys: HashSet<&str> = confirmed.iter()
        .map(|a| a.rating_key.as_str())
        .collect();

    let mut artists_with_solo: HashSet<String> = HashSet::new();
    for album in &albums {
        if confirmed_keys.contains(album.rating_key.as_str()) {
            continue; // Skip compilation albums
        }
        if let Some(ref artist_key) = album.parent_rating_key {
            artists_with_solo.insert(artist_key.clone());
        }
    }

    // Artist-only-keys = compilation track artists that have no solo albums
    // AND are actual artists in the library (have an entry in all_artist_keys)
    let artist_only_keys: HashSet<String> = compilation_track_artist_keys.iter()
        .filter(|key| all_artist_keys.contains(*key) && !artists_with_solo.contains(*key))
        .cloned()
        .collect();

    tracing::info!(
        "Compilation detection complete: {} confirmed, {} single-artist, {} artist-only keys, {} track artist keys, {} artists on compilations",
        confirmed.len(), single_artist_compilations.values().map(|v| v.len()).sum::<usize>(),
        artist_only_keys.len(), compilation_track_artist_keys.len(), artist_compilation_map.len()
    );

    let _ = event_tx.send(Event::CompilationsDetected {
        library_key,
        albums: confirmed,
        artist_only_keys,
        track_artist_keys: compilation_track_artist_keys,
        artist_compilation_map,
        single_artist_compilations,
    }).await;
}
