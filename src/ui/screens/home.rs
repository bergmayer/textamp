//! Home screen with hubs.

use crate::app::AppState;
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Fill background
    frame.render_widget(
        Block::default().style(Style::default().bg(t.colors.bg_primary)),
        area,
    );

    let block = Block::default()
        .title(" Home ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.home_loading {
        let loading = Paragraph::new("Loading...")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(loading, inner);
        return;
    }

    if state.home_hubs.is_empty() {
        let empty = Paragraph::new("No hubs available.\nPress 'r' to refresh.")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    // Render hubs as a list
    let items: Vec<ListItem> = state
        .home_hubs
        .iter()
        .enumerate()
        .map(|(i, hub)| {
            let style = if i == state.list_state.home_hub_index {
                Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
            } else {
                Style::default().fg(t.colors.fg_primary)
            };

            let content = format!(
                "{} ({} items)",
                hub.title,
                hub.metadata.len()
            );

            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}
