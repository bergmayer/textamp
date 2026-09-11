//! Configuration management for textamp.
//!
//! Handles XDG-compliant paths and TOML configuration loading.

pub mod settings;
mod xdg;

pub use settings::{Config, GeneralConfig, PlaybackConfig, UiConfig};
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

    let mut migrated = false;
    let mut config = if config_path.exists() {
        let contents = std::fs::read_to_string(&config_path)?;
        match parse_config(&contents) {
            Ok((config, removed)) => {
                migrated = removed;
                config
            }
            Err(e) => {
                // A corrupt config must not brick startup. Preserve the
                // broken file (a later settings save would otherwise
                // overwrite it) and continue with defaults.
                let backup =
                    config_path.with_extension(format!("toml.{}.invalid", uuid::Uuid::new_v4()));
                std::fs::rename(&config_path, &backup).map_err(|error| {
                    anyhow!(
                        "Cannot preserve invalid config at {}: {error}",
                        config_path.display()
                    )
                })?;
                eprintln!(
                    "Warning: {} is not valid TOML ({}). Using default settings.",
                    config_path.display(),
                    e.message()
                );
                eprintln!("The unreadable file was moved to {}.", backup.display());
                Config::default()
            }
        }
    } else {
        Config::default()
    };

    // Validate and fix invalid values
    validate_config(&mut config);

    let mut source_ids = std::collections::HashSet::new();
    for source in &config.folder_sources {
        source.validate()?;
        if !source_ids.insert(&source.id) {
            return Err(anyhow!("Duplicate folder library ID: {}", source.id));
        }
    }
    if config
        .default_folder_source
        .as_ref()
        .is_some_and(|id| !source_ids.contains(id))
    {
        return Err(anyhow!("The default folder library is not configured"));
    }

    let mut nav_ids = std::collections::HashSet::new();
    for source in &config.navidrome_sources {
        source.validate()?;
        if !nav_ids.insert(&source.id) {
            return Err(anyhow!("Duplicate Navidrome account ID"));
        }
    }
    if config
        .default_navidrome
        .as_ref()
        .is_some_and(|selection| !nav_ids.contains(&selection.source_id))
    {
        return Err(anyhow!("The default Navidrome account is not configured"));
    }
    migrated |= canonicalize_navidrome(&mut config);
    if migrated {
        save_config(&config)?;
        tracing::info!("Migrated saved library selections and discontinued transports");
    }
    Ok(config)
}

/// Keep saved selections and per-library preferences on the same identity when
/// discovery reveals that the account has only one music folder.
pub fn canonicalize_navidrome(config: &mut Config) -> bool {
    let mut changed = false;
    if let Some(selection) = &mut config.default_navidrome {
        if let Some(source) = config
            .navidrome_sources
            .iter()
            .find(|s| s.id == selection.source_id)
        {
            let folder = source.canonical_folder(selection.folder.clone());
            changed |= folder != selection.folder;
            selection.folder = folder;
        }
    }
    config.sonic_disabled_libraries = config
        .sonic_disabled_libraries
        .iter()
        .map(|key| {
            let Ok((provider, id, url, user, folder)) =
                serde_json::from_str::<(String, String, String, String, Option<String>)>(key)
            else {
                return key.clone();
            };
            let Some(source) = config.navidrome_sources.iter().find(|s| {
                provider == "navidrome" && s.id == id && s.url == url && s.username == user
            }) else {
                return key.clone();
            };
            let canonical = source.canonical_folder(folder.clone());
            if canonical == folder {
                return key.clone();
            }
            changed = true;
            serde_json::to_string(&(provider, id, url, user, canonical)).expect("string tuple")
        })
        .collect();
    changed
}

/// Remove only the discontinued transport, before typed deserialization. Unknown
/// source kinds and malformed settings must still fail normal validation.
fn parse_config(contents: &str) -> Result<(Config, bool), toml::de::Error> {
    let mut value: toml::Value = toml::from_str(contents)?;
    let mut removed_ids = Vec::new();
    let mut removed = false;
    if let Some(sources) = value
        .get_mut("folder_sources")
        .and_then(toml::Value::as_array_mut)
    {
        sources.retain(|source| {
            if source.get("kind").and_then(toml::Value::as_str) != Some("sftp") {
                return true;
            }
            removed = true;
            if let Some(id) = source.get("id").and_then(toml::Value::as_str) {
                removed_ids.push(id.to_owned());
            }
            false
        });
    }
    if value
        .get("default_folder_source")
        .and_then(toml::Value::as_str)
        .is_some_and(|id| removed_ids.iter().any(|removed| removed == id))
    {
        value
            .as_table_mut()
            .expect("config table")
            .remove("default_folder_source");
    }
    Ok((value.try_into()?, removed))
}

/// Validate config values and replace invalid ones with defaults.
fn validate_config(config: &mut Config) {
    // Volume must be 0.0-1.0
    if !(0.0..=1.0).contains(&config.playback.default_volume) {
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
    let toml_str = toml::to_string(config)?;

    // Write to temp file in the same directory (ensures same filesystem for rename)
    let temp_file = config_file.with_extension("toml.tmp");

    std::fs::write(&temp_file, &toml_str)
        .map_err(|e| anyhow!("Failed to write temp config file: {}", e))?;

    // Atomic rename
    std::fs::rename(&temp_file, &config_file).map_err(|e| {
        // Clean up temp file on failure
        let _ = std::fs::remove_file(&temp_file);
        anyhow!("Failed to save config: {}", e)
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_transport_preserves_other_libraries_and_settings() {
        let input = r#"
default_folder_source = "old"
[[folder_sources]]
id = "old"
name = "Old remote"
kind = "sftp"
host = "example.test"
[[folder_sources]]
id = "local"
name = "Music"
kind = "local"
path = "/music"
[[folder_sources]]
id = "dav"
name = "Remote"
kind = "webdav"
url = "https://example.test/music/"
[playback]
default_volume = 0.3
"#;
        let (config, migrated) = parse_config(input).unwrap();
        assert!(migrated);
        assert!(config.default_folder_source.is_none());
        assert_eq!(config.folder_sources.len(), 2);
        assert_eq!(config.playback.default_volume, 0.3);
        let (again, migrated) = parse_config(&toml::to_string(&config).unwrap()).unwrap();
        assert!(!migrated);
        assert_eq!(again.folder_sources.len(), 2);
        assert!(parse_config(&input.replace("sftp", "unknown")).is_err());
    }

    #[test]
    fn invalid_and_nonfinite_volume_uses_default() {
        for volume in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.1, 1.1] {
            let mut config = Config::default();
            config.playback.default_volume = volume;
            validate_config(&mut config);
            assert_eq!(config.playback.default_volume, 0.8);
        }
        for volume in [0.0, 0.5, 1.0] {
            let mut config = Config::default();
            config.playback.default_volume = volume;
            validate_config(&mut config);
            assert_eq!(config.playback.default_volume, volume);
        }
    }
}
