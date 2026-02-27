//! Related artists popup overlay.
//!
//! Shows artists related to the selected artist, with their albums
//! grouped by artist, rendered as a centered popup over the previous view.

use crate::app::AppState;
use crate::app::state::{RelatedSource, RelatedArtistGroup};
use crate::services::NavigationService;
use crate::ui::layout::centered_rect;
use crate::ui::theme::theme;
use crate::util::truncate_middle;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

/// Render the related artists popup overlay.
pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Centered popup: 65% wide, 80% tall
    let popup_area = centered_rect(65, 80, area);
    frame.render_widget(Clear, popup_area);

    let title = format!(" related to: {} ", state.related.source_title);
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.fg_accent))
        .style(Style::default().bg(t.colors.bg_primary));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // Register hit regions for mouse handler
    {
        let mut hr = state.hit_regions.borrow_mut();
        hr.related_content = Some(crate::ui::hit_regions::RelatedRegions {
            outer: popup_area,
            inner,
        });
    }

    if state.related.loading {
        let loading = Paragraph::new("Loading related artists...")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(loading, inner);
        return;
    }

    if state.related.groups.is_empty() {
        let empty = Paragraph::new("No related artists found")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    // Split inner: content list + footer
    let content_height = inner.height.saturating_sub(1);
    let content_area = Rect::new(inner.x, inner.y, inner.width, content_height);
    let footer_area = Rect::new(inner.x, inner.y + content_height, inner.width, 1);

    let selected_idx = state.list_state.related_index;
    let total = flat_count(&state.related.groups);
    let visible_item_count = content_area.height as usize;
    let max_text_width = content_area.width.saturating_sub(2) as usize;

    let scroll_offset = match state.scroll.related {
        Some(pinned) => pinned,
        None => NavigationService::calc_scroll_offset(selected_idx, visible_item_count, total),
    };

    // Build flat list items
    let mut items: Vec<ListItem> = Vec::new();
    let mut flat_idx = 0;

    for group in &state.related.groups {
        let group_size = 1 + group.albums.len();
        let group_end = flat_idx + group_size;

        // Skip groups entirely before scroll offset
        if group_end <= scroll_offset {
            flat_idx = group_end;
            continue;
        }

        // Artist header
        if flat_idx >= scroll_offset && items.len() < visible_item_count {
            let is_selected = flat_idx == selected_idx;
            let alias_suffix = match group.source {
                RelatedSource::Alias => " (alias)",
                RelatedSource::SimilarTag => " (similar)",
                RelatedSource::Plex => "",
            };
            let header_text = format!("  {}{}", group.artist.title, alias_suffix);
            let header_display = truncate_middle(&header_text, max_text_width);

            let (fg, bg) = if is_selected {
                (Style::default().fg(t.colors.selection_text).add_modifier(Modifier::BOLD),
                 Style::default().bg(t.colors.selection_bar_bg))
            } else {
                (Style::default().fg(t.colors.fg_accent).add_modifier(Modifier::BOLD),
                 Style::default())
            };

            items.push(ListItem::new(Line::from(Span::styled(header_display, fg))).style(bg));
        }
        flat_idx += 1;

        // Albums
        for album in &group.albums {
            if flat_idx >= scroll_offset && items.len() < visible_item_count {
                let is_selected = flat_idx == selected_idx;
                let year_str = album.year.map(|y| format!("{}", y)).unwrap_or_default();
                let title_width = if !year_str.is_empty() {
                    max_text_width.saturating_sub(year_str.len() + 7) // "    " + "  " + year
                } else {
                    max_text_width.saturating_sub(4)
                };
                let title_display = truncate_middle(&album.title, title_width);

                let (fg, bg) = if is_selected {
                    (Style::default().fg(t.colors.selection_text),
                     Style::default().bg(t.colors.selection_bar_bg))
                } else {
                    (Style::default().fg(t.colors.fg_primary),
                     Style::default())
                };

                let line = if !year_str.is_empty() {
                    let title_chars = title_display.chars().count();
                    let pad = title_width.saturating_sub(title_chars);
                    format!("    {}{}  {}", title_display, " ".repeat(pad), year_str)
                } else {
                    format!("    {}", title_display)
                };

                items.push(ListItem::new(Line::from(Span::styled(line, fg))).style(bg));
            }
            flat_idx += 1;
        }

        if items.len() >= visible_item_count {
            break;
        }
    }

    let list = List::new(items);
    frame.render_widget(list, content_area);

    // Scrollbar + position indicator
    if total > visible_item_count {
        crate::ui::widgets::render_scrollbar(frame, popup_area, total, visible_item_count, scroll_offset, None);

        let footer_pos = format!("{}/{}", selected_idx + 1, total);
        let footer_pos_area = Rect::new(
            popup_area.x + popup_area.width.saturating_sub(footer_pos.len() as u16 + 2),
            popup_area.y + popup_area.height - 1,
            footer_pos.len() as u16 + 1,
            1,
        );
        frame.render_widget(
            Paragraph::new(footer_pos).style(Style::default().fg(t.colors.fg_muted)),
            footer_pos_area,
        );
    }

    // Footer
    let footer_spans = vec![
        Span::styled(" [Esc] ", Style::default().fg(t.colors.shortcut_key)),
        Span::styled("close", Style::default().fg(t.colors.fg_muted)),
    ];
    let footer = Paragraph::new(Line::from(footer_spans));
    frame.render_widget(footer, footer_area);
}

/// Count total flat items in related groups (1 header + N albums per group).
fn flat_count(groups: &[RelatedArtistGroup]) -> usize {
    groups.iter().map(|g| 1 + g.albums.len()).sum()
}
