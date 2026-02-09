//! Settings and Help view key handling.

use crossterm::event::{self, KeyCode};

use crate::app::Action;
use crate::app::state::View;
use crate::app::AppState;
use crate::plex::PlexAuth;

/// Handle Help view keys.
pub(super) fn handle_help_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    match key.code {
        KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('?') => {
            state.help_scroll = 0;  // Reset scroll when closing
            vec![Action::SetView(View::Browse)]
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.help_scroll = state.help_scroll.saturating_sub(1);
            vec![]
        }
        KeyCode::Down | KeyCode::Char('j') => {
            // Cap at max reasonable scroll (help text is ~140 lines)
            let max_scroll = 140u16.saturating_sub(state.terminal_height.saturating_sub(4));
            state.help_scroll = state.help_scroll.saturating_add(1).min(max_scroll);
            vec![]
        }
        KeyCode::PageUp => {
            state.help_scroll = state.help_scroll.saturating_sub(20);
            vec![]
        }
        KeyCode::PageDown => {
            // Cap at max reasonable scroll (help text is ~140 lines)
            let max_scroll = 140u16.saturating_sub(state.terminal_height.saturating_sub(4));
            state.help_scroll = state.help_scroll.saturating_add(20).min(max_scroll);
            vec![]
        }
        KeyCode::Home => {
            state.help_scroll = 0;
            vec![]
        }
        KeyCode::End => {
            // Set to max scroll based on terminal height
            let max_scroll = 140u16.saturating_sub(state.terminal_height.saturating_sub(4));
            state.help_scroll = max_scroll;
            vec![]
        }
        _ => vec![],
    }
}

/// Handle Settings view keys.
pub(super) fn handle_settings_keys(key: event::KeyEvent, state: &mut AppState, config: &crate::config::Config) -> Vec<Action> {
    use crate::app::state::{CredentialField, SettingsFocus, SettingsSection};

    // Handle credential editing mode first
    if let Some(field) = state.settings_state.editing_credential {
        match key.code {
            KeyCode::Esc => {
                // Cancel editing, restore original value
                state.settings_state.editing_credential = None;
                // Restore username from stored auth or config
                state.settings_state.username_input = PlexAuth::load_token()
                    .and_then(|s| s.username)
                    .or_else(|| config.plex.username.clone())
                    .unwrap_or_default();
                state.settings_state.password_input = String::new();
                return vec![];
            }
            KeyCode::Enter => {
                // Save credential and exit edit mode
                state.settings_state.editing_credential = None;
                return vec![Action::SaveCredentials];
            }
            KeyCode::Backspace => {
                // Delete last character
                match field {
                    CredentialField::Username => {
                        state.settings_state.username_input.pop();
                    }
                    CredentialField::Password => {
                        state.settings_state.password_input.pop();
                    }
                }
                return vec![];
            }
            KeyCode::Char(c) => {
                // Add character to input
                match field {
                    CredentialField::Username => {
                        state.settings_state.username_input.push(c);
                    }
                    CredentialField::Password => {
                        state.settings_state.password_input.push(c);
                    }
                }
                return vec![];
            }
            _ => return vec![],
        }
    }

    match key.code {
        KeyCode::Esc => {
            if state.settings_state.signing_in {
                // Cancel sign-in mode, go back to Account view
                state.settings_state.signing_in = false;
                state.settings_state.item_index = 0;
                state.settings_state.editing_credential = None;
                vec![]
            } else {
                vec![Action::SetView(View::Browse)]
            }
        }
        // Panel switching
        KeyCode::Tab | KeyCode::Right => {
            if state.settings_state.focus == SettingsFocus::Sections {
                state.settings_state.focus = SettingsFocus::Content;
                state.settings_state.item_index = 0;
            }
            vec![]
        }
        KeyCode::BackTab | KeyCode::Left => {
            if state.settings_state.focus == SettingsFocus::Content {
                state.settings_state.focus = SettingsFocus::Sections;
            }
            vec![]
        }
        KeyCode::Up => {
            match state.settings_state.focus {
                SettingsFocus::Sections => {
                    // Navigate sections
                    state.settings_state.section = state.settings_state.section.prev();
                    state.settings_state.item_index = 0;
                }
                SettingsFocus::Content => {
                    // Navigate items within section
                    if state.settings_state.item_index > 0 {
                        state.settings_state.item_index -= 1;
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
                }
                SettingsFocus::Content => {
                    // Navigate items within section with bounds check
                    let max_index = match state.settings_state.section {
                        SettingsSection::Account => {
                            if state.settings_state.signing_in {
                                // username(0), password(1), sign in(2), then servers(3+)
                                2 + state.available_servers.len()
                            } else if matches!(state.connection, crate::app::state::ConnectionState::Connected { .. }) {
                                0 // Sign Out(0)
                            } else {
                                0 // Sign In(0)
                            }
                        }
                        SettingsSection::Libraries => {
                            // Libraries + 4 action buttons (Clear Library/Artwork/Subfolder, Start Crawl)
                            (state.libraries.len() + 4).saturating_sub(1)
                        }
                        SettingsSection::Interface => {
                            crate::ui::theme::ThemeName::all().len().saturating_sub(1)
                        }
                        SettingsSection::Playback => 0,
                        SettingsSection::About => 0, // No selectable items
                    };
                    if state.settings_state.item_index < max_index {
                        state.settings_state.item_index += 1;
                    }
                }
            }
            vec![]
        }
        KeyCode::Enter => {
            if state.settings_state.focus == SettingsFocus::Sections {
                // Enter on section -> move to content
                state.settings_state.focus = SettingsFocus::Content;
                state.settings_state.item_index = 0;
                vec![]
            } else if state.settings_state.section == SettingsSection::Account && state.settings_state.signing_in {
                // In sign-in mode: handle credential fields vs sign in vs server selection
                match state.settings_state.item_index {
                    0 => {
                        // Username field - start editing
                        state.settings_state.editing_credential = Some(CredentialField::Username);
                        vec![]
                    }
                    1 => {
                        // Password field - start editing
                        state.settings_state.editing_credential = Some(CredentialField::Password);
                        vec![]
                    }
                    2 => {
                        // Sign In button - authenticate with entered credentials
                        vec![Action::SettingsSignIn]
                    }
                    _ => {
                        // Server selection (index 3+)
                        vec![Action::SettingsSelect]
                    }
                }
            } else {
                // Enter on content -> select item
                vec![Action::SettingsSelect]
            }
        }
        _ => vec![],
    }
}
