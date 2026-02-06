//! Keyboard input handler functions.
//!
//! All keyboard event processing extracted from the event loop as free functions.

use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::app::Action;
use crate::app::state::{
    BrowseCategory, Focus, PlaybackMode, RightPanelMode, SearchSection, SearchTab, View,
};
use crate::app::AppState;
use crate::api::models::Track;
use crate::plex::PlexAuth;
use super::helpers;

/// Handle keyboard input (CUA-style with Ctrl shortcuts).
pub fn handle_key(key: event::KeyEvent, state: &mut AppState, config: &crate::config::Config) -> Vec<Action> {
    // Track Alt key state for bottom bar display
    state.alt_held = key.modifiers.contains(KeyModifiers::ALT);

    // Clear error on any key
    if state.last_error.is_some() {
        state.clear_error();
        return vec![];
    }

    // Handle confirm dialog if active
    if state.confirm_dialog.is_some() {
        match key.code {
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                state.confirm_dialog = None;
                return helpers::refresh_current_view(state);
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                state.confirm_dialog = None;
                return vec![];
            }
            _ => return vec![],
        }
    }

    // Handle input dialog if active
    if let Some(ref mut dialog) = state.input_dialog {
        match key.code {
            KeyCode::Esc => {
                // Cancel dialog and adventure if it was for adventure length
                let was_adventure = matches!(dialog.action_type, crate::app::state::InputDialogAction::AdventureLength);
                state.input_dialog = None;
                if was_adventure {
                    return vec![Action::CancelAdventure];
                }
            }
            KeyCode::Enter => {
                // Confirm dialog
                let input = dialog.input.clone();
                let action_type = dialog.action_type.clone();
                state.input_dialog = None;
                match action_type {
                    crate::app::state::InputDialogAction::SavePlaylist => {
                        return vec![Action::SaveQueueAsPlaylist(input)];
                    }
                    crate::app::state::InputDialogAction::AdventureLength => {
                        // Parse the length (default to 20)
                        let length = input.parse::<usize>().unwrap_or(20).clamp(5, 100);
                        return vec![Action::SetAdventureLength(length)];
                    }
                }
            }
            KeyCode::Backspace => {
                dialog.input.pop();
            }
            KeyCode::Char(c) => {
                // For adventure length, only allow digits
                if matches!(dialog.action_type, crate::app::state::InputDialogAction::AdventureLength) {
                    if c.is_ascii_digit() && dialog.input.len() < 3 {
                        dialog.input.push(c);
                    }
                } else {
                    // Allow all printable characters for other dialogs
                    if dialog.input.len() < 100 {
                        dialog.input.push(c);
                    }
                }
            }
            _ => {}
        }
        return vec![];
    }

    // Handle adventure mode Esc separately
    if state.adventure.active && !state.adventure.generating {
        if key.code == KeyCode::Esc {
            return vec![Action::CancelAdventure];
        }
    }

    // Global CUA shortcuts (work everywhere)
    match (key.modifiers, key.code) {
        // Quit: Ctrl+Q
        (KeyModifiers::CONTROL, KeyCode::Char('q')) => return vec![Action::Quit],

        // Global navigation shortcuts
        (KeyModifiers::CONTROL, KeyCode::Char('f')) => {
            // Ctrl+F = Search/Filter popup (floating dialog)
            if state.search_popup_active {
                return vec![Action::CloseSearchPopup];
            } else {
                return vec![Action::OpenSearchPopup];
            }
        }
        (KeyModifiers::CONTROL, KeyCode::Char('g')) => {
            // Ctrl+G = Genres category, or cycle content type if already there
            if state.view == View::Browse && state.browse_category == BrowseCategory::Genres {
                // Already in genres view - cycle content type
                return vec![Action::CycleGenreContentType];
            }
            // Not in genres view - switch to it and reset right panel
            state.browse_category = BrowseCategory::Genres;
            reset_right_panel(state);
            // Load the appropriate content based on current type
            let load_action = match state.genre_content_type {
                crate::app::state::GenreContentType::Genres => Action::LoadGenres,
                crate::app::state::GenreContentType::ArtistGenres => Action::LoadArtistGenres,
                crate::app::state::GenreContentType::AlbumGenres => Action::LoadAlbumGenres,
                crate::app::state::GenreContentType::Moods => Action::LoadMoods,
                crate::app::state::GenreContentType::Styles => Action::LoadStyles,
                crate::app::state::GenreContentType::Stations => Action::LoadStations,
            };
            return vec![load_action, Action::SetView(View::Browse)];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
            // Ctrl+N = Now Playing, or cycle mode if already there
            if state.view == View::NowPlaying {
                // Already in Now Playing - cycle mode (Queue → Recently Played)
                return vec![Action::CycleNowPlayingMode];
            }
            return vec![Action::SetView(View::NowPlaying)];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('s')) => {
            // Ctrl+S = Save queue/radio as playlist (in Now Playing with tracks)
            if state.view == View::NowPlaying {
                let has_tracks = !state.queue.is_empty() || !state.radio.tracks.is_empty();
                if has_tracks {
                    return vec![Action::PromptSavePlaylist];
                }
            }
        }
        (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
            // Ctrl+A = Artists category, or cycle view mode if already there
            if state.view == View::Browse && state.browse_category == BrowseCategory::Artists {
                // Already in artists view - cycle view mode (Artist → Album Artist → Album)
                return vec![Action::CycleArtistViewMode];
            }
            // Not in artists view - switch to it and reset right panel
            state.browse_category = BrowseCategory::Artists;
            reset_right_panel(state);
            // Only load if data not already preloaded
            let needs_load = match state.artist_view_mode {
                crate::app::state::ArtistViewMode::Artist |
                crate::app::state::ArtistViewMode::AlbumArtist => state.artists.is_empty(),
                crate::app::state::ArtistViewMode::Album => state.albums.is_empty(),
            };
            if needs_load {
                let load_action = match state.artist_view_mode {
                    crate::app::state::ArtistViewMode::Artist |
                    crate::app::state::ArtistViewMode::AlbumArtist => Action::LoadArtists,
                    crate::app::state::ArtistViewMode::Album => Action::LoadAlbums,
                };
                return vec![load_action, Action::SetView(View::Browse)];
            }
            return vec![Action::SetView(View::Browse)];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
            // Ctrl+P = Playlists category, or cycle mode if already there
            if state.view == View::Browse && state.browse_category == BrowseCategory::Playlists {
                // Already in Playlists - cycle mode (All → Recently Added → Recent)
                return vec![Action::CyclePlaylistsMode];
            }
            // Not in Playlists - switch to it and reset right panel
            state.browse_category = BrowseCategory::Playlists;
            reset_right_panel(state);
            if state.playlists.is_empty() {
                return vec![Action::LoadPlaylists, Action::SetView(View::Browse)];
            }
            return vec![Action::SetView(View::Browse)];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('o')) => {
            // Ctrl+O = Folders category
            state.browse_category = BrowseCategory::Folders;
            reset_right_panel(state);
            if state.folder_state.is_none() {
                return vec![Action::LoadFolderRoot, Action::SetView(View::Browse)];
            }
            return vec![Action::SetView(View::Browse)];
        }

        // Global function keys - work from any screen
        (_, KeyCode::F(1)) => {
            if state.view != View::Help {
                return vec![Action::SetView(View::Help)];
            }
        }
        (_, KeyCode::F(2)) => {
            if state.view != View::Settings {
                return vec![Action::OpenSettings];
            }
        }
        (_, KeyCode::F(5)) => {
            // F5 = Refresh current view
            return helpers::refresh_current_view(state);
        }

        // Playback controls with Ctrl
        (KeyModifiers::CONTROL, KeyCode::Char(' ')) |
        (_, KeyCode::Char(' ')) if state.view != View::Search => {
            return vec![Action::TogglePlayPause];
        }
        (KeyModifiers::CONTROL, KeyCode::Left) => return vec![Action::Previous],
        (KeyModifiers::CONTROL, KeyCode::Right) => return vec![Action::Next],
        // < and > for prev/next track (crossterm reports these with NONE modifiers, not SHIFT)
        (_, KeyCode::Char('<')) if state.view != View::Search && !state.list_filter_active && !state.search_popup_active => {
            return vec![Action::Previous];
        }
        (_, KeyCode::Char('>')) if state.view != View::Search && !state.list_filter_active && !state.search_popup_active => {
            return vec![Action::Next];
        }
        (KeyModifiers::CONTROL, KeyCode::Up) => return vec![Action::VolumeUp],
        (KeyModifiers::CONTROL, KeyCode::Down) => return vec![Action::VolumeDown],
        // Shift+Left/Right for seeking (10 second skip)
        (KeyModifiers::SHIFT, KeyCode::Left) => return vec![Action::SeekRelative(-10000)],
        (KeyModifiers::SHIFT, KeyCode::Right) => return vec![Action::SeekRelative(10000)],

        // Alt key commands (global)
        (KeyModifiers::ALT, KeyCode::Char('r')) => {
            // Alt+R = Create radio from current selection
            return create_station_from_context(state);
        }
        (KeyModifiers::ALT, KeyCode::Char('e')) => {
            // Alt+E = Enqueue selection
            return vec![Action::EnqueueSelection];
        }
        (KeyModifiers::ALT, KeyCode::Char('o')) => {
            // Alt+O = Toggle queue shuffle
            if !state.queue.is_empty() {
                return vec![Action::ToggleQueueShuffle];
            }
        }
        (KeyModifiers::ALT, KeyCode::Char('s')) => {
            // Alt+S = Similar albums/tracks for current context
            return get_similar_action(state);
        }
        (KeyModifiers::ALT, KeyCode::Char('v')) => {
            // Alt+V = Sonic Adventure
            return handle_adventure_key(state);
        }
        // Ctrl+Alt shortcuts
        (mods, KeyCode::Char('l')) if mods == KeyModifiers::CONTROL | KeyModifiers::ALT => {
            // Ctrl+Alt+L = Play Library Radio station
            if let Some(lib_key) = &state.active_library {
                let key = format!("/library/sections/{}/stations/library", lib_key);
                return vec![Action::PlayStation(key)];
            }
            return vec![];
        }
        (mods, KeyCode::Char('r')) if mods == KeyModifiers::CONTROL | KeyModifiers::ALT => {
            // Ctrl+Alt+R = Play Random Album Radio station
            if let Some(lib_key) = &state.active_library {
                let key = format!("/library/sections/{}/stations/randomAlbum", lib_key);
                return vec![Action::PlayStation(key)];
            }
            return vec![];
        }
        (mods, KeyCode::Char('s')) if mods == KeyModifiers::CONTROL | KeyModifiers::ALT => {
            // Ctrl+Alt+S = Quick library switcher
            if !state.libraries.is_empty() {
                return vec![Action::OpenLibraryPicker];
            }
            return vec![];
        }

        _ => {}
    }

    // Search popup handling (takes priority over view-specific handling)
    if state.search_popup_active {
        return handle_search_keys(key, state);
    }

    // Library picker popup handling
    if state.library_picker_active {
        return handle_library_picker_keys(key, state);
    }

    // View-specific handling
    match state.view {
        View::Auth => handle_auth_keys(key, state),
        View::Browse => handle_browse_keys(key, state),
        View::NowPlaying => handle_now_playing_keys(key, state),
        View::Search => handle_search_keys(key, state),
        View::Similar => handle_similar_keys(key, state),
        View::Help => handle_help_keys(key, state),
        View::Settings => handle_settings_keys(key, state, config),
    }
}

