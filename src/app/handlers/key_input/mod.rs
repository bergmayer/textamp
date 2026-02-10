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
pub use browse::{update_filter_column_selection, get_filter_drilldown_actions, truncate_filter_right_columns};
pub use self::alt_commands::{AltCommand, available_alt_commands};

mod alt_commands;

use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::app::Action;
use crate::app::state::{
    BrowseCategory, BrowseItem, BrowseNavigationState, Focus, PlaybackMode, RightPanelMode, View,
};
use crate::app::AppState;
use crate::api::models::Track;
use super::helpers;

/// Handle keyboard input (CUA-style with Ctrl shortcuts).
pub fn handle_key(key: event::KeyEvent, state: &mut AppState, config: &crate::config::Config) -> Vec<Action> {
    // Clear mouse scroll pin on keyboard input, EXCEPT for drill-down/back keys
    // (Enter, Right, Left, Backspace, Esc) which should preserve the pinned
    // scroll position so the viewport doesn't re-center during column changes.
    let preserve_pin = matches!(key.code,
        KeyCode::Enter | KeyCode::Right | KeyCode::Left | KeyCode::Backspace | KeyCode::Esc
    ) && !key.modifiers.contains(KeyModifiers::SHIFT)
      && !key.modifiers.contains(KeyModifiers::CONTROL);
    if !preserve_pin {
        state.browse_scroll_pin = None;
    }
    state.browse_click_time = None;

    // Track modifier bar display.
    // Alt+/ (or Alt+?) cycles: off → Alt bar → Ctrl+Alt bar → off
    // Any non-Alt key immediately dismisses both bars.
    let has_alt = key.modifiers.contains(KeyModifiers::ALT);
    let is_help_key = matches!(key.code, KeyCode::Char('?') | KeyCode::Char('/'));
    let bar_duration = std::time::Duration::from_secs(4);

    if is_help_key && has_alt {
        // Alt+/ or Alt+? — cycle: off → alt bar → ctrl+alt bar → off
        if state.ctrl_alt_bar_until.is_some() {
            // Ctrl+Alt bar showing → dismiss both
            state.ctrl_alt_bar_until = None;
            state.alt_bar_until = None;
        } else if state.alt_bar_until.is_some() {
            // Alt bar showing → switch to ctrl+alt bar
            state.alt_bar_until = None;
            state.ctrl_alt_bar_until = Some(std::time::Instant::now() + bar_duration);
        } else {
            // Nothing showing → show alt bar
            state.alt_bar_until = Some(std::time::Instant::now() + bar_duration);
        }
        return vec![];
    } else if !has_alt {
        // Non-Alt key: dismiss both bars immediately
        state.alt_bar_until = None;
        state.ctrl_alt_bar_until = None;
    }

    // Clear error on any key
    if state.last_error.is_some() {
        state.clear_error();
        return vec![];
    }

    // Handle confirm dialog if active
    if let Some(dialog) = state.confirm_dialog.take() {
        match key.code {
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                use crate::app::state::ConfirmAction;
                return match dialog.on_confirm {
                    ConfirmAction::RefreshCache => helpers::refresh_current_view(state),
                    ConfirmAction::ClearLibraryCache => vec![Action::ClearLibraryCache],
                    ConfirmAction::ClearArtworkCache => vec![Action::ClearArtworkCache],
                    ConfirmAction::ClearSubfolderCache => vec![Action::ClearSubfolderCache],
                };
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                return vec![];
            }
            _ => {
                // Put dialog back — key not handled
                state.confirm_dialog = Some(dialog);
                return vec![];
            }
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
    if state.adventure.active {
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
            let tier1 = match state.genre_content_type {
                crate::app::state::GenreContentType::Genres => crate::app::state::RefreshCategory::Genres,
                crate::app::state::GenreContentType::ArtistGenres => crate::app::state::RefreshCategory::ArtistGenres,
                crate::app::state::GenreContentType::AlbumGenres => crate::app::state::RefreshCategory::AlbumGenres,
                crate::app::state::GenreContentType::Moods => crate::app::state::RefreshCategory::Moods,
                crate::app::state::GenreContentType::Styles => crate::app::state::RefreshCategory::Styles,
                crate::app::state::GenreContentType::Stations => crate::app::state::RefreshCategory::Stations,
            };
            return vec![load_action, Action::SetView(View::Browse), Action::CheckStaleness(tier1)];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
            // Ctrl+N = Now Playing, or cycle mode if already there
            if state.view == View::NowPlaying {
                // Already in Now Playing - cycle mode (Queue → Recently Played)
                return vec![Action::CycleNowPlayingMode];
            }
            return vec![Action::SetView(View::NowPlaying)];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
            // Ctrl+A = Artists category, or cycle view mode if already there
            if state.view == View::Browse && state.browse_category == BrowseCategory::Artists {
                // Already in artists view - cycle view mode (Artist → Album)
                return vec![Action::CycleArtistViewMode];
            }
            // Not in artists view - switch to it and reset right panel
            state.browse_category = BrowseCategory::Artists;
            reset_right_panel(state);
            let tier1 = match state.artist_view_mode {
                crate::app::state::ArtistViewMode::Artist |
                crate::app::state::ArtistViewMode::AlbumArtist => crate::app::state::RefreshCategory::Artists,
                crate::app::state::ArtistViewMode::Album => crate::app::state::RefreshCategory::Albums,
            };
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
                return vec![load_action, Action::SetView(View::Browse), Action::CheckStaleness(tier1)];
            }
            return vec![Action::SetView(View::Browse), Action::CheckStaleness(tier1)];
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
            let mut actions = vec![Action::RefreshPlaylistsView, Action::SetView(View::Browse)];
            if state.playlists.is_empty() {
                actions.insert(0, Action::LoadPlaylists);
            }
            let tier1 = match state.playlists_mode {
                crate::app::state::PlaylistsMode::All => crate::app::state::RefreshCategory::Playlists,
                crate::app::state::PlaylistsMode::Stations => crate::app::state::RefreshCategory::Stations,
                crate::app::state::PlaylistsMode::RecentlyAdded => crate::app::state::RefreshCategory::RecentlyAdded,
            };
            actions.push(Action::CheckStaleness(tier1));
            return actions;
        }
        (KeyModifiers::CONTROL, KeyCode::Char('o')) => {
            // Ctrl+O = Folders category
            state.browse_category = BrowseCategory::Folders;
            reset_right_panel(state);
            let staleness = Action::CheckStaleness(crate::app::state::RefreshCategory::Folders);
            if state.folder_state.is_none() {
                return vec![Action::LoadFolderRoot, Action::SetView(View::Browse), staleness];
            }
            return vec![Action::SetView(View::Browse), staleness];
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

        // Playback controls
        (_, KeyCode::Char(' ')) if state.view != View::Search && !state.list_filter.active && !state.search_popup_active => {
            return vec![Action::TogglePlayPause];
        }
        // < and > for prev/next track (crossterm reports these with NONE modifiers, not SHIFT)
        (_, KeyCode::Char('<')) if state.view != View::Search && !state.list_filter.active && !state.search_popup_active => {
            return vec![Action::Previous];
        }
        (_, KeyCode::Char('>')) if state.view != View::Search && !state.list_filter.active && !state.search_popup_active => {
            return vec![Action::Next];
        }
        (mods, KeyCode::Up) if mods == KeyModifiers::CONTROL | KeyModifiers::SHIFT => return vec![Action::VolumeUp],
        (mods, KeyCode::Down) if mods == KeyModifiers::CONTROL | KeyModifiers::SHIFT => return vec![Action::VolumeDown],
        // Shift+Left/Right for seeking (10 second skip)
        (KeyModifiers::SHIFT, KeyCode::Left) => return vec![Action::SeekRelative(-10000)],
        (KeyModifiers::SHIFT, KeyCode::Right) => return vec![Action::SeekRelative(10000)],

        // Alt key commands (global) — gated by availability check
        (KeyModifiers::ALT, KeyCode::Char(c)) if alt_commands::is_alt_command_available(state, c) => {
            match c {
                'r' => return create_station_from_context(state),
                'q' => return vec![Action::EnqueueSelection],
                's' => {
                    if state.view == View::Browse {
                        return vec![Action::ToggleBrowseShuffle];
                    } else {
                        return vec![Action::ToggleQueueShuffle];
                    }
                }
                'm' => return get_similar_action(state),
                'a' => return handle_adventure_key(state),
                'b' => return navigate_to_album(state),
                'g' => return navigate_to_artist(state),
                'w' => return vec![Action::PromptSavePlaylist],
                'c' => return vec![Action::ToggleAlbumArtView],
                _ => {}
            }
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
        (mods, KeyCode::Char('a')) if mods == KeyModifiers::CONTROL | KeyModifiers::ALT => {
            // Ctrl+Alt+A = Play album of highlighted track (or now-playing track as fallback)
            // First, try to get album key from the highlighted item in the current view
            let album_key = match state.view {
                View::Browse => {
                    // Get the active nav's selected item
                    let selected = match state.browse_category {
                        BrowseCategory::Artists => state.artist_nav.selected_item().cloned(),
                        BrowseCategory::Genres => state.genre_nav.selected_item().cloned(),
                        BrowseCategory::Playlists => state.playlist_nav.selected_item().cloned(),
                        BrowseCategory::Folders => None,
                    };
                    match selected {
                        Some(BrowseItem::Album { key, .. }) => Some(key),
                        Some(BrowseItem::Track { .. }) => {
                            // Get full Track from the focused column's tracks vec
                            let nav: Option<&BrowseNavigationState> = match state.browse_category {
                                BrowseCategory::Artists => Some(&state.artist_nav),
                                BrowseCategory::Genres => Some(&state.genre_nav),
                                BrowseCategory::Playlists => Some(&state.playlist_nav),
                                BrowseCategory::Folders => None,
                            };
                            nav.and_then(|n| n.focused())
                               .and_then(|col| col.tracks.get(col.selected_index))
                               .and_then(|t| t.parent_rating_key.clone())
                        }
                        _ => None,
                    }
                }
                View::NowPlaying => {
                    let idx = state.list_state.queue_index;
                    let track = match state.playback_mode {
                        PlaybackMode::Queue | PlaybackMode::None => state.queue.get(idx),
                        PlaybackMode::Radio => state.radio.tracks.get(idx),
                    };
                    track.and_then(|t| t.parent_rating_key.clone())
                }
                _ => None,
            };
            // Fall back to now-playing track's album
            let album_key = album_key.or_else(|| {
                state.current_track().and_then(|t| t.parent_rating_key.clone())
            });
            if let Some(key) = album_key {
                return vec![Action::PlayAlbum { rating_key: key }];
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
    // Build flat list matching what render_library_picker shows
    let multi_server = state.has_multiple_servers();
    let all_libs: Vec<(&str, &str, &crate::api::models::Library)> = if multi_server {
        state.all_libraries_with_servers()
    } else {
        let server_id = state.active_server_id.as_deref().unwrap_or("");
        let server_name = state.active_server_name().unwrap_or("");
        state.libraries.iter()
            .map(|lib| (server_id, server_name, lib))
            .collect()
    };

    let lib_count = all_libs.len();
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
            if let Some((server_id, _, lib)) = all_libs.get(state.library_picker_index) {
                let lib_key = lib.key.clone();
                let is_different_server = state.active_server_id.as_deref() != Some(*server_id);

                if is_different_server && multi_server {
                    return vec![
                        Action::SelectLibraryOnServer(lib_key, server_id.to_string()),
                        Action::CloseLibraryPicker,
                    ];
                } else {
                    return vec![Action::SelectLibrary(lib_key), Action::CloseLibraryPicker];
                }
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

/// Navigate to the album of the currently selected track (Alt+B).
/// Switches to Browse/Artists, finds the artist, loads albums, and auto-selects the album.
fn navigate_to_album(state: &mut AppState) -> Vec<Action> {
    // Try Miller column context first, then folder context, then now-playing track fallback
    let (album_key, artist_key, album_title, artist_name) =
        if let Some(ctx) = get_miller_album_context(state) {
            ctx
        } else if let Some(ctx) = get_folder_album_context(state) {
            ctx
        } else if let Some(track) = get_selected_track(state)
            .or_else(|| state.current_track().cloned())
        {
            let ak = match &track.parent_rating_key { Some(k) => k.clone(), None => return vec![] };
            let rk = match &track.grandparent_rating_key { Some(k) => k.clone(), None => return vec![] };
            (ak, rk, track.album_name().to_string(), track.artist_name().to_string())
        } else {
            return vec![];
        };

    // Navigate to the artist in Miller columns, with pending album auto-select
    state.pending_album_key = Some(album_key);
    state.selected_album_title = album_title;
    state.selected_artist_name = artist_name;
    state.view = View::Browse;
    state.browse_category = BrowseCategory::Artists;

    // Select the artist in the Miller column
    if let Some(idx) = state.artist_nav.columns.first()
        .and_then(|col| col.items.iter().position(|item| matches!(item, BrowseItem::Artist { key, .. } if *key == artist_key)))
    {
        if let Some(col) = state.artist_nav.columns.first_mut() {
            col.selected_index = idx;
        }
        state.artist_nav.focused_column = 0;
        state.artist_nav.truncate_right();
    }
    // Also update old state for backward compatibility
    if let Some(idx) = state.artists.iter().position(|a| a.rating_key == artist_key) {
        state.list_state.artists_index = idx;
    }

    vec![Action::LoadArtistAlbumsForMiller { artist_key }]
}

/// Navigate to the artist of the currently selected track or album (Alt+G).
/// Switches to Browse/Artists and loads the artist's album list.
fn navigate_to_artist(state: &mut AppState) -> Vec<Action> {
    // Try Miller column context first, then folder context, then old context, then now-playing fallback
    let (artist_key, artist_name) =
        if let Some(ctx) = get_miller_artist_context(state) {
            ctx
        } else if let Some(ctx) = get_folder_artist_context(state) {
            ctx
        } else if let Some(key) = get_artist_key_from_context(state) {
            let name = state.artists.iter()
                .find(|a| a.rating_key == key)
                .map(|a| a.title.clone())
                .unwrap_or_default();
            (key, name)
        } else if let Some(track) = state.current_track().cloned() {
            if let Some(key) = track.grandparent_rating_key.clone() {
                (key, track.artist_name().to_string())
            } else {
                return vec![];
            }
        } else {
            return vec![];
        };

    state.view = View::Browse;
    state.browse_category = BrowseCategory::Artists;
    state.selected_artist_name = artist_name;
    state.pending_album_key = None;
    state.selected_album_title.clear();

    // Select the artist in the Miller column
    if let Some(idx) = state.artist_nav.columns.first()
        .and_then(|col| col.items.iter().position(|item| matches!(item, BrowseItem::Artist { key, .. } if *key == artist_key)))
    {
        if let Some(col) = state.artist_nav.columns.first_mut() {
            col.selected_index = idx;
        }
        state.artist_nav.focused_column = 0;
        state.artist_nav.truncate_right();
    }
    // Also update old state for backward compatibility
    if let Some(idx) = state.artists.iter().position(|a| a.rating_key == artist_key) {
        state.list_state.artists_index = idx;
    }

    vec![Action::LoadArtistAlbumsForMiller { artist_key }]
}

/// Get album context from the selected folder track: (album_key, artist_key, album_title, artist_name).
fn get_folder_album_context(state: &AppState) -> Option<(String, String, String, String)> {
    if state.view != View::Browse || state.browse_category != BrowseCategory::Folders {
        return None;
    }
    let item = state.folder_state.as_ref()?.selected_item()?;
    if !item.is_track() { return None; }
    let album_key = item.parent_rating_key.clone()?;
    let artist_key = item.grandparent_rating_key.clone()?;
    // We don't have album/artist titles in FolderItem, use empty strings
    // (navigate_to_album will look them up from the artists list)
    Some((album_key, artist_key, String::new(), String::new()))
}

/// Get artist context from the selected folder track: (artist_key, artist_name).
fn get_folder_artist_context(state: &AppState) -> Option<(String, String)> {
    if state.view != View::Browse || state.browse_category != BrowseCategory::Folders {
        return None;
    }
    let item = state.folder_state.as_ref()?.selected_item()?;
    if !item.is_track() { return None; }
    let artist_key = item.grandparent_rating_key.clone()?;
    let artist_name = state.artists.iter()
        .find(|a| a.rating_key == artist_key)
        .map(|a| a.title.clone())
        .unwrap_or_default();
    Some((artist_key, artist_name))
}

/// Extract the artist rating key from the current context.
/// Works from tracks (grandparent_rating_key) and albums (parent_rating_key).
fn get_artist_key_from_context(state: &AppState) -> Option<String> {
    // Try track first
    if let Some(track) = get_selected_track(state) {
        return track.grandparent_rating_key.clone();
    }

    // Try album context
    match state.view {
        View::Browse => {
            match state.right_panel_mode {
                RightPanelMode::ArtistAlbums => {
                    let album_idx = state.list_state.right_albums_index.saturating_sub(1);
                    if state.list_state.right_albums_index > 0 {
                        if let Some(album) = state.selected_artist_albums.get(album_idx) {
                            return album.parent_rating_key.clone();
                        }
                    }
                    // Index 0 is "All Tracks" - use the current artist
                    state.artists.get(state.list_state.artists_index)
                        .map(|a| a.rating_key.clone())
                }
                RightPanelMode::CategoryAlbums => {
                    state.genre_albums.get(state.genre_albums_index)
                        .and_then(|a| a.parent_rating_key.clone())
                }
                _ => None,
            }
        }
        View::Similar => {
            if let Some(album) = state.similar_albums.get(state.list_state.similar_index) {
                return album.parent_rating_key.clone();
            }
            None
        }
        _ => None,
    }
}

/// Extract album context from Miller columns: (album_key, artist_key, album_title, artist_name).
/// Works when a Track or Album is selected in the artist/genre/playlist navigation.
fn get_miller_album_context(state: &AppState) -> Option<(String, String, String, String)> {
    if state.view != View::Browse {
        return None;
    }

    let nav = match state.browse_category {
        BrowseCategory::Artists => &state.artist_nav,
        BrowseCategory::Genres => &state.genre_nav,
        BrowseCategory::Playlists => &state.playlist_nav,
        _ => return None,
    };

    let focused = nav.focused_column;
    let selected_item = nav.columns.get(focused)
        .and_then(|c| c.items.get(c.selected_index))?;

    match selected_item {
        BrowseItem::Track { .. } => {
            // Track selected: album is in parent column, artist in grandparent
            let album = (focused > 0).then(|| nav.columns.get(focused - 1)).flatten()
                .and_then(|c| c.items.get(c.selected_index));
            let (album_key, album_title) = match album {
                Some(BrowseItem::Album { key, title, .. }) => (key.clone(), title.clone()),
                _ => return None,
            };
            // Try to find artist from column hierarchy
            let artist_key = find_artist_key_in_nav(nav);
            let artist_name = find_artist_name_in_nav(nav, state);
            let artist_key = artist_key?;
            Some((album_key, artist_key, album_title, artist_name))
        }
        BrowseItem::Album { key, title, artist, .. } => {
            // Album selected: artist is in parent column
            let artist_key = find_artist_key_in_nav(nav);
            let artist_key = artist_key?;
            let artist_name = artist.clone();
            Some((key.clone(), artist_key, title.clone(), artist_name))
        }
        _ => None,
    }
}

/// Extract artist context from Miller columns: (artist_key, artist_name).
fn get_miller_artist_context(state: &AppState) -> Option<(String, String)> {
    if state.view != View::Browse {
        return None;
    }

    let nav = match state.browse_category {
        BrowseCategory::Artists => &state.artist_nav,
        BrowseCategory::Genres => &state.genre_nav,
        BrowseCategory::Playlists => &state.playlist_nav,
        _ => return None,
    };

    let focused = nav.focused_column;
    let selected_item = nav.columns.get(focused)
        .and_then(|c| c.items.get(c.selected_index))?;

    match selected_item {
        BrowseItem::Track { .. } | BrowseItem::Album { .. } | BrowseItem::AllTracks { .. } => {
            let key = find_artist_key_in_nav(nav)?;
            let name = find_artist_name_in_nav(nav, state);
            Some((key, name))
        }
        BrowseItem::Artist { key, title } => {
            Some((key.clone(), title.clone()))
        }
        _ => None,
    }
}

/// Find artist key by walking up the Miller column hierarchy.
fn find_artist_key_in_nav(nav: &BrowseNavigationState) -> Option<String> {
    for col in &nav.columns {
        if let Some(item) = col.items.get(col.selected_index) {
            if let BrowseItem::Artist { key, .. } = item {
                return Some(key.clone());
            }
        }
    }
    None
}

/// Find artist name from Miller columns or state.
fn find_artist_name_in_nav(nav: &BrowseNavigationState, state: &AppState) -> String {
    for col in &nav.columns {
        if let Some(BrowseItem::Artist { title, .. }) = col.items.get(col.selected_index) {
            return title.clone();
        }
    }
    state.selected_artist_name.clone()
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
