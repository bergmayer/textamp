//! Folder browsing models for Plex API.
//!
//! These models represent the response from folder-based browsing endpoints.

use serde::{Deserialize, Serialize};

/// Response from /library/sections/{id}/folder endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FolderResponse {
    pub media_container: FolderContainer,
}

/// Container for folder contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FolderContainer {
    #[serde(default)]
    pub size: u32,

    /// Library name
    #[serde(default)]
    pub title1: Option<String>,

    /// Current folder name
    #[serde(default)]
    pub title2: Option<String>,

    /// Subdirectories in this folder
    #[serde(default, rename = "Directory")]
    pub directories: Vec<FolderDirectory>,

    /// Media items (tracks) in this folder
    #[serde(default, rename = "Metadata")]
    pub metadata: Vec<FolderMetadata>,
}

/// A subdirectory in folder view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderDirectory {
    /// Key for navigating into this directory
    pub key: String,

    /// Display title
    pub title: String,

    /// Filesystem path (if available)
    #[serde(default)]
    pub path: Option<String>,
}

/// A media item (track) in folder view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderMetadata {
    /// Rating key for this track (may be absent for non-track items)
    #[serde(default)]
    pub rating_key: Option<String>,

    /// API key/path
    pub key: String,

    /// Track title
    pub title: String,

    /// Duration in milliseconds
    #[serde(default)]
    pub duration: Option<u64>,

    /// Parent (album) title
    #[serde(default)]
    pub parent_title: Option<String>,

    /// Grandparent (artist) title
    #[serde(default)]
    pub grandparent_title: Option<String>,

    /// Track index
    #[serde(default)]
    pub index: Option<u32>,

    /// Media information
    #[serde(default, rename = "Media")]
    pub media: Vec<FolderMedia>,
}

/// Media container in folder view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderMedia {
    #[serde(default)]
    pub id: Option<u64>,

    #[serde(default)]
    pub duration: Option<u64>,

    #[serde(default)]
    pub bitrate: Option<u32>,

    #[serde(default)]
    pub audio_channels: Option<u32>,

    #[serde(default)]
    pub audio_codec: Option<String>,

    #[serde(default)]
    pub container: Option<String>,

    #[serde(default, rename = "Part")]
    pub parts: Vec<FolderMediaPart>,
}

/// Media part (file) in folder view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderMediaPart {
    #[serde(default)]
    pub id: Option<u64>,

    /// Streaming key
    #[serde(default)]
    pub key: Option<String>,

    /// Duration in milliseconds
    #[serde(default)]
    pub duration: Option<u64>,

    /// File path
    #[serde(default)]
    pub file: Option<String>,

    /// File size
    #[serde(default)]
    pub size: Option<u64>,

    /// Container format
    #[serde(default)]
    pub container: Option<String>,
}
