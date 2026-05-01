//! Settings and Help view key handling.

use crate::app::action::*;
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
            vec![NavigationAction::SetView(View::Browse).into()]
        }
        KeyCode::Up => {
            state.help_scroll = state.help_scroll.saturating_sub(1);
            vec![]
        }
        KeyCode::Down => {
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
                return vec![SettingsAction::SaveCredentials.into()];
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
                vec![NavigationAction::SetView(View::Browse).into()]
            }
        }
        // Panel switching
        KeyCode::Tab | KeyCode::Right => {
            if state.settings_state.focus == SettingsFocus::Sections {
                state.settings_state.focus = SettingsFocus::Content;
                state.settings_state.item_index = 0;
                state.settings_state.scroll = 0;
            }
            vec![]
        }
        KeyCode::BackTab | KeyCode::Left => {
            if state.settings_state.focus == SettingsFocus::Content {
                state.settings_state.focus = SettingsFocus::Sections;
                state.settings_state.scroll = 0;
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
                            SettingsSection::Account => {
                                if state.settings_state.signing_in {
                                    // username(0), password(1), sign in(2), then servers(3+)
                                    2 + state.available_servers.len()
                                } else if matches!(state.connection, crate::app::state::ConnectionState::Connected { .. }) {
                                    // libraries(0..lib_count-1), actions(lib_count..lib_count+4), sign out(lib_count+5)
                                    (state.libraries.len() + 6).saturating_sub(1)
                                } else {
                                    0 // Sign In(0)
                                }
                            }
                            SettingsSection::Textamp => {
                                // Themes + Artwork modes + Local + remotes
                                // + Refresh + Transcode + 3 external-search
                                // toggles (Apple Music / Spotify / YouTube).
                                let theme_count = crate::app::theme::ThemeName::all().len();
                                let artwork_count = crate::app::state::ArtworkMode::all().len();
                                theme_count + artwork_count + 1 + state.remote.players.len() + 1 + 3
                            }
                            SettingsSection::Sections => {
                                crate::app::state::BrowseCategory::all().len().saturating_sub(1)
                            }
                            SettingsSection::Cache => 0,
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
            if state.settings_state.section == SettingsSection::About && state.settings_state.focus == SettingsFocus::Content {
                state.settings_state.scroll = state.settings_state.scroll.saturating_sub(10);
            }
            vec![]
        }
        KeyCode::PageDown => {
            if state.settings_state.section == SettingsSection::About && state.settings_state.focus == SettingsFocus::Content {
                state.settings_state.scroll = state.settings_state.scroll.saturating_add(10);
            }
            vec![]
        }
        KeyCode::Home => {
            if state.settings_state.section == SettingsSection::About && state.settings_state.focus == SettingsFocus::Content {
                state.settings_state.scroll = 0;
            }
            vec![]
        }
        KeyCode::End => {
            if state.settings_state.section == SettingsSection::About && state.settings_state.focus == SettingsFocus::Content {
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
                        vec![SettingsAction::SettingsSignIn.into()]
                    }
                    _ => {
                        // Server selection (index 3+)
                        vec![SettingsAction::SettingsSelect.into()]
                    }
                }
            } else {
                // Enter on content -> select item
                vec![SettingsAction::SettingsSelect.into()]
            }
        }
        _ => vec![],
    }
}
