//! Album detail screen.

use crate::app::AppState;
use crate::ui::theme::theme;
use crate::ui::widgets::track_list;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

pub fn render(frame: &mut Frame, state: &AppState, _rating_key: &str, area: Rect) {
    let t = theme();

    // Fill background
    frame.render_widget(
        Block::default().style(Style::default().bg(t.colors.bg_primary)),
        area,
    );

    let block = Block::default()
        .title(" Album ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // Album header
            Constraint::Min(5),    // Tracks
        ])
        .split(inner);

    // Album header
    if let Some(album) = &state.current_album {
        let year = album.year.map(|y| format!(" ({})", y)).unwrap_or_default();
        // Use actual loaded tracks count if available, otherwise fall back to metadata
        let track_count = if !state.current_album_tracks.is_empty() {
            state.current_album_tracks.len() as u32
        } else {
            album.track_count()
        };
        let header_text = format!(
            "{}\n{}{}\n{} tracks  [s] Similar",
            album.title,
            album.artist_name(),
            year,
            track_count
        );
        let header = Paragraph::new(header_text)
            .style(Style::default().fg(t.colors.fg_accent));
        frame.render_widget(header, chunks[0]);
    } else {
        let loading = Paragraph::new("Loading album...")
            .style(Style::default().fg(t.colors.fg_muted));
        frame.render_widget(loading, chunks[0]);
    }

    // Tracks
    if state.current_album_tracks.is_empty() {
        let empty = Paragraph::new("Loading tracks...")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, chunks[1]);
        return;
    }

    let current_track_key = state.current_track().map(|t| t.rating_key.as_str());

    track_list::render(
        frame,
        &state.current_album_tracks,
        state.list_state.album_tracks_index,
        current_track_key,
        chunks[1],
    );
}
