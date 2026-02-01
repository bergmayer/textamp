//! Artist, Album, and Track models.

use super::serde_helpers::from_str_or_num_opt;
use serde::{Deserialize, Serialize};

/// Artist metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    #[serde(default)]
    pub rating_key: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub art: Option<String>,
    #[serde(default, rename = "Genre")]
    pub genre: Vec<GenreTag>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub added_at: Option<i64>,
}

/// Album metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    #[serde(default)]
    pub rating_key: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub parent_title: Option<String>,
    #[serde(default)]
    pub parent_rating_key: Option<String>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub year: Option<u16>,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default, rename = "Genre")]
    pub genre: Vec<GenreTag>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub leaf_count: Option<u32>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub duration: Option<u64>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub added_at: Option<i64>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub loudness_analysis_version: Option<u32>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub last_viewed_at: Option<i64>,
}

impl Album {
    /// Get track count (leaf_count in Plex API).
    pub fn track_count(&self) -> u32 {
        self.leaf_count.unwrap_or(0)
    }

    /// Get artist name.
    pub fn artist_name(&self) -> &str {
        self.parent_title.as_deref().unwrap_or("Unknown Artist")
    }

    /// Create an Album stub from a Track's parent info.
    /// Used for updating recently played without fetching full album data.
    pub fn from_track(track: &Track) -> Option<Self> {
        let rating_key = track.parent_rating_key.as_ref()?;
        Some(Album {
            rating_key: rating_key.clone(),
            key: format!("/library/metadata/{}/children", rating_key),
            title: track.parent_title.clone().unwrap_or_else(|| "Unknown Album".to_string()),
            parent_title: track.grandparent_title.clone(),
            parent_rating_key: track.grandparent_rating_key.clone(),
            year: None,
            thumb: track.parent_thumb.clone(),
            genre: Vec::new(),
            leaf_count: None,
            duration: None,
            added_at: None,
            loudness_analysis_version: None,
            last_viewed_at: Some(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)),
        })
    }
}

/// Track metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    #[serde(default)]
    pub rating_key: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub parent_title: Option<String>,
    #[serde(default)]
    pub grandparent_title: Option<String>,
    #[serde(default)]
    pub parent_rating_key: Option<String>,
    #[serde(default)]
    pub grandparent_rating_key: Option<String>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub index: Option<u32>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub duration: Option<u64>,
    #[serde(default, rename = "Media")]
    pub media: Vec<Media>,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub parent_thumb: Option<String>,
    #[serde(default)]
    pub grandparent_thumb: Option<String>,
}

impl Track {
    /// Get album name.
    pub fn album_name(&self) -> &str {
        self.parent_title.as_deref().unwrap_or("Unknown Album")
    }

    /// Get artist name.
    pub fn artist_name(&self) -> &str {
        self.grandparent_title.as_deref().unwrap_or("Unknown Artist")
    }

    /// Get track number.
    pub fn track_number(&self) -> u32 {
        self.index.unwrap_or(0)
    }

    /// Get duration in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        self.duration.unwrap_or(0)
    }

    /// Get the first available media part for streaming.
    pub fn stream_part(&self) -> Option<&MediaPart> {
        self.media.first().and_then(|m| m.part.first())
    }

    /// Get best thumbnail (track > album > artist).
    pub fn best_thumb(&self) -> Option<&str> {
        self.thumb
            .as_deref()
            .or(self.parent_thumb.as_deref())
            .or(self.grandparent_thumb.as_deref())
    }
}

/// Media container for a track.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub id: Option<u64>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub duration: Option<u64>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub bitrate: Option<u32>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub audio_channels: Option<u8>,
    #[serde(default)]
    pub audio_codec: Option<String>,
    #[serde(default)]
    pub container: Option<String>,
    #[serde(default, rename = "Part")]
    pub part: Vec<MediaPart>,
}

/// Individual media file/part.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaPart {
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub id: Option<u64>,
    #[serde(default)]
    pub key: String,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub duration: Option<u64>,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub size: Option<u64>,
    #[serde(default)]
    pub container: Option<String>,
}

/// Genre tag.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GenreTag {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub tag: String,
}

/// Response wrapper for artists.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ArtistsResponse {
    pub media_container: ArtistsContainer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistsContainer {
    #[serde(default)]
    pub size: u32,
    #[serde(default)]
    pub total_size: Option<u32>,
    #[serde(default, rename = "Metadata")]
    pub metadata: Vec<Artist>,
}

/// Response wrapper for albums.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AlbumsResponse {
    pub media_container: AlbumsContainer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumsContainer {
    #[serde(default)]
    pub size: u32,
    #[serde(default)]
    pub total_size: Option<u32>,
    #[serde(default, rename = "Metadata")]
    pub metadata: Vec<Album>,
}

/// Response wrapper for tracks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TracksResponse {
    pub media_container: TracksContainer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TracksContainer {
    #[serde(default, rename = "Metadata")]
    pub metadata: Vec<Track>,
}
