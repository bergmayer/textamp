//! Map `app::theme::ThemeName` to an `iced::Theme` — custom palettes
//! built so the GUI tracks the TUI's Plexamp-orange / Solarized /
//! Borland look rather than falling back to off-the-shelf Iced themes.

use iced::theme::palette::{Background, Danger, Extended, Pair, Primary, Secondary, Success};
use iced::theme::{Custom, Palette};
use iced::{Color, Theme};
use std::sync::{Arc, OnceLock};

use crate::app::theme::ThemeName;

/// The single `Arc<Custom>` we hand out for the strict B&W theme.
/// Cached so `iced_theme(BlackAndWhite)` always returns the same Arc
/// — that lets `is_monochrome` compare by `Arc::ptr_eq` without any
/// string formatting in the hot render path.
fn bw_theme_singleton() -> &'static Arc<Custom> {
    static CELL: OnceLock<Arc<Custom>> = OnceLock::new();
    CELL.get_or_init(|| {
        let palette = Palette {
            background: rgb(255, 255, 255),
            text:       rgb(0, 0, 0),
            primary:    rgb(0, 0, 0),
            success:    rgb(0, 0, 0),
            danger:     rgb(0, 0, 0),
        };
        Arc::new(Custom::with_fn(
            BLACK_AND_WHITE_NAME.to_string(),
            palette,
            bw_extended,
        ))
    })
}

pub fn iced_theme(name: ThemeName) -> Theme {
    match name {
        ThemeName::Dark => Theme::Dark,
        ThemeName::SolarizedDark => Theme::SolarizedDark,
        ThemeName::SolarizedLight => Theme::SolarizedLight,
        ThemeName::Borland => borland_theme(),
        ThemeName::Platinum => platinum_theme(),
        ThemeName::BlackAndWhite => black_and_white_theme(),
    }
}

/// True when the active theme is the strict pure-black + pure-white
/// palette. Several styled widgets need to flip their highlight
/// swatches in this theme because every "soft" pair (background.weak,
/// primary.weak) collapses to white-on-white and is invisible on the
/// body. Zero-alloc: compares by Arc identity against the singleton
/// in `bw_theme_singleton`.
pub fn is_monochrome(theme: &Theme) -> bool {
    matches!(theme, Theme::Custom(c) if Arc::ptr_eq(c, bw_theme_singleton()))
}

/// User-visible name of the strict-monochrome theme.
pub const BLACK_AND_WHITE_NAME: &str = "Black and White";

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}

/// Classic Turbo Pascal / Norton Commander palette — blue background,
/// white text, cyan selection, yellow accents. Matches the TUI's
/// `ThemeColors::borland()` so the two front-ends look equivalent.
fn borland_theme() -> Theme {
    let palette = Palette {
        background: rgb(0, 0, 170),  // classic #0000AA blue
        text:       rgb(255, 255, 255),
        primary:    rgb(0, 170, 170), // cyan — selection bar
        success:    rgb(0, 170, 0),
        danger:     rgb(170, 0, 0),
    };
    Theme::Custom(Arc::new(Custom::new("Borland".to_string(), palette)))
}

/// Mac OS 9 "Platinum" palette — light gray window chrome, black text,
/// classic highlight blue for selection.
fn platinum_theme() -> Theme {
    let palette = Palette {
        background: rgb(221, 221, 221), // #DDDDDD platinum gray
        text:       rgb(0, 0, 0),
        primary:    rgb(59, 120, 255),  // #3B78FF highlight blue
        success:    rgb(0, 128, 0),
        danger:     rgb(204, 0, 0),
    };
    Theme::Custom(Arc::new(Custom::new("Platinum".to_string(), palette)))
}

/// Strict monochrome palette — pure black and pure white only.
///
/// `Extended::generate` mixes 15 % / 40 % text colour into the base
/// to derive `background.weak` / `background.strong`; with white +
/// black that produces light-grey tones that show up as a tint on
/// the menu and transport bars (which use `background.weak` for
/// their fill). Bypass the generator entirely so weak / base remain
/// pure white and strong is pure black — the bars then read as
/// white on white, separated only by a 1 px black border.
///
/// The Arc is cached in `bw_theme_singleton` so every call returns
/// the same handle — required for `is_monochrome`'s `Arc::ptr_eq`
/// fast path.
fn black_and_white_theme() -> Theme {
    Theme::Custom(bw_theme_singleton().clone())
}

/// Custom Extended palette generator for the Black and White theme.
///
/// Key invariant: every `Pair` is paired so `Pair::text` is readable
/// on `Pair::color`. Styles in this codebase that pick a `(bg, fg)`
/// from the same pair (`primary.weak.color, primary.weak.text` etc.)
/// stay legible in every theme. Backgrounds collapse to the white /
/// black extremes:
///   - `background.weak` and `.base` = pure white field.
///   - `background.strong` = pure black border / pressed-state fill.
///   - `primary.weak` and `.base` = white (raised look for unpressed
///     buttons / inactive tabs / hover, paired with black text).
///   - `primary.strong` = black (selection / active fill, paired with
///     white text).
fn bw_extended(palette: Palette) -> Extended {
    let white = palette.background;
    let black = palette.text;

    let pair_black_on_white = Pair { color: white, text: black };
    let pair_white_on_black = Pair { color: black, text: white };

    Extended {
        background: Background {
            base:   pair_black_on_white,
            weak:   pair_black_on_white,
            strong: pair_white_on_black,
        },
        primary: Primary {
            // Unpressed / inactive surfaces use the white-field pair
            // so buttons read as raised against the body. The strong
            // pair flips to black-on-white for pressed / selected
            // chrome.
            base:   pair_black_on_white,
            weak:   pair_black_on_white,
            strong: pair_white_on_black,
        },
        secondary: Secondary {
            base:   pair_black_on_white,
            weak:   pair_black_on_white,
            strong: pair_white_on_black,
        },
        success: Success {
            base:   pair_white_on_black,
            weak:   pair_black_on_white,
            strong: pair_white_on_black,
        },
        danger: Danger {
            base:   pair_white_on_black,
            weak:   pair_black_on_white,
            strong: pair_white_on_black,
        },
        is_dark: false,
    }
}
