//! Search popup key handling.

use crossterm::event::{self, KeyCode};

use crate::app::Action;
use crate::app::state::{SearchFocus, SearchTab};
use crate::app::AppState;

/// Handle search popup keys (Ctrl+F floating dialog).
pub(super) fn handle_search_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    // Clear mouse scroll pin on any keyboard navigation
    state.search_scroll_pin = None;

    match key.code {
        KeyCode::Esc => {
            state.search_query.clear();
            state.search_results = None;
            state.search_focus = SearchFocus::Input;
            vec![Action::CloseSearchPopup]
        }
        KeyCode::Enter => {
            match state.search_focus {
                SearchFocus::Input => {
                    // Move focus to results if we have any
                    if let Some(ref results) = state.search_results {
                        if !results.is_empty() {
                            state.search_focus = SearchFocus::Results;
                            state.list_state.search_item_index = 0;
                        }
                    }
                    vec![]
                }
                SearchFocus::Results => {
                    // Enter: navigate to the selected result in library
                    vec![Action::SelectSearchResult]
                }
            }
        }
        KeyCode::Down => {
            match state.search_focus {
                SearchFocus::Input => {
                    // Move focus to results if we have any
                    if let Some(ref results) = state.search_results {
                        if !results.is_empty() {
                            state.search_focus = SearchFocus::Results;
                            state.list_state.search_item_index = 0;
                        }
                    }
                    vec![]
                }
                SearchFocus::Results => {
                    // Navigate down in results
                    let total = search_result_count(state);
                    if total > 0 && state.list_state.search_item_index + 1 < total {
                        state.list_state.search_item_index += 1;
                    }
                    vec![]
                }
            }
        }
        KeyCode::Up => {
            match state.search_focus {
                SearchFocus::Input => vec![],
                SearchFocus::Results => {
                    if state.list_state.search_item_index == 0 {
                        // Back to input
                        state.search_focus = SearchFocus::Input;
                    } else {
                        state.list_state.search_item_index -= 1;
                    }
                    vec![]
                }
            }
        }
        KeyCode::Tab => {
            state.search_tab = state.search_tab.next();
            state.search_focus = SearchFocus::Input;
            state.list_state.search_item_index = 0;
            if !state.search_query.is_empty() {
                return vec![Action::ExecuteLocalSearch];
            }
            vec![]
        }
        KeyCode::BackTab => {
            state.search_tab = state.search_tab.prev();
            state.search_focus = SearchFocus::Input;
            state.list_state.search_item_index = 0;
            if !state.search_query.is_empty() {
                return vec![Action::ExecuteLocalSearch];
            }
            vec![]
        }
        KeyCode::Left => {
            state.search_tab = state.search_tab.prev();
            state.search_focus = SearchFocus::Input;
            state.list_state.search_item_index = 0;
            if !state.search_query.is_empty() {
                return vec![Action::ExecuteLocalSearch];
            }
            vec![]
        }
        KeyCode::Right => {
            state.search_tab = state.search_tab.next();
            state.search_focus = SearchFocus::Input;
            state.list_state.search_item_index = 0;
            if !state.search_query.is_empty() {
                return vec![Action::ExecuteLocalSearch];
            }
            vec![]
        }
        KeyCode::Backspace => {
            state.search_query.pop();
            state.search_focus = SearchFocus::Input;
            state.list_state.search_item_index = 0;
            if !state.search_query.is_empty() {
                vec![Action::ExecuteLocalSearch]
            } else {
                state.search_results = None;
                vec![]
            }
        }
        KeyCode::Char(c) => {
            state.search_query.push(c);
            state.search_focus = SearchFocus::Input;
            state.list_state.search_item_index = 0;
            vec![Action::ExecuteLocalSearch]
        }
        _ => vec![],
    }
}

/// Count total selectable items in current search results for the active tab.
fn search_result_count(state: &AppState) -> usize {
    let results = match &state.search_results {
        Some(r) => r,
        None => return 0,
    };

    match state.search_tab {
        SearchTab::Global => {
            // All tab: sum of all sections (section headers not counted)
            results.artists.len() + results.albums.len() + results.playlists.len()
                + results.genres.len() + results.tracks.len()
        }
        SearchTab::Artists => results.artists.len(),
        SearchTab::Albums => results.albums.len(),
        SearchTab::Playlists => results.playlists.len(),
        SearchTab::Tracks => results.tracks.len(),
        SearchTab::Genres => results.genres.len(),
    }
}
