//! Scrollbar widget for long lists.
//!
//! Renders a visible scrollbar with a track and thumb.

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use crate::ui::theme::theme;

/// Render a scrollbar on the right border of a bordered area.
///
/// Uses `█` for the thumb and `▕` for the track, overlaid on the right border.
pub fn render_scrollbar(
    frame: &mut Frame,
    col_area: Rect,
    total_items: usize,
    visible_items: usize,
    scroll_offset: usize,
) {
    let t = theme();

    // Scrollbar occupies the right border column, between top and bottom borders
    let track_height = col_area.height.saturating_sub(2) as usize; // exclude top/bottom border
    if track_height == 0 || total_items == 0 || visible_items >= total_items {
        return;
    }

    let (thumb_size, thumb_pos) = calc_thumb(total_items, visible_items, scroll_offset, track_height);

    let bar_x = col_area.x + col_area.width - 1; // Right border column
    let bar_y_start = col_area.y + 1; // Skip top border

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
                Paragraph::new("▕").style(track_style),
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

    let (thumb_size, thumb_pos) = calc_thumb(total_items, visible_items, scroll_offset, track_height);

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
                Paragraph::new("▕").style(track_style),
                Rect::new(bar_x, y, 1, 1),
            );
        }
    }
}

/// Calculate thumb size and position.
pub fn calc_thumb(total_items: usize, visible_items: usize, scroll_offset: usize, track_height: usize) -> (usize, usize) {
    let thumb_size = ((visible_items as f64 / total_items as f64) * track_height as f64)
        .ceil()
        .max(1.0) as usize;
    let max_scroll = total_items.saturating_sub(visible_items);
    let thumb_pos = if max_scroll > 0 {
        ((scroll_offset as f64 / max_scroll as f64) * (track_height - thumb_size) as f64)
            .round() as usize
    } else {
        0
    };
    (thumb_size, thumb_pos)
}

/// Convert a mouse Y position into a scroll offset by reversing the thumb position math.
pub fn scroll_offset_from_y(
    mouse_y: u16,
    track_y_start: u16,
    track_height: u16,
    total_items: usize,
    visible_items: usize,
    grab_offset: u16,
) -> usize {
    if track_height == 0 || total_items == 0 || visible_items >= total_items {
        return 0;
    }

    let th = track_height as usize;
    let (thumb_size, _) = calc_thumb(total_items, visible_items, 0, th);
    let max_thumb_pos = th.saturating_sub(thumb_size);
    if max_thumb_pos == 0 {
        return 0;
    }

    // Where the top of the thumb should be based on mouse position
    let thumb_top = (mouse_y.saturating_sub(track_y_start).saturating_sub(grab_offset)) as usize;
    let clamped = thumb_top.min(max_thumb_pos);

    let max_scroll = total_items.saturating_sub(visible_items);
    // Reverse: thumb_pos = (scroll / max_scroll) * max_thumb_pos
    //       => scroll = thumb_pos * max_scroll / max_thumb_pos
    let offset = (clamped as f64 / max_thumb_pos as f64 * max_scroll as f64).round() as usize;
    offset.min(max_scroll)
}
