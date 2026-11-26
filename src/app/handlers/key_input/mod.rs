//! Keyboard input handler functions.
//!
//! Split into focused submodules:
//! - `browse` — Browse view key handling (folders, stations, artists, genres, playlists)
//! - `now_playing` — Now Playing view key handling
//! - `search` — Search view key handling
//! - `similar` — Similar view key handling
//! - `settings` — Settings and Help view key handling

pub(in crate::app::handlers) mod adventure_launcher;
mod artist_radio_picker;
mod browse;
mod now_playing;
pub(crate) mod related;
mod search;
mod settings;
pub(crate) mod similar;

// Re-export public items used by other handler modules.
pub use self::alt_commands::{AltCommand, CommandModifier, available_alt_commands};
pub use browse::{
    get_filter_drilldown_actions, truncate_filter_right_columns, update_filter_column_selection,
};

mod alt_commands;
pub(crate) mod sort_popup;

use crate::app::action::*;
use crossterm::event::{self, KeyCode, KeyModifiers};

use super::helpers;
use crate::app::Action;
use crate::app::AppState;
use crate::app::state::{
    BrowseCategory, BrowseItem, BrowseNavigationState, Focus, PlaybackMode, RightPanelMode, View,
};
use crate::library::models::Track;

/// Both shortcut entry points select the same station used by the radio palette.
pub(super) fn random_album_radio(state: &AppState) -> Vec<Action> {
    state
        .random_album_station()
        .map(|station| RadioAction::PlayStation(station.key.clone()).into())
        .into_iter()
        .collect()
}

/// Navigate a browser column independently of a separate workflow's modal focus.
pub fn handle_column_navigation(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    state.focus = crate::app::state::Focus::Right;
    state.category_column_focused = false;
    state.track_pane_focused = false;
    state.alphabet_strip_focused = false;
    browse::handle_browse_keys(key, state)
}

