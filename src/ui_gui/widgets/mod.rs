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
    // Single pass: borrow until we hit the first codepoint that
    // needs to change, then upgrade to an owned String seeded with
    // the prefix we've already validated.
    let mut owned: Option<String> = None;
    for (idx, c) in input.char_indices() {
        let strip = needs_strip(c);
        let sub = needs_substitute(c);
        if !strip && sub.is_none() {
            if let Some(buf) = owned.as_mut() {
                buf.push(c);
            }
            continue;
        }
        let buf = owned.get_or_insert_with(|| {
            let mut b = String::with_capacity(input.len());
            b.push_str(&input[..idx]);
            b
        });
        if let Some(replacement) = sub {
            buf.push(replacement);
        }
        // strip-only: skip the char.
    }
    match owned {
        Some(s) => Cow::Owned(s),
        None => Cow::Borrowed(input),
    }
}

fn needs_strip(c: char) -> bool {
    let cp = c as u32;
    matches!(cp,
        0xFE00..=0xFE0F | 0x200D | 0xE0020..=0xE007F
    )
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
