//! Alt command availability — single source of truth for both the shortcut bar
//! display and the key handler dispatch.

use crate::app::state::{
    BrowseCategory, BrowseItem, Focus, PlaybackMode, RightPanelMode, View,
};
use crate::app::AppState;

/// An available Alt command shown in the shortcut bar.
#[derive(Debug, Clone)]
pub struct AltCommand {
    pub key: char,
    pub label: &'static str,
}

/// Returns the list of Alt commands available in the current state.
///
/// Used by both `render_shortcuts()` (to display the bar) and the key handler
/// (to gate dispatch), so the bar and behavior are always in sync.
pub fn available_alt_commands(state: &AppState) -> Vec<AltCommand> {
    let mut cmds = Vec::new();

    let has_track = has_track_context(state);
    let has_album = has_album_context(state);
    let has_artist_or_album_or_track = has_track || has_album || has_artist_context(state);
    let has_playing = state.current_track().is_some();

    // Alt+R radio: need a track, album, or artist in context, or something playing
    if has_artist_or_album_or_track || has_playing {
        cmds.push(AltCommand { key: 'r', label: "radio" });
    }

    // Alt+Q queue: need a track or album that can be enqueued
    if has_track || has_album || has_enqueue_context(state) {
        cmds.push(AltCommand { key: 'q', label: "queue" });
    }

    // Alt+S shuffle: in Library root column cycles sub-mode, otherwise shuffle column/queue
    if state.view == View::Browse && state.browse_category == BrowseCategory::Library
        && state.artist_nav.focused_column == 0
    {
        let label = match state.library_sub_mode {
            crate::app::state::LibrarySubMode::Normal => "all albums",
            crate::app::state::LibrarySubMode::AllByArtist => "shuffle albums",
            crate::app::state::LibrarySubMode::AllShuffled => "artists",
        };
        cmds.push(AltCommand { key: 's', label });
    } else if state.view == View::Browse
        || !state.queue.is_empty()
        || !state.radio.tracks.is_empty()
    {
        cmds.push(AltCommand { key: 's', label: "shuffle" });
    }

    // Alt+M similar: need a track or album in context, or something playing
    if has_track || has_album || has_playing {
        cmds.push(AltCommand { key: 'm', label: "similar" });
    }

    // Alt+B album: need a track with album info (Miller columns, folder, old state, or now-playing)
    if has_track_with_album(state) || has_miller_album_context(state)
        || has_folder_track_with_album(state) || has_playing_with_album(state)
    {
        cmds.push(AltCommand { key: 'b', label: "album" });
    }

    // Alt+G artist: need a track/album with artist info (Miller columns, folder, old state, or now-playing)
    if has_track_with_artist(state) || has_album_with_artist(state)
        || has_miller_artist_context(state) || has_folder_track_with_artist(state)
        || has_playing_with_artist(state)
    {
        cmds.push(AltCommand { key: 'g', label: "artist" });
    }

    // Alt+A adventure: library loaded
    if !state.artists.is_empty() {
        cmds.push(AltCommand { key: 'a', label: "adventure" });
    }

    // Alt+W save: has tracks in queue or radio (only in NowPlaying view)
    if state.view == View::NowPlaying
        && (!state.queue.is_empty() || !state.radio.tracks.is_empty())
    {
        cmds.push(AltCommand { key: 'w', label: "save" });
    }

    // Alt+C covers: available in Browse view except Folders (toggles album art grid)
    if state.view == View::Browse && state.browse_category != BrowseCategory::Folders {
        let label = if state.album_art_view { "list view" } else { "covers" };
        cmds.push(AltCommand { key: 'c', label });
    }

    cmds
}

/// Check if a command key is currently available.
pub fn is_alt_command_available(state: &AppState, key: char) -> bool {
    available_alt_commands(state).iter().any(|cmd| cmd.key == key)
}

// --- Context helpers ---

/// Is there a track highlighted/selected in the current view?
fn has_track_context(state: &AppState) -> bool {
    match state.view {
        View::NowPlaying => {
            let idx = state.list_state.queue_index;
            match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => {
                    let history_len = state.play_history.len();
                    idx < history_len || idx - history_len < state.queue.len()
                }
                PlaybackMode::Radio => idx < state.radio.tracks.len(),
            }
        }
        View::Browse => {
            matches!(
                state.right_panel_mode,
                RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks
            ) && state.list_state.tracks_index < state.selected_album_tracks.len()
        }
        View::Search => {
            // Search popup tracks: check if a track tab result is selected
            matches!(state.search_tab,
                crate::app::state::SearchTab::Tracks | crate::app::state::SearchTab::Global
            ) && state.search_results.as_ref()
                .map(|r| !r.tracks.is_empty())
                .unwrap_or(false)
        }
        _ => false,
    }
}