/// Handle shared terminal keyboard input.
pub fn handle_key(
    key: event::KeyEvent,
    state: &mut AppState,
    config: &crate::config::Config,
) -> Vec<Action> {
    if matches!(
        key.code,
        KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right
    ) {
        state.scroll.track_detail = None;
    }
    if !state.desktop_frontend {
        use crossterm::event::{KeyEventKind, ModifierKeyCode};
        if let KeyCode::Modifier(modifier) = key.code {
            if matches!(
                modifier,
                ModifierKeyCode::LeftShift | ModifierKeyCode::RightShift
            ) {
                state.tui_shift_held = key.kind != KeyEventKind::Release;
            }
            return vec![];
        }
        if key.kind == KeyEventKind::Release
            || (key.kind == KeyEventKind::Repeat
                && matches!(key.code, KeyCode::Char('^' | '[' | ']' | '|')))
        {
            return vec![];
        }
    }

    let is_quit_keypress = matches!(
        (key.modifiers, key.code),
        (KeyModifiers::CONTROL, KeyCode::Char('q'))
            | (KeyModifiers::CONTROL, KeyCode::Char('c'))
            | (KeyModifiers::SUPER, KeyCode::Char('q'))
            | (KeyModifiers::ALT, KeyCode::F(4))
    );

    if is_quit_keypress {
        state.popups.close_all();
        return vec![SystemAction::Quit.into()];
    }
    if key.code == KeyCode::Char('p')
        && key.modifiers == KeyModifiers::CONTROL | KeyModifiers::SHIFT
    {
        return crate::app::command_palette::open(state);
    }
    if let Some(command) = crate::app::shortcuts::navigation(key) {
        return crate::app::command_palette::run(command, state);
    }
    if let Some(actions) = crate::app::terminal_text::delete_selection(key, state) {
        return actions;
    }
    if state.palette.open {
        use crate::app::command_palette::{PaletteOutcome, handle_key, run};
        return match handle_key(state, key) {
            PaletteOutcome::Continue => vec![],
            PaletteOutcome::Cancel => {
                state.palette.close();
                vec![]
            }
            PaletteOutcome::Execute(command) => {
                let mut actions = crate::app::command_palette::remember(&command, state);
                state.palette.close();
                actions.extend(run(command, state));
                actions
            }
        };
    }
    if state.popups.input_dialog().is_none()
        && state.popups.library_dialog().is_none()
        && state.popups.confirm_dialog().is_none()
        && state.popups.text().is_none()
        && state.popups.lyrics().is_none()
    {
        if key.code == KeyCode::F(7) && key.modifiers.is_empty() {
            return vec![NavigationAction::ToggleFullscreen.into()];
        }
        if state.fullscreen.is_open() {
            return crate::app::fullscreen::key(key, state);
        }
    }
    if let Some(panel) = state.popups.lyrics_mut() {
        match key.code {
            KeyCode::Esc => {
                state.popups.set_lyrics(None);
                state.sources.nav_tasks.remove("lyrics");
            }
            KeyCode::Char('s') => {
                return vec![crate::app::action::PlaybackAction::ToggleLyricsSync.into()];
            }
            KeyCode::Char('g') => return crate::app::lyrics::Control::Search.actions(state),
            KeyCode::Char('l') => return crate::app::lyrics::Control::NextLanguage.actions(state),
            KeyCode::Char('r') => {
                return vec![crate::app::action::PlaybackAction::RetryLyrics.into()];
            }
            KeyCode::Up => panel.scroll = panel.scroll.saturating_sub(1),
            KeyCode::Down => panel.scroll = panel.scroll.saturating_add(1),
            KeyCode::PageUp => panel.scroll = panel.scroll.saturating_sub(10),
            KeyCode::PageDown => panel.scroll = panel.scroll.saturating_add(10),
            _ => {}
        }
        return vec![];
    }
    if let Some(popup) = state.popups.text_mut() {
        match key.code {
            KeyCode::Char('g') if popup.lyrics_query.is_some() => {
                return vec![
                    crate::app::action::PlaybackAction::SearchLyrics(
                        popup.lyrics_query.clone().unwrap_or_default(),
                    )
                    .into(),
                ];
            }
            KeyCode::Esc => {
                state.popups.set_text(None);
                state.sources.nav_tasks.remove("lyrics");
            }
            KeyCode::Up => popup.scroll = popup.scroll.saturating_sub(1),
            KeyCode::Down => popup.scroll = popup.scroll.saturating_add(1),
            KeyCode::PageUp => popup.scroll = popup.scroll.saturating_sub(10),
            KeyCode::PageDown => popup.scroll = popup.scroll.saturating_add(10),
            KeyCode::Home => popup.scroll = 0,
            _ => {}
        }
        return vec![];
    }
    // Clear mouse scroll pin on keyboard input, EXCEPT for drill-down/back keys
    // (Enter, Right, Left, Backspace, Esc) which should preserve the pinned
    // scroll position so the viewport doesn't re-center during column changes.
    let preserve_pin = matches!(
        key.code,
        KeyCode::Enter | KeyCode::Right | KeyCode::Left | KeyCode::Backspace | KeyCode::Esc
    ) && !key.modifiers.contains(KeyModifiers::SHIFT)
        && !key.modifiers.contains(KeyModifiers::CONTROL);
    state.terminal_text_selection = None;
    if !preserve_pin {
        state.presentation_anchors = None;
        state.scroll.browse = None;
    }
    state.scroll.category = None;
    state.scroll.browse_click_time = None;
    state.scroll.browse_last_click = None;

    if !state.list_filter.active
        && state.tui_column_header.is_none()
        && state.tui_transport_focus.is_none()
        && state.tui_artist_focus.is_none()
    {
        state.miller_scroll_manual = false;
    }

    // Track modifier bar display.
    // Alt+/ or Ctrl+/ toggles the contextual shortcut bar on/off.
    // Any non-modifier key immediately dismisses it.
    let has_alt = key.modifiers.contains(KeyModifiers::ALT);
    let has_ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let is_help_key = key.code == KeyCode::Char('/');
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
    if state.popups.library_dialog().is_some() && state.popups.confirm_dialog().is_none() {
        return crate::app::sources::dialogs::key(key, state);
    }
    if let Some(mut dialog) = state.popups.take_confirm_dialog() {
        // Pressing any quit shortcut a second time confirms immediately.
        let repeat_quit = matches!(dialog.on_confirm, crate::app::state::ConfirmAction::Quit)
            && matches!(
                (key.modifiers, key.code),
                (KeyModifiers::CONTROL, KeyCode::Char('q'))
                    | (KeyModifiers::SUPER, KeyCode::Char('q'))
                    | (KeyModifiers::SUPER, KeyCode::Char('w'))
                    | (KeyModifiers::ALT, KeyCode::F(4))
            );
        if repeat_quit {
            return vec![SystemAction::Quit.into()];
        }
        match key.code {
            KeyCode::Enter => {
                if dialog.selected_yes {
                    use crate::app::state::ConfirmAction;
                    return match dialog.on_confirm {
                        ConfirmAction::RefreshCache => helpers::refresh_current_view(state),
                        ConfirmAction::ClearSourceCache(choice) => {
                            vec![SettingsAction::ClearSourceCache(choice).into()]
                        }
                        ConfirmAction::ClearLibraryCache => {
                            vec![SettingsAction::ClearLibraryCache.into()]
                        }
                        ConfirmAction::ClearArtworkCache(choice) => {
                            vec![SettingsAction::ClearArtworkCache(choice).into()]
                        }

                        ConfirmAction::RemoveFolder(id) => vec![Action::Source(
                            crate::app::sources::SourceAction::Remove(id),
                        )],

                        ConfirmAction::NavidromeDeletePlaylist(id) => {
                            vec![crate::app::sources::navidrome::NavAction::Command(
                                crate::app::sources::navidrome::commands::Command::DeletePlaylist(
                                    id,
                                ),
                            )
                            .into()]
                        }
                        ConfirmAction::NavidromeReplacePlaylist(id) => {
                            vec![crate::app::sources::navidrome::NavAction::Command(
                                crate::app::sources::navidrome::commands::Command::ReplacePlaylist(
                                    id,
                                ),
                            )
                            .into()]
                        }
                        ConfirmAction::RemoveNavidrome(id) => {
                            vec![crate::app::sources::navidrome::NavAction::Remove(id).into()]
                        }

                        ConfirmAction::Quit => vec![SystemAction::Quit.into()],
                    };
                } else {
                    // No selected — dismiss
                    crate::app::sources::dialogs::restore_parent(state);
                    return vec![];
                }
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                use crate::app::state::ConfirmAction;
                return match dialog.on_confirm {
                    ConfirmAction::RefreshCache => helpers::refresh_current_view(state),
                    ConfirmAction::ClearSourceCache(choice) => {
                        vec![SettingsAction::ClearSourceCache(choice).into()]
                    }
                    ConfirmAction::ClearLibraryCache => {
                        vec![SettingsAction::ClearLibraryCache.into()]
                    }
                    ConfirmAction::ClearArtworkCache(choice) => {
                        vec![SettingsAction::ClearArtworkCache(choice).into()]
                    }

                    ConfirmAction::RemoveFolder(id) => vec![Action::Source(
                        crate::app::sources::SourceAction::Remove(id),
                    )],

                    ConfirmAction::NavidromeDeletePlaylist(id) => {
                        vec![
                            crate::app::sources::navidrome::NavAction::Command(
                                crate::app::sources::navidrome::commands::Command::DeletePlaylist(
                                    id,
                                ),
                            )
                            .into(),
                        ]
                    }
                    ConfirmAction::NavidromeReplacePlaylist(id) => {
                        vec![
                            crate::app::sources::navidrome::NavAction::Command(
                                crate::app::sources::navidrome::commands::Command::ReplacePlaylist(
                                    id,
                                ),
                            )
                            .into(),
                        ]
                    }
                    ConfirmAction::RemoveNavidrome(id) => {
                        vec![crate::app::sources::navidrome::NavAction::Remove(id).into()]
                    }

                    ConfirmAction::Quit => vec![SystemAction::Quit.into()],
                };
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                crate::app::sources::dialogs::restore_parent(state);
                return vec![];
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                dialog.selected_yes = !dialog.selected_yes;
                state.popups.set_confirm_dialog(Some(dialog));
                return vec![];
            }
            _ => {
                state.popups.set_confirm_dialog(Some(dialog));
                return vec![];
            }
        }
    }

    // Handle input dialog if active
    if let Some(dialog) = state.popups.input_dialog_mut() {
        match key.code {
            KeyCode::Esc => {
                state.popups.set_input_dialog(None);
                crate::app::sources::dialogs::restore_parent(state);
            }
            KeyCode::Enter => {
                if matches!(
                    dialog.action_type,
                    crate::app::state::InputDialogAction::CreatePlaylist { .. }
                ) && dialog.input.as_str().trim().is_empty()
                {
                    return vec![];
                }
                // Confirm dialog
                let input = dialog.input.clone();
                let action_type = dialog.action_type.clone();
                state.popups.set_input_dialog(None);
                let input = input.to_string();
                match action_type {
                    crate::app::state::InputDialogAction::SinglesTrackLimit => {
                        return match input.trim().parse::<u32>() {
                            Ok(limit) => vec![SettingsAction::SetSinglesTrackLimit(limit).into()],
                            Err(_) => {
                                state.set_error(
                                    "Enter a whole number for the Singles track limit".into(),
                                );
                                vec![]
                            }
                        };
                    }
                    crate::app::state::InputDialogAction::AudioMuseSearch(feature) => {
                        return vec![
                            crate::app::sources::audiomuse::Command::Search {
                                feature,
                                query: input,
                            }
                            .into(),
                        ];
                    }
                    crate::app::state::InputDialogAction::CreatePlaylist {
                        generation,
                        addition,
                    } => {
                        return vec![
                            crate::app::sources::navidrome::NavAction::Command(
                                crate::app::sources::navidrome::commands::Command::CreatePlaylist {
                                    generation,
                                    addition,
                                    name: input.trim().to_owned(),
                                },
                            )
                            .into(),
                        ];
                    }
                    crate::app::state::InputDialogAction::NavidromePlaylistName { id } => {
                        return vec![crate::app::sources::navidrome::NavAction::Command(
                            crate::app::sources::navidrome::commands::Command::SetPlaylistName {
                                id,
                                name: input,
                            },
                        )
                        .into()];
                    }
                    crate::app::state::InputDialogAction::NavidromeName(id) => {
                        return vec![
                            crate::app::sources::navidrome::NavAction::Rename { id, name: input }
                                .into(),
                        ];
                    }
                    crate::app::state::InputDialogAction::FolderLocation => {
                        return vec![Action::Source(
                            crate::app::sources::SourceAction::AddLocation(input),
                        )];
                    }
                    crate::app::state::InputDialogAction::FolderName(id) => {
                        return vec![Action::Source(crate::app::sources::SourceAction::Rename {
                            id,
                            name: input,
                        })];
                    }
                    crate::app::state::InputDialogAction::SaveDescriptionPlaylist {
                        library_key,
                        tracks,
                    } => {
                        return vec![
                            crate::app::sources::audiomuse::Command::SavePlaylist {
                                library_key,
                                tracks,
                                name: input,
                            }
                            .into(),
                        ];
                    }
                    crate::app::state::InputDialogAction::ImportPlaylist { generation } => {
                        return vec![
                            QueueAction::ImportPlaylist {
                                generation,
                                path: input.into(),
                            }
                            .into(),
                        ];
                    }
                    crate::app::state::InputDialogAction::ExportPlaylist {
                        generation,
                        contents,
                        ..
                    } => {
                        return vec![
                            QueueAction::ExportPlaylist {
                                generation,
                                contents,
                                path: std::path::PathBuf::from(input),
                            }
                            .into(),
                        ];
                    }
                    crate::app::state::InputDialogAction::SavePlaylist => {
                        return vec![QueueAction::SaveQueueAsPlaylist(input).into()];
                    }
                }
            }
            KeyCode::Backspace => {
                dialog.input.pop();
            }
            KeyCode::Char(c)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                if dialog.input.len() < 2048 {
                    dialog.input.push(c);
                }
            }
            _ => {}
        }
        return vec![];
    }

    // Sort popup handling (takes priority over view-specific handling)
    if state.popups.sort().is_some() {
        return sort_popup::handle_sort_popup_keys(key, state);
    }

    let inline_radio = crate::app::radio_navigation::inline_input(state);
    if inline_radio && matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
        crate::app::tui_navigation::traverse(state, key.code == KeyCode::BackTab);
        return vec![];
    }
    // Adventure launcher popup handling (takes priority over view-specific handling)
    if state.popups.adventure_launcher().is_some() && (!inline_radio || state.track_pane_focused) {
        return adventure_launcher::handle_adventure_launcher_keys(key, state);
    }

    // Artist radio picker popup handling
    if state.popups.artist_radio_picker().is_some() && (!inline_radio || state.track_pane_focused) {
        return artist_radio_picker::handle_artist_radio_picker_keys(key, state);
    }

    if !state.desktop_frontend
        && matches!(
            state.popups.modal(),
            crate::app::state::Modal::Closed | crate::app::state::Modal::Search
        )
        && (state.tui_artist_focus.is_some()
            || (key.modifiers.contains(KeyModifiers::ALT) && key.code == KeyCode::Char('a')))
    {
        let mut artists = state
            .hit_regions
            .artist_links
            .iter()
            .filter(|link| link.generation == state.library_generation)
            .collect::<Vec<_>>();
        artists.sort_by_key(|link| (link.area.y, link.area.x));
        artists.dedup_by(|left, right| left.area == right.area && left.key == right.key);
        if key.modifiers.contains(KeyModifiers::ALT) && key.code == KeyCode::Char('a') {
            state.tui_artist_focus = artists.first().map(|link| (*link).clone());
            return vec![];
        }
        if let Some(focused) = state.tui_artist_focus.as_ref() {
            if let Some(index) = artists
                .iter()
                .position(|link| link.key == focused.key && link.area == focused.area)
            {
                match key.code {
                    KeyCode::Tab | KeyCode::BackTab if !state.popups.search_active() => {
                        crate::app::tui_navigation::traverse(state, key.code == KeyCode::BackTab);
                        return vec![];
                    }
                    KeyCode::Left | KeyCode::Up => {
                        state.tui_artist_focus =
                            Some((*artists[(index + artists.len() - 1) % artists.len()]).clone());
                        return vec![];
                    }
                    KeyCode::Right | KeyCode::Down => {
                        state.tui_artist_focus =
                            Some((*artists[(index + 1) % artists.len()]).clone());
                        return vec![];
                    }
                    KeyCode::Enter => {
                        let action = artists[index].action(state.library_generation);
                        state.tui_artist_focus = None;
                        return action.into_iter().collect();
                    }
                    KeyCode::Esc => {
                        state.tui_artist_focus = None;
                        return vec![];
                    }
                    _ => {}
                }
            }
            state.tui_artist_focus = None;
        }
    }

    // Search popup handling (takes priority over view-specific handling)
    if state.popups.search_active() {
        return search::handle_search_keys(key, state);
    }

    // Artist bio popup handling
    if state.popups.artist_bio().is_some() {
        return handle_artist_bio_popup_keys(key, state);
    }

    if state.popups.library_picker_active() {
        if key.code == KeyCode::F(2) {
            return vec![SearchAction::ManageLibraries.into()];
        }
        if key.code == KeyCode::F(3) {
            return vec![SearchAction::CloseLibraryPicker.into()];
        }
        return handle_library_picker_keys(key, state);
    }

    if state.view == View::Search {
        return search::handle_search_keys(key, state);
    }
    if state.view == View::Browse
        && !state.category_column_focused
        && state.tui_transport_focus.is_none()
        && let Some((title, _)) = state.tui_sidebar_group.clone()
    {
        if title == "Playlists" {
            if let Some(query) = &mut state.playlist_group_filter {
                match key.code {
                    KeyCode::Char(ch)
                        if !key.modifiers.intersects(
                            KeyModifiers::CONTROL | KeyModifiers::SUPER | KeyModifiers::ALT,
                        ) =>
                    {
                        query.push(ch);
                        state.tui_sidebar_group = Some((title, 0));
                        return vec![];
                    }
                    KeyCode::Backspace => {
                        query.pop();
                        state.tui_sidebar_group = Some((title, 0));
                        return vec![];
                    }
                    KeyCode::Esc => {
                        state.playlist_group_filter = None;
                        state.tui_sidebar_group = Some((title, 0));
                        return vec![];
                    }
                    _ => {}
                }
            } else if key.code == KeyCode::Char('/') {
                state.playlist_group_filter = Some(String::new());
                return vec![];
            }
        }
    }
    // ":" opens the command palette from any normal context. Allow
    // SHIFT in the modifier set because `:` is typed as Shift+; on
    // most keyboards — iced reports that as Char(':') + SHIFT, so
    // an `is_empty()` gate would never fire there. The TUI's
    // crossterm backend strips SHIFT for printable characters, so
    // both paths land here. Must run before any other
    // character-key handlers consume the keystroke.
    if matches!(key.code, KeyCode::Char(':'))
        && !key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
        && !key.modifiers.contains(KeyModifiers::SUPER)
        && !state.list_filter.active
    {
        state.palette.player_context = false;
        return crate::app::command_palette::open(state);
    }
    // "/" activates the inline list filter from anywhere — the
    // filter input is rendered in the active column and grabs keyboard focus until Esc /
    // Enter closes it.
    if matches!(key.code, KeyCode::Char('/'))
        && !key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
        && !state.list_filter.active
        && !crate::app::sources::manager::settings_active(state)
    {
        return vec![SearchAction::ActivateListFilter.into()];
    }

    if !state.desktop_frontend
        && key.modifiers.contains(KeyModifiers::ALT)
        && key.code == KeyCode::Char('t')
        && !state.list_filter.active
    {
        state.tui_transport_focus = if state.tui_transport_focus.is_some() {
            None
        } else {
            Some(crate::app::transport_controls::TransportControl::Play)
        };
        return vec![];
    }
    if !crate::app::tui_navigation::filter_focused(state)
        && !state.tui_description_focused
        && !key
            .modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL | KeyModifiers::SUPER)
    {
        match key.code {
            KeyCode::Char('[') => {
                state.tui_sidebar_visible = !state.tui_sidebar_visible;
                if !state.tui_sidebar_visible && !cfg!(target_os = "macos") {
                    state.category_column_focused = false;
                }
                return vec![];
            }
            KeyCode::Char(']' | '|') => return vec![NavigationAction::ToggleQueuePanel.into()],
            KeyCode::Char('^') => {
                return crate::app::transport_controls::activate(
                    state,
                    crate::app::transport_controls::TransportControl::Player,
                );
            }
            KeyCode::Char('R') => {
                return crate::app::command_palette::run(
                    crate::app::state::PaletteCommandKind::GoToRandomAlbum,
                    state,
                );
            }
            _ => {}
        }
    }
    if !state.desktop_frontend
        && matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
        && !key
            .modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL | KeyModifiers::SUPER)
        && matches!(state.view, View::Browse | View::NowPlaying | View::Queue)
    {
        crate::app::tui_navigation::traverse(state, key.code == KeyCode::BackTab);
        return vec![];
    }
    if let Some(control) = state.tui_rail_focus {
        use crate::app::tui_navigation::RailControl;
        let controls = RailControl::visible(state);
        if let Some(index) = controls.iter().position(|item| *item == control) {
            let len = controls.len();
            match key.code {
                KeyCode::Left | KeyCode::Up => {
                    state.tui_rail_focus = Some(controls[(index + len - 1) % len])
                }
                KeyCode::Right | KeyCode::Down => {
                    state.tui_rail_focus = Some(controls[(index + 1) % len])
                }
                KeyCode::Enter => return control.activate(state),
                KeyCode::Esc => state.tui_rail_focus = None,
                _ => {}
            }
            return vec![];
        }
        state.tui_rail_focus = None;
    }
    if let Some(index) = state.tui_footer_focus {
        match key.code {
            KeyCode::Left | KeyCode::Up => state.tui_footer_focus = Some(index.saturating_sub(1)),
            KeyCode::Right | KeyCode::Down => {
                state.tui_footer_focus =
                    Some((index + 1).min(crate::app::tui_navigation::FOOTER.len() - 1))
            }
            KeyCode::Enter => return crate::app::tui_navigation::footer(state, index),
            KeyCode::Esc => state.tui_footer_focus = None,
            _ => {}
        }
        return vec![];
    }
    if state.tui_description_submit_focused && crate::app::tui_navigation::describing(state) {
        match key.code {
            KeyCode::Enter if !state.tui_description_query.trim().is_empty() => {
                return vec![
                    crate::app::sources::audiomuse::Command::Search {
                        feature: crate::audiomuse::Feature::Describe,
                        query: state.tui_description_query.trim().to_owned(),
                    }
                    .into(),
                ];
            }
            KeyCode::Left | KeyCode::Esc => {
                state.tui_description_submit_focused = false;
                state.tui_description_focused = true;
            }
            _ => {}
        }
        return vec![];
    }
    if state.tui_description_focused && crate::app::tui_navigation::describing(state) {
        match key.code {
            KeyCode::Char(c)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                state.tui_description_query.push(c)
            }
            KeyCode::Backspace => {
                state.tui_description_query.pop();
            }
            KeyCode::Enter if !state.tui_description_query.trim().is_empty() => {
                state.tui_description_focused = false;
                return vec![
                    crate::app::sources::audiomuse::Command::Search {
                        feature: crate::audiomuse::Feature::Describe,
                        query: state.tui_description_query.trim().to_owned(),
                    }
                    .into(),
                ];
            }
            KeyCode::Down | KeyCode::Esc => {
                state.tui_description_focused = false;
                state.category_column_focused = false;
            }
            _ => {}
        }
        return vec![];
    }
    if crate::app::radio_navigation::visible(state)
        && !state.category_column_focused
        && state.tui_transport_focus.is_none()
        && state.tui_footer_focus.is_none()
        && let Some(actions) = crate::app::radio_navigation::key(state, key.code)
    {
        return actions;
    }
    if state.view == View::Browse
        && !state.category_column_focused
        && state.tui_transport_focus.is_none()
        && let Some((title, selected)) = state.tui_sidebar_group.clone()
    {
        let rows = crate::app::sidebar::filtered_group_rows(
            state,
            &title,
            state
                .playlist_group_filter
                .as_deref()
                .filter(|_| title == "Playlists")
                .unwrap_or_default(),
        );
        match key.code {
            KeyCode::Up => state.tui_sidebar_group = Some((title, selected.saturating_sub(1))),
            KeyCode::Down => {
                state.tui_sidebar_group =
                    Some((title, (selected + 1).min(rows.len().saturating_sub(1))))
            }
            KeyCode::Enter => {
                return crate::app::sidebar::open_group_child(state, &title, selected);
            }
            KeyCode::Left | KeyCode::Esc | KeyCode::Backspace => {
                state.tui_sidebar_group = None;
                state.category_column_focused =
                    state.tui_sidebar_visible || cfg!(target_os = "macos");
            }
            _ => {}
        }
        return vec![];
    }
    if let Some(control) = state.tui_transport_focus {
        use crate::app::transport_controls::{self, TransportControl};
        let controls = transport_controls::visible(state);
        let index = controls
            .iter()
            .position(|item| *item == control)
            .unwrap_or(0);
        match key.code {
            KeyCode::Esc => {
                state.tui_transport_focus = None;
                return vec![];
            }
            KeyCode::Left | KeyCode::Right
                if control == TransportControl::Seek && control.enabled(state) =>
            {
                return vec![
                    PlaybackAction::SeekRelative(if key.code == KeyCode::Left {
                        -5000
                    } else {
                        5000
                    })
                    .into(),
                ];
            }
            KeyCode::Left | KeyCode::Right if control == TransportControl::Volume => {
                return vec![
                    if key.code == KeyCode::Left {
                        PlaybackAction::VolumeDown
                    } else {
                        PlaybackAction::VolumeUp
                    }
                    .into(),
                ];
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
                let backwards = matches!(key.code, KeyCode::Left | KeyCode::Up);
                state.tui_transport_focus = Some(
                    controls[if backwards {
                        (index + controls.len() - 1) % controls.len()
                    } else {
                        (index + 1) % controls.len()
                    }],
                );
                return vec![];
            }
            KeyCode::Enter => return transport_controls::activate(state, control),
            KeyCode::Char(' ') => return vec![PlaybackAction::TogglePlayPause.into()],
            _ => {}
        }
    }

    // Text capture precedes single-character global commands, including layout keys.
    if state.list_filter.active
        && (state.desktop_frontend || crate::app::tui_navigation::filter_focused(state))
        && let Some(actions) = handle_filter_input(key)
    {
        return actions;
    }

    if key.code == KeyCode::Backspace
        && key.modifiers.is_empty()
        && (state.tui_player_raised()
            || (state.view == View::Browse && state.jump_history.head().is_some()))
    {
        return vec![NavigationAction::Back.into()];
    }

    // Modal owners have returned. An inline filter can still reserve function keys.
    let in_text_capture = state.list_filter.active
        && (state.desktop_frontend || crate::app::tui_navigation::filter_focused(state));
    if !in_text_capture
        && state.view == View::Browse
        && key.modifiers.is_empty()
        && matches!(key.code, KeyCode::Char('x') | KeyCode::Delete)
        && let Some(id) = state
            .browse_nav()
            .and_then(|nav| nav.focused())
            .and_then(|col| col.selected_item())
            .and_then(|item| item.pending_download())
    {
        return vec![crate::app::downloads::Command::Cancel(id.to_owned()).into()];
    }
    if state.view == View::Settings
        && !(matches!(key.code, KeyCode::Char('f' | 'p'))
            && key.modifiers.contains(KeyModifiers::CONTROL))
        && !matches!(
            key.code,
            KeyCode::F(1) | KeyCode::F(2) | KeyCode::F(3) | KeyCode::Char(',')
        )
    {
        return settings::handle_settings_keys(key, state, config);
    }
    match (key.modifiers, key.code) {
        (KeyModifiers::ALT, KeyCode::Left) => return vec![NavigationAction::Back.into()],
        (KeyModifiers::CONTROL, KeyCode::Char('z')) => return vec![NavigationAction::Undo.into()],
        (KeyModifiers::CONTROL | KeyModifiers::SUPER, KeyCode::Char('a')) => {
            match state.view {
                View::Queue | View::NowPlaying => {
                    state.queue.selected = (0..state.queue.tracks.len()).collect()
                }
                View::Browse => {
                    let index = state
                        .browse_nav()
                        .map(|nav| nav.focused_column)
                        .or_else(|| state.folder_state.as_ref().map(|nav| nav.focused_column));
                    if let Some(index) = index {
                        crate::app::column_controls::select_all(state, index);
                    }
                }
                _ => {}
            }
            return vec![];
        }

        // Global navigation shortcuts
        (KeyModifiers::CONTROL, KeyCode::Char('f')) => {
            // Ctrl+F = Search/Filter popup (floating dialog)
            if state.popups.search_active() {
                return vec![SearchAction::CloseSearchPopup.into()];
            } else {
                return vec![SearchAction::OpenSearchPopup.into()];
            }
        }
        (KeyModifiers::CONTROL, KeyCode::Char('g')) => {
            // Ctrl+G = Album Genres section. The other tag sections
            // (Artist Genres, Moods, Styles, Decades, Years, …) are
            // accessed by navigating the leftmost sections column.
            if state.view == View::Browse && state.browse_category == BrowseCategory::AlbumGenres {
                return vec![];
            }
            state.set_browse_category(BrowseCategory::AlbumGenres, false);
            reset_right_panel(state);
            return vec![
                BrowseAction::RefreshTagView.into(),
                NavigationAction::SetView(View::Browse).into(),
                SystemAction::CheckStaleness(crate::app::state::RefreshCategory::AlbumGenres)
                    .into(),
            ];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
            // Ctrl+N = Now Playing (visualizer view)
            return vec![
                NavigationAction::SetView(View::NowPlaying).into(),
                SystemAction::LoadWaveform.into(),
            ];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
            // Ctrl+L = Library category (no cycling — server doesn't distinguish album artists)
            if state.view == View::Browse && state.browse_category == BrowseCategory::Library {
                return vec![];
            }
            // Not in library view - switch to it and reset right panel
            state.set_browse_category(BrowseCategory::Library, false);
            reset_right_panel(state);
            let tier1 = crate::app::state::RefreshCategory::Artists;
            if state.library.artists.is_empty() {
                return vec![
                    DataAction::LoadArtists.into(),
                    NavigationAction::SetView(View::Browse).into(),
                    SystemAction::CheckStaleness(tier1).into(),
                ];
            }
            return vec![
                NavigationAction::SetView(View::Browse).into(),
                SystemAction::CheckStaleness(tier1).into(),
            ];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
            // Ctrl+P / Cmd+P = open the command palette. Playlists no
            // longer has a dedicated shortcut — they're listed as
            // individual rows in the leftmost browse column, so the
            // category itself doesn't need a hot-key.
            if !state.palette.open {
                return crate::app::command_palette::open(state);
            }
            return vec![];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('o')) => {
            // Ctrl+O = Folders category
            state.set_browse_category(BrowseCategory::Folders, false);
            reset_right_panel(state);
            let staleness =
                SystemAction::CheckStaleness(crate::app::state::RefreshCategory::Folders).into();
            if state.folder_state.is_none() {
                return vec![
                    FolderAction::LoadFolderRoot.into(),
                    NavigationAction::SetView(View::Browse).into(),
                    staleness,
                ];
            }
            return vec![NavigationAction::SetView(View::Browse).into(), staleness];
        }

        // Global function keys — work from every screen as toggles.
        //
        // F1 / Ctrl+H open Help; pressing the same key again from Help
        // returns to Browse.
        // F2 / `,`  open Settings; pressing again from Settings
        // returns to Browse (mirrors Esc).
        //
        // The `in_text_capture` precondition lets these globals defer
        // to popups / inline filters / credential editors that need
        // the literal `,` keystroke. (See the let-binding just
        // above this match.)
        (_, KeyCode::F(1))
        | (KeyModifiers::CONTROL, KeyCode::Char('h'))
        | (KeyModifiers::CONTROL, KeyCode::Char('H'))
            if !in_text_capture =>
        {
            return if state.view == View::Help {
                state.help_scroll = 0;
                vec![NavigationAction::SetView(View::Browse).into()]
            } else {
                vec![NavigationAction::SetView(View::Help).into()]
            };
        }
        (_, KeyCode::F(2)) if !in_text_capture => {
            return if state.view == View::Settings {
                vec![NavigationAction::SetView(View::Browse).into()]
            } else {
                vec![SettingsAction::OpenSettings.into()]
            };
        }
        // `,` mirrors the macOS Cmd+, "Preferences" convention.
        (m, KeyCode::Char(',')) if m.is_empty() && !in_text_capture => {
            return if state.view == View::Settings {
                vec![NavigationAction::SetView(View::Browse).into()]
            } else {
                vec![SettingsAction::OpenSettings.into()]
            };
        }
        (_, KeyCode::F(3)) => {
            return vec![SearchAction::OpenLibraryPicker.into()];
        }
        (_, KeyCode::F(4)) => {
            if state.popups.artist_bio().is_some() {
                return handle_artist_bio_popup_keys(key, state);
            }
            return crate::app::command_palette::run(
                crate::app::state::PaletteCommandKind::ShowArtistBio,
                state,
            );
        }
        (_, KeyCode::F(5)) => {
            // F5 = Refresh current view
            return helpers::refresh_current_view(state);
        }
        // Space toggles a focused sidebar-visibility checkbox in Settings;
        // elsewhere it toggles playback. Track selection uses v / V.
        (KeyModifiers::NONE, KeyCode::Char(' '))
            if state.view == View::Browse
                && !state.category_column_focused
                && !state.list_filter.active
                && state
                    .browse_nav()
                    .and_then(|n| n.focused())
                    .is_some_and(|c| {
                        c.playlist_editing
                            && crate::app::sources::navidrome::playlist_edit::editable(state, c)
                                .is_some()
                    }) =>
        {
            let nav = state.browse_nav_mut().unwrap();
            let index = nav.focused_column;
            let row = nav.columns[index].selected_index;
            if crate::app::sources::navidrome::playlist_edit::mark_group_row(
                &mut nav.columns,
                index,
                row,
                None,
            ) {
                return vec![];
            }
            let col = &mut nav.columns[index];
            if !col.items.is_empty() && !col.selected_set.remove(&col.selected_index) {
                col.selected_set.insert(col.selected_index);
            }
            return vec![];
        }
        (_, KeyCode::Char(' ')) if !state.list_filter.active => {
            use crate::app::state::{SettingsFocus, SettingsSection};
            if matches!(
                (
                    state.view,
                    state.settings_state.section,
                    state.settings_state.focus
                ),
                (
                    View::Settings,
                    SettingsSection::Textamp,
                    SettingsFocus::Content
                )
            ) {
                return vec![SettingsAction::SettingsSelect.into()];
            }
            return vec![PlaybackAction::TogglePlayPause.into()];
        }
        // `v` enters select mode on the focused track list, clearing
        // any prior selection. `V` (Shift+v) enters / extends select
        // mode WITHOUT clearing — same "additive" behaviour the old
        // Ctrl+Space provided. Vim convention matches.
        (mods, KeyCode::Char('v')) | (mods, KeyCode::Char('V'))
            if !mods.contains(KeyModifiers::CONTROL)
                && !mods.contains(KeyModifiers::ALT)
                && !state.list_filter.active =>
        {
            let add_to_selection = mods.contains(KeyModifiers::SHIFT);
            if toggle_select_mode_on_focused_list(state, add_to_selection) {
                return vec![];
            }
            // Not on a track list — fall through so `v` still works as
            // a letter-jump in artist / genre / playlist columns.
        }
        // < and > for prev/next track (crossterm reports these with NONE modifiers, not SHIFT)
        (_, KeyCode::Char('<')) if !state.list_filter.active => {
            return vec![PlaybackAction::Previous.into()];
        }
        (_, KeyCode::Char('>')) if !state.list_filter.active => {
            return vec![PlaybackAction::Next.into()];
        }
        // Ctrl+Shift+Up/Down: multi-select in Queue view, volume elsewhere
        (mods, KeyCode::Up)
            if mods == KeyModifiers::CONTROL | KeyModifiers::SHIFT && state.view == View::Queue =>
        {
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
        (mods, KeyCode::Down)
            if mods == KeyModifiers::CONTROL | KeyModifiers::SHIFT && state.view == View::Queue =>
        {
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
            state.volume_slider_until =
                Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
            return vec![PlaybackAction::VolumeUp.into()];
        }
        (mods, KeyCode::Down) if mods == KeyModifiers::CONTROL | KeyModifiers::SHIFT => {
            state.volume_slider_until =
                Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
            return vec![PlaybackAction::VolumeDown.into()];
        }
        // Shift+Left/Right for seeking (10 second skip)
        (KeyModifiers::SHIFT, KeyCode::Left) => {
            return vec![PlaybackAction::SeekRelative(-10000).into()];
        }
        (KeyModifiers::SHIFT, KeyCode::Right) => {
            return vec![PlaybackAction::SeekRelative(10000).into()];
        }
        // Action commands (Ctrl+key) — gated by availability check
        // Ctrl+E: Add to END of queue (skip if in search popup - handled there)
        (KeyModifiers::CONTROL, KeyCode::Char('e'))
            if alt_commands::is_action_command_available(state, 'e') =>
        {
            return vec![QueueAction::EnqueueSelection.into()];
        }
        // Ctrl+Shift+E: Insert NEXT in queue after current track (skip if in search popup - handled there)
        (mods, KeyCode::Char('e')) | (mods, KeyCode::Char('E'))
            if mods == KeyModifiers::CONTROL | KeyModifiers::SHIFT
                && alt_commands::is_action_command_available(state, 'e') =>
        {
            return vec![QueueAction::EnqueueSelectionNext.into()];
        }
        (mods, KeyCode::Char('r' | 'R'))
            if mods == KeyModifiers::CONTROL | KeyModifiers::SHIFT
                || mods == KeyModifiers::ALT | KeyModifiers::SHIFT
                || (mods == KeyModifiers::ALT && key.code == KeyCode::Char('R')) =>
        {
            return crate::app::command_palette::run(
                crate::app::state::PaletteCommandKind::GoToRandomAlbum,
                state,
            );
        }
        (KeyModifiers::CONTROL, KeyCode::Char('j'))
            if alt_commands::is_action_command_available(state, 'j') =>
        {
            return navigate_to_album(state);
        }
        // Ctrl+S = Save queue as playlist (standard Save shortcut).
        (KeyModifiers::CONTROL, KeyCode::Char('s'))
            if alt_commands::is_action_command_available(state, 's') =>
        {
            return vec![QueueAction::PromptSavePlaylist.into()];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('v')) | (KeyModifiers::NONE, KeyCode::F(6)) => {
            return crate::app::view_options::open_for_input(state);
        }
        // Ctrl+W = Close current Miller column. Drops the focused
        // column + any cols to its right and moves focus one left.
        // At the root (or the hidden Playlists col-offset prefix),
        // falls back to focusing the cat col so the user can pivot
        // to another category.
        (KeyModifiers::CONTROL, KeyCode::Char('w'))
            if state.view == View::Browse || state.tui_player_raised() =>
        {
            close_focused_browse_column(state);
            crate::app::tui_navigation::focus_root_after_close(state);
            return vec![];
        }
        (KeyModifiers::CONTROL, KeyCode::Char('x'))
            if alt_commands::is_action_command_available(state, 'x') =>
        {
            return vec![QueueAction::ClearQueue.into()];
        }
        // Alt shortcuts (station/global commands)
        (KeyModifiers::ALT, KeyCode::Char('f')) => {
            // Alt+F = Activate inline filter (Browse view only)
            if state.view == View::Browse && !state.list_filter.active {
                return vec![SearchAction::ActivateListFilter.into()];
            }
            return vec![];
        }
        (KeyModifiers::ALT, KeyCode::Char('r')) => {
            return random_album_radio(state);
        }

        _ => {}
    }

    // View-specific handling
    let actions = match state.view {
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
            if let Some(nav) = state.browse_nav_mut()
                && let Some(col) = nav.focused_mut()
            {
                col.selected_set.insert(col.selected_index);
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
                if let Some(nav) = state.browse_nav_mut()
                    && let Some(col) = nav.focused_mut()
                {
                    col.selected_set.insert(col.selected_index);
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
                if let Some(nav) = state.browse_nav_mut()
                    && let Some(col) = nav.focused_mut()
                {
                    col.selected_set.clear();
                    col.selected_set.insert(col.selected_index);
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
fn handle_filter_input(key: event::KeyEvent) -> Option<Vec<Action>> {
    use crate::app::action::SearchAction;
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
    {
        return None;
    }
    match key.code {
        KeyCode::Esc => Some(vec![SearchAction::DeactivateListFilter.into()]),
        KeyCode::Backspace => Some(vec![SearchAction::DeleteListFilterChar.into()]),
        KeyCode::Char(c) => Some(vec![SearchAction::AppendListFilterChar(c).into()]),
        // Up/Down/Left/Right/Tab/Enter fall through to the view's
        // handler so the user can keep typing into the filter while
        // navigating the filtered list AND activate (drill / play)
        // the highlighted row with Enter.
        _ => None,
    }
}

/// Handle keys when library picker popup is active.
pub(super) fn handle_library_picker_keys(
    key: event::KeyEvent,
    state: &mut AppState,
) -> Vec<Action> {
    use crate::app::sources::{LibraryChoice, choices};
    // These are plain-letter manager commands, not the global Ctrl+P/A/etc.
    // Quit is handled before entering this modal handler.
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
    {
        return vec![];
    }
    let entries = choices(state);
    let count = entries.len();
    state.sources.picker_scroll_pin = None;
    if state.popups.library_picker_active()
        && !matches!(
            key.code,
            KeyCode::Esc
                | KeyCode::Enter
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::F(5)
        )
    {
        return vec![];
    }
    match key.code {
        KeyCode::Esc => return vec![SearchAction::CloseLibraryPicker.into()],
        KeyCode::Up => {
            state.popups.library_picker_index = state.popups.library_picker_index.saturating_sub(1)
        }
        KeyCode::Down => {
            state.popups.library_picker_index =
                (state.popups.library_picker_index + 1).min(count.saturating_sub(1))
        }
        KeyCode::Home => state.popups.library_picker_index = 0,
        KeyCode::End => state.popups.library_picker_index = count.saturating_sub(1),
        KeyCode::PageUp => {
            state.popups.library_picker_index = state.popups.library_picker_index.saturating_sub(10)
        }
        KeyCode::PageDown => {
            state.popups.library_picker_index =
                (state.popups.library_picker_index + 10).min(count.saturating_sub(1))
        }
        KeyCode::Enter => {
            return entries
                .get(state.popups.library_picker_index)
                .map(|entry| crate::app::sources::options::open(entry.clone(), state))
                .unwrap_or_default();
        }
        KeyCode::F(5) if !state.popups.library_picker_active() => {
            if let Some(choice) = entries.get(state.popups.library_picker_index)
                && choice.cache_store().is_ok()
            {
                return vec![SettingsAction::RescanSourceCache(choice.clone()).into()];
            }
        }
        KeyCode::Char('a') => return vec![LibraryChoice::Add.action()],
        KeyCode::Char('n') => return vec![LibraryChoice::AddNavidrome.action()],
        KeyCode::Char('w') => return vec![LibraryChoice::AddWebdav.action()],
        _ => {
            if let Some(choice) = entries.get(state.popups.library_picker_index) {
                return crate::app::sources::options::shortcut(choice, key.code, state);
            }
        }
    }
    vec![]
}

/// Handle keys when artist bio popup is active.
fn handle_artist_bio_popup_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    let max_scroll = state
        .hit_regions
        .biography_text
        .as_ref()
        .map_or(0, |r| r.max_scroll());
    let Some(popup) = state.popups.artist_bio_mut() else {
        return vec![];
    };
    match key.code {
        KeyCode::Char('g') | KeyCode::Char('G') => {
            return vec![SearchAction::SearchBiographyOnWeb.into()];
        }
        KeyCode::Enter if popup.google_focused => {
            return vec![SearchAction::SearchBiographyOnWeb.into()];
        }
        KeyCode::Tab | KeyCode::BackTab => popup.google_focused = !popup.google_focused,
        KeyCode::Left | KeyCode::Right => {
            if popup.google_focused {
                popup.google_focused = false;
            } else {
                let count = popup.document.images.len();
                if count > 0 {
                    popup.image_index = if key.code == KeyCode::Right {
                        (popup.image_index + 1) % count
                    } else {
                        (popup.image_index + count - 1) % count
                    };
                    popup.scroll = 0;
                }
            }
        }
        KeyCode::Char('b') | KeyCode::Char('B') => {
            return vec![SearchAction::OpenBiographySource.into()];
        }
        KeyCode::Esc | KeyCode::F(4) => {
            state.popups.set_artist_bio(None);
            // Bio popup overlays the Now Playing artwork when shown
            // from that view; dropping the protocol forces a fresh
            // image placement on the next render so the terminal
            // re-displays the cover that the popup hid.
        }
        KeyCode::Up => {
            if popup.google_focused {
                popup.google_focused = false;
            } else {
                popup.scroll = popup.scroll.min(max_scroll).saturating_sub(1);
            }
        }
        KeyCode::Down if !popup.google_focused => {
            if popup.scroll >= max_scroll {
                popup.google_focused = true;
            } else {
                popup.scroll += 1;
            }
        }
        KeyCode::PageUp => {
            popup.google_focused = false;
            popup.scroll = popup.scroll.min(max_scroll).saturating_sub(10);
        }
        KeyCode::PageDown => {
            popup.google_focused = false;
            popup.scroll = popup.scroll.saturating_add(10).min(max_scroll);
        }
        KeyCode::Home => {
            popup.google_focused = false;
            popup.scroll = 0;
        }
        KeyCode::End => popup.google_focused = true,
        _ => {} // Absorb all other keys
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
    state.library.tag_albums.clear();
    state.library.tag_albums_index = 0;
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
            let ak = match &track.parent_rating_key {
                Some(k) => k.clone(),
                None => return vec![],
            };
            let rk = match &track.grandparent_rating_key {
                Some(k) => k.clone(),
                None => return vec![],
            };
            (
                ak,
                rk,
                track.album_name().to_string(),
                track.artist_name().to_string(),
            )
        } else {
            return vec![];
        }
    } else if let Some(track) = get_selected_track(state) {
        // Highlighted track takes first priority outside Library
        let ak = match &track.parent_rating_key {
            Some(k) => k.clone(),
            None => return vec![],
        };
        let rk = match &track.grandparent_rating_key {
            Some(k) => k.clone(),
            None => return vec![],
        };
        (
            ak,
            rk,
            track.album_name().to_string(),
            track.artist_name().to_string(),
        )
    } else if let Some(ctx) = get_miller_album_context(state) {
        ctx
    } else if let Some(ctx) = get_folder_album_context(state) {
        ctx
    } else if let Some(track) = state.current_track().cloned() {
        // Fallback: now-playing track
        let ak = match &track.parent_rating_key {
            Some(k) => k.clone(),
            None => return vec![],
        };
        let rk = match &track.grandparent_rating_key {
            Some(k) => k.clone(),
            None => return vec![],
        };
        (
            ak,
            rk,
            track.album_name().to_string(),
            track.artist_name().to_string(),
        )
    } else {
        return vec![];
    };

    // Navigate to the artist in Miller columns, with pending album auto-select
    state.search.pending_album_key = Some(album_key);
    state.library.selected_album_title = album_title;
    state.library.selected_artist_name = artist_name;
    state.set_view(View::Browse);
    state.set_browse_category(BrowseCategory::Library, false);

    // Select the artist in the Miller column
    if let Some(idx) = state.artist_nav.columns.first().and_then(|col| {
        col.items
            .iter()
            .position(|item| matches!(item, BrowseItem::Artist { key, .. } if *key == artist_key))
    }) {
        if let Some(col) = state.artist_nav.columns.first_mut() {
            col.selected_index = idx;
        }
        state.artist_nav.focused_column = 0;
        state.artist_nav.truncate_right();
    }
    // Keep the artist-list cursor aligned with the Miller selection.
    if let Some(idx) = state
        .library
        .artists
        .iter()
        .position(|a| a.rating_key == artist_key)
    {
        state.list_state.artists_index = idx;
    }

    vec![
        MillerAction::LoadArtistAlbumsForMiller {
            artist_key,
            replace_child: false,
        }
        .into(),
    ]
}

/// Get album context from the selected folder track: (album_key, artist_key, album_title, artist_name).
fn get_folder_album_context(state: &AppState) -> Option<(String, String, String, String)> {
    if state.view != View::Browse || state.browse_category != BrowseCategory::Folders {
        return None;
    }
    let item = state.folder_state.as_ref()?.selected_item()?;
    if !item.is_track() {
        return None;
    }
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
    let selected_item = nav
        .columns
        .get(focused)
        .and_then(|c| c.items.get(c.selected_index))?;

    match selected_item {
        BrowseItem::Track { .. } => {
            // Track selected: album is in parent column, artist in grandparent
            let album = (focused > 0)
                .then(|| nav.columns.get(focused - 1))
                .flatten()
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
        BrowseItem::Album {
            key, title, artist, ..
        } => {
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
        if let Some(BrowseItem::Artist { key, .. }) = col.items.get(col.selected_index) {
            return Some(key.clone());
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
                        let (tab, local) =
                            crate::app::handlers::dispatch_search::resolve_global_index(
                                results, idx,
                            );
                        if tab == crate::app::state::SearchTab::Tracks {
                            return results.tracks.get(local).cloned();
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
                PlaybackMode::Queue | PlaybackMode::None => state.queue.tracks.get(idx).cloned(),
                PlaybackMode::Radio => state.radio.tracks.get(idx).cloned(),
            }
        }

        // Explicit detail selections take precedence over their parent list.
        View::Browse => {
            if let Some(track) = crate::app::sonic_details::selected_track(state) {
                return Some(track.clone());
            }
            // Miller column Track item → get full Track from column's tracks vec
            if let Some(nav) = state.browse_nav()
                && let Some(col) = nav.columns.get(nav.focused_column)
                && let Some(BrowseItem::Track { .. }) = col.items.get(col.selected_index)
                && let Some(track) = col.tracks.get(col.selected_index)
            {
                return Some(track.clone());
            }
            // Right-panel tracks
            match state.library.right_panel_mode {
                RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => state
                    .library
                    .selected_album_tracks
                    .get(state.list_state.tracks_index)
                    .cloned(),
                _ => None,
            }
        }

        // Similar view - return highlighted track when in Tracks mode
        View::Similar => {
            use crate::app::state::SimilarMode;
            if state.similar.mode == SimilarMode::Tracks {
                state
                    .similar
                    .tracks
                    .get(state.list_state.similar_index)
                    .cloned()
            } else {
                None
            }
        }

        // Other views don't show selectable tracks
        _ => None,
    }
}

/// Close the focused content column. The cat ("section") column is
/// the only un-closable surface — every Miller column, the Folders
/// root, and the track-details pane are closable. The pane closes
/// first if visible (so two Ctrl+W presses tear it down then drop
/// the next column rather than skipping straight to the column
/// underneath). When the last content column closes, focus falls
/// back to the cat col.
pub fn close_focused_browse_column(state: &mut AppState) {
    if crate::app::sonic_details::focused(state) {
        state.sonic_details = None;
        return;
    }
    if state.track_pane_focused {
        state.sonic_details = None;
        state.track_pane_open = false;
        state.track_pane_focused = false;
        state.track_pane_index = 0;
        return;
    }
    if state.tui_sidebar_group.take().is_some() {
        return;
    }
    let index = state
        .browse_nav()
        .map(|nav| nav.focused_column)
        .or_else(|| state.folder_state.as_ref().map(|nav| nav.focused_column));
    if let Some(index) = index {
        close_browse_column_at(state, index);
    }
}

/// Close a specific Miller column (and every column to its right) by
/// index, regardless of which column currently has keyboard focus.
/// Drives the "x" close-button hit region in the column title bars.
pub fn close_browse_column_at(state: &mut AppState, col_idx: usize) {
    state.miller_reveal_right = false;
    state.miller_scroll_manual = false;
    state.tui_column_header = None;
    state.pending_library_details = None;
    if state.browse_category.is_tag_section() {
        state.tag_nav_request_id = state.tag_nav_request_id.wrapping_add(1);
    } else {
        state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
    }
    if let Some(nav) = state.browse_nav_mut() {
        nav.loading = false;
    }
    state.sources.nav_tasks.remove("artist-browse");
    state.sources.nav_tasks.remove("album-browse");
    use crate::app::state::BrowseCategory;

    if state.browse_category == BrowseCategory::Folders {
        if let Some(fs) = state.folder_state.as_mut() {
            if col_idx == 0 {
                if state.sources.listing.take().is_some() {
                    state.sources.list_id = state.sources.list_id.wrapping_add(1);
                    state.library_loading = false;
                }
                state.folder_state = None;
                state.focus_category_column();
            } else if col_idx < fs.columns.len() {
                fs.columns.truncate(col_idx);
                fs.focused_column = col_idx.saturating_sub(1);
            }
        }
        return;
    }

    let column_offset: usize = state.browse_column_offset();

    let drop_nav = if let Some(nav) = state.browse_nav_mut() {
        if col_idx <= column_offset {
            nav.columns.truncate(column_offset);
            nav.focused_column = column_offset.saturating_sub(1);
            true
        } else if col_idx < nav.columns.len() {
            nav.columns.truncate(col_idx);
            nav.focused_column = col_idx.saturating_sub(1);
            false
        } else {
            false
        }
    } else {
        true
    };

    if drop_nav {
        state.track_pane_open = false;
        state.track_pane_focused = false;
        state.focus_category_column();
    }
}

/// Jump to first item in current list starting with given letter.
/// Uses sort_key logic to match the sorting (ignores "The " prefix).
fn jump_to_letter(state: &mut AppState, letter: char) {
    let letter_lower = letter.to_ascii_lowercase();

    // Check if sort key starts with the given letter (matches sorting logic)
    let starts_with =
        |title: &str| -> bool { helpers::sort_letter(title, 0).is_some_and(|c| c == letter_lower) };

    if state.focus == Focus::Left {
        // Jump in category list
        match state.browse_category {
            BrowseCategory::Library => {
                if let Some(idx) = state
                    .library
                    .artists
                    .iter()
                    .position(|a| starts_with(&a.title))
                {
                    state.list_state.artists_index = idx;
                }
            }
            BrowseCategory::Playlists => {
                if let Some(idx) = state
                    .library
                    .playlists
                    .iter()
                    .position(|p| starts_with(&p.title))
                {
                    state.list_state.playlists_index = idx;
                }
            }
            BrowseCategory::Folders => {}
            cat if cat.is_tag_section() => {
                let list = state.tag_list_for(cat);
                if let Some(idx) = list.iter().position(|g| starts_with(&g.title))
                    && let Some(c) = state.tag_nav.columns.first_mut()
                {
                    c.selected_index = idx;
                }
            }
            _ => {}
        }
    } else {
        // Jump in right panel
        match state.library.right_panel_mode {
            RightPanelMode::ArtistAlbums => {
                // +1 offset for "All Tracks" at index 0
                if let Some(idx) = state
                    .library
                    .selected_artist_albums
                    .iter()
                    .position(|a| starts_with(&a.title))
                {
                    state.list_state.right_albums_index = idx + 1;
                }
            }
            RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                if let Some(idx) = state
                    .library
                    .selected_album_tracks
                    .iter()
                    .position(|t| starts_with(&t.title))
                {
                    state.list_state.tracks_index = idx;
                }
            }
            RightPanelMode::CategoryAlbums => {
                if let Some(idx) = state
                    .library
                    .tag_albums
                    .iter()
                    .position(|a| starts_with(&a.title))
                {
                    state.library.tag_albums_index = idx;
                }
            }
            RightPanelMode::Empty => {}
        }
    }
}
