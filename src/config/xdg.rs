//! XDG Base Directory Specification paths with platform-aware fallbacks.
//!
//! On macOS and Linux, checks for XDG environment variables first.
//! If not set, falls back to platform-specific defaults:
//! - Linux: ~/.config, ~/.cache, ~/.local/share, ~/.local/state
//! - macOS: ~/Library/Application Support, ~/Library/Caches, etc.

use std::path::PathBuf;

/// XDG-compliant directory paths for the application.
pub struct XdgPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub state_dir: PathBuf,
}

impl XdgPaths {
    /// Create XDG paths for the given application name.
    ///
    /// Checks XDG environment variables first, then falls back to platform defaults.
    pub fn new(app_name: &str) -> Self {
        Self {
            config_dir: get_config_dir(app_name),
            data_dir: get_data_dir(app_name),
            cache_dir: get_cache_dir(app_name),
            state_dir: get_state_dir(app_name),
        }
    }

    /// Path to the main config file.
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.yaml")
    }

    /// Path to the auth token storage.
    pub fn token_file(&self) -> PathBuf {
        self.data_dir.join("auth.yaml")
    }

    /// Path to the log file.
    pub fn log_file(&self) -> PathBuf {
        self.state_dir.join("textamp.log")
    }

    /// Path to the image cache directory.
    pub fn image_cache_dir(&self) -> PathBuf {
        self.cache_dir.join("images")
    }

    /// Ensure all directories exist.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.config_dir)?;
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.cache_dir)?;
        std::fs::create_dir_all(&self.state_dir)?;
        Ok(())
    }
}

/// Get config directory: $XDG_CONFIG_HOME/app_name or platform default.
fn get_config_dir(app_name: &str) -> PathBuf {
    // Check XDG env var first (works on both macOS and Linux)
    if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg_config).join(app_name);
    }

    // Fall back to platform default
    #[cfg(target_os = "linux")]
    {
        dirs::home_dir()
            .map(|h| h.join(".config"))
            .unwrap_or_else(|| PathBuf::from(".config"))
            .join(app_name)
    }

    #[cfg(not(target_os = "linux"))]
    {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from(".config"))
            .join(app_name)
    }
}

/// Get data directory: $XDG_DATA_HOME/app_name or platform default.
fn get_data_dir(app_name: &str) -> PathBuf {
    // Check XDG env var first
    if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg_data).join(app_name);
    }

    // Fall back to platform default
    #[cfg(target_os = "linux")]
    {
        dirs::home_dir()
            .map(|h| h.join(".local/share"))
            .unwrap_or_else(|| PathBuf::from(".local/share"))
            .join(app_name)
    }

    #[cfg(not(target_os = "linux"))]
    {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from(".local/share"))
            .join(app_name)
    }
}

/// Get cache directory: $XDG_CACHE_HOME/app_name or platform default.
fn get_cache_dir(app_name: &str) -> PathBuf {
    // Check XDG env var first
    if let Ok(xdg_cache) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(xdg_cache).join(app_name);
    }

    // Fall back to platform default
    #[cfg(target_os = "linux")]
    {
        dirs::home_dir()
            .map(|h| h.join(".cache"))
            .unwrap_or_else(|| PathBuf::from(".cache"))
            .join(app_name)
    }

    #[cfg(not(target_os = "linux"))]
    {
        dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from(".cache"))
            .join(app_name)
    }
}

/// Get state directory: $XDG_STATE_HOME/app_name or platform default.
fn get_state_dir(app_name: &str) -> PathBuf {
    // Check XDG env var first
    if let Ok(xdg_state) = std::env::var("XDG_STATE_HOME") {
        return PathBuf::from(xdg_state).join(app_name);
    }

    // Fall back to platform default
    // Note: state_dir is Linux-specific in XDG spec, macOS/Windows use data_dir
    #[cfg(target_os = "linux")]
    {
        dirs::home_dir()
            .map(|h| h.join(".local/state"))
            .unwrap_or_else(|| PathBuf::from(".local/state"))
            .join(app_name)
    }

    #[cfg(not(target_os = "linux"))]
    {
        // On macOS/Windows, use data_dir for state (no native equivalent)
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from(".local/state"))
            .join(app_name)
    }
}
