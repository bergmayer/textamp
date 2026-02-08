//! Search view key handling.

use crossterm::event::{self, KeyCode};

use crate::app::Action;
use crate::app::state::{BrowseCategory, SearchSection, SearchTab, View};
use crate::app::AppState;

/// Handle unified Search view keys (with tabs for Global/Artists/Playlists/Tracks/Genres).
pub(super) fn handle_search_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    match key.code {
        KeyCode::Esc => {
            state.search_query.clear();
            state.search_results = None;
            state.filter_results = None;
            // Close popup if active, otherwise return to Browse view
            if state.search_popup_active {
                vec![Action::CloseSearchPopup]
            } else {
                vec![Action::SetView(View::Browse)]
            }
        }
        KeyCode::Enter => {
            match state.search_tab {
                SearchTab::Global => {
                    if state.search_results.is_some() {
                        select_search_result(state)
                    } else if !state.search_query.is_empty() && !state.search_loading {
                        // Only trigger new search if not already loading
                        // (avoids discarding pending search results)
                        vec![Action::ExecuteSearch]
                    } else {
                        vec![]  // Wait for pending search to complete
                    }
                }
                _ => {
                    // Filter tabs - select filter result (only if not loading)
                    if !state.filter_loading {
                        vec![Action::SelectFilterResult]
                    } else {
                        vec![]  // Wait for pending filter to complete
                    }
                }
            }
        }
        KeyCode::Backspace => {
            state.search_query.pop();
            state.list_state.search_item_index = 0;
            // Clear old results when modifying query
            state.search_results = None;
            state.filter_results = None;
            // Trigger search for all tabs (requires 2+ chars)
            if state.search_query.len() >= 2 {
                match state.search_tab {
                    SearchTab::Global => vec![Action::ExecuteSearch],
                    _ => vec![Action::ExecuteFilterSearch],
                }
            } else {
                vec![]
            }
        }
        KeyCode::Up => {
            match state.search_tab {
                SearchTab::Global => {
                    navigate_search_results(state, -1);
                    vec![]
                }
                _ => vec![Action::ListUp],
            }
        }
        KeyCode::Down => {
            match state.search_tab {
                SearchTab::Global => {
                    navigate_search_results(state, 1);
                    vec![]
                }
                _ => vec![Action::ListDown],
            }
        }
        KeyCode::Tab => {
            // Tab always switches between search tabs
            state.search_tab = state.search_tab.next();
            state.list_state.search_item_index = 0;
            state.list_state.search_section = SearchSection::Artists;
            // Trigger appropriate search for new tab if we have a query
            if !state.search_query.is_empty() {
                if state.search_tab == SearchTab::Global {
                    return vec![Action::ExecuteSearch];
                } else {
                    return vec![Action::ExecuteFilterSearch];
                }
            }
            vec![]
        }
        KeyCode::BackTab => {
            // Shift+Tab switches to previous tab
            state.search_tab = state.search_tab.prev();
            state.list_state.search_item_index = 0;
            state.list_state.search_section = SearchSection::Artists;
            if !state.search_query.is_empty() {
                if state.search_tab == SearchTab::Global {
                    return vec![Action::ExecuteSearch];
                } else {
                    return vec![Action::ExecuteFilterSearch];
                }
            }
            vec![]
        }
        KeyCode::Left => {
            // Left arrow switches sections within Global search results
            if state.search_tab == SearchTab::Global && state.search_results.is_some() {
                next_search_section(state, -1);
            }
            vec![]
        }
        KeyCode::Right => {
            // Right arrow switches sections within Global search results
            if state.search_tab == SearchTab::Global && state.search_results.is_some() {
                next_search_section(state, 1);
            }
            vec![]
        }
        KeyCode::Char(c) => {
            state.search_query.push(c);
            state.list_state.search_item_index = 0;
            // Clear old results when typing new query
            state.search_results = None;
            state.filter_results = None;
            // Trigger search for all tabs (requires 2+ chars)
            if state.search_query.len() >= 2 {
                match state.search_tab {
                    SearchTab::Global => vec![Action::ExecuteSearch],
                    _ => vec![Action::ExecuteFilterSearch],
                }
            } else {
                vec![]
            }
        }
        _ => vec![],
    }
}

fn select_search_result(state: &mut AppState) -> Vec<Action> {
    if let Some(results) = &state.search_results {
        let section = state.list_state.search_section;
        let idx = state.list_state.search_item_index;

        match section {
            SearchSection::Artists => {
                if let Some(artist) = results.artists.get(idx).cloned() {
                    // Store artist info for loading albums
                    state.selected_artist_name = artist.title.clone();
                    state.pending_filter_key = Some(artist.rating_key.clone());
                    // Set category directly - LoadArtistAlbums will load artists if needed
                    state.browse_category = BrowseCategory::Artists;
                    state.search_query.clear();
                    state.search_results = None;
                    state.view = View::Browse;
                    state.search_popup_active = false; // Close popup
                    return vec![Action::LoadArtistAlbums];
                }
            }
            SearchSection::Albums => {
                if let Some(album) = results.albums.get(idx).cloned() {
                    // Play album - close popup after playing
                    state.search_popup_active = false;
                    return vec![Action::PlayAlbum { rating_key: album.rating_key.clone() }];
                }
            }
            SearchSection::Tracks => {
                if let Some(track) = results.tracks.get(idx).cloned() {
                    // Play track - close popup after playing
                    state.search_popup_active = false;
                    return vec![Action::PlayTrack(track)];
                }
            }
        }
    }
    vec![]
}

fn navigate_search_results(state: &mut AppState, delta: i32) {
    if let Some(results) = &state.search_results {
        let section = state.list_state.search_section;
        let idx = state.list_state.search_item_index as i32;

        let section_len = match section {
            SearchSection::Artists => results.artists.len(),
            SearchSection::Albums => results.albums.len(),
            SearchSection::Tracks => results.tracks.len(),
        };

        if section_len == 0 {
            return;
        }

        let new_idx = idx + delta;

        if new_idx < 0 {
            next_search_section(state, -1);
            if let Some(results) = &state.search_results {
                let new_len = match state.list_state.search_section {
                    SearchSection::Artists => results.artists.len(),
                    SearchSection::Albums => results.albums.len(),
                    SearchSection::Tracks => results.tracks.len(),
                };
                state.list_state.search_item_index = new_len.saturating_sub(1);
            }
        } else if new_idx >= section_len as i32 {
            next_search_section(state, 1);
            state.list_state.search_item_index = 0;
        } else {
            state.list_state.search_item_index = new_idx as usize;
        }
    }
}

fn next_search_section(state: &mut AppState, direction: i32) {
    if let Some(results) = &state.search_results {
        let sections: Vec<SearchSection> = [
            (!results.artists.is_empty(), SearchSection::Artists),
            (!results.albums.is_empty(), SearchSection::Albums),
            (!results.tracks.is_empty(), SearchSection::Tracks),
        ]
        .iter()
        .filter(|(has_items, _)| *has_items)
        .map(|(_, section)| *section)
        .collect();

        if sections.is_empty() {
            return;
        }

        let current_idx = sections
            .iter()
            .position(|s| *s == state.list_state.search_section)
            .unwrap_or(0);

        let new_idx = if direction > 0 {
            (current_idx + 1) % sections.len()
        } else if current_idx == 0 {
            sections.len() - 1
        } else {
            current_idx - 1
        };

        state.list_state.search_section = sections[new_idx];
        state.list_state.search_item_index = 0;
    }
}
