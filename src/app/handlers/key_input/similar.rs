//! Similar view key handling.

use crossterm::event::{self, KeyCode};

use crate::app::Action;
use crate::app::state::View;
use crate::app::AppState;

/// Handle Similar view keys.
pub(in crate::app::handlers) fn handle_similar_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    use crate::app::state::SimilarMode;

    match key.code {
        KeyCode::Esc => {
            // Return to previous view, or Browse if none
            let target = state.previous_view.take().unwrap_or(View::Browse);
            vec![Action::SetView(target)]
        }
        KeyCode::F(1) | KeyCode::Char('?') => vec![Action::SetView(View::Help)],

        KeyCode::Up => { state.scroll.similar = None; vec![Action::ListUp] }
        KeyCode::Down => { state.scroll.similar = None; vec![Action::ListDown] }
        KeyCode::PageUp => { state.scroll.similar = None; vec![Action::ListPageUp] }
        KeyCode::PageDown => { state.scroll.similar = None; vec![Action::ListPageDown] }
        KeyCode::Home => { state.scroll.similar = None; vec![Action::ListTop] }
        KeyCode::End => { state.scroll.similar = None; vec![Action::ListBottom] }

        KeyCode::Enter => activate_similar_item(state),

        KeyCode::Tab => {
            match state.similar.mode {
                SimilarMode::Tracks => {
                    // Tracks → Albums: use stored album key
                    if let Some(album_key) = state.similar.tab_album_key.clone() {
                        let title = state.similar.tab_album_title.clone().unwrap_or_default();
                        return vec![Action::LoadSimilarAlbums {
                            rating_key: album_key,
                            title,
                        }];
                    } else {
                        state.set_status("No album context for similar albums.".to_string());
                    }
                }
                SimilarMode::Albums => {
                    // Albums → Tracks: prefer stored original track, fall back to current track
                    if let Some(track_key) = state.similar.tab_track_key.clone() {
                        let title = state.similar.tab_track_title.clone().unwrap_or_default();
                        return vec![Action::LoadSimilarTracks {
                            rating_key: track_key,
                            title,
                        }];
                    } else if let Some(track) = state.current_track().cloned() {
                        let title = format!("{} - {}", track.artist_name(), track.title);
                        // Store track key for Tab back
                        state.similar.tab_track_key = Some(track.rating_key.clone());
                        state.similar.tab_track_title = Some(title.clone());
                        // Store album key for Tab back
                        state.similar.tab_album_key = track.parent_rating_key.clone();
                        state.similar.tab_album_title = Some(track.album_name().to_string());
                        return vec![Action::LoadSimilarTracks {
                            rating_key: track.rating_key.clone(),
                            title,
                        }];
                    } else {
                        state.set_status("No track playing.".to_string());
                    }
                }
                SimilarMode::Artists => {
                    state.set_status("No album/track context.".to_string());
                }
            }
            vec![]
        }

        // Alphabet jumping
        KeyCode::Char(c) if c.is_ascii_alphabetic() && key.modifiers.is_empty() => {
            let letter_lower = c.to_ascii_lowercase();
            match state.similar.mode {
                SimilarMode::Albums => {
                    if let Some(idx) = state.similar.albums.iter().position(|a| {
                        a.title.chars().next()
                            .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                            .unwrap_or(false)
                    }) {
                        state.list_state.similar_index = idx;
                    }
                }
                SimilarMode::Tracks => {
                    if let Some(idx) = state.similar.tracks.iter().position(|t| {
                        t.title.chars().next()
                            .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                            .unwrap_or(false)
                    }) {
                        state.list_state.similar_index = idx;
                    }
                }
                SimilarMode::Artists => {
                    if let Some(idx) = state.similar.artists.iter().position(|a| {
                        a.title.chars().next()
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

/// Activate the currently highlighted similar item (Enter or second click).
/// Albums: navigate to album in library (Miller columns). Tracks: play the track.
/// Artists: navigate to artist in Library (Miller columns).
pub(in crate::app::handlers) fn activate_similar_item(state: &mut AppState) -> Vec<Action> {
    use crate::app::state::SimilarMode;

    let idx = state.list_state.similar_index;
    match state.similar.mode {
        SimilarMode::Albums => {
            if let Some(album) = state.similar.albums.get(idx).cloned() {
                state.pending_album_key = Some(album.rating_key.clone());
                state.selected_album_title = album.title.clone();
                state.selected_artist_name = album.artist_name().to_string();
                state.set_view(View::Browse);
                state.browse_category = crate::app::state::BrowseCategory::Library;
                if let Some(ref artist_key) = album.parent_rating_key {
                    // Select artist in Miller column 0
                    if let Some(pos) = state.artist_nav.columns.first()
                        .and_then(|col| col.items.iter().position(|i| i.key() == artist_key.as_str()))
                    {
                        if let Some(col) = state.artist_nav.columns.first_mut() {
                            col.selected_index = pos;
                        }
                    }
                    state.artist_nav.focused_column = 0;
                    state.artist_nav.truncate_right();
                    return vec![Action::LoadArtistAlbumsForMiller { artist_key: artist_key.clone() }];
                }
                // No parent artist key — try All Artists
                if let Some(col) = state.artist_nav.columns.first_mut() {
                    col.selected_index = 0;
                }
                state.artist_nav.focused_column = 0;
                state.artist_nav.truncate_right();
                vec![Action::LoadAllAlbumsForMiller]
            } else {
                vec![]
            }
        }
        SimilarMode::Tracks => {
            if let Some(track) = state.similar.tracks.get(idx).cloned() {
                vec![Action::PlayTrack(track)]
            } else {
                vec![]
            }
        }
        SimilarMode::Artists => {
            if let Some(artist) = state.similar.artists.get(idx).cloned() {
                let artist_key = artist.rating_key.clone();
                state.set_view(View::Browse);
                state.browse_category = crate::app::state::BrowseCategory::Library;
                // Select artist in Miller column 0
                if let Some(pos) = state.artist_nav.columns.first()
                    .and_then(|col| col.items.iter().position(|i| i.key() == artist_key.as_str()))
                {
                    if let Some(col) = state.artist_nav.columns.first_mut() {
                        col.selected_index = pos;
                    }
                }
                state.artist_nav.focused_column = 0;
                state.artist_nav.truncate_right();
                vec![Action::LoadArtistAlbumsForMiller { artist_key }]
            } else {
                vec![]
            }
        }
    }
}
