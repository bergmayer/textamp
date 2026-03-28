//! Artist detail screen.

use crate::app::AppState;
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

pub fn render(frame: &mut Frame, state: &AppState, _rating_key: &str, area: Rect) {
    let t = theme();

    // Fill background
    frame.render_widget(
        Block::default().style(Style::default().bg(t.colors.bg_primary)),
        area,
    );

    let block = Block::default()
        .title(" Artist ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Artist name
            Constraint::Min(5),    // Albums
        ])
        .split(inner);

    // Artist name
    if let Some(artist) = &state.current_artist {
        let name = Paragraph::new(artist.title.as_str())
            .style(Style::default().fg(t.colors.fg_accent))
            .alignment(Alignment::Left);
        frame.render_widget(name, chunks[0]);
    } else {
        let loading = Paragraph::new("Loading artist...")
            .style(Style::default().fg(t.colors.fg_muted));
        frame.render_widget(loading, chunks[0]);
    }

    // Albums list
    if state.library.albums.is_empty() {
        let empty = Paragraph::new("Loading albums...")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, chunks[1]);
        return;
    }

    let items: Vec<ListItem> = state
        .albums
        .iter()
        .enumerate()
        .map(|(i, album)| {
            let style = if i == state.list_state.albums_index {
                Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
            } else {
                Style::default().fg(t.colors.fg_primary)
            };

            let year = album.year.map(|y| format!(" ({})", y)).unwrap_or_default();
            let text = format!("{}{} - {} tracks", album.title, year, album.track_count());

            ListItem::new(text).style(style)
        })
        .collect();

    let albums_block = Block::default()
        .title(" Albums ")
        .title_style(Style::default().fg(t.colors.fg_muted))
        .borders(Borders::TOP)
        .style(Style::default().bg(t.colors.bg_primary));

    let list = List::new(items).block(albums_block);
    frame.render_widget(list, chunks[1]);
}
