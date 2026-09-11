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

pub mod artist_alias_service;
mod browse_drill;
pub mod compilations;
pub mod external_search;
mod folder_service;

mod list_filter_service;
mod navigation_service;
mod playback_service;
pub mod radio;
mod search_filter_service;
pub mod track_context;

pub use browse_drill::{plan_drill, ClickContext, DrillPlan};
pub use folder_service::{
    FolderColumn, FolderItem, FolderItemType, FolderNavigationState, FolderService,
};

pub use list_filter_service::{
    browse_filter_records, filter_browse_items, filter_browse_records, filter_folder_items,
    filter_stations, filter_with_priority, search_albums_with_ranking, search_tracks_with_ranking,
    search_with_ranking, BrowseFilterRecord, DEFAULT_MAX_RESULTS,
};
pub use navigation_service::NavigationService;
pub use playback_service::{shuffle_queue, MAX_HISTORY_SIZE};
pub use search_filter_service::{FilteredItem, SearchFilterService};

// Re-export waveform from media module for backward compatibility
pub use crate::media::{
    generate_waveform, generate_waveform_from_pcm, WaveformCache, WaveformData, WaveformError,
};
// Re-export spectrogram from media module
pub use crate::media::{
    generate_spectrogram, generate_spectrogram_from_pcm, SpectrogramCache, SpectrogramData,
};
pub mod biography;
