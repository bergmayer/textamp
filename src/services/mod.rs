//! Service layer for business logic.
//!
//! Services contain reusable business logic that is independent of the UI.
//! This allows the same logic to be used with different frontends.
//!
//! # Cross-Platform Design
//!
//! All services in this module are designed to be:
//! - UI-agnostic: No imports from `ui` or terminal-specific code
//! - Stateless: Operate on data passed to them, return results
//! - Testable: Pure functions where possible
//!

pub mod artist_alias_service;
pub mod compilations;
pub mod external_search;
pub mod web_search;

mod list_filter_service;
mod navigation_service;
mod playback_service;
pub mod radio;
pub mod sonic_adventure;

pub use crate::library::models::{FolderColumn, FolderItem, FolderItemType, FolderNavigationState};

pub(crate) use list_filter_service::CommandSearch;
pub use list_filter_service::{
    BrowseFilterRecord, DEFAULT_MAX_RESULTS, ListFilterResults, filter_browse_records,
    filter_browse_records_until, filter_with_priority, filter_with_priority_until,
    search_albums_with_ranking, search_tracks_with_ranking, search_tracks_with_ranking_until,
    search_with_ranking,
};
pub use navigation_service::NavigationService;
pub use playback_service::shuffle_queue;

pub mod biography;
pub mod catalog_search;

pub mod release_dates;

pub mod playlist_export;

pub mod catalog_lookup;

pub mod singles;

pub mod downloads;

pub mod player_sync;
