//! Reusable GUI widgets — Miller column, track list, transport bar,
//! now-playing bar, visualizer canvases, marquee.

pub mod context_menu;
pub mod menu_bar;
pub mod miller_column;
pub mod spectrogram_canvas;
pub mod spectrum_canvas;
pub mod tab_strip;
pub mod transport_bar;
pub mod waveform_canvas;

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
