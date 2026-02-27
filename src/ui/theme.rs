//! UI theming system with multiple theme support.
//!
//! Provides semantic color naming and three built-in themes:
//! - Default (Plexamp-inspired dark)
//! - Solarized Dark
//! - Solarized Light
//! - Retro (Norton Commander/Borland style)

use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};

/// Available theme names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeName {
    #[default]
    SolarizedDark,
    SolarizedLight,
    Dark,
    Borland,
}

impl ThemeName {
    /// All available themes.
    pub fn all() -> &'static [ThemeName] {
        &[
            ThemeName::SolarizedDark,
            ThemeName::SolarizedLight,
            ThemeName::Dark,
            ThemeName::Borland,
        ]
    }

    /// Display name for the theme.
    pub fn display_name(&self) -> &'static str {
        match self {
            ThemeName::Dark => "dark",
            ThemeName::SolarizedDark => "solarized dark",
            ThemeName::SolarizedLight => "solarized light",
            ThemeName::Borland => "borland",
        }
    }

    /// Config string value.
    pub fn config_name(&self) -> &'static str {
        match self {
            ThemeName::Dark => "dark",
            ThemeName::SolarizedDark => "solarized-dark",
            ThemeName::SolarizedLight => "solarized-light",
            ThemeName::Borland => "borland",
        }
    }

    /// Parse from config string.
    pub fn from_config(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "solarized-dark" | "solarizeddark" => ThemeName::SolarizedDark,
            "solarized-light" | "solarizedlight" => ThemeName::SolarizedLight,
            "dark" => ThemeName::Dark,
            "borland" | "retro" | "norton" => ThemeName::Borland,
            _ => ThemeName::SolarizedDark,
        }
    }

    /// Cycle to the next theme.
    pub fn next(&self) -> Self {
        match self {
            ThemeName::SolarizedDark => ThemeName::SolarizedLight,
            ThemeName::SolarizedLight => ThemeName::Dark,
            ThemeName::Dark => ThemeName::Borland,
            ThemeName::Borland => ThemeName::SolarizedDark,
        }
    }
}

/// Color palette with semantic naming.
#[derive(Debug, Clone, Copy)]
pub struct ThemeColors {
    // Backgrounds
    pub bg_primary: Color,
    pub bg_secondary: Color,
    pub bg_highlight: Color,
    pub bg_selection: Color,

    // Foregrounds
    pub fg_primary: Color,
    pub fg_secondary: Color,
    pub fg_muted: Color,
    pub fg_accent: Color,
    pub fg_accent_dim: Color,

    // Borders
    pub border: Color,
    pub border_focused: Color,
    pub title_focused: Color,

    // Semantic colors
    pub error: Color,
    pub success: Color,
    pub warning: Color,

    // Special elements
    pub selection_bar_bg: Color,
    pub selection_bar_fg: Color,
    pub selection_text: Color,
    pub transport_bg: Color,
    pub shortcut_key: Color,
    pub shortcut_text: Color,
}

impl ThemeColors {
    /// Default dark theme (Plexamp-inspired).
    pub fn dark() -> Self {
        Self {
            bg_primary: Color::Rgb(24, 24, 24),
            bg_secondary: Color::Rgb(35, 35, 35),
            bg_highlight: Color::Rgb(45, 45, 45),
            bg_selection: Color::Rgb(60, 60, 60),

            fg_primary: Color::Rgb(220, 220, 220),
            fg_secondary: Color::Rgb(180, 180, 180),
            fg_muted: Color::Rgb(128, 128, 128),
            fg_accent: Color::Rgb(229, 160, 60),      // Plexamp orange
            fg_accent_dim: Color::Rgb(180, 120, 40),

            border: Color::Rgb(60, 60, 60),
            border_focused: Color::Rgb(229, 160, 60),
            title_focused: Color::Rgb(255, 200, 100),

            error: Color::Rgb(220, 80, 80),
            success: Color::Rgb(80, 200, 120),
            warning: Color::Rgb(229, 160, 60),

            selection_bar_bg: Color::Rgb(60, 60, 60),
            selection_bar_fg: Color::Rgb(229, 160, 60),
            selection_text: Color::Rgb(229, 160, 60),
            transport_bg: Color::Rgb(30, 30, 30),
            shortcut_key: Color::Rgb(229, 160, 60),
            shortcut_text: Color::Rgb(128, 128, 128),
        }
    }

