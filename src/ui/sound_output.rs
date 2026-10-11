//! Sound Output modal: current route, discovered routes and one shared selector.
use crate::ui::RenderState;
use ratatui::prelude::*;
use ratatui::widgets::{Clear, Paragraph, Wrap};

pub fn render(frame: &mut Frame, state: &RenderState) {
    let screen = frame.area();
    let width = screen.width.min(74);
    let height = screen.height.min(19);
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height - height) / 2,
        width,
        height,
    );
    let block = crate::ui::screens::settings::panel(" Sound Output ");
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .split(inner);
    frame.render_widget(
        Paragraph::new(format!(
            "Current: {}\nStop playback before changing outputs.",
            state.sound_output.current.label()
        ))
        .style(crate::ui::screens::settings::paper()),
        rows[0],
    );
    let choices = state.sound_output.choices();
    let visible = rows[1].height as usize;
    let offset = state
        .sound_output
        .scroll_pin
        .unwrap_or_else(|| {
            state
                .sound_output
                .index
                .saturating_sub(visible.saturating_sub(1))
        })
        .min(choices.len().saturating_sub(visible));
    for (index, choice) in choices.iter().enumerate().skip(offset).take(visible) {
        let line = Rect::new(
            rows[1].x,
            rows[1].y + (index - offset) as u16,
            rows[1].width,
            1,
        );
        let label = format!(
            "{} {}",
            if *choice == state.sound_output.current {
                "[*]"
            } else {
                "[ ]"
            },
            choice.label()
        );
        frame.render_widget(
            Paragraph::new(label).style(if index == state.sound_output.index {
                crate::ui::screens::settings::selected()
            } else {
                crate::ui::screens::settings::paper()
            }),
            line,
        );
        state
            .hit_regions
            .borrow_mut()
            .sound_output_rows
            .push((line, index));
    }
    let note = if let Some(error) = &state.sound_output.error {
        error.clone()
    } else if state.sound_output.loading {
        "Finding local outputs and AirPlay speakers...".into()
    } else if state.sound_output.destinations.is_empty() {
        "No AirPlay speakers found. F5 refreshes. Linux AirPlay requires avahi-browse and cliairplay.".into()
    } else {
        "One AirPlay destination. Pairing/PTP requirements are reported as errors; multiroom is not enabled.".into()
    };
    frame.render_widget(
        Paragraph::new(note)
            .wrap(Wrap { trim: true })
            .style(crate::ui::screens::settings::paper()),
        rows[2],
    );
    let buttons = Layout::horizontal([Constraint::Ratio(1, 3); 3]).split(rows[3]);
    for (index, label) in ["Enter Select", "F5 Refresh", "Esc Cancel"]
        .iter()
        .enumerate()
    {
        frame.render_widget(
            Paragraph::new(*label)
                .alignment(Alignment::Center)
                .style(crate::ui::screens::settings::selected()),
            buttons[index],
        );
        state
            .hit_regions
            .borrow_mut()
            .sound_output_buttons
            .push((buttons[index], index));
    }
}
