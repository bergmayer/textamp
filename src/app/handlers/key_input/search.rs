//! Search popup key handling.

use crate::app::action::*;
use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::app::Action;
use crate::app::state::{SearchFocus, SearchTab};
use crate::app::AppState;

/// Handle search popup keys (Ctrl+F floating dialog).
pub(super) fn handle_search_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    // Clear mouse scroll pin on any keyboard navigation
    state.scroll.search = None;

    // Handle Ctrl+E / Ctrl+Shift+E before other keys
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('e') => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    // Ctrl+Shift+E: insert NEXT in queue after current track
                    return vec![QueueAction::EnqueueSearchResultNext.into()];
                } else {
                    // Ctrl+E: add to END of queue
                    return vec![QueueAction::EnqueueSearchResult.into()];
                }
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Esc => {
            state.search.query.clear();
            state.search.results = None;
            state.search.focus = SearchFocus::Input;
            vec![SearchAction::CloseSearchPopup.into()]
        }
        KeyCode::Enter => {
            match state.search.focus {
                SearchFocus::Input => {
                    // Move focus to results if we have any
                    if let Some(ref results) = state.search.results {
                        if !results.is_empty() {
                            state.search.focus = SearchFocus::Results;
                            state.list_state.search_item_index = 0;
                        }
                    }
                    vec![]
                }
                SearchFocus::Results => {
                    // Enter: navigate to the selected result in library
                    vec![SearchAction::SelectSearchResult.into()]
                }
            }
        }
        KeyCode::Down => {
            match state.search.focus {
                SearchFocus::Input => {
                    // Move focus to results if we have any
                    if let Some(ref results) = state.search.results {
                        if !results.is_empty() {
                            state.search.focus = SearchFocus::Results;
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
            match state.search.focus {
                SearchFocus::Input => vec![],
                SearchFocus::Results => {
                    if state.list_state.search_item_index == 0 {
                        // Back to input
                        state.search.focus = SearchFocus::Input;
                    } else {
                        state.list_state.search_item_index -= 1;
                    }
                    vec![]
                }
            }
        }
        KeyCode::Tab => {
            state.search.tab = state.search.tab.next();
            state.search.focus = SearchFocus::Input;
            state.list_state.search_item_index = 0;
            if !state.search.query.is_empty() {
                return vec![SearchAction::ExecuteLocalSearch.into()];
            }
            vec![]
        }
        KeyCode::BackTab => {
            state.search.tab = state.search.tab.prev();
            state.search.focus = SearchFocus::Input;
            state.list_state.search_item_index = 0;
            if !state.search.query.is_empty() {
                return vec![SearchAction::ExecuteLocalSearch.into()];
            }
            vec![]
        }
        KeyCode::Left => {
            state.search.tab = state.search.tab.prev();
            state.search.focus = SearchFocus::Input;
            state.list_state.search_item_index = 0;
            if !state.search.query.is_empty() {
                return vec![SearchAction::ExecuteLocalSearch.into()];
            }
            vec![]
        }
        KeyCode::Right => {
            state.search.tab = state.search.tab.next();
            state.search.focus = SearchFocus::Input;
            state.list_state.search_item_index = 0;
            if !state.search.query.is_empty() {
                return vec![SearchAction::ExecuteLocalSearch.into()];
            }
            vec![]
        }
        KeyCode::Backspace => {
            state.search.query.pop();
            state.search.focus = SearchFocus::Input;
            state.list_state.search_item_index = 0;
            if !state.search.query.is_empty() {
                vec![SearchAction::ExecuteLocalSearch.into()]
            } else {
                state.search.results = None;
                vec![]
            }
        }
        KeyCode::Char(c) => {
            state.search.query.push(c);
            state.search.focus = SearchFocus::Input;
            state.list_state.search_item_index = 0;
            vec![SearchAction::ExecuteLocalSearch.into()]
        }
        _ => vec![],
    }
}

/// Count total selectable items in current search results for the active tab.
fn search_result_count(state: &AppState) -> usize {
    let results = match &state.search.results {
        Some(r) => r,
        None => return 0,
    };

    match state.search.tab {
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
