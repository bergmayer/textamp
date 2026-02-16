//! Adventure launcher popup key handling.

use crossterm::event::{self, KeyCode};

use crate::app::Action;
use crate::app::state::{AdventureDrillLevel, AdventureStep, SearchFocus, SearchTab};
use crate::app::AppState;

/// Handle adventure launcher popup keys.
pub(super) fn handle_adventure_launcher_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    let launcher = match state.adventure_launcher.as_mut() {
        Some(l) => l,
        None => return vec![],
    };

    match launcher.step {
        AdventureStep::FindStartTrack | AdventureStep::FindEndTrack => {
            handle_track_finder_keys(key, state)
        }
        AdventureStep::EnterTrackCount => {
            handle_track_count_keys(key, state)
        }
    }
}

/// Handle keys for step 1/3 (find start/end track with search + drill-down).
fn handle_track_finder_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    let launcher = match state.adventure_launcher.as_mut() {
        Some(l) => l,
        None => return vec![],
    };

    match key.code {
        KeyCode::Esc => {
            vec![Action::AdventureLauncherBack]
        }
        KeyCode::Left => {
            // Left arrow: same as Esc in drill levels, ignored in Search
            match &launcher.drill {
                AdventureDrillLevel::ArtistAlbums { .. } | AdventureDrillLevel::AlbumTracks { .. } => {
                    vec![Action::AdventureLauncherBack]
                }
                AdventureDrillLevel::Search => vec![],
            }
        }
        KeyCode::Enter => {
            match launcher.focus {
                SearchFocus::Input => {
                    // Move focus to results if we have any
                    let count = result_count(launcher);
                    if count > 0 {
                        launcher.focus = SearchFocus::Results;
                        launcher.item_index = 0;
                    }
                    vec![]
                }
                SearchFocus::Results => {
                    handle_enter_on_result(launcher)
                }
            }
        }
        KeyCode::Down => {
            launcher.scroll_pin = None;
            match launcher.focus {
                SearchFocus::Input => {
                    let count = result_count(launcher);
                    if count > 0 {
                        launcher.focus = SearchFocus::Results;
                        launcher.item_index = 0;
                    }
                    vec![]
                }
                SearchFocus::Results => {
                    let total = result_count(launcher);
                    if total > 0 && launcher.item_index + 1 < total {
                        launcher.item_index += 1;
                    }
                    vec![]
                }
            }
        }
        KeyCode::Up => {
            launcher.scroll_pin = None;
            match launcher.focus {
                SearchFocus::Input => vec![],
                SearchFocus::Results => {
                    if launcher.item_index == 0 {
                        // Only go back to input in search mode
                        if matches!(launcher.drill, AdventureDrillLevel::Search) {
                            launcher.focus = SearchFocus::Input;
                        }
                    } else {
                        launcher.item_index -= 1;
                    }
                    vec![]
                }
            }
        }
        KeyCode::Tab => {
            if matches!(launcher.drill, AdventureDrillLevel::Search) {
                launcher.search_tab = launcher.search_tab.next();
                launcher.item_index = 0;
                launcher.scroll_pin = None;
            }
            vec![]
        }
        KeyCode::BackTab => {
            if matches!(launcher.drill, AdventureDrillLevel::Search) {
                launcher.search_tab = launcher.search_tab.prev();
                launcher.item_index = 0;
                launcher.scroll_pin = None;
            }
            vec![]
        }
        KeyCode::Backspace => {
            match &launcher.drill {
                AdventureDrillLevel::Search => {
                    launcher.query.pop();
                    launcher.focus = SearchFocus::Input;
                    launcher.item_index = 0;
                    if !launcher.query.is_empty() {
                        vec![Action::AdventureLauncherSearch]
                    } else {
                        launcher.results = None;
                        vec![]
                    }
                }
                _ => {
                    // In drill mode, backspace goes back
                    vec![Action::AdventureLauncherBack]
                }
            }
        }
        KeyCode::Char(c) => {
            match &launcher.drill {
                AdventureDrillLevel::Search => {
                    launcher.query.push(c);
                    launcher.focus = SearchFocus::Input;
                    launcher.item_index = 0;
                    vec![Action::AdventureLauncherSearch]
                }
                _ => vec![], // No typing in drill mode
            }
        }
        _ => vec![],
    }
}

