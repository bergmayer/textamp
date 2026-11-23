//! Shared semantic palettes without terminal or native widget dependencies.
use super::theme::ThemeName;

/// Literal color or the host's default color for System appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Rgb(u8, u8, u8),
    Black,
    White,
    Reset,
}

impl Color {
    pub(crate) fn rgb(self) -> Option<[u8; 3]> {
        match self {
            Self::Rgb(r, g, b) => Some([r, g, b]),
            Self::Black => Some([0; 3]),
            Self::White => Some([255; 3]),
            Self::Reset => None,
        }
    }
}

fn luminance(color: [u8; 3]) -> f64 {
    let linear = |v: u8| {
        let v = f64::from(v) / 255.;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color[0]) + 0.7152 * linear(color[1]) + 0.0722 * linear(color[2])
}
pub(crate) fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
pub(crate) fn mix(a: [u8; 3], b: [u8; 3], amount: f64) -> [u8; 3] {
    std::array::from_fn(|i| {
        (f64::from(a[i]) * (1. - amount) + f64::from(b[i]) * amount).round() as u8
    })
}

/// Move a palette color toward black or white just far enough to read on its surfaces.
pub(crate) fn readable_rgb(color: [u8; 3], backgrounds: &[[u8; 3]], minimum: f64) -> [u8; 3] {
    let weakest = |candidate| {
        backgrounds
            .iter()
            .map(|bg| contrast(candidate, *bg))
            .fold(f64::INFINITY, f64::min)
    };
    if weakest(color) >= minimum {
        return color;
    }
    let target = if weakest([0; 3]) > weakest([255; 3]) {
        [0; 3]
    } else {
        [255; 3]
    };
    (1..=20)
        .map(|step| mix(color, target, f64::from(step) / 20.))
        .find(|candidate| weakest(*candidate) >= minimum)
        .unwrap_or(target)
}

/// Color palette with semantic naming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette<C = Color> {
    // Backgrounds
    pub bg_primary: C,
    pub bg_secondary: C,
    pub bg_highlight: C,
    pub bg_selection: C,

    // Foregrounds
    pub fg_primary: C,
    pub fg_secondary: C,
    pub fg_muted: C,
    pub fg_accent: C,
    pub fg_accent_dim: C,

    // Borders
    pub border: C,
    pub border_focused: C,
    pub title_focused: C,

    // Semantic colors
    pub error: C,
    pub success: C,
    pub warning: C,

    // Special elements
    pub selection_bar_bg: C,
    pub selection_bar_fg: C,
    pub selection_text: C,
    pub transport_bg: C,
    pub shortcut_key: C,
    pub shortcut_text: C,
}

