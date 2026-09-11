//! Folder navigation service.
//!
//! Handles folder browsing logic independent of UI.

// Re-export types from media module for backward compatibility
pub use crate::library::models::{FolderColumn, FolderItem, FolderItemType, FolderNavigationState};

/// Service for folder navigation logic.
pub struct FolderService;

impl FolderService {
    /// Sort folder items: folders first, then tracks, both ASCIIbetically by title.
    pub fn sort_items(items: &mut [FolderItem]) {
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
