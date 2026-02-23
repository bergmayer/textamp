//! Generic selectable list widget.
//!
//! Provides a reusable component for rendering scrollable, selectable lists
//! with consistent styling and scroll offset calculation.

use crate::ui::theme::theme;
use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem, Paragraph};
use std::borrow::Cow;

/// Trait for items that can be displayed in a selectable list.
pub trait DisplayItem {
    /// Get the display text for this item.
    fn display_text(&self) -> Cow<'_, str>;
}

/// Calculate scroll offset to keep selected item visible in the viewport.
pub fn calculate_scroll_offset(selected: usize, viewport_height: usize, total_items: usize) -> usize {
    if total_items == 0 || viewport_height == 0 {
        return 0;
    }

    let half_height = viewport_height / 2;

    if selected < half_height {
        0
    } else if selected + half_height >= total_items {
        total_items.saturating_sub(viewport_height)
    } else {
        selected.saturating_sub(half_height)
    }
}

/// Render a selectable list with scroll support and position footer.
///
/// # Arguments
/// * `frame` - The frame to render to
/// * `items` - The items to display (must implement DisplayItem)
/// * `selected_idx` - The currently selected item index
/// * `area` - The area to render in
/// * `empty_message` - Message to show when list is empty
pub fn render_selectable_list<T: DisplayItem>(
    frame: &mut Frame,
    items: &[T],
    selected_idx: usize,
    area: Rect,
    empty_message: &str,
) {
    let t = theme();

    if items.is_empty() {
        let empty = Paragraph::new(empty_message)
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, area);
        return;
    }

    let visible_height = area.height as usize;
    let total = items.len();
    let scroll_offset = calculate_scroll_offset(selected_idx, visible_height, total);

    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(i, item)| {
            let style = if i == selected_idx {
                Style::default()
                    .fg(t.colors.selection_text)
                    .bg(t.colors.selection_bar_bg)
            } else {
                Style::default().fg(t.colors.fg_primary)
            };
            ListItem::new(item.display_text()).style(style)
        })
        .collect();

    let list = List::new(list_items);
    frame.render_widget(list, area);

    // Scrollbar for long lists
    if total > visible_height {
        super::render_scrollbar_borderless(frame, area, total, visible_height, scroll_offset);
    }

    // Render position footer
    render_position_footer(frame, selected_idx, total, area);
}

/// Render a position footer showing "X / Y" in the bottom right.
pub fn render_position_footer(frame: &mut Frame, selected: usize, total: usize, area: Rect) {
    let t = theme();
    let footer = format!(" {} / {} ", selected + 1, total);
    let footer_area = Rect::new(
        area.x + area.width.saturating_sub(footer.len() as u16 + 1),
        area.y + area.height.saturating_sub(1),
        footer.len() as u16,
        1,
    );
    frame.render_widget(
        Paragraph::new(footer).style(Style::default().fg(t.colors.fg_muted)),
        footer_area,
    );
}

// Implement DisplayItem for common types

impl DisplayItem for crate::plex::models::Artist {
    fn display_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.title)
    }
}

impl DisplayItem for crate::plex::models::Album {
    fn display_text(&self) -> Cow<'_, str> {
        let year = self.year.map(|y| format!(" ({})", y)).unwrap_or_default();
        Cow::Owned(format!("{} - {}{}", self.artist_name(), self.title, year))
    }
}

impl DisplayItem for crate::plex::models::Track {
    fn display_text(&self) -> Cow<'_, str> {
        Cow::Owned(format!("{} - {}", self.track_artist(), self.title))
    }
}

impl DisplayItem for crate::plex::models::Playlist {
    fn display_text(&self) -> Cow<'_, str> {
        Cow::Owned(format!("{} ({} tracks)", self.title, self.track_count()))
    }
}

impl DisplayItem for crate::plex::models::Genre {
    fn display_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.title)
    }
}

impl DisplayItem for crate::plex::models::Station {
    fn display_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.title)
    }
}

// Simple wrapper for string slices
pub struct StringItem<'a>(pub &'a str);

impl<'a> DisplayItem for StringItem<'a> {
    fn display_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.0)
    }
}

// Wrapper for (display_text, key) tuples
pub struct KeyedItem<'a> {
    pub text: &'a str,
    pub key: &'a str,
}

impl<'a> DisplayItem for KeyedItem<'a> {
    fn display_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.text)
    }
}
