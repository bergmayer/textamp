//! Browse view key handling.
//!
//! Handles all browse-related keyboard input including:
//! - Main browse view dispatch
//! - Folder browsing (Miller columns)
//! - Station browsing (Miller columns)
//! - Artist browsing (dynamic Miller columns)
//! - Genre browsing (dynamic Miller columns)
//! - Playlist browsing (dynamic Miller columns)
//! - Common navigation keys for BrowseNavigationState

use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::app::Action;
use crate::app::state::{
    BrowseCategory, Focus, RightPanelMode, View,
};
use crate::app::AppState;

/// Handle Browse view keys (CUA-style).
pub(super) fn handle_browse_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    // Inline list filter mode - handle filter-specific keys
    if state.list_filter.active {
        // Check if current focus is on the filter's target column
        use crate::app::state::GenreContentType;
        let focused_on_filter_column = match state.list_filter.category {
            BrowseCategory::Artists => state.artist_nav.focused_column == state.list_filter.column,
            BrowseCategory::Playlists => {
                if state.playlists_mode == crate::app::state::PlaylistsMode::Stations {
                    state.station_nav.focused_column == state.list_filter.column
                } else {
                    state.playlist_nav.focused_column == state.list_filter.column
                }
            }
            BrowseCategory::Genres => {
                if state.genre_content_type == GenreContentType::Stations {
                    state.station_nav.focused_column == state.list_filter.column
                } else {
                    state.genre_nav.focused_column == state.list_filter.column
                }
            }
            BrowseCategory::Folders => {
                state.folder_state.as_ref()
                    .map(|fs| fs.focused_column == state.list_filter.column)
                    .unwrap_or(false)
            }
        };

        match key.code {
            // Esc always deactivates filter
            KeyCode::Esc => {
                return vec![Action::DeactivateListFilter];
            }
            // Backspace deletes from filter query
            KeyCode::Backspace => {
                return vec![Action::DeleteListFilterChar];
            }
            // Up/Down/Enter only intercept when focused on filter column
            KeyCode::Up if focused_on_filter_column => {
                return vec![Action::FilteredListUp];
            }
            KeyCode::Down if focused_on_filter_column => {
                return vec![Action::FilteredListDown];
            }
            KeyCode::Enter if focused_on_filter_column => {
                return vec![Action::SelectFilteredItem];
            }
            // Typing appends to filter query (only unmodified chars)
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT) => {
                return vec![Action::AppendListFilterChar(c)];
            }
            // Other keys (arrows, etc.) fall through to normal handling
            _ => {}
        }
    }

    // Activate filter with / key (when not in filter mode)
    if key.code == KeyCode::Char('/') && !key.modifiers.contains(KeyModifiers::CONTROL) {
        return vec![Action::ActivateListFilter];
    }

    // Tab/Shift+Tab cycles through nav bar views (handle before category-specific handlers)
    // Shift+Up/Down cycles through modes within current category
    match key.code {
        KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
            return vec![Action::PrevMode];
        }
        KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
            return vec![Action::NextMode];
        }
        KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
            return vec![Action::PrevView];
        }
        KeyCode::BackTab => {
            return vec![Action::PrevView];
        }
        KeyCode::Tab => {
            return vec![Action::NextView];
        }
        _ => {}
    }

    // Handle Folders category separately (Miller columns view)
    if state.browse_category == BrowseCategory::Folders {
        return handle_folder_browse_keys(key, state);
    }

    // Handle Artists category with Miller columns when artist_nav is populated
    if state.browse_category == BrowseCategory::Artists && !state.artist_nav.is_empty() {
        return handle_artist_browse_keys(key, state);
    }

    // Handle Playlists category with Miller columns when playlist_nav is populated
    if state.browse_category == BrowseCategory::Playlists {
        if state.playlists_mode == crate::app::state::PlaylistsMode::Stations {
            return handle_station_browse_keys(key, state);
        }
        if !state.playlist_nav.is_empty() {
            return handle_playlist_browse_keys(key, state);
        }
    }

    // Handle Genres category with Miller columns (Genre | Albums | Tracks)
    // When GenreContentType::Stations is active, redirect to station handling
    if state.browse_category == BrowseCategory::Genres {
        if state.genre_content_type == crate::app::state::GenreContentType::Stations {
            return handle_station_browse_keys(key, state);
        }
        return handle_genre_browse_keys(key, state);
    }

    // Ctrl+R = Create station from current selection (Browse-specific)
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('r') {
        return super::create_station_from_context(state);
    }

    match key.code {
        // Help
        KeyCode::F(1) | KeyCode::Char('?') => vec![Action::SetView(View::Help)],

        // Settings
        KeyCode::F(2) => vec![Action::OpenSettings],

        // Navigation (Tab is handled above, before category-specific handlers)
        KeyCode::Up => vec![Action::ListUp],
        KeyCode::Down => vec![Action::ListDown],
        KeyCode::PageUp => vec![Action::ListPageUp],
        KeyCode::PageDown => vec![Action::ListPageDown],
        KeyCode::Home => vec![Action::ListTop],
        KeyCode::End => vec![Action::ListBottom],

        // Selection/Action - depends on focus and current mode
        KeyCode::Enter | KeyCode::Right => {
            if state.focus == Focus::Left {
                // Left panel: depends on category
                match state.browse_category {
                    BrowseCategory::Artists => {
                        // Artist -> load their albums into right panel
                        vec![Action::LoadArtistAlbums]
                    }
                    BrowseCategory::Playlists => {
                        // Playlists -> load tracks directly
                        vec![Action::LoadCategoryTracks]
                    }
                    BrowseCategory::Genres => {
                        // Genre/Mood/Stations -> handled by genre browse keys
                        // (Stations are now part of genre content type cycle)
                        vec![Action::LoadGenreAlbums]
                    }
                    BrowseCategory::Folders => {
                        // Folders use folder navigation
                        vec![Action::LoadFolderRoot]
                    }
                }
            } else {
                // Right panel: depends on mode
                match state.right_panel_mode {
                    RightPanelMode::ArtistAlbums => {
                        // Index 0 = "All Tracks", otherwise album
                        if state.list_state.right_albums_index == 0 {
                            vec![Action::LoadArtistAllTracks]
                        } else {
                            vec![Action::LoadSelectedAlbumTracks]
                        }
                    }
                    RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                        // Track selected -> play it
                        vec![Action::PlayTrackFromCategory(state.list_state.tracks_index)]
                    }
                    RightPanelMode::CategoryAlbums => {
                        // Album selected in genre view -> load album tracks
                        if let Some(album) = state.genre_albums.get(state.genre_albums_index).cloned() {
                            state.selected_album_title = album.title.clone();
                            state.pending_album_key = Some(album.rating_key.clone());
                            vec![Action::LoadAlbumTracks { rating_key: album.rating_key }]
                        } else {
                            vec![]
                        }
                    }
                    RightPanelMode::Empty => vec![],
                }
            }
        }
        KeyCode::Left | KeyCode::Backspace | KeyCode::Esc => {
            if state.focus == Focus::Right {
                // Check if we should go back to album list (from tracks view)
                if state.right_panel_mode == RightPanelMode::AlbumTracks {
                    // If we came from a genre album, go back to CategoryAlbums
                    if state.browse_category == BrowseCategory::Genres {
                        state.right_panel_mode = RightPanelMode::CategoryAlbums;
                        state.selected_album_tracks.clear();
                        vec![]
                    } else {
                        vec![Action::GoBackInRightPanel]
                    }
                } else {
                    vec![Action::ToggleFocus]
                }
            } else if state.browse_category == BrowseCategory::Genres && state.genre_content_type == crate::app::state::GenreContentType::Stations {
                // In stations view (via Genres), go back in Miller columns
                if state.station_nav.can_go_left() {
                    state.station_nav.focus_left();
                    // Update legacy state to match focused column
                    if let Some(col) = state.station_nav.focused() {
                        state.stations = col.stations.clone();
                    }
                }
                vec![]
            } else {
                vec![]
            }
        }

        // Alphabet jumping - jump to first item starting with letter
        // Allow with no modifiers or just SHIFT (for uppercase)
        KeyCode::Char(c) if c.is_ascii_alphabetic() && !key.modifiers.contains(KeyModifiers::CONTROL) => {
            super::jump_to_letter(state, c);
            vec![]
        }

        _ => vec![],
    }
}

