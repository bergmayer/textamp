//! Search popup screen (Ctrl+F).
//!
//! Local-first search with tabs: All | Artists | Albums | Playlists | Tracks | Genres

use crate::app::state::{SearchFocus, SearchTab};
use crate::app::AppState;
use crate::api::models::SearchResults;
use crate::services::NavigationService;
use crate::ui::layout::centered_rect;
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs};

/// Render the search popup as an overlay.
pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Popup takes 50% width, 70% height, centered
    let popup_area = centered_rect(50, 70, area);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" search ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.fg_accent))
        .style(Style::default().bg(t.colors.bg_primary));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // Split inner area: tabs, search input, results
    let chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(2),
            ratatui::layout::Constraint::Length(3),
            ratatui::layout::Constraint::Min(3),
        ])
        .split(inner);

    // Tabs
    render_tabs(frame, state, chunks[0]);

    // Search input
    render_search_input(frame, state, chunks[1]);

    // Results
    render_results(frame, state, chunks[2]);
}

fn render_tabs(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    let labels = SearchTab::all();
    let selected_idx = match state.search_tab {
        SearchTab::Global => 0,
        SearchTab::Artists => 1,
        SearchTab::Albums => 2,
        SearchTab::Playlists => 3,
        SearchTab::Tracks => 4,
        SearchTab::Genres => 5,
    };

    let is_tab_focused = state.search_focus == SearchFocus::Input;

    let titles: Vec<Line> = labels.iter().enumerate().map(|(i, tab)| {
        if i == selected_idx && is_tab_focused {
            Line::from(Span::styled(
                format!(" {} ", tab.name()),
                Style::default()
                    .fg(t.colors.selection_text)
                    .bg(t.colors.selection_bar_bg),
            ))
        } else if i == selected_idx {
            Line::from(Span::styled(
                format!(" {} ", tab.name()),
                Style::default()
                    .fg(t.colors.fg_accent)
                    .add_modifier(Modifier::BOLD),
            ))
        } else {
            Line::from(Span::styled(
                format!(" {} ", tab.name()),
                Style::default().fg(t.colors.fg_muted),
            ))
        }
    }).collect();

    let tabs = Tabs::new(titles)
        .select(selected_idx)
        .highlight_style(Style::default())
        .style(Style::default().bg(t.colors.bg_primary).fg(t.colors.fg_muted))
        .divider(Span::styled(" │ ", Style::default().fg(t.colors.fg_muted)))
        .padding("", "");

    frame.render_widget(tabs, area);
}

fn render_search_input(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    let is_input_focused = state.search_focus == SearchFocus::Input;

    let input_block = Block::default()
        .title(" search ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if is_input_focused {
            t.colors.border_focused
        } else {
            t.colors.border
        }))
        .style(Style::default().bg(t.colors.bg_primary));

    let input_inner = input_block.inner(area);
    frame.render_widget(input_block, area);

    // Show search query with cursor only when input is focused
    let query_text = if is_input_focused {
        format!("{}▋", state.search_query)
    } else {
        state.search_query.clone()
    };
    let fg = if is_input_focused { t.colors.fg_primary } else { t.colors.fg_muted };
    let input = Paragraph::new(query_text).style(Style::default().fg(fg));
    frame.render_widget(input, input_inner);
}

fn render_results(frame: &mut Frame, state: &AppState, area: Rect) {
    let scroll_pin = state.search_scroll_pin;
    let t = theme();

    let results = match &state.search_results {
        Some(r) => r,
        None => {
            let msg = if state.search_query.is_empty() {
                "Type to search library"
            } else {
                "Searching..."
            };
            let empty = Paragraph::new(msg)
                .style(Style::default().fg(t.colors.fg_muted))
                .alignment(Alignment::Center);
            frame.render_widget(empty, area);
            return;
        }
    };

    let is_results_focused = state.search_focus == SearchFocus::Results;
    let selected_idx = state.list_state.search_item_index;

    match state.search_tab {
        SearchTab::Global => render_all_tab(frame, results, is_results_focused, selected_idx, scroll_pin, area),
        SearchTab::Artists => render_single_section(
            frame, &results.artists, |a| if a.title.is_empty() { "Unknown Artist".to_string() } else { a.title.clone() },
            is_results_focused, selected_idx, scroll_pin, area,
        ),
        SearchTab::Albums => render_single_section(
            frame, &results.albums, |a| {
                let artist = a.artist_name();
                let title = if a.title.is_empty() {
                    format!("Unknown Album ({})", artist)
                } else if let Some(year) = a.year {
                    format!("{} ({})", a.title, year)
                } else {
                    a.title.clone()
                };
                format!("{} - {}", title, artist)
            },
            is_results_focused, selected_idx, scroll_pin, area,
        ),
        SearchTab::Playlists => render_single_section(
            frame, &results.playlists, |p| p.title.clone(),
            is_results_focused, selected_idx, scroll_pin, area,
        ),
        SearchTab::Tracks => {
            if state.search_track_loading && results.tracks.is_empty() {
                let loading = Paragraph::new("Searching tracks...")
                    .style(Style::default().fg(t.colors.fg_muted))
                    .alignment(Alignment::Center);
                frame.render_widget(loading, area);
            } else {
                render_single_section(
                    frame, &results.tracks, |tr| {
                        let title = if tr.title.is_empty() {
                            tr.file_name().unwrap_or("Unknown Track")
                        } else {
                            &tr.title
                        };
                        format!("{} - {}", title, tr.artist_name())
                    },
                    is_results_focused, selected_idx, scroll_pin, area,
                );
            }
        }
        SearchTab::Genres => render_single_section(
            frame, &results.genres, |g| g.title.clone(),
            is_results_focused, selected_idx, scroll_pin, area,
        ),
    }
}

