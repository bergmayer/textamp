//! Folder navigation state shared by local files and WebDAV.
//!
//! These models represent the response from folder-based browsing endpoints.

use crate::miller::{MillerColumn, MillerState};
use serde::{Deserialize, Serialize};

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
    /// server key for navigation/API calls
    pub key: String,
    /// Display title
    pub title: String,
    /// Type of item
    pub item_type: FolderItemType,
    /// Rating key (for tracks)
    pub rating_key: Option<String>,
    /// Duration in milliseconds (for tracks)
    pub duration_ms: Option<u64>,
    /// Filesystem path (for folders, from server API)
    #[serde(default)]
    pub path: Option<String>,
    /// Parent (album) rating key (for tracks)
    #[serde(default)]
    pub parent_rating_key: Option<String>,
    /// Grandparent (artist) rating key (for tracks)
    #[serde(default)]
    pub grandparent_rating_key: Option<String>,
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
            path: None,
            parent_rating_key: None,
            grandparent_rating_key: None,
        }
    }

    /// Create a new folder item with filesystem path.
    pub fn folder_with_path(key: String, title: String, path: Option<String>) -> Self {
        Self {
            key,
            title,
            item_type: FolderItemType::Folder,
            rating_key: None,
            duration_ms: None,
            path,
            parent_rating_key: None,
            grandparent_rating_key: None,
        }
    }

    /// Create a new track item.
    pub fn track(
        key: String,
        title: String,
        rating_key: String,
        duration_ms: Option<u64>,
        parent_rating_key: Option<String>,
        grandparent_rating_key: Option<String>,
    ) -> Self {
        Self {
            key,
            title,
            item_type: FolderItemType::Track,
            rating_key: Some(rating_key),
            duration_ms,
            path: None,
            parent_rating_key,
            grandparent_rating_key,
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
    /// Original items before shuffle (None if not shuffled)
    #[serde(skip)]
    original_items: Option<Vec<FolderItem>>,
}

impl FolderColumn {
    /// Create a new column.
    pub fn new(key: Option<String>, title: String, items: Vec<FolderItem>) -> Self {
        Self {
            key,
            title,
            items,
            selected_index: 0,
            original_items: None,
        }
    }

    /// Get the selected item, if any.
    pub fn selected_item(&self) -> Option<&FolderItem> {
        self.items.get(self.selected_index)
    }

    /// Whether this column is currently shuffled.
    pub fn is_shuffled(&self) -> bool {
        self.original_items.is_some()
    }

    /// Shuffle items. Saves originals for restore.
    pub fn shuffle(&mut self) {
        use rand::seq::SliceRandom;
        self.original_items = Some(self.items.clone());
        let mut rng = rand::rng();
        self.items.shuffle(&mut rng);
        self.selected_index = 0;
    }

    /// Restore original order.
    pub fn unshuffle(&mut self) {
        if let Some(items) = self.original_items.take() {
            self.items = items;
        }
        self.selected_index = 0;
    }

    /// Get items in their original (unshuffled) order for cache persistence.
    pub fn unshuffled_items(&self) -> &[FolderItem] {
        self.original_items.as_deref().unwrap_or(&self.items)
    }
}

impl MillerColumn for FolderColumn {
    fn item_count(&self) -> usize {
        self.items.len()
    }
    fn selected_index(&self) -> usize {
        self.selected_index
    }
    fn set_selected_index(&mut self, idx: usize) {
        self.selected_index = idx;
    }
}

/// Navigation state for folder browsing (Miller columns style).
///
/// Wraps `MillerState<FolderColumn>` with an additional `library_key` field.
/// Uses `Deref`/`DerefMut` so all `MillerState` methods are accessible directly.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FolderNavigationState {
    /// Which library this folder state belongs to (for cache validation)
    #[serde(default)]
    pub library_key: String,
    /// Inner Miller column state.
    #[serde(flatten)]
    pub inner: MillerState<FolderColumn>,
}

impl std::ops::Deref for FolderNavigationState {
    type Target = MillerState<FolderColumn>;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl std::ops::DerefMut for FolderNavigationState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
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

    /// Create a folder navigation state with a root column.
    pub fn with_root(library_key: String, root_column: FolderColumn) -> Self {
        Self {
            library_key,
            inner: MillerState {
                columns: vec![root_column],
                focused_column: 0,
                loading: false,
            },
        }
    }

    /// Get the selected item in the focused column.
    pub fn selected_item(&self) -> Option<&FolderItem> {
        self.focused().and_then(|c| c.selected_item())
    }

    /// Get the current folder's key (focused column's key).
    pub fn current_folder_key(&self) -> Option<&str> {
        self.focused().and_then(|c| c.key.as_deref())
    }

    /// Backward-compatible alias for `truncate_right()`.
    pub fn truncate_right_columns(&mut self) {
        self.truncate_right();
    }

    /// Get the number of visible columns (for layout).
    pub fn visible_columns(&self) -> usize {
        self.columns.len()
    }

    /// No-op kept for backward compatibility. Previously this padded
    /// the folder nav with a synthetic empty column so the view
    /// always rendered two cols, but the cat col now fills that
    /// "second slot" naturally. Adding the padding column made the
    /// browse view render three panes (cat + root + empty) on first
    /// click into Folders.
    pub fn ensure_placeholder(&mut self) {}
}
