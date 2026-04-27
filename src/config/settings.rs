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

    /// Transcode bitrate in kbps. 0 = disabled (direct play), e.g. 256 = transcode to 256kbps MP3.
    /// Useful for remote connections where bandwidth is limited.
    #[serde(default)]
    pub transcode_kbps: u32,
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            default_volume: default_volume(),
            gapless: true,
            buffer_size_kb: default_buffer_size(),
            transcode_kbps: 0,
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
    /// Default ON — album art is a core part of the browsing experience.
    /// TUI users on limited terminals can turn it off and the choice
    /// will stick after Alt+C persists the flag.
    #[serde(default = "default_cover_art_view")]
    pub cover_art_view: bool,

    /// Artwork rendering mode: "auto", "halfblocks", or "braille".
    #[serde(default = "default_artwork_mode")]
    pub artwork_mode: String,

    /// GUI-only: last window geometry. Ignored by the TUI.
    #[serde(default)]
    pub window: WindowConfig,

    /// GUI content scale (zoom). 1.0 = native size. Ignored by the TUI.
    /// Adjustable from Settings → View Options. Clamped to a sensible
    /// range at runtime to keep the UI usable.
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f32,
}

/// GUI window geometry persisted across launches.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct WindowConfig {
    #[serde(default = "default_window_width")]
    pub width: u32,
    #[serde(default = "default_window_height")]
    pub height: u32,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            show_album_art: default_show_album_art(),
            album_art_size: default_album_art_size(),
            theme: default_theme(),
            cover_art_view: default_cover_art_view(),
            artwork_mode: default_artwork_mode(),
            window: WindowConfig::default(),
            ui_scale: default_ui_scale(),
        }
    }
}

fn default_ui_scale() -> f32 {
    // macOS handles HiDPI by rendering at 2× and downsampling, so a
    // 1.25 logical scale on top of that produces oversized chrome —
    // 1.0 reads correctly. Other platforms stay on 1.25, which suits
    // typical 1080p+ Windows/Linux displays. Existing saved values
    // are honoured (serde reads them back); only fresh installs /
    // reset-to-default pick this up.
    #[cfg(target_os = "macos")]
    { 1.0 }
    #[cfg(not(target_os = "macos"))]
    { 1.25 }
}

/// Clamp bounds for the user-settable UI scale. Keep the UI within
/// reach of the cursor at both ends.
pub const UI_SCALE_MIN: f32 = 0.75;
pub const UI_SCALE_MAX: f32 = 2.0;
pub const UI_SCALE_STEP: f32 = 0.1;

fn default_cover_art_view() -> bool {
    true
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: default_window_width(),
            height: default_window_height(),
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

fn default_window_width() -> u32 {
    1280
}

fn default_window_height() -> u32 {
    840
}