/// Handle keys when library picker popup is active.
fn handle_library_picker_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    let lib_count = state.libraries.len();
    if lib_count == 0 {
        state.library_picker_active = false;
        return vec![];
    }

    match key.code {
        KeyCode::Esc => {
            return vec![Action::CloseLibraryPicker];
        }
        KeyCode::Up => {
            if state.library_picker_index > 0 {
                state.library_picker_index -= 1;
            }
        }
        KeyCode::Down => {
            if state.library_picker_index + 1 < lib_count {
                state.library_picker_index += 1;
            }
        }
        KeyCode::Home => {
            state.library_picker_index = 0;
        }
        KeyCode::End => {
            state.library_picker_index = lib_count.saturating_sub(1);
        }
        KeyCode::Enter => {
            if let Some(lib) = state.libraries.get(state.library_picker_index) {
                let key = lib.key.clone();
                return vec![Action::SelectLibrary(key), Action::CloseLibraryPicker];
            }
        }
        _ => {} // Absorb all other keys
    }
    vec![]
}

fn handle_auth_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    use crate::app::state::AuthStep;

    match state.auth_state.step {
        AuthStep::Checking | AuthStep::Authenticating | AuthStep::Connecting => {
            // No input during these states
            vec![]
        }
        AuthStep::Login => {
            if state.auth_state.editing {
                // Text input mode
                match key.code {
                    KeyCode::Char(c) => {
                        if state.auth_state.field_index == 0 {
                            state.auth_state.username_input.push(c);
                        } else if state.auth_state.field_index == 1 {
                            state.auth_state.password_input.push(c);
                        }
                        vec![]
                    }
                    KeyCode::Backspace => {
                        if state.auth_state.field_index == 0 {
                            state.auth_state.username_input.pop();
                        } else if state.auth_state.field_index == 1 {
                            state.auth_state.password_input.pop();
                        }
                        vec![]
                    }
                    KeyCode::Enter => {
                        // Stop editing, move to next field or submit
                        state.auth_state.editing = false;
                        if state.auth_state.field_index < 2 {
                            state.auth_state.field_index += 1;
                        }
                        // If we're now on the sign in button, submit
                        if state.auth_state.field_index == 2 {
                            return vec![Action::AuthSignIn];
                        }
                        vec![]
                    }
                    KeyCode::Esc => {
                        state.auth_state.editing = false;
                        vec![]
                    }
                    KeyCode::Tab => {
                        // Move to next field while editing
                        state.auth_state.editing = false;
                        state.auth_state.field_index = (state.auth_state.field_index + 1) % 3;
                        vec![]
                    }
                    _ => vec![],
                }
            } else {
                // Navigation mode
                match key.code {
                    KeyCode::Up => {
                        if state.auth_state.field_index > 0 {
                            state.auth_state.field_index -= 1;
                        }
                        vec![]
                    }
                    KeyCode::Down | KeyCode::Tab => {
                        if state.auth_state.field_index < 2 {
                            state.auth_state.field_index += 1;
                        }
                        vec![]
                    }
                    KeyCode::BackTab => {
                        if state.auth_state.field_index > 0 {
                            state.auth_state.field_index -= 1;
                        }
                        vec![]
                    }
                    KeyCode::Enter => {
                        if state.auth_state.field_index == 2 {
                            // Sign In button
                            vec![Action::AuthSignIn]
                        } else {
                            // Start editing the field
                            state.auth_state.editing = true;
                            vec![]
                        }
                    }
                    KeyCode::Char(c) => {
                        // Start editing and add the character (for username/password fields)
                        if state.auth_state.field_index < 2 {
                            state.auth_state.editing = true;
                            if state.auth_state.field_index == 0 {
                                state.auth_state.username_input.push(c);
                            } else {
                                state.auth_state.password_input.push(c);
                            }
                        }
                        vec![]
                    }
                    _ => vec![],
                }
            }
        }
        AuthStep::ServerSelect => {
            match key.code {
                KeyCode::Up => {
                    if state.auth_state.server_index > 0 {
                        state.auth_state.server_index -= 1;
                    }
                    vec![]
                }
                KeyCode::Down => {
                    if state.auth_state.server_index + 1 < state.available_servers.len() {
                        state.auth_state.server_index += 1;
                    }
                    vec![]
                }
                KeyCode::Enter => {
                    vec![Action::AuthSelectServer]
                }
                _ => vec![],
            }
        }
    }
}

