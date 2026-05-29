//! UI-agnostic theme identifier.
//!
//! Lives in the app layer so `AppState` does not depend on ratatui directly.
//! The UI layer maps `ThemeName` to a concrete `crate::ui::theme::Theme`
//! palette.

use serde::{Deserialize, Serialize};

/// Named application theme. UI-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeName {
    #[default]
    SolarizedDark,
    SolarizedLight,
    Dark,
    Platinum,
    BlackAndWhite,
    /// Vintage amber-on-black CRT (IBM 3270 / DEC monochrome look).
    Amber,
    /// Vintage green-on-black CRT (Apple ][ / DEC VT terminal look).
    PhosphorGreen,
    /// Norton Commander DOS file manager — blue background with
    /// cyan / yellow / white accents.
    Norton,
    /// Dracula — dark gray background with purple / pink / cyan accents.
    Dracula,
    /// Nord — muted icy-blue dark theme.
    Nord,
}

impl ThemeName {
    /// All available themes.
    pub fn all() -> &'static [ThemeName] {
        &[
            ThemeName::SolarizedDark,
            ThemeName::SolarizedLight,
            ThemeName::Dark,
            ThemeName::Dracula,
            ThemeName::Nord,
            ThemeName::Norton,
            ThemeName::Platinum,
            ThemeName::BlackAndWhite,
            ThemeName::Amber,
            ThemeName::PhosphorGreen,
        ]
    }

    /// Display name for the theme.
    pub fn display_name(&self) -> &'static str {
        match self {
            ThemeName::Dark => "dark",
            ThemeName::SolarizedDark => "solarized dark",
            ThemeName::SolarizedLight => "solarized light",
            ThemeName::Platinum => "platinum",
            ThemeName::BlackAndWhite => "black and white",
            ThemeName::Amber => "amber crt",
            ThemeName::PhosphorGreen => "phosphor green",
            ThemeName::Norton => "norton",
            ThemeName::Dracula => "dracula",
            ThemeName::Nord => "nord",
        }
    }

    /// Config string value.
    pub fn config_name(&self) -> &'static str {
        match self {
            ThemeName::Dark => "dark",
            ThemeName::SolarizedDark => "solarized-dark",
            ThemeName::SolarizedLight => "solarized-light",
            ThemeName::Platinum => "platinum",
            ThemeName::BlackAndWhite => "black-and-white",
            ThemeName::Amber => "amber",
            ThemeName::PhosphorGreen => "phosphor-green",
            ThemeName::Norton => "norton",
            ThemeName::Dracula => "dracula",
            ThemeName::Nord => "nord",
        }
    }

    /// Parse from config string.
    pub fn from_config(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "solarized-dark" | "solarizeddark" => ThemeName::SolarizedDark,
            "solarized-light" | "solarizedlight" => ThemeName::SolarizedLight,
            "dark" => ThemeName::Dark,
            "platinum" | "mac" | "macos9" => ThemeName::Platinum,
            "black-and-white" | "blackandwhite" | "bw" | "mono" => ThemeName::BlackAndWhite,
            "amber" | "amber-crt" | "ambercrt" => ThemeName::Amber,
            "phosphor" | "phosphor-green" | "phosphorgreen" | "green" => ThemeName::PhosphorGreen,
            // Old "borland" config values map to Norton — they're the
            // same family (DOS-blue + cyan/yellow), and Norton is the
            // surviving theme.
            "norton" | "norton-commander" | "nc" | "dos" | "borland" | "retro" => ThemeName::Norton,
            "dracula" => ThemeName::Dracula,
            "nord" => ThemeName::Nord,
            _ => ThemeName::SolarizedDark,
        }
    }

    /// Cycle to the next theme.
    pub fn next(&self) -> Self {
        let all = Self::all();
        let idx = all.iter().position(|t| t == self).unwrap_or(0);
        all[(idx + 1) % all.len()]
    }
}
