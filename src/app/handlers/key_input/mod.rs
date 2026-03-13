//! Keyboard input handler functions.
//!
//! Split into focused submodules:
//! - `browse` — Browse view key handling (folders, stations, artists, genres, playlists)
//! - `now_playing` — Now Playing view key handling
//! - `search` — Search view key handling
//! - `similar` — Similar view key handling
//! - `settings` — Settings and Help view key handling

mod adventure_launcher;
mod artist_radio_picker;
mod browse;
mod now_playing;
mod radio_launcher;
mod search;
pub(in crate::app::handlers) mod similar;
pub(in crate::app::handlers) mod related;
mod settings;

// Re-export public items used by other handler modules.
pub use browse::{update_filter_column_selection, get_filter_drilldown_actions, truncate_filter_right_columns};
pub(crate) use browse::{auto_drill_artist_action, auto_drill_genre_action, auto_drill_playlist_action};
pub use self::alt_commands::{AltCommand, CommandModifier, available_alt_commands};

mod alt_commands;
pub(crate) mod sort_popup;

use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::app::Action;
use crate::app::state::{
    BrowseCategory, BrowseItem, BrowseNavigationState, Focus, PlaybackMode,
    RightPanelMode, View,
};
use crate::app::AppState;
use crate::plex::models::Track;
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
        state.scroll.browse = None;
    }
    state.scroll.browse_click_time = None;
    state.scroll.browse_last_click = None;

    // Track modifier bar display.
    // Alt+/ or Ctrl+/ toggles the contextual shortcut bar on/off.
    // Any non-modifier key immediately dismisses it.
    let has_alt = key.modifiers.contains(KeyModifiers::ALT);
    let has_ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let is_help_key = matches!(key.code, KeyCode::Char('?') | KeyCode::Char('/'));
    let bar_duration = std::time::Duration::from_secs(4);

    if is_help_key && (has_alt || has_ctrl) {
        // Alt+/ or Ctrl+/ — toggle shortcut bar
        if state.alt_bar_until.is_some() {
            state.alt_bar_until = None;
        } else {
            state.alt_bar_until = Some(std::time::Instant::now() + bar_duration);
        }
        return vec![];
    } else if !has_alt && !has_ctrl {
        // Non-modifier key: dismiss bar immediately
        state.alt_bar_until = None;
    }

    // Clear error on any key
    if state.notifications.last_error.is_some() {
        state.clear_error();
        return vec![];
    }

    // Handle confirm dialog if active
    if let Some(mut dialog) = state.popups.confirm_dialog.take() {
        // Pressing the same shortcut again (e.g. Ctrl+Q Ctrl+Q) confirms immediately
        if matches!(dialog.on_confirm, crate::app::state::ConfirmAction::Quit)
            && key.modifiers == KeyModifiers::CONTROL
            && key.code == KeyCode::Char('q')
        {
            return vec![Action::Quit];
        }
        match key.code {
            KeyCode::Enter => {
                if dialog.selected_yes {
                    use crate::app::state::ConfirmAction;
                    return match dialog.on_confirm {
                        ConfirmAction::RefreshCache => helpers::refresh_current_view(state),
                        ConfirmAction::ClearLibraryCache => vec![Action::ClearLibraryCache],
                        ConfirmAction::ClearArtworkCache => vec![Action::ClearArtworkCache],
                        ConfirmAction::ClearSubfolderCache => vec![Action::ClearSubfolderCache],
                        ConfirmAction::Quit => vec![Action::Quit],
                    };
                } else {
                    // No selected — dismiss
                    return vec![];
                }
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                use crate::app::state::ConfirmAction;
                return match dialog.on_confirm {
                    ConfirmAction::RefreshCache => helpers::refresh_current_view(state),
                    ConfirmAction::ClearLibraryCache => vec![Action::ClearLibraryCache],
                    ConfirmAction::ClearArtworkCache => vec![Action::ClearArtworkCache],
                    ConfirmAction::ClearSubfolderCache => vec![Action::ClearSubfolderCache],
                    ConfirmAction::Quit => vec![Action::Quit],
                };
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                return vec![];
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                dialog.selected_yes = !dialog.selected_yes;
                state.popups.confirm_dialog = Some(dialog);
                return vec![];
            }
            _ => {
                state.popups.confirm_dialog = Some(dialog);
                return vec![];
            }
        }
    }

    // Handle input dialog if active
    if let Some(ref mut dialog) = state.popups.input_dialog {
        match key.code {
            KeyCode::Esc => {
                // Cancel dialog and adventure if it was for adventure length
                let was_adventure = matches!(dialog.action_type, crate::app::state::InputDialogAction::AdventureLength);
                state.popups.input_dialog = None;
                if was_adventure {
                    return vec![Action::CancelAdventure];
                }
            }
            KeyCode::Enter => {
                // Confirm dialog
                let input = dialog.input.clone();
                let action_type = dialog.action_type.clone();
                state.popups.input_dialog = None;
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
        // Quit: Ctrl+Q (with confirmation)
        (KeyModifiers::CONTROL, KeyCode::Char('q')) => {
            use crate::app::state::{ConfirmDialog, ConfirmAction};
            state.popups.close_all();
            state.popups.confirm_dialog = Some(ConfirmDialog {
                title: "Quit".to_string(),
                message: "Are you sure you want to quit?".to_string(),
                on_confirm: ConfirmAction::Quit,
                selected_yes: false,
            });
            return vec![];
        }

        // Global navigation shortcuts
        (KeyModifiers::CONTROL, KeyCode::Char('f')) => {
            // Ctrl+F = Search/Filter popup (floating dialog)
            if state.popups.search_active {
                return vec![Action::CloseSearchPopup];
            } else {
                return vec![Action::OpenSearchPopup];
            }
        }
        (KeyModifiers::CONTROL, KeyCode::Char('g')) => {
            // Ctrl+G = Genres category (no cycling — use Tab to switch tabs)
            if state.view == View::Browse && state.browse_category == BrowseCategory::Genres {
                return vec![];
            }
            // Not in genres view - switch to it and reset right panel
            state.browse_category = BrowseCategory::Genres;
            reset_right_panel(state);
            // RefreshGenreView uses cached data when available, only fetches if empty
            return vec![Action::RefreshGenreView, Action::SetView(View::Browse), Action::CheckStaleness(crate::app::state::RefreshCategory::Genres)];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
            // Ctrl+N = Now Playing (visualizer view)
            return vec![Action::SetView(View::NowPlaying), Action::LoadWaveform];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
            // Ctrl+U = Queue view
            return vec![Action::SetView(View::Queue)];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
            // Ctrl+L = Library category (no cycling — Plex doesn't distinguish album artists)
            if state.view == View::Browse && state.browse_category == BrowseCategory::Library {
                return vec![];
            }
            // Not in library view - switch to it and reset right panel
            state.browse_category = BrowseCategory::Library;
            reset_right_panel(state);
            let tier1 = crate::app::state::RefreshCategory::Artists;
            if state.artists.is_empty() {
                return vec![Action::LoadArtists, Action::SetView(View::Browse), Action::CheckStaleness(tier1)];
            }
            // Auto-drill if only root column exists
            let mut actions = vec![Action::SetView(View::Browse), Action::CheckStaleness(tier1)];
            if state.artist_nav.columns.len() == 1 && !state.artist_nav.is_empty() {
                if let Some(drill) = browse::auto_drill_artist_action(state) {
                    state.auto_drill_pending = true;
                    actions.push(drill);
                }
            }
            return actions;
        }
        (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
            // Ctrl+P = Playlists category
            if state.view == View::Browse && state.browse_category == BrowseCategory::Playlists {
                return vec![];
            }
            state.browse_category = BrowseCategory::Playlists;
            reset_right_panel(state);
            let mut actions = vec![Action::SetView(View::Browse)];
            if state.playlists.is_empty() {
                actions.insert(0, Action::LoadPlaylists);
            } else {
                let items = crate::app::state::BrowseItem::from_playlists(&state.playlists);
                state.playlist_nav.reset("playlists", items);
                // Auto-drill if only root column exists
                if state.playlist_nav.columns.len() == 1 && !state.playlist_nav.is_empty() {
                    if let Some(drill) = browse::auto_drill_playlist_action(state) {
                        state.auto_drill_pending = true;
                        actions.push(drill);
                    }
                }
            }
            actions.push(Action::CheckStaleness(crate::app::state::RefreshCategory::Playlists));
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
        (_, KeyCode::F(3)) => {
            // F3 = Quick library switcher
            if !state.libraries.is_empty() {
                return vec![Action::OpenLibraryPicker];
            }
        }
        (_, KeyCode::F(4)) => {
            // F4 = Artist bio popup
            // Priority: selected track → selected album → selected artist → now-playing track
            if let Some((artist_key, artist_name)) = helpers::get_artist_for_bio(state) {
                return vec![Action::ShowArtistBio { artist_key, artist_name }];
            }
        }
        (_, KeyCode::F(5)) => {
            // F5 = Refresh current view
            return helpers::refresh_current_view(state);
        }
        // Playback controls
        (_, KeyCode::Char(' ')) if state.view != View::Search && !state.list_filter.active && !state.popups.search_active && state.popups.radio_launcher.is_none() && state.popups.adventure_launcher.is_none() && state.popups.artist_radio_picker.is_none() => {
            return vec![Action::TogglePlayPause];
        }
        // < and > for prev/next track (crossterm reports these with NONE modifiers, not SHIFT)
        (_, KeyCode::Char('<')) if state.view != View::Search && !state.list_filter.active && !state.popups.search_active && state.popups.radio_launcher.is_none() && state.popups.adventure_launcher.is_none() => {
            return vec![Action::Previous];
        }
        (_, KeyCode::Char('>')) if state.view != View::Search && !state.list_filter.active && !state.popups.search_active && state.popups.radio_launcher.is_none() && state.popups.adventure_launcher.is_none() => {
            return vec![Action::Next];
        }
        // Ctrl+Shift+Up/Down: multi-select in Queue view, volume elsewhere
        (mods, KeyCode::Up) if mods == KeyModifiers::CONTROL | KeyModifiers::SHIFT && state.view == View::Queue => {
            // Toggle current item into queue_selected, then move cursor up
            let queue_idx = state.list_state.queue_index;
            if queue_idx < state.queue.len() {
                if state.queue_selected.contains(&queue_idx) {
                    state.queue_selected.remove(&queue_idx);
                } else {
                    state.queue_selected.insert(queue_idx);
                }
            }
            if state.list_state.queue_index > 0 {
                state.list_state.queue_index -= 1;
            }
            return vec![];
        }
        (mods, KeyCode::Down) if mods == KeyModifiers::CONTROL | KeyModifiers::SHIFT && state.view == View::Queue => {
            let queue_idx = state.list_state.queue_index;
            if queue_idx < state.queue.len() {
                if state.queue_selected.contains(&queue_idx) {
                    state.queue_selected.remove(&queue_idx);
                } else {
                    state.queue_selected.insert(queue_idx);
                }
            }
            let max = state.queue.len().saturating_sub(1);
            state.list_state.queue_index = (state.list_state.queue_index + 1).min(max);
            return vec![];
        }
        (mods, KeyCode::Up) if mods == KeyModifiers::CONTROL | KeyModifiers::SHIFT => {
            state.volume_slider_until = Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
            return vec![Action::VolumeUp];
        }
        (mods, KeyCode::Down) if mods == KeyModifiers::CONTROL | KeyModifiers::SHIFT => {
            state.volume_slider_until = Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
            return vec![Action::VolumeDown];
        }
        // Shift+Left/Right for seeking (10 second skip)
        (KeyModifiers::SHIFT, KeyCode::Left) => return vec![Action::SeekRelative(-10000)],
        (KeyModifiers::SHIFT, KeyCode::Right) => return vec![Action::SeekRelative(10000)],
        // Action commands (Ctrl+key) — gated by availability check
        // Ctrl+E: Add to TOP of queue and play (skip if in search popup - handled there)
        (KeyModifiers::CONTROL, KeyCode::Char('e')) if !state.popups.search_active && alt_commands::is_action_command_available(state, 'e') => {
            return vec![Action::EnqueueSelectionNext];
        }
        // Ctrl+Shift+E: Add to END of queue (skip if in search popup - handled there)
        (mods, KeyCode::Char('e')) | (mods, KeyCode::Char('E')) if !state.popups.search_active && mods == KeyModifiers::CONTROL | KeyModifiers::SHIFT && alt_commands::is_action_command_available(state, 'e') => {
            return vec![Action::EnqueueSelection];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('m')) if alt_commands::is_action_command_available(state, 'm') => {
            return get_similar_action(state);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('r')) if alt_commands::is_action_command_available(state, 'r') => {
            return get_related_action(state);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('j')) if alt_commands::is_action_command_available(state, 'j') => {
            return navigate_to_album(state);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('w')) if alt_commands::is_action_command_available(state, 'w') => {
            return vec![Action::PromptSavePlaylist];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('x')) if alt_commands::is_action_command_available(state, 'x') => {
            return vec![Action::ClearQueue];
        }
        // Alt shortcuts (station/global commands)
        (KeyModifiers::ALT, KeyCode::Char('f')) => {
            // Alt+F = Activate inline filter (Browse view only)
            if state.view == View::Browse && !state.list_filter.active
                && !state.popups.search_active && state.popups.sort.is_none()
                && state.popups.radio_launcher.is_none() && state.popups.adventure_launcher.is_none()
                && state.popups.artist_radio_picker.is_none()
            {
                return vec![Action::ActivateListFilter];
            }
            return vec![];
        }
        (KeyModifiers::ALT, KeyCode::Char('r')) => {
            // Alt+R = Play Random Album Radio station
            if let Some(lib_key) = &state.active_library {
                let key = format!("/library/sections/{}/stations/randomAlbum", lib_key);
                return vec![Action::PlayStation(key)];
            }
            return vec![];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('s')) if alt_commands::is_action_command_available(state, 's') => {
            // Ctrl+S = Sort popup for current column
            return vec![Action::OpenSortPopup];
        }

        // External service search: Ctrl+Alt+A/S/Y
        (mods, KeyCode::Char('a')) if mods == KeyModifiers::CONTROL | KeyModifiers::ALT => {
            let query = build_external_search_query(state);
            if !query.is_empty() {
                let url = crate::services::external_search::generate_search_url(
                    crate::services::external_search::SearchTarget::AppleMusic, &query);
                let _ = open::that(&url);
            }
            return vec![];
        }
        (mods, KeyCode::Char('s')) if mods == KeyModifiers::CONTROL | KeyModifiers::ALT => {
            let query = build_external_search_query(state);
            if !query.is_empty() {
                let url = crate::services::external_search::generate_search_url(
                    crate::services::external_search::SearchTarget::Spotify, &query);
                let _ = open::that(&url);
            }
            return vec![];
        }
        (mods, KeyCode::Char('y')) if mods == KeyModifiers::CONTROL | KeyModifiers::ALT => {
            let query = build_external_search_query(state);
            if !query.is_empty() {
                let url = crate::services::external_search::generate_search_url(
                    crate::services::external_search::SearchTarget::YouTube, &query);
                let _ = open::that(&url);
            }
            return vec![];
        }

        _ => {}
    }

    // Sort popup handling (takes priority over view-specific handling)
    if state.popups.sort.is_some() {
        return sort_popup::handle_sort_popup_keys(key, state);
    }

    // Adventure launcher popup handling (takes priority over view-specific handling)
    if state.popups.adventure_launcher.is_some() {
        return adventure_launcher::handle_adventure_launcher_keys(key, state);
    }

    // Radio launcher popup handling (takes priority over view-specific handling)
    if state.popups.radio_launcher.is_some() {
        return radio_launcher::handle_radio_launcher_keys(key, state);
    }

    // Artist radio picker popup handling
    if state.popups.artist_radio_picker.is_some() {
        return artist_radio_picker::handle_artist_radio_picker_keys(key, state);
    }

    // Search popup handling (takes priority over view-specific handling)
    if state.popups.search_active {
        return search::handle_search_keys(key, state);
    }

    // Artist bio popup handling
    if state.popups.artist_bio.is_some() {
        return handle_artist_bio_popup_keys(key, state);
    }

    // Library picker popup handling
    if state.popups.library_picker_active {
        return handle_library_picker_keys(key, state);
    }

    // View-specific handling
    match state.view {
        View::Auth => handle_auth_keys(key, state),
        View::Browse => browse::handle_browse_keys(key, state),
        View::Queue => now_playing::handle_queue_keys(key, state),
        View::NowPlaying => now_playing::handle_now_playing_visualizer_keys(key, state),
        View::Search => search::handle_search_keys(key, state),
        View::Similar => similar::handle_similar_keys(key, state),
        View::Related => related::handle_related_keys(key, state),
        View::Help => settings::handle_help_keys(key, state),
        View::Settings => settings::handle_settings_keys(key, state, config),
    }
}

/// Handle keys when library picker popup is active.
fn handle_library_picker_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    // Build flat list matching what render_library_picker shows
    let multi_server = state.has_multiple_servers();
    let all_libs: Vec<(&str, &str, &crate::plex::models::Library)> = if multi_server {
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
        state.popups.library_picker_active = false;
        return vec![];
    }

    match key.code {
        KeyCode::Esc => {
            return vec![Action::CloseLibraryPicker];
        }
        KeyCode::Up => {
            if state.popups.library_picker_index > 0 {
                state.popups.library_picker_index -= 1;
            }
        }
        KeyCode::Down => {
            if state.popups.library_picker_index + 1 < lib_count {
                state.popups.library_picker_index += 1;
            }
        }
        KeyCode::Home => {
            state.popups.library_picker_index = 0;
        }
        KeyCode::End => {
            state.popups.library_picker_index = lib_count.saturating_sub(1);
        }
        KeyCode::Enter => {
            if let Some((server_id, _, lib)) = all_libs.get(state.popups.library_picker_index) {
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

/// Handle keys when artist bio popup is active.
fn handle_artist_bio_popup_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    match key.code {
        KeyCode::Esc | KeyCode::F(4) => {
            state.popups.artist_bio = None;
        }
        KeyCode::Up => {
            if let Some(ref mut popup) = state.popups.artist_bio {
                popup.scroll = popup.scroll.saturating_sub(1);
            }
        }
        KeyCode::Down => {
            if let Some(ref mut popup) = state.popups.artist_bio {
                popup.scroll = popup.scroll.saturating_add(1);
            }
        }
        KeyCode::PageUp => {
            if let Some(ref mut popup) = state.popups.artist_bio {
                popup.scroll = popup.scroll.saturating_sub(10);
            }
        }
        KeyCode::PageDown => {
            if let Some(ref mut popup) = state.popups.artist_bio {
                popup.scroll = popup.scroll.saturating_add(10);
            }
        }
        KeyCode::Home => {
            if let Some(ref mut popup) = state.popups.artist_bio {
                popup.scroll = 0;
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
///
/// Priority: highlighted artist → highlighted track → highlighted album → now-playing track.
pub(crate) fn get_similar_action(state: &mut AppState) -> Vec<Action> {
    // Store current view so we can return to it
    state.previous_view = Some(state.view);

    // 0. Highlighted artist → LoadSimilarArtists
    if state.view == View::Browse {
        if let Some(nav) = state.browse_nav() {
            if let Some(item) = nav.selected_item() {
                if let BrowseItem::Artist { key, title, .. } = item {
                    let key = key.clone();
                    let title = title.clone();
                    state.similar.tab_album_key = None;
                    state.similar.tab_album_title = None;
                    state.similar.tab_track_key = None;
                    state.similar.tab_track_title = None;
                    return vec![Action::LoadSimilarArtists {
                        artist_key: key,
                        title,
                    }];
                }
            }
        }
    }

    // 1. Highlighted track → LoadSimilarTracks
    if let Some(track) = get_selected_track(state) {
        let title = format!("{} - {}", track.artist_name(), track.title);
        state.similar.tab_album_key = track.parent_rating_key.clone();
        state.similar.tab_album_title = Some(track.album_name().to_string());
        state.similar.tab_track_key = Some(track.rating_key.clone());
        state.similar.tab_track_title = Some(title.clone());
        return vec![Action::LoadSimilarTracks {
            rating_key: track.rating_key.clone(),
            title,
        }];
    }

    // 2. Highlighted album → LoadSimilarAlbums
    if let Some((rating_key, title)) = get_selected_album(state) {
        state.similar.tab_album_key = None;
        state.similar.tab_album_title = None;
        state.similar.tab_track_key = None;
        state.similar.tab_track_title = None;
        return vec![Action::LoadSimilarAlbums {
            rating_key,
            title,
        }];
    }

    // 3. Fallback: now-playing track → LoadSimilarTracks
    if let Some(track) = state.current_track().cloned() {
        let title = format!("{} - {}", track.artist_name(), track.title);
        state.similar.tab_album_key = track.parent_rating_key.clone();
        state.similar.tab_album_title = Some(track.album_name().to_string());
        state.similar.tab_track_key = Some(track.rating_key.clone());
        state.similar.tab_track_title = Some(title.clone());
        return vec![Action::LoadSimilarTracks {
            rating_key: track.rating_key.clone(),
            title,
        }];
    }

    vec![]
}

/// Get the related artists action based on current context.
///
/// Priority: highlighted artist → highlighted album's artist → highlighted track's artist → now-playing track's artist.
pub(crate) fn get_related_action(state: &mut AppState) -> Vec<Action> {
    state.previous_view = Some(state.view);

    // 1. Highlighted artist in Browse nav
    if state.view == View::Browse {
        if let Some(nav) = state.browse_nav() {
            if let Some(item) = nav.selected_item() {
                if let BrowseItem::Artist { key, title, .. } = item {
                    return vec![Action::LoadRelated { artist_key: key.clone(), title: title.clone() }];
                }
            }
        }
    }

    // 2. Highlighted album → use album's parent artist
    if let Some((_album_key, _album_title)) = get_selected_album(state) {
        if let Some(nav) = state.browse_nav() {
            if let Some(artist_key) = find_artist_key_in_nav(nav) {
                let artist_name = find_artist_name_in_nav(nav, state);
                return vec![Action::LoadRelated { artist_key, title: artist_name }];
            }
        }
    }

    // 3. Highlighted track → use track's grandparent artist
    if let Some(track) = get_selected_track(state) {
        if let Some(artist_key) = track.grandparent_rating_key.clone() {
            let artist_name = track.artist_name().to_string();
            return vec![Action::LoadRelated { artist_key, title: artist_name }];
        }
    }

    // 4. Now-playing track → use its artist
    if let Some(track) = state.current_track().cloned() {
        if let Some(artist_key) = track.grandparent_rating_key.clone() {
            let artist_name = track.artist_name().to_string();
            return vec![Action::LoadRelated { artist_key, title: artist_name }];
        }
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

/// Navigate to the album of the currently selected track (Ctrl+B).
/// Switches to Browse/Artists, finds the artist, loads albums, and auto-selects the album.
///
/// Priority:
/// - In Library view: skip Miller/folder context (you're already there), use now-playing track
/// - Otherwise: highlighted track → Miller/folder album context → now-playing track
pub(crate) fn navigate_to_album(state: &mut AppState) -> Vec<Action> {
    let in_library = state.view == View::Browse && state.browse_category == BrowseCategory::Library;

    let (album_key, artist_key, album_title, artist_name) = if in_library {
        // In Library view, always use now-playing track (user is already browsing albums)
        if let Some(track) = state.current_track().cloned() {
            let ak = match &track.parent_rating_key { Some(k) => k.clone(), None => return vec![] };
            let rk = match &track.grandparent_rating_key { Some(k) => k.clone(), None => return vec![] };
            (ak, rk, track.album_name().to_string(), track.artist_name().to_string())
        } else {
            return vec![];
        }
    } else if let Some(track) = get_selected_track(state) {
        // Highlighted track takes first priority outside Library
        let ak = match &track.parent_rating_key { Some(k) => k.clone(), None => return vec![] };
        let rk = match &track.grandparent_rating_key { Some(k) => k.clone(), None => return vec![] };
        (ak, rk, track.album_name().to_string(), track.artist_name().to_string())
    } else if let Some(ctx) = get_miller_album_context(state) {
        ctx
    } else if let Some(ctx) = get_folder_album_context(state) {
        ctx
    } else if let Some(track) = state.current_track().cloned() {
        // Fallback: now-playing track
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
    state.set_view(View::Browse);
    state.browse_category = BrowseCategory::Library;

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

/// Extract album context from Miller columns: (album_key, artist_key, album_title, artist_name).
/// Works when a Track or Album is selected in the artist/genre/playlist navigation.
fn get_miller_album_context(state: &AppState) -> Option<(String, String, String, String)> {
    if state.view != View::Browse {
        return None;
    }

    let nav = state.browse_nav()?;

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
    match state.view {
        // Search popup - get track from search results
        View::Search => {
            let idx = state.list_state.search_item_index;
            if let Some(ref results) = state.search_results {
                match state.search_tab {
                    crate::app::state::SearchTab::Tracks => {
                        return results.tracks.get(idx).cloned();
                    }
                    crate::app::state::SearchTab::Global => {
                        // In All tab, need to resolve global index
                        let offset = results.artists.len() + results.albums.len()
                            + results.playlists.len() + results.genres.len();
                        if idx >= offset && idx < offset + results.tracks.len() {
                            return results.tracks.get(idx - offset).cloned();
                        }
                    }
                    _ => {}
                }
            }
            None
        }

        // Now Playing / Queue views - get highlighted track from queue or radio
        View::NowPlaying | View::Queue => {
            let idx = state.list_state.queue_index;
            match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => {
                    state.queue.get(idx).cloned()
                }
                PlaybackMode::Radio => {
                    state.radio.tracks.get(idx).cloned()
                }
            }
        }

        // Browse view - check Miller columns first, then right panel
        View::Browse => {
            // Miller column Track item → get full Track from column's tracks vec
            if let Some(nav) = state.browse_nav() {
                if let Some(col) = nav.columns.get(nav.focused_column) {
                    if let Some(BrowseItem::Track { .. }) = col.items.get(col.selected_index) {
                        if let Some(track) = col.tracks.get(col.selected_index) {
                            return Some(track.clone());
                        }
                    }
                }
            }
            // Legacy right panel tracks
            match state.right_panel_mode {
                RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                    state.selected_album_tracks.get(state.list_state.tracks_index).cloned()
                }
                _ => None
            }
        }

        // Similar view - return highlighted track when in Tracks mode
        View::Similar => {
            use crate::app::state::SimilarMode;
            if state.similar.mode == SimilarMode::Tracks {
                state.similar.tracks.get(state.list_state.similar_index).cloned()
            } else {
                None
            }
        }

        // Other views don't show selectable tracks
        _ => None
    }
}

/// Get the currently selected/highlighted album based on context.
/// Returns (rating_key, title) for the highlighted album in any view.
fn get_selected_album(state: &AppState) -> Option<(String, String)> {
    match state.view {
        View::Browse => {
            // Miller column Album item
            if let Some(nav) = state.browse_nav() {
                if let Some(item) = nav.selected_item() {
                    if let BrowseItem::Album { key, title, artist, .. } = item {
                        return Some((key.clone(), format!("{} - {}", artist, title)));
                    }
                }
            }
            // Legacy right panel: ArtistAlbums (index > 0) or CategoryAlbums
            match state.right_panel_mode {
                RightPanelMode::ArtistAlbums if state.list_state.right_albums_index > 0 => {
                    let album_idx = state.list_state.right_albums_index.saturating_sub(1);
                    state.selected_artist_albums.get(album_idx).map(|a| {
                        (a.rating_key.clone(), format!("{} - {}", a.artist_name(), a.title))
                    })
                }
                RightPanelMode::CategoryAlbums => {
                    state.genre_albums.get(state.genre_albums_index).map(|a| {
                        (a.rating_key.clone(), format!("{} - {}", a.artist_name(), a.title))
                    })
                }
                _ => None,
            }
        }
        View::Similar => {
            use crate::app::state::SimilarMode;
            if state.similar.mode == SimilarMode::Albums {
                state.similar.albums.get(state.list_state.similar_index).map(|a| {
                    (a.rating_key.clone(), format!("{} - {}", a.artist_name(), a.title))
                })
            } else {
                None
            }
        }
        // Queue and NowPlaying don't have album selection
        _ => None,
    }
}

/// Build a search query for external services based on current context.
///
/// Priority: selected artist → selected album → selected track → now-playing track.
/// Returns an empty string if nothing is available.
fn build_external_search_query(state: &AppState) -> String {
    // 1. Selected artist in browse nav
    if state.view == View::Browse {
        if let Some(nav) = state.browse_nav() {
            if let Some(item) = nav.selected_item() {
                if let BrowseItem::Artist { title, .. } = item {
                    return title.clone();
                }
            }
        }
    }

    // 2. Selected album
    if let Some((_key, title)) = get_selected_album(state) {
        return title; // Already formatted as "artist - album"
    }

    // 3. Selected track
    if let Some(track) = get_selected_track(state) {
        return format!("{} - {}", track.artist_name(), track.album_name());
    }

    // 4. Now-playing fallback
    if let Some(track) = state.current_track() {
        return format!("{} - {}", track.artist_name(), track.album_name());
    }

    String::new()
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
            BrowseCategory::Library => {
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
                if let Some(idx) = state.genres.iter().position(|g| starts_with(&g.title)) {
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
