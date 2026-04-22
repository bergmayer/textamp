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

use crate::app::action::*;
use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::app::Action;
use crate::app::state::{
    BrowseCategory, Focus, RightPanelMode, View,
};
use crate::app::AppState;
use super::super::helpers;

/// Handle Browse view keys (CUA-style).
pub(super) fn handle_browse_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    // Inline list filter mode - handle filter-specific keys
    if state.list_filter.active {
        // Check if current focus is on the filter's target column
        let focused_on_filter_column = match state.list_filter.category {
            BrowseCategory::Library => state.artist_nav.focused_column == state.list_filter.column,
            BrowseCategory::Playlists => state.playlist_nav.focused_column == state.list_filter.column,
            BrowseCategory::Genres => state.genre_nav.focused_column == state.list_filter.column,
            BrowseCategory::Folders => {
                state.folder_state.as_ref()
                    .map(|fs| fs.focused_column == state.list_filter.column)
                    .unwrap_or(false)
            }
        };

        match key.code {
            // Esc always deactivates filter
            KeyCode::Esc => {
                return vec![SearchAction::DeactivateListFilter.into()];
            }
            // Backspace deletes from filter query
            KeyCode::Backspace => {
                return vec![SearchAction::DeleteListFilterChar.into()];
            }
            // Up/Down/Enter navigate filtered results
            KeyCode::Up if focused_on_filter_column => {
                return vec![SearchAction::FilteredListUp.into()];
            }
            KeyCode::Up if !focused_on_filter_column => {
                // Move focus back to the filter column so user can navigate results
                truncate_filter_right_columns(state);
                return vec![];
            }
            KeyCode::Down if focused_on_filter_column => {
                return vec![SearchAction::FilteredListDown.into()];
            }
            KeyCode::Enter | KeyCode::Right if focused_on_filter_column => {
                return vec![SearchAction::SelectFilteredItem.into()];
            }
            // Typing appends to filter query (only unmodified chars)
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT) => {
                return vec![SearchAction::AppendListFilterChar(c).into()];
            }
            // Left/Esc on non-filter column: deactivate and fall through
            KeyCode::Left if !focused_on_filter_column => {
                state.list_filter.deactivate();
            }
            // Other keys fall through to normal handling
            _ => {}
        }
    }

    // Activate filter with / key (when not in filter mode and not on category column)
    if key.code == KeyCode::Char('/') && !key.modifiers.contains(KeyModifiers::CONTROL)
        && !state.category_column_focused {
        return vec![SearchAction::ActivateListFilter.into()];
    }

    // Tab/Shift+Tab navigates through main views:
    // Library → Playlists → Queue → Now Playing
    match key.code {
        KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
            return tab_navigate_prev(state);
        }
        KeyCode::BackTab => {
            return tab_navigate_prev(state);
        }
        KeyCode::Tab => {
            return tab_navigate_next(state);
        }
        _ => {}
    }

    // Category column navigation (column 0 in browse view)
    if state.category_column_focused {
        return handle_category_column_keys(key, state);
    }

    // Handle Folders category separately (Miller columns view)
    if state.browse_category == BrowseCategory::Folders {
        return handle_folder_browse_keys(key, state);
    }

    // Handle Artists category with Miller columns when artist_nav is populated
    if state.browse_category == BrowseCategory::Library && !state.artist_nav.is_empty() {
        return handle_artist_browse_keys(key, state);
    }

    // Handle Playlists category with Miller columns when playlist_nav is populated
    if state.browse_category == BrowseCategory::Playlists {
        if !state.playlist_nav.is_empty() {
            return handle_playlist_browse_keys(key, state);
        }
    }

    // Handle Genres category with Miller columns (Genre | Albums | Tracks)
    if state.browse_category == BrowseCategory::Genres {
        return handle_genre_browse_keys(key, state);
    }

    match key.code {
        // Help
        KeyCode::F(1) | KeyCode::Char('?') => vec![NavigationAction::SetView(View::Help).into()],

        // Settings
        KeyCode::F(2) => vec![SettingsAction::OpenSettings.into()],

        // Navigation (Tab is handled above, before category-specific handlers)
        KeyCode::Up => vec![DataAction::ListUp.into()],
        KeyCode::Down => vec![DataAction::ListDown.into()],
        KeyCode::PageUp => vec![DataAction::ListPageUp.into()],
        KeyCode::PageDown => vec![DataAction::ListPageDown.into()],
        KeyCode::Home => vec![DataAction::ListTop.into()],
        KeyCode::End => vec![DataAction::ListBottom.into()],

        // Selection/Action - depends on focus and current mode
        KeyCode::Enter | KeyCode::Right => {
            if state.focus == Focus::Left {
                // Left panel: depends on category
                match state.browse_category {
                    BrowseCategory::Library => {
                        // Artist -> load their albums into right panel
                        vec![DataAction::LoadArtistAlbums.into()]
                    }
                    BrowseCategory::Playlists => {
                        // Playlists -> load tracks directly
                        vec![DataAction::LoadCategoryTracks.into()]
                    }
                    BrowseCategory::Genres => {
                        // Genre/Mood -> handled by genre browse keys
                        vec![BrowseAction::LoadGenreAlbums.into()]
                    }
                    BrowseCategory::Folders => {
                        // Folders use folder navigation
                        vec![FolderAction::LoadFolderRoot.into()]
                    }
                }
            } else {
                // Right panel: depends on mode
                match state.library.right_panel_mode {
                    RightPanelMode::ArtistAlbums => {
                        // Index 0 = "All Tracks", otherwise album
                        if state.list_state.right_albums_index == 0 {
                            vec![DataAction::LoadArtistAllTracks.into()]
                        } else {
                            vec![DataAction::LoadSelectedAlbumTracks.into()]
                        }
                    }
                    RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                        // Track selected -> play it
                        vec![QueueAction::PlayTrackFromCategory(state.list_state.tracks_index).into()]
                    }
                    RightPanelMode::CategoryAlbums => {
                        // Album selected in genre view -> load album tracks
                        if let Some(album) = state.library.genre_albums.get(state.library.genre_albums_index).cloned() {
                            state.library.selected_album_title = album.title.clone();
                            state.search.pending_album_key = Some(album.rating_key.clone());
                            vec![DataAction::LoadAlbumTracks { rating_key: album.rating_key }.into()]
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
                if state.library.right_panel_mode == RightPanelMode::AlbumTracks {
                    // If we came from a genre album, go back to CategoryAlbums
                    if state.browse_category == BrowseCategory::Genres {
                        state.library.right_panel_mode = RightPanelMode::CategoryAlbums;
                        state.library.selected_album_tracks.clear();
                        vec![]
                    } else {
                        vec![DataAction::GoBackInRightPanel.into()]
                    }
                } else {
                    vec![NavigationAction::ToggleFocus.into()]
                }
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

/// Handle keys when the category selector column (column 0) is focused.
fn handle_category_column_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    let categories = BrowseCategory::all();
    let num_categories = categories.len();

    match key.code {
        // Help
        KeyCode::F(1) | KeyCode::Char('?') => vec![NavigationAction::SetView(View::Help).into()],

        // Settings
        KeyCode::F(2) => vec![SettingsAction::OpenSettings.into()],

        // Up/Down navigate the category list
        KeyCode::Up => {
            state.category_column_index = state.category_column_index.saturating_sub(1);
            vec![]
        }
        KeyCode::Down => {
            state.category_column_index = (state.category_column_index + 1).min(num_categories - 1);
            vec![]
        }
        KeyCode::Home => {
            state.category_column_index = 0;
            vec![]
        }
        KeyCode::End => {
            state.category_column_index = num_categories - 1;
            vec![]
        }

        // Right/Enter drills into the selected category
        KeyCode::Right | KeyCode::Enter => {
            let cat = categories[state.category_column_index];
            state.category_column_focused = false;
            vec![NavigationAction::SetCategory(cat).into()]
        }

        // Letter jump: L=Library, P=Playlists, G=Genres, F=Folders
        KeyCode::Char(c) if c.is_ascii_alphabetic() && !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let lower = c.to_ascii_lowercase();
            if let Some(idx) = categories.iter().position(|cat| {
                cat.name().starts_with(lower)
            }) {
                state.category_column_index = idx;
            }
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
        KeyCode::F(1) | KeyCode::Char('?') => vec![NavigationAction::SetView(View::Help).into()],

        // Settings
        KeyCode::F(2) => vec![SettingsAction::OpenSettings.into()],

        // Up/Down - navigate within current column, auto-drill into new selection
        // Don't truncate child columns — let auto-drill replace them so the
        // rightmost columns stay visible on screen instead of vanishing.
        KeyCode::Up => {
            if let Some(ref mut folder_state) = state.folder_state {
                folder_state.move_up();
            }
            return auto_drill_folder(state);
        }
        KeyCode::Down => {
            if let Some(ref mut folder_state) = state.folder_state {
                folder_state.move_down();
            }
            return auto_drill_folder(state);
        }
        KeyCode::PageUp => {
            if let Some(ref mut folder_state) = state.folder_state {
                if let Some(col) = folder_state.focused_mut() {
                    col.selected_index = col.selected_index.saturating_sub(10);
                }
            }
            return auto_drill_folder(state);
        }
        KeyCode::PageDown => {
            if let Some(ref mut folder_state) = state.folder_state {
                if let Some(col) = folder_state.focused_mut() {
                    let max = col.items.len().saturating_sub(1);
                    col.selected_index = (col.selected_index + 10).min(max);
                }
            }
            return auto_drill_folder(state);
        }
        KeyCode::Home => {
            if let Some(ref mut folder_state) = state.folder_state {
                if let Some(col) = folder_state.focused_mut() {
                    col.selected_index = 0;
                }
            }
            return auto_drill_folder(state);
        }
        KeyCode::End => {
            if let Some(ref mut folder_state) = state.folder_state {
                if let Some(col) = folder_state.focused_mut() {
                    col.selected_index = col.items.len().saturating_sub(1);
                }
            }
            return auto_drill_folder(state);
        }

        // Right/Enter - go into selected folder; only Enter plays tracks
        KeyCode::Enter | KeyCode::Right => {
            if let Some(ref mut folder_state) = state.folder_state {
                // Check if there's a non-empty column to the right we can move to.
                // Skip empty placeholder columns — they exist only for visual layout
                // and shouldn't intercept navigation.
                let next_col_has_items = folder_state.columns
                    .get(folder_state.focused_column + 1)
                    .map_or(false, |col| !col.items.is_empty());
                if next_col_has_items && folder_state.focus_right() {
                    return vec![];
                }

                // Otherwise, load the selected item
                if let Some(item) = folder_state.selected_item().cloned() {
                    match item.item_type {
                        FolderItemType::Folder => {
                            return vec![FolderAction::NavigateIntoFolder(item.key).into()];
                        }
                        FolderItemType::Track if key.code == KeyCode::Enter => {
                            // Enter: play this track + all following tracks (replaces queue)
                            return vec![FolderAction::PlayFolderTracks.into()];
                        }
                        _ => {}
                    }
                }
            }
            vec![]
        }

        // Left/Backspace - move focus to previous column or category column
        KeyCode::Left | KeyCode::Backspace => {
            if let Some(ref mut folder_state) = state.folder_state {
                if folder_state.can_go_left() {
                    folder_state.focus_left();
                } else {
                    state.focus_category_column();
                }
            } else {
                state.focus_category_column();
            }
            vec![]
        }

        // Escape - go back, or truncate child columns at root
        KeyCode::Esc => {
            if let Some(ref mut folder_state) = state.folder_state {
                if folder_state.can_go_left() {
                    folder_state.focus_left();
                    return vec![];
                }
                // At root column: truncate child columns, keep placeholder
                if folder_state.columns.len() > 1 {
                    folder_state.columns.truncate(1);
                    folder_state.ensure_placeholder();
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
                folder_state.ensure_placeholder();
            }
            vec![]
        }

        _ => vec![],
    }
}

/// Handle Artist browsing with dynamic Miller columns.
pub(super) fn handle_artist_browse_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    use crate::app::state::BrowseItem;

    // Handle common navigation keys
    let is_up_down = matches!(key.code, KeyCode::Up | KeyCode::Down);
    // Left/Backspace at root column → return to category column
    if matches!(key.code, KeyCode::Left | KeyCode::Backspace) && !state.artist_nav.can_go_left() {
        state.focus_category_column();
        return vec![];
    }

    if let Some(mut actions) = handle_browse_nav_keys(key, &mut state.artist_nav) {
        // Auto-drill on Up/Down: always replace child columns so rightmost
        // columns stay visible. Non-drillable items keep stale children visible
        // (better than vanishing columns).
        if is_up_down {
            if let Some(drill) = auto_drill_artist_action(state) {
                state.auto_drill_pending = true;
                actions.push(drill);
            }
        }
        // After scroll, lazily load album art for newly visible items
        let art_batch = super::super::dispatch_miller::collect_viewport_art(state);
        if !art_batch.is_empty() {
            actions.push(SystemAction::LoadAlbumArt(art_batch).into());
        }
        return actions;
    }

    // Handle Enter/Right - drill down into containers; Enter plays tracks
    if matches!(key.code, KeyCode::Enter | KeyCode::Right) {
        if let Some(item) = state.artist_nav.selected_item().cloned() {
            return match item {
                BrowseItem::Artist { key, title, .. } => {
                    state.library.selected_artist_name = title;
                    vec![MillerAction::LoadArtistAlbumsForMiller { artist_key: key }.into()]
                }
                BrowseItem::Album { key, title, .. } => {
                    state.library.selected_album_title = title;
                    vec![MillerAction::LoadAlbumTracksForMiller { album_key: key }.into()]
                }
                BrowseItem::AllArtists => {
                    vec![MillerAction::LoadAllAlbumsForMiller.into()]
                }
                BrowseItem::ArtistRadio { artist_key, artist_name, .. } if key.code == KeyCode::Enter => {
                    vec![RadioAction::StartPlexRadio { key: artist_key, title: artist_name }.into()]
                }
                BrowseItem::ArtistRadio { .. } => {
                    // Right arrow on ArtistRadio is a no-op (not drillable)
                    vec![]
                }
                BrowseItem::AllTracks { artist_key, artist_name, .. } => {
                    if artist_key == "__all_library__" {
                        state.library.selected_album_title = "All Tracks".to_string();
                        vec![MillerAction::LoadAllLibraryTracksForMiller.into()]
                    } else if artist_key == "__all_comp__" {
                        state.library.selected_album_title = "All Tracks".to_string();
                        vec![MillerAction::LoadAllCompilationTracksForMiller.into()]
                    } else if let Some(real_key) = artist_key.strip_prefix("__comp_tracks:") {
                        vec![MillerAction::LoadCompilationAllTracksForMiller {
                            artist_key: real_key.to_string(),
                            artist_name,
                        }.into()]
                    } else {
                        state.library.selected_album_title = format!("All tracks by {}", artist_name);
                        vec![MillerAction::LoadArtistAllTracksForMiller { artist_key }.into()]
                    }
                }
                BrowseItem::Compilations => {
                    vec![MillerAction::LoadCompilationsForMiller.into()]
                }
                BrowseItem::CompilationTracks { artist_key, artist_name } => {
                    vec![MillerAction::LoadCompilationAlbumsForMiller { artist_key, artist_name }.into()]
                }
                BrowseItem::Track { .. } if key.code == KeyCode::Enter => {
                    // Enter: play this track + all following tracks (replaces queue)
                    if let Some(col) = state.artist_nav.focused() {
                        let idx = col.selected_index;
                        vec![MillerAction::PlayTrackFromMiller { column_index: state.artist_nav.focused_column, track_index: idx, single_track: false }.into()]
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

    // Left/Backspace at root column → return to category column
    if matches!(key.code, KeyCode::Left | KeyCode::Backspace) && !state.genre_nav.can_go_left() {
        state.focus_category_column();
        return vec![];
    }

    // Handle common navigation keys
    let is_up_down = matches!(key.code, KeyCode::Up | KeyCode::Down);
    if let Some(mut actions) = handle_browse_nav_keys(key, &mut state.genre_nav) {
        if is_up_down {
            if let Some(drill) = auto_drill_genre_action(state) {
                state.auto_drill_pending = true;
                actions.push(drill);
            }
        }
        // After scroll, lazily load album art for newly visible items
        let art_batch = super::super::dispatch_miller::collect_viewport_art(state);
        if !art_batch.is_empty() {
            actions.push(SystemAction::LoadAlbumArt(art_batch).into());
        }
        return actions;
    }

    // Handle Enter/Right - drill into containers; Enter plays tracks
    if matches!(key.code, KeyCode::Enter | KeyCode::Right) {
        if let Some(item) = state.genre_nav.selected_item().cloned() {
            return match item {
                BrowseItem::GenreCategory { key: cat_key, .. } => {
                    vec![BrowseAction::DrillGenreCategory { category_key: cat_key }.into()]
                }
                BrowseItem::Genre { key, .. } => {
                    vec![MillerAction::LoadGenreAlbumsForMiller { genre_key: key }.into()]
                }
                BrowseItem::Album { key, title, .. } => {
                    // Check grouped-by-album (same as playlist handler)
                    if let Some(col) = state.genre_nav.focused() {
                        if col.grouped_by_album {
                            if let Some(new_col) = helpers::drill_grouped_album(col, col.selected_index) {
                                state.genre_nav.push_column(new_col);
                                return vec![];
                            }
                        }
                    }
                    state.library.selected_album_title = title;
                    vec![MillerAction::LoadGenreTracksForMiller { album_key: key }.into()]
                }
                BrowseItem::Track { .. } if key.code == KeyCode::Enter => {
                    // Enter: play this track + all following tracks (replaces queue)
                    if let Some(col) = state.genre_nav.focused() {
                        let idx = col.selected_index;
                        vec![MillerAction::PlayGenreTrackFromMiller { column_index: state.genre_nav.focused_column, track_index: idx, single_track: false }.into()]
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

    // Left/Backspace at root column → return to category column
    if matches!(key.code, KeyCode::Left | KeyCode::Backspace) && !state.playlist_nav.can_go_left() {
        state.focus_category_column();
        return vec![];
    }

    // Handle common navigation keys
    let is_up_down = matches!(key.code, KeyCode::Up | KeyCode::Down);
    if let Some(mut actions) = handle_browse_nav_keys(key, &mut state.playlist_nav) {
        // Auto-drill on Up/Down: if child column exists, replace it
        if is_up_down {
            let has_child = state.playlist_nav.columns.len() > state.playlist_nav.focused_column + 1;
            if has_child {
                // Check if focused column is grouped-by-album (sync drill)
                let did_sync_drill = {
                    let fc = state.playlist_nav.focused_column;
                    state.playlist_nav.columns.get(fc)
                        .filter(|col| col.grouped_by_album)
                        .and_then(|col| helpers::drill_grouped_album(col, col.selected_index))
                };
                if let Some(new_col) = did_sync_drill {
                    state.playlist_nav.replace_child_column(new_col);
                } else if let Some(drill) = auto_drill_playlist_action(state) {
                    state.auto_drill_pending = true;
                    actions.push(drill);
                }
            }
        }
        // After scroll, lazily load album art for newly visible items
        let art_batch = super::super::dispatch_miller::collect_viewport_art(state);
        if !art_batch.is_empty() {
            actions.push(SystemAction::LoadAlbumArt(art_batch).into());
        }
        return actions;
    }

    // Handle Enter/Right - drill into containers; Enter plays tracks
    if matches!(key.code, KeyCode::Enter | KeyCode::Right) {
        if let Some(item) = state.playlist_nav.selected_item().cloned() {
            return match item {
                BrowseItem::Playlist { key, .. } => {
                    vec![MillerAction::LoadPlaylistTracksForMiller { playlist_key: key }.into()]
                }
                BrowseItem::Album { key, title, .. } => {
                    // Grouped-by-album: drill into local track group
                    if let Some(col) = state.playlist_nav.focused() {
                        if col.grouped_by_album {
                            if let Some(new_col) = helpers::drill_grouped_album(col, col.selected_index) {
                                state.playlist_nav.push_column(new_col);
                                return vec![];
                            }
                        }
                    }
                    state.library.selected_album_title = title;
                    vec![MillerAction::LoadAlbumTracksForMiller { album_key: key }.into()]
                }
                BrowseItem::Track { .. } if key.code == KeyCode::Enter => {
                    // Enter: play this track + all following tracks (replaces queue)
                    if let Some(col) = state.playlist_nav.focused() {
                        let idx = col.selected_index;
                        vec![MillerAction::PlayPlaylistTrackFromMiller { column_index: state.playlist_nav.focused_column, track_index: idx, single_track: false }.into()]
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
        KeyCode::F(1) | KeyCode::Char('?') => Some(vec![NavigationAction::SetView(View::Help).into()]),

        // Settings
        KeyCode::F(2) => Some(vec![SettingsAction::OpenSettings.into()]),

        // Up - move selection up (caller handles auto-drill or truncate)
        KeyCode::Up => {
            nav.move_up();
            Some(vec![])
        }

        // Down - move selection down (caller handles auto-drill or truncate)
        KeyCode::Down => {
            nav.move_down();
            Some(vec![])
        }

        // Page Up
        KeyCode::PageUp => {
            if let Some(col) = nav.focused_mut() {
                col.selected_index = col.selected_index.saturating_sub(10);
            }
            Some(vec![])
        }

        // Page Down
        KeyCode::PageDown => {
            if let Some(col) = nav.focused_mut() {
                let max_idx = col.items.len().saturating_sub(1);
                col.selected_index = (col.selected_index + 10).min(max_idx);
            }
            Some(vec![])
        }

        // Home - go to first item
        KeyCode::Home => {
            if let Some(col) = nav.focused_mut() {
                col.selected_index = 0;
            }
            Some(vec![])
        }

        // End - go to last item
        KeyCode::End => {
            if let Some(col) = nav.focused_mut() {
                col.selected_index = col.items.len().saturating_sub(1);
            }
            Some(vec![])
        }

        // Left/Backspace/Esc - focus previous column
        KeyCode::Left | KeyCode::Backspace | KeyCode::Esc => {
            if nav.can_go_left() {
                nav.focus_left();
            } else if key.code == KeyCode::Esc && nav.columns.len() > 1 {
                // At root column: Esc truncates child columns
                nav.columns.truncate(1);
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
    let category = state.list_filter.category;
    let column = state.list_filter.column;

    match category {
        BrowseCategory::Library => {
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
            if let Some(col) = state.genre_nav.columns.get_mut(column) {
                col.selected_index = item_idx;
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

/// Truncate columns to the right of the filter's target column and move focus there.
/// Call this when the filter query or selection changes to remove stale drill-down columns.
pub fn truncate_filter_right_columns(state: &mut AppState) {
    let column = state.list_filter.column;

    match state.list_filter.category {
        BrowseCategory::Library => {
            state.artist_nav.columns.truncate(column + 1);
            state.artist_nav.focused_column = column;
        }
        BrowseCategory::Playlists => {
            state.playlist_nav.columns.truncate(column + 1);
            state.playlist_nav.focused_column = column;
        }
        BrowseCategory::Genres => {
            state.genre_nav.columns.truncate(column + 1);
            state.genre_nav.focused_column = column;
        }
        BrowseCategory::Folders => {
            if let Some(ref mut fs) = state.folder_state {
                fs.columns.truncate(column + 1);
                fs.focused_column = column;
            }
        }
    }
}

/// Get the drill-down actions for the selected filtered item.
/// This simulates pressing Enter on the selected item to drill into it.
pub fn get_filter_drilldown_actions(state: &mut AppState) -> Vec<Action> {
    let category = state.list_filter.category;

    // Get the appropriate drill-down action based on category
    match category {
        BrowseCategory::Library => {
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
            {
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

/// Determine the auto-drill action for the currently selected item in artist_nav.
/// Returns None if the item is not drillable (e.g. Track, ArtistRadio).
pub(crate) fn auto_drill_artist_action(state: &mut AppState) -> Option<Action> {
    use crate::app::state::BrowseItem;
    let item = state.artist_nav.selected_item()?.clone();
    match item {
        BrowseItem::Artist { key, title, .. } => {
            state.library.selected_artist_name = title;
            Some(MillerAction::LoadArtistAlbumsForMiller { artist_key: key }.into())
        }
        BrowseItem::Album { key, title, .. } => {
            state.library.selected_album_title = title;
            Some(MillerAction::LoadAlbumTracksForMiller { album_key: key }.into())
        }
        BrowseItem::AllArtists => {
            Some(MillerAction::LoadAllAlbumsForMiller.into())
        }
        BrowseItem::AllTracks { artist_key, artist_name, .. } => {
            if artist_key == "__all_library__" {
                state.library.selected_album_title = "All Tracks".to_string();
                Some(MillerAction::LoadAllLibraryTracksForMiller.into())
            } else if artist_key == "__all_comp__" {
                state.library.selected_album_title = "All Tracks".to_string();
                Some(MillerAction::LoadAllCompilationTracksForMiller.into())
            } else if let Some(real_key) = artist_key.strip_prefix("__comp_tracks:") {
                Some(MillerAction::LoadCompilationAllTracksForMiller {
                    artist_key: real_key.to_string(),
                    artist_name,
                }.into())
            } else {
                state.library.selected_album_title = format!("All tracks by {}", artist_name);
                Some(MillerAction::LoadArtistAllTracksForMiller { artist_key }.into())
            }
        }
        BrowseItem::Compilations => {
            Some(MillerAction::LoadCompilationsForMiller.into())
        }
        BrowseItem::CompilationTracks { artist_key, artist_name } => {
            Some(MillerAction::LoadCompilationAlbumsForMiller { artist_key, artist_name }.into())
        }
        // Track, ArtistRadio, Genre, Playlist — not drillable
        _ => None,
    }
}

/// Determine the auto-drill action for the currently selected item in genre_nav.
pub(crate) fn auto_drill_genre_action(state: &AppState) -> Option<Action> {
    use crate::app::state::BrowseItem;
    let item = state.genre_nav.selected_item()?.clone();
    match item {
        BrowseItem::GenreCategory { key, .. } => {
            Some(BrowseAction::DrillGenreCategory { category_key: key }.into())
        }
        BrowseItem::Genre { key, .. } => {
            Some(MillerAction::LoadGenreAlbumsForMiller { genre_key: key }.into())
        }
        BrowseItem::Album { key, .. } => {
            Some(MillerAction::LoadGenreTracksForMiller { album_key: key }.into())
        }
        _ => None,
    }
}

/// Determine the auto-drill action for the currently selected item in playlist_nav.
pub(crate) fn auto_drill_playlist_action(state: &AppState) -> Option<Action> {
    use crate::app::state::BrowseItem;
    let item = state.playlist_nav.selected_item()?.clone();
    match item {
        BrowseItem::Playlist { key, .. } => {
            Some(MillerAction::LoadPlaylistTracksForMiller { playlist_key: key }.into())
        }
        _ => None,
    }
}

/// Auto-drill into the currently selected folder item.
/// Returns a NavigateIntoFolder action if the selected item is a folder,
/// or an empty vec if it's a track or nothing is selected.
pub(crate) fn auto_drill_folder(state: &mut AppState) -> Vec<Action> {
    use crate::services::FolderItemType;

    let item = state.folder_state.as_ref()
        .and_then(|fs| fs.selected_item())
        .cloned();

    if let Some(item) = item {
        if item.item_type == FolderItemType::Folder {
            state.auto_drill_pending = true;
            return vec![FolderAction::NavigateIntoFolder(item.key).into()];
        }
    }
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{AppState, BrowseCategory, BrowseColumn, BrowseItem, BrowseNavigationState};
    use crate::services::{FolderColumn, FolderNavigationState};

    fn make_browse_column(title: &str) -> BrowseColumn {
        BrowseColumn::new(title, vec![
            BrowseItem::Artist { key: "1".into(), title: "A".into(), thumb: None, is_placeholder: false },
        ])
    }

    fn make_folder_column(title: &str) -> FolderColumn {
        FolderColumn::new(None, title.into(), vec![])
    }

    #[test]
    fn test_truncate_folders_removes_right_columns() {
        let mut state = AppState::new();
        state.list_filter.active = true;
        state.list_filter.category = BrowseCategory::Folders;
        state.list_filter.column = 0;

        // Simulate 3 columns (root + 2 drill-downs), focused on column 2
        let mut fs = FolderNavigationState::for_library("lib1".into());
        fs.columns = vec![
            make_folder_column("root"),
            make_folder_column("sub1"),
            make_folder_column("sub2"),
        ];
        fs.focused_column = 2;
        state.folder_state = Some(fs);

        truncate_filter_right_columns(&mut state);

        let fs = state.folder_state.as_ref().unwrap();
        assert_eq!(fs.columns.len(), 1, "should keep only the filter column");
        assert_eq!(fs.focused_column, 0, "focus should move to filter column");
    }

    #[test]
    fn test_truncate_artists_removes_right_columns() {
        let mut state = AppState::new();
        state.list_filter.active = true;
        state.list_filter.category = BrowseCategory::Library;
        state.list_filter.column = 0;

        state.artist_nav = BrowseNavigationState {
            columns: vec![
                make_browse_column("artists"),
                make_browse_column("albums"),
                make_browse_column("tracks"),
            ],
            focused_column: 2,
            loading: false,
        };

        truncate_filter_right_columns(&mut state);

        assert_eq!(state.artist_nav.columns.len(), 1);
        assert_eq!(state.artist_nav.focused_column, 0);
    }

    #[test]
    fn test_truncate_preserves_filter_column() {
        let mut state = AppState::new();
        state.list_filter.active = true;
        state.list_filter.category = BrowseCategory::Folders;
        state.list_filter.column = 1; // filter on second column

        let mut fs = FolderNavigationState::for_library("lib1".into());
        fs.columns = vec![
            make_folder_column("root"),
            make_folder_column("sub1"),
            make_folder_column("sub2"),
        ];
        fs.focused_column = 2;
        state.folder_state = Some(fs);

        truncate_filter_right_columns(&mut state);

        let fs = state.folder_state.as_ref().unwrap();
        assert_eq!(fs.columns.len(), 2, "should keep root + filter column");
        assert_eq!(fs.focused_column, 1, "focus should move to filter column");
    }

    #[test]
    fn test_truncate_noop_when_no_right_columns() {
        let mut state = AppState::new();
        state.list_filter.active = true;
        state.list_filter.category = BrowseCategory::Library;
        state.list_filter.column = 0;

        state.artist_nav = BrowseNavigationState {
            columns: vec![make_browse_column("artists")],
            focused_column: 0,
            loading: false,
        };

        truncate_filter_right_columns(&mut state);

        assert_eq!(state.artist_nav.columns.len(), 1);
        assert_eq!(state.artist_nav.focused_column, 0);
    }
}

/// Navigate forward through main views.
/// Order: Library → Playlists → Queue → Now Playing → Library
/// Genre/Folder categories are accessed via Ctrl+G / Ctrl+O, not Tab.
fn tab_navigate_next(state: &mut AppState) -> Vec<Action> {
    // Browse → Queue (categories are navigated via the category column, not Tab)
    state.set_view(View::Queue);
    vec![]
}

/// Navigate backward through main views.
/// Order: Browse ← Queue ← Now Playing ← Browse
fn tab_navigate_prev(state: &mut AppState) -> Vec<Action> {
    // Browse → wrap to Now Playing
    state.set_view(View::NowPlaying);
    vec![]
}
