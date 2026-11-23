//! Configuration structures.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Root configuration structure.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub players: Vec<crate::services::player_sync::Player>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub webdav_player_sync: crate::app::player_sync::WebdavPlayers,
    /// Recent successful descriptions and ranked result IDs, keyed by exact library identity.
    #[serde(default)]
    pub description_history: HashMap<String, Vec<crate::audiomuse::DescriptionHistory>>,
    #[serde(default)]
    /// Optional analysis connection keyed by the exact Navidrome library identity.
    pub audiomuse_connections: HashMap<String, crate::audiomuse::Connection>,
    /// Source/account/server/library-scoped opt-outs. Absent preserves existing behavior.
    #[serde(default)]
    pub sonic_disabled_libraries: std::collections::HashSet<String>,
    #[serde(default)]
    pub navidrome_sources: Vec<crate::navidrome::Source>,
    /// Account and optional music-folder selection, independent of server defaults.
    #[serde(default)]
    pub default_navidrome: Option<crate::navidrome::Selection>,
    /// Named filesystem libraries, independent of server sign-in.
    #[serde(default)]
    pub folder_sources: Vec<crate::library::FolderSource>,
    /// Last selected folder, opened at startup when configured.
    #[serde(default)]
    pub default_folder_source: Option<String>,

    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub playback: PlaybackConfig,
    #[serde(default)]
    pub ui: UiConfig,
}

impl Config {
    /// Persist playlist grouping, omitting the ungrouped default.
    pub fn set_playlist_view(&mut self, lib_key: &str, playlist_key: &str, view: PlaylistView) {
        let lib = self
            .ui
            .library_view_settings
            .entry(lib_key.to_string())
            .or_default();
        if view.is_default() {
            lib.playlists.remove(playlist_key);
            if lib.is_empty() {
                self.ui.library_view_settings.remove(lib_key);
            }
        } else {
            lib.playlists.insert(playlist_key.to_string(), view);
        }
    }

    /// Resolve rendering for one library and presentation; terminal image choices
    /// are retained even when the current terminal uses a character fallback.
    pub fn artwork_mode(
        &self,
        lib_key: &str,
        presentation: crate::app::launch::WindowedMode,
    ) -> crate::app::state::ArtworkMode {
        let library = self.ui.library_view_settings.get(lib_key);
        let value = match presentation {
            crate::app::launch::WindowedMode::Text => library
                .and_then(|view| view.artwork_mode.as_deref())
                .unwrap_or(&self.ui.artwork_mode),
            crate::app::launch::WindowedMode::Gui => library
                .and_then(|view| view.graphical_artwork_mode.as_deref())
                .unwrap_or("auto"),
        };
        crate::app::state::ArtworkMode::from_config(value)
    }
    pub fn set_artwork_mode(
        &mut self,
        lib_key: &str,
        presentation: crate::app::launch::WindowedMode,
        mode: crate::app::state::ArtworkMode,
    ) {
        let library = self
            .ui
            .library_view_settings
            .entry(lib_key.to_owned())
            .or_default();
        let value = match presentation {
            crate::app::launch::WindowedMode::Text => &mut library.artwork_mode,
            crate::app::launch::WindowedMode::Gui => &mut library.graphical_artwork_mode,
        };
        *value = Some(mode.name().to_owned());
    }

    pub fn set_column_artwork(&mut self, lib_key: &str, column_key: &str, visible: bool) {
        let lib = self
            .ui
            .library_view_settings
            .entry(lib_key.to_string())
            .or_default();
        lib.columns.insert(column_key.to_string(), visible);
        if lib.is_empty() {
            self.ui.library_view_settings.remove(lib_key);
        }
    }

