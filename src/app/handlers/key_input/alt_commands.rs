//! Contextual command availability — single source of truth for both the shortcut bar
//! display and the key handler dispatch.

use crate::app::state::{
    BrowseCategory, BrowseItem, Focus, PlaybackMode,
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
    /// Whether the command is currently available. Disabled commands are shown greyed out.
    pub enabled: bool,
}

/// Returns all commands for the shortcut bar. Disabled commands have `enabled: false`
/// and are shown greyed out rather than hidden.
///
/// Used by both `render_shortcuts()` (to display the bar) and the key handler
/// (to gate dispatch), so the bar and behavior are always in sync.
pub fn available_alt_commands(state: &AppState) -> Vec<AltCommand> {
    let mut cmds = Vec::new();

    let has_track = has_track_context(state);
    let has_album = has_album_context(state);
    let has_playing = state.current_track().is_some();

    // --- Top row: function keys (always present) ---

    cmds.push(AltCommand { modifier: CommandModifier::None, key: '\0', label: "help", display_key: Some("F1"),
        enabled: state.view != View::Help });
    cmds.push(AltCommand { modifier: CommandModifier::None, key: '\0', label: "settings", display_key: Some("F2"),
        enabled: state.view != View::Settings });
    cmds.push(AltCommand { modifier: CommandModifier::None, key: '\0', label: "library", display_key: Some("F3"),
        enabled: !state.libraries.is_empty() });
    cmds.push(AltCommand { modifier: CommandModifier::None, key: '\0', label: "bio", display_key: Some("F4"),
        enabled: super::super::helpers::get_artist_for_bio(state).is_some() });
    cmds.push(AltCommand { modifier: CommandModifier::None, key: '\0', label: "refresh", display_key: Some("F5"),
        enabled: true });

    // --- Bottom row: contextual commands (always present, greyed out when unavailable) ---

    // Ctrl+F find
    cmds.push(AltCommand { modifier: CommandModifier::Ctrl, key: 'f', label: "find", display_key: None,
        enabled: true });

    // Ctrl+E enqueue
    let enqueue_enabled = state.view != View::Queue && state.view != View::NowPlaying
        && (has_track || has_album || has_enqueue_context(state));
    cmds.push(AltCommand { modifier: CommandModifier::Ctrl, key: 'e', label: "enqueue", display_key: None,
        enabled: enqueue_enabled });

    // Ctrl+M similar
    let similar_enabled = has_artist_context(state) || has_track || has_album || has_playing;
    cmds.push(AltCommand { modifier: CommandModifier::Ctrl, key: 'm', label: "similar", display_key: None,
        enabled: similar_enabled });

    // Ctrl+R related artists
    let related_enabled = has_artist_context(state) || has_track || has_album || has_playing;
    cmds.push(AltCommand { modifier: CommandModifier::Ctrl, key: 'r', label: "related", display_key: None,
        enabled: related_enabled });

    // Ctrl+J jump to album
    // In Library view, only useful when now-playing track is from a different album
    // than the one currently viewed in Miller columns.
    let in_library = state.view == View::Browse && state.browse_category == BrowseCategory::Library;
    let album_enabled = if in_library {
        playing_album_differs_from_viewed(state)
    } else {
        has_track_with_album(state) || has_miller_album_context(state)
            || has_folder_track_with_album(state) || has_playing_with_album(state)
    };
    cmds.push(AltCommand { modifier: CommandModifier::Ctrl, key: 'j', label: "jump to album", display_key: None,
        enabled: album_enabled });

    // Ctrl+W save
    let save_enabled = (state.view == View::Queue || state.view == View::NowPlaying)
        && (!state.queue.is_empty() || !state.radio.tracks.is_empty());
    cmds.push(AltCommand { modifier: CommandModifier::Ctrl, key: 'w', label: "save", display_key: None,
        enabled: save_enabled });

    // Ctrl+X clear
    cmds.push(AltCommand { modifier: CommandModifier::Ctrl, key: 'x', label: "clear", display_key: None,
        enabled: save_enabled });

    // Ctrl+S sort
    let sort_enabled = state.view == View::Browse && state.browse_nav()
        .and_then(|nav| nav.focused())
        .map_or(false, |col| {
            col.items.first().map_or(false, |i| {
                matches!(i, BrowseItem::Artist { .. } | BrowseItem::Album { .. } | BrowseItem::Track { .. })
            }) || col.items.iter().take(4).any(|i| matches!(i, BrowseItem::Artist { .. } | BrowseItem::Album { .. }))
        });
    cmds.push(AltCommand { modifier: CommandModifier::Ctrl, key: 's', label: "sort", display_key: None,
        enabled: sort_enabled });

    // Alt global commands
    let lib_enabled = state.active_library.is_some();
    let filter_enabled = state.view == View::Browse && !state.list_filter.active
        && !state.popups.search_active && state.popups.sort.is_none()
        && state.popups.radio_launcher.is_none() && state.popups.adventure_launcher.is_none()
        && state.popups.artist_radio_picker.is_none();
    cmds.push(AltCommand { modifier: CommandModifier::Alt, key: 'f', label: "filter", display_key: None,
        enabled: filter_enabled });
    cmds.push(AltCommand { modifier: CommandModifier::Alt, key: 'r', label: "random album", display_key: None,
        enabled: lib_enabled });

    cmds
}

