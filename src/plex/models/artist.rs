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
    /// Artist biography/summary from Plex.
    #[serde(default)]
    pub summary: Option<String>,
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
    /// Album subtype from Plex (e.g., "compilation" for compilation albums).
    #[serde(default)]
    pub subtype: Option<String>,
}

impl Album {
    /// Get track count (leaf_count in Plex API).
    pub fn track_count(&self) -> u32 {
        self.leaf_count.unwrap_or(0)
    }

    /// Whether this album is a candidate for being a compilation.
    /// Returns true if the Plex subtype is "compilation" or the artist name
    /// matches common compilation artist names.
    pub fn is_compilation_candidate(&self) -> bool {
        if self.subtype.as_deref() == Some("compilation") {
            return true;
        }
        let name = self.artist_name().to_lowercase();
        name == "various artists" || name == "various"
    }

    /// Get artist name (handles None and empty string).
    pub fn artist_name(&self) -> &str {
        match self.parent_title.as_deref() {
            Some(s) if !s.is_empty() => s,
            _ => "Unknown Artist",
        }
    }

    /// Create an Album stub from a Track's parent info.
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
            subtype: None,
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
    pub year: Option<u16>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub parent_year: Option<u16>,
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
    /// Track-level artist name (used by Plex for compilation tracks).
    #[serde(default)]
    pub original_title: Option<String>,
}

impl Track {
    /// Get track-level artist (original_title), falling back to album artist.
    pub fn track_artist(&self) -> &str {
        match self.original_title.as_deref() {
            Some(s) if !s.is_empty() => s,
            _ => self.artist_name(),
        }
    }

    /// Get album name (handles None and empty string).
    pub fn album_name(&self) -> &str {
        match self.parent_title.as_deref() {
            Some(s) if !s.is_empty() => s,
            _ => "Unknown Album",
        }
    }

    /// Get artist name (handles None and empty string).
    pub fn artist_name(&self) -> &str {
        match self.grandparent_title.as_deref() {
            Some(s) if !s.is_empty() => s,
            _ => "Unknown Artist",
        }
    }

    /// Get filename from the first media part's file path (with extension).
    pub fn file_name(&self) -> Option<&str> {
        self.media.first()
            .and_then(|m| m.part.first())
            .and_then(|p| p.file.as_deref())
            .and_then(|f| f.rsplit('/').next())
            // Also handle Windows-style backslash paths
            .map(|f| f.rsplit('\\').next().unwrap_or(f))
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