    /// Solarized Dark theme.
    /// Uses the canonical 16-color Solarized palette.
    pub fn solarized_dark() -> Self {
        // Solarized base colors
        let base03 = Color::Rgb(0, 43, 54);      // #002b36 - background
        let base02 = Color::Rgb(7, 54, 66);      // #073642 - background highlight
        let base01 = Color::Rgb(88, 110, 117);   // #586e75 - optional emphasis
        let base00 = Color::Rgb(101, 123, 131);  // #657b83 - body text
        let base0 = Color::Rgb(131, 148, 150);   // #839496 - primary content
        let base1 = Color::Rgb(147, 161, 161);   // #93a1a1 - comments
        let _base2 = Color::Rgb(238, 232, 213);  // #eee8d5
        let _base3 = Color::Rgb(253, 246, 227);  // #fdf6e3

        // Solarized accent colors
        let yellow = Color::Rgb(181, 137, 0);    // #b58900
        let orange = Color::Rgb(203, 75, 22);    // #cb4b16
        let red = Color::Rgb(220, 50, 47);       // #dc322f
        let _magenta = Color::Rgb(211, 54, 130); // #d33682
        let _violet = Color::Rgb(108, 113, 196); // #6c71c4
        let blue = Color::Rgb(38, 139, 210);     // #268bd2
        let cyan = Color::Rgb(42, 161, 152);     // #2aa198
        let green = Color::Rgb(133, 153, 0);     // #859900

        Self {
            bg_primary: base03,
            bg_secondary: base02,
            bg_highlight: base02,
            bg_selection: base01,

            fg_primary: base0,
            fg_secondary: base1,
            fg_muted: base01,
            fg_accent: blue,
            fg_accent_dim: cyan,

            border: base01,
            border_focused: blue,
            title_focused: orange,

            error: red,
            success: green,
            warning: yellow,

            selection_bar_bg: base02,
            selection_bar_fg: blue,
            selection_text: blue,
            transport_bg: base02,
            shortcut_key: yellow,
            shortcut_text: base00,
        }
    }

    /// Solarized Light theme.
    pub fn solarized_light() -> Self {
        // Solarized base colors (inverted for light)
        let _base03 = Color::Rgb(0, 43, 54);     // #002b36
        let _base02 = Color::Rgb(7, 54, 66);     // #073642
        let base01 = Color::Rgb(88, 110, 117);   // #586e75
        let base00 = Color::Rgb(101, 123, 131);  // #657b83 - primary content (light mode)
        let _base0 = Color::Rgb(131, 148, 150);  // #839496
        let _base1 = Color::Rgb(147, 161, 161);  // #93a1a1
        let base2 = Color::Rgb(238, 232, 213);   // #eee8d5 - background highlight
        let base3 = Color::Rgb(253, 246, 227);   // #fdf6e3 - background

        // Solarized accent colors
        let _yellow = Color::Rgb(181, 137, 0);   // #b58900
        let orange = Color::Rgb(203, 75, 22);    // #cb4b16
        let red = Color::Rgb(220, 50, 47);       // #dc322f
        let magenta = Color::Rgb(211, 54, 130);  // #d33682
        let _violet = Color::Rgb(108, 113, 196); // #6c71c4
        let blue = Color::Rgb(38, 139, 210);     // #268bd2
        let cyan = Color::Rgb(42, 161, 152);     // #2aa198
        let green = Color::Rgb(133, 153, 0);     // #859900

        Self {
            bg_primary: base3,
            bg_secondary: base2,
            bg_highlight: base2,
            bg_selection: base01,

            fg_primary: base00,
            fg_secondary: base01,
            fg_muted: base01,
            fg_accent: magenta,
            fg_accent_dim: cyan,

            border: base2,
            border_focused: magenta,
            title_focused: orange,

            error: red,
            success: green,
            warning: orange,

            selection_bar_bg: base2,
            selection_bar_fg: magenta,
            selection_text: magenta,
            transport_bg: base2,
            shortcut_key: blue,
            shortcut_text: base00,
        }
    }

