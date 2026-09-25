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

/// Settings owns navigation; Tab visits menus and controls in screen order.
pub(super) fn handle_settings_keys(
    key: event::KeyEvent,
    state: &mut AppState,
    _config: &crate::config::Config,
) -> Vec<Action> {
    use crate::app::state::{SettingsFocus as Focus, SettingsSection as Section};
    let textamp = state.settings_state.section == Section::Textamp;
    let focus = state.settings_state.focus;
    if matches!(key.code, KeyCode::F(10)) {
        state.settings_state.focus = Focus::Sections;
        return vec![];
    }
    if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
        state.settings_state.focus = match (focus, key.code == KeyCode::Tab, textamp) {
            (Focus::Sections, true, true) | (Focus::Content, false, true) => Focus::Pages,
            (Focus::Sections, _, _) | (Focus::Pages, true, _) => Focus::Content,
            _ => Focus::Sections,
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
    match focus {
        Focus::Sections => {
            let section = state.settings_state.section;
            state.settings_state.section = match key.code {
                KeyCode::Left => section.prev(),
                KeyCode::Right => section.next(),
                KeyCode::Char('1') => Section::Libraries,
                KeyCode::Char('2') => Section::Textamp,
                KeyCode::Char('3') => Section::About,
                KeyCode::Down | KeyCode::Enter | KeyCode::Char(' ') => {
                    state.settings_state.focus = if textamp {
                        Focus::Pages
                    } else {
                        Focus::Content
                    };
                    section
                }
                _ => section,
            };
            if section != state.settings_state.section {
                state.settings_state.item_index = 0;
                state.settings_state.scroll = 0;
                state.scroll.settings_textamp = None;
            }
        }
        Focus::Pages => match key.code {
            KeyCode::Left | KeyCode::Right => {
                state.settings_state.page =
                    state.settings_state.page.step(key.code == KeyCode::Right);
                state.settings_state.item_index = 0;
            }
            KeyCode::Up => state.settings_state.focus = Focus::Sections,
            KeyCode::Down | KeyCode::Enter | KeyCode::Char(' ') => {
                state.settings_state.focus = Focus::Content
            }
            _ => {}
        },
        Focus::Content => {
            if key.code == KeyCode::Left {
                state.settings_state.focus = if textamp {
                    Focus::Pages
                } else {
                    Focus::Sections
                };
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
                let scroll = &mut state.settings_state.scroll;
                match key.code {
                    KeyCode::Up => *scroll = scroll.saturating_sub(1),
                    KeyCode::Down => *scroll = scroll.saturating_add(1),
                    KeyCode::PageUp => *scroll = scroll.saturating_sub(10),
                    KeyCode::PageDown => *scroll = scroll.saturating_add(10),
                    KeyCode::Home => *scroll = 0,
                    KeyCode::End => *scroll = u16::MAX,
                    _ => {}
                }
            } else {
                let last = state.textamp_settings().len().saturating_sub(1);
                let index = &mut state.settings_state.item_index;
                match key.code {
                    KeyCode::Up => *index = index.saturating_sub(1),
                    KeyCode::Down => *index = index.saturating_add(1).min(last),
                    KeyCode::PageUp => *index = index.saturating_sub(10),
                    KeyCode::PageDown => *index = index.saturating_add(10).min(last),
                    KeyCode::Home => *index = 0,
                    KeyCode::End => *index = last,
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        return vec![SettingsAction::SettingsSelect.into()]
                    }
                    _ => {}
                }
            }
        }
    }
    vec![]
}
