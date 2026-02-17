//! Sort popup renderer (Ctrl+S).

use crate::app::state::{ColumnSortMode, SortPopupOption};
use crate::app::AppState;
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

/// Render the sort popup as an overlay.
pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let popup = match &state.sort_popup {
        Some(p) => p,
        None => return,
    };
    let t = theme();

    // Fixed-size popup: 38 wide, height based on option count
    let popup_height = (popup.options.len() as u16) + 4; // 2 border + 1 header + 1 footer
    let popup_width = 38u16;

    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width.min(area.width), popup_height.min(area.height));

    frame.render_widget(Clear, popup_area);

    let title = format!(" sort: {} ", popup.column_title);
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.fg_accent))
        .style(Style::default().bg(t.colors.bg_primary));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // Get current column state for display
    let (current_mode, ascending, artwork_visible, grouped_by_album) = state.browse_nav()
        .and_then(|nav| nav.columns.get(popup.column_idx))
        .map(|col| (col.sort_mode, col.sort_ascending, col.artwork_visible, col.grouped_by_album))
        .unwrap_or((ColumnSortMode::Default, true, false, false));

    // Build list items
    let items: Vec<ListItem> = popup.options.iter().enumerate().map(|(i, option)| {
        let is_selected = i == popup.selected_index;

        let (prefix, label) = match option {
            SortPopupOption::SortMode(mode) => {
                let radio = if *mode == current_mode { "\u{25cf}" } else { "\u{25cb}" };
                let name = match mode {
                    ColumnSortMode::Default => popup.default_label,
                    ColumnSortMode::ByArtist => "By artist",
                    ColumnSortMode::ByAlbum => "By album",
                    ColumnSortMode::ByTitle => "By title",
                    ColumnSortMode::ByDuration => "By duration",
                    ColumnSortMode::Shuffled => "Shuffled",
                };
                (radio.to_string(), name.to_string())
            }
            SortPopupOption::Direction => {
                let arrow = if ascending { "\u{2191}" } else { "\u{2193}" };
                let label = if ascending { "Ascending" } else { "Descending" };
                (format!(" {}", arrow), label.to_string())
            }
            SortPopupOption::Artwork => {
                let check = if artwork_visible { "\u{2611}" } else { "\u{2610}" };
                (check.to_string(), "Artwork".to_string())
            }
            SortPopupOption::GroupByAlbum => {
                let check = if grouped_by_album { "\u{2611}" } else { "\u{2610}" };
                (check.to_string(), "Group by album".to_string())
            }
        };

        let text = format!(" {} {}", prefix, label);
        let style = if is_selected {
            Style::default().bg(t.colors.bg_selection).fg(t.colors.fg_primary)
        } else {
            Style::default().fg(t.colors.fg_primary)
        };
        ListItem::new(Line::from(Span::styled(text, style)))
    }).collect();

    if inner.height > 1 {
        // Render options
        let list_area = Rect::new(inner.x, inner.y, inner.width, inner.height.saturating_sub(1));
        let list = List::new(items);
        frame.render_widget(list, list_area);

        // Footer: [Esc] Close
        let footer_area = Rect::new(inner.x, inner.y + inner.height.saturating_sub(1), inner.width, 1);
        let footer = Paragraph::new(Line::from(Span::styled(
            " [Esc] Close",
            Style::default().fg(t.colors.fg_muted),
        )));
        frame.render_widget(footer, footer_area);
    }
}