/// Handle folder browsing mode keys (Miller columns style).
pub(super) fn handle_folder_browse_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    use crate::services::FolderItemType;

    match key.code {
        // Help
        KeyCode::F(1) | KeyCode::Char('?') => vec![Action::SetView(View::Help)],

        // Settings
        KeyCode::F(2) => vec![Action::OpenSettings],

        // Up/Down - navigate within current column
        // BUG FIX: Clear columns to the right when selection changes
        KeyCode::Up => {
            if let Some(ref mut folder_state) = state.folder_state {
                folder_state.move_up();
                // Clear columns to the right since selection changed
                folder_state.truncate_right_columns();
            }
            vec![]
        }
        KeyCode::Down => {
            if let Some(ref mut folder_state) = state.folder_state {
                folder_state.move_down();
                // Clear columns to the right since selection changed
                folder_state.truncate_right_columns();
            }
            vec![]
        }
        KeyCode::PageUp => {
            if let Some(ref mut folder_state) = state.folder_state {
                if let Some(col) = folder_state.focused_mut() {
                    col.selected_index = col.selected_index.saturating_sub(10);
                }
                // Clear columns to the right since selection changed
                folder_state.truncate_right_columns();
            }
            vec![]
        }
        KeyCode::PageDown => {
            if let Some(ref mut folder_state) = state.folder_state {
                if let Some(col) = folder_state.focused_mut() {
                    let max = col.items.len().saturating_sub(1);
                    col.selected_index = (col.selected_index + 10).min(max);
                }
                // Clear columns to the right since selection changed
                folder_state.truncate_right_columns();
            }
            vec![]
        }
        KeyCode::Home => {
            if let Some(ref mut folder_state) = state.folder_state {
                if let Some(col) = folder_state.focused_mut() {
                    col.selected_index = 0;
                }
                // Clear columns to the right since selection changed
                folder_state.truncate_right_columns();
            }
            vec![]
        }
        KeyCode::End => {
            if let Some(ref mut folder_state) = state.folder_state {
                if let Some(col) = folder_state.focused_mut() {
                    col.selected_index = col.items.len().saturating_sub(1);
                }
                // Clear columns to the right since selection changed
                folder_state.truncate_right_columns();
            }
            vec![]
        }

        // Right/Enter - go into selected folder or play track
        KeyCode::Enter | KeyCode::Right => {
            if let Some(ref mut folder_state) = state.folder_state {
                // First check if there's already a column to the right we can move to
                if folder_state.focus_right() {
                    return vec![];
                }

                // Otherwise, load the selected item
                if let Some(item) = folder_state.selected_item().cloned() {
                    match item.item_type {
                        FolderItemType::Folder => {
                            return vec![Action::NavigateIntoFolder(item.key)];
                        }
                        FolderItemType::Track => {
                            return vec![Action::PlayFolderTracks];
                        }
                    }
                }
            }
            vec![]
        }

        // Left/Backspace - move focus to previous column
        KeyCode::Left | KeyCode::Backspace => {
            if let Some(ref mut folder_state) = state.folder_state {
                if folder_state.can_go_left() {
                    folder_state.focus_left();
                }
            }
            vec![]
        }

        // Escape - go back or exit
        KeyCode::Esc => {
            if let Some(ref mut folder_state) = state.folder_state {
                if folder_state.can_go_left() {
                    folder_state.focus_left();
                    return vec![];
                }
            }
            vec![]
        }

        // Alphabet jumping in current column
        // Plain letter: jump to first item starting with that letter
        // Shift+letter: jump to first item where first char matches current item's first char
        //               AND second char matches the pressed letter
        KeyCode::Char(c) if c.is_ascii_alphabetic() && !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let letter_lower = c.to_ascii_lowercase();
            let use_second_char = key.modifiers.contains(KeyModifiers::SHIFT);
            if let Some(ref mut folder_state) = state.folder_state {
                if let Some(col) = folder_state.focused_mut() {
                    if use_second_char {
                        // Get the first letter of the currently selected item
                        let first_letter = col.items.get(col.selected_index)
                            .map(|item| item.title.chars().next())
                            .flatten()
                            .map(|ch| ch.to_ascii_lowercase());

                        if let Some(first_letter) = first_letter {
                            // Find first item starting with that letter AND having pressed letter as second char
                            if let Some(idx) = col.items.iter().position(|item| {
                                let mut chars = item.title.chars();
                                let first = chars.next().map(|ch| ch.to_ascii_lowercase());
                                let second = chars.next().map(|ch| ch.to_ascii_lowercase());
                                first == Some(first_letter) && second == Some(letter_lower)
                            }) {
                                col.selected_index = idx;
                            }
                        }
                    } else {
                        // Normal first-letter jump
                        if let Some(idx) = col.items.iter().position(|item| {
                            item.title.chars().next()
                                .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                                .unwrap_or(false)
                        }) {
                            col.selected_index = idx;
                        }
                    }
                }
                // Clear columns to the right since selection changed
                folder_state.truncate_right_columns();
            }
            vec![]
        }

        _ => vec![],
    }
}

