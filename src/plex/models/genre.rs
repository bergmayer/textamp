//! Genre data models.

use serde::{Deserialize, Serialize};

/// A music genre from the library.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Genre {
    /// Unique key for this genre (e.g., "/library/sections/1/all?genre=123")
    #[serde(default)]
    pub key: String,
    /// Display title (some Plex versions use "tag" instead)
    #[serde(default)]
    pub title: String,
    /// Alternative title field (some Plex responses use this)
    #[serde(default)]
    pub tag: Option<String>,
    /// Alternative key path (Plex sometimes uses this)
    #[serde(default)]
    pub fast_key: Option<String>,
    /// Filter path for getting items in this genre
    #[serde(default)]
    pub filter: Option<String>,
    /// Number of items with this genre
    #[serde(default)]
    pub count: Option<u32>,
    /// Genre ID (some Plex versions return this as numeric)
    #[serde(default)]
    pub id: Option<String>,
    /// Numeric ratingKey (used by some Plex versions)
    #[serde(default)]
    pub rating_key: Option<String>,
}

impl Genre {
    /// Get the display title, preferring title over tag.
    pub fn display_title(&self) -> &str {
        if !self.title.is_empty() {
            &self.title
        } else if let Some(ref tag) = self.tag {
            tag
        } else {
            "Unknown Genre"
        }
    }

    /// Get the best key to use for fetching genre tracks.
    /// Extracts genre ID from various possible field formats.
    pub fn effective_key(&self) -> &str {
        // Try key first - it might be a full path or just an ID
        if !self.key.is_empty() {
            &self.key
        } else if let Some(ref fk) = self.fast_key {
            fk
        } else if let Some(ref rk) = self.rating_key {
            rk
        } else if let Some(ref id) = self.id {
            id
        } else {
            ""
        }
    }
}

/// Response wrapper for genres.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GenresResponse {
    pub media_container: GenresContainer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenresContainer {
    #[serde(default)]
    pub size: u32,
    #[serde(default, rename = "Directory")]
    pub directory: Vec<Genre>,
}
