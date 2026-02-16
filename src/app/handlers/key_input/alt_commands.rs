//! Contextual command availability — single source of truth for both the shortcut bar
//! display and the key handler dispatch.

use crate::app::state::{
    BrowseCategory, BrowseItem, Focus, PlaybackMode, PlaylistViewMode,
    RightPanelMode, View,
};
use crate::app::AppState;

/// Modifier key for a shortcut command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandModifier {
    Ctrl,
    Alt,
    None,
}

/// An available command shown in the shortcut bar.
#[derive(Debug, Clone)]
pub struct AltCommand {
    pub modifier: CommandModifier,
    pub key: char,
    pub label: &'static str,
    /// Display string for the key (e.g., "^E", "⌥L", "F1"). Overrides modifier+key when set.
    pub display_key: Option<&'static str>,
}

/// Returns the list of Alt commands available in the current state.
///
/// Used by both `render_shortcuts()` (to display the bar) and the key handler
/// (to gate dispatch), so the bar and behavior are always in sync.
pub fn available_alt_commands(state: &AppState) -> Vec<AltCommand> {
    let mut cmds = Vec::new();

    let has_track = has_track_context(state);
    let has_album = has_album_context(state);
    let has_playing = state.current_track().is_some();

    // Ctrl+E enqueue: need a track or album that can be enqueued (not from Queue or Now Playing)
    if state.view != View::Queue && state.view != View::NowPlaying && (has_track || has_album || has_enqueue_context(state)) {
        cmds.push(AltCommand { modifier: CommandModifier::Ctrl, key: 'e', label: "enqueue", display_key: None });
    }

    // Ctrl+V view cycle: context-dependent cycling (albums, playlist tracks, genre tabs)
    if let Some(label) = get_view_cycle_label(state) {
        cmds.push(AltCommand { modifier: CommandModifier::Ctrl, key: 'v', label, display_key: None });
    }

    // Ctrl+M similar: need a track or album in context, or something playing
    if has_track || has_album || has_playing {
        cmds.push(AltCommand { modifier: CommandModifier::Ctrl, key: 'm', label: "similar", display_key: None });
    }

    // Ctrl+B album: need a track with album info (Miller columns, folder, or now-playing)
    if has_track_with_album(state) || has_miller_album_context(state)
        || has_folder_track_with_album(state) || has_playing_with_album(state)
    {
        cmds.push(AltCommand { modifier: CommandModifier::Ctrl, key: 'b', label: "album", display_key: None });
    }

    // Ctrl+W save: has tracks in queue or radio (in Queue or NowPlaying view)
    if (state.view == View::Queue || state.view == View::NowPlaying)
        && (!state.queue.is_empty() || !state.radio.tracks.is_empty())
    {
        cmds.push(AltCommand { modifier: CommandModifier::Ctrl, key: 'w', label: "save", display_key: None });
    }

    // Ctrl+X clear: has tracks in queue or radio (in Queue or NowPlaying view)
    if (state.view == View::Queue || state.view == View::NowPlaying)
        && (!state.queue.is_empty() || !state.radio.tracks.is_empty())
    {
        cmds.push(AltCommand { modifier: CommandModifier::Ctrl, key: 'x', label: "clear", display_key: None });
    }

    // Alt global commands
    if state.active_library.is_some() {
        cmds.push(AltCommand { modifier: CommandModifier::Alt, key: 'l', label: "library radio", display_key: None });
        cmds.push(AltCommand { modifier: CommandModifier::Alt, key: 'r', label: "random album", display_key: None });
    }
    if !state.libraries.is_empty() {
        cmds.push(AltCommand { modifier: CommandModifier::Alt, key: 's', label: "switch library", display_key: None });
    }

    // Function key commands (always available)
    if state.view != View::Help {
        cmds.push(AltCommand { modifier: CommandModifier::None, key: '\0', label: "help", display_key: Some("F1") });
    }
    if state.view != View::Settings {
        cmds.push(AltCommand { modifier: CommandModifier::None, key: '\0', label: "settings", display_key: Some("F2") });
    }
    cmds.push(AltCommand { modifier: CommandModifier::None, key: '\0', label: "refresh", display_key: Some("F5") });

    cmds
}