/// Handle Station browsing with Miller columns.
pub(super) fn handle_station_browse_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    match key.code {
        // Help
        KeyCode::F(1) | KeyCode::Char('?') => vec![Action::SetView(View::Help)],

        // Settings
        KeyCode::F(2) => vec![Action::OpenSettings],

        // Up/Down - navigate within current column
        KeyCode::Up => {
            state.station_nav.move_up();
            // Clear columns to the right since selection changed
            state.station_nav.truncate_right_columns();
            vec![]
        }
        KeyCode::Down => {
            state.station_nav.move_down();
            // Clear columns to the right since selection changed
            state.station_nav.truncate_right_columns();
            vec![]
        }
        KeyCode::PageUp => {
            if let Some(col) = state.station_nav.focused_mut() {
                col.selected_index = col.selected_index.saturating_sub(10);
            }
            state.station_nav.truncate_right_columns();
            vec![]
        }
        KeyCode::PageDown => {
            if let Some(col) = state.station_nav.focused_mut() {
                let max = col.stations.len().saturating_sub(1);
                col.selected_index = (col.selected_index + 10).min(max);
            }
            state.station_nav.truncate_right_columns();
            vec![]
        }
        KeyCode::Home => {
            if let Some(col) = state.station_nav.focused_mut() {
                col.selected_index = 0;
            }
            state.station_nav.truncate_right_columns();
            vec![]
        }
        KeyCode::End => {
            if let Some(col) = state.station_nav.focused_mut() {
                col.selected_index = col.stations.len().saturating_sub(1);
            }
            state.station_nav.truncate_right_columns();
            vec![]
        }

        // Right/Enter - drill into category or play station
        KeyCode::Enter | KeyCode::Right => {
            // First check if there's already a column to the right we can move to
            if state.station_nav.focus_right() {
                // Update legacy state
                if let Some(col) = state.station_nav.focused() {
                    state.stations = col.stations.clone();
                }
                return vec![];
            }

            // Otherwise, load the selected station
            if let Some(station) = state.station_nav.selected_station().cloned() {
                if station.is_category() {
                    return vec![Action::DrillIntoStation(station.key.clone(), station.title.clone())];
                } else {
                    return vec![Action::PlayStation(station.key.clone())];
                }
            }
            vec![]
        }

        // Left/Backspace - move focus to previous column
        KeyCode::Left | KeyCode::Backspace => {
            if state.station_nav.can_go_left() {
                state.station_nav.focus_left();
                // Update legacy state
                if let Some(col) = state.station_nav.focused() {
                    state.stations = col.stations.clone();
                }
            }
            vec![]
        }

        // Escape - go back or do nothing
        KeyCode::Esc => {
            if state.station_nav.can_go_left() {
                state.station_nav.focus_left();
                if let Some(col) = state.station_nav.focused() {
                    state.stations = col.stations.clone();
                }
            }
            vec![]
        }

        // Alphabet jumping in current column
        // Plain letter: jump to first item starting with that letter
        // Shift+letter: jump to first item where first char matches current item's first char
        //               AND second char matches the pressed letter
        KeyCode::Char(c) if c.is_ascii_alphabetic() && !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let letter_lower = c.to_ascii_lowercase();
            let use_second_char = key.modifiers.contains(KeyModifiers::SHIFT);
            if let Some(col) = state.station_nav.focused_mut() {
                if use_second_char {
                    // Get the first letter of the currently selected item
                    let first_letter = col.stations.get(col.selected_index)
                        .and_then(|s| s.title.chars().next())
                        .map(|ch| ch.to_ascii_lowercase());

                    if let Some(first_letter) = first_letter {
                        // Find first item starting with that letter AND having pressed letter as second char
                        if let Some(idx) = col.stations.iter().position(|s| {
                            let mut chars = s.title.chars();
                            let first = chars.next().map(|ch| ch.to_ascii_lowercase());
                            let second = chars.next().map(|ch| ch.to_ascii_lowercase());
                            first == Some(first_letter) && second == Some(letter_lower)
                        }) {
                            col.selected_index = idx;
                        }
                    }
                } else {
                    // Normal first-letter jump
                    if let Some(idx) = col.stations.iter().position(|s| {
                        s.title.chars().next()
                            .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                            .unwrap_or(false)
                    }) {
                        col.selected_index = idx;
                    }
                }
            }
            state.station_nav.truncate_right_columns();
            vec![]
        }

        _ => vec![],
    }
}

