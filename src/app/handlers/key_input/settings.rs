//! Settings and Help view key handling.

use crate::app::action::*;
use crossterm::event::{self, KeyCode};

use crate::app::state::View;
use crate::app::Action;
use crate::app::AppState;

/// Handle Help view keys.
pub(super) fn handle_help_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    let visible = state
        .hit_regions
        .help
        .as_ref()
        .map_or(state.terminal_height.saturating_sub(2), |r| {
            r.visible as u16
        });
    let max_scroll = state.hit_regions.help.as_ref().map_or_else(
        || (crate::util::help_text::total_lines() as u16).saturating_sub(visible),
        |r| r.max_scroll(),
    );
    state.help_scroll = state.help_scroll.min(max_scroll);
    match key.code {
        KeyCode::Esc | KeyCode::F(1) => {
            state.help_scroll = 0; // Reset scroll when closing
            vec![NavigationAction::SetView(View::Browse).into()]
        }
        KeyCode::Up => {
            state.help_scroll = state.help_scroll.saturating_sub(1);
            vec![]
        }
        KeyCode::Down => {
            state.help_scroll = state.help_scroll.saturating_add(1).min(max_scroll);
            vec![]
        }
        KeyCode::PageUp => {
            state.help_scroll = state.help_scroll.saturating_sub(visible);
            vec![]
        }
        KeyCode::PageDown => {
            state.help_scroll = state.help_scroll.saturating_add(visible).min(max_scroll);
            vec![]
        }
        KeyCode::Home => {
            state.help_scroll = 0;
            vec![]
        }
        KeyCode::End => {
            state.help_scroll = max_scroll;
            vec![]
        }
        _ => vec![],
    }
}

/// Settings has two focus targets: categories and the selected page's controls.
pub(super) fn handle_settings_keys(
    key: event::KeyEvent,
    state: &mut AppState,
    _config: &crate::config::Config,
) -> Vec<Action> {
    use crate::app::state::{SettingsFocus as Focus, SettingsSection as Section, SettingsState};
    if key.modifiers.intersects(
        event::KeyModifiers::CONTROL | event::KeyModifiers::ALT | event::KeyModifiers::SUPER,
    ) {
        return vec![];
    }
    let focus = state.settings_state.focus;
    if key.code == KeyCode::F(10) {
        state.settings_state.focus = Focus::Sections;
        return vec![];
    }
    if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
        state.settings_state.focus = if focus == Focus::Sections {
            Focus::Content
        } else {
            Focus::Sections
        };
        return vec![];
    }
    if key.code == KeyCode::Esc {
        if focus == Focus::Sections {
            return vec![NavigationAction::SetView(View::Browse).into()];
        }
        state.settings_state.focus = Focus::Sections;
        return vec![];
    }
    if matches!(
        key.code,
        KeyCode::Up
            | KeyCode::Down
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::PageUp
            | KeyCode::PageDown
    ) {
        state.scroll.settings_textamp = None;
        state.sources.picker_scroll_pin = None;
    }
    if focus == Focus::Sections {
        let index = state.settings_state.category_index();
        let next = match key.code {
            KeyCode::Up => index.saturating_sub(1),
            KeyCode::Down => (index + 1).min(SettingsState::CATEGORY_COUNT - 1),
            KeyCode::Home | KeyCode::PageUp => 0,
            KeyCode::End | KeyCode::PageDown => SettingsState::CATEGORY_COUNT - 1,
            KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => {
                state.settings_state.focus = Focus::Content;
                index
            }
            KeyCode::Char(c @ '1'..='7') => c as usize - '1' as usize,
            KeyCode::Char('a' | 'A') if state.settings_state.section == Section::Libraries => {
                return vec![crate::app::sources::LibraryChoice::Add.action()];
            }
            _ => index,
        };
        if next != index {
            state.settings_state.select_category(next);
            state.scroll.settings_textamp = None;
        }
        return vec![];
    }
    if key.code == KeyCode::Left {
        state.settings_state.focus = Focus::Sections;
        return vec![];
    }
    if state.settings_state.section == Section::Libraries {
        if key.code == KeyCode::Up && state.popups.library_picker_index == 0 {
            state.settings_state.focus = Focus::Sections;
            return vec![];
        }
        return super::handle_library_picker_keys(key, state);
    }
    if state.settings_state.section == Section::About {
        let max_scroll = state
            .hit_regions
            .settings_about
            .as_ref()
            .map_or(0, |r| r.max_scroll());
        let scroll = &mut state.settings_state.scroll;
        *scroll = (*scroll).min(max_scroll);
        match key.code {
            KeyCode::Up => *scroll = scroll.saturating_sub(1),
            KeyCode::Down => *scroll = scroll.saturating_add(1).min(max_scroll),
            KeyCode::PageUp => *scroll = scroll.saturating_sub(10),
            KeyCode::PageDown => *scroll = scroll.saturating_add(10).min(max_scroll),
            KeyCode::Home => *scroll = 0,
            KeyCode::End => *scroll = max_scroll,
            _ => {}
        }
        return vec![];
    }
    let last = state.textamp_settings().len().saturating_sub(1);
    let visible = state
        .hit_regions
        .settings_textamp
        .as_ref()
        .map_or(10, |r| r.inner.height.max(1) as usize);
    let index = &mut state.settings_state.item_index;
    match key.code {
        KeyCode::Up => *index = index.saturating_sub(1),
        KeyCode::Down => *index = index.saturating_add(1).min(last),
        KeyCode::PageUp => *index = index.saturating_sub(visible),
        KeyCode::PageDown => *index = index.saturating_add(visible).min(last),
        KeyCode::Home => *index = 0,
        KeyCode::End => *index = last,
        KeyCode::Enter | KeyCode::Char(' ') => return vec![SettingsAction::SettingsSelect.into()],
        _ => {}
    }
    vec![]
}