/// Is there an album highlighted in the current view?
fn has_album_context(state: &AppState) -> bool {
    match state.view {
        View::Browse => {
            // Check Miller columns first
            let nav = match state.browse_category {
                BrowseCategory::Library => Some(&state.artist_nav),
                BrowseCategory::Genres => Some(&state.genre_nav),
                BrowseCategory::Playlists => Some(&state.playlist_nav),
                _ => None,
            };
            if nav.and_then(|n| n.selected_item())
                .map(|i| matches!(i, BrowseItem::Album { .. }))
                .unwrap_or(false)
            {
                return true;
            }
            // Fall back to legacy right panel mode
            match state.right_panel_mode {
                RightPanelMode::ArtistAlbums => {
                    // Index 0 is "All Tracks", 1+ are albums
                    state.list_state.right_albums_index > 0
                        && state.list_state.right_albums_index <= state.selected_artist_albums.len()
                }
                RightPanelMode::CategoryAlbums => {
                    state.genre_albums_index < state.genre_albums.len()
                }
                _ => false,
            }
        }
        View::Similar => {
            !state.similar_albums.is_empty()
        }
        _ => false,
    }
}

/// Is there an artist highlighted in the current view?
fn has_artist_context(state: &AppState) -> bool {
    state.view == View::Browse
        && state.focus == Focus::Left
        && state.browse_category == BrowseCategory::Library
        && !state.artists.is_empty()
}

/// Does the enqueue action have valid context? (albums on left panel, playlists, etc.)
fn has_enqueue_context(state: &AppState) -> bool {
    match state.view {
        View::Browse => {
            // Left panel artist selected (enqueues all tracks)
            if state.focus == Focus::Left && state.browse_category == BrowseCategory::Library {
                return !state.artists.is_empty();
            }
            // Right panel with albums or tracks
            match state.right_panel_mode {
                RightPanelMode::ArtistAlbums => true,
                RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                    !state.selected_album_tracks.is_empty()
                }
                RightPanelMode::CategoryAlbums => !state.genre_albums.is_empty(),
                _ => false,
            }
        }
        View::NowPlaying => true, // Can always re-queue from now playing
        _ => false,
    }
}

/// Is there a selected track that has album info (parent_rating_key)?
fn has_track_with_album(state: &AppState) -> bool {
    match state.view {
        View::NowPlaying => {
            let idx = state.list_state.queue_index;
            let track = match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => {
                    let history_len = state.play_history.len();
                    if idx < history_len {
                        state.play_history.get(idx)
                    } else {
                        state.queue.get(idx - history_len)
                    }
                }
                PlaybackMode::Radio => state.radio.tracks.get(idx),
            };
            track.map(|t| t.parent_rating_key.is_some()).unwrap_or(false)
        }
        View::Browse => {
            match state.right_panel_mode {
                RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                    state.selected_album_tracks.get(state.list_state.tracks_index)
                        .map(|t| t.parent_rating_key.is_some())
                        .unwrap_or(false)
                }
                _ => false,
            }
        }
        _ => false,
    }
}

/// Is there a selected track that has artist info (grandparent_rating_key)?
fn has_track_with_artist(state: &AppState) -> bool {
    match state.view {
        View::NowPlaying => {
            let idx = state.list_state.queue_index;
            let track = match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => {
                    let history_len = state.play_history.len();
                    if idx < history_len {
                        state.play_history.get(idx)
                    } else {
                        state.queue.get(idx - history_len)
                    }
                }
                PlaybackMode::Radio => state.radio.tracks.get(idx),
            };
            track.map(|t| t.grandparent_rating_key.is_some()).unwrap_or(false)
        }
        View::Browse => {
            match state.right_panel_mode {
                RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                    state.selected_album_tracks.get(state.list_state.tracks_index)
                        .map(|t| t.grandparent_rating_key.is_some())
                        .unwrap_or(false)
                }
                _ => false,
            }
        }
        _ => false,
    }
}

