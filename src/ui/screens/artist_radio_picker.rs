//! Multi-artist radio picker popup.
//!
//! Two-step popup:
//! 1. Enter number of artists to blend (2-12)
//! 2. Search and select artists from cached library

use crate::app::state::{ArtistRadioPickerStep, SearchFocus};
use crate::app::AppState;
use crate::services::NavigationService;
use crate::ui::layout::centered_rect;
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

/// Render the artist radio picker popup as an overlay.
pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let picker = match &state.artist_radio_picker {
        Some(p) => p,
        None => return,
    };
    // Popup takes 50% width, 70% height, centered
    let popup_area = centered_rect(50, 70, area);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    match picker.step {
        ArtistRadioPickerStep::EnterCount => render_count_step(frame, picker, popup_area),
        ArtistRadioPickerStep::SelectArtists => render_select_step(frame, picker, popup_area),
    }
}

fn render_count_step(
    frame: &mut Frame,
    picker: &crate::app::state::ArtistRadioPickerState,
    area: Rect,
) {
    let t = theme();

    let block = Block::default()
        .title(" artist radio ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.fg_accent))
        .style(Style::default().bg(t.colors.bg_primary));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(2), // instructions
            ratatui::layout::Constraint::Length(3), // input
            ratatui::layout::Constraint::Min(1),    // spacer
        ])
        .split(inner);

    // Instructions
    let instructions = Paragraph::new(vec![
        Line::from(Span::styled(
            "How many artists to blend? (1-12)",
            Style::default().fg(t.colors.fg_muted),
        )),
        Line::from(Span::styled(
            "Enter a number and press Enter.",
            Style::default().fg(t.colors.fg_muted).italic(),
        )),
    ]);
    frame.render_widget(instructions, chunks[0]);

    // Input field
    let input_block = Block::default()
        .title(" count ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border_focused))
        .style(Style::default().bg(t.colors.bg_primary));
    let input_inner = input_block.inner(chunks[1]);
    frame.render_widget(input_block, chunks[1]);

    let input_text = format!("{}▋", picker.count_input);
    let input = Paragraph::new(input_text)
        .style(Style::default().fg(t.colors.fg_primary));
    frame.render_widget(input, input_inner);
}

fn render_select_step(
    frame: &mut Frame,
    picker: &crate::app::state::ArtistRadioPickerState,
    area: Rect,
) {
    let t = theme();

    let title = format!(
        " select {} artists ({}/{} selected) ",
        picker.max_artists,
        picker.selected_artists.len(),
        picker.max_artists,
    );

    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.fg_accent))
        .style(Style::default().bg(t.colors.bg_primary));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(2), // selected artists + hint
            ratatui::layout::Constraint::Length(3), // search input
            ratatui::layout::Constraint::Min(3),    // results
        ])
        .split(inner);

    // Selected artists display + launch hint
    let selected_names: Vec<&str> = picker.selected_artists.iter()
        .map(|a| a.title.as_str())
        .collect();
    let selected_text = if selected_names.is_empty() {
        "No artists selected yet".to_string()
    } else {
        selected_names.join(", ")
    };
    let remaining = picker.max_artists - picker.selected_artists.len();
    let hint = if remaining == 0 {
        "Press Tab to launch".to_string()
    } else if remaining == 1 {
        "Enter to select (auto-launches on final)".to_string()
    } else {
        format!("Enter to select ({} remaining)", remaining)
    };
    let selected_display = Paragraph::new(vec![
        Line::from(Span::styled(
            selected_text,
            Style::default().fg(t.colors.fg_accent),
        )),
        Line::from(Span::styled(
            hint,
            Style::default().fg(t.colors.fg_muted).italic(),
        )),
    ]);
    frame.render_widget(selected_display, chunks[0]);

    // Search input
    let is_focused = picker.focus == SearchFocus::Input;
    let input_block = Block::default()
        .title(" filter ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if is_focused {
            t.colors.border_focused
        } else {
            t.colors.border
        }))
        .style(Style::default().bg(t.colors.bg_primary));
    let input_inner = input_block.inner(chunks[1]);
    frame.render_widget(input_block, chunks[1]);

    let query_text = if is_focused {
        format!("{}▋", picker.query)
    } else {
        picker.query.clone()
    };
    let fg = if is_focused { t.colors.fg_primary } else { t.colors.fg_muted };
    let input = Paragraph::new(query_text).style(Style::default().fg(fg));
    frame.render_widget(input, input_inner);

    // Results list
    render_artist_list(frame, picker, chunks[2]);
}

fn render_artist_list(
    frame: &mut Frame,
    picker: &crate::app::state::ArtistRadioPickerState,
    area: Rect,
) {
    let t = theme();

    if picker.filtered_artists.is_empty() {
        let empty = Paragraph::new("No matching artists")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, area);
        return;
    }

    let is_focused = picker.focus == SearchFocus::Results;
    let selected_idx = picker.item_index;
    let visible_height = area.height as usize;
    let total = picker.filtered_artists.len();
    let scroll_offset = match picker.scroll_pin {
        Some(pinned) => pinned,
        None => NavigationService::calc_scroll_offset(selected_idx, visible_height, total),
    };

    // Build set of selected artist keys for quick lookup
    let selected_keys: std::collections::HashSet<&str> = picker.selected_artists.iter()
        .map(|a| a.rating_key.as_str())
        .collect();

    let items: Vec<ListItem> = picker.filtered_artists.iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(i, artist)| {
            let is_selected_item = is_focused && i == selected_idx;
            let is_picked = selected_keys.contains(artist.rating_key.as_str());

            let prefix = if is_picked { "● " } else { "  " };
            let text = format!("{}{}", prefix, artist.title);

            let style = if is_selected_item {
                Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
            } else if is_picked {
                Style::default().fg(t.colors.fg_accent)
            } else {
                Style::default().fg(t.colors.fg_primary)
            };
            ListItem::new(text).style(style)
        })
        .collect();

    frame.render_widget(List::new(items), area);

    // Scrollbar for long lists
    if total > visible_height {
        crate::ui::widgets::render_scrollbar_borderless(frame, area, total, visible_height, scroll_offset);
    }
}

