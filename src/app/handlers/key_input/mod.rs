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
pub use self::alt_commands::{AltCommand, CommandModifier, available_alt_commands};

mod alt_commands;
pub(crate) mod sort_popup;

use crate::app::action::*;
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
        // Pressing any quit shortcut a second time confirms immediately.
        let repeat_quit = matches!(dialog.on_confirm, crate::app::state::ConfirmAction::Quit)
            && match (key.modifiers, key.code) {
                (KeyModifiers::CONTROL, KeyCode::Char('q')) => true,
                (KeyModifiers::SUPER,   KeyCode::Char('q')) => true,
                (KeyModifiers::SUPER,   KeyCode::Char('w')) => true,
                (KeyModifiers::ALT,     KeyCode::F(4))      => true,
                _ => false,
            };
        if repeat_quit {
            return vec![SystemAction::Quit.into()];
        }
        match key.code {
            KeyCode::Enter => {
                if dialog.selected_yes {
                    use crate::app::state::ConfirmAction;
                    return match dialog.on_confirm {
                        ConfirmAction::RefreshCache => helpers::refresh_current_view(state),
                        ConfirmAction::ClearLibraryCache => vec![SettingsAction::ClearLibraryCache.into()],
                        ConfirmAction::ClearArtworkCache => vec![SettingsAction::ClearArtworkCache.into()],
                        ConfirmAction::ClearSubfolderCache => vec![SettingsAction::ClearSubfolderCache.into()],
                        ConfirmAction::Quit => vec![SystemAction::Quit.into()],
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
                    ConfirmAction::ClearLibraryCache => vec![SettingsAction::ClearLibraryCache.into()],
                    ConfirmAction::ClearArtworkCache => vec![SettingsAction::ClearArtworkCache.into()],
                    ConfirmAction::ClearSubfolderCache => vec![SettingsAction::ClearSubfolderCache.into()],
                    ConfirmAction::Quit => vec![SystemAction::Quit.into()],
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
                    return vec![SettingsAction::CancelAdventure.into()];
                }
            }
            KeyCode::Enter => {
                // Confirm dialog
                let input = dialog.input.clone();
                let action_type = dialog.action_type.clone();
                state.popups.input_dialog = None;
                match action_type {
                    crate::app::state::InputDialogAction::SavePlaylist => {
                        return vec![QueueAction::SaveQueueAsPlaylist(input).into()];
                    }
                    crate::app::state::InputDialogAction::AdventureLength => {
                        // Parse the length (default to 20)
                        let length = input.parse::<usize>().unwrap_or(20).clamp(5, 100);
                        return vec![SettingsAction::SetAdventureLength(length).into()];
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
            return vec![SettingsAction::CancelAdventure.into()];
        }
    }

    // Global CUA shortcuts (work everywhere)
    //
    // Quit shortcuts (with confirmation):
    //   - Ctrl+Q       (Linux/Windows/TUI standard)
    //   - Cmd+Q        (Mac standard — `SUPER` is the cross-platform
    //                   crossterm name for the OS Logo / Cmd / Win key)
    //   - Alt+F4       (Windows standard)
    //
    // Cmd+W on Mac GUI is rebound to Ctrl+W upstream (close current
    // Miller column), so it does NOT quit. The single-window-app
    // convention loses to the column-close affordance which is far more
    // frequently useful.
    let is_quit_keypress = match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('q')) => true,
        (KeyModifiers::SUPER,   KeyCode::Char('q')) => true,
        (KeyModifiers::ALT,     KeyCode::F(4))      => true,
        _ => false,
    };
    match (key.modifiers, key.code) {
        _ if is_quit_keypress => {
            // Quit immediately. The previous confirmation dialog was
            // muscle-memory hostile — every Cmd+Q / Ctrl+Q press needed
            // a second confirmation, and the platform conventions are
            // already destructive ("close window") so users expect them
            // to act without a prompt.
            state.popups.close_all();
            return vec![SystemAction::Quit.into()];
        }

        // Cmd+A / Ctrl+A — select all rows in the currently focused
        // list. Works on the queue (when Now Playing / Queue view is
        // up) and on the focused Miller track column. The shared
        // dispatchers (RemoveSelectedFromQueue, MoveSelectedTracksUp/
        // Down, the GUI's bulk-enqueue context-menu items) already
        // read these `selected_set` collections.
        (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
            match state.view {
                View::Queue | View::NowPlaying => {
                    let n = state.queue.tracks.len();
                    state.queue.selected = (0..n).collect();
                    return vec![];
                }
                View::Browse => {
                    if let Some(nav) = state.browse_nav_mut() {
                        if let Some(col) = nav.focused_mut() {
                            // Only meaningful for track columns. For
                            // artist/album columns the multi-select
                            // wouldn't drive any current action.
                            let is_track_col = col.items.first()
                                .map(|it| matches!(it, BrowseItem::Track { .. }))
                                .unwrap_or(false);
                            if is_track_col {
                                col.selected_set = (0..col.items.len()).collect();
                            }
                        }
                    }
                    return vec![];
                }
                _ => return vec![],
            }
        }

        // Global navigation shortcuts
        (KeyModifiers::CONTROL, KeyCode::Char('f')) => {
            // Ctrl+F = Search/Filter popup (floating dialog)
            if state.popups.search_active {
                return vec![SearchAction::CloseSearchPopup.into()];
            } else {
                return vec![SearchAction::OpenSearchPopup.into()];
            }
        }
        (KeyModifiers::CONTROL, KeyCode::Char('g')) => {
            // Ctrl+G = Genres category (no cycling — use Tab to switch tabs)
            if state.view == View::Browse && state.browse_category == BrowseCategory::Genres {
                return vec![];
            }
            // Not in genres view - switch to it and reset right panel
            state.set_browse_category(BrowseCategory::Genres);
            reset_right_panel(state);
            // RefreshGenreView uses cached data when available, only fetches if empty
            return vec![BrowseAction::RefreshGenreView.into(), NavigationAction::SetView(View::Browse).into(), SystemAction::CheckStaleness(crate::app::state::RefreshCategory::Genres).into()];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
            // Ctrl+N = Now Playing (visualizer view)
            return vec![NavigationAction::SetView(View::NowPlaying).into(), SystemAction::LoadWaveform.into()];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
            // Ctrl+U = Queue view
            return vec![NavigationAction::SetView(View::Queue).into()];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
            // Ctrl+L = Library category (no cycling — Plex doesn't distinguish album artists)
            if state.view == View::Browse && state.browse_category == BrowseCategory::Library {
                return vec![];
            }
            // Not in library view - switch to it and reset right panel
            state.set_browse_category(BrowseCategory::Library);
            reset_right_panel(state);
            let tier1 = crate::app::state::RefreshCategory::Artists;
            if state.library.artists.is_empty() {
                return vec![DataAction::LoadArtists.into(), NavigationAction::SetView(View::Browse).into(), SystemAction::CheckStaleness(tier1).into()];
            }
            return vec![NavigationAction::SetView(View::Browse).into(), SystemAction::CheckStaleness(tier1).into()];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
            // Ctrl+P = Playlists category
            if state.view == View::Browse && state.browse_category == BrowseCategory::Playlists {
                return vec![];
            }
            state.set_browse_category(BrowseCategory::Playlists);
            reset_right_panel(state);
            let mut actions = vec![NavigationAction::SetView(View::Browse).into()];
            if state.library.playlists.is_empty() {
                actions.insert(0, DataAction::LoadPlaylists.into());
            } else {
                let items = crate::app::state::BrowseItem::from_playlists(&state.library.playlists);
                state.playlist_nav.reset("playlists", items);
            }
            actions.push(SystemAction::CheckStaleness(crate::app::state::RefreshCategory::Playlists).into());
            return actions;
        }
        (KeyModifiers::CONTROL, KeyCode::Char('o')) => {
            // Ctrl+O = Folders category
            state.set_browse_category(BrowseCategory::Folders);
            reset_right_panel(state);
            let staleness = SystemAction::CheckStaleness(crate::app::state::RefreshCategory::Folders).into();
            if state.folder_state.is_none() {
                return vec![FolderAction::LoadFolderRoot.into(), NavigationAction::SetView(View::Browse).into(), staleness];
            }
            return vec![NavigationAction::SetView(View::Browse).into(), staleness];
        }

        // Global function keys - work from any screen
        (_, KeyCode::F(1)) => {
            if state.view != View::Help {
                return vec![NavigationAction::SetView(View::Help).into()];
            }
        }
        (_, KeyCode::F(2)) => {
            if state.view != View::Settings {
                return vec![SettingsAction::OpenSettings.into()];
            }
        }
        (_, KeyCode::F(3)) => {
            // F3 = Quick library switcher
            if !state.libraries.is_empty() {
                return vec![SearchAction::OpenLibraryPicker.into()];
            }
        }
        (_, KeyCode::F(4)) => {
            // F4 = Artist bio popup
            // Priority: selected track → selected album → selected artist → now-playing track
            if let Some((artist_key, artist_name)) = helpers::get_artist_for_bio(state) {
                return vec![SearchAction::ShowArtistBio { artist_key, artist_name }.into()];
            }
        }
        (_, KeyCode::F(5)) => {
            // F5 = Refresh current view
            return helpers::refresh_current_view(state);
        }
        // Space: multi-select on track lists (queue or focused
        // Miller track column), play/pause everywhere else. Plain
        // Space toggles select mode and clears prior selection on
        // entry; Ctrl+Space, Alt+Space, or Shift+Space activates
        // the mode WITHOUT clearing so multiple ranges can be
        // combined. We accept all three because terminals vary in
        // which modifier+Space combos they actually disambiguate
        // — Ctrl+Space is the most reliably distinct (most
        // terminals send it as Ctrl+Char(' ') or KeyCode::Null).
        (mods, KeyCode::Char(' '))
            if state.view != View::Search && !state.list_filter.active
                && !state.popups.search_active
                && state.popups.radio_launcher.is_none()
                && state.popups.adventure_launcher.is_none()
                && state.popups.artist_radio_picker.is_none() =>
        {
            let add_to_selection = mods.contains(KeyModifiers::SHIFT)
                || mods.contains(KeyModifiers::ALT)
                || mods.contains(KeyModifiers::CONTROL);
            if toggle_select_mode_on_focused_list(state, add_to_selection) {
                return vec![];
            }
            return vec![PlaybackAction::TogglePlayPause.into()];
        }
        // Some terminals (xterm, Terminal.app) send Ctrl+Space as
        // KeyCode::Null instead of Char(' ') with the CTRL flag.
        // Treat it as the "add to selection" trigger.
        (_, KeyCode::Null)
            if state.view != View::Search && !state.list_filter.active
                && !state.popups.search_active
                && state.popups.radio_launcher.is_none()
                && state.popups.adventure_launcher.is_none()
                && state.popups.artist_radio_picker.is_none() =>
        {
            if toggle_select_mode_on_focused_list(state, true) {
                return vec![];
            }
            return vec![];
        }
        // < and > for prev/next track (crossterm reports these with NONE modifiers, not SHIFT)
        (_, KeyCode::Char('<')) if state.view != View::Search && !state.list_filter.active && !state.popups.search_active && state.popups.radio_launcher.is_none() && state.popups.adventure_launcher.is_none() => {
            return vec![PlaybackAction::Previous.into()];
        }
        (_, KeyCode::Char('>')) if state.view != View::Search && !state.list_filter.active && !state.popups.search_active && state.popups.radio_launcher.is_none() && state.popups.adventure_launcher.is_none() => {
            return vec![PlaybackAction::Next.into()];
        }
        // Ctrl+Shift+Up/Down: multi-select in Queue view, volume elsewhere
        (mods, KeyCode::Up) if mods == KeyModifiers::CONTROL | KeyModifiers::SHIFT && state.view == View::Queue => {
            // Toggle current item into queue_selected, then move cursor up
            let queue_idx = state.list_state.queue_index;
            if queue_idx < state.queue.tracks.len() {
                if state.queue.selected.contains(&queue_idx) {
                    state.queue.selected.remove(&queue_idx);
                } else {
                    state.queue.selected.insert(queue_idx);
                }
            }
            if state.list_state.queue_index > 0 {
                state.list_state.queue_index -= 1;
            }
            return vec![];
        }
        (mods, KeyCode::Down) if mods == KeyModifiers::CONTROL | KeyModifiers::SHIFT && state.view == View::Queue => {
            let queue_idx = state.list_state.queue_index;
            if queue_idx < state.queue.tracks.len() {
                if state.queue.selected.contains(&queue_idx) {
                    state.queue.selected.remove(&queue_idx);
                } else {
                    state.queue.selected.insert(queue_idx);
                }
            }
            let max = state.queue.tracks.len().saturating_sub(1);
            state.list_state.queue_index = (state.list_state.queue_index + 1).min(max);
            return vec![];
        }
        (mods, KeyCode::Up) if mods == KeyModifiers::CONTROL | KeyModifiers::SHIFT => {
            state.volume_slider_until = Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
            return vec![PlaybackAction::VolumeUp.into()];
        }
        (mods, KeyCode::Down) if mods == KeyModifiers::CONTROL | KeyModifiers::SHIFT => {
            state.volume_slider_until = Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
            return vec![PlaybackAction::VolumeDown.into()];
        }
        // Shift+Left/Right for seeking (10 second skip)
        (KeyModifiers::SHIFT, KeyCode::Left) => return vec![PlaybackAction::SeekRelative(-10000).into()],
        (KeyModifiers::SHIFT, KeyCode::Right) => return vec![PlaybackAction::SeekRelative(10000).into()],
        // Action commands (Ctrl+key) — gated by availability check
        // Ctrl+E: Add to END of queue (skip if in search popup - handled there)
        (KeyModifiers::CONTROL, KeyCode::Char('e')) if !state.popups.search_active && alt_commands::is_action_command_available(state, 'e') => {
            return vec![QueueAction::EnqueueSelection.into()];
        }
        // Ctrl+Shift+E: Insert NEXT in queue after current track (skip if in search popup - handled there)
        (mods, KeyCode::Char('e')) | (mods, KeyCode::Char('E')) if !state.popups.search_active && mods == KeyModifiers::CONTROL | KeyModifiers::SHIFT && alt_commands::is_action_command_available(state, 'e') => {
            return vec![QueueAction::EnqueueSelectionNext.into()];
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
        // Ctrl+S = Save queue as playlist (standard Save shortcut).
        (KeyModifiers::CONTROL, KeyCode::Char('s')) if alt_commands::is_action_command_available(state, 's') => {
            return vec![QueueAction::PromptSavePlaylist.into()];
        }
        // Ctrl+V = View options popup (sort modes, direction,
        // group-by-album, cover-art toggle) for the focused column.
        (KeyModifiers::CONTROL, KeyCode::Char('v')) => {
            return vec![SearchAction::OpenSortPopup.into()];
        }
        // Ctrl+W = Close current Miller column. Drops the focused
        // column + any cols to its right and moves focus one left.
        // At the root (or the hidden Playlists col-offset prefix),
        // falls back to focusing the cat col so the user can pivot
        // to another category.
        (KeyModifiers::CONTROL, KeyCode::Char('w')) if state.view == View::Browse => {
            close_focused_browse_column(state);
            return vec![];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('x')) if alt_commands::is_action_command_available(state, 'x') => {
            return vec![QueueAction::ClearQueue.into()];
        }
        // Alt shortcuts (station/global commands)
        (KeyModifiers::ALT, KeyCode::Char('f')) => {
            // Alt+F = Activate inline filter (Browse view only)
            if state.view == View::Browse && !state.list_filter.active
                && !state.popups.search_active && state.popups.sort.is_none()
                && state.popups.radio_launcher.is_none() && state.popups.adventure_launcher.is_none()
                && state.popups.artist_radio_picker.is_none()
            {
                return vec![SearchAction::ActivateListFilter.into()];
            }
            return vec![];
        }
        (KeyModifiers::ALT, KeyCode::Char('r')) => {
            // Alt+R = Play Random Album Radio station
            if let Some(lib_key) = &state.active_library {
                let key = format!("/library/sections/{}/stations/randomAlbum", lib_key);
                return vec![RadioAction::PlayStation(key).into()];
            }
            return vec![];
        }
        // F6 = Sort popup for current column. Was Ctrl+S until that
        // became the standard Save shortcut; F6 keeps sort reachable
        // from the keyboard without colliding with save.
        (KeyModifiers::NONE, KeyCode::F(6)) => {
            return vec![SearchAction::OpenSortPopup.into()];
        }

        // External-search keyboard shortcuts have all been retired —
        // Apple Music / Spotify / YouTube search are palette- and
        // menu-driven only.

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

    // Global inline filter handler — once `/` activates the filter,
    // every printable key goes to the query, Backspace deletes,
    // Esc cancels, Enter promotes to the global Search popup. This
    // runs BEFORE the view-specific dispatch so filtering works
    // on Queue / Now Playing / Similar / etc., not just Browse.
    if state.list_filter.active {
        if let Some(actions) = handle_filter_input(key, state) {
            return actions;
        }
        // Fall through for keys we don't handle here (Tab, etc.).
    }

    // View-specific handling
    let actions = match state.view {
        View::Auth => handle_auth_keys(key, state),
        View::Browse => browse::handle_browse_keys(key, state),
        View::Queue => now_playing::handle_queue_keys(key, state),
        View::NowPlaying => now_playing::handle_now_playing_visualizer_keys(key, state),
        View::Search => search::handle_search_keys(key, state),
        View::Similar => similar::handle_similar_keys(key, state),
        View::Related => related::handle_related_keys(key, state),
        View::Help => settings::handle_help_keys(key, state),
        View::Settings => settings::handle_settings_keys(key, state, config),
    };

    // Select-mode lifecycle:
    //   - Plain Up / Down (no modifiers) extends the selection.
    //   - Any other key exits select mode, including PgUp/Down,
    //     Home/End, Left/Right (e.g. moving to another column),
    //     letter jumps, etc. The selection itself persists so the
    //     user can act on it from outside the mode.
    //   - Space variants are handled in the global match above and
    //     return before reaching this block, so they don't trigger
    //     this exit path.
    let plain_up_down = matches!(key.code, KeyCode::Up | KeyCode::Down)
        && !key.modifiers.contains(KeyModifiers::SHIFT)
        && !key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT);
    if state.select_mode {
        if plain_up_down {
            extend_selection_after_move(state);
        } else {
            state.select_mode = false;
        }
    }

    actions
}

/// Extend the active list's selection set to include the current
/// cursor position. Used by `select_mode` after Up/Down keys.
fn extend_selection_after_move(state: &mut AppState) {
    match state.view {
        View::Queue | View::NowPlaying => {
            let idx = state.list_state.queue_index;
            if idx < state.queue.tracks.len() {
                state.queue.selected.insert(idx);
            }
        }
        View::Browse => {
            if let Some(nav) = state.browse_nav_mut() {
                if let Some(col) = nav.focused_mut() {
                    col.selected_set.insert(col.selected_index);
                }
            }
        }
        _ => {}
    }
}

/// Whether the currently-focused list supports multi-select (a
/// queue track list or a Miller column whose first item is a
/// `BrowseItem::Track`).
fn focused_list_is_track_list(state: &AppState) -> bool {
    match state.view {
        View::Queue | View::NowPlaying => !state.queue.tracks.is_empty(),
        View::Browse => state
            .browse_nav()
            .and_then(|n| n.columns.get(n.focused_column))
            .and_then(|c| c.items.first())
            .map(|it| matches!(it, BrowseItem::Track { .. }))
            .unwrap_or(false),
        _ => false,
    }
}

/// Toggle / activate `select_mode` on the focused track list, if
/// any. Returns `true` if the action was consumed (Space should
/// not fall through to play/pause).
///
///   - Plain Space, mode off → clear selection, enter mode, mark current item
///   - Plain Space, mode on  → exit mode (selection persists)
///   - Shift+Space          → enter mode without clearing; mark current item
fn toggle_select_mode_on_focused_list(state: &mut AppState, shift: bool) -> bool {
    if !focused_list_is_track_list(state) {
        return false;
    }
    if shift {
        state.select_mode = true;
        match state.view {
            View::Queue | View::NowPlaying => {
                let idx = state.list_state.queue_index;
                if idx < state.queue.tracks.len() {
                    state.queue.selected.insert(idx);
                }
            }
            View::Browse => {
                if let Some(nav) = state.browse_nav_mut() {
                    if let Some(col) = nav.focused_mut() {
                        col.selected_set.insert(col.selected_index);
                    }
                }
            }
            _ => {}
        }
        return true;
    }
    if state.select_mode {
        // Exit mode, keep selection so the user can act on it.
        state.select_mode = false;
    } else {
        // Enter mode: clear any prior selection first, then mark
        // the current item.
        state.select_mode = true;
        match state.view {
            View::Queue | View::NowPlaying => {
                state.queue.selected.clear();
                let idx = state.list_state.queue_index;
                if idx < state.queue.tracks.len() {
                    state.queue.selected.insert(idx);
                }
            }
            View::Browse => {
                if let Some(nav) = state.browse_nav_mut() {
                    if let Some(col) = nav.focused_mut() {
                        col.selected_set.clear();
                        col.selected_set.insert(col.selected_index);
                    }
                }
            }
            _ => {}
        }
    }
    true
}

/// Process a key while the inline filter is active. Returns `Some`
/// when the key was consumed, `None` to fall through to the view's
/// own handler (e.g. Tab to switch panes, F-keys, etc.).
fn handle_filter_input(key: event::KeyEvent, state: &mut AppState) -> Option<Vec<Action>> {
    use crate::app::action::SearchAction;
    if key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }
    match key.code {
        KeyCode::Esc => Some(vec![SearchAction::DeactivateListFilter.into()]),
        KeyCode::Backspace => Some(vec![SearchAction::DeleteListFilterChar.into()]),
        KeyCode::Enter => {
            let q = state.list_filter.query.trim().to_string();
            if q.is_empty() {
                Some(vec![])
            } else {
                Some(vec![
                    SearchAction::OpenSearchPopup.into(),
                    SearchAction::SetSearchQuery(q).into(),
                ])
            }
        }
        KeyCode::Char(c) => Some(vec![SearchAction::AppendListFilterChar(c).into()]),
        // Up/Down/Left/Right/Tab fall through to the view's handler
        // so the user can keep navigating the filtered list while
        // continuing to type.
        _ => None,
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
            return vec![SearchAction::CloseLibraryPicker.into()];
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
                        SettingsAction::SelectLibraryOnServer(lib_key, server_id.to_string()).into(),
                        SearchAction::CloseLibraryPicker.into(),
                    ];
                } else {
                    return vec![SettingsAction::SelectLibrary(lib_key).into(), SearchAction::CloseLibraryPicker.into()];
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
                            return vec![SettingsAction::AuthSignIn.into()];
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
                            vec![SettingsAction::AuthSignIn.into()]
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
                    vec![SettingsAction::AuthSelectServer.into()]
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
                    return vec![DataAction::LoadSimilarArtists {
                        artist_key: key,
                        title,
                    }.into()];
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
        return vec![DataAction::LoadSimilarTracks {
            rating_key: track.rating_key.clone(),
            title,
        }.into()];
    }

    // 2. Highlighted album → LoadSimilarAlbums
    if let Some((rating_key, title)) = get_selected_album(state) {
        state.similar.tab_album_key = None;
        state.similar.tab_album_title = None;
        state.similar.tab_track_key = None;
        state.similar.tab_track_title = None;
        return vec![DataAction::LoadSimilarAlbums {
            rating_key,
            title,
        }.into()];
    }

    // 3. Fallback: now-playing track → LoadSimilarTracks
    if let Some(track) = state.current_track().cloned() {
        let title = format!("{} - {}", track.artist_name(), track.title);
        state.similar.tab_album_key = track.parent_rating_key.clone();
        state.similar.tab_album_title = Some(track.album_name().to_string());
        state.similar.tab_track_key = Some(track.rating_key.clone());
        state.similar.tab_track_title = Some(title.clone());
        return vec![DataAction::LoadSimilarTracks {
            rating_key: track.rating_key.clone(),
            title,
        }.into()];
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
                    return vec![DataAction::LoadRelated { artist_key: key.clone(), title: title.clone() }.into()];
                }
            }
        }
    }

    // 2. Highlighted album → use album's parent artist
    if let Some((_album_key, _album_title)) = get_selected_album(state) {
        if let Some(nav) = state.browse_nav() {
            if let Some(artist_key) = find_artist_key_in_nav(nav) {
                let artist_name = find_artist_name_in_nav(nav, state);
                return vec![DataAction::LoadRelated { artist_key, title: artist_name }.into()];
            }
        }
    }

    // 3. Highlighted track → use track's grandparent artist
    if let Some(track) = get_selected_track(state) {
        if let Some(artist_key) = track.grandparent_rating_key.clone() {
            let artist_name = track.artist_name().to_string();
            return vec![DataAction::LoadRelated { artist_key, title: artist_name }.into()];
        }
    }

    // 4. Now-playing track → use its artist
    if let Some(track) = state.current_track().cloned() {
        if let Some(artist_key) = track.grandparent_rating_key.clone() {
            let artist_name = track.artist_name().to_string();
            return vec![DataAction::LoadRelated { artist_key, title: artist_name }.into()];
        }
    }

    vec![]
}

