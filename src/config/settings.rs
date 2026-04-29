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

impl Config {
    /// Read-only lookup of stored view settings for a single
    /// playlist. Returns `None` when neither the library nor the
    /// playlist has any saved entry — the caller should fall back to
    /// defaults (no grouping, no artwork) in that case.
    pub fn playlist_view(&self, lib_key: &str, playlist_key: &str) -> Option<PlaylistView> {
        self.ui.library_view_settings
            .get(lib_key)?
            .playlists
            .get(playlist_key)
            .copied()
    }

    /// Write the current per-playlist view to config. An all-default
    /// view (no grouping, no artwork) deletes the entry rather than
    /// storing a redundant row — keeps the on-disk config minimal.
    pub fn set_playlist_view(&mut self, lib_key: &str, playlist_key: &str, view: PlaylistView) {
        let lib = self.ui.library_view_settings
            .entry(lib_key.to_string())
            .or_default();
        if view.is_default() {
            lib.playlists.remove(playlist_key);
            if lib.playlists.is_empty() {
                self.ui.library_view_settings.remove(lib_key);
            }
        } else {
            lib.playlists.insert(playlist_key.to_string(), view);
        }
    }

    /// Drop stored playlist entries for any playlist that no longer
    /// exists in the given library. Called after `PlaylistsLoaded`
    /// so deleted playlists' settings don't accumulate forever.
    pub fn prune_stale_playlist_views(
        &mut self,
        lib_key: &str,
        live_playlist_keys: &std::collections::HashSet<String>,
    ) {
        if let Some(lib) = self.ui.library_view_settings.get_mut(lib_key) {
            lib.playlists.retain(|k, _| live_playlist_keys.contains(k));
            if lib.playlists.is_empty() {
                self.ui.library_view_settings.remove(lib_key);
            }
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

    /// Whether the "Search Apple Music" entry is offered in palette,
    /// menus, and right-click context menus. When false the entry is
    /// hidden everywhere and the action becomes a no-op (with a status
    /// notification). Default ON.
    #[serde(default = "default_enable_search_service")]
    pub enable_apple_music_search: bool,

    /// Same as `enable_apple_music_search` but for Spotify search.
    #[serde(default = "default_enable_search_service")]
    pub enable_spotify_search: bool,

    /// Same as `enable_apple_music_search` but for YouTube search.
    #[serde(default = "default_enable_search_service")]
    pub enable_youtube_search: bool,

    /// Per-library view settings (currently per-playlist toggles for
    /// "Group by album" and "Show album artwork"). Outer key is the
    /// library's `lib_key`, inner is the playlist's `rating_key`.
    /// Pruned on `PlaylistsLoaded` so deleted playlists don't keep
    /// accumulating stale entries.
    #[serde(default)]
    pub library_view_settings: std::collections::HashMap<String, LibraryViewSettings>,
}

/// View toggles scoped to a single Plex library.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibraryViewSettings {
    /// Per-playlist toggles, keyed by the playlist's Plex rating_key.
    #[serde(default)]
    pub playlists: std::collections::HashMap<String, PlaylistView>,
}

/// View toggles for a single playlist's tracks column.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlaylistView {
    #[serde(default)]
    pub group_by_album: bool,
    #[serde(default)]
    pub show_artwork: bool,
}

impl PlaylistView {
    /// True when neither toggle is on — used by the saver to delete
    /// the entry instead of writing all-default values.
    pub fn is_default(&self) -> bool {
        !self.group_by_album && !self.show_artwork
    }
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
            enable_apple_music_search: default_enable_search_service(),
            enable_spotify_search: default_enable_search_service(),
            enable_youtube_search: default_enable_search_service(),
            library_view_settings: std::collections::HashMap::new(),
        }
    }
}

fn default_enable_search_service() -> bool {
    true
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
