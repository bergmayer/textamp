//! Radio launcher popup (Start Radio from Radio section).
//!
//! Search for an artist to start Plex radio.
//! Uses Plex's playQueue API which incorporates full server-side heuristics.

use crate::app::state::{RadioLauncherTab, SearchFocus};
use crate::app::AppState;
use crate::services::NavigationService;
use crate::ui::layout::centered_rect;
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs};

/// Render the radio launcher popup as an overlay.
pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let launcher = match &state.popups.radio_launcher {
        Some(l) => l,
        None => return,
    };
    let t = theme();

    // Popup takes 50% width, 70% height, centered
    let popup_area = centered_rect(50, 70, area);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" start radio ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.fg_accent))
        .style(Style::default().bg(t.colors.bg_primary));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // Split inner area: instructions, tabs, search input, results
    let chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(2), // instructions
            ratatui::layout::Constraint::Length(2), // tabs
            ratatui::layout::Constraint::Length(3), // search input
            ratatui::layout::Constraint::Min(3),    // results
        ])
        .split(inner);

    // Instructions
    let instructions = Paragraph::new(vec![
        Line::from(Span::styled(
            "Search for an artist to start radio.",
            Style::default().fg(t.colors.fg_muted),
        )),
        Line::from(Span::styled(
            "Tip: press Alt+R on any selection in Library or Search for sonic radio.",
            Style::default().fg(t.colors.fg_muted).italic(),
        )),
    ]);
    frame.render_widget(instructions, chunks[0]);

    // Tabs
    render_tabs(frame, launcher.tab, chunks[1]);

    // Search input
    render_search_input(frame, &launcher.query, launcher.focus == SearchFocus::Input, chunks[2]);

    // Results
    render_results(frame, launcher, chunks[3]);
}

fn render_tabs(frame: &mut Frame, tab: RadioLauncherTab, area: Rect) {
    let t = theme();

    let titles: Vec<&str> = RadioLauncherTab::all().iter().map(|t| t.name()).collect();
    let selected_idx = match tab {
        RadioLauncherTab::All => 0,
        RadioLauncherTab::Artists => 1,
    };

    let tabs = Tabs::new(titles)
        .select(selected_idx)
        .style(Style::default().fg(t.colors.fg_muted))
        .highlight_style(Style::default().fg(t.colors.fg_accent).bold())
        .divider(" | ");

    frame.render_widget(tabs, area);
}

fn render_search_input(frame: &mut Frame, query: &str, is_focused: bool, area: Rect) {
    let t = theme();

    let input_block = Block::default()
        .title(" search ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if is_focused {
            t.colors.border_focused
        } else {
            t.colors.border
        }))
        .style(Style::default().bg(t.colors.bg_primary));

    let input_inner = input_block.inner(area);
    frame.render_widget(input_block, area);

    let query_text = if is_focused {
        format!("{}▋", query)
    } else {
        query.to_string()
    };
    let fg = if is_focused { t.colors.fg_primary } else { t.colors.fg_muted };
    let input = Paragraph::new(query_text).style(Style::default().fg(fg));
    frame.render_widget(input, input_inner);
}

fn render_results(frame: &mut Frame, launcher: &crate::app::state::RadioLauncherState, area: Rect) {
    let t = theme();

    let results = match &launcher.results {
        Some(r) => r,
        None => {
            let msg = if launcher.query.is_empty() {
                "Type to search your library"
            } else if launcher.loading {
                "Searching..."
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

    let is_results_focused = launcher.focus == SearchFocus::Results;
    let selected_idx = launcher.item_index;

    // Both tabs show artists only (All shows with header, Artists shows plain list)
    match launcher.tab {
        RadioLauncherTab::All => render_all_tab(frame, results, is_results_focused, selected_idx, area),
        RadioLauncherTab::Artists => render_single_section(
            frame, &results.artists, |a| a.title.clone(),
            is_results_focused, selected_idx, area,
        ),
    }
}

/// Render the All tab with artists section header.
fn render_all_tab(
    frame: &mut Frame,
    results: &crate::plex::models::SearchResults,
    is_focused: bool,
    selected_idx: usize,
    area: Rect,
) {
    let t = theme();

    if results.artists.is_empty() {
        let empty = Paragraph::new("No matches found")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, area);
        return;
    }

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

    let visible_height = area.height as usize;
    let display_selected = entries.iter().position(|(_, _, idx)| *idx == Some(selected_idx)).unwrap_or(0);
    let scroll_offset = NavigationService::calc_scroll_offset(display_selected, visible_height, entries.len());

    let items: Vec<ListItem> = entries.iter()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(text, is_header, sel_idx)| {
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

    // Scrollbar for long lists
    if entries.len() > visible_height {
        crate::ui::widgets::render_scrollbar_borderless(frame, area, entries.len(), visible_height, scroll_offset);
    }
}

/// Render a single-section list.
fn render_single_section<T, F>(
    frame: &mut Frame,
    items: &[T],
    format_item: F,
    is_focused: bool,
    selected_idx: usize,
    area: Rect,
) where
    F: Fn(&T) -> String,
{
    let t = theme();

    if items.is_empty() {
        let empty = Paragraph::new("No matches")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, area);
        return;
    }

    let visible_height = area.height as usize;
    let total = items.len();
    let scroll_offset = NavigationService::calc_scroll_offset(selected_idx, visible_height, total);

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

    // Scrollbar for long lists
    if total > visible_height {
        crate::ui::widgets::render_scrollbar_borderless(frame, area, total, visible_height, scroll_offset);
    }
}

