//! Scrollbar widget for long lists.
//!
//! Renders a visible scrollbar with a track and thumb.

use crate::ui::theme::theme;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Render a scrollbar on the right border of a bordered area.
///
/// Uses `█` for the thumb and `│` for the track, overlaid on the right border.
/// When `border_override` is `Some(color)`, the track uses that color instead of the
/// theme's default border color.
pub fn render_scrollbar(
    frame: &mut Frame,
    col_area: Rect,
    total_items: usize,
    visible_items: usize,
    scroll_offset: usize,
    border_override: Option<Color>,
) {
    let t = theme();

    // Scrollbar occupies the right border column, between top and bottom borders
    let track_height = col_area.height.saturating_sub(2) as usize; // exclude top/bottom border
    if track_height == 0 || total_items == 0 || visible_items >= total_items {
        return;
    }

    let (thumb_size, thumb_pos) =
        calc_thumb(total_items, visible_items, scroll_offset, track_height);

    let bar_x = col_area.x + col_area.width - 1; // Right border column
    let bar_y_start = col_area.y + 1; // Skip top border

    let thumb_style = Style::default().fg(t.colors.fg_secondary);
    let border_color = border_override.unwrap_or(t.colors.border);
    let track_style = Style::default().fg(border_color);

    for row in 0..track_height {
        let y = bar_y_start + row as u16;
        if row >= thumb_pos && row < thumb_pos + thumb_size {
            frame.render_widget(
                Paragraph::new("█").style(thumb_style),
                Rect::new(bar_x, y, 1, 1),
            );
        } else {
            frame.render_widget(
                Paragraph::new("│").style(track_style),
                Rect::new(bar_x, y, 1, 1),
            );
        }
    }
}

/// Render a scrollbar for a borderless area (e.g., popup content areas).
///
/// Same layout but uses full area height without border offsets.
pub fn render_scrollbar_borderless(
    frame: &mut Frame,
    area: Rect,
    total_items: usize,
    visible_items: usize,
    scroll_offset: usize,
) {
    let t = theme();

    let track_height = area.height as usize;
    if track_height == 0 || total_items == 0 || visible_items >= total_items {
        return;
    }

    let (thumb_size, thumb_pos) =
        calc_thumb(total_items, visible_items, scroll_offset, track_height);

    let bar_x = area.x + area.width.saturating_sub(1);
    let bar_y_start = area.y;

    let thumb_style = Style::default().fg(t.colors.fg_secondary);
    let track_style = Style::default().fg(t.colors.border);

    for row in 0..track_height {
        let y = bar_y_start + row as u16;
        if row >= thumb_pos && row < thumb_pos + thumb_size {
            frame.render_widget(
                Paragraph::new("█").style(thumb_style),
                Rect::new(bar_x, y, 1, 1),
            );
        } else {
            frame.render_widget(
                Paragraph::new("│").style(track_style),
                Rect::new(bar_x, y, 1, 1),
            );
        }
    }
}

pub use crate::app::scrollbar::{calc_thumb, scroll_offset_from_y};
