//! Similar content popup overlay.
//!
//! Shows albums or tracks sonically similar to the selected item,
//! rendered as a centered popup over the previous view.

use crate::app::AppState;
use crate::app::state::SimilarMode;
use crate::services::NavigationService;
use crate::ui::layout::centered_rect;
use crate::ui::theme::theme;
use crate::util::{format_duration, truncate_middle};

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

/// Render the similar content popup overlay.
pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Centered popup: 65% wide, 80% tall
    let popup_area = centered_rect(65, 80, area);
    frame.render_widget(Clear, popup_area);

    let mode_label = match state.similar.mode {
        SimilarMode::Albums => "albums",
        SimilarMode::Tracks => "tracks",
    };
    let title = format!(" similar {} to: {} ", mode_label, state.similar.source_title);
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.fg_accent))
        .style(Style::default().bg(t.colors.bg_primary));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // Register hit regions for mouse handler (tab_hint set below after footer rendering)
    {
        let mut hr = state.hit_regions.borrow_mut();
        hr.similar_content = Some(crate::ui::hit_regions::SimilarRegions {
            outer: popup_area,
            inner,
            rows_per_item: 2,
            tab_hint: None,
        });
    }

    if state.similar.loading {
        let msg = match state.similar.mode {
            SimilarMode::Albums => "Loading similar albums...",
            SimilarMode::Tracks => "Loading similar tracks...",
        };
        let loading = Paragraph::new(msg)
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(loading, inner);
        return;
    }

    // Split inner: content list + footer
    let content_height = inner.height.saturating_sub(1);
    let content_area = Rect::new(inner.x, inner.y, inner.width, content_height);
    let footer_area = Rect::new(inner.x, inner.y + content_height, inner.width, 1);

    match state.similar.mode {
        SimilarMode::Albums => render_albums(frame, state, content_area, popup_area),
        SimilarMode::Tracks => render_tracks(frame, state, content_area, popup_area),
    }

    // Footer with dynamic Tab hint
    let mut footer_spans = vec![
        Span::styled(" [Esc] ", Style::default().fg(t.colors.shortcut_key)),
        Span::styled("close", Style::default().fg(t.colors.fg_muted)),
    ];

    // Track width before [Tab] to compute click region
    let esc_width: u16 = " [Esc] close".len() as u16;
    let mut has_tab_hint = false;

    match state.similar.mode {
        SimilarMode::Tracks => {
            if let Some(ref album_title) = state.similar.tab_album_title {
                has_tab_hint = true;
                footer_spans.push(Span::styled("  [Tab] ", Style::default().fg(t.colors.shortcut_key)));
                footer_spans.push(Span::styled(
                    format!("similar albums to: {}", album_title),
                    Style::default().fg(t.colors.fg_muted),
                ));
            }
        }
        SimilarMode::Albums => {
            if let Some(track) = state.current_track() {
                has_tab_hint = true;
                footer_spans.push(Span::styled("  [Tab] ", Style::default().fg(t.colors.shortcut_key)));
                footer_spans.push(Span::styled(
                    format!("similar tracks to: {} - {}", track.artist_name(), track.title),
                    Style::default().fg(t.colors.fg_muted),
                ));
            }
        }
    }

    let footer = Paragraph::new(Line::from(footer_spans));
    frame.render_widget(footer, footer_area);

    // Register [Tab] hint click region
    if has_tab_hint {
        let tab_x = footer_area.x + esc_width;
        let tab_width = footer_area.width.saturating_sub(esc_width);
        let tab_rect = Rect::new(tab_x, footer_area.y, tab_width, 1);
        let mut hr = state.hit_regions.borrow_mut();
        if let Some(ref mut regions) = hr.similar_content {
            regions.tab_hint = Some(tab_rect);
        }
    }
}

