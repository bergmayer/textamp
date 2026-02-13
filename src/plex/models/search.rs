//! Search result models.

use super::{Album, Artist, Genre, Playlist, Track};
use serde::{Deserialize, Serialize};

/// Combined search results.
#[derive(Debug, Clone, Default)]
pub struct SearchResults {
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
    pub playlists: Vec<Playlist>,
    pub genres: Vec<Genre>,
}

impl SearchResults {
    /// Check if there are any results.
    pub fn is_empty(&self) -> bool {
        self.artists.is_empty()
            && self.albums.is_empty()
            && self.tracks.is_empty()
            && self.playlists.is_empty()
            && self.genres.is_empty()
    }

    /// Get total result count.
    pub fn total_count(&self) -> usize {
        self.artists.len() + self.albums.len() + self.tracks.len() + self.playlists.len() + self.genres.len()
    }
}

/// Response wrapper for search (uses hub format).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SearchResponse {
    pub media_container: SearchContainer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SearchContainer {
    #[serde(default, rename = "Hub")]
    pub hub: Vec<SearchHub>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHub {
    #[serde(rename = "type")]
    pub hub_type: String,
    pub title: String,
    #[serde(default)]
    pub size: u32,
    #[serde(default, rename = "Metadata")]
    pub metadata: Option<serde_json::Value>,
}
