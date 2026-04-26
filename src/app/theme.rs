//! UI-agnostic theme identifier.
//!
//! Lives in the app layer so `AppState` does not depend on any concrete UI
//! (ratatui, iced). Each UI maps `ThemeName` to its own concrete palette:
//! - TUI: `crate::ui::theme::Theme` / `ThemeColors` (ratatui styles)
//! - GUI: `crate::ui_gui::theme` (iced Theme)

use serde::{Deserialize, Serialize};

/// Named application theme. UI-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeName {
    #[default]
    SolarizedDark,
    SolarizedLight,
    Dark,
    Borland,
    Platinum,
    BlackAndWhite,
}

impl ThemeName {
    /// All available themes.
    pub fn all() -> &'static [ThemeName] {
        &[
            ThemeName::SolarizedDark,
            ThemeName::SolarizedLight,
            ThemeName::Dark,
            ThemeName::Borland,
            ThemeName::Platinum,
            ThemeName::BlackAndWhite,
        ]
    }

    /// Display name for the theme.
    pub fn display_name(&self) -> &'static str {
        match self {
            ThemeName::Dark => "dark",
            ThemeName::SolarizedDark => "solarized dark",
            ThemeName::SolarizedLight => "solarized light",
            ThemeName::Borland => "borland",
            ThemeName::Platinum => "platinum",
            ThemeName::BlackAndWhite => "black and white",
        }
    }

    /// Config string value.
    pub fn config_name(&self) -> &'static str {
        match self {
            ThemeName::Dark => "dark",
            ThemeName::SolarizedDark => "solarized-dark",
            ThemeName::SolarizedLight => "solarized-light",
            ThemeName::Borland => "borland",
            ThemeName::Platinum => "platinum",
            ThemeName::BlackAndWhite => "black-and-white",
        }
    }

    /// Parse from config string.
    pub fn from_config(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "solarized-dark" | "solarizeddark" => ThemeName::SolarizedDark,
            "solarized-light" | "solarizedlight" => ThemeName::SolarizedLight,
            "dark" => ThemeName::Dark,
            "borland" | "retro" | "norton" => ThemeName::Borland,
            "platinum" | "mac" | "macos9" => ThemeName::Platinum,
            "black-and-white" | "blackandwhite" | "bw" | "mono" => ThemeName::BlackAndWhite,
            _ => ThemeName::SolarizedDark,
        }
    }

    /// Cycle to the next theme.
    pub fn next(&self) -> Self {
        match self {
            ThemeName::SolarizedDark => ThemeName::SolarizedLight,
            ThemeName::SolarizedLight => ThemeName::Dark,
            ThemeName::Dark => ThemeName::Borland,
            ThemeName::Borland => ThemeName::Platinum,
            ThemeName::Platinum => ThemeName::BlackAndWhite,
            ThemeName::BlackAndWhite => ThemeName::SolarizedDark,
        }
    }
}
