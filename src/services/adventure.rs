//! Sonic Adventure generation algorithm.
//!
//! Creates a smooth sonic journey between two tracks by finding
//! tracks that bridge the sonic space between start and end.

use crate::api::models::Track;
use crate::api::{ApiError, PlexClient};
use std::collections::{HashMap, HashSet};

/// Generate a Sonic Adventure playlist from start to end track.
///
/// The algorithm:
/// 1. Get similar tracks for both start and end points
/// 2. Find "bridge" tracks that are similar to both
/// 3. Build a path with artist diversity: avoid consecutive tracks from same artist
/// 4. Structure: start -> start-similar -> bridges -> end-similar -> end
pub async fn generate_adventure(
    client: &PlexClient,
    start: &Track,
    end: &Track,
    length: usize,
) -> Result<Vec<Track>, ApiError> {
    // Clamp length to valid range
    let length = length.clamp(5, 100);

    // Get similar tracks for both endpoints (request more than needed for filtering)
    let start_similar: Vec<Track> = client.get_similar_tracks(&start.rating_key, 100).await?;
    let end_similar: Vec<Track> = client.get_similar_tracks(&end.rating_key, 100).await?;

    // Build key set for end-similar tracks for efficient bridge detection
    let end_keys: HashSet<String> = end_similar.iter().map(|t| t.rating_key.clone()).collect();

    // Categorize start_similar tracks: bridges vs start-only
    let mut bridges: Vec<Track> = Vec::new();
    let mut start_only: Vec<Track> = Vec::new();
    for track in start_similar {
        if end_keys.contains(&track.rating_key) {
            bridges.push(track);
        } else {
            start_only.push(track);
        }
    }

    // End-only tracks (not in start_similar)
    let start_keys: HashSet<String> = bridges
        .iter()
        .chain(start_only.iter())
        .map(|t| t.rating_key.clone())
        .collect();
    let end_only: Vec<Track> = end_similar
        .into_iter()
        .filter(|t| !start_keys.contains(&t.rating_key))
        .collect();

    let mut playlist = Vec::with_capacity(length);
    let mut used_keys = HashSet::new();
    let mut artist_counts: HashMap<String, usize> = HashMap::new();

    // Helper to get artist key for diversity tracking
    fn artist_key(track: &Track) -> String {
        track.artist_name().to_lowercase()
    }

    // Helper to select next track with artist diversity
    // Prefers tracks from artists we haven't used, or used least
    fn select_diverse<'a>(
        candidates: &'a [Track],
        used_keys: &HashSet<String>,
        artist_counts: &HashMap<String, usize>,
        last_artist: Option<&str>,
    ) -> Option<&'a Track> {
        // First pass: find tracks from unused artists (not the last artist)
        let mut best: Option<&Track> = None;
        let mut best_count = usize::MAX;

        for track in candidates {
            if used_keys.contains(&track.rating_key) {
                continue;
            }
            let artist = artist_key(track);

            // Skip if same as last artist (avoid consecutive)
            if let Some(last) = last_artist {
                if artist == last {
                    continue;
                }
            }

            let count = artist_counts.get(&artist).copied().unwrap_or(0);
            if count < best_count {
                best_count = count;
                best = Some(track);
            }
        }

        // If no diverse option, allow same artist as fallback
        if best.is_none() {
            for track in candidates {
                if used_keys.contains(&track.rating_key) {
                    continue;
                }
                let artist = artist_key(track);
                let count = artist_counts.get(&artist).copied().unwrap_or(0);
                if count < best_count {
                    best_count = count;
                    best = Some(track);
                }
            }
        }

        best
    }

    // Add a track to playlist with bookkeeping
    let add_track = |track: Track,
                         playlist: &mut Vec<Track>,
                         used_keys: &mut HashSet<String>,
                         artist_counts: &mut HashMap<String, usize>| {
        let artist = artist_key(&track);
        *artist_counts.entry(artist).or_insert(0) += 1;
        used_keys.insert(track.rating_key.clone());
        playlist.push(track);
    };

    // Always start with start track
    add_track(
        start.clone(),
        &mut playlist,
        &mut used_keys,
        &mut artist_counts,
    );
    used_keys.insert(end.rating_key.clone()); // Reserve end slot

    let middle_count = length.saturating_sub(2);

    // Build the middle section with three phases:
    // Phase 1: ~1/3 from start_only (tracks similar to start but not end)
    // Phase 2: ~1/3 from bridges (similar to both)
    // Phase 3: ~1/3 from end_only (tracks similar to end but not start)
    let phase1_target = middle_count / 3;
    let phase2_target = middle_count / 3;
    // phase3 gets the rest

    // Phase 1: Start-similar tracks
    for _ in 0..phase1_target {
        let last_artist = playlist.last().map(|t| artist_key(t));
        if let Some(track) =
            select_diverse(&start_only, &used_keys, &artist_counts, last_artist.as_deref())
        {
            add_track(
                track.clone(),
                &mut playlist,
                &mut used_keys,
                &mut artist_counts,
            );
        }
    }

    // Phase 2: Bridge tracks (similar to both)
    for _ in 0..phase2_target {
        let last_artist = playlist.last().map(|t| artist_key(t));
        if let Some(track) =
            select_diverse(&bridges, &used_keys, &artist_counts, last_artist.as_deref())
        {
            add_track(
                track.clone(),
                &mut playlist,
                &mut used_keys,
                &mut artist_counts,
            );
        }
    }

    // Phase 3: End-similar tracks (fill remaining)
    while playlist.len() < length - 1 {
        let last_artist = playlist.last().map(|t| artist_key(t));
        if let Some(track) =
            select_diverse(&end_only, &used_keys, &artist_counts, last_artist.as_deref())
        {
            add_track(
                track.clone(),
                &mut playlist,
                &mut used_keys,
                &mut artist_counts,
            );
        } else {
            break;
        }
    }

    // Fill any remaining slots from all pools
    let all_candidates: Vec<Track> = start_only
        .into_iter()
        .chain(bridges)
        .chain(end_only)
        .collect();

    while playlist.len() < length - 1 {
        let last_artist = playlist.last().map(|t| artist_key(t));
        if let Some(track) = select_diverse(
            &all_candidates,
            &used_keys,
            &artist_counts,
            last_artist.as_deref(),
        ) {
            add_track(
                track.clone(),
                &mut playlist,
                &mut used_keys,
                &mut artist_counts,
            );
        } else {
            break;
        }
    }

    // Always end with end track
    playlist.push(end.clone());

    Ok(playlist)
}
