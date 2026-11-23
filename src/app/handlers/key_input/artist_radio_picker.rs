//! Multi-artist radio picker popup key handling.

use crate::app::action::*;
use crossterm::event::{self, KeyCode};

use crate::app::Action;
use crate::app::AppState;
use crate::app::state::SearchFocus;

/// Handle artist radio picker popup keys.
pub(super) fn handle_artist_radio_picker_keys(
    key: event::KeyEvent,
    state: &mut AppState,
) -> Vec<Action> {
    let picker = match state.popups.artist_radio_picker_mut() {
        Some(p) => p,
        None => return vec![],
    };

    handle_select_step(key, picker)
}

/// Handle keys in the SelectArtists step: search, navigate, toggle selection.
fn handle_select_step(
    key: event::KeyEvent,
    picker: &mut crate::app::state::ArtistRadioPickerState,
) -> Vec<Action> {
    match key.code {
        KeyCode::Esc => {
            if !picker.query.is_empty() {
                picker.query.clear();
                picker.focus = SearchFocus::Input;
                picker.item_index = 0;
                return vec![SearchAction::ArtistRadioPickerSearch.into()];
            }
            vec![SearchAction::CloseArtistRadioPicker.into()]
        }
        KeyCode::Enter => {
            match picker.focus {
                SearchFocus::Input => {
                    // Move to results if available
                    if !picker.filtered_artists.is_empty() {
                        picker.focus = SearchFocus::Results;
                        picker.item_index = 0;
                    }
                    vec![]
                }
                SearchFocus::Results => {
                    // Toggle selection without starting playback.
                    vec![SearchAction::ArtistRadioPickerToggleArtist.into()]
                }
            }
        }
        KeyCode::Down => {
            picker.scroll_pin = None;
            match picker.focus {
                SearchFocus::Input => {
                    if !picker.filtered_artists.is_empty() {
                        picker.focus = SearchFocus::Results;
                        picker.item_index = 0;
                    }
                    vec![]
                }
                SearchFocus::Results => {
                    let total = picker.filtered_artists.len();
                    if total > 0 && picker.item_index + 1 < total {
                        picker.item_index += 1;
                    }
                    vec![]
                }
            }
        }
        KeyCode::Up => {
            picker.scroll_pin = None;
            match picker.focus {
                SearchFocus::Input => vec![],
                SearchFocus::Results => {
                    if picker.item_index == 0 {
                        picker.focus = SearchFocus::Input;
                    } else {
                        picker.item_index -= 1;
                    }
                    vec![]
                }
            }
        }
        // Tab explicitly starts the selected artist mix.
        KeyCode::Tab => {
            if !picker.selected_artists.is_empty() {
                vec![SearchAction::ArtistRadioPickerLaunch.into()]
            } else {
                vec![]
            }
        }
        KeyCode::Backspace => {
            picker.query.pop();
            picker.focus = SearchFocus::Input;
            picker.item_index = 0;
            vec![SearchAction::ArtistRadioPickerSearch.into()]
        }
        KeyCode::Char(c) => {
            picker.query.push(c);
            picker.focus = SearchFocus::Input;
            picker.item_index = 0;
            vec![SearchAction::ArtistRadioPickerSearch.into()]
        }
        _ => vec![],
    }
}
