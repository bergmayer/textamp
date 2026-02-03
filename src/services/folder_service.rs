//! Folder navigation service.
//!
//! Handles folder browsing logic independent of UI.

// Re-export types from plex module for backward compatibility
pub use crate::plex::models::{FolderColumn, FolderItem, FolderItemType, FolderNavigationState};

use crate::plex::models::FolderResponse;

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
