//! Translate every Textamp palette into Kit's shared semantic control tokens.
use crate::app::theme::ThemeName;
use gpui_kit::component::{Theme, ThemeColor, ThemeMode};
use gpui_kit::{App, Hsla, Window, black, rgb, white};
fn color(value: crate::app::theme_palette::Color) -> Hsla {
    use crate::app::theme_palette::Color;
    match value {
        Color::Rgb(r, g, b) => {
            rgb((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)).into()
        }
        Color::Black => black(),
        Color::White => white(),
        _ => rgb(0x808080).into(),
    }
}
fn luminance(color: Hsla) -> f32 {
    let c = color.to_rgb();
    let linear = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
}
fn on_accent(accent: Hsla) -> Hsla {
    if luminance(accent) > 0.179 {
        black()
    } else {
        white()
    }
}

fn readable_ink(ink: Hsla, surfaces: &[Hsla]) -> Hsla {
    let readable = |candidate: Hsla| {
        surfaces.iter().all(|background| {
            let background_light = luminance(*background);
            let ink_light = luminance(background.blend(candidate));
            (ink_light.max(background_light) + 0.05) / (ink_light.min(background_light) + 0.05)
                >= 4.5
        })
    };
    if readable(ink) {
        return ink;
    }
    if ink.a < 1. {
        for step in 1..=20 {
            let candidate = Hsla {
                a: ink.a + (1. - ink.a) * step as f32 / 20.,
                ..ink
            };
            if readable(candidate) {
                return candidate;
            }
        }
    }
    let channels = |value: Hsla| {
        let c = value.to_rgb();
        [c.r, c.g, c.b].map(|channel| (channel * 255.).round() as u8)
    };
    let backgrounds = surfaces.iter().copied().map(channels).collect::<Vec<_>>();
    let [r, g, b] = crate::app::theme_palette::readable_rgb(channels(ink), &backgrounds, 4.5);
    rgb((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)).into()
}
pub(super) fn selection_foreground(foreground: Hsla, background: Hsla) -> Hsla {
    let a = luminance(foreground);
    let b = luminance(background);
    if (a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5 {
        foreground
    } else {
        on_accent(background)
    }
}
/// Kit's default buttons and list hover states retain their normal foreground.
fn interaction_background(background: Hsla, foreground: Hsla, secondary: Hsla, base: Hsla) -> Hsla {
    let readable = |background| {
        [foreground, secondary]
            .into_iter()
            .all(|ink| selection_foreground(ink, background) == ink)
    };
    if readable(background) {
        background
    } else {
        [0.12, 0.08, 0.04, 0.]
            .into_iter()
            .map(|opacity| base.blend(foreground.opacity(opacity)))
            .find(|candidate| readable(*candidate))
            .unwrap_or(base)
    }
}
pub(super) fn colors(name: ThemeName) -> ThemeColor {
    if name == ThemeName::System {
        return system_colors(false);
    }
    let p = crate::app::theme_palette::Palette::<crate::app::theme_palette::Color>::for_theme(name);
    let light = matches!(
        name,
        ThemeName::SolarizedLight | ThemeName::Platinum | ThemeName::BlackAndWhite
    );
    let mut c = if light {
        *ThemeColor::light()
    } else {
        *ThemeColor::dark()
    };
    c.background = color(p.bg_primary);
    c.foreground = color(p.fg_primary);
    c.muted = color(p.bg_secondary);
    c.muted_foreground = color(p.fg_secondary);
    c.border = color(p.border);
    c.input = c.border;
    c.ring = color(p.border_focused);
    c.primary = color(p.fg_accent);
    c.primary_foreground = on_accent(c.primary);
    c.primary_hover = c.primary;
    c.primary_active = c.primary;
    c.accent = interaction_background(
        color(p.bg_highlight),
        c.foreground,
        c.muted_foreground,
        c.muted,
    );
    c.accent_foreground = selection_foreground(c.foreground, c.accent);
    c.secondary = c.muted;
    c.secondary_foreground = c.foreground;
    c.secondary_hover = interaction_background(c.accent, c.foreground, c.muted_foreground, c.muted);
    c.secondary_active = interaction_background(
        color(p.bg_selection),
        c.foreground,
        c.muted_foreground,
        c.muted,
    );
    c.button = c.muted;
    c.button_foreground = c.foreground;
    c.button_hover = c.secondary_hover;
    c.button_active = c.secondary_active;
    if name == ThemeName::BlackAndWhite {
        c.button_hover = c.button;
        c.button_active = c.button;
    }
    c.button_primary = c.primary;
    c.button_primary_foreground = c.primary_foreground;
    c.button_primary_hover = c.primary_hover;
    c.button_primary_active = c.primary_active;
    c.popover = c.muted;
    c.popover_foreground = c.foreground;
    c.sidebar = c.muted;
    c.sidebar_foreground = c.foreground;
    c.sidebar_border = c.border;
    c.sidebar_accent = c.primary;
    c.sidebar_accent_foreground = c.primary_foreground;
    c.sidebar_primary = c.primary;
    c.sidebar_primary_foreground = c.primary_foreground;
    c.list_active = color(p.bg_selection);
    c.list_active_border = c.primary;
    c.list_hover = interaction_background(c.accent, c.foreground, c.muted_foreground, c.background);
    c.list_head = c.muted;
    c.list_even = c.background;
    c.selection = c.primary.opacity(0.3);
    c.title_bar = color(p.transport_bg);
    c.title_bar_border = c.border;
    c.tab = c.background;
    c.tab_bar = c.muted;
    c.tab_bar_segmented = c.muted;
    c.tab_foreground = c.muted_foreground;
    c.tab_active = c.accent;
    c.tab_active_foreground = c.accent_foreground;
    c.scrollbar = c.background;
    c.scrollbar_thumb = color(p.fg_muted);
    c.scrollbar_thumb_hover = c.primary;
    c.danger = color(p.error);
    c.success = color(p.success);
    c.warning = color(p.warning);
    c
}
/// Track information uses the same semantic surface as the terminal card.
pub(super) fn track_card(name: ThemeName, cx: &App) -> Hsla {
    if name == ThemeName::BlackAndWhite {
        use gpui_kit::component::ActiveTheme;
        return cx.theme().background;
    }
    if name == ThemeName::System {
        use gpui_kit::component::ActiveTheme;
        return cx
            .theme()
            .title_bar
            .blend(cx.theme().foreground.opacity(0.08));
    }
    let palette =
        crate::app::theme_palette::Palette::<crate::app::theme_palette::Color>::for_theme(name);
    color(crate::app::theme_palette::track_card_background(
        palette.bg_highlight,
        palette.transport_bg,
        palette.fg_primary,
    ))
}

fn system_colors(dark: bool) -> ThemeColor {
    let c = if dark {
        *ThemeColor::dark()
    } else {
        *ThemeColor::light()
    };
    #[cfg(all(target_os = "macos", not(test)))]
    let c = {
        let mut c = c;
        unsafe extern "C" {
            fn textamp_system_theme_colors(rgba: *mut f32) -> u32;
        }
        let mut rgba = [0_f32; 56];
        // # Safety
        // AppKit runs on the app thread. The function writes at most 14 RGBA
        // colors into this 56-float buffer and retains no pointer.
        if unsafe { textamp_system_theme_colors(rgba.as_mut_ptr()) } == 14 {
            let color = |index: usize| {
                gpui_kit::Rgba {
                    r: rgba[index * 4],
                    g: rgba[index * 4 + 1],
                    b: rgba[index * 4 + 2],
                    a: rgba[index * 4 + 3],
                }
                .into()
            };
            c.background = color(2);
            c.foreground = color(3);
            c.muted = color(1);
            c.muted_foreground = color(4);
            c.border = color(5);
            c.input = c.border;
            c.ring = color(10);
            c.primary = color(6);
            c.primary_foreground = on_accent(c.primary);
            c.primary_hover = c.primary;
            c.primary_active = c.primary;
            c.accent = color(9);
            c.accent_foreground = c.foreground;
            c.secondary = c.muted;
            c.secondary_foreground = c.foreground;
            c.secondary_hover = c.accent;
            c.secondary_active = c.accent;
            c.button = c.muted;
            c.button_foreground = c.foreground;
            c.button_hover = c.accent;
            c.button_active = c.accent;
            c.button_primary = c.primary;
            c.button_primary_foreground = c.primary_foreground;
            c.button_primary_hover = c.primary;
            c.button_primary_active = c.primary;
            c.popover = c.muted;
            c.popover_foreground = c.foreground;
            c.sidebar = color(0);
            c.sidebar_foreground = c.foreground;
            c.sidebar_border = c.border;
            c.sidebar_accent = color(7);
            c.sidebar_accent_foreground = color(8);
            c.sidebar_primary = c.primary;
            c.sidebar_primary_foreground = c.primary_foreground;
            c.list_active = color(7);
            c.list_active_border = c.primary;
            c.list_hover = c.accent;
            c.list_head = c.muted;
            c.list_even = c.background;
            c.selection = color(7);
            c.title_bar = color(0);
            c.title_bar_border = c.border;
            c.tab = c.background;
            c.tab_bar = c.muted;
            c.tab_bar_segmented = c.muted;
            c.tab_foreground = c.muted_foreground;
            c.tab_active = c.accent;
            c.tab_active_foreground = c.foreground;
            c.scrollbar = c.background;
            c.scrollbar_thumb = c.muted_foreground;
            c.scrollbar_thumb_hover = c.primary;
            c.danger = color(11);
            c.success = color(12);
            c.warning = color(13);
        }
        c
    };
    let mut c = c;
    let surfaces = [c.background, c.muted, c.sidebar];
    c.foreground = readable_ink(c.foreground, &surfaces);
    c.muted_foreground = readable_ink(c.muted_foreground, &surfaces);
    c.secondary_foreground = c.foreground;
    c.button_foreground = c.foreground;
    c.popover_foreground = c.foreground;
    c.sidebar_foreground = c.foreground;
    c.accent = interaction_background(c.accent, c.foreground, c.muted_foreground, c.muted);
    c.accent_foreground = selection_foreground(c.foreground, c.accent);
    c.secondary_hover = c.accent;
    c.secondary_active = c.accent;
    c.button_hover = c.accent;
    c.button_active = c.accent;
    c.list_hover =
        interaction_background(c.list_hover, c.foreground, c.muted_foreground, c.background);
    c.tab_foreground = c.muted_foreground;
    c.tab_active = c.accent;
    c.tab_active_foreground = c.accent_foreground;
    c
}

pub(super) fn apply(name: ThemeName, window: Option<&mut Window>, cx: &mut App) {
    if name == ThemeName::System {
        Theme::sync_system_appearance(window, cx);
        let colors = system_colors(Theme::global(cx).is_dark());
        Theme::update(cx, |theme| {
            theme.colors = colors;
            #[cfg(target_os = "macos")]
            {
                theme.font_family = ".SystemUIFont".into();
            }
            theme.list.active_highlight = true;
        });
        #[cfg(all(target_os = "macos", not(test)))]
        super::icon::apply(name, cx);
        return;
    }
    #[cfg(all(target_os = "macos", not(test)))]
    super::icon::apply(name, cx);
    let light = matches!(
        name,
        ThemeName::SolarizedLight | ThemeName::Platinum | ThemeName::BlackAndWhite
    );
    Theme::change(
        if light {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        },
        window,
        cx,
    );
    Theme::update(cx, |theme| {
        theme.colors = colors(name);
        #[cfg(target_os = "macos")]
        {
            theme.font_family = ".SystemUIFont".into();
        }
        theme.list.active_highlight = true;
    });
}
#[cfg(test)]
mod tests {
    use super::{ThemeName, color, colors, luminance};
    #[test]
    fn every_palette_maps_to_kit_with_readable_sidebar_selection() {
        for name in ThemeName::all() {
            let c = colors(*name);
            if *name != ThemeName::System {
                assert_eq!(
                    c.background,
                    color(crate::app::theme_palette::Palette::<crate::app::theme_palette::Color>::for_theme(*name).bg_primary)
                );
            }
            let a = luminance(c.sidebar_accent);
            let b = luminance(c.sidebar_accent_foreground);
            assert!((a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5, "{name:?}");
            assert_ne!(c.sidebar, c.sidebar_accent, "{name:?}");
            for (background, foreground) in [
                (c.accent, c.accent_foreground),
                (c.primary, c.primary_foreground),
                (c.tab_active, c.tab_active_foreground),
                (c.list_hover, c.foreground),
                (c.button_hover, c.button_foreground),
                (c.button_active, c.button_foreground),
                (c.background, c.foreground),
                (c.muted, c.muted_foreground),
                (c.list_hover, c.muted_foreground),
                (c.button_hover, c.muted_foreground),
                (c.button_active, c.muted_foreground),
            ] {
                let a = luminance(background);
                let b = luminance(foreground);
                assert!(
                    (a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5,
                    "{name:?}: {background:?}/{foreground:?}"
                );
            }
            if *name == ThemeName::BlackAndWhite {
                assert_eq!(c.button_hover, c.background);
                assert_eq!(c.button_active, c.background);
                assert_ne!(c.button_foreground, c.button_hover);
            }
            if *name != ThemeName::System {
                assert_eq!(c.popover, c.muted);
            }
        }
    }
}

/// Native System roles supplied to the embedded terminal adapter.
#[cfg(target_os = "macos")]
pub(super) fn terminal_palette(cx: &App) -> crate::app::theme_palette::Palette<[u8; 3]> {
    use gpui_kit::component::ActiveTheme;
    let c = cx.theme();
    let rgb = |color: Hsla| {
        let background = c.background.to_rgb();
        let color = color.to_rgb();
        let channels = [color.r, color.g, color.b];
        let background = [background.r, background.g, background.b];
        std::array::from_fn(|index| {
            ((channels[index] * color.a + background[index] * (1. - color.a)) * 255.).round() as u8
        })
    };
    crate::app::theme_palette::Palette {
        bg_primary: rgb(c.background),
        bg_secondary: rgb(c.sidebar),
        bg_highlight: rgb(c.accent),
        bg_selection: rgb(c.list_active),
        fg_primary: rgb(c.foreground),
        fg_secondary: rgb(c.muted_foreground),
        fg_muted: rgb(c.muted_foreground),
        fg_accent: rgb(c.primary),
        fg_accent_dim: rgb(c.muted_foreground),
        border: rgb(c.border),
        border_focused: rgb(c.primary),
        title_focused: rgb(c.foreground),
        error: rgb(c.danger),
        success: rgb(c.success),
        warning: rgb(c.warning),
        selection_bar_bg: rgb(c.list_active),
        selection_bar_fg: rgb(c.sidebar_accent_foreground),
        selection_text: rgb(c.sidebar_accent_foreground),
        transport_bg: rgb(c.title_bar),
        shortcut_key: rgb(c.primary),
        shortcut_text: rgb(c.muted_foreground),
    }
}
