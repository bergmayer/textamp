//! Track positions indexed once per catalog snapshot for inexpensive browsing.
use super::artist_alias_service::normalize_artist_name;
use crate::library::models::{Album, Artist, Track};
use std::collections::HashMap;

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrackLookup {
    pub albums: HashMap<String, Vec<usize>>,
    pub artists: HashMap<String, Vec<usize>>,
    pub len: usize,
}
impl TrackLookup {
    pub fn build(
        artists: &[Artist],
        performers: &[Artist],
        albums: &[Album],
        tracks: &[Track],
    ) -> Self {
        let names: HashMap<_, _> = artists
            .iter()
            .chain(performers)
            .map(|artist| {
                (
                    normalize_artist_name(&artist.title),
                    artist.rating_key.as_str(),
                )
            })
            .collect();
        let owners: HashMap<_, _> = albums
            .iter()
            .filter_map(|album| {
                Some((
                    album.rating_key.as_str(),
                    album.parent_rating_key.as_deref()?,
                ))
            })
            .collect();
        let mut lookup = Self {
            len: tracks.len(),
            ..Default::default()
        };
        for (index, track) in tracks.iter().enumerate() {
            if let Some(album) = &track.parent_rating_key {
                lookup.albums.entry(album.clone()).or_default().push(index);
            }
            let owner = track
                .parent_rating_key
                .as_deref()
                .and_then(|album| owners.get(album).copied());
            let name = normalize_artist_name(track.track_artist());
            for key in [
                track.grandparent_rating_key.as_deref(),
                owner,
                names.get(&name).copied(),
            ]
            .into_iter()
            .flatten()
            {
                let rows = lookup.artists.entry(key.to_owned()).or_default();
                if rows.last() != Some(&index) {
                    rows.push(index);
                }
            }
        }
        lookup
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn album_owner_and_performer_lookups_share_positions_without_duplicates() {
        let artists = vec![Artist {
            rating_key: "a".into(),
            title: "Artist".into(),
            ..Default::default()
        }];
        let albums = vec![Album {
            rating_key: "album".into(),
            parent_rating_key: Some("a".into()),
            ..Default::default()
        }];
        let tracks = vec![Track {
            parent_rating_key: Some("album".into()),
            grandparent_rating_key: Some("a".into()),
            original_title: Some("Artist".into()),
            ..Default::default()
        }];
        let lookup = TrackLookup::build(&artists, &[], &albums, &tracks);
        assert_eq!(lookup.albums["album"], vec![0]);
        assert_eq!(lookup.artists["a"], vec![0]);
    }
}
