//! Filter screen (musikcube-style).
//!
//! Unified search/filter with tabs: All | Artists | Album Artists | Albums | Playlists | Tracks | Genres

use crate::app::state::{SearchSection, SearchTab};
use crate::app::AppState;
use crate::services::{NavigationService, SearchFilterService};
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs};

/// Render the filter screen as an overlay.
pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Filter popup takes 60% width, 70% height, centered
    let popup_area = centered_rect(60, 70, area);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" search / filter ")
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

    // Filtered results
    render_filtered_results(frame, state, chunks[2]);
}

fn render_tabs(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    let titles: Vec<&str> = SearchTab::all().iter().map(|t| t.name()).collect();
    let selected_idx = match state.search_tab {
        SearchTab::Global => 0,
        SearchTab::Artists => 1,
        SearchTab::AlbumArtists => 2,
        SearchTab::Albums => 3,
        SearchTab::Playlists => 4,
        SearchTab::Tracks => 5,
        SearchTab::Genres => 6,
    };

    let tabs = Tabs::new(titles)
        .select(selected_idx)
        .style(Style::default().fg(t.colors.fg_muted))
        .highlight_style(Style::default().fg(t.colors.fg_accent).bold())
        .divider(" | ");

    frame.render_widget(tabs, area);
}

fn render_search_input(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    let input_block = Block::default()
        .title(" search ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border))
        .style(Style::default().bg(t.colors.bg_primary));

    let input_inner = input_block.inner(area);
    frame.render_widget(input_block, area);

    // Show search query with cursor
    let query_text = format!("{}▋", state.search_query);
    let input = Paragraph::new(query_text).style(Style::default().fg(t.colors.fg_primary));
    frame.render_widget(input, input_inner);
}

fn render_filtered_results(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Show loading indicator while searching
    if state.filter_loading || state.search_loading {
        let loading = Paragraph::new("Searching...")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(loading, area);
        return;
    }

    // Global/All tab uses sectioned display
    if state.search_tab == SearchTab::Global {
        render_global_results(frame, state, area);
        return;
    }

    let results = get_filtered_items(state);
    let selected_idx = state.list_state.search_item_index;
    let visible_height = area.height as usize;
    let total = results.len();

    if results.is_empty() {
        let msg = if state.search_query.is_empty() {
            format!("Type to filter {}", state.search_tab.name().to_lowercase())
        } else if state.search_query.len() < 2 {
            "Type at least 2 characters to search".to_string()
        } else {
            "No matches".to_string()
        };
        let empty = Paragraph::new(msg)
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, area);
        return;
    }

    // Calculate scroll offset
    let scroll_offset = NavigationService::calc_scroll_offset(selected_idx, visible_height, total);

    let items: Vec<ListItem> = results
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(i, (title, _key))| {
            let is_selected = i == selected_idx;
            let style = if is_selected {
                Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
            } else {
                Style::default().fg(t.colors.fg_primary)
            };
            ListItem::new(title.as_str()).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
}

/// Render global search results with sections (Artists | Albums | Tracks).
fn render_global_results(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Check if we have search results
    let results = match &state.search_results {
        Some(r) => r,
        None => {
            let msg = if state.search_query.is_empty() {
                "Type to search all categories"
            } else if state.search_query.len() < 2 {
                "Type at least 2 characters to search"
            } else {
                "No results"
            };
            let empty = Paragraph::new(msg)
                .style(Style::default().fg(t.colors.fg_muted))
                .alignment(Alignment::Center);
            frame.render_widget(empty, area);
            return;
        }
    };

    if results.is_empty() {
        let empty = Paragraph::new("No matches found")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, area);
        return;
    }

    // Split area into 3 columns for Artists, Albums, Tracks
    let col_width = area.width / 3;
    let columns = [
        Rect::new(area.x, area.y, col_width, area.height),
        Rect::new(area.x + col_width, area.y, col_width, area.height),
        Rect::new(area.x + col_width * 2, area.y, area.width - col_width * 2, area.height),
    ];

    let selected_idx = state.list_state.search_item_index;

    // Column 0: Artists
    let is_active = state.list_state.search_section == SearchSection::Artists;
    render_global_section(
        frame, &columns[0], "Artists", is_active, selected_idx,
        &results.artists,
        |a| a.title.clone(),
        &t,
    );

    // Column 1: Albums
    let is_active = state.list_state.search_section == SearchSection::Albums;
    render_global_section(
        frame, &columns[1], "Albums", is_active, selected_idx,
        &results.albums,
        |a| format!("{} - {}", a.title, a.artist_name()),
        &t,
    );

    // Column 2: Tracks
    let is_active = state.list_state.search_section == SearchSection::Tracks;
    render_global_section(
        frame, &columns[2], "Tracks", is_active, selected_idx,
        &results.tracks,
        |tr| format!("{} - {}", tr.title, tr.artist_name()),
        &t,
    );
}