/// Handle Browse view keys (CUA-style).
fn handle_browse_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    // Inline list filter mode - handle filter-specific keys
    if state.list_filter_active {
        // Check if current focus is on the filter's target column
        use crate::app::state::GenreContentType;
        let focused_on_filter_column = match state.list_filter_category {
            BrowseCategory::Artists => state.artist_nav.focused_column == state.list_filter_column,
            BrowseCategory::Playlists => {
                if state.playlists_mode == crate::app::state::PlaylistsMode::Stations {
                    state.station_nav.focused_column == state.list_filter_column
                } else {
                    state.playlist_nav.focused_column == state.list_filter_column
                }
            }
            BrowseCategory::Genres => {
                if state.genre_content_type == GenreContentType::Stations {
                    state.station_nav.focused_column == state.list_filter_column
                } else {
                    state.genre_nav.focused_column == state.list_filter_column
                }
            }
            BrowseCategory::Folders => {
                state.folder_state.as_ref()
                    .map(|fs| fs.focused_column == state.list_filter_column)
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
        return create_station_from_context(state);
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
                        state.stations_index = col.selected_index;
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
            jump_to_letter(state, c);
            vec![]
        }

        _ => vec![],
    }
}

/// Handle folder browsing mode keys (Miller columns style).
fn handle_folder_browse_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
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
fn handle_station_browse_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
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
            // Update legacy state
            if let Some(col) = state.station_nav.focused() {
                state.stations_index = col.selected_index;
            }
            vec![]
        }
        KeyCode::Down => {
            state.station_nav.move_down();
            // Clear columns to the right since selection changed
            state.station_nav.truncate_right_columns();
            // Update legacy state
            if let Some(col) = state.station_nav.focused() {
                state.stations_index = col.selected_index;
            }
            vec![]
        }
        KeyCode::PageUp => {
            if let Some(col) = state.station_nav.focused_mut() {
                col.selected_index = col.selected_index.saturating_sub(10);
            }
            state.station_nav.truncate_right_columns();
            if let Some(col) = state.station_nav.focused() {
                state.stations_index = col.selected_index;
            }
            vec![]
        }
        KeyCode::PageDown => {
            if let Some(col) = state.station_nav.focused_mut() {
                let max = col.stations.len().saturating_sub(1);
                col.selected_index = (col.selected_index + 10).min(max);
            }
            state.station_nav.truncate_right_columns();
            if let Some(col) = state.station_nav.focused() {
                state.stations_index = col.selected_index;
            }
            vec![]
        }
        KeyCode::Home => {
            if let Some(col) = state.station_nav.focused_mut() {
                col.selected_index = 0;
            }
            state.station_nav.truncate_right_columns();
            if let Some(col) = state.station_nav.focused() {
                state.stations_index = col.selected_index;
            }
            vec![]
        }
        KeyCode::End => {
            if let Some(col) = state.station_nav.focused_mut() {
                col.selected_index = col.stations.len().saturating_sub(1);
            }
            state.station_nav.truncate_right_columns();
            if let Some(col) = state.station_nav.focused() {
                state.stations_index = col.selected_index;
            }
            vec![]
        }

        // Right/Enter - drill into category or play station
        KeyCode::Enter | KeyCode::Right => {
            // First check if there's already a column to the right we can move to
            if state.station_nav.focus_right() {
                // Update legacy state
                if let Some(col) = state.station_nav.focused() {
                    state.stations = col.stations.clone();
                    state.stations_index = col.selected_index;
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
                    state.stations_index = col.selected_index;
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
                    state.stations_index = col.selected_index;
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
            if let Some(col) = state.station_nav.focused() {
                state.stations_index = col.selected_index;
            }
            vec![]
        }

        _ => vec![],
    }
}

