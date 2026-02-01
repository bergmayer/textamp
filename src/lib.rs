//! textamp - A keyboard-driven TUI client for Plex Music.
//!
//! This library provides the core functionality for the textamp application.

pub mod app;
pub mod audio;
pub mod config;
pub mod plex;
pub mod services;
pub mod ui;
pub mod util;

// Backward-compatible aliases for existing code
// This allows gradual migration to the new plex module structure

/// Alias for the plex module (backward compatibility).
pub mod api {
    pub use crate::plex::constants;
    pub use crate::plex::models;
    pub use crate::plex::{ApiError, PlexAuth, PlexClient, PlexClientInfo};
}

/// Alias for the library cache (backward compatibility).
pub mod cache {
    pub use crate::plex::{CacheData, LibraryCache};
}
