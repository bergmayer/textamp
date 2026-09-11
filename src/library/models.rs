//! Shared catalog and navigation models, independent of transport protocols.
pub use super::catalog::{Album, Artist, GenreTag, Playlist};
pub use super::folder::{FolderColumn, FolderItem, FolderItemType, FolderNavigationState};
pub use super::station::{RadioSource, Station, StationKind};
pub use super::track::{Media, MediaPart, Track};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Genre {
    pub key: String,
    pub title: String,
    pub count: Option<u32>,
}
impl Genre {
    pub fn display_title(&self) -> &str {
        &self.title
    }
    pub fn effective_key(&self) -> &str {
        &self.key
    }
}

#[derive(Debug, Clone, Default)]
pub struct SearchResults {
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
    pub playlists: Vec<Playlist>,
    pub genres: Vec<Genre>,
}
impl SearchResults {
    pub fn is_empty(&self) -> bool {
        self.total_count() == 0
    }
    pub fn total_count(&self) -> usize {
        self.artists.len()
            + self.albums.len()
            + self.tracks.len()
            + self.playlists.len()
            + self.genres.len()
    }
}