/// Handle Enter on a result item — drill or select depending on type and tab.
fn handle_enter_on_result(launcher: &mut crate::app::state::AdventureLauncherState) -> Vec<Action> {
    match &launcher.drill {
        AdventureDrillLevel::Search => {
            if let Some(ref results) = launcher.results {
                let idx = launcher.item_index;

                // Tab-aware Enter behavior
                match launcher.search_tab {
                    SearchTab::Artists => {
                        if let Some(artist) = results.artists.get(idx) {
                            return vec![Action::AdventureLauncherDrillArtist {
                                key: artist.rating_key.clone(),
                                name: artist.title.clone(),
                            }];
                        }
                    }
                    SearchTab::Albums => {
                        if let Some(album) = results.albums.get(idx) {
                            return vec![Action::AdventureLauncherDrillAlbum {
                                key: album.rating_key.clone(),
                                title: album.title.clone(),
                                artist_name: album.artist_name().to_string(),
                            }];
                        }
                    }
                    SearchTab::Tracks => {
                        return vec![Action::AdventureLauncherSelectTrack];
                    }
                    SearchTab::Playlists | SearchTab::Genres => {
                        // Not actionable in adventure mode
                        return vec![];
                    }
                    SearchTab::Global => {
                        // Global tab: determine type by index offset
                        let artist_count = results.artists.len();
                        let album_count = results.albums.len();

                        if idx < artist_count {
                            let artist = &results.artists[idx];
                            return vec![Action::AdventureLauncherDrillArtist {
                                key: artist.rating_key.clone(),
                                name: artist.title.clone(),
                            }];
                        } else if idx < artist_count + album_count {
                            let album = &results.albums[idx - artist_count];
                            return vec![Action::AdventureLauncherDrillAlbum {
                                key: album.rating_key.clone(),
                                title: album.title.clone(),
                                artist_name: album.artist_name().to_string(),
                            }];
                        } else {
                            return vec![Action::AdventureLauncherSelectTrack];
                        }
                    }
                }
            }
            vec![]
        }
        AdventureDrillLevel::ArtistAlbums { albums, artist_name, .. } => {
            if let Some(album) = albums.get(launcher.item_index) {
                vec![Action::AdventureLauncherDrillAlbum {
                    key: album.rating_key.clone(),
                    title: album.title.clone(),
                    artist_name: artist_name.clone(),
                }]
            } else {
                vec![]
            }
        }
        AdventureDrillLevel::AlbumTracks { .. } => {
            vec![Action::AdventureLauncherSelectTrack]
        }
    }
}

/// Handle keys for step 2 (enter track count).
fn handle_track_count_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    let launcher = match state.adventure_launcher.as_mut() {
        Some(l) => l,
        None => return vec![],
    };

    match key.code {
        KeyCode::Esc => {
            // Go back to FindStartTrack
            vec![Action::AdventureLauncherBack]
        }
        KeyCode::Enter => {
            // Parse count and advance to FindEndTrack
            let count = launcher.track_count_input.parse::<usize>().unwrap_or(20).clamp(5, 100);
            launcher.track_count_input = count.to_string();
            launcher.step = AdventureStep::FindEndTrack;
            launcher.query.clear();
            launcher.drill = AdventureDrillLevel::Search;
            launcher.item_index = 0;
            launcher.focus = SearchFocus::Input;
            launcher.results = None;
            launcher.search_tab = SearchTab::default();
            vec![]
        }
        KeyCode::Backspace => {
            launcher.track_count_input.pop();
            vec![]
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            if launcher.track_count_input.len() < 3 {
                launcher.track_count_input.push(c);
            }
            vec![]
        }
        _ => vec![],
    }
}

/// Count total selectable items for the current drill level (tab-aware).
fn result_count(launcher: &crate::app::state::AdventureLauncherState) -> usize {
    match &launcher.drill {
        AdventureDrillLevel::Search => {
            if let Some(ref results) = launcher.results {
                match launcher.search_tab {
                    SearchTab::Global => results.artists.len() + results.albums.len() + results.tracks.len(),
                    SearchTab::Artists => results.artists.len(),
                    SearchTab::Albums => results.albums.len(),
                    SearchTab::Tracks => results.tracks.len(),
                    SearchTab::Playlists => results.playlists.len(),
                    SearchTab::Genres => results.genres.len(),
                }
            } else {
                0
            }
        }
        AdventureDrillLevel::ArtistAlbums { albums, .. } => albums.len(),
        AdventureDrillLevel::AlbumTracks { tracks, .. } => tracks.len(),
    }
}