    /// Borland theme (Turbo Pascal / Norton Commander style).
    /// Classic CGA/EGA colors with blue background.
    pub fn borland() -> Self {
        // Classic CGA/EGA colors
        let blue = Color::Rgb(0, 0, 170);        // #0000AA - Classic blue background
        let cyan = Color::Rgb(0, 170, 170);      // #00AAAA - Selection bar
        let white = Color::Rgb(255, 255, 255);   // #FFFFFF - High-intensity white
        let light_gray = Color::Rgb(170, 170, 170); // #AAAAAA - Secondary text
        let yellow = Color::Rgb(255, 255, 85);   // #FFFF55 - Command/status text
        let red = Color::Rgb(170, 0, 0);         // #AA0000 - Errors/warnings
        let green = Color::Rgb(0, 170, 0);       // #00AA00 - Success
        let dark_blue = Color::Rgb(0, 0, 85);    // #000055 - Darker blue for secondary

        Self {
            bg_primary: blue,
            bg_secondary: dark_blue,
            bg_highlight: cyan,
            bg_selection: cyan,

            fg_primary: white,
            fg_secondary: light_gray,
            fg_muted: light_gray,
            fg_accent: yellow,
            fg_accent_dim: light_gray,

            border: cyan,
            border_focused: yellow,
            title_focused: white,

            error: red,
            success: green,
            warning: yellow,

            selection_bar_bg: cyan,
            selection_bar_fg: Color::Black,
            selection_text: Color::Black,
            transport_bg: dark_blue,
            shortcut_key: yellow,
            shortcut_text: white,
        }
    }

    /// Get colors for a theme name.
    pub fn for_theme(theme: ThemeName) -> Self {
        match theme {
            ThemeName::Dark => Self::dark(),
            ThemeName::SolarizedDark => Self::solarized_dark(),
            ThemeName::SolarizedLight => Self::solarized_light(),
            ThemeName::Borland => Self::borland(),
        }
    }
}

/// Active theme with pre-computed styles.
/// This is the primary interface for UI components.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub colors: ThemeColors,
    name: ThemeName,
}

impl Default for Theme {
    fn default() -> Self {
        Self::new(ThemeName::SolarizedDark)
    }
}

impl Theme {
    /// Create a new theme instance.
    pub fn new(name: ThemeName) -> Self {
        Self {
            colors: ThemeColors::for_theme(name),
            name,
        }
    }

    /// Get the theme name.
    pub fn name(&self) -> ThemeName {
        self.name
    }

    // ==================== Instance Style Methods ====================
    // These are called on a Theme instance (used by static methods via theme()).

    fn _title(&self) -> Style {
        Style::default()
            .fg(self.colors.fg_primary)
            .add_modifier(Modifier::BOLD)
    }

    fn _subtitle(&self) -> Style {
        Style::default().fg(self.colors.fg_secondary)
    }

    fn _accent(&self) -> Style {
        Style::default().fg(self.colors.fg_accent)
    }

    fn _muted(&self) -> Style {
        Style::default().fg(self.colors.fg_muted)
    }

    fn _selected(&self) -> Style {
        Style::default()
            .fg(self.colors.fg_accent)
            .add_modifier(Modifier::BOLD)
    }

    fn _highlight(&self) -> Style {
        Style::default()
            .bg(self.colors.bg_highlight)
            .fg(self.colors.fg_primary)
    }

    fn _selection_bar(&self) -> Style {
        Style::default()
            .bg(self.colors.selection_bar_bg)
            .fg(self.colors.selection_bar_fg)
            .add_modifier(Modifier::BOLD)
    }

    fn _border(&self) -> Style {
        Style::default().fg(self.colors.border)
    }

    fn _border_focused(&self) -> Style {
        Style::default().fg(self.colors.border_focused)
    }

    fn _error(&self) -> Style {
        Style::default().fg(self.colors.error)
    }

    fn _success(&self) -> Style {
        Style::default().fg(self.colors.success)
    }

    fn _warning(&self) -> Style {
        Style::default().fg(self.colors.warning)
    }

    fn _status_bar(&self) -> Style {
        Style::default()
            .bg(self.colors.bg_secondary)
            .fg(self.colors.fg_primary)
    }

    fn _transport_bar(&self) -> Style {
        Style::default()
            .bg(self.colors.transport_bg)
            .fg(self.colors.fg_primary)
    }

    fn _shortcut_key(&self) -> Style {
        Style::default()
            .fg(self.colors.shortcut_key)
            .add_modifier(Modifier::BOLD)
    }

    fn _shortcut_text(&self) -> Style {
        Style::default().fg(self.colors.shortcut_text)
    }

    fn _text(&self) -> Style {
        Style::default().fg(self.colors.fg_primary)
    }

    /// Primary background color.
    pub fn bg(&self) -> Color {
        self.colors.bg_primary
    }

    /// Primary foreground color.
    pub fn fg(&self) -> Color {
        self.colors.fg_primary
    }

    /// Accent foreground color.
    pub fn accent_color(&self) -> Color {
        self.colors.fg_accent
    }

    /// Border color (unfocused).
    pub fn border_color(&self) -> Color {
        self.colors.border
    }

    /// Border color (focused).
    pub fn border_focused_color(&self) -> Color {
        self.colors.border_focused
    }