/// Reset right panel state when switching categories.
/// Clears album/track selections and resets focus to left panel.
fn reset_right_panel(state: &mut AppState) {
    state.library.right_panel_mode = RightPanelMode::Empty;
    state.focus = Focus::Left;
    state.library.selected_artist_albums.clear();
    state.library.selected_album_tracks.clear();
    state.library.genre_albums.clear();
    state.library.genre_albums_index = 0;
    state.library.selected_artist_name.clear();
    state.library.selected_album_title.clear();
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
    state.search.pending_album_key = Some(album_key);
    state.library.selected_album_title = album_title;
    state.library.selected_artist_name = artist_name;
    state.set_view(View::Browse);
    state.set_browse_category(BrowseCategory::Library);

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
    if let Some(idx) = state.library.artists.iter().position(|a| a.rating_key == artist_key) {
        state.list_state.artists_index = idx;
    }

    vec![MillerAction::LoadArtistAlbumsForMiller { artist_key }.into()]
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
    state.library.selected_artist_name.clone()
}

/// Get the currently selected/highlighted track based on context.
/// Returns the track the user is highlighting in any view where tracks are visible.
fn get_selected_track(state: &AppState) -> Option<Track> {
    match state.view {
        // Search popup - get track from search results
        View::Search => {
            let idx = state.list_state.search_item_index;
            if let Some(ref results) = state.search.results {
                match state.search.tab {
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
                    state.queue.tracks.get(idx).cloned()
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
            match state.library.right_panel_mode {
                RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                    state.library.selected_album_tracks.get(state.list_state.tracks_index).cloned()
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
            match state.library.right_panel_mode {
                RightPanelMode::ArtistAlbums if state.list_state.right_albums_index > 0 => {
                    let album_idx = state.list_state.right_albums_index.saturating_sub(1);
                    state.library.selected_artist_albums.get(album_idx).map(|a| {
                        (a.rating_key.clone(), format!("{} - {}", a.artist_name(), a.title))
                    })
                }
                RightPanelMode::CategoryAlbums => {
                    state.library.genre_albums.get(state.library.genre_albums_index).map(|a| {
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
/// Close the focused content column. The cat ("section") column is
/// the only un-closable surface — every Miller column, the Folders
/// root, and the track-details pane are closable. The pane closes
/// first if visible (so two Ctrl+W presses tear it down then drop
/// the next column rather than skipping straight to the column
/// underneath). When the last content column closes, focus falls
/// back to the cat col.
pub fn close_focused_browse_column(state: &mut AppState) {
    use crate::app::state::BrowseCategory;

    // Pane visible? Close ONLY the pane.
    if state.pane_track().is_some() {
        state.track_details = None;
        state.track_pane_focused = false;
        state.track_pane_index = 0;
        return;
    }

    // Stale pane-focus flag with no pane visible: clear it instead
    // of popping a Miller column the user wasn't aiming at.
    if state.track_pane_focused {
        state.track_pane_focused = false;
        state.track_pane_index = 0;
        return;
    }

    // Folders use their own nav state. Drop the focused folder col
    // and everything to its right; if that was the root col, the
    // whole folder_state goes — focus falls back to the cat col.
    if state.browse_category == BrowseCategory::Folders {
        let drop_state = state
            .folder_state
            .as_mut()
            .map(|fs| {
                if fs.can_go_left() {
                    fs.focus_left();
                    fs.truncate_right_columns();
                    false
                } else {
                    true
                }
            })
            .unwrap_or(true);
        if drop_state {
            state.folder_state = None;
            state.focus_category_column();
        }
        return;
    }

    // Library / Genres / Playlists. The Playlists nav has a hidden
    // root col (column_offset=1) that mirrors the cat col list — we
    // never let the user "close" past column_offset (would put focus
    // on a column they can't see). For Library/Genres column_offset=0,
    // so closing col 0 drops the entire content nav and falls back
    // to the cat col.
    let column_offset: usize = match state.browse_category {
        BrowseCategory::Playlists => 1,
        _ => 0,
    };

    let drop_nav = if let Some(nav) = state.browse_nav_mut() {
        if nav.focused_column > column_offset {
            nav.focus_left();
            nav.truncate_right();
            false
        } else {
            // Focused col IS the leftmost closable col. Truncate
            // everything (including it) so only the hidden prefix
            // remains. For non-Playlists categories that means an
            // empty `columns` Vec — the cat col is now the only
            // visible surface.
            nav.columns.truncate(column_offset);
            nav.focused_column = column_offset.saturating_sub(1);
            true
        }
    } else {
        true
    };

    if drop_nav {
        state.focus_category_column();
    }
}

pub fn build_external_search_query(state: &AppState) -> String {
    // 0. Highlighted Sonically-Similar row inside the track pane —
    //    when the user has picked a similar song, third-party
    //    search should target THAT track's artist+album, not the
    //    parent miller row's.
    if state.track_pane_focused && state.track_pane_index > 0 {
        if let Some(parent) = state.focused_track() {
            let sim_idx = state.track_pane_index - 1;
            if let Some(sim) = state
                .track_pane_similar
                .get(&parent.rating_key)
                .and_then(|v| v.get(sim_idx))
            {
                return format!("{} - {}", sim.artist_name(), sim.album_name());
            }
        }
    }
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
                if let Some(idx) = state.library.artists.iter().position(|a| starts_with(&a.title)) {
                    state.list_state.artists_index = idx;
                }
            }
            BrowseCategory::Playlists => {
                if let Some(idx) = state.library.playlists.iter().position(|p| starts_with(&p.title)) {
                    state.list_state.playlists_index = idx;
                }
            }
            BrowseCategory::Genres => {
                if let Some(idx) = state.library.genres.iter().position(|g| starts_with(&g.title)) {
                    state.library.genres_index = idx;
                }
            }
            BrowseCategory::Folders => {
                // Handled separately in folder navigation
            }
        }
    } else {
        // Jump in right panel
        match state.library.right_panel_mode {
            RightPanelMode::ArtistAlbums => {
                // +1 offset for "All Tracks" at index 0
                if let Some(idx) = state.library.selected_artist_albums.iter().position(|a| starts_with(&a.title)) {
                    state.list_state.right_albums_index = idx + 1;
                }
            }
            RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                if let Some(idx) = state.library.selected_album_tracks.iter().position(|t| starts_with(&t.title)) {
                    state.list_state.tracks_index = idx;
                }
            }
            RightPanelMode::CategoryAlbums => {
                if let Some(idx) = state.library.genre_albums.iter().position(|a| starts_with(&a.title)) {
                    state.library.genre_albums_index = idx;
                }
            }
            RightPanelMode::Empty => {}
        }
    }
}