/// Handle Artist browsing with dynamic Miller columns.
fn handle_artist_browse_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
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
fn handle_genre_browse_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
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
/// Handle Playlist browsing with dynamic Miller columns
fn handle_playlist_browse_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
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

/// Get the similar albums/tracks action based on current context.
fn get_similar_action(state: &mut AppState) -> Vec<Action> {
    // Store current view so we can return to it
    state.previous_view = Some(state.view);

    // In Now Playing view, use the selected track
    if state.view == View::NowPlaying {
        let track = match state.playback_mode {
            PlaybackMode::Queue | PlaybackMode::None => {
                state.queue.get(state.list_state.queue_index).cloned()
            }
            PlaybackMode::Radio => {
                state.radio.tracks.get(state.list_state.queue_index).cloned()
            }
        };
        if let Some(track) = track {
            let title = format!("{} - {}", track.artist_name(), track.title);
            return vec![Action::LoadSimilarTracks {
                rating_key: track.rating_key.clone(),
                title,
            }];
        }
    }
    // When in right panel showing albums for an artist, use selected album
    // Index 0 is "All Tracks", so skip it for similar albums
    else if state.focus == Focus::Right && state.right_panel_mode == RightPanelMode::ArtistAlbums {
        let album_idx = state.list_state.right_albums_index.saturating_sub(1);
        if state.list_state.right_albums_index > 0 {
            if let Some(album) = state.selected_artist_albums.get(album_idx) {
                let title = format!("{} - {}", album.artist_name(), album.title);
                return vec![Action::LoadSimilarAlbums {
                    rating_key: album.rating_key.clone(),
                    title,
                }];
            }
        }
    }
    // When in genre albums, use selected album
    else if state.focus == Focus::Right && state.right_panel_mode == RightPanelMode::CategoryAlbums {
        if let Some(album) = state.genre_albums.get(state.genre_albums_index) {
            let title = format!("{} - {}", album.artist_name(), album.title);
            return vec![Action::LoadSimilarAlbums {
                rating_key: album.rating_key.clone(),
                title,
            }];
        }
    }
    // When viewing tracks, use the selected track
    else if state.focus == Focus::Right && (state.right_panel_mode == RightPanelMode::AlbumTracks || state.right_panel_mode == RightPanelMode::CategoryTracks) {
        if let Some(track) = state.selected_album_tracks.get(state.list_state.tracks_index) {
            let title = format!("{} - {}", track.artist_name(), track.title);
            return vec![Action::LoadSimilarTracks {
                rating_key: track.rating_key.clone(),
                title,
            }];
        }
    }
    // Otherwise, use the now-playing track
    else if let Some(track) = state.current_track().cloned() {
        let title = format!("{} - {}", track.artist_name(), track.title);
        return vec![Action::LoadSimilarTracks {
            rating_key: track.rating_key.clone(),
            title,
        }];
    }
    vec![]
}

