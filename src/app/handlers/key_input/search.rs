//! Search popup key handling.

use crate::app::action::*;
use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::app::Action;
use crate::app::AppState;
use crate::app::state::{SearchFocus, SearchTab};

/// Handle search popup keys (Ctrl+F floating dialog).
pub(super) fn handle_search_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    if key.code == KeyCode::Right && state.search.focus == SearchFocus::Results {
        return crate::app::library_details::search_result_actions(state);
    }
    // Clear mouse scroll pin on any keyboard navigation
    state.scroll.search = None;
    if state.sources.active.folder().is_some()
        && matches!(
            key.code,
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right
        )
    {
        return vec![];
    }

    // Handle Ctrl+E / Ctrl+Shift+E before other keys
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && let KeyCode::Char('e') = key.code
    {
        if key.modifiers.contains(KeyModifiers::SHIFT) {
            // Ctrl+Shift+E: insert NEXT in queue after current track
            return vec![QueueAction::EnqueueSearchResultNext.into()];
        } else {
            // Ctrl+E: add to END of queue
            return vec![QueueAction::EnqueueSearchResult.into()];
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
                    if let Some(ref results) = state.search.results
                        && !results.is_empty()
                    {
                        state.search.focus = SearchFocus::Results;
                        state.list_state.search_item_index = 0;
                    }
                    if crate::app::handlers::dispatch_search::search_pending(state)
                        && state.search.results.is_none()
                    {
                        return vec![SearchAction::SubmitSearch.into()];
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
                    if let Some(ref results) = state.search.results
                        && !results.is_empty()
                    {
                        state.search.focus = SearchFocus::Results;
                        state.list_state.search_item_index = 0;
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
            vec![]
        }
        KeyCode::BackTab => {
            state.search.tab = state.search.tab.prev();
            state.search.focus = SearchFocus::Input;
            state.list_state.search_item_index = 0;
            vec![]
        }
        KeyCode::Left => {
            state.search.tab = state.search.tab.prev();
            state.search.focus = SearchFocus::Input;
            state.list_state.search_item_index = 0;
            vec![]
        }
        KeyCode::Right => {
            state.search.tab = state.search.tab.next();
            state.search.focus = SearchFocus::Input;
            state.list_state.search_item_index = 0;
            vec![]
        }
        KeyCode::Backspace => {
            let mut query = state.search.query.clone();
            query.pop();
            vec![SearchAction::SetSearchQuery(query).into()]
        }
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER) =>
        {
            let mut query = state.search.query.clone();
            query.push(c);
            vec![SearchAction::SetSearchQuery(query).into()]
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
            results.artists.len()
                + results.albums.len()
                + results.playlists.len()
                + results.genres.len()
                + results.tracks.len()
        }
        SearchTab::Artists => results.artists.len(),
        SearchTab::Albums => results.albums.len(),
        SearchTab::Playlists => results.playlists.len(),
        SearchTab::Tracks => results.tracks.len(),
        SearchTab::Genres => results.genres.len(),
    }
}
