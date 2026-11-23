//! Sort popup renderer (Ctrl+S).

use crate::ui::RenderState as AppState;
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

/// Render the sort popup as an overlay.
pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let popup = match state.popups.sort() {
        Some(p) => p,
        None => return,
    };
    let t = theme();

    // Fixed-size popup: 38 wide, height based on option count
    let popup_height = (popup.options.len() as u16) + 4; // 2 border + 1 header + 1 footer
    let popup_width = 38u16;

    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(
        x,
        y,
        popup_width.min(area.width),
        popup_height.min(area.height),
    );

    frame.render_widget(Clear, popup_area);
    crate::ui::widgets::register_popup_close(state, popup_area);

    // Register hit regions
    {
        let mut hr = state.hit_regions.borrow_mut();
        let block_tmp = Block::default().borders(Borders::ALL);
        let inner_tmp = block_tmp.inner(popup_area);
        hr.sort_popup = Some(crate::app::presentation::SortPopupRegions {
            outer: popup_area,
            inner: inner_tmp,
            option_count: popup.options.len(),
        });
    }

    let title = format!(
        " {} ",
        crate::util::truncate_middle(
            &popup.column_title,
            popup_area.width.saturating_sub(4) as usize
        )
    );
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.fg_accent))
        .style(Style::default().bg(t.colors.bg_primary));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let items: Vec<ListItem> = crate::app::view_options::choices(state, popup)
        .iter()
        .enumerate()
        .map(|(i, option)| {
            let is_selected = i == popup.selected_index;
            let prefix = match (option.kind, option.checked) {
                (crate::app::view_options::Kind::Radio, true) => "●",
                (crate::app::view_options::Kind::Radio, false) => "○",
                (crate::app::view_options::Kind::Checkbox, true) => "☑",
                (crate::app::view_options::Kind::Checkbox, false) => "☐",
            };
            let label = option.label;
            let text = format!(" {} {}", prefix, label);
            let style = if is_selected {
                Style::default()
                    .bg(t.colors.bg_selection)
                    .fg(t.colors.selection_text)
            } else {
                Style::default().fg(t.colors.fg_primary)
            };
            ListItem::new(Line::from(Span::styled(text, style)))
        })
        .collect();

    if inner.height > 0 {
        let list = List::new(items);
        frame.render_widget(list, inner);
    }
}
