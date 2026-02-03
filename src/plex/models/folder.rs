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

// ============================================================================
// Folder Navigation Types (used by cache and UI)
// ============================================================================

/// Type of item in folder view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FolderItemType {
    /// A directory/folder
    Folder,
    /// A playable track
    Track,
}

/// An item in the folder view (either a folder or track).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderItem {
    /// Plex key for navigation/API calls
    pub key: String,
    /// Display title
    pub title: String,
    /// Type of item
    pub item_type: FolderItemType,
    /// Rating key (for tracks)
    pub rating_key: Option<String>,
    /// Duration in milliseconds (for tracks)
    pub duration_ms: Option<u64>,
}

impl FolderItem {
    /// Create a new folder item.
    pub fn folder(key: String, title: String) -> Self {
        Self {
            key,
            title,
            item_type: FolderItemType::Folder,
            rating_key: None,
            duration_ms: None,
        }
    }

    /// Create a new track item.
    pub fn track(key: String, title: String, rating_key: String, duration_ms: Option<u64>) -> Self {
        Self {
            key,
            title,
            item_type: FolderItemType::Track,
            rating_key: Some(rating_key),
            duration_ms,
        }
    }

    /// Check if this is a folder.
    pub fn is_folder(&self) -> bool {
        self.item_type == FolderItemType::Folder
    }

    /// Check if this is a track.
    pub fn is_track(&self) -> bool {
        self.item_type == FolderItemType::Track
    }
}

/// A single column in the Miller columns view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderColumn {
    /// Key for this folder (None for root)
    pub key: Option<String>,
    /// Display title
    pub title: String,
    /// Items in this column
    pub items: Vec<FolderItem>,
    /// Currently selected index
    pub selected_index: usize,
}

impl FolderColumn {
    /// Create a new column.
    pub fn new(key: Option<String>, title: String, items: Vec<FolderItem>) -> Self {
        Self {
            key,
            title,
            items,
            selected_index: 0,
        }
    }

    /// Get the selected item, if any.
    pub fn selected_item(&self) -> Option<&FolderItem> {
        self.items.get(self.selected_index)
    }
}

/// Navigation state for folder browsing (Miller columns style).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FolderNavigationState {
    /// Which library this folder state belongs to (for cache validation)
    #[serde(default)]
    pub library_key: String,
    /// Columns from left to right (root is first)
    pub columns: Vec<FolderColumn>,
    /// Which column currently has focus (0-indexed)
    pub focused_column: usize,
    /// Loading indicator
    pub loading: bool,
}

impl FolderNavigationState {
    /// Create a new empty folder navigation state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new folder navigation state for a specific library.
    pub fn for_library(library_key: String) -> Self {
        Self {
            library_key,
            ..Default::default()
        }
    }

    /// Get the focused column.
    pub fn focused(&self) -> Option<&FolderColumn> {
        self.columns.get(self.focused_column)
    }

    /// Get the focused column mutably.
    pub fn focused_mut(&mut self) -> Option<&mut FolderColumn> {
        self.columns.get_mut(self.focused_column)
    }

    /// Get the selected item in the focused column.
    pub fn selected_item(&self) -> Option<&FolderItem> {
        self.focused().and_then(|c| c.selected_item())
    }

    /// Check if we can go left (focus previous column).
    pub fn can_go_left(&self) -> bool {
        self.focused_column > 0
    }

    /// Check if focus is at root column.
    pub fn is_at_root(&self) -> bool {
        self.focused_column == 0
    }

    /// Move focus left.
    pub fn focus_left(&mut self) {
        if self.focused_column > 0 {
            self.focused_column -= 1;
        }
    }

    /// Move focus right (if there's a column to the right).
    pub fn focus_right(&mut self) -> bool {
        if self.focused_column + 1 < self.columns.len() {
            self.focused_column += 1;
            true
        } else {
            false
        }
    }

    /// Add a new column to the right, removing any columns after current focus.
    pub fn push_column(&mut self, column: FolderColumn) {
        // Remove columns to the right of focus
        self.truncate_right_columns();
        // Add new column
        self.columns.push(column);
        // Move focus to new column
        self.focused_column = self.columns.len() - 1;
    }

    /// Clear columns to the right of the focused column.
    /// Call this when selection changes to prevent stale column data.
    pub fn truncate_right_columns(&mut self) {
        self.columns.truncate(self.focused_column + 1);
    }

    /// Get the current folder's key (focused column's key).
    pub fn current_folder_key(&self) -> Option<&str> {
        self.focused().and_then(|c| c.key.as_deref())
    }

    /// Navigate up in current column.
    pub fn move_up(&mut self) {
        if let Some(col) = self.focused_mut() {
            if col.selected_index > 0 {
                col.selected_index -= 1;
            }
        }
    }

    /// Navigate down in current column.
    pub fn move_down(&mut self) {
        if let Some(col) = self.focused_mut() {
            let max = col.items.len().saturating_sub(1);
            if col.selected_index < max {
                col.selected_index += 1;
            }
        }
    }

    /// Get the number of visible columns (for layout).
    pub fn visible_columns(&self) -> usize {
        self.columns.len()
    }
}
