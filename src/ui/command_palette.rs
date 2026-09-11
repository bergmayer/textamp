//! Command-palette rendering. Behavior lives in app::command_palette.
use crate::ui::theme::theme;
use crate::ui::RenderState as AppState;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};
pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    if !state.palette.open {
        return;
    }
    let t = theme();

    let popup_area = centered(area, 60, 20);
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.title_focused))
        .title(if state.station_nav.loading {
            " command — loading station choices… "
        } else {
            " command "
        })
        .title_style(Style::default().fg(t.colors.title_focused))
        .style(
            Style::default()
                .bg(t.colors.bg_primary)
                .fg(t.colors.fg_primary),
        );
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(1),
            ratatui::layout::Constraint::Length(1),
            ratatui::layout::Constraint::Min(1),
        ])
        .split(inner);

    let prompt = ":";
    let input_line = Line::from(vec![
        Span::styled(prompt, Style::default().fg(t.colors.fg_accent)),
        Span::raw(" "),
        Span::raw(state.palette.query.clone()),
    ]);
    frame.render_widget(Paragraph::new(input_line), chunks[0]);
    let cursor_x = chunks[0].x + 2 + state.palette.cursor as u16;
    if cursor_x < chunks[0].x + chunks[0].width {
        frame.set_cursor_position((cursor_x, chunks[0].y));
    }

    frame.render_widget(
        Paragraph::new("─".repeat(chunks[1].width as usize))
            .style(Style::default().fg(t.colors.border)),
        chunks[1],
    );

    let items: Vec<ListItem> = state
        .palette
        .matches
        .iter()
        .filter_map(|&i| state.palette.entries.get(i))
        .map(|e| {
            let label_w = chunks[2].width.saturating_sub(e.hint.len() as u16 + 2) as usize;
            let label = if e.label.chars().count() > label_w {
                let truncated: String = e.label.chars().take(label_w.saturating_sub(1)).collect();
                format!("{}…", truncated)
            } else {
                e.label.clone()
            };
            let label_display_len = label.chars().count();
            let pad = " ".repeat(label_w.saturating_sub(label_display_len));
            ListItem::new(Line::from(vec![
                Span::raw(label),
                Span::raw(pad),
                Span::styled(e.hint.clone(), Style::default().fg(t.colors.fg_muted)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(t.colors.bg_selection)
                .fg(t.colors.selection_text)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    if !state.palette.matches.is_empty() {
        list_state.select(Some(state.palette.selected));
    }
    frame.render_stateful_widget(list, chunks[2], &mut list_state);

    // Register hit regions so mouse clicks can pick a row. Each
    // visible row maps to its `state.palette.matches` index — the
    // mouse handler bumps `state.palette.selected` to that index and
    // then runs the same Execute path Enter triggers.
    let visible = chunks[2].height as usize;
    let total_matches = state.palette.matches.len();
    // Mirror ratatui's ListState scroll: keep `selected` in view.
    let scroll = if total_matches <= visible {
        0
    } else if state.palette.selected >= visible {
        state.palette.selected + 1 - visible
    } else {
        0
    };
    let mut rows = Vec::with_capacity(visible.min(total_matches));
    for vis_row in 0..visible {
        let match_idx = scroll + vis_row;
        if match_idx >= total_matches {
            break;
        }
        rows.push((
            Rect {
                x: chunks[2].x,
                y: chunks[2].y + vis_row as u16,
                width: chunks[2].width,
                height: 1,
            },
            match_idx,
        ));
    }
    state.hit_regions.borrow_mut().command_palette =
        Some(crate::app::presentation::CommandPaletteRegions {
            outer: popup_area,
            rows,
        });
}

fn centered(area: Rect, width_pct: u16, height_lines: u16) -> Rect {
    let w = (area.width * width_pct / 100)
        .max(40)
        .min(area.width.saturating_sub(4));
    let h = height_lines.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 3;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}
