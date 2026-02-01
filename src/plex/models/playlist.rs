//! Playlist models.

use serde::{Deserialize, Serialize};

/// Playlist metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub rating_key: String,
    pub key: String,
    pub title: String,
    #[serde(rename = "type")]
    pub playlist_type: String,
    #[serde(default)]
    pub composite: Option<String>,
    #[serde(default)]
    pub duration: Option<u64>,
    #[serde(default)]
    pub leaf_count: Option<u32>,
    #[serde(default)]
    pub added_at: Option<i64>,
    #[serde(default)]
    pub updated_at: Option<i64>,
    #[serde(default)]
    pub smart: bool,
}

impl Playlist {
    /// Get track count.
    pub fn track_count(&self) -> u32 {
        self.leaf_count.unwrap_or(0)
    }

    /// Check if this is an audio playlist.
    pub fn is_audio(&self) -> bool {
        self.playlist_type == "audio"
    }
}

/// Response wrapper for playlists.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaylistsResponse {
    pub media_container: PlaylistsContainer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaylistsContainer {
    #[serde(default, rename = "Metadata")]
    pub metadata: Vec<Playlist>,
}
