//! Radio columns and their inline station configuration.
use crate::ui::{RenderState, theme::theme};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

pub fn render(frame: &mut Frame, state: &RenderState<'_>, single: bool, area: Rect) {
    if area.is_empty() {
        return;
    }
    let colors = &theme().colors;
    let count = state.station_nav.columns.len();
    let total = (count + usize::from(state.station_detail.is_some())).max(1);
    let visible = if single {
        1
    } else {
        total.min((area.width / 28).max(1) as usize)
    };
    let start = if single && !state.track_pane_focused {
        state.station_nav.focused_column.min(total - 1)
    } else {
        total.saturating_sub(visible)
    };
    let width = area.width / visible as u16;
    for slot in start..start + visible {
        let x = area.x + (slot - start) as u16 * width;
        let rect = Rect::new(
            x,
            area.y,
            if slot + 1 == total {
                area.right() - x
            } else {
                width
            },
            area.height,
        );
        let focused = !state.category_column_focused
            && if slot == count {
                state.track_pane_focused || crate::app::radio_navigation::inline_input(state)
            } else {
                state.station_nav.focused_column == slot && !state.track_pane_focused
            };
        let title = if let Some(column) = state.station_nav.columns.get(slot) {
            column.title.as_str()
        } else {
            state
                .station_detail
                .as_deref()
                .and_then(|key| state.station_by_key(key))
                .map(|s| s.title.as_str())
                .unwrap_or("Radio")
        };
        let block = Block::default()
            .title(crate::util::column_heading::capitalized(title))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if focused {
                colors.border_focused
            } else {
                colors.border
            }))
            .style(Style::default().bg(colors.bg_primary).fg(colors.fg_primary));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        if let Some(column) = state.station_nav.columns.get(slot) {
            let mut selection = ListState::default().with_selected(Some(column.selected_index));
            let offset = crate::services::NavigationService::calc_scroll_offset(
                column.selected_index,
                inner.height as usize,
                column.stations().len(),
            );
            *selection.offset_mut() = offset;
            let rows = column
                .stations()
                .iter()
                .map(|station| ListItem::new(station.title.as_str()))
                .collect::<Vec<_>>();
            frame.render_stateful_widget(
                List::new(rows).highlight_style(if focused {
                    colors.focused_row()
                } else {
                    Style::default()
                        .fg(colors.fg_primary)
                        .bg(colors.bg_secondary)
                }),
                inner,
                &mut selection,
            );
            for (index, station) in column
                .stations()
                .iter()
                .enumerate()
                .skip(offset)
                .take(inner.height as usize)
            {
                state.hit_regions.borrow_mut().station_rows.push((
                    Rect::new(inner.x, inner.y + (index - offset) as u16, inner.width, 1),
                    slot,
                    station.key.clone(),
                ));
            }
        } else if let Some(station) = state
            .station_detail
            .as_deref()
            .and_then(|key| state.station_by_key(key))
        {
            let lines = (station.description_text().chars().count() as u16 / inner.width.max(1)
                + 2)
            .min(inner.height);
            frame.render_widget(
                Paragraph::new(station.description_text()).wrap(Wrap { trim: true }),
                Rect::new(inner.x, inner.y, inner.width, lines),
            );
            let body = Rect::new(
                inner.x,
                inner.y + lines,
                inner.width,
                inner.height.saturating_sub(lines),
            );
            if state.popups.adventure_launcher().is_some() {
                super::adventure_launcher::render_inline(frame, state, body);
            } else if state.popups.artist_radio_picker().is_some() {
                super::artist_radio_picker::render_inline(frame, state, body);
            } else if body.height > 0 {
                let text = if station.requires_input() {
                    "Enter: choose station options"
                } else {
                    "[ Play Station · p ]"
                };
                frame.render_widget(
                    Paragraph::new(text).style(Style::default().fg(colors.fg_accent)),
                    body,
                );
                state.hit_regions.borrow_mut().station_play = Some((
                    Rect::new(body.x, body.y, body.width, 1),
                    station.key.clone(),
                ));
            }
        }
    }
}
