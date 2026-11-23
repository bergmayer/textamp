//! Ratatui styles projected from shared semantic palettes.
pub use crate::app::theme::ThemeName;
use ratatui::style::{Color, Modifier, Style};

/// Terminal projection of the application palette.
pub type ThemeColors = crate::app::theme_palette::Palette<Color>;
fn terminal_color(color: crate::app::theme_palette::Color) -> Color {
    use crate::app::theme_palette::Color as Shared;
    match color {
        Shared::Rgb(r, g, b) => Color::Rgb(r, g, b),
        Shared::Black => Color::Black,
        Shared::White => Color::White,
        Shared::Reset => Color::Reset,
    }
}

fn contrast_text(background: Color, preferred: Color) -> Color {
    fn luminance(color: Color) -> Option<f64> {
        let (r, g, b) = match color {
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Black => (0, 0, 0),
            Color::White => (255, 255, 255),
            _ => return None,
        };
        let linear = |v: u8| {
            let v = f64::from(v) / 255.;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        Some(0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b))
    }
    let Some(bg) = luminance(background) else {
        return preferred;
    };
    if luminance(preferred).is_some_and(|fg| (fg.max(bg) + 0.05) / (fg.min(bg) + 0.05) >= 4.5) {
        preferred
    } else if bg > 0.179 {
        Color::Black
    } else {
        Color::White
    }
}

impl ThemeColors {
    /// Keyboard selection uses the theme accent with a contrasting label.
    pub fn focused_row(&self) -> Style {
        Style::default()
            .bg(self.fg_accent)
            .fg(contrast_text(self.fg_accent, self.selection_text))
    }

    /// Retain the navigation path without presenting an inactive row as focused.
    pub fn inactive_row(&self) -> Style {
        Style::default()
            .bg(self.bg_highlight)
            .fg(contrast_text(self.bg_highlight, self.fg_primary))
    }

    /// Readable primary text on the playback track card.
    pub fn track_card_style(&self) -> Style {
        let background = self.track_card_background();
        Style::default()
            .bg(background)
            .fg(contrast_text(background, self.fg_primary))
    }

    /// Secondary metadata remains legible in monochrome and inverted themes.
    pub fn track_card_secondary_style(&self) -> Style {
        let background = self.track_card_background();
        Style::default()
            .bg(background)
            .fg(contrast_text(background, self.fg_muted))
    }

    /// Distinguish the track card from the surrounding playback controls.
    pub fn track_card_background(&self) -> Color {
        let source = |color| match color {
            Color::Rgb(r, g, b) => crate::app::theme_palette::Color::Rgb(r, g, b),
            Color::Black => crate::app::theme_palette::Color::Black,
            Color::White => crate::app::theme_palette::Color::White,
            _ => crate::app::theme_palette::Color::Reset,
        };
        terminal_color(crate::app::theme_palette::track_card_background(
            source(self.bg_highlight),
            source(self.transport_bg),
            source(self.fg_primary),
        ))
    }
    /// Translate the shared palette into terminal colors.
    pub fn for_theme(name: ThemeName) -> Self {
        crate::app::theme_palette::Palette::<crate::app::theme_palette::Color>::for_theme(name)
            .map(terminal_color)
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
        self.colors.inactive_row()
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

    /// Border color (unfocused).
    pub fn border_color(&self) -> Color {
        self.colors.border
    }
}

// ==================== Global Theme State ====================
// Thread-local storage for the active theme.

use std::cell::RefCell;

thread_local! {
    static ACTIVE_THEME: RefCell<Theme> = RefCell::new(Theme::default());
    #[cfg(all(feature="desktop",target_os="macos"))]
    static HOST_PALETTE: RefCell<Option<ThemeColors>> = const {RefCell::new(None)};
}

/// Resolve System roles from the embedded native host without changing named palettes.
#[cfg(all(feature = "desktop", target_os = "macos"))]
pub(crate) fn set_host_palette(palette: Option<crate::app::theme_palette::Palette<[u8; 3]>>) {
    HOST_PALETTE.with(|host| {
        *host.borrow_mut() = palette.map(|palette| palette.map(|[r, g, b]| Color::Rgb(r, g, b)))
    });
}

/// Set the active theme globally.
pub fn set_theme(name: ThemeName) {
    ACTIVE_THEME.with(|t| {
        let theme = Theme::new(name);
        #[cfg(all(feature = "desktop", target_os = "macos"))]
        let theme = if name == ThemeName::System {
            HOST_PALETTE.with(|host| {
                host.borrow()
                    .map_or(theme, |colors| Theme { colors, ..theme })
            })
        } else {
            theme
        };
        *t.borrow_mut() = theme;
    });
}

/// Get the active theme.
pub fn theme() -> Theme {
    ACTIVE_THEME.with(|t| *t.borrow())
}

// ==================== Static Style Methods ====================
// These use the global theme state.

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
}
