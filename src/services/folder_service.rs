//! Folder navigation service.
//!
//! Handles folder browsing logic independent of UI.

use serde::{Deserialize, Serialize};
use crate::api::models::FolderResponse;

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
        self.columns.truncate(self.focused_column + 1);
        // Add new column
        self.columns.push(column);
        // Move focus to new column
        self.focused_column = self.columns.len() - 1;
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

/// Service for folder navigation logic.
pub struct FolderService;

impl FolderService {
    /// Convert a FolderResponse from the API to a list of FolderItems.
    pub fn from_response(response: &FolderResponse) -> Vec<FolderItem> {
        let mut items = Vec::new();

        // Add directories as folders
        for dir in &response.media_container.directories {
            items.push(FolderItem::folder(dir.key.clone(), dir.title.clone()));
        }

        // Add metadata items as tracks (only if they have a rating_key)
        for meta in &response.media_container.metadata {
            if let Some(ref rating_key) = meta.rating_key {
                items.push(FolderItem::track(
                    meta.key.clone(),
                    meta.title.clone(),
                    rating_key.clone(),
                    meta.duration,
                ));
            } else {
                // Items without rating_key are likely containers, treat as folders
                items.push(FolderItem::folder(meta.key.clone(), meta.title.clone()));
            }
        }

        // Sort: folders first, then tracks, both alphabetically
        Self::sort_items(&mut items);
        items
    }

    /// Sort folder items: folders first, then tracks, both ASCIIbetically by title.
    pub fn sort_items(items: &mut Vec<FolderItem>) {
        items.sort_by(|a, b| {
            // Folders come before tracks
            match (&a.item_type, &b.item_type) {
                (FolderItemType::Folder, FolderItemType::Track) => std::cmp::Ordering::Less,
                (FolderItemType::Track, FolderItemType::Folder) => std::cmp::Ordering::Greater,
                _ => a.title.cmp(&b.title), // Same type: sort ASCIIbetically
            }
        });
    }

    /// Get only the tracks from a list of items, sorted ASCIIbetically.
    pub fn extract_tracks(items: &[FolderItem]) -> Vec<&FolderItem> {
        let mut tracks: Vec<_> = items.iter().filter(|i| i.is_track()).collect();
        tracks.sort_by(|a, b| a.title.cmp(&b.title));
        tracks
    }
}