/// Reset right panel state when switching categories.
/// Clears album/track selections and resets focus to left panel.
fn reset_right_panel(state: &mut AppState) {
    state.right_panel_mode = RightPanelMode::Empty;
    state.focus = Focus::Left;
    state.selected_artist_albums.clear();
    state.selected_album_tracks.clear();
    state.genre_albums.clear();
    state.genre_albums_index = 0;
    state.selected_artist_name.clear();
    state.selected_album_title.clear();
}

/// Create a station from current context (artist, album, or track).
/// Track selected -> Track radio (individual similar tracks)
/// Album selected -> Album radio (similar albums played in order)
/// Artist selected -> Artist radio
fn create_station_from_context(state: &AppState) -> Vec<Action> {
    // If viewing album tracks, create TRACK radio for the highlighted track
    if state.focus == Focus::Right && state.right_panel_mode == RightPanelMode::AlbumTracks {
        if let Some(track) = state.selected_album_tracks.get(state.list_state.tracks_index) {
            let title = format!("{} - {}", track.artist_name(), track.title);
            return vec![Action::StartTrackRadio {
                track_key: track.rating_key.clone(),
                title,
            }];
        }
    }
    // If viewing category tracks (playlist, etc), create TRACK radio
    else if state.focus == Focus::Right && state.right_panel_mode == RightPanelMode::CategoryTracks {
        if let Some(track) = state.selected_album_tracks.get(state.list_state.tracks_index) {
            let title = format!("{} - {}", track.artist_name(), track.title);
            return vec![Action::StartTrackRadio {
                track_key: track.rating_key.clone(),
                title,
            }];
        }
    }
    // If viewing artist albums, check what's selected
    else if state.focus == Focus::Right && state.right_panel_mode == RightPanelMode::ArtistAlbums {
        // Index 0 is "All Tracks" - create artist radio
        if state.list_state.right_albums_index == 0 {
            if let Some(artist) = state.artists.get(state.list_state.artists_index) {
                return vec![Action::StartArtistRadio {
                    artist_key: artist.rating_key.clone(),
                    title: artist.title.clone(),
                }];
            }
        }
        // Otherwise, create album radio for the selected album
        else if let Some(album) = state.selected_artist_albums.get(state.list_state.right_albums_index - 1) {
            return vec![Action::StartAlbumRadio {
                album_key: album.rating_key.clone(),
                title: album.title.clone(),
            }];
        }
    }
    // If viewing genre/mood albums, create album radio
    else if state.focus == Focus::Right && state.right_panel_mode == RightPanelMode::CategoryAlbums {
        if let Some(album) = state.genre_albums.get(state.genre_albums_index) {
            return vec![Action::StartAlbumRadio {
                album_key: album.rating_key.clone(),
                title: album.title.clone(),
            }];
        }
    }
    // If focused on left panel artist, create artist radio
    else if state.focus == Focus::Left && state.browse_category == BrowseCategory::Artists {
        if let Some(artist) = state.artists.get(state.list_state.artists_index) {
            return vec![Action::StartArtistRadio {
                artist_key: artist.rating_key.clone(),
                title: artist.title.clone(),
            }];
        }
    }
    // Otherwise, use the current playing track
    else if let Some(track) = state.current_track() {
        let title = format!("{} - {}", track.artist_name(), track.title);
        return vec![Action::StartTrackRadio {
            track_key: track.rating_key.clone(),
            title,
        }];
    }
    vec![]
}

/// Handle Alt+V for Sonic Adventure.
fn handle_adventure_key(state: &mut AppState) -> Vec<Action> {
    // Ignore if already generating
    if state.adventure.generating {
        return vec![];
    }

    // Get the currently selected/highlighted track
    let selected_track = get_selected_track(state);

    if !state.adventure.active {
        // Start adventure mode
        if let Some(track) = selected_track {
            return vec![Action::SetAdventureStart(track)];
        } else {
            return vec![Action::StartAdventure];
        }
    }

    // Adventure mode is active
    if state.adventure.start_track.is_some() && state.adventure.end_track.is_none() {
        // Set end track
        if let Some(track) = selected_track {
            return vec![Action::SetAdventureEnd(track)];
        }
    }

    vec![]
}

