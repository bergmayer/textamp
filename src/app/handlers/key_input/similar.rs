//! Similar view key handling.

use crossterm::event::{self, KeyCode};

use crate::app::Action;
use crate::app::state::View;
use crate::app::AppState;

/// Handle Similar view keys.
pub(super) fn handle_similar_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    use crate::app::state::SimilarMode;

    match key.code {
        KeyCode::Esc => {
            // Return to previous view, or Browse if none
            let target = state.previous_view.take().unwrap_or(View::Browse);
            vec![Action::SetView(target)]
        }
        KeyCode::F(1) | KeyCode::Char('?') => vec![Action::SetView(View::Help)],

        KeyCode::Up => { state.similar_scroll_pin = None; vec![Action::ListUp] }
        KeyCode::Down => { state.similar_scroll_pin = None; vec![Action::ListDown] }
        KeyCode::PageUp => { state.similar_scroll_pin = None; vec![Action::ListPageUp] }
        KeyCode::PageDown => { state.similar_scroll_pin = None; vec![Action::ListPageDown] }
        KeyCode::Home => { state.similar_scroll_pin = None; vec![Action::ListTop] }
        KeyCode::End => { state.similar_scroll_pin = None; vec![Action::ListBottom] }

        KeyCode::Enter => {
            let idx = state.list_state.similar_index;
            match state.similar_mode {
                SimilarMode::Albums => {
                    // Enter: navigate to album in library
                    if let Some(album) = state.similar_albums.get(idx).cloned() {
                        state.pending_album_key = Some(album.rating_key.clone());
                        state.selected_album_title = album.title.clone();
                        state.selected_artist_name = album.artist_name().to_string();
                        state.set_view(View::Browse);
                        state.browse_category = crate::app::state::BrowseCategory::Library;
                        if let Some(artist_key) = &album.parent_rating_key {
                            if let Some(idx) = state.artists.iter().position(|a| &a.rating_key == artist_key) {
                                state.list_state.artists_index = idx;
                            }
                        }
                        vec![Action::LoadArtistAlbums]
                    } else {
                        vec![]
                    }
                }
                SimilarMode::Tracks => {
                    // Enter: play single track
                    if let Some(track) = state.similar_tracks.get(idx).cloned() {
                        vec![Action::PlayTrack(track)]
                    } else {
                        vec![]
                    }
                }
            }
        }

        // Alphabet jumping
        KeyCode::Char(c) if c.is_ascii_alphabetic() && key.modifiers.is_empty() => {
            let letter_lower = c.to_ascii_lowercase();
            match state.similar_mode {
                SimilarMode::Albums => {
                    if let Some(idx) = state.similar_albums.iter().position(|a| {
                        a.title.chars().next()
                            .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                            .unwrap_or(false)
                    }) {
                        state.list_state.similar_index = idx;
                    }
                }
                SimilarMode::Tracks => {
                    if let Some(idx) = state.similar_tracks.iter().position(|t| {
                        t.title.chars().next()
                            .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                            .unwrap_or(false)
                    }) {
                        state.list_state.similar_index = idx;
                    }
                }
            }
            vec![]
        }

        _ => vec![],
    }
}