fn render_albums(frame: &mut Frame, state: &AppState, inner: Rect, popup_area: Rect) {
    let t = theme();

    if state.similar.albums.is_empty() {
        let empty = Paragraph::new("No similar albums found")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let selected_idx = state.list_state.similar_index;
    let rows_per_item = 2usize;
    let visible_item_count = inner.height as usize / rows_per_item;
    let total = state.similar.albums.len();

    let scroll_offset = match state.scroll.similar {
        Some(pinned) => pinned,
        None => NavigationService::calc_scroll_offset(selected_idx, visible_item_count, total),
    };

    let max_text_width = inner.width.saturating_sub(4) as usize;

    let items: Vec<ListItem> = state
        .similar.albums
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_item_count)
        .map(|(i, album)| {
            let is_selected = i == selected_idx;

            let title = &album.title;
            let year_str = album.year.map(|y| format!("({})", y)).unwrap_or_default();
            let artist = album.artist_name();

            // Line 1: title + right-aligned year
            let title_width = if !year_str.is_empty() {
                max_text_width.saturating_sub(year_str.len() + 1)
            } else {
                max_text_width
            };
            let title_display = truncate_middle(title, title_width);

            // Line 2: indented artist
            let subtitle_width = max_text_width.saturating_sub(5);
            let artist_display = truncate_middle(artist, subtitle_width);

            let (line1_fg, line2_fg, item_bg) = if is_selected {
                (
                    Style::default().fg(t.colors.selection_text),
                    Style::default().fg(t.colors.selection_text),
                    Style::default().bg(t.colors.selection_bar_bg),
                )
            } else {
                (
                    Style::default().fg(t.colors.fg_primary),
                    Style::default().fg(t.colors.fg_muted),
                    Style::default(),
                )
            };

            let line1 = if !year_str.is_empty() {
                let title_chars = title_display.chars().count();
                let pad = title_width.saturating_sub(title_chars);
                Line::from(Span::styled(
                    format!(" {}{}{} {}", title_display, " ".repeat(pad), "", year_str),
                    line1_fg,
                ))
            } else {
                Line::from(Span::styled(format!(" {}", title_display), line1_fg))
            };
            let line2 = Line::from(Span::styled(format!("     {}", artist_display), line2_fg));

            ListItem::new(Text::from(vec![line1, line2])).style(item_bg)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);

    // Scrollbar + position indicator
    if total > visible_item_count {
        crate::ui::widgets::render_scrollbar(frame, popup_area, total, visible_item_count, scroll_offset);

        let footer = format!("{}/{}", selected_idx + 1, total);
        let footer_area = Rect::new(
            popup_area.x + popup_area.width.saturating_sub(footer.len() as u16 + 2),
            popup_area.y + popup_area.height - 1,
            footer.len() as u16 + 1,
            1,
        );
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().fg(t.colors.fg_muted)),
            footer_area,
        );
    }
}

fn render_tracks(frame: &mut Frame, state: &AppState, inner: Rect, popup_area: Rect) {
    let t = theme();

    if state.similar.tracks.is_empty() {
        let empty = Paragraph::new("No similar tracks found")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let selected_idx = state.list_state.similar_index;
    let rows_per_item = 2usize;
    let visible_item_count = inner.height as usize / rows_per_item;
    let total = state.similar.tracks.len();

    let scroll_offset = match state.scroll.similar {
        Some(pinned) => pinned,
        None => NavigationService::calc_scroll_offset(selected_idx, visible_item_count, total),
    };

    let max_text_width = inner.width.saturating_sub(4) as usize;

    let items: Vec<ListItem> = state
        .similar.tracks
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_item_count)
        .map(|(i, track)| {
            let is_selected = i == selected_idx;

            let title = &track.title;
            let artist = track.track_artist();
            let album = track.album_name();
            let dur_str = format_duration(track.duration_ms());

            // Line 1: title + right-aligned duration
            let title_width = max_text_width.saturating_sub(dur_str.len() + 1);
            let title_display = truncate_middle(title, title_width);

            // Line 2: indented artist — album
            let subtitle = format!("{} \u{2014} {}", artist, album);
            let subtitle_width = max_text_width.saturating_sub(5);
            let subtitle_display = truncate_middle(&subtitle, subtitle_width);

            let (line1_fg, line2_fg, item_bg) = if is_selected {
                (
                    Style::default().fg(t.colors.selection_text),
                    Style::default().fg(t.colors.selection_text),
                    Style::default().bg(t.colors.selection_bar_bg),
                )
            } else {
                (
                    Style::default().fg(t.colors.fg_primary),
                    Style::default().fg(t.colors.fg_muted),
                    Style::default(),
                )
            };

            let title_chars = title_display.chars().count();
            let pad = title_width.saturating_sub(title_chars);
            let line1 = Line::from(Span::styled(
                format!(" {}{} {}", title_display, " ".repeat(pad), dur_str),
                line1_fg,
            ));
            let line2 = Line::from(Span::styled(format!("     {}", subtitle_display), line2_fg));

            ListItem::new(Text::from(vec![line1, line2])).style(item_bg)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);

    // Scrollbar + position indicator
    if total > visible_item_count {
        crate::ui::widgets::render_scrollbar(frame, popup_area, total, visible_item_count, scroll_offset);

        let footer = format!("{}/{}", selected_idx + 1, total);
        let footer_area = Rect::new(
            popup_area.x + popup_area.width.saturating_sub(footer.len() as u16 + 2),
            popup_area.y + popup_area.height - 1,
            footer.len() as u16 + 1,
            1,
        );
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().fg(t.colors.fg_muted)),
            footer_area,
        );
    }
}