/// Get the currently selected/highlighted track based on context.
/// Returns the track the user is highlighting in any view where tracks are visible.
fn get_selected_track(state: &AppState) -> Option<Track> {
    match state.view {
        // Search/Filter view - handle both Global search and tab-specific filters
        View::Search => {
            let idx = state.list_state.search_item_index;

            match state.search_tab {
                // Global search - uses search_results with sections
                SearchTab::Global => {
                    if state.list_state.search_section == SearchSection::Tracks {
                        if let Some(ref results) = state.search_results {
                            return results.tracks.get(idx).cloned();
                        }
                    }
                    None
                }
                // Tracks tab - uses filter_results
                SearchTab::Tracks => {
                    if let Some(ref results) = state.filter_results {
                        return results.tracks.get(idx).cloned();
                    }
                    None
                }
                // Other tabs don't show tracks directly
                _ => None
            }
        }

        // Now Playing view - get highlighted track from queue or radio
        View::NowPlaying => {
            let idx = state.list_state.queue_index;
            match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => {
                    // Account for play history offset
                    let history_len = state.play_history.len();
                    if idx < history_len {
                        state.play_history.get(idx).cloned()
                    } else {
                        state.queue.get(idx - history_len).cloned()
                    }
                }
                PlaybackMode::Radio => {
                    state.radio.tracks.get(idx).cloned()
                }
            }
        }

        // Browse view - check if tracks are showing in right panel
        View::Browse => {
            match state.right_panel_mode {
                RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                    state.selected_album_tracks.get(state.list_state.tracks_index).cloned()
                }
                _ => None
            }
        }

        // Similar view - check if showing similar tracks
        View::Similar => {
            // Similar view shows albums by default, not individual tracks
            None
        }

        // Other views don't show selectable tracks
        _ => None
    }
}

/// Jump to first item in current list starting with given letter.
/// Uses sort_key logic to match the sorting (ignores "The " prefix).
fn jump_to_letter(state: &mut AppState, letter: char) {
    let letter_lower = letter.to_ascii_lowercase();

    // Check if sort key starts with the given letter (matches sorting logic)
    let starts_with = |title: &str| -> bool {
        helpers::sort_key(title).chars().next()
            .map(|c| c.to_ascii_lowercase() == letter_lower)
            .unwrap_or(false)
    };

    if state.focus == Focus::Left {
        // Jump in category list
        match state.browse_category {
            BrowseCategory::Artists => {
                if let Some(idx) = state.artists.iter().position(|a| starts_with(&a.title)) {
                    state.list_state.artists_index = idx;
                }
            }
            BrowseCategory::Playlists => {
                if let Some(idx) = state.playlists.iter().position(|p| starts_with(&p.title)) {
                    state.list_state.playlists_index = idx;
                }
            }
            BrowseCategory::Genres => {
                // Stations are now accessed via genre content type
                if state.genre_content_type == crate::app::state::GenreContentType::Stations {
                    if let Some(idx) = state.stations.iter().position(|s| starts_with(&s.title)) {
                        state.stations_index = idx;
                    }
                } else if let Some(idx) = state.genres.iter().position(|g| starts_with(&g.title)) {
                    state.genres_index = idx;
                }
            }
            BrowseCategory::Folders => {
                // Handled separately in folder navigation
            }
        }
    } else {
        // Jump in right panel
        match state.right_panel_mode {
            RightPanelMode::ArtistAlbums => {
                // +1 offset for "All Tracks" at index 0
                if let Some(idx) = state.selected_artist_albums.iter().position(|a| starts_with(&a.title)) {
                    state.list_state.right_albums_index = idx + 1;
                }
            }
            RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                if let Some(idx) = state.selected_album_tracks.iter().position(|t| starts_with(&t.title)) {
                    state.list_state.tracks_index = idx;
                }
            }
            RightPanelMode::CategoryAlbums => {
                if let Some(idx) = state.genre_albums.iter().position(|a| starts_with(&a.title)) {
                    state.genre_albums_index = idx;
                }
            }
            RightPanelMode::Empty => {}
        }
    }
}

