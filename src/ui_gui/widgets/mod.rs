//! Reusable GUI widgets — Miller column, track list, transport bar,
//! now-playing bar, visualizer canvases, marquee.

pub mod context_menu;
pub mod menu_bar;
pub mod miller_column;
pub mod vectorscope_canvas;
pub mod spectrogram_canvas;
pub mod spectrum_canvas;
pub mod tab_strip;
pub mod transport_bar;
pub mod waveform_canvas;

/// Drop-in replacement for `iced::widget::text` that always uses
/// `Shaping::Advanced`.
///
/// iced 0.13 defaults `Text` widgets to `Shaping::Basic`, which —
/// per the iced docs — performs "no shaping and no font fallback" and
/// "will not display complex scripts properly nor try to find missing
/// glyphs in your system fonts". With our default font (.SF NS / SF
/// Pro on macOS, Segoe UI on Windows) being Latin-only, that means
/// every CJK title, emoji, and dingbat in user-sourced metadata
/// renders as the primary font's `.notdef` glyph (the boxy tofu the
/// user kept reporting). Advanced shaping is what triggers the whole
/// cosmic-text fallback chain we wired up — without it, the chain
/// never runs.
///
/// Every `text(...)` call in `src/ui_gui/` should resolve to this
/// wrapper, not `iced::widget::text`. Files do that by importing
/// `text` from `crate::ui_gui::widgets` (this module) instead of
/// from `iced::widget::*`.
pub fn text<'a, Theme, Renderer>(
    content: impl iced::widget::text::IntoFragment<'a>,
) -> iced::widget::Text<'a, Theme, Renderer>
where
    Theme: iced::widget::text::Catalog + 'a,
    Renderer: iced::advanced::text::Renderer,
{
    iced::widget::text(content).shaping(iced::widget::text::Shaping::Advanced)
}

/// Sanitize user-facing strings so iced/cosmic-text/swash can render
/// them without falling through to the LastResort font (the boxy
/// "tofu" the user keeps reporting on playlist titles).
///
/// Two kinds of fix happen here:
///
/// **1. Strip codepoints that derail the shaper.**
/// - `U+FE00..=U+FE0F` variation selectors (esp. VS-16, the emoji
///   presentation hint). VS-16 forces the shaper to find an emoji
///   font; Apple Color Emoji is sbix-format, which swash can't
///   render, so without this strip we get tofu.
/// - `U+200D` zero-width joiner — only meaningful inside emoji
///   sequences, which we can't render anyway.
/// - `U+E0020..=U+E007F` tag characters (flag sequences).
///
/// **2. Substitute codepoints that *only* live in unrenderable
/// fonts.**
/// - `U+2764` ❤ HEAVY BLACK HEART → `U+2665` ♥ BLACK HEART SUIT.
///   U+2764 is in cosmic-text's macOS common_fallback only via
///   `Apple Color Emoji` (sbix; swash can't draw it). U+2665 is in
///   `.SF NS` itself, so it renders cleanly as part of the system-
///   font run with no fallback round-trip.
///
/// The borrow path is hot (every Plex title every render) so we keep
/// it allocation-free for ASCII / Latin titles by returning a `Cow`
/// that only allocates when a substitution actually happens.
pub fn safe_text(input: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    // First pass: shared sanitizer (control / format / private-use /
    // variation selectors). Common case is Borrowed (no allocation).
    let stripped = crate::util::sanitize_display_text(input);
    // Second pass: GUI-specific font substitutions (e.g. U+2764
    // HEAVY BLACK HEART → U+2665 BLACK HEART SUIT, because cosmic-
    // text can't render U+2764 from Apple Color Emoji).
    let mut owned: Option<String> = None;
    for (idx, c) in stripped.char_indices() {
        if let Some(replacement) = needs_substitute(c) {
            let buf = owned.get_or_insert_with(|| {
                let mut b = String::with_capacity(stripped.len());
                b.push_str(&stripped[..idx]);
                b
            });
            buf.push(replacement);
            continue;
        }
        if let Some(buf) = owned.as_mut() {
            buf.push(c);
        }
    }
    match (owned, stripped) {
        (Some(s), _) => Cow::Owned(s),
        (None, Cow::Owned(s)) => Cow::Owned(s),
        (None, Cow::Borrowed(s)) => {
            // Map borrow back to the original input slice (same bytes).
            // sanitize_display_text returns Borrowed only when nothing
            // changed, so input == s.
            let _ = s;
            Cow::Borrowed(input)
        }
    }
}

