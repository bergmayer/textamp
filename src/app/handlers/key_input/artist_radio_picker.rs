//! Multi-artist radio picker popup key handling.

use crossterm::event::{self, KeyCode};

use crate::app::Action;
use crate::app::state::{ArtistRadioPickerStep, SearchFocus};
use crate::app::AppState;

/// Handle artist radio picker popup keys.
pub(super) fn handle_artist_radio_picker_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    let picker = match state.popups.artist_radio_picker.as_mut() {
        Some(p) => p,
        None => return vec![],
    };

    match picker.step {
        ArtistRadioPickerStep::EnterCount => handle_count_step(key, picker),
        ArtistRadioPickerStep::SelectArtists => handle_select_step(key, picker),
    }
}

/// Handle keys in the EnterCount step: type a number (2-12), Enter to proceed.
fn handle_count_step(key: event::KeyEvent, picker: &mut crate::app::state::ArtistRadioPickerState) -> Vec<Action> {
    match key.code {
        KeyCode::Esc => {
            vec![Action::CloseArtistRadioPicker]
        }
        KeyCode::Enter => {
            let count = picker.count_input.parse::<usize>().unwrap_or(0);
            if count >= 1 && count <= 12 {
                vec![Action::ArtistRadioPickerSetCount]
            } else {
                vec![]
            }
        }
        KeyCode::Backspace => {
            picker.count_input.pop();
            vec![]
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            if picker.count_input.len() < 2 {
                picker.count_input.push(c);
            }
            vec![]
        }
        _ => vec![],
    }
}

/// Handle keys in the SelectArtists step: search, navigate, toggle selection.
fn handle_select_step(key: event::KeyEvent, picker: &mut crate::app::state::ArtistRadioPickerState) -> Vec<Action> {
    match key.code {
        KeyCode::Esc => {
            if !picker.query.is_empty() {
                picker.query.clear();
                picker.focus = SearchFocus::Input;
                picker.item_index = 0;
                return vec![Action::ArtistRadioPickerSearch];
            }
            vec![Action::CloseArtistRadioPicker]
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
                    // Toggle artist selection, then auto-launch if max reached
                    vec![Action::ArtistRadioPickerToggleArtist]
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
        // Tab launches when enough artists selected
        KeyCode::Tab => {
            if picker.selected_artists.len() == picker.max_artists {
                vec![Action::ArtistRadioPickerLaunch]
            } else {
                vec![]
            }
        }
        KeyCode::Backspace => {
            picker.query.pop();
            picker.focus = SearchFocus::Input;
            picker.item_index = 0;
            vec![Action::ArtistRadioPickerSearch]
        }
        KeyCode::Char(c) => {
            picker.query.push(c);
            picker.focus = SearchFocus::Input;
            picker.item_index = 0;
            vec![Action::ArtistRadioPickerSearch]
        }
        _ => vec![],
    }
}