    /// Drop stored playlist entries for any playlist that no longer
    /// exists in the given library. Called after a catalog refresh
    /// so deleted playlists' settings don't accumulate forever.
    pub fn prune_stale_playlist_views(
        &mut self,
        lib_key: &str,
        live_playlist_keys: &std::collections::HashSet<String>,
    ) {
        if let Some(lib) = self.ui.library_view_settings.get_mut(lib_key) {
            lib.playlists.retain(|k, _| live_playlist_keys.contains(k));
            if lib.is_empty() {
                self.ui.library_view_settings.remove(lib_key);
            }
        }
    }
}

/// General application settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeneralConfig {
    #[serde(default)]
    pub default_library: Option<String>,
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

    /// Maximum server streaming bitrate in kbps; 0 requests the original file.
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
    /// Most recently activated windowed presentation; external terminals do not change it.
    #[serde(default)]
    pub last_windowed_mode: crate::app::launch::WindowedMode,
    #[serde(default = "default_show_album_art")]
    pub show_album_art: bool,

    #[serde(default = "default_album_art_size")]
    pub album_art_size: u16,

    #[serde(default = "default_theme")]
    pub theme: String,

    /// Default for libraries without their own rendering choice.
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

    /// Browser search provider for artist information and web queries.
    #[serde(default)]
    pub web_search_provider: crate::services::web_search::Provider,

    /// Include Spotify search in menus and palettes. Disabled by default.
    #[serde(default)]
    pub enable_spotify_search: bool,

    /// Same as `enable_apple_music_search` but for YouTube search.
    #[serde(default = "default_enable_search_service")]
    pub enable_youtube_search: bool,

    /// Per-library artwork and playlist view settings. Outer key is the
    /// library's `lib_key`, inner is the playlist's `rating_key`.
    /// Pruned on a catalog refresh so deleted playlists don't keep
    /// accumulating stale entries.
    #[serde(default)]
    pub library_view_settings: std::collections::HashMap<String, LibraryViewSettings>,

    /// Sections hidden from the leftmost browse column. Each entry is a
    /// `BrowseCategory` variant. The user toggles these from the
    /// Settings dialog's "Sections" panel; sparsely-populated tag types
    /// (Collections, Countries, Labels, Formats, Studios) are hidden by
    /// default to keep the column readable.
    #[serde(default = "default_hidden_sections")]
    pub hidden_sections: Vec<crate::app::state::BrowseCategory>,
    #[serde(default)]
    pub desktop_hidden_library_roots: Vec<crate::app::state::LibraryRoot>,
    /// Multiple persistent desktop windows; false uses temporary in-window dialogs.
    #[serde(default)]
    pub desktop_separate_player: bool,
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub last_playlist_destinations: std::collections::HashMap<String, String>,
    #[serde(default, skip_serializing)]
    pub desktop_command_history: Vec<String>,
    #[serde(default)]
    pub command_history: crate::app::command_history::History,
    #[serde(default)]
    pub desktop_large_album_covers: bool,
    #[serde(default)]
    pub remix_allow_same_artist: bool,
    #[serde(default)]
    pub show_compilation_only_artists: bool,
    #[serde(default = "crate::services::singles::default_limit")]
    pub singles_track_limit: u32,
    #[serde(default)]
    pub desktop_collapsed_sections: Vec<String>,
    /// Optional Navidrome / AudioMuse sidebar views. Existing category preferences
    /// retain their original configuration format.
    #[serde(default)]
    pub hidden_collections: Vec<crate::app::sources::navidrome::commands::CollectionKind>,
    /// Keep the desktop player visible alongside the library.
    #[serde(default, alias = "tall_mode")]
    pub desktop_split_view: bool,
}

fn default_hidden_sections() -> Vec<crate::app::state::BrowseCategory> {
    crate::app::state::BrowseCategory::hidden_by_default().to_vec()
}

/// View toggles scoped to a single server library.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibraryViewSettings {
    /// Per-playlist toggles, keyed by the playlist's server rating_key.
    #[serde(default)]
    pub playlists: std::collections::HashMap<String, PlaylistView>,
    /// Artwork visibility keyed by column type, shared by all presentations.
    #[serde(default)]
    pub columns: std::collections::HashMap<String, bool>,
    /// Terminal rendering method for this library.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artwork_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Graphical artwork encoding, independent of the terminal encoding.
    pub graphical_artwork_mode: Option<String>,
}

