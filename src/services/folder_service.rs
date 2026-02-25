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

        // Add directories as folders (with filesystem path if available)
        for dir in &response.media_container.directories {
            // Use path as title when the API returns an empty or useless title (e.g. "?" for drive roots)
            let title = if dir.title.is_empty() || dir.title == "?" {
                match dir.path.as_deref().filter(|p| !p.is_empty()) {
                    Some(path) => path.to_string(),
                    None => continue, // Skip entries with no usable title or path
                }
            } else {
                dir.title.clone()
            };
            items.push(FolderItem::folder_with_path(dir.key.clone(), title, dir.path.clone()));
        }

        // Add metadata items as tracks (only if they have a rating_key)
        for meta in &response.media_container.metadata {
            if let Some(ref rating_key) = meta.rating_key {
                // Use the actual file name (with extension) instead of ID3 tag title
                // Split on both / and \ to handle Windows and Unix paths
                let title = meta.media.first()
                    .and_then(|m| m.parts.first())
                    .and_then(|p| p.file.as_deref())
                    .and_then(|path| path.rsplit(|c| c == '/' || c == '\\').next())
                    .filter(|name| !name.is_empty())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| meta.title.clone());
                items.push(FolderItem::track(
                    meta.key.clone(),
                    title,
                    rating_key.clone(),
                    meta.duration,
                    meta.parent_rating_key.clone(),
                    meta.grandparent_rating_key.clone(),
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

    /// Extract the filesystem path of a folder from the API response.
    ///
    /// Tries: subdirectory path parent, then track file path parent, then title2.
    pub fn folder_path(response: &FolderResponse) -> Option<String> {
        // Try to get path from the first subdirectory's path field (parent of the subdir)
        if let Some(dir) = response.media_container.directories.first() {
            if let Some(ref dir_path) = dir.path {
                // dir_path is the subdir's full path; we want its parent (this folder's path)
                if let Some(pos) = dir_path.rfind(|c: char| c == '/' || c == '\\') {
                    let parent = &dir_path[..pos];
                    if !parent.is_empty() {
                        return Some(parent.to_string());
                    }
                }
            }
        }
        // Try to get path from the first track's file path (parent directory)
        if let Some(meta) = response.media_container.metadata.first() {
            if let Some(file) = meta.media.first()
                .and_then(|m| m.parts.first())
                .and_then(|p| p.file.as_deref())
            {
                if let Some(pos) = file.rfind(|c: char| c == '/' || c == '\\') {
                    let parent = &file[..pos];
                    if !parent.is_empty() {
                        return Some(parent.to_string());
                    }
                }
            }
        }
        // Fall back to title2 from the API response
        response.media_container.title2.clone()
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

    /// Filter out folder items with useless titles (empty or "?" from Plex drive roots).
    pub fn filter_invalid(items: Vec<FolderItem>) -> Vec<FolderItem> {
        items.into_iter().filter(|item| {
            !item.is_folder() || (!item.title.is_empty() && item.title != "?")
        }).collect()
    }

    /// Get only the tracks from a list of items, sorted ASCIIbetically.
    pub fn extract_tracks(items: &[FolderItem]) -> Vec<&FolderItem> {
        let mut tracks: Vec<_> = items.iter().filter(|i| i.is_track()).collect();
        tracks.sort_by(|a, b| a.title.cmp(&b.title));
        tracks
    }
}
