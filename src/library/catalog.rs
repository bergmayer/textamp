//! Shared catalog metadata. Wire field names preserve existing cache/config compatibility;
//! transports convert into these models and keep response envelopes in their own modules.
use crate::util::serde_helpers::from_str_or_num_opt;
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
    /// Provider biography/summary.
    #[serde(default)]
    pub summary: Option<String>,
    /// "Similar" metadata tags from server (artist detail only).
    #[serde(default, rename = "Similar")]
    pub similar: Vec<GenreTag>,
}

/// Album metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    #[serde(default, rename = "Mood")]
    pub mood: Vec<GenreTag>,
    #[serde(default)]
    pub originally_available_at: Option<String>,
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
    /// Provider album subtype (e.g., "compilation").
    #[serde(default)]
    pub subtype: Option<String>,
}

impl Album {
    /// Get track count (leaf_count in server API).
    pub fn track_count(&self) -> u32 {
        self.leaf_count.unwrap_or(0)
    }

    /// Whether this album is a candidate for being a compilation.
    /// Returns true if the subtype is "compilation" or the artist name
    /// matches common compilation artist names.
    pub fn is_compilation_candidate(&self) -> bool {
        if self
            .subtype
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("compilation"))
        {
            return true;
        }
        let name = self.artist_name().trim().to_lowercase();
        name == "various artists" || name == "various"
    }

    /// Get artist name (handles None and empty string).
    pub fn artist_name(&self) -> &str {
        match self.parent_title.as_deref() {
            Some(s) if !s.is_empty() => s,
            _ => "Unknown Artist",
        }
    }
}

/// Genre tag.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GenreTag {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub tag: String,
}

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