/// Handle Artist browsing with dynamic Miller columns.
pub(super) fn handle_artist_browse_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    use crate::app::state::BrowseItem;

    // Handle common navigation keys
    if let Some(actions) = handle_browse_nav_keys(key, &mut state.artist_nav) {
        return actions;
    }

    // Handle Enter/Right - drill down or play track
    if matches!(key.code, KeyCode::Enter | KeyCode::Right) {
        if let Some(item) = state.artist_nav.selected_item().cloned() {
            return match item {
                BrowseItem::Artist { key, title } => {
                    state.selected_artist_name = title;
                    vec![Action::LoadArtistAlbumsForMiller { artist_key: key }]
                }
                BrowseItem::Album { key, title, .. } => {
                    state.selected_album_title = title;
                    vec![Action::LoadAlbumTracksForMiller { album_key: key }]
                }
                BrowseItem::AllTracks { artist_key, artist_name } => {
                    state.selected_album_title = format!("All tracks by {}", artist_name);
                    vec![Action::LoadArtistAllTracksForMiller { artist_key }]
                }
                BrowseItem::Track { .. } => {
                    if let Some(col) = state.artist_nav.focused() {
                        let idx = col.selected_index;
                        vec![Action::PlayTrackFromMiller { column_index: state.artist_nav.focused_column, track_index: idx }]
                    } else {
                        vec![]
                    }
                }
                _ => vec![],
            };
        }
    }

    vec![]
}

