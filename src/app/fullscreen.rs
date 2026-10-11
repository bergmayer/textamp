//! Viewer controls preserve the underlying navigation and playback state.
use crate::app::{action::NavigationAction, state::VisualizerTab, Action, AppState};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

pub fn mode(state: &AppState) -> usize {
    if state.fullscreen_artwork {
        0
    } else {
        VisualizerTab::ALL
            .iter()
            .position(|tab| *tab == state.visualizer_tab)
            .unwrap_or(0)
            + 1
    }
}
pub fn select(state: &mut AppState, index: usize) {
    state.fullscreen_artwork = index == 0;
    if let Some(tab) = index
        .checked_sub(1)
        .and_then(|index| VisualizerTab::ALL.get(index))
    {
        state.visualizer_tab = *tab;
    }
}
pub fn key(key: KeyEvent, state: &mut AppState) -> Vec<Action> {
    match key.code {
        KeyCode::Esc => return vec![NavigationAction::ToggleFullscreen.into()],
        KeyCode::Right | KeyCode::Tab => select(state, (mode(state) + 1) % 7),
        KeyCode::Left | KeyCode::BackTab => select(state, (mode(state) + 6) % 7),
        KeyCode::Char(c @ '1'..='7') if key.modifiers.is_empty() => {
            select(state, (c as u8 - b'1') as usize)
        }
        KeyCode::Char(':') => crate::app::command_palette::open(state),
        KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
            crate::app::command_palette::open(state)
        }
        _ => {}
    }
    vec![]
}
pub fn mouse(event: MouseEvent, state: &mut AppState) -> Vec<Action> {
    if event.kind == MouseEventKind::Down(MouseButton::Left) {
        let point = ratatui::layout::Position::new(event.column, event.row);
        if state
            .hit_regions
            .fullscreen_exit
            .is_some_and(|area| area.contains(point))
        {
            return vec![NavigationAction::ToggleFullscreen.into()];
        }
        if let Some(index) = state
            .hit_regions
            .fullscreen_modes
            .iter()
            .find_map(|(area, index)| area.contains(point).then_some(*index))
        {
            select(state, index);
        }
    }
    vec![]
}
