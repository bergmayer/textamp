//! Path utilities for XDG-compliant directory resolution.
//!
//! Provides cache and data directory resolution following XDG Base Directory spec.
//! Used by both plex module (for portability) and config module.

use std::path::PathBuf;

/// Get the cache directory for an application.
///
/// Checks $XDG_CACHE_HOME first, then falls back to platform defaults:
/// - Linux: ~/.cache/{app_name}
/// - macOS/Windows: platform cache dir/{app_name}
pub fn get_cache_dir(app_name: &str) -> Option<PathBuf> {
    // Check XDG env var first (works on both macOS and Linux)
    if let Ok(xdg_cache) = std::env::var("XDG_CACHE_HOME") {
        return Some(PathBuf::from(xdg_cache).join(app_name));
    }

    // Fall back to platform default
    #[cfg(target_os = "linux")]
    {
        dirs::home_dir().map(|h| h.join(".cache").join(app_name))
    }

    #[cfg(not(target_os = "linux"))]
    {
        dirs::cache_dir().map(|p| p.join(app_name))
    }
}

/// Get the data directory for an application.
///
/// Checks $XDG_DATA_HOME first, then falls back to platform defaults.
pub fn get_data_dir(app_name: &str) -> Option<PathBuf> {
    // Check XDG env var first
    if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
        return Some(PathBuf::from(xdg_data).join(app_name));
    }

    // Fall back to platform default
    #[cfg(target_os = "linux")]
    {
        dirs::home_dir().map(|h| h.join(".local/share").join(app_name))
    }

    #[cfg(not(target_os = "linux"))]
    {
        dirs::data_dir().map(|p| p.join(app_name))
    }
}

/// Get the config directory for an application.
///
/// Checks $XDG_CONFIG_HOME first, then falls back to platform defaults.
pub fn get_config_dir(app_name: &str) -> Option<PathBuf> {
    // Check XDG env var first
    if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg_config).join(app_name));
    }

    // Fall back to platform default
    #[cfg(target_os = "linux")]
    {
        dirs::home_dir().map(|h| h.join(".config").join(app_name))
    }

    #[cfg(not(target_os = "linux"))]
    {
        dirs::config_dir().map(|p| p.join(app_name))
    }
}

/// Get the state directory for an application.
///
/// Checks $XDG_STATE_HOME first, then falls back to platform defaults.
/// On non-Linux platforms, falls back to data_dir.
pub fn get_state_dir(app_name: &str) -> Option<PathBuf> {
    // Check XDG env var first
    if let Ok(xdg_state) = std::env::var("XDG_STATE_HOME") {
        return Some(PathBuf::from(xdg_state).join(app_name));
    }

    // Fall back to platform default
    #[cfg(target_os = "linux")]
    {
        dirs::home_dir().map(|h| h.join(".local/state").join(app_name))
    }

    #[cfg(not(target_os = "linux"))]
    {
        // On macOS/Windows, use data_dir for state (no native equivalent)
        dirs::data_dir().map(|p| p.join(app_name))
    }
}