/// Handle Genre browsing with dynamic Miller columns
pub(super) fn handle_genre_browse_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    use crate::app::state::BrowseItem;

    // Handle common navigation keys
    if let Some(actions) = handle_browse_nav_keys(key, &mut state.genre_nav) {
        return actions;
    }

    // Handle Enter/Right - drill into selected item or play track
    if matches!(key.code, KeyCode::Enter | KeyCode::Right) {
        if let Some(item) = state.genre_nav.selected_item().cloned() {
            return match item {
                BrowseItem::Genre { key, .. } => {
                    vec![Action::LoadGenreAlbumsForMiller { genre_key: key }]
                }
                BrowseItem::Album { key, .. } => {
                    vec![Action::LoadGenreTracksForMiller { album_key: key }]
                }
                BrowseItem::Track { .. } => {
                    if let Some(col) = state.genre_nav.focused() {
                        let idx = col.selected_index;
                        vec![Action::PlayGenreTrackFromMiller { column_index: state.genre_nav.focused_column, track_index: idx }]
                    } else {
                        vec![]
                    }
                }
                _ => vec![],
            };
        }
    }

    vec![]
}

/// Handle Playlist browsing with dynamic Miller columns
pub(super) fn handle_playlist_browse_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    use crate::app::state::BrowseItem;

    // Handle common navigation keys
    if let Some(actions) = handle_browse_nav_keys(key, &mut state.playlist_nav) {
        return actions;
    }

    // Handle Enter/Right - drill into playlist/album or play track
    if matches!(key.code, KeyCode::Enter | KeyCode::Right) {
        if let Some(item) = state.playlist_nav.selected_item().cloned() {
            return match item {
                BrowseItem::Playlist { key, .. } => {
                    vec![Action::LoadPlaylistTracksForMiller { playlist_key: key }]
                }
                BrowseItem::Album { key, title, .. } => {
                    // For Recently Added mode - load album tracks
                    state.selected_album_title = title;
                    vec![Action::LoadAlbumTracksForPlaylistMiller { album_key: key }]
                }
                BrowseItem::Track { .. } => {
                    if let Some(col) = state.playlist_nav.focused() {
                        let idx = col.selected_index;
                        vec![Action::PlayPlaylistTrackFromMiller { column_index: state.playlist_nav.focused_column, track_index: idx }]
                    } else {
                        vec![]
                    }
                }
                _ => vec![],
            };
        }
    }

    vec![]
}