/// Is there a selected album with artist info (parent_rating_key)?
fn has_album_with_artist(state: &AppState) -> bool {
    match state.view {
        View::Browse => {
            match state.right_panel_mode {
                RightPanelMode::ArtistAlbums => {
                    let idx = state.list_state.right_albums_index.saturating_sub(1);
                    state.list_state.right_albums_index > 0
                        && state.selected_artist_albums.get(idx)
                            .map(|a| a.parent_rating_key.is_some())
                            .unwrap_or(false)
                }
                RightPanelMode::CategoryAlbums => {
                    state.genre_albums.get(state.genre_albums_index)
                        .map(|a| a.parent_rating_key.is_some())
                        .unwrap_or(false)
                }
                _ => false,
            }
        }
        View::Similar => {
            state.similar_albums.get(state.list_state.similar_index)
                .map(|a| a.parent_rating_key.is_some())
                .unwrap_or(false)
        }
        _ => false,
    }
}

/// Is there a Track or Album selected in Miller columns that has album context?
fn has_miller_album_context(state: &AppState) -> bool {
    if state.view != View::Browse {
        return false;
    }
    let nav = match state.browse_category {
        BrowseCategory::Library => &state.artist_nav,
        BrowseCategory::Genres => &state.genre_nav,
        BrowseCategory::Playlists => &state.playlist_nav,
        _ => return false,
    };
    let focused = nav.focused_column;
    let item = nav.columns.get(focused).and_then(|c| c.items.get(c.selected_index));
    match item {
        Some(BrowseItem::Track { .. }) => {
            // Track needs a parent album column
            focused > 0 && nav.columns.get(focused - 1)
                .and_then(|c| c.items.get(c.selected_index))
                .map(|i| matches!(i, BrowseItem::Album { .. }))
                .unwrap_or(false)
        }
        Some(BrowseItem::Album { .. }) => true,
        _ => false,
    }
}

/// Is there a Track, Album, or Artist selected in Miller columns with artist context?
fn has_miller_artist_context(state: &AppState) -> bool {
    if state.view != View::Browse {
        return false;
    }
    let nav = match state.browse_category {
        BrowseCategory::Library => &state.artist_nav,
        BrowseCategory::Genres => &state.genre_nav,
        BrowseCategory::Playlists => &state.playlist_nav,
        _ => return false,
    };
    let focused = nav.focused_column;
    let item = nav.columns.get(focused).and_then(|c| c.items.get(c.selected_index));
    match item {
        Some(BrowseItem::Track { .. } | BrowseItem::Album { .. } | BrowseItem::AllTracks { .. }) => {
            // Need an Artist column somewhere in the hierarchy
            nav.columns.iter().any(|c| {
                c.items.get(c.selected_index)
                    .map(|i| matches!(i, BrowseItem::Artist { .. }))
                    .unwrap_or(false)
            })
        }
        Some(BrowseItem::Artist { .. }) => true,
        _ => false,
    }
}

/// Is there a selected track in folder view with album info (parent_rating_key)?
fn has_folder_track_with_album(state: &AppState) -> bool {
    if state.view != View::Browse || state.browse_category != BrowseCategory::Folders {
        return false;
    }
    state.folder_state.as_ref()
        .and_then(|fs| fs.selected_item())
        .map(|item| item.is_track() && item.parent_rating_key.is_some())
        .unwrap_or(false)
}

/// Is there a selected track in folder view with artist info (grandparent_rating_key)?
fn has_folder_track_with_artist(state: &AppState) -> bool {
    if state.view != View::Browse || state.browse_category != BrowseCategory::Folders {
        return false;
    }
    state.folder_state.as_ref()
        .and_then(|fs| fs.selected_item())
        .map(|item| item.is_track() && item.grandparent_rating_key.is_some())
        .unwrap_or(false)
}

/// Does the now-playing track have album info?
fn has_playing_with_album(state: &AppState) -> bool {
    state.current_track()
        .map(|t| t.parent_rating_key.is_some() && t.grandparent_rating_key.is_some())
        .unwrap_or(false)
}

/// Does the now-playing track have artist info?
fn has_playing_with_artist(state: &AppState) -> bool {
    state.current_track()
        .map(|t| t.grandparent_rating_key.is_some())
        .unwrap_or(false)
}
