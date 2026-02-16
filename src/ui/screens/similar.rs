//! Similar content screen.
//!
//! Shows albums or tracks sonically similar to the selected item.

use crate::app::AppState;
use crate::app::state::SimilarMode;
use crate::services::NavigationService;
use crate::ui::theme::theme;
use crate::util::format_duration;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

/// Render the similar content screen.
pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Fill background
    frame.render_widget(
        Block::default().style(Style::default().bg(t.colors.bg_primary)),
        area,
    );

    let mode_label = match state.similar_mode {
        SimilarMode::Albums => "albums",
        SimilarMode::Tracks => "tracks",
    };
    let title = format!(" similar {} to: {} ", mode_label, state.similar_source_title);
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.fg_accent))
        .style(Style::default().bg(t.colors.bg_primary));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.similar_loading {
        let msg = match state.similar_mode {
            SimilarMode::Albums => "Loading similar albums...",
            SimilarMode::Tracks => "Loading similar tracks...",
        };
        let loading = Paragraph::new(msg)
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(loading, inner);
        return;
    }

    match state.similar_mode {
        SimilarMode::Albums => render_albums(frame, state, inner, area),
        SimilarMode::Tracks => render_tracks(frame, state, inner, area),
    }
}

fn render_albums(frame: &mut Frame, state: &AppState, inner: Rect, area: Rect) {
    let t = theme();

    if state.similar_albums.is_empty() {
        let empty = Paragraph::new("No similar albums found")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let selected_idx = state.list_state.similar_index;
    let visible_height = inner.height as usize;
    let total = state.similar_albums.len();

    let scroll_offset = match state.similar_scroll_pin {
        Some(pinned) => pinned,
        None => NavigationService::calc_scroll_offset(selected_idx, visible_height, total),
    };

    let items: Vec<ListItem> = state
        .similar_albums
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(i, album)| {
            let is_selected = i == selected_idx;

            let artist = album.artist_name();
            let title = &album.title;
            let year = album.year.map(|y| format!(" ({})", y)).unwrap_or_default();

            let line = format!("{} - {}{}", artist, title, year);

            let style = if is_selected {
                Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
            } else {
                Style::default().fg(t.colors.fg_primary)
            };

            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);

    // Position indicator
    if total > visible_height {
        let footer = format!("{}/{}", selected_idx + 1, total);
        let footer_area = Rect::new(
            area.x + area.width.saturating_sub(footer.len() as u16 + 2),
            area.y + area.height - 1,
            footer.len() as u16 + 1,
            1,
        );
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().fg(t.colors.fg_muted)),
            footer_area,
        );
    }
}

fn render_tracks(frame: &mut Frame, state: &AppState, inner: Rect, area: Rect) {
    let t = theme();

    if state.similar_tracks.is_empty() {
        let empty = Paragraph::new("No similar tracks found")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let selected_idx = state.list_state.similar_index;
    let visible_height = inner.height as usize;
    let total = state.similar_tracks.len();

    let scroll_offset = match state.similar_scroll_pin {
        Some(pinned) => pinned,
        None => NavigationService::calc_scroll_offset(selected_idx, visible_height, total),
    };

    let items: Vec<ListItem> = state
        .similar_tracks
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(i, track)| {
            let is_selected = i == selected_idx;

            let artist = track.artist_name();
            let title = &track.title;
            let album = track.album_name();
            let duration = format_duration(track.duration_ms());

            let line = format!("{} - {} ({}) [{}]", artist, title, album, duration);

            let style = if is_selected {
                Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
            } else {
                Style::default().fg(t.colors.fg_primary)
            };

            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);

    // Position indicator
    if total > visible_height {
        let footer = format!("{}/{}", selected_idx + 1, total);
        let footer_area = Rect::new(
            area.x + area.width.saturating_sub(footer.len() as u16 + 2),
            area.y + area.height - 1,
            footer.len() as u16 + 1,
            1,
        );
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().fg(t.colors.fg_muted)),
            footer_area,
        );
    }
}