/// Returns Some(actions) if the key was handled, None if not.
pub fn handle_browse_nav_keys(
    key: event::KeyEvent,
    nav: &mut crate::app::state::BrowseNavigationState,
) -> Option<Vec<Action>> {
    match key.code {
        // Help
        KeyCode::F(1) | KeyCode::Char('?') => Some(vec![Action::SetView(View::Help)]),

        // Settings
        KeyCode::F(2) => Some(vec![Action::OpenSettings]),

        // Up - move selection up, truncate columns to the right
        KeyCode::Up => {
            nav.move_up();
            nav.truncate_right();
            Some(vec![])
        }

        // Down - move selection down, truncate columns to the right
        KeyCode::Down => {
            nav.move_down();
            nav.truncate_right();
            Some(vec![])
        }

        // Page Up
        KeyCode::PageUp => {
            if let Some(col) = nav.focused_mut() {
                col.selected_index = col.selected_index.saturating_sub(10);
            }
            nav.truncate_right();
            Some(vec![])
        }

        // Page Down
        KeyCode::PageDown => {
            if let Some(col) = nav.focused_mut() {
                let max_idx = col.items.len().saturating_sub(1);
                col.selected_index = (col.selected_index + 10).min(max_idx);
            }
            nav.truncate_right();
            Some(vec![])
        }

        // Home - go to first item
        KeyCode::Home => {
            if let Some(col) = nav.focused_mut() {
                col.selected_index = 0;
            }
            nav.truncate_right();
            Some(vec![])
        }

        // End - go to last item
        KeyCode::End => {
            if let Some(col) = nav.focused_mut() {
                col.selected_index = col.items.len().saturating_sub(1);
            }
            nav.truncate_right();
            Some(vec![])
        }

        // Left/Backspace/Esc - focus previous column
        KeyCode::Left | KeyCode::Backspace | KeyCode::Esc => {
            if nav.can_go_left() {
                nav.focus_left();
            }
            Some(vec![])
        }

        // Tab is NOT handled here - it's handled globally to cycle between views
        // (Artists → Playlists → Genres → Folders → Now Playing)

        // Alphabet jumping in current column
        // Plain letter: jump to first item starting with that letter
        // Shift+letter: jump to first item where first char matches current item's first char
        //               AND second char matches the pressed letter
        KeyCode::Char(c) if c.is_ascii_alphabetic() && !key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(col) = nav.focused_mut() {
                let letter_lower = c.to_ascii_lowercase();
                let use_second_char = key.modifiers.contains(KeyModifiers::SHIFT);

                if use_second_char {
                    // Get the first letter of the currently selected item
                    let first_letter = col.items.get(col.selected_index)
                        .and_then(|item| item.title().chars().next())
                        .map(|ch| ch.to_ascii_lowercase());

                    if let Some(first_letter) = first_letter {
                        // Find first item starting with that letter AND having pressed letter as second char
                        if let Some(idx) = col.items.iter().position(|item| {
                            let mut chars = item.title().chars();
                            let first = chars.next().map(|ch| ch.to_ascii_lowercase());
                            let second = chars.next().map(|ch| ch.to_ascii_lowercase());
                            first == Some(first_letter) && second == Some(letter_lower)
                        }) {
                            col.selected_index = idx;
                        }
                    }
                } else {
                    // Normal first-letter jump
                    if let Some(idx) = col.items.iter().position(|item| {
                        item.title().chars().next()
                            .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                            .unwrap_or(false)
                    }) {
                        col.selected_index = idx;
                    }
                }
            }
            nav.truncate_right();
            Some(vec![])
        }

        // Not handled by common navigation
        _ => None,
    }
}