impl LibraryViewSettings {
    fn is_empty(&self) -> bool {
        self.playlists.is_empty()
            && self.columns.is_empty()
            && self.artwork_mode.is_none()
            && self.graphical_artwork_mode.is_none()
    }
}

/// View toggles for a single playlist's tracks column.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlaylistView {
    #[serde(default)]
    pub group_by_album: bool,
}

impl PlaylistView {
    /// True for the default ungrouped playlist.
    pub fn is_default(&self) -> bool {
        !self.group_by_album
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
            last_windowed_mode: Default::default(),
            show_album_art: default_show_album_art(),
            album_art_size: default_album_art_size(),
            theme: default_theme(),
            artwork_mode: default_artwork_mode(),
            window: WindowConfig::default(),
            ui_scale: default_ui_scale(),
            enable_apple_music_search: default_enable_search_service(),
            web_search_provider: Default::default(),
            enable_spotify_search: false,
            enable_youtube_search: default_enable_search_service(),
            library_view_settings: std::collections::HashMap::new(),
            hidden_sections: default_hidden_sections(),
            desktop_hidden_library_roots: vec![],
            desktop_separate_player: false,
            last_playlist_destinations: Default::default(),
            desktop_command_history: vec![],
            command_history: Default::default(),
            desktop_large_album_covers: false,
            remix_allow_same_artist: false,
            show_compilation_only_artists: false,
            singles_track_limit: crate::services::singles::DEFAULT_LIMIT,
            desktop_collapsed_sections: vec![],
            hidden_collections: Vec::new(),
            desktop_split_view: false,
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
    {
        1.0
    }
    #[cfg(not(target_os = "macos"))]
    {
        1.25
    }
}

/// Clamp bounds for the user-settable UI scale. Keep the UI within
/// reach of the cursor at both ends.
pub const UI_SCALE_MIN: f32 = 0.75;
pub const UI_SCALE_MAX: f32 = 2.0;
pub const UI_SCALE_STEP: f32 = 0.1;

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

#[cfg(test)]
mod desktop_layout_tests {
    use super::*;
    #[test]
    fn artwork_rendering_is_independent_and_legacy_terminal_choice_survives() {
        use crate::app::{launch::WindowedMode, state::ArtworkMode};
        let mut config: Config =
            toml::from_str("[ui]\nartwork_mode = \"braille\"\ncover_art_view = false\n").unwrap();
        assert_eq!(
            config.artwork_mode("library", WindowedMode::Text),
            ArtworkMode::Braille
        );
        assert_eq!(
            config.artwork_mode("library", WindowedMode::Gui),
            ArtworkMode::Auto
        );
        config.set_artwork_mode("library", WindowedMode::Gui, ArtworkMode::Halfblocks);
        let saved = toml::to_string(&config).unwrap();
        assert!(!saved.contains("cover_art_view"));
        let restored: Config = toml::from_str(&saved).unwrap();
        assert_eq!(
            restored.artwork_mode("library", WindowedMode::Text),
            ArtworkMode::Braille
        );
        assert_eq!(
            restored.artwork_mode("library", WindowedMode::Gui),
            ArtworkMode::Halfblocks
        );
    }
    #[test]
    fn obsolete_column_sizing_is_ignored_and_not_reserialized() {
        let mut config: Config =
            toml::from_str("[ui]\ndesktop_column_sizing = \"fit_content\"\n").unwrap();
        config.ui.desktop_separate_player = true;
        let restored: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
        assert!(
            !toml::to_string(&restored)
                .unwrap()
                .contains("desktop_column_sizing")
        );
        assert!(restored.ui.desktop_separate_player);
    }
}