/// Render the All tab with section headers.
fn render_all_tab(
    frame: &mut Frame,
    results: &SearchResults,
    is_focused: bool,
    selected_idx: usize,
    scroll_pin: Option<usize>,
    area: Rect,
) {
    let t = theme();

    if results.is_empty() {
        let empty = Paragraph::new("No matches found")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, area);
        return;
    }

    // Build flat list with section headers
    // Each entry is: (display_text, is_header, is_selectable_idx)
    // selectable_idx maps to the global flat index for selection tracking
    let mut entries: Vec<(String, bool, Option<usize>)> = Vec::new();
    let mut global_idx: usize = 0;

    // Artists section
    if !results.artists.is_empty() {
        entries.push((format!("── Artists ({}) ──", results.artists.len()), true, None));
        for a in &results.artists {
            entries.push((format!("  {}", a.title), false, Some(global_idx)));
            global_idx += 1;
        }
    }

    // Albums section
    if !results.albums.is_empty() {
        entries.push((format!("── Albums ({}) ──", results.albums.len()), true, None));
        for a in &results.albums {
            let artist = a.artist_name();
            let text = if a.title.is_empty() {
                format!("  Unknown Album ({}) - {}", artist, artist)
            } else if let Some(year) = a.year {
                format!("  {} ({}) - {}", a.title, year, artist)
            } else {
                format!("  {} - {}", a.title, artist)
            };
            entries.push((text, false, Some(global_idx)));
            global_idx += 1;
        }
    }

    // Playlists section
    if !results.playlists.is_empty() {
        entries.push((format!("── Playlists ({}) ──", results.playlists.len()), true, None));
        for p in &results.playlists {
            entries.push((format!("  {}", p.title), false, Some(global_idx)));
            global_idx += 1;
        }
    }

    // Genres section
    if !results.genres.is_empty() {
        entries.push((format!("── Genres ({}) ──", results.genres.len()), true, None));
        for g in &results.genres {
            entries.push((format!("  {}", g.title), false, Some(global_idx)));
            global_idx += 1;
        }
    }

    // Tracks section
    if !results.tracks.is_empty() {
        entries.push((format!("── Tracks ({}) ──", results.tracks.len()), true, None));
        for tr in &results.tracks {
            let title = if tr.title.is_empty() {
                tr.file_name().unwrap_or("Unknown Track")
            } else {
                &tr.title
            };
            entries.push((format!("  {} - {}", title, tr.artist_name()), false, Some(global_idx)));
            global_idx += 1;
        }
    }

    let visible_height = area.height as usize;

    // Find display position of selected item
    let display_selected = entries.iter().position(|(_, _, idx)| *idx == Some(selected_idx)).unwrap_or(0);
    let scroll_offset = match scroll_pin {
        Some(pinned) => pinned,
        None => NavigationService::calc_scroll_offset(display_selected, visible_height, entries.len()),
    };

    let items: Vec<ListItem> = entries.iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(_, (text, is_header, sel_idx))| {
            if *is_header {
                ListItem::new(text.as_str())
                    .style(Style::default().fg(t.colors.fg_accent))
            } else {
                let is_selected = is_focused && *sel_idx == Some(selected_idx);
                let style = if is_selected {
                    Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                } else {
                    Style::default().fg(t.colors.fg_primary)
                };
                ListItem::new(text.as_str()).style(style)
            }
        })
        .collect();

    frame.render_widget(List::new(items), area);
}

/// Render a single-section list (for Artists, Albums, Playlists, Tracks, Genres tabs).
fn render_single_section<T, F>(
    frame: &mut Frame,
    items: &[T],
    format_item: F,
    is_focused: bool,
    selected_idx: usize,
    scroll_pin: Option<usize>,
    area: Rect,
) where
    F: Fn(&T) -> String,
{
    let t = theme();

    if items.is_empty() {
        let msg = "No matches";
        let empty = Paragraph::new(msg)
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, area);
        return;
    }

    let visible_height = area.height as usize;
    let total = items.len();
    let scroll_offset = match scroll_pin {
        Some(pinned) => pinned,
        None => NavigationService::calc_scroll_offset(selected_idx, visible_height, total),
    };

    let list_items: Vec<ListItem> = items.iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(i, item)| {
            let is_selected = is_focused && i == selected_idx;
            let style = if is_selected {
                Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
            } else {
                Style::default().fg(t.colors.fg_primary)
            };
            ListItem::new(format_item(item)).style(style)
        })
        .collect();

    frame.render_widget(List::new(list_items), area);
}

