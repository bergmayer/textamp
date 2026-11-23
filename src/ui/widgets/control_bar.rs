//! Terminal command bars publish the same typed controls they draw.
use crate::ui::theme::theme;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    widgets::Paragraph,
};

/// A rendered command's frontend text and shared semantic state.
pub struct Button<'a, Id> {
    pub id: Id,
    pub text: &'a str,
    pub enabled: bool,
    pub active: bool,
}

/// Horizontal sizing and alignment of each row of controls.
pub enum Arrangement {
    Fill(u16),
    Centered {
        columns: u16,
        button_width: u16,
        row_height: u16,
    },
}

/// Render equal-width rows and return only visible, typed activation targets.
/// Callers use these targets for mouse input and keyboard traversal.
pub fn render<Id: Copy + Eq>(
    frame: &mut Frame,
    area: Rect,
    buttons: &[Button<'_, Id>],
    arrangement: Arrangement,
    focused: Option<Id>,
) -> Vec<(Rect, Id)> {
    let colors = &theme().colors;
    let (columns, preferred_width, row_height) = match arrangement {
        Arrangement::Fill(columns) => (columns.max(1), None, 1),
        Arrangement::Centered {
            columns,
            button_width,
            row_height,
        } => (columns.max(1), Some(button_width), row_height.max(1)),
    };
    let width = preferred_width
        .unwrap_or(area.width / columns)
        .min(area.width / columns)
        .max(1);
    buttons
        .iter()
        .enumerate()
        .filter_map(|(index, button)| {
            let row = index / usize::from(columns);
            let count =
                (buttons.len() - row * usize::from(columns)).min(usize::from(columns)) as u16;
            let inset = if preferred_width.is_some() {
                area.width.saturating_sub(count * width) / 2
            } else {
                0
            };
            let rect = Rect::new(
                area.x + inset + index as u16 % columns * width,
                area.y + index as u16 / columns * row_height,
                width,
                row_height,
            )
            .intersection(area);
            if rect.is_empty() {
                return None;
            }
            let style = if focused == Some(button.id) {
                colors.focused_row()
            } else if !button.enabled {
                Style::new().fg(colors.fg_muted)
            } else if button.active {
                Style::new()
                    .fg(colors.fg_accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(colors.fg_primary)
            };
            frame.render_widget(
                Paragraph::new(button.text)
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(style),
                rect,
            );
            Some((rect, button.id))
        })
        .collect()
}
