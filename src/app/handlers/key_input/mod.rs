//! Keyboard input handler functions.
//!
//! Split into focused submodules:
//! - `browse` — Browse view key handling (folders, stations, artists, genres, playlists)
//! - `now_playing` — Now Playing view key handling
//! - `search` — Search view key handling
//! - `similar` — Similar view key handling
//! - `settings` — Settings and Help view key handling

mod browse;
mod now_playing;
mod search;
mod similar;
mod settings;

// Re-export public items used by other handler modules.
pub use browse::{update_filter_column_selection, get_filter_drilldown_actions};

use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::app::Action;
use crate::app::state::{
    BrowseCategory, Focus, PlaybackMode, RightPanelMode, View,
};
use crate::app::AppState;
use crate::api::models::Track;
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
        (_, KeyCode::Char(' ')) if state.view != View::Search && !state.list_filter.active && !state.search_popup_active => {
            return vec![Action::TogglePlayPause];
        }
        (KeyModifiers::CONTROL, KeyCode::Left) => return vec![Action::Previous],
        (KeyModifiers::CONTROL, KeyCode::Right) => return vec![Action::Next],
        // < and > for prev/next track (crossterm reports these with NONE modifiers, not SHIFT)
        (_, KeyCode::Char('<')) if state.view != View::Search && !state.list_filter.active && !state.search_popup_active => {
            return vec![Action::Previous];
        }
        (_, KeyCode::Char('>')) if state.view != View::Search && !state.list_filter.active && !state.search_popup_active => {
            return vec![Action::Next];
        }
        (KeyModifiers::CONTROL, KeyCode::Up) => return vec![Action::VolumeUp],
        (KeyModifiers::CONTROL, KeyCode::Down) => return vec![Action::VolumeDown],
        // Shift+Left/Right for seeking (10 second skip)
        (KeyModifiers::SHIFT, KeyCode::Left) => return vec![Action::SeekRelative(-10000)],
        (KeyModifiers::SHIFT, KeyCode::Right) => return vec![Action::SeekRelative(10000)],

        // Alt key commands (global)
        (KeyModifiers::ALT, KeyCode::Char('r')) => {
            // Alt+R = Sonic radio from current selection
            return create_station_from_context(state);
        }
        (KeyModifiers::ALT, KeyCode::Char('q')) => {
            // Alt+Q = Queue selection (enqueue)
            return vec![Action::EnqueueSelection];
        }
        (KeyModifiers::ALT, KeyCode::Char('s')) => {
            // Alt+S = Shuffle: browse view or queue/radio depending on context
            if state.view == View::Browse {
                return vec![Action::ToggleBrowseShuffle];
            } else if !state.queue.is_empty() || !state.radio.tracks.is_empty() {
                return vec![Action::ToggleQueueShuffle];
            }
        }
        (KeyModifiers::ALT, KeyCode::Char('m')) => {
            // Alt+M = More like this (similar albums/tracks)
            return get_similar_action(state);
        }
        (KeyModifiers::ALT, KeyCode::Char('a')) => {
            // Alt+A = Sonic Adventure
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
        return search::handle_search_keys(key, state);
    }

    // Library picker popup handling
    if state.library_picker_active {
        return handle_library_picker_keys(key, state);
    }

    // View-specific handling
    match state.view {
        View::Auth => handle_auth_keys(key, state),
        View::Browse => browse::handle_browse_keys(key, state),
        View::NowPlaying => now_playing::handle_now_playing_keys(key, state),
        View::Search => search::handle_search_keys(key, state),
        View::Similar => similar::handle_similar_keys(key, state),
        View::Help => settings::handle_help_keys(key, state),
        View::Settings => settings::handle_settings_keys(key, state, config),
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

/// Handle Auth view keys.
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

/// Create a sonic radio from current context (artist, album, or track).
/// Track selected -> Sonic track radio (individual similar tracks)
/// Album selected -> Sonic album radio (similar albums played in order)
/// Artist selected -> Sonic artist radio
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

/// Handle Alt+A for Sonic Adventure.
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
    use crate::app::state::SearchSection;

    match state.view {
        // Search/Filter view - handle both Global search and tab-specific filters
        View::Search => {
            let idx = state.list_state.search_item_index;

            match state.search_tab {
                // Global search - uses search_results with sections
                crate::app::state::SearchTab::Global => {
                    if state.list_state.search_section == SearchSection::Tracks {
                        if let Some(ref results) = state.search_results {
                            return results.tracks.get(idx).cloned();
                        }
                    }
                    None
                }
                // Tracks tab - uses filter_results
                crate::app::state::SearchTab::Tracks => {
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
                        if let Some(col) = state.station_nav.focused_mut() {
                            col.selected_index = idx;
                        }
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
