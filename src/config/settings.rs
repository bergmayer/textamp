//! Configuration structures.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Root configuration structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub plex: PlexConfig,
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub playback: PlaybackConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub libraries: LibrariesConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            plex: PlexConfig::default(),
            general: GeneralConfig::default(),
            playback: PlaybackConfig::default(),
            ui: UiConfig::default(),
            libraries: LibrariesConfig::default(),
        }
    }
}

/// Per-library settings (indexed by library key).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibrarySettings {
    /// Keep subfolder cache entries indefinitely (don't purge at 32 days).
    #[serde(default, alias = "keep_folder_cache")]
    pub keep_subfolder_cache: bool,
}

/// Libraries configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibrariesConfig {
    /// Default library to open on startup (by key)
    #[serde(default)]
    pub default_library: Option<String>,

    /// Selected server identifier (for multi-server setups)
    #[serde(default)]
    pub selected_server: Option<String>,

    /// Per-library settings keyed by library key
    #[serde(default)]
    pub per_library: HashMap<String, LibrarySettings>,
}

/// Plex server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlexConfig {
    /// Plex server URL (e.g., "http://localhost:32400")
    #[serde(default = "default_server_url")]
    pub server_url: String,

    /// Plex username (display only, authentication uses tokens)
    #[serde(default)]
    pub username: Option<String>,

    /// Pre-existing auth token (primary authentication method)
    #[serde(default)]
    pub token: Option<String>,
}

impl Default for PlexConfig {
    fn default() -> Self {
        Self {
            server_url: default_server_url(),
            username: None,
            token: None,
        }
    }
}

fn default_server_url() -> String {
    "http://localhost:32400".to_string()
}

/// General application settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    #[serde(default)]
    pub default_library: Option<String>,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            default_library: None,
        }
    }
}

/// Playback settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackConfig {
    #[serde(default = "default_volume")]
    pub default_volume: f32,

    #[serde(default)]
    pub gapless: bool,

    #[serde(default = "default_buffer_size")]
    pub buffer_size_kb: u32,
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            default_volume: default_volume(),
            gapless: true,
            buffer_size_kb: default_buffer_size(),
        }
    }
}

fn default_volume() -> f32 {
    0.8
}

fn default_buffer_size() -> u32 {
    1024
}

/// UI settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_show_album_art")]
    pub show_album_art: bool,

    #[serde(default = "default_album_art_size")]
    pub album_art_size: u16,

    #[serde(default = "default_theme")]
    pub theme: String,

    /// Persist cover art view mode across sessions (Alt+C toggle).
    #[serde(default)]
    pub cover_art_view: bool,

    /// Artwork rendering mode: "auto", "halfblocks", or "braille".
    #[serde(default = "default_artwork_mode")]
    pub artwork_mode: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            show_album_art: default_show_album_art(),
            album_art_size: default_album_art_size(),
            theme: default_theme(),
            cover_art_view: false,
            artwork_mode: default_artwork_mode(),
        }
    }
}

fn default_show_album_art() -> bool {
    true
}

fn default_album_art_size() -> u16 {
    40
}

fn default_theme() -> String {
    "solarized-dark".to_string()
}

fn default_artwork_mode() -> String {
    "auto".to_string()
}
