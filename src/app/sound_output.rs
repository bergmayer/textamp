//! Input actions shared by the Sound Output popup. Rendering owns no state.
use crate::app::action::{Action, PlaybackAction};
use crate::app::AppState;
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};

fn cancel(state: &mut AppState) {
    state.popups.sound_output_active = false;
    state.sound_output.request_id = state.sound_output.request_id.wrapping_add(1);
    state.sound_output.loading = false;
}
fn apply(state: &AppState) -> Vec<Action> {
    state
        .sound_output
        .choices()
        .get(state.sound_output.index)
        .cloned()
        .map(|output| vec![PlaybackAction::SelectOutput(output).into()])
        .unwrap_or_default()
}
pub fn key(key: KeyEvent, state: &mut AppState) -> Vec<Action> {
    if matches!(
        key.code,
        KeyCode::Up | KeyCode::Down | KeyCode::Home | KeyCode::End
    ) {
        state.sound_output.scroll_pin = None;
    }
    let last = state.sound_output.choices().len().saturating_sub(1);
    match key.code {
        KeyCode::Esc | KeyCode::F(8) => cancel(state),
        KeyCode::Up => state.sound_output.index = state.sound_output.index.saturating_sub(1),
        KeyCode::Down => {
            state.sound_output.index = state.sound_output.index.saturating_add(1).min(last)
        }
        KeyCode::Home => state.sound_output.index = 0,
        KeyCode::End => state.sound_output.index = last,
        KeyCode::Enter => return apply(state),
        KeyCode::F(5) => return vec![PlaybackAction::DiscoverAirplay.into()],
        _ => {}
    }
    vec![]
}
pub fn mouse(event: MouseEvent, state: &mut AppState) -> Vec<Action> {
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let position = ratatui::layout::Position::new(event.column, event.row);
            if let Some((_, index)) = state
                .hit_regions
                .sound_output_rows
                .iter()
                .find(|(area, _)| area.contains(position))
            {
                state.sound_output.index = *index;
                state.sound_output.scroll_pin = state
                    .hit_regions
                    .sound_output_rows
                    .first()
                    .map(|(_, index)| *index);
            } else if let Some((_, button)) = state
                .hit_regions
                .sound_output_buttons
                .iter()
                .find(|(area, _)| area.contains(position))
            {
                match button {
                    0 => return apply(state),
                    1 => return vec![PlaybackAction::DiscoverAirplay.into()],
                    _ => cancel(state),
                }
            }
        }
        MouseEventKind::ScrollUp => {
            state.sound_output.scroll_pin = None;
            state.sound_output.index = state.sound_output.index.saturating_sub(1)
        }
        MouseEventKind::ScrollDown => {
            state.sound_output.scroll_pin = None;
            state.sound_output.index = state
                .sound_output
                .index
                .saturating_add(1)
                .min(state.sound_output.choices().len().saturating_sub(1))
        }
        _ => {}
    }
    vec![]
}
