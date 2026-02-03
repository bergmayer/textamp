//! XDG Base Directory Specification paths with platform-aware fallbacks.
//!
//! On macOS and Linux, checks for XDG environment variables first.
//! If not set, falls back to platform-specific defaults:
//! - Linux: ~/.config, ~/.cache, ~/.local/share, ~/.local/state
//! - macOS: ~/Library/Application Support, ~/Library/Caches, etc.
//!
//! Uses shared utility functions from crate::util::paths.

use crate::util::paths;
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
            config_dir: paths::get_config_dir(app_name).unwrap_or_else(|| PathBuf::from(".config").join(app_name)),
            data_dir: paths::get_data_dir(app_name).unwrap_or_else(|| PathBuf::from(".local/share").join(app_name)),
            cache_dir: paths::get_cache_dir(app_name).unwrap_or_else(|| PathBuf::from(".cache").join(app_name)),
            state_dir: paths::get_state_dir(app_name).unwrap_or_else(|| PathBuf::from(".local/state").join(app_name)),
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