/// Update the column's selected_index for the filter's target category/column.
pub fn update_filter_column_selection(state: &mut AppState, item_idx: usize) {
    use crate::app::state::GenreContentType;
    let category = state.list_filter.category;
    let column = state.list_filter.column;

    match category {
        BrowseCategory::Artists => {
            if let Some(col) = state.artist_nav.columns.get_mut(column) {
                col.selected_index = item_idx;
            }
        }
        BrowseCategory::Playlists => {
            if let Some(col) = state.playlist_nav.columns.get_mut(column) {
                col.selected_index = item_idx;
            }
        }
        BrowseCategory::Genres => {
            if state.genre_content_type == GenreContentType::Stations {
                if let Some(col) = state.station_nav.columns.get_mut(column) {
                    col.selected_index = item_idx;
                }
            } else {
                if let Some(col) = state.genre_nav.columns.get_mut(column) {
                    col.selected_index = item_idx;
                }
            }
        }
        BrowseCategory::Folders => {
            if let Some(ref mut folder_state) = state.folder_state {
                if let Some(col) = folder_state.columns.get_mut(column) {
                    col.selected_index = item_idx;
                }
            }
        }
    }
}

/// Get the drill-down actions for the selected filtered item.
/// This simulates pressing Enter on the selected item to drill into it.
pub fn get_filter_drilldown_actions(state: &mut AppState) -> Vec<Action> {
    use crate::app::state::GenreContentType;
    let category = state.list_filter.category;

    // Get the appropriate drill-down action based on category
    match category {
        BrowseCategory::Artists => {
            // Use the artist_nav enter key logic
            handle_artist_browse_keys(
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Enter,
                    crossterm::event::KeyModifiers::NONE,
                ),
                state,
            )
        }
        BrowseCategory::Playlists => {
            handle_playlist_browse_keys(
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Enter,
                    crossterm::event::KeyModifiers::NONE,
                ),
                state,
            )
        }
        BrowseCategory::Genres => {
            if state.genre_content_type == GenreContentType::Stations {
                handle_station_browse_keys(
                    crossterm::event::KeyEvent::new(
                        crossterm::event::KeyCode::Enter,
                        crossterm::event::KeyModifiers::NONE,
                    ),
                    state,
                )
            } else {
                handle_genre_browse_keys(
                    crossterm::event::KeyEvent::new(
                        crossterm::event::KeyCode::Enter,
                        crossterm::event::KeyModifiers::NONE,
                    ),
                    state,
                )
            }
        }
        BrowseCategory::Folders => {
            handle_folder_browse_keys(
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Enter,
                    crossterm::event::KeyModifiers::NONE,
                ),
                state,
            )
        }
    }
}
