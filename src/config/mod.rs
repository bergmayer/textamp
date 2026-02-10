//! Configuration management for textamp.
//!
//! Handles XDG-compliant paths and YAML configuration loading.

mod settings;
mod xdg;

pub use settings::{Config, GeneralConfig, LibrariesConfig, LibrarySettings, PlaybackConfig, PlexConfig, UiConfig};
pub use xdg::XdgPaths;

use anyhow::{anyhow, Result};
use std::path::PathBuf;

/// Load configuration from XDG config directory.
///
/// Validates configuration values after loading and replaces
/// invalid values with defaults.
pub fn load_config() -> Result<Config> {
    let paths = XdgPaths::new("textamp");
    paths.ensure_dirs()?;

    let config_path = paths.config_file();

    let mut config = if config_path.exists() {
        let contents = std::fs::read_to_string(&config_path)?;
        serde_yaml::from_str(&contents)?
    } else {
        Config::default()
    };

    // Validate and fix invalid values
    validate_config(&mut config);

    Ok(config)
}

/// Validate config values and replace invalid ones with defaults.
fn validate_config(config: &mut Config) {
    // Volume must be 0.0-1.0
    if config.playback.default_volume < 0.0 || config.playback.default_volume > 1.0 {
        config.playback.default_volume = 0.8;
    }

    // Buffer size must be positive
    if config.playback.buffer_size_kb == 0 {
        config.playback.buffer_size_kb = 1024;
    }

    // Album art size must be reasonable
    if config.ui.album_art_size == 0 || config.ui.album_art_size > 200 {
        config.ui.album_art_size = 40;
    }

    // Log level must be valid
    let valid_levels = ["trace", "debug", "info", "warn", "error"];
    if !valid_levels.contains(&config.general.log_level.to_lowercase().as_str()) {
        config.general.log_level = "info".to_string();
    }
}

/// Get the path to the config file.
pub fn config_path() -> PathBuf {
    XdgPaths::new("textamp").config_file()
}

/// Get the path to the auth token file.
pub fn token_path() -> PathBuf {
    XdgPaths::new("textamp").token_file()
}

/// Get the path to the log file.
pub fn log_path() -> PathBuf {
    XdgPaths::new("textamp").log_file()
}

/// Save configuration to XDG config directory.
///
/// Uses atomic write pattern: writes to temp file first, then renames.
/// This prevents data corruption if the app crashes during write.
pub fn save_config(config: &Config) -> Result<()> {
    let paths = XdgPaths::new("textamp");
    paths.ensure_dirs()?;

    let config_file = paths.config_file();
    let yaml = serde_yaml::to_string(config)?;

    // Write to temp file in the same directory (ensures same filesystem for rename)
    let temp_file = config_file.with_extension("yaml.tmp");

    std::fs::write(&temp_file, &yaml).map_err(|e| {
        anyhow!("Failed to write temp config file: {}", e)
    })?;

    // Atomic rename
    std::fs::rename(&temp_file, &config_file).map_err(|e| {
        // Clean up temp file on failure
        let _ = std::fs::remove_file(&temp_file);
        anyhow!("Failed to save config: {}", e)
    })?;

    Ok(())
}
