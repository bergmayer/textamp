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
//! When porting to other platforms (iOS, Web), these services can be
//! reused directly via FFI or compiled to the target platform.

mod adventure;
mod cache_service;
mod folder_service;
mod library_service;
mod list_filter_service;
mod navigation_service;
mod playback_service;
mod preload_service;
mod search_filter_service;
mod selection_service;

pub use adventure::generate_adventure;
pub use cache_service::{CacheDataSources, CacheSaveConditions, CacheService, CACHE_IDLE_THRESHOLD_SECS, CACHE_SAVE_INTERVAL_SECS};
pub use folder_service::{FolderColumn, FolderItem, FolderItemType, FolderNavigationState, FolderService};
pub use library_service::LibraryService;
pub use list_filter_service::{filter_with_priority, filter_browse_items, filter_folder_items, filter_stations, DEFAULT_MAX_RESULTS};
pub use navigation_service::NavigationService;
pub use playback_service::{PlaybackService, QueueManager, NavigationResult, MAX_HISTORY_SIZE};
pub use preload_service::{ConnectionParams, PreloadService};
pub use search_filter_service::{FilteredItem, SearchFilterService};
pub use selection_service::{SelectionContext, SelectionService, SimilarSource};

// Re-export waveform from plex module for backward compatibility
pub use crate::plex::{WaveformCache, WaveformData, WaveformError, generate_waveform};