/// Check if a Ctrl+key command is currently available.
pub fn is_action_command_available(state: &AppState, key: char) -> bool {
    available_alt_commands(state).iter().any(|cmd| cmd.modifier == CommandModifier::Ctrl && cmd.key == key)
}

// --- Context helpers ---

/// Determine the view cycle context and return the label for the next state.
/// Returns None if Alt+V is not available in the current context.
fn get_view_cycle_label(state: &AppState) -> Option<&'static str> {
    // Alt+V cycles visualizer tab in NowPlaying view
    if state.view == View::NowPlaying {
        return Some("cycle viz");
    }

    if state.view != View::Browse {
        return None;
    }

    // Genre tab cycle: Genres category, focused column has Genre items
    if state.browse_category == BrowseCategory::Genres {
        let is_genre_col = state.genre_nav.focused()
            .and_then(|col| col.items.first())
            .map_or(false, |item| matches!(item, BrowseItem::Genre { .. }));
        if is_genre_col {
            return Some("cycle view");
        }
    }

    // Playlist track/album cycle: Playlists, view-cycle column focused (index > 0)
    if state.browse_category == BrowseCategory::Playlists && state.playlist_nav.focused_column > 0 {
        let col = state.playlist_nav.focused()?;
        let first_item = col.items.first()?;
        let is_valid = match (state.playlist_view_mode, first_item) {
            (PlaylistViewMode::Tracks, BrowseItem::Track { .. }) => true,
            (PlaylistViewMode::TracksByAlbum, BrowseItem::Album { .. }) => true,
            _ => false,
        };
        if is_valid {
            return Some("cycle view");
        }
        return None;
    }

    // General column cycle: Library | Genres | Playlists (not TracksByAlbum)
    let nav = match state.browse_category {
        BrowseCategory::Library => Some(&state.artist_nav),
        BrowseCategory::Genres => Some(&state.genre_nav),
        BrowseCategory::Playlists if state.playlist_view_mode != PlaylistViewMode::TracksByAlbum => {
            Some(&state.playlist_nav)
        }
        _ => None,
    }?;
    let col = nav.focused()?;
    if col.items.is_empty() {
        return None;
    }

    Some("cycle view")
}

/// Is there a track highlighted/selected in the current view?
fn has_track_context(state: &AppState) -> bool {
    match state.view {
        View::NowPlaying | View::Queue => {
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
            // Check Miller columns for a Track item
            if state.browse_nav()
                .and_then(|n| n.selected_item())
                .map(|i| matches!(i, BrowseItem::Track { .. }))
                .unwrap_or(false)
            {
                return true;
            }
            // Legacy right panel tracks
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
        View::Similar => {
            state.similar_mode == crate::app::state::SimilarMode::Tracks
                && !state.similar_tracks.is_empty()
        }
        _ => false,
    }
}

/// Is there an album highlighted in the current view?
fn has_album_context(state: &AppState) -> bool {
    match state.view {
        View::Browse => {
            // Check Miller columns first
            if state.browse_nav()
                .and_then(|n| n.selected_item())
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
        View::NowPlaying | View::Queue => {
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
            // Check Miller columns for a Track item with album info
            if let Some(nav) = state.browse_nav() {
                if let Some(col) = nav.columns.get(nav.focused_column) {
                    if matches!(col.items.get(col.selected_index), Some(BrowseItem::Track { .. })) {
                        if let Some(track) = col.tracks.get(col.selected_index) {
                            if track.parent_rating_key.is_some() {
                                return true;
                            }
                        }
                    }
                }
            }
            // Legacy right panel tracks
            match state.right_panel_mode {
                RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                    state.selected_album_tracks.get(state.list_state.tracks_index)
                        .map(|t| t.parent_rating_key.is_some())
                        .unwrap_or(false)
                }
                _ => false,
            }
        }
        View::Similar => {
            state.similar_mode == crate::app::state::SimilarMode::Tracks
                && state.similar_tracks.get(state.list_state.similar_index)
                    .map(|t| t.parent_rating_key.is_some())
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

/// Does the now-playing track have album info?
fn has_playing_with_album(state: &AppState) -> bool {
    state.current_track()
        .map(|t| t.parent_rating_key.is_some() && t.grandparent_rating_key.is_some())
        .unwrap_or(false)
}

