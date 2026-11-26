//! Reusable UI widgets.

pub mod logo;
pub mod scrollbar;
pub mod selectable_list;
pub mod track_list;
pub mod transport;

pub use scrollbar::{render_scrollbar, render_scrollbar_borderless};
pub use selectable_list::{DisplayItem, calculate_scroll_offset, render_selectable_list};

pub(crate) mod album_page;
pub mod column_header;
pub(crate) mod text_input;

/// Register the close target of the topmost terminal overlay.
pub(crate) fn register_popup_close(
    state: &crate::ui::RenderState<'_>,
    area: ratatui::layout::Rect,
) {
    state.hit_regions.borrow_mut().popup_outer = Some(area);
    if area.width >= 4 && area.height > 0 {
        state.hit_regions.borrow_mut().popup_close =
            Some(ratatui::layout::Rect::new(area.right() - 3, area.y, 1, 1));
    }
}

/// Typed terminal command-bar composition.
pub mod control_bar;

/// Place the terminal caret after visible input, keeping native IME geometry inside its field.
pub(crate) fn text_cursor(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, prefix: &str) {
    use unicode_width::UnicodeWidthStr;
    if area.width == 0 || area.height == 0 {
        return;
    }
    let mut x = 0usize;
    let mut y = 0usize;
    for (index, line) in prefix.split('\n').enumerate() {
        if index > 0 {
            y += 1;
            x = 0;
        }
        let width = UnicodeWidthStr::width(line);
        x += width;
        y += x / usize::from(area.width);
        x %= usize::from(area.width);
    }
    frame.set_cursor_position((area.x + x as u16, area.y + (y as u16).min(area.height - 1)));
}