impl<C> Palette<C> {
    /// Translate all semantic tokens at a frontend boundary.
    pub fn map<D>(self, mut convert: impl FnMut(C) -> D) -> Palette<D> {
        Palette {
            bg_primary: convert(self.bg_primary),
            bg_secondary: convert(self.bg_secondary),
            bg_highlight: convert(self.bg_highlight),
            bg_selection: convert(self.bg_selection),
            fg_primary: convert(self.fg_primary),
            fg_secondary: convert(self.fg_secondary),
            fg_muted: convert(self.fg_muted),
            fg_accent: convert(self.fg_accent),
            fg_accent_dim: convert(self.fg_accent_dim),
            border: convert(self.border),
            border_focused: convert(self.border_focused),
            title_focused: convert(self.title_focused),
            error: convert(self.error),
            success: convert(self.success),
            warning: convert(self.warning),
            selection_bar_bg: convert(self.selection_bar_bg),
            selection_bar_fg: convert(self.selection_bar_fg),
            selection_text: convert(self.selection_text),
            transport_bg: convert(self.transport_bg),
            shortcut_key: convert(self.shortcut_key),
            shortcut_text: convert(self.shortcut_text),
        }
    }
}
impl Palette {
    /// Default dark theme (warm).
    pub fn dark() -> Self {
        Self {
            bg_primary: Color::Rgb(24, 24, 24),
            bg_secondary: Color::Rgb(35, 35, 35),
            bg_highlight: Color::Rgb(45, 45, 45),
            bg_selection: Color::Rgb(60, 60, 60),

            fg_primary: Color::Rgb(220, 220, 220),
            fg_secondary: Color::Rgb(180, 180, 180),
            fg_muted: Color::Rgb(128, 128, 128),
            fg_accent: Color::Rgb(229, 160, 60), // Warm orange
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
        let base03 = Color::Rgb(0, 43, 54); // #002b36 - background
        let base02 = Color::Rgb(7, 54, 66); // #073642 - background highlight
        let base01 = Color::Rgb(88, 110, 117); // #586e75 - optional emphasis
        let base00 = Color::Rgb(101, 123, 131); // #657b83 - body text
        let base0 = Color::Rgb(131, 148, 150); // #839496 - primary content
        let base1 = Color::Rgb(147, 161, 161); // #93a1a1 - comments

        // Solarized accent colors
        let yellow = Color::Rgb(181, 137, 0); // #b58900
        let orange = Color::Rgb(203, 75, 22); // #cb4b16
        let red = Color::Rgb(220, 50, 47); // #dc322f
        let blue = Color::Rgb(38, 139, 210); // #268bd2
        let cyan = Color::Rgb(42, 161, 152); // #2aa198
        let green = Color::Rgb(133, 153, 0); // #859900

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
        let _base03 = Color::Rgb(0, 43, 54); // #002b36
        let _base02 = Color::Rgb(7, 54, 66); // #073642
        let base01 = Color::Rgb(88, 110, 117); // #586e75
        let base00 = Color::Rgb(101, 123, 131); // #657b83 - primary content (light mode)
        let _base0 = Color::Rgb(131, 148, 150); // #839496
        let _base1 = Color::Rgb(147, 161, 161); // #93a1a1
        let base2 = Color::Rgb(238, 232, 213); // #eee8d5 - background highlight
        let base3 = Color::Rgb(253, 246, 227); // #fdf6e3 - background

        // Solarized accent colors
        let _yellow = Color::Rgb(181, 137, 0); // #b58900
        let orange = Color::Rgb(203, 75, 22); // #cb4b16
        let red = Color::Rgb(220, 50, 47); // #dc322f
        let magenta = Color::Rgb(211, 54, 130); // #d33682
        let blue = Color::Rgb(38, 139, 210); // #268bd2
        let cyan = Color::Rgb(42, 161, 152); // #2aa198
        let green = Color::Rgb(133, 153, 0); // #859900

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
        let bg = Color::Rgb(221, 221, 221); // platinum gray #DDDDDD
        let bg_light = Color::Rgb(238, 238, 238); // #EEEEEE (lighter field)
        let bg_shadow = Color::Rgb(170, 170, 170); // #AAAAAA (groove shadow)
        let text = Color::Rgb(0, 0, 0);
        let muted = Color::Rgb(102, 102, 102); // #666666
        let blue = Color::Rgb(59, 120, 255); // #3B78FF classic highlight
        let dark_blue = Color::Rgb(0, 58, 168); // deeper blue
        let red = Color::Rgb(204, 0, 0);
        let green = Color::Rgb(0, 128, 0);

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

    /// Pure black backgrounds with half-bright white text.
    pub fn white_on_black() -> Self {
        let background = Color::Rgb(0, 0, 0);
        let foreground = Color::Rgb(128, 128, 128);

        Self {
            bg_primary: background,
            bg_secondary: background,
            bg_highlight: background,
            bg_selection: background,

            fg_primary: foreground,
            fg_secondary: foreground,
            fg_muted: foreground,
            fg_accent: foreground,
            fg_accent_dim: foreground,

            border: foreground,
            border_focused: foreground,
            title_focused: foreground,

            error: foreground,
            success: foreground,
            warning: foreground,

            selection_bar_bg: background,
            selection_bar_fg: foreground,
            selection_text: foreground,
            transport_bg: background,
            shortcut_key: foreground,
            shortcut_text: foreground,
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

    /// Norton Commander colors: blue background, cyan title, yellow accents.
    pub fn norton() -> Self {
        let bg_blue = Color::Rgb(0, 0, 170); // DOS bright blue
        let cyan = Color::Rgb(85, 255, 255); // DOS bright cyan (titles)
        let yellow = Color::Rgb(255, 255, 85); // DOS bright yellow (highlight)
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
        let bg = Color::Rgb(40, 42, 54); // #282a36
        let current_line = Color::Rgb(68, 71, 90); // #44475a
        let foreground = Color::Rgb(248, 248, 242); // #f8f8f2
        let comment = Color::Rgb(98, 114, 164); // #6272a4
        let cyan = Color::Rgb(139, 233, 253); // #8be9fd
        let green = Color::Rgb(80, 250, 123); // #50fa7b
        let pink = Color::Rgb(255, 121, 198); // #ff79c6
        let purple = Color::Rgb(189, 147, 249); // #bd93f9
        let red = Color::Rgb(255, 85, 85); // #ff5555
        let yellow = Color::Rgb(241, 250, 140); // #f1fa8c
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
        let nord0 = Color::Rgb(46, 52, 64); // #2e3440
        let nord1 = Color::Rgb(59, 66, 82); // #3b4252
        let nord2 = Color::Rgb(67, 76, 94); // #434c5e
        let nord3 = Color::Rgb(76, 86, 106); // #4c566a
        // Snow Storm
        let nord4 = Color::Rgb(216, 222, 233); // #d8dee9
        let nord5 = Color::Rgb(229, 233, 240); // #e5e9f0
        // Frost
        let focus_pink = Color::Rgb(196, 145, 162); // #c491a2 - subdued focus pink
        let nord8 = Color::Rgb(136, 192, 208); // #88c0d0
        let nord9 = Color::Rgb(129, 161, 193); // #81a1c1
        let nord10 = Color::Rgb(94, 129, 172); // #5e81ac
        // Aurora
        let nord11 = Color::Rgb(191, 97, 106); // #bf616a — red
        let nord13 = Color::Rgb(235, 203, 139); // #ebcb8b — yellow
        let nord14 = Color::Rgb(163, 190, 140); // #a3be8c — green
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
            border_focused: focus_pink,
            title_focused: focus_pink,

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

    pub fn hot_dog_stand() -> Self {
        let yellow = Color::Rgb(255, 255, 0);
        let red = Color::Rgb(255, 60, 8);
        let ink = Color::Rgb(68, 0, 0);
        Self {
            bg_primary: red,
            bg_secondary: yellow,
            bg_highlight: Color::Rgb(255, 194, 0),
            bg_selection: yellow,
            fg_primary: Color::Black,
            fg_secondary: Color::Black,
            fg_muted: ink,
            fg_accent: ink,
            fg_accent_dim: ink,
            border: ink,
            border_focused: ink,
            title_focused: ink,
            error: ink,
            success: Color::Rgb(0, 77, 0),
            warning: Color::Rgb(100, 45, 0),
            selection_bar_bg: yellow,
            selection_bar_fg: Color::Black,
            selection_text: Color::Black,
            transport_bg: yellow,
            shortcut_key: ink,
            shortcut_text: Color::Black,
        }
    }

    /// Get colors for a theme name.
    pub fn for_theme(theme: ThemeName) -> Self {
        let palette = match theme {
            ThemeName::System => {
                let mut colors = Self::dark();
                colors.bg_primary = Color::Reset;
                colors.bg_secondary = Color::Reset;
                colors.fg_primary = Color::Reset;
                colors
            }
            ThemeName::Dark => Self::dark(),
            ThemeName::SolarizedDark => Self::solarized_dark(),
            ThemeName::SolarizedLight => Self::solarized_light(),
            ThemeName::Platinum => Self::platinum(),
            ThemeName::BlackAndWhite => Self::black_and_white(),
            ThemeName::WhiteOnBlack => Self::white_on_black(),
            ThemeName::Amber => Self::amber(),
            ThemeName::PhosphorGreen => Self::phosphor_green(),
            ThemeName::Norton => Self::norton(),
            ThemeName::Dracula => Self::dracula(),
            ThemeName::Nord => Self::nord(),
            ThemeName::HotDogStand => Self::hot_dog_stand(),
        };
        palette.readable()
    }

    fn readable(mut self) -> Self {
        let (Some(canvas), Some(card), Some(transport)) = (
            self.bg_primary.rgb(),
            self.bg_secondary.rgb(),
            self.transport_bg.rgb(),
        ) else {
            return self;
        };
        let surfaces = [canvas, card, transport];
        let repair = |ink: &mut Color, backgrounds: &[[u8; 3]]| {
            if let Some(rgb) = ink.rgb() {
                let adjusted = readable_rgb(rgb, backgrounds, 4.5);
                if adjusted != rgb {
                    *ink = Color::Rgb(adjusted[0], adjusted[1], adjusted[2]);
                }
            }
        };
        for ink in [
            &mut self.fg_accent,
            &mut self.fg_accent_dim,
            &mut self.title_focused,
            &mut self.shortcut_key,
            &mut self.error,
            &mut self.success,
            &mut self.warning,
        ] {
            repair(ink, &surfaces);
        }
        if let Some(rgb) = self.border_focused.rgb() {
            let [r, g, b] = readable_rgb(rgb, &surfaces, 3.);
            if [r, g, b] != rgb {
                self.border_focused = Color::Rgb(r, g, b);
            }
        }
        let accent = self.fg_accent.rgb().unwrap_or(canvas);
        let text_surfaces = [
            canvas,
            card,
            transport,
            mix(canvas, accent, 0.18),
            mix(card, accent, 0.18),
        ];
        for ink in [
            &mut self.fg_primary,
            &mut self.fg_secondary,
            &mut self.fg_muted,
            &mut self.shortcut_text,
        ] {
            repair(ink, &text_surfaces);
        }
        if let Some(background) = self.bg_selection.rgb() {
            repair(&mut self.selection_text, &[background]);
        }
        if let Some(background) = self.selection_bar_bg.rgb() {
            repair(&mut self.selection_bar_fg, &[background]);
        }
        self
    }
}

#[cfg(test)]
mod contrast_tests {
    use super::*;

    #[test]
    fn every_named_theme_keeps_text_and_selection_readable() {
        for name in ThemeName::all()
            .iter()
            .copied()
            .filter(|name| *name != ThemeName::System)
        {
            let p = Palette::<Color>::for_theme(name);
            let surfaces = [p.bg_primary, p.bg_secondary, p.transport_bg];
            for (role, ink) in [
                ("primary", p.fg_primary),
                ("secondary", p.fg_secondary),
                ("muted", p.fg_muted),
                ("accent", p.fg_accent),
                ("error", p.error),
                ("success", p.success),
                ("warning", p.warning),
            ] {
                for surface in surfaces {
                    let ratio = contrast(ink.rgb().unwrap(), surface.rgb().unwrap());
                    assert!(ratio >= 4.5, "{name:?} {role}: {ratio:.2}");
                }
            }
            for (ink, background) in [
                (p.selection_text, p.bg_selection),
                (p.selection_bar_fg, p.selection_bar_bg),
            ] {
                assert!(
                    contrast(ink.rgb().unwrap(), background.rgb().unwrap()) >= 4.5,
                    "{name:?} selection"
                );
            }
            for surface in [p.bg_primary, p.bg_secondary] {
                let background = mix(surface.rgb().unwrap(), p.fg_accent.rgb().unwrap(), 0.18);
                for ink in [p.fg_primary, p.fg_secondary, p.fg_muted] {
                    assert!(
                        contrast(ink.rgb().unwrap(), background) >= 4.5,
                        "{name:?} tinted row"
                    );
                }
            }
        }
    }

    #[test]
    fn system_palette_retains_the_terminal_defaults() {
        let p = Palette::<Color>::for_theme(ThemeName::System);
        assert_eq!(p.bg_primary, Color::Reset);
        assert_eq!(p.bg_secondary, Color::Reset);
        assert_eq!(p.fg_primary, Color::Reset);
    }
}

/// Track-card surface retains a visible distinction from its transport background.
pub fn track_card_background(highlight: Color, transport: Color, foreground: Color) -> Color {
    if highlight != transport {
        return highlight;
    }
    match (transport, foreground) {
        (Color::Rgb(r, g, b), Color::Rgb(fr, fg, fb)) => {
            let blend = |base: u8, text: u8| ((u16::from(base) * 7 + u16::from(text)) / 8) as u8;
            Color::Rgb(blend(r, fr), blend(g, fg), blend(b, fb))
        }
        _ => highlight,
    }
}
