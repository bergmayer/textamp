//! Similar view key handling.

use crate::app::action::*;
use crossterm::event::{self, KeyCode};

use crate::app::Action;
use crate::app::AppState;
use crate::app::state::View;

/// Handle Similar view keys.
pub(in crate::app::handlers) fn handle_similar_keys(
    key: event::KeyEvent,
    state: &mut AppState,
) -> Vec<Action> {
    use crate::app::state::SimilarMode;

    match key.code {
        KeyCode::Esc => {
            // Return to previous view, or Browse if none
            let target = state.previous_view.take().unwrap_or(View::Browse);
            vec![NavigationAction::SetView(target).into()]
        }
        KeyCode::F(1) => vec![NavigationAction::SetView(View::Help).into()],

        KeyCode::Up => {
            state.scroll.similar = None;
            vec![DataAction::ListUp.into()]
        }
        KeyCode::Down => {
            state.scroll.similar = None;
            vec![DataAction::ListDown.into()]
        }
        KeyCode::PageUp => {
            state.scroll.similar = None;
            vec![DataAction::ListPageUp.into()]
        }
        KeyCode::PageDown => {
            state.scroll.similar = None;
            vec![DataAction::ListPageDown.into()]
        }
        KeyCode::Home => {
            state.scroll.similar = None;
            vec![DataAction::ListTop.into()]
        }
        KeyCode::End => {
            state.scroll.similar = None;
            vec![DataAction::ListBottom.into()]
        }

        KeyCode::Enter => activate_similar_item(state),

        KeyCode::Tab => {
            match state.similar.mode {
                SimilarMode::Tracks => {
                    // Tracks → Albums: use stored album key
                    if let Some(album_key) = state.similar.tab_album_key.clone() {
                        let title = state.similar.tab_album_title.clone().unwrap_or_default();
                        return vec![
                            DataAction::LoadSimilarAlbums {
                                rating_key: album_key,
                                title,
                            }
                            .into(),
                        ];
                    } else {
                        state.set_status("No album context for similar albums.".to_string());
                    }
                }
                SimilarMode::Albums => {
                    // Albums → Tracks: prefer stored original track, fall back to current track
                    if let Some(track_key) = state.similar.tab_track_key.clone() {
                        let title = state.similar.tab_track_title.clone().unwrap_or_default();
                        return vec![
                            DataAction::LoadSimilarTracks {
                                rating_key: track_key,
                                title,
                            }
                            .into(),
                        ];
                    } else if let Some(track) = state.current_track().cloned() {
                        let title = format!("{} - {}", track.artist_name(), track.title);
                        // Store track key for Tab back
                        state.similar.tab_track_key = Some(track.rating_key.clone());
                        state.similar.tab_track_title = Some(title.clone());
                        // Store album key for Tab back
                        state.similar.tab_album_key = track.parent_rating_key.clone();
                        state.similar.tab_album_title = Some(track.album_name().to_string());
                        return vec![
                            DataAction::LoadSimilarTracks {
                                rating_key: track.rating_key.clone(),
                                title,
                            }
                            .into(),
                        ];
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
                        a.title
                            .chars()
                            .next()
                            .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                            .unwrap_or(false)
                    }) {
                        state.list_state.similar_index = idx;
                    }
                }
                SimilarMode::Tracks => {
                    if let Some(idx) = state.similar.tracks.iter().position(|t| {
                        t.title
                            .chars()
                            .next()
                            .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                            .unwrap_or(false)
                    }) {
                        state.list_state.similar_index = idx;
                    }
                }
                SimilarMode::Artists => {
                    if let Some(idx) = state.similar.artists.iter().position(|a| {
                        a.title
                            .chars()
                            .next()
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
/// Albums: navigate to album in library (Miller columns). Tracks: open details without starting playback.
/// Artists: navigate to artist in Library (Miller columns).
pub(crate) fn activate_similar_item(state: &mut AppState) -> Vec<Action> {
    use crate::app::{library_details::Destination, state::SimilarMode};
    use crate::library::models::Track;
    let index = state.list_state.similar_index;
    let target = match state.similar.mode {
        SimilarMode::Tracks => state
            .similar
            .tracks
            .get(index)
            .cloned()
            .map(|track| (track, Destination::Track)),
        SimilarMode::Albums => state.similar.albums.get(index).map(|album| {
            (
                Track {
                    parent_rating_key: Some(album.rating_key.clone()),
                    parent_title: Some(album.title.clone()),
                    grandparent_rating_key: album.parent_rating_key.clone(),
                    grandparent_title: Some(album.artist_name().into()),
                    ..Default::default()
                },
                Destination::Album,
            )
        }),
        SimilarMode::Artists => state.similar.artists.get(index).map(|artist| {
            (
                Track {
                    grandparent_rating_key: Some(artist.rating_key.clone()),
                    grandparent_title: Some(artist.title.clone()),
                    ..Default::default()
                },
                Destination::Artist,
            )
        }),
    };
    target
        .map(|(track, destination)| {
            vec![
                BrowseAction::OpenDetailsInLibrary {
                    track: Box::new(track),
                    destination,
                }
                .into(),
            ]
        })
        .unwrap_or_default()
}