/// Render a section column for global search results.
fn render_global_section<T, F>(
    frame: &mut Frame,
    area: &Rect,
    title: &str,
    is_active: bool,
    selected_idx: usize,
    items: &[T],
    format_item: F,
    theme: &crate::ui::theme::Theme,
) where
    F: Fn(&T) -> String,
{
    let t = theme;

    let section_block = Block::default()
        .title(format!(" {} ({}) ", title, items.len()))
        .title_style(Style::default().fg(if is_active {
            t.colors.fg_accent
        } else {
            t.colors.fg_muted
        }))
        .borders(Borders::ALL)
        .border_style(if is_active {
            Style::default().fg(t.colors.border_focused)
        } else {
            Style::default().fg(t.colors.border)
        })
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = section_block.inner(*area);
    frame.render_widget(section_block, *area);

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("No results").style(Style::default().fg(t.colors.fg_muted)),
            inner,
        );
        return;
    }

    let visible_height = inner.height as usize;
    let total = items.len();
    let scroll = if is_active {
        NavigationService::calc_scroll_offset(selected_idx, visible_height, total)
    } else {
        0
    };

    let list_items: Vec<ListItem> = items.iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(i, item)| {
            let is_selected = is_active && i == selected_idx;
            let style = if is_selected {
                Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
            } else {
                Style::default().fg(t.colors.fg_primary)
            };
            ListItem::new(format_item(item)).style(style)
        })
        .collect();

    frame.render_widget(List::new(list_items), inner);
}

/// Get filtered items based on current search tab and search query.
/// Uses API search results if available, otherwise falls back to local filtering.
fn get_filtered_items(state: &AppState) -> Vec<(String, String)> {
    let query = &state.search_query;
    let api_results = state.filter_results.as_ref();

    let items = match state.search_tab {
        SearchTab::Global => vec![], // Global search uses the separate search screen
        SearchTab::Artists => {
            SearchFilterService::filter_artists(query, api_results, &state.artists)
        }
        SearchTab::AlbumArtists => {
            SearchFilterService::filter_album_artists(query, &state.albums)
        }
        SearchTab::Albums => {
            SearchFilterService::filter_albums(query, api_results, &state.albums)
        }
        SearchTab::Playlists => {
            SearchFilterService::filter_playlists(query, &state.playlists)
        }
        SearchTab::Tracks => {
            SearchFilterService::filter_tracks(query, api_results, &state.selected_album_tracks)
        }
        SearchTab::Genres => {
            SearchFilterService::filter_genres(query, &state.genres)
        }
    };

    // Convert FilteredItem to tuple for backward compatibility
    items.into_iter().map(|item| item.into()).collect()
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Percentage((100 - percent_y) / 2),
            ratatui::layout::Constraint::Percentage(percent_y),
            ratatui::layout::Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Horizontal)
        .constraints([
            ratatui::layout::Constraint::Percentage((100 - percent_x) / 2),
            ratatui::layout::Constraint::Percentage(percent_x),
            ratatui::layout::Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
