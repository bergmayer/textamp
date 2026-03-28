//! Playlist screen.

use crate::app::AppState;
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

pub fn render(frame: &mut Frame, state: &AppState, rating_key: &str, area: Rect) {
    let t = theme();

    // Fill background
    frame.render_widget(
        Block::default().style(Style::default().bg(t.colors.bg_primary)),
        area,
    );

    let block = Block::default()
        .title(" Playlist ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Find the playlist
    if let Some(playlist) = state.library.playlists.iter().find(|p| p.rating_key == rating_key) {
        let text = format!(
            "{}\n{} tracks",
            playlist.title,
            playlist.track_count()
        );
        let content = Paragraph::new(text)
            .style(Style::default().fg(t.colors.fg_accent));
        frame.render_widget(content, inner);
    } else {
        let loading = Paragraph::new("Loading playlist...")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(loading, inner);
    }
}
