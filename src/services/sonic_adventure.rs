//! Artist diversity constraints for sonic paths, shared by every frontend.
use crate::library::models::Track;
use anyhow::{Result, ensure};
use std::collections::HashSet;

fn artist(track: &Track) -> Result<String> {
    let artist = track
        .track_artist()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    ensure!(
        !artist.is_empty() && artist != "unknown artist",
        "Journey requires artist metadata for every track"
    );
    Ok(artist)
}

pub fn validate_endpoints(start: &Track, end: &Track) -> Result<()> {
    ensure!(
        artist(start)? != artist(end)?,
        "Choose start and end tracks by different artists"
    );
    Ok(())
}

/// Preserve the endpoints and path order, excluding repeated intermediate artists.
/// The requested path length is a maximum, so a diverse path may be shorter.
pub fn unique_artists(tracks: Vec<Track>) -> Result<Vec<Track>> {
    ensure!(
        tracks.len() >= 3,
        "No sonic path found. Choose different tracks"
    );
    validate_endpoints(&tracks[0], &tracks[tracks.len() - 1])?;
    let mut seen = HashSet::from([artist(&tracks[0])?, artist(&tracks[tracks.len() - 1])?]);
    let last = tracks.len() - 1;
    let mut path = Vec::with_capacity(tracks.len());
    for (index, track) in tracks.into_iter().enumerate() {
        if index == 0 || index == last || seen.insert(artist(&track)?) {
            path.push(track);
        }
    }
    ensure!(
        path.len() >= 3,
        "No path with distinct artists found. Choose different tracks"
    );
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn track(key: &str, artist: &str) -> Track {
        Track {
            rating_key: key.into(),
            grandparent_title: Some(artist.into()),
            ..Default::default()
        }
    }
    #[test]
    fn endpoints_are_reserved_and_intermediate_order_is_preserved() {
        let path = unique_artists(vec![
            track("a", "Start"),
            track("b", " end "),
            track("c", "Middle"),
            track("d", "MIDDLE"),
            track("e", "start"),
            track("f", "End"),
        ])
        .unwrap();
        assert_eq!(
            path.iter()
                .map(|t| t.rating_key.as_str())
                .collect::<Vec<_>>(),
            ["a", "c", "f"]
        );
    }
    #[test]
    fn rejects_same_artist_endpoints_and_paths_without_a_bridge() {
        assert!(validate_endpoints(&track("a", "Artist"), &track("b", " artist ")).is_err());
        assert!(unique_artists(vec![track("a", "A"), track("b", "A"), track("c", "C")]).is_err());
        assert!(unique_artists(vec![track("a", "A"), track("b", ""), track("c", "C")]).is_err());
    }
}