/// Handle Queue view keys (CUA-style).
/// Handle Now Playing view keys (unified queue/radio/playlist view).
fn handle_now_playing_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    // Get the max index based on current mode
    let get_max_index = |state: &AppState| -> usize {
        match state.playback_mode {
            PlaybackMode::Queue | PlaybackMode::None => state.queue.len().saturating_sub(1),
            PlaybackMode::Radio => state.radio.tracks.len().saturating_sub(1),
        }
    };

    match key.code {
        KeyCode::Esc => vec![Action::SetView(View::Browse)],
        KeyCode::F(1) | KeyCode::Char('?') => vec![Action::SetView(View::Help)],

        // Tab/Shift+Tab cycles through nav bar views
        KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => vec![Action::PrevView],
        KeyCode::Tab => vec![Action::NextView],

        KeyCode::Up => {
            if state.list_state.queue_index > 0 {
                state.list_state.queue_index -= 1;
            }
            vec![]
        }
        KeyCode::Down => {
            let max = get_max_index(state);
            state.list_state.queue_index = (state.list_state.queue_index + 1).min(max);
            vec![]
        }
        KeyCode::PageUp => {
            state.list_state.queue_index = state.list_state.queue_index.saturating_sub(10);
            vec![]
        }
        KeyCode::PageDown => {
            let max = get_max_index(state);
            state.list_state.queue_index = (state.list_state.queue_index + 10).min(max);
            vec![]
        }
        KeyCode::Home => {
            state.list_state.queue_index = 0;
            vec![]
        }
        KeyCode::End => {
            let max = get_max_index(state);
            state.list_state.queue_index = max;
            vec![]
        }

        KeyCode::Enter => {
            // If the selected track is already playing, switch to NowPlaying view
            let is_current = match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => state.queue_index == Some(state.list_state.queue_index),
                PlaybackMode::Radio => state.radio.track_index == Some(state.list_state.queue_index),
            };
            if is_current {
                state.now_playing_mode = crate::app::state::NowPlayingMode::NowPlaying;
                return vec![Action::LoadWaveform];
            }

            // Play selected item from queue or radio
            match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => {
                    if let Some(track) = state.queue.get(state.list_state.queue_index).cloned() {
                        state.queue_index = Some(state.list_state.queue_index);
                        vec![Action::PlayTrack(track)]
                    } else {
                        vec![]
                    }
                }
                PlaybackMode::Radio => {
                    // Jump to selected radio track without clearing radio state
                    if state.list_state.queue_index < state.radio.tracks.len() {
                        vec![Action::JumpToRadioTrack(state.list_state.queue_index)]
                    } else {
                        vec![]
                    }
                }
            }
        }

        KeyCode::Delete => {
            // Only allow delete in queue mode
            if state.playback_mode == PlaybackMode::Queue {
                vec![Action::RemoveFromQueue(state.list_state.queue_index)]
            } else {
                vec![]
            }
        }

        // Left/Right arrow seeking in visualizer mode (1 second increments)
        KeyCode::Left if state.now_playing_mode == crate::app::state::NowPlayingMode::NowPlaying => {
            vec![Action::SeekRelative(-1000)]
        }
        KeyCode::Right if state.now_playing_mode == crate::app::state::NowPlayingMode::NowPlaying => {
            vec![Action::SeekRelative(1000)]
        }

        // Alphabet jumping
        KeyCode::Char(c) if c.is_ascii_alphabetic() && key.modifiers.is_empty() => {
            let letter_lower = c.to_ascii_lowercase();
            let tracks: &[Track] = match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => &state.queue,
                PlaybackMode::Radio => &state.radio.tracks,
            };
            if let Some(idx) = tracks.iter().position(|t| {
                t.title.chars().next()
                    .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                    .unwrap_or(false)
            }) {
                state.list_state.queue_index = idx;
            }
            vec![]
        }

        _ => vec![],
    }
}

/// Handle unified Search view keys (with tabs for Global/Artists/Playlists/Tracks/Genres).
fn handle_search_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    use crate::app::state::SearchTab;

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