fn needs_substitute(c: char) -> Option<char> {
    match c {
        // HEAVY BLACK HEART → BLACK HEART SUIT (renderable in SF NS).
        '\u{2764}' => Some('\u{2665}'),
        _ => None,
    }
}

/// Vertical scrollbar tuned to feel like a normal always-shown macOS
/// scrollbar: ~16 px wide track, scroller fills almost the full width.
/// Pair with `chunky_scrollable_style` so the scroller renders in a
/// high-contrast colour against the rail — iced's defaults pick two
/// neighbouring `background.weak`/`background.strong` swatches that
/// can read as a single muted blob in dark themes.
/// Use as:
///   `scrollable(content)
///       .direction(fat_vertical_scrollbar())
///       .style(chunky_scrollable_style)`
pub fn fat_vertical_scrollbar() -> iced::widget::scrollable::Direction {
    use iced::widget::scrollable::{Direction, Scrollbar};
    // Width / scroller_width follow normal desktop scrollbar
    // proportions. The "tall enough to grab" feel comes from a
    // local patch in `vendor/iced_widget/src/scrollable.rs` that
    // bumps the minimum scroller HEIGHT from 2 px to 40 px so the
    // dragger doesn't collapse on libraries with 10k+ rows.
    Direction::Vertical(
        Scrollbar::default()
            .width(16)
            .scroller_width(14)
            .margin(1),
    )
}

/// Horizontal scrollbar tuned to feel like a normal always-shown
/// scrollbar — used by the Browse view's scrolling Miller-column
/// mode so the user can pan through nav columns wider than the
/// viewport. Pair with `chunky_scrollable_style`.
pub fn fat_horizontal_scrollbar() -> iced::widget::scrollable::Direction {
    use iced::widget::scrollable::{Direction, Scrollbar};
    Direction::Horizontal(
        Scrollbar::default()
            .width(16)
            .scroller_width(14)
            .margin(1),
    )
}

/// Custom scrollable style: keep the rail subtle but render the
/// scroller in a strong, plainly-visible colour so the dragger
/// reads as a chunky bar rather than a thin line. Matches the
/// existing accent + hover/drag colour conventions of the rest of
/// the app (selection highlights, primary buttons).
pub fn chunky_scrollable_style(
    theme: &iced::Theme,
    status: iced::widget::scrollable::Status,
) -> iced::widget::scrollable::Style {
    use iced::border;
    use iced::widget::scrollable::{Rail, Scroller, Status, Style};
    use iced::widget::container;

    let p = theme.extended_palette();
    let rail = Rail {
        background: Some(p.background.weak.color.into()),
        border: border::rounded(2),
        scroller: Scroller {
            color: p.background.strong.color,
            border: border::rounded(2),
        },
    };
    let hot = |base_color: iced::Color| -> Rail {
        Rail {
            scroller: Scroller {
                color: base_color,
                ..rail.scroller
            },
            ..rail
        }
    };
    let active = Rail {
        scroller: Scroller {
            // Pull the resting scroller colour up to a strong-but-not-
            // accent grey so it shows clearly even when the user
            // isn't hovering the rail.
            color: p.primary.weak.color,
            ..rail.scroller
        },
        ..rail
    };
    match status {
        Status::Active => Style {
            container: container::Style::default(),
            vertical_rail: active,
            horizontal_rail: active,
            gap: None,
        },
        Status::Hovered {
            is_horizontal_scrollbar_hovered,
            is_vertical_scrollbar_hovered,
        } => Style {
            container: container::Style::default(),
            vertical_rail: if is_vertical_scrollbar_hovered { hot(p.primary.strong.color) } else { active },
            horizontal_rail: if is_horizontal_scrollbar_hovered { hot(p.primary.strong.color) } else { active },
            gap: None,
        },
        Status::Dragged {
            is_horizontal_scrollbar_dragged,
            is_vertical_scrollbar_dragged,
        } => Style {
            container: container::Style::default(),
            vertical_rail: if is_vertical_scrollbar_dragged { hot(p.primary.base.color) } else { active },
            horizontal_rail: if is_horizontal_scrollbar_dragged { hot(p.primary.base.color) } else { active },
            gap: None,
        },
    }
}
