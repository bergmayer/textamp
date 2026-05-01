//! UI theming system with multiple theme support.
//!
//! Provides semantic color naming and three built-in themes:
//! - Default (Plexamp-inspired dark)
//! - Solarized Dark
//! - Solarized Light
//! - Retro (Norton Commander DOS style)

use ratatui::style::{Color, Modifier, Style};

// UI-agnostic theme identifier lives in the app layer.
pub use crate::app::theme::ThemeName;

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

    /// Platinum theme — inspired by classic Mac OS 9 "Platinum" UI:
    /// light gray window chrome, black text, blue highlight.
    pub fn platinum() -> Self {
        let bg          = Color::Rgb(221, 221, 221); // platinum gray #DDDDDD
        let bg_light    = Color::Rgb(238, 238, 238); // #EEEEEE (lighter field)
        let bg_shadow   = Color::Rgb(170, 170, 170); // #AAAAAA (groove shadow)
        let text        = Color::Rgb(0, 0, 0);
        let muted       = Color::Rgb(102, 102, 102); // #666666
        let blue        = Color::Rgb(59, 120, 255);  // #3B78FF classic highlight
        let dark_blue   = Color::Rgb(0, 58, 168);    // deeper blue
        let red         = Color::Rgb(204, 0, 0);
        let green       = Color::Rgb(0, 128, 0);

        Self {
            bg_primary: bg,
            bg_secondary: bg_light,
            bg_highlight: bg_shadow,
            bg_selection: blue,

            fg_primary: text,
            fg_secondary: muted,
            fg_muted: muted,
            fg_accent: dark_blue,
            fg_accent_dim: muted,

            border: bg_shadow,
            border_focused: blue,
            title_focused: dark_blue,

            error: red,
            success: green,
            warning: Color::Rgb(180, 120, 40),

            selection_bar_bg: blue,
            selection_bar_fg: Color::Rgb(255, 255, 255),
            selection_text: Color::Rgb(255, 255, 255),
            transport_bg: bg_light,
            shortcut_key: dark_blue,
            shortcut_text: text,
        }
    }

    /// Black and white theme — pure black and pure white, no greys.
    /// Selection inverts to white-on-black, everything else is one of
    /// the two extremes. Keeps the look strictly tonal.
    pub fn black_and_white() -> Self {
        let white = Color::Rgb(255, 255, 255);
        let black = Color::Black;

        Self {
            bg_primary: white,
            bg_secondary: white,
            bg_highlight: black,
            bg_selection: black,

            fg_primary: black,
            fg_secondary: black,
            fg_muted: black,
            fg_accent: black,
            fg_accent_dim: black,

            border: black,
            border_focused: black,
            title_focused: black,

            error: black,
            success: black,
            warning: black,

            selection_bar_bg: black,
            selection_bar_fg: white,
            selection_text: white,
            transport_bg: white,
            shortcut_key: black,
            shortcut_text: black,
        }
    }

    /// Vintage amber-on-black CRT — IBM 3270 / DEC monochrome look.
    /// One hue, three brightnesses; the highlight states use a dim
    /// amber background so foreground amber stays legible on top.
    pub fn amber() -> Self {
        let bg = Color::Rgb(15, 8, 0);
        let amber = Color::Rgb(255, 176, 0);
        let amber_dim = Color::Rgb(180, 120, 0);
        let amber_dark = Color::Rgb(80, 50, 0);
        Self {
            bg_primary: bg,
            bg_secondary: Color::Rgb(25, 14, 0),
            bg_highlight: amber_dark,
            bg_selection: amber_dark,

            fg_primary: amber,
            fg_secondary: amber_dim,
            fg_muted: Color::Rgb(120, 80, 0),
            fg_accent: amber,
            fg_accent_dim: amber_dim,

            border: amber_dim,
            border_focused: amber,
            title_focused: amber,

            error: Color::Rgb(255, 100, 50),
            success: amber,
            warning: amber,

            selection_bar_bg: amber,
            selection_bar_fg: bg,
            selection_text: bg,
            transport_bg: Color::Rgb(20, 11, 0),
            shortcut_key: amber,
            shortcut_text: amber_dim,
        }
    }

    /// Vintage green-on-black CRT (Apple ][, DEC VT, early Macs).
    pub fn phosphor_green() -> Self {
        let bg = Color::Rgb(0, 12, 0);
        let green = Color::Rgb(80, 255, 100);
        let green_dim = Color::Rgb(50, 180, 70);
        let green_dark = Color::Rgb(0, 60, 20);
        Self {
            bg_primary: bg,
            bg_secondary: Color::Rgb(0, 22, 5),
            bg_highlight: green_dark,
            bg_selection: green_dark,

            fg_primary: green,
            fg_secondary: green_dim,
            fg_muted: Color::Rgb(40, 120, 50),
            fg_accent: green,
            fg_accent_dim: green_dim,

            border: green_dim,
            border_focused: green,
            title_focused: green,

            error: Color::Rgb(255, 100, 60),
            success: green,
            warning: Color::Rgb(255, 200, 60),

            selection_bar_bg: green,
            selection_bar_fg: bg,
            selection_text: bg,
            transport_bg: Color::Rgb(0, 18, 5),
            shortcut_key: green,
            shortcut_text: green_dim,
        }
    }

    /// Norton Commander DOS file manager — blue background with the
    /// classic cyan title, yellow accents, white body text. The
    /// surviving member of the DOS-blue family (the older Borland
    /// theme has been removed).
    pub fn norton() -> Self {
        let bg_blue = Color::Rgb(0, 0, 170);     // DOS bright blue
        let cyan = Color::Rgb(85, 255, 255);     // DOS bright cyan (titles)
        let yellow = Color::Rgb(255, 255, 85);   // DOS bright yellow (highlight)
        let white = Color::Rgb(255, 255, 255);
        let lightgray = Color::Rgb(200, 200, 200);
        let darkblue = Color::Rgb(0, 0, 110);
        Self {
            bg_primary: bg_blue,
            bg_secondary: darkblue,
            bg_highlight: cyan,
            bg_selection: cyan,

            fg_primary: white,
            fg_secondary: lightgray,
            fg_muted: Color::Rgb(160, 160, 200),
            fg_accent: yellow,
            fg_accent_dim: Color::Rgb(200, 200, 70),

            border: cyan,
            border_focused: yellow,
            title_focused: yellow,

            error: Color::Rgb(255, 85, 85),
            success: Color::Rgb(85, 255, 85),
            warning: yellow,

            selection_bar_bg: cyan,
            selection_bar_fg: Color::Rgb(0, 0, 0),
            selection_text: Color::Rgb(0, 0, 0),
            transport_bg: darkblue,
            shortcut_key: yellow,
            shortcut_text: white,
        }
    }

    /// Dracula — popular dark theme: dark gray background with
    /// purple, pink, cyan, and green accents.
    /// Reference palette: https://draculatheme.com/contribute
    pub fn dracula() -> Self {
        let bg = Color::Rgb(40, 42, 54);          // #282a36
        let current_line = Color::Rgb(68, 71, 90); // #44475a
        let foreground = Color::Rgb(248, 248, 242); // #f8f8f2
        let comment = Color::Rgb(98, 114, 164);   // #6272a4
        let cyan = Color::Rgb(139, 233, 253);     // #8be9fd
        let green = Color::Rgb(80, 250, 123);     // #50fa7b
        let pink = Color::Rgb(255, 121, 198);     // #ff79c6
        let purple = Color::Rgb(189, 147, 249);   // #bd93f9
        let red = Color::Rgb(255, 85, 85);        // #ff5555
        let yellow = Color::Rgb(241, 250, 140);   // #f1fa8c
        Self {
            bg_primary: bg,
            bg_secondary: Color::Rgb(33, 34, 44),
            bg_highlight: current_line,
            bg_selection: current_line,

            fg_primary: foreground,
            fg_secondary: Color::Rgb(200, 200, 220),
            fg_muted: comment,
            fg_accent: pink,
            fg_accent_dim: purple,

            border: comment,
            border_focused: purple,
            title_focused: pink,

            error: red,
            success: green,
            warning: yellow,

            selection_bar_bg: purple,
            selection_bar_fg: bg,
            selection_text: bg,
            transport_bg: Color::Rgb(30, 31, 40),
            shortcut_key: cyan,
            shortcut_text: foreground,
        }
    }

    /// Nord — muted icy-blue dark theme.
    /// Reference palette: https://www.nordtheme.com/docs/colors-and-palettes
    pub fn nord() -> Self {
        // Polar Night
        let nord0 = Color::Rgb(46, 52, 64);        // #2e3440
        let nord1 = Color::Rgb(59, 66, 82);        // #3b4252
        let nord2 = Color::Rgb(67, 76, 94);        // #434c5e
        let nord3 = Color::Rgb(76, 86, 106);       // #4c566a
        // Snow Storm
        let nord4 = Color::Rgb(216, 222, 233);     // #d8dee9
        let nord5 = Color::Rgb(229, 233, 240);     // #e5e9f0
        let _nord6 = Color::Rgb(236, 239, 244);    // #eceff4
        // Frost
        let nord7 = Color::Rgb(143, 188, 187);     // #8fbcbb
        let nord8 = Color::Rgb(136, 192, 208);     // #88c0d0
        let nord9 = Color::Rgb(129, 161, 193);     // #81a1c1
        let nord10 = Color::Rgb(94, 129, 172);     // #5e81ac
        // Aurora
        let nord11 = Color::Rgb(191, 97, 106);     // #bf616a — red
        let nord13 = Color::Rgb(235, 203, 139);    // #ebcb8b — yellow
        let nord14 = Color::Rgb(163, 190, 140);    // #a3be8c — green
        Self {
            bg_primary: nord0,
            bg_secondary: nord1,
            bg_highlight: nord2,
            bg_selection: nord3,

            fg_primary: nord4,
            fg_secondary: nord5,
            fg_muted: Color::Rgb(120, 130, 150),
            fg_accent: nord8,
            fg_accent_dim: nord9,

            border: nord3,
            border_focused: nord8,
            title_focused: nord7,

            error: nord11,
            success: nord14,
            warning: nord13,

            selection_bar_bg: nord10,
            selection_bar_fg: nord5,
            selection_text: nord5,
            transport_bg: nord1,
            shortcut_key: nord8,
            shortcut_text: nord4,
        }
    }

    /// Get colors for a theme name.
    pub fn for_theme(theme: ThemeName) -> Self {
        match theme {
            ThemeName::Dark => Self::dark(),
            ThemeName::SolarizedDark => Self::solarized_dark(),
            ThemeName::SolarizedLight => Self::solarized_light(),
            ThemeName::Platinum => Self::platinum(),
            ThemeName::BlackAndWhite => Self::black_and_white(),
            ThemeName::Amber => Self::amber(),
            ThemeName::PhosphorGreen => Self::phosphor_green(),
            ThemeName::Norton => Self::norton(),
            ThemeName::Dracula => Self::dracula(),
            ThemeName::Nord => Self::nord(),
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
