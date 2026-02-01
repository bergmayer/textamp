//! Library-related models.

use serde::{Deserialize, Serialize};

/// Plex library section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    #[serde(rename = "key")]
    pub key: String,
    pub title: String,
    #[serde(rename = "type")]
    pub library_type: String,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub scanner: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
}

impl Library {
    /// Check if this is a music library.
    pub fn is_music(&self) -> bool {
        self.library_type == "artist"
    }
}

/// Library type enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryType {
    Artist,
    Movie,
    Show,
    Photo,
    Other,
}

impl From<&str> for LibraryType {
    fn from(s: &str) -> Self {
        match s {
            "artist" => LibraryType::Artist,
            "movie" => LibraryType::Movie,
            "show" => LibraryType::Show,
            "photo" => LibraryType::Photo,
            _ => LibraryType::Other,
        }
    }
}

/// Response wrapper for library sections.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LibrarySectionsResponse {
    pub media_container: LibrarySectionsContainer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LibrarySectionsContainer {
    #[serde(default, rename = "Directory")]
    pub directory: Vec<Library>,
}
