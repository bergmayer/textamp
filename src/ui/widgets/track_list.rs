//! Track list widget.

use crate::api::models::Track;
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Row, Table};

/// Render a list of tracks as a table.
pub fn render(
    frame: &mut Frame,
    tracks: &[Track],
    selected_index: usize,
    current_track_key: Option<&str>,
    area: Rect,
) {
    let t = theme();

    let header = Row::new(vec!["#", "Title", "Artist", "Duration"])
        .style(Style::default().fg(t.colors.fg_muted))
        .height(1);

    let rows: Vec<Row> = tracks
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let is_playing = current_track_key == Some(track.rating_key.as_str());
            let is_selected = i == selected_index;

            let style = if is_playing {
                Style::default().fg(t.colors.fg_accent)
            } else if is_selected {
                Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
            } else {
                Style::default().fg(t.colors.fg_primary)
            };

            let prefix = if is_playing { "▶" } else { " " };
            let num = format!("{}{}", prefix, track.track_number());

            Row::new(vec![
                num,
                track.title.clone(),
                track.track_artist().to_string(),
                format_duration(track.duration_ms()),
            ])
            .style(style)
        })
        .collect();

    let widths = [
        Constraint::Length(4),
        Constraint::Percentage(50),
        Constraint::Percentage(35),
        Constraint::Length(8),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::NONE).style(Style::default().bg(t.colors.bg_primary)))
        .row_highlight_style(Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg));

    frame.render_widget(table, area);
}

fn format_duration(ms: u64) -> String {
    let secs = ms / 1000;
    let mins = secs / 60;
    let secs = secs % 60;
    format!("{}:{:02}", mins, secs)
}
