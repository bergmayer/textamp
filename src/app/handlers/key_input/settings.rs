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

/// Handle Settings view keys.
pub(super) fn handle_settings_keys(
    key: event::KeyEvent,
    state: &mut AppState,
    _config: &crate::config::Config,
) -> Vec<Action> {
    use crate::app::state::{SettingsFocus, SettingsSection};

    if matches!(
        key.code,
        KeyCode::Up
            | KeyCode::Down
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Tab
            | KeyCode::BackTab
    ) {
        state.scroll.settings_textamp = None;
    }
    if matches!(state.settings_state.section, SettingsSection::Textamp)
        && state.settings_state.focus == SettingsFocus::Content
    {
        let last = state.textamp_settings().len().saturating_sub(1);
        let index = state.settings_state.item_index;
        match key.code {
            KeyCode::PageUp => state.settings_state.item_index = index.saturating_sub(10),
            KeyCode::PageDown => {
                state.settings_state.item_index = index.saturating_add(10).min(last)
            }
            KeyCode::Home => state.settings_state.item_index = 0,
            KeyCode::End => state.settings_state.item_index = last,
            _ => {}
        }
    }
    match key.code {
        KeyCode::Esc => vec![NavigationAction::SetView(View::Browse).into()],
        // Tab/Shift+Tab always switch sidebar/content; arrows navigate within it.
        KeyCode::Tab | KeyCode::BackTab => {
            state.settings_state.focus = match state.settings_state.focus {
                SettingsFocus::Sections => SettingsFocus::Content,
                SettingsFocus::Content => SettingsFocus::Sections,
            };
            vec![]
        }
        KeyCode::Right => {
            if state.settings_state.focus == SettingsFocus::Sections {
                state.settings_state.focus = SettingsFocus::Content;
                state.settings_state.item_index = 0;
                state.settings_state.scroll = 0;
            }
            vec![]
        }
        KeyCode::Left => {
            if state.settings_state.focus == SettingsFocus::Content {
                state.settings_state.focus = SettingsFocus::Sections;
                state.settings_state.scroll = 0;
                state.sources.picker_scroll_pin = None;
            }
            vec![]
        }
        KeyCode::Up => {
            match state.settings_state.focus {
                SettingsFocus::Sections => {
                    // Navigate sections
                    state.settings_state.section = state.settings_state.section.prev();
                    state.settings_state.item_index = 0;
                    state.settings_state.scroll = 0;
                }
                SettingsFocus::Content => {
                    if state.settings_state.section == SettingsSection::About {
                        // Scroll About content
                        state.settings_state.scroll = state.settings_state.scroll.saturating_sub(1);
                    } else {
                        // Navigate items within section
                        if state.settings_state.item_index > 0 {
                            state.settings_state.item_index -= 1;
                        }
                    }
                }
            }
            vec![]
        }
        KeyCode::Down => {
            match state.settings_state.focus {
                SettingsFocus::Sections => {
                    // Navigate sections
                    state.settings_state.section = state.settings_state.section.next();
                    state.settings_state.item_index = 0;
                    state.settings_state.scroll = 0;
                }
                SettingsFocus::Content => {
                    if state.settings_state.section == SettingsSection::About {
                        // Scroll About content (renderer will clamp to max)
                        state.settings_state.scroll = state.settings_state.scroll.saturating_add(1);
                    } else {
                        // Navigate items within section with bounds check
                        let max_index = match state.settings_state.section {
                            SettingsSection::Textamp => {
                                state.textamp_settings().len().saturating_sub(1)
                            }
                            SettingsSection::Libraries => 0,
                            SettingsSection::About => 0,
                        };
                        if state.settings_state.item_index < max_index {
                            state.settings_state.item_index += 1;
                        }
                    }
                }
            }
            vec![]
        }
        KeyCode::PageUp => {
            if state.settings_state.section == SettingsSection::About
                && state.settings_state.focus == SettingsFocus::Content
            {
                state.settings_state.scroll = state.settings_state.scroll.saturating_sub(10);
            }
            vec![]
        }
        KeyCode::PageDown => {
            if state.settings_state.section == SettingsSection::About
                && state.settings_state.focus == SettingsFocus::Content
            {
                state.settings_state.scroll = state.settings_state.scroll.saturating_add(10);
            }
            vec![]
        }
        KeyCode::Home => {
            if state.settings_state.section == SettingsSection::About
                && state.settings_state.focus == SettingsFocus::Content
            {
                state.settings_state.scroll = 0;
            }
            vec![]
        }
        KeyCode::End => {
            if state.settings_state.section == SettingsSection::About
                && state.settings_state.focus == SettingsFocus::Content
            {
                state.settings_state.scroll = u16::MAX; // renderer will clamp
            }
            vec![]
        }
        KeyCode::Enter => {
            if state.settings_state.focus == SettingsFocus::Sections {
                // Enter on section -> move to content
                state.settings_state.focus = SettingsFocus::Content;
                state.settings_state.item_index = 0;
                vec![]
            } else {
                // Enter on content -> select item
                vec![SettingsAction::SettingsSelect.into()]
            }
        }
        _ => vec![],
    }
}