/// Handle Similar view keys (CUA-style).
fn handle_similar_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    use crate::app::state::SimilarMode;

    match key.code {
        KeyCode::Esc => {
            // Return to previous view, or Browse if none
            let target = state.previous_view.take().unwrap_or(View::Browse);
            vec![Action::SetView(target)]
        }
        KeyCode::F(1) | KeyCode::Char('?') => vec![Action::SetView(View::Help)],

        KeyCode::Up => vec![Action::ListUp],
        KeyCode::Down => vec![Action::ListDown],
        KeyCode::PageUp => vec![Action::ListPageUp],
        KeyCode::PageDown => vec![Action::ListPageDown],
        KeyCode::Home => vec![Action::ListTop],
        KeyCode::End => vec![Action::ListBottom],

        KeyCode::Enter => {
            match state.similar_mode {
                SimilarMode::Albums => {
                    // Navigate to selected similar album - show as artist's album view
                    if let Some(album) = state.similar_albums.get(state.list_state.similar_index).cloned() {
                        state.pending_album_key = Some(album.rating_key.clone());
                        state.selected_album_title = album.title.clone();
                        state.selected_artist_name = album.artist_name().to_string();
                        state.view = View::Browse;
                        state.browse_category = BrowseCategory::Artists;
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
                    // Play selected track and queue remaining similar tracks
                    let idx = state.list_state.similar_index;
                    if idx < state.similar_tracks.len() {
                        state.queue = state.similar_tracks[idx..].to_vec();
                        state.queue_index = Some(0);
                        if let Some(track) = state.queue.first().cloned() {
                            vec![Action::PlayTrack(track)]
                        } else {
                            vec![]
                        }
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

/// Handle Help view keys.
fn handle_help_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    match key.code {
        KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('?') => {
            state.help_scroll = 0;  // Reset scroll when closing
            vec![Action::SetView(View::Browse)]
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.help_scroll = state.help_scroll.saturating_sub(1);
            vec![]
        }
        KeyCode::Down | KeyCode::Char('j') => {
            // Cap at max reasonable scroll (help text is ~140 lines)
            let max_scroll = 140u16.saturating_sub(state.terminal_height.saturating_sub(4));
            state.help_scroll = state.help_scroll.saturating_add(1).min(max_scroll);
            vec![]
        }
        KeyCode::PageUp => {
            state.help_scroll = state.help_scroll.saturating_sub(20);
            vec![]
        }
        KeyCode::PageDown => {
            // Cap at max reasonable scroll (help text is ~140 lines)
            let max_scroll = 140u16.saturating_sub(state.terminal_height.saturating_sub(4));
            state.help_scroll = state.help_scroll.saturating_add(20).min(max_scroll);
            vec![]
        }
        KeyCode::Home => {
            state.help_scroll = 0;
            vec![]
        }
        KeyCode::End => {
            // Set to max scroll based on terminal height
            let max_scroll = 140u16.saturating_sub(state.terminal_height.saturating_sub(4));
            state.help_scroll = max_scroll;
            vec![]
        }
        _ => vec![],
    }
}

/// Handle Settings view keys.
fn handle_settings_keys(key: event::KeyEvent, state: &mut AppState, config: &crate::config::Config) -> Vec<Action> {
    use crate::app::state::{CredentialField, SettingsFocus, SettingsSection};

    // Handle credential editing mode first
    if let Some(field) = state.settings_state.editing_credential {
        match key.code {
            KeyCode::Esc => {
                // Cancel editing, restore original value
                state.settings_state.editing_credential = None;
                // Restore username from stored auth or config
                state.settings_state.username_input = PlexAuth::load_token()
                    .and_then(|s| s.username)
                    .or_else(|| config.plex.username.clone())
                    .unwrap_or_default();
                state.settings_state.password_input = String::new();
                return vec![];
            }
            KeyCode::Enter => {
                // Save credential and exit edit mode
                state.settings_state.editing_credential = None;
                return vec![Action::SaveCredentials];
            }
            KeyCode::Backspace => {
                // Delete last character
                match field {
                    CredentialField::Username => {
                        state.settings_state.username_input.pop();
                    }
                    CredentialField::Password => {
                        state.settings_state.password_input.pop();
                    }
                }
                return vec![];
            }
            KeyCode::Char(c) => {
                // Add character to input
                match field {
                    CredentialField::Username => {
                        state.settings_state.username_input.push(c);
                    }
                    CredentialField::Password => {
                        state.settings_state.password_input.push(c);
                    }
                }
                return vec![];
            }
            _ => return vec![],
        }
    }

    match key.code {
        KeyCode::Esc => {
            if state.settings_state.signing_in {
                // Cancel sign-in mode, go back to Account view
                state.settings_state.signing_in = false;
                state.settings_state.item_index = 0;
                state.settings_state.editing_credential = None;
                vec![]
            } else {
                vec![Action::SetView(View::Browse)]
            }
        }
        // Panel switching
        KeyCode::Tab | KeyCode::Right => {
            if state.settings_state.focus == SettingsFocus::Sections {
                state.settings_state.focus = SettingsFocus::Content;
                state.settings_state.item_index = 0;
            }
            vec![]
        }
        KeyCode::BackTab | KeyCode::Left => {
            if state.settings_state.focus == SettingsFocus::Content {
                state.settings_state.focus = SettingsFocus::Sections;
            }
            vec![]
        }
        KeyCode::Up => {
            match state.settings_state.focus {
                SettingsFocus::Sections => {
                    // Navigate sections
                    state.settings_state.section = state.settings_state.section.prev();
                    state.settings_state.item_index = 0;
                }
                SettingsFocus::Content => {
                    // Navigate items within section
                    if state.settings_state.item_index > 0 {
                        state.settings_state.item_index -= 1;
                    }
                }
            }
            vec![]
        }
        KeyCode::Down => {
            match state.settings_state.focus {
                SettingsFocus::Sections => {
                    // Navigate sections
                    state.settings_state.section = state.settings_state.section.next();
                    state.settings_state.item_index = 0;
                }
                SettingsFocus::Content => {
                    // Navigate items within section with bounds check
                    let max_index = match state.settings_state.section {
                        SettingsSection::Account => {
                            if state.settings_state.signing_in {
                                // username(0), password(1), sign in(2), then servers(3+)
                                2 + state.available_servers.len()
                            } else if matches!(state.connection, crate::app::state::ConnectionState::Connected { .. }) {
                                1 // Clear Cache(0), Sign Out(1)
                            } else {
                                0 // Sign In(0)
                            }
                        }
                        SettingsSection::Libraries => {
                            state.libraries.len().saturating_sub(1)
                        }
                        SettingsSection::Interface => {
                            crate::ui::theme::ThemeName::all().len().saturating_sub(1)
                        }
                        SettingsSection::Playback => 0,
                        SettingsSection::About => 0, // No selectable items
                    };
                    if state.settings_state.item_index < max_index {
                        state.settings_state.item_index += 1;
                    }
                }
            }
            vec![]
        }
        KeyCode::Enter => {
            if state.settings_state.focus == SettingsFocus::Sections {
                // Enter on section -> move to content
                state.settings_state.focus = SettingsFocus::Content;
                state.settings_state.item_index = 0;
                vec![]
            } else if state.settings_state.section == SettingsSection::Account && state.settings_state.signing_in {
                // In sign-in mode: handle credential fields vs sign in vs server selection
                match state.settings_state.item_index {
                    0 => {
                        // Username field - start editing
                        state.settings_state.editing_credential = Some(CredentialField::Username);
                        vec![]
                    }
                    1 => {
                        // Password field - start editing
                        state.settings_state.editing_credential = Some(CredentialField::Password);
                        vec![]
                    }
                    2 => {
                        // Sign In button - authenticate with entered credentials
                        vec![Action::SettingsSignIn]
                    }
                    _ => {
                        // Server selection (index 3+)
                        vec![Action::SettingsSelect]
                    }
                }
            } else {
                // Enter on content -> select item
                vec![Action::SettingsSelect]
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
    let category = state.list_filter_category;
    let column = state.list_filter_column;

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
    let category = state.list_filter_category;

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
