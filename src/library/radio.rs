//! Folder radio records tagged albums without fetching audio from WebDAV.
use super::track::{Track, TrackOrigin};
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FolderRadio {
    #[serde(default)]
    pub albums: BTreeMap<String, Track>,
}

pub fn album_key(track: &Track) -> Option<String> {
    match &track.origin {
        TrackOrigin::Folder { .. } => {
            let artist = track.grandparent_title.as_deref()?.trim().to_lowercase();
            let album = track.parent_title.as_deref()?.trim().to_lowercase();
            (!artist.is_empty() && !album.is_empty()).then(|| format!("{artist}\0{album}"))
        }
        _ => track.parent_rating_key.clone(),
    }
}
impl FolderRadio {
    pub fn observe(&mut self, mut track: Track) {
        if let Some(key) = album_key(&track) {
            track.lyrics = None;
            self.albums.insert(key, track);
        }
    }
    pub fn available(&self) -> bool {
        let mut artists = HashSet::new();
        self.albums
            .keys()
            .filter_map(|k| k.split_once('\0'))
            .any(|(artist, _)| {
                artists.insert(artist)
                    && artists.len() >= crate::services::radio::MIN_STATION_ARTISTS
            })
    }
    pub fn select(
        &self,
        excluded: &HashSet<String>,
        seed: Option<&Track>,
    ) -> anyhow::Result<Vec<Track>> {
        anyhow::ensure!(self.available(), crate::services::radio::SPARSE_STATION);
        let mut candidates: Vec<_> = self
            .albums
            .iter()
            .filter(|(k, _)| !excluded.contains(*k))
            .map(|(_, t)| t)
            .collect();
        candidates.shuffle(&mut rand::rng());
        if let Some(seed) = seed {
            candidates.sort_by_cached_key(|t| {
                std::cmp::Reverse(t.genres.iter().filter(|g| seed.genres.contains(g)).count())
            });
        }
        Ok(candidates.into_iter().take(100).cloned().collect())
    }
}