/// Check if a Ctrl+key command is currently available (enabled).
pub fn is_action_command_available(state: &AppState, key: char) -> bool {
    available_alt_commands(state).iter().any(|cmd| cmd.modifier == CommandModifier::Ctrl && cmd.key == key && cmd.enabled)
}

// --- Context helpers ---

/// Is there an artist highlighted in the current view?
fn has_artist_context(state: &AppState) -> bool {
    if state.view == View::Browse {
        if let Some(nav) = state.browse_nav() {
            if let Some(item) = nav.selected_item() {
                if matches!(item, BrowseItem::Artist { .. }) {
                    return true;
                }
            }
        }
    }
    false
}

/// Is there a track highlighted/selected in the current view?
fn has_track_context(state: &AppState) -> bool {
    match state.view {
        View::NowPlaying | View::Queue => {
            let idx = state.list_state.queue_index;
            match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => idx < state.queue.len(),
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
            // Folder view: any track item in the focused column
            if state.browse_category == BrowseCategory::Folders {
                if let Some(ref fs) = state.folder_state {
                    if let Some(col) = fs.focused() {
                        return col.items.iter().any(|item| item.rating_key.is_some());
                    }
                }
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
            state.similar.mode == crate::app::state::SimilarMode::Tracks
                && !state.similar.tracks.is_empty()
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
            !state.similar.albums.is_empty()
        }
        View::Related => {
            // Album row selected in related view
            let idx = state.list_state.related_index;
            let resolved = super::super::helpers::navigation::related_flat_resolve(&state.related.groups, idx);
            resolved.map(|(_, is_header, _)| !is_header).unwrap_or(false)
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
                PlaybackMode::Queue | PlaybackMode::None => state.queue.get(idx),
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
            state.similar.mode == crate::app::state::SimilarMode::Tracks
                && state.similar.tracks.get(state.list_state.similar_index)
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

/// In Library view, does the now-playing track's album differ from the currently viewed album?
/// Returns false if there's no now-playing track.
fn playing_album_differs_from_viewed(state: &AppState) -> bool {
    let playing_album_key = state.current_track()
        .and_then(|t| t.parent_rating_key.clone());
    let Some(playing_key) = playing_album_key else { return false };

    // Also need album info (grandparent = artist) to navigate
    if state.current_track().and_then(|t| t.grandparent_rating_key.as_ref()).is_none() {
        return false;
    }

    // Find the album key currently visible in Miller columns
    let nav = &state.artist_nav;
    let focused = nav.focused_column;
    let current_key = nav.columns.get(focused)
        .and_then(|c| c.items.get(c.selected_index))
        .and_then(|item| match item {
            BrowseItem::Album { key, .. } => Some(key.clone()),
            BrowseItem::Track { .. } => {
                // Track focused → check parent column for album
                (focused > 0).then(|| nav.columns.get(focused - 1)).flatten()
                    .and_then(|c| c.items.get(c.selected_index))
                    .and_then(|i| match i {
                        BrowseItem::Album { key, .. } => Some(key.clone()),
                        _ => None,
                    })
            }
            _ => None,
        });

    match current_key {
        Some(key) => key != playing_key, // Different album → useful to jump
        None => true,                     // Not viewing any album → jump is useful
    }
}