    // ==================== Legacy Compatibility ====================
    // These constants provide backward compatibility with existing code.
    // They use the default (dark) theme colors for places that need compile-time constants.

    pub const BG: Color = Color::Rgb(24, 24, 24);
    pub const FG: Color = Color::Rgb(220, 220, 220);
    pub const ACCENT: Color = Color::Rgb(229, 160, 60);
    pub const ACCENT_DIM: Color = Color::Rgb(180, 120, 40);
    pub const MUTED: Color = Color::Rgb(128, 128, 128);
    pub const BORDER: Color = Color::Rgb(60, 60, 60);
    pub const HIGHLIGHT_BG: Color = Color::Rgb(45, 45, 45);
    pub const BG_HIGHLIGHT: Color = Color::Rgb(35, 35, 35);
    pub const ERROR: Color = Color::Rgb(220, 80, 80);
    pub const SUCCESS: Color = Color::Rgb(80, 200, 120);
}

// ==================== Global Theme State ====================
// Thread-local storage for the active theme.

use std::cell::RefCell;

thread_local! {
    static ACTIVE_THEME: RefCell<Theme> = RefCell::new(Theme::default());
}

/// Set the active theme globally.
pub fn set_theme(name: ThemeName) {
    ACTIVE_THEME.with(|t| {
        *t.borrow_mut() = Theme::new(name);
    });
}

/// Get the active theme.
pub fn theme() -> Theme {
    ACTIVE_THEME.with(|t| *t.borrow())
}

// ==================== Static Style Methods ====================
// These use the global theme state and provide easy migration from old Theme::method() calls.

impl Theme {
    /// Title style using global theme.
    pub fn title() -> Style {
        theme()._title()
    }

    /// Subtitle style using global theme.
    pub fn subtitle() -> Style {
        theme()._subtitle()
    }

    /// Accent style using global theme.
    pub fn accent() -> Style {
        theme()._accent()
    }

    /// Muted style using global theme.
    pub fn muted() -> Style {
        theme()._muted()
    }

    /// Selected item style using global theme.
    pub fn selected() -> Style {
        theme()._selected()
    }

    /// Highlight style using global theme.
    pub fn highlight() -> Style {
        theme()._highlight()
    }

    /// Selection bar style using global theme.
    pub fn selection_bar() -> Style {
        theme()._selection_bar()
    }

    /// Border style using global theme.
    pub fn border() -> Style {
        theme()._border()
    }

    /// Border focused style using global theme.
    pub fn border_focused() -> Style {
        theme()._border_focused()
    }

    /// Error style using global theme.
    pub fn error() -> Style {
        theme()._error()
    }

    /// Success style using global theme.
    pub fn success() -> Style {
        theme()._success()
    }

    /// Warning style using global theme.
    pub fn warning() -> Style {
        theme()._warning()
    }

    /// Status bar style using global theme.
    pub fn status_bar() -> Style {
        theme()._status_bar()
    }

    /// Transport bar style using global theme.
    pub fn now_playing_bar() -> Style {
        theme()._transport_bar()
    }

    /// Shortcut key style using global theme.
    pub fn shortcut_key() -> Style {
        theme()._shortcut_key()
    }

    /// Shortcut text style using global theme.
    pub fn shortcut_text() -> Style {
        theme()._shortcut_text()
    }

    /// Text style using global theme.
    pub fn text() -> Style {
        theme()._text()
    }

    /// Style for a list item based on selection state.
    /// Use this instead of inline conditional style computation.
    ///
    /// # Arguments
    /// * `is_selected` - Whether this item is currently selected
    ///
    /// # Returns
    /// Selection bar style if selected, default foreground otherwise.
    pub fn list_item_style(is_selected: bool) -> Style {
        let t = theme();
        if is_selected {
            Style::default()
                .fg(t.colors.selection_text)
                .bg(t.colors.selection_bar_bg)
        } else {
            Style::default().fg(t.colors.fg_primary)
        }
    }

    /// Style for a list item with focus awareness.
    /// Selected items show the same style regardless of focus,
    /// but unfocused panels may want to use this for visual distinction.
    ///
    /// # Arguments
    /// * `is_selected` - Whether this item is currently selected
    /// * `is_focused` - Whether the containing panel has focus
    ///
    /// # Returns
    /// Selection bar style if selected (regardless of focus),
    /// default foreground otherwise.
    pub fn list_item_style_with_focus(is_selected: bool, _is_focused: bool) -> Style {
        // Currently focus doesn't affect item styling, but this method
        // exists for future extensibility. Using the same style for both
        // ensures consistency and avoids redundant is_focused checks.
        Self::list_item_style(is_selected)
    }
}
