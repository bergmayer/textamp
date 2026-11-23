//! Terminal Note application artwork shared by platform icon exporters.
use crate::app::{
    theme::ThemeName,
    theme_palette::{Color, Palette, mix, readable_rgb},
};
use image::{Rgba, RgbaImage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Ios,
    MacOs,
}

// Each occupied character is one cell on the same 28 × 28 canvas as the prompt.
const NOTE: [&str; 18] = [
    "..............XX",
    "...........XXXXX",
    "........XXXXXXXX",
    ".....XXXXXXXXXXX",
    ".....XXX......XX",
    ".....XX.......XX",
    ".....XX.......XX",
    ".....XX.......XX",
    ".....XX.......XX",
    ".....XX.......XX",
    ".....XX.......XX",
    "...XXXX....XXXXX",
    ".XXXXXX..XXXXXXX",
    "XXXXXXX.XXXXXXXX",
    "XXXXXXX.XXXXXXXX",
    "XXXXXX..XXXXXXX.",
    ".XXXX...XXXXXX..",
    "..XX......XX....",
];
const PROMPT: [(i32, i32); 8] = [
    (3, 9),
    (4, 10),
    (5, 11),
    (4, 12),
    (3, 13),
    (7, 13),
    (8, 13),
    (9, 13),
];

fn rgb(color: Color) -> [u8; 3] {
    color.rgb().unwrap_or([255; 3])
}
fn readable(color: [u8; 3], background: [u8; 3]) -> [u8; 3] {
    readable_rgb(color, &[background], 3.)
}
fn palette(theme: ThemeName, appearance: Appearance) -> Palette {
    let mut palette = Palette::<Color>::for_theme(theme);
    if theme == ThemeName::System {
        let dark = appearance == Appearance::Dark;
        palette.bg_primary = if dark {
            Color::Rgb(28, 28, 30)
        } else {
            Color::Rgb(242, 242, 247)
        };
        palette.fg_primary = if dark {
            Color::Rgb(242, 242, 247)
        } else {
            Color::Rgb(28, 28, 30)
        };
        palette.fg_accent = if dark {
            Color::Rgb(10, 132, 255)
        } else {
            Color::Rgb(0, 122, 255)
        };
        palette.success = if dark {
            Color::Rgb(48, 209, 88)
        } else {
            Color::Rgb(36, 138, 61)
        };
    }
    palette
}
fn note(x: i32, y: i32) -> bool {
    let (x, y) = (x - 8, y - 5);
    x >= 0
        && y >= 0
        && NOTE
            .get(y as usize)
            .and_then(|row| row.as_bytes().get(x as usize))
            == Some(&b'X')
}

/// iOS supplies its own mask; macOS artwork includes an inset rounded plate and alpha.
pub fn render(
    theme: ThemeName,
    appearance: Appearance,
    size: u32,
    platform: Platform,
) -> RgbaImage {
    let palette = palette(theme, appearance);
    let background = rgb(palette.bg_primary);
    let note_color = if theme == ThemeName::HotDogStand {
        palette.bg_secondary
    } else {
        palette.fg_accent
    };
    let body = readable(rgb(note_color), background);
    let highlight = if platform == Platform::Ios {
        readable(mix(body, [255; 3], 0.2), background)
    } else {
        readable(rgb(palette.fg_primary), background)
    };
    let shadow = readable(mix(body, background, 0.3), background);
    let prompt = if platform == Platform::Ios {
        body
    } else {
        readable(rgb(palette.success), background)
    };
    let inset = if platform == Platform::MacOs {
        f64::from(size) / 16.
    } else {
        0.
    };
    let side = f64::from(size) - 2. * inset;
    let cell = side / 28.;
    RgbaImage::from_fn(size, size, |x, y| {
        let (x, y) = (f64::from(x) + 0.5 - inset, f64::from(y) + 0.5 - inset);
        let alpha = if platform == Platform::MacOs {
            let radius = side * 0.22;
            let dx = (x - side / 2.).abs() - side / 2. + radius;
            let dy = (y - side / 2.).abs() - side / 2. + radius;
            let distance = dx.max(0.).hypot(dy.max(0.)) + dx.max(dy).min(0.) - radius;
            ((0.5 - distance).clamp(0., 1.) * 255.).round() as u8
        } else {
            255
        };
        if alpha == 0 {
            return Rgba([0; 4]);
        }
        let (column, row) = ((x / cell).floor() as i32, (y / cell).floor() as i32);
        let is_note = note(column, row);
        let is_prompt = PROMPT.contains(&(column, row));
        let color = if is_note || is_prompt {
            // Narrow seams keep individual terminal cells visible at large sizes.
            let gap = (cell * 0.06).min(3.);
            let on_seam = x.rem_euclid(cell) < gap || y.rem_euclid(cell) < gap;
            if on_seam {
                background
            } else if is_prompt {
                prompt
            } else if !note(column, row - 1) || !note(column - 1, row) {
                highlight
            } else if !note(column, row + 1) || !note(column + 1, row) {
                shadow
            } else {
                body
            }
        } else {
            background
        };
        Rgba([color[0], color[1], color[2], alpha])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_theme_is_legible_on_both_platforms_and_system_appearances() {
        for &theme in ThemeName::all() {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let colors = palette(theme, appearance);
                let background = rgb(colors.bg_primary);
                for color in [colors.fg_accent, colors.fg_primary, colors.success] {
                    assert!(
                        crate::app::theme_palette::contrast(
                            readable(rgb(color), background),
                            background
                        ) >= 3.,
                        "{theme:?}"
                    );
                }
                let ios = render(theme, appearance, 128, Platform::Ios);
                assert!(ios.pixels().all(|p| p[3] == 255));
                assert!(ios.pixels().any(|p| p.0[..3] != background));
                let mac = render(theme, appearance, 128, Platform::MacOs);
                assert_eq!(mac[(0, 0)][3], 0);
                assert_eq!(mac[(64, 64)][3], 255);
            }
        }
        assert_ne!(
            render(ThemeName::System, Appearance::Light, 128, Platform::Ios),
            render(ThemeName::System, Appearance::Dark, 128, Platform::Ios)
        );
    }
}
