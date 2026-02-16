//! Background compilation detection.

use std::collections::HashSet;

use crate::api::models::Album;
use crate::api::PlexClient;
use crate::app::Event;
use tokio::sync::mpsc;

/// Detect compilation albums from the full album list.
///
/// A compilation candidate (subtype="compilation" or "Various Artists") is confirmed
/// as a true compilation if its tracks come from multiple distinct artists.
/// Same-artist compilations (e.g., greatest hits) are excluded.
///
/// Also computes the set of artist keys that appear ONLY on compilations
/// (i.e., they have no solo albums), so they can be hidden from the artist list.
pub async fn detect_compilations(
    event_tx: mpsc::Sender<Event>,
    library_key: String,
    albums: Vec<Album>,
    all_artist_keys: HashSet<String>,
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
        }).await;
        return;
    }

    tracing::info!("Checking {} compilation candidates...", candidates.len());

    let mut confirmed: Vec<Album> = Vec::new();
    // Track artist keys that appear on compilations (track-level grandparent_rating_key)
    let mut compilation_track_artist_keys: HashSet<String> = HashSet::new();

    for album in &candidates {
        // Fetch tracks for each candidate to check if multi-artist
        match client.get_album_tracks(&album.rating_key).await {
            Ok(tracks) => {
                if tracks.is_empty() {
                    continue;
                }

                // Collect distinct artist names from tracks
                let mut track_artists: HashSet<String> = HashSet::new();
                for track in &tracks {
                    track_artists.insert(track.artist_name().to_lowercase());
                    // Collect grandparent_rating_key (artist key) for each track
                    if let Some(ref key) = track.grandparent_rating_key {
                        compilation_track_artist_keys.insert(key.clone());
                    }
                }

                // Multi-artist = true compilation
                if track_artists.len() > 1 {
                    confirmed.push((*album).clone());
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
        "Compilation detection complete: {} confirmed, {} artist-only keys, {} track artist keys",
        confirmed.len(), artist_only_keys.len(), compilation_track_artist_keys.len()
    );

    let _ = event_tx.send(Event::CompilationsDetected {
        library_key,
        albums: confirmed,
        artist_only_keys,
        track_artist_keys: compilation_track_artist_keys,
    }).await;
}
