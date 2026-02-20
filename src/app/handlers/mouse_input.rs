//! Mouse input handler functions.
//!
//! All mouse event processing extracted from the event loop as free functions.

use crate::app::Action;
use crate::app::state::{
    BrowseCategory, BrowseItem, BrowseNavigationState, PlaybackMode,
    ScrollbarDrag, ScrollbarView, SearchTab, View,
};
use crate::app::AppState;
use crate::ui::widgets::scrollbar::{calc_thumb, scroll_offset_from_y};
use super::helpers;

/// Check if a browse column uses 2-row display for mouse hit-testing.
/// Mirrors the logic in `is_two_row_column()` in `src/ui/app.rs`.
fn is_two_row_browse_column(
    state: &AppState,
    col: &crate::app::state::BrowseColumn,
    col_idx: usize,
    nav: &BrowseNavigationState,
) -> bool {
    let first_is_track = col.items.first().map_or(false, |item| matches!(item, BrowseItem::Track { .. }));
    let first_is_album = col.items.first().map_or(false, |item| matches!(item, BrowseItem::Album { .. }));

    // Special track columns always get two-row display
    if first_is_track && state.is_special_track_column(nav, col_idx) {
        return true;
    }

    // Album columns in "All Artists" mode
    if first_is_album && (nav.columns.first()
        .and_then(|c| c.selected_item())
        .map_or(false, |item| matches!(item, BrowseItem::AllArtists))
        || (state.browse_category == BrowseCategory::Library
            && state.library_sub_mode != crate::app::state::LibrarySubMode::Normal
            && col_idx == 0))
    {
        return true;
    }

    // Genre/mood album columns
    if first_is_album && state.browse_category == BrowseCategory::Genres {
        return true;
    }

    false
}

/// Handle a click on a Miller column that has an active filter.
///
/// Returns `true` if this is a "second click" (drill-down): filter is deactivated,
/// caller should dispatch the drill-down action.
/// Returns `false` if this is a "first click" (selection only): filter stays active,
/// the item is highlighted and the caller should return without drilling.
///
/// Common logic shared by browse, folder, genre, and playlist column click handlers.
fn handle_filtered_column_click(
    state: &mut AppState,
    col_idx: usize,
    item_idx: usize,
    scroll_offset: usize,
    col_selected_index: usize,
) -> bool {
    // Detect "second click": item is already selected AND there was a recent click
    // (distinguishes "auto-selected by filter typing" from "selected by prior click")
    let is_drill = col_selected_index == item_idx
        && state.scroll.browse_click_time
            .map(|t| t.elapsed().as_millis() < 2000)
            .unwrap_or(false);

    state.scroll.browse = Some((col_idx, scroll_offset));
    state.scroll.browse_click_time = Some(std::time::Instant::now());

    if is_drill {
        // Second click → deactivate filter, caller dispatches drill-down
        state.list_filter.deactivate();
        true
    } else {
        // First click → update filter selection, keep filter active
        if let Some(ref results) = state.list_filter.results {
            if let Some(pos) = results.matched_indices.iter().position(|&idx| idx == item_idx) {
                state.list_filter.selected = pos;
            }
        }
        false
    }
}

/// Check if a BrowseItem should be rendered as a one-row item (no artwork) in art grid mode.
fn is_one_row_item(item: &BrowseItem) -> bool {
    matches!(item,
        BrowseItem::ArtistRadio { .. } |
        BrowseItem::CompilationTracks { .. } |
        BrowseItem::Compilations
    )
}

/// Handle mouse events.
pub fn handle_mouse(event: crossterm::event::MouseEvent, state: &mut AppState) -> Vec<Action> {
    use crossterm::event::{MouseEventKind, MouseButton};

    let click_row = event.row;
    let click_col = event.column;

    // Calculate layout regions
    // Layout: [tab_bar(1)] [content] [transport(2)] [commands(3)]
    let tab_bar_row = 0u16;
    let commands_start = state.terminal_height.saturating_sub(3); // bottom 3 rows
    let transport_start = state.terminal_height.saturating_sub(5); // 2 rows above commands

    // Confirm dialog intercepts mouse clicks when active
    if state.popups.confirm_dialog.is_some() {
        if let crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) = event.kind {
            // Read registered regions (drop borrow before mutating state)
            let regions = {
                let hr = state.hit_regions.borrow();
                hr.confirm_dialog.clone()
            };
            if let Some(regions) = regions {
                // Determine which button was clicked (if any)
                let clicked_yes = if click_row == regions.yes_button.y
                    && click_col >= regions.yes_button.x
                    && click_col < regions.yes_button.right()
                {
                    Some(true)
                } else if click_row == regions.no_button.y
                    && click_col >= regions.no_button.x
                    && click_col < regions.no_button.right()
                {
                    Some(false)
                } else {
                    None
                };

                if let Some(clicked_yes) = clicked_yes {
                    if let Some(dialog) = state.popups.confirm_dialog.as_ref() {
                        let already_selected = clicked_yes == dialog.selected_yes;
                        if already_selected {
                            let dialog = state.popups.confirm_dialog.take().unwrap();
                            if clicked_yes {
                                use crate::app::state::ConfirmAction;
                                return match dialog.on_confirm {
                                    ConfirmAction::RefreshCache => helpers::refresh_current_view(state),
                                    ConfirmAction::ClearLibraryCache => vec![Action::ClearLibraryCache],
                                    ConfirmAction::ClearArtworkCache => vec![Action::ClearArtworkCache],
                                    ConfirmAction::ClearSubfolderCache => vec![Action::ClearSubfolderCache],
                                    ConfirmAction::Quit => vec![Action::Quit],
                                };
                            }
                            return vec![];
                        } else {
                            state.popups.confirm_dialog.as_mut().unwrap().selected_yes = clicked_yes;
                            return vec![];
                        }
                    }
                }
            }
        }
        return vec![];
    }

    // Library picker popup intercepts all mouse events when active
    if state.popups.library_picker_active {
        return handle_library_picker_mouse(event, state);
    }

    // Search popup intercepts mouse clicks when active
    if state.popups.search_active {
        if let crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) = event.kind {
            let shift = event.modifiers.contains(crossterm::event::KeyModifiers::SHIFT);
            return handle_search_popup_click(click_row, click_col, shift, state);
        }
        return vec![];
    }

    // Artist radio picker popup intercepts mouse clicks when active
    if state.popups.artist_radio_picker.is_some() {
        if let crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) = event.kind {
            return handle_artist_radio_picker_click(click_row, click_col, state);
        }
        return vec![];
    }

    // Adventure launcher popup intercepts mouse clicks when active
    if state.popups.adventure_launcher.is_some() {
        if let crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) = event.kind {
            return handle_adventure_launcher_click(click_row, click_col, state);
        }
        return vec![];
    }

    // Artist bio popup intercepts all mouse events when active
    if state.popups.artist_bio.is_some() {
        match event.kind {
            crossterm::event::MouseEventKind::ScrollDown => {
                if let Some(ref mut popup) = state.popups.artist_bio {
                    popup.scroll = popup.scroll.saturating_add(3);
                }
            }
            crossterm::event::MouseEventKind::ScrollUp => {
                if let Some(ref mut popup) = state.popups.artist_bio {
                    popup.scroll = popup.scroll.saturating_sub(3);
                }
            }
            crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                state.popups.artist_bio = None;
                crate::ui::screens::now_playing::clear_artwork_cache();
            }
            _ => {}
        }
        return vec![];
    }

    // Sort popup intercepts mouse clicks when active
    if state.popups.sort.is_some() {
        if let crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) = event.kind {
            return handle_sort_popup_click(click_row, click_col, state);
        }
        return vec![];
    }

    match event.kind {
        // Left click
        MouseEventKind::Down(MouseButton::Left) => {
            // Click outside filter box closes it (filter is in the transport bar).
            // Exception: in Browse view, clicks on items in the filtered column pass
            // through (so you can select a filtered result). But if the query is empty,
            // clicking anywhere closes the filter.
            if state.list_filter.active && !(click_row >= transport_start && click_row < commands_start) {
                if state.view != View::Browse || state.list_filter.query.is_empty() {
                    state.list_filter.deactivate();
                    return vec![];
                }
                // Browse view with non-empty query: check if click lands on the filtered column
                let on_filtered_column = {
                    let hr = state.hit_regions.borrow();
                    hr.miller_columns.as_ref().and_then(|mc| {
                        mc.columns.iter().find(|c| c.col_idx == state.list_filter.column)
                            .map(|c| click_col >= c.area.x && click_col < c.area.right()
                                      && click_row >= c.area.y && click_row < c.area.bottom())
                    }).unwrap_or(false)
                };
                if !on_filtered_column {
                    state.list_filter.deactivate();
                    return vec![];
                }
            }

            // Check tab bar (top row)
            if click_row == tab_bar_row {
                return handle_tab_bar_click(click_col, state);
            }

            // Check command bar (bottom 3 rows: top commands, spacer, bottom commands)
            if click_row >= commands_start {
                if click_row == commands_start {
                    return handle_command_bar_click(click_col, state, true);
                } else if click_row == commands_start + 2 {
                    return handle_command_bar_click(click_col, state, false);
                }
                // Middle row is spacer — ignore clicks
                return vec![];
            }

            // Check transport bar (2 rows above commands)
            if click_row >= transport_start && click_row < commands_start {
                return handle_transport_down(click_col, click_row, transport_start, state);
            }

            // Content area clicks depend on view
            match state.view {
                View::Auth => {
                    return handle_auth_click(click_row, click_col, state);
                }
                View::Browse => {
                    let shift = event.modifiers.contains(crossterm::event::KeyModifiers::SHIFT);
                    return handle_browse_click(click_row, click_col, shift, state);
                }
                View::Queue | View::NowPlaying => {
                    return handle_now_playing_down(click_row, click_col, event.modifiers, state);
                }
                View::Search => {
                    let shift = event.modifiers.contains(crossterm::event::KeyModifiers::SHIFT);
                    return handle_search_click(click_row, click_col, shift, state);
                }
                View::Settings => {
                    return handle_settings_click(click_row, click_col, state);
                }
                View::Help => {
                    return handle_help_click(click_row, click_col, state);
                }
                View::Similar => {
                    let shift = event.modifiers.contains(crossterm::event::KeyModifiers::SHIFT);
                    return handle_similar_click(click_row, click_col, shift, state);
                }
            }
        }

        // Mouse drag - scrollbar drag, seek drag, or volume drag
        MouseEventKind::Drag(MouseButton::Left) => {
            if state.scroll.scrollbar_drag.is_some() {
                return handle_scrollbar_drag(click_row, state);
            }
            if state.volume_drag {
                return handle_volume_drag(click_col, state);
            }
            if state.seeking_drag {
                // When dragging, respond to either transport bar or visualizer area
                // This allows smooth dragging even if mouse moves between areas

                // Dragging in transport bar area
                if click_row >= transport_start && click_row < commands_start {
                    return handle_transport_drag(click_col, state);
                }

                // Dragging in visualizer seekbar (Now Playing view)
                if state.view == View::NowPlaying {
                    return handle_visualizer_drag(click_col, state);
                }

                // If dragging but mouse is in content area, still update based on horizontal position
                // This makes seeking feel more responsive
                if state.playback.duration_ms > 0 {
                    return handle_visualizer_drag(click_col, state);
                }
            }
        }

        // Mouse up - clear drag state
        MouseEventKind::Up(MouseButton::Left) => {
            state.seeking_drag = false;
            state.volume_drag = false;
            state.scroll.scrollbar_drag = None;
        }

        // Scroll wheel
        MouseEventKind::ScrollUp => {
            return handle_scroll(true, click_row, click_col, state);
        }
        MouseEventKind::ScrollDown => {
            return handle_scroll(false, click_row, click_col, state);
        }

        _ => {}
    }

    vec![]
}

/// Handle mouse events when the library picker popup is active.
fn handle_library_picker_mouse(event: crossterm::event::MouseEvent, state: &mut AppState) -> Vec<Action> {
    use crossterm::event::{MouseEventKind, MouseButton};

    let click_row = event.row;
    let click_col = event.column;

    // Read registered regions (drop borrow before mutating state)
    let regions = {
        let hr = state.hit_regions.borrow();
        hr.library_picker.clone()
    };
    let Some(regions) = regions else { return vec![] };

    let inside_popup = click_row >= regions.outer.y && click_row < regions.outer.bottom()
        && click_col >= regions.outer.x && click_col < regions.outer.right();

    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if inside_popup {
                if click_row >= regions.items_area.y && click_row < regions.items_area.bottom() {
                    let clicked_idx = (click_row - regions.items_area.y) as usize;

                    if clicked_idx < regions.item_count {
                        let already_highlighted = state.popups.library_picker_index == clicked_idx;
                        if already_highlighted {
                            // Second click on highlighted item: select it
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

                            if let Some((server_id, _, lib)) = all_libs.get(clicked_idx) {
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
                        } else {
                            // First click: just highlight
                            state.popups.library_picker_index = clicked_idx;
                        }
                    }
                }
            } else {
                return vec![Action::CloseLibraryPicker];
            }
        }
        MouseEventKind::ScrollUp if inside_popup => {
            if state.popups.library_picker_index > 0 {
                state.popups.library_picker_index -= 1;
            }
        }
        MouseEventKind::ScrollDown if inside_popup => {
            if state.popups.library_picker_index + 1 < regions.item_count {
                state.popups.library_picker_index += 1;
            }
        }
        _ => {}
    }

    vec![]
}

/// Shared: handle a click in the search tab bar area.
/// Returns actions if a tab was clicked, or empty vec otherwise.
fn handle_search_tab_click(rel_col: u16, state: &mut AppState) -> Vec<Action> {
    let tab_labels = [" all ", " artists ", " albums ", " playlists ", " tracks ", " genres "];
    if let Some(tab_idx) = tab_hit_test(rel_col, &tab_labels) {
        let new_tab = match tab_idx {
            0 => SearchTab::Global,
            1 => SearchTab::Artists,
            2 => SearchTab::Albums,
            3 => SearchTab::Playlists,
            4 => SearchTab::Tracks,
            _ => SearchTab::Genres,
        };
        state.search_tab = new_tab;
        state.search_focus = crate::app::state::SearchFocus::Input;
        state.list_state.search_item_index = 0;
        state.scroll.search = None;
        if !state.search_query.is_empty() {
            return vec![Action::ExecuteLocalSearch];
        }
    }
    vec![]
}

/// Shared: handle a click in the search results area.
/// Click highlights item. Click on already-highlighted item (if not a rapid double-click) opens in library.
fn handle_search_result_click(visual_row: usize, results_height: usize, state: &mut AppState) -> Vec<Action> {
    use crate::services::NavigationService;

    if visual_row >= results_height {
        return vec![];
    }

    let Some(ref results) = state.search_results else { return vec![] };
    let prev_idx = state.list_state.search_item_index;
    let was_focused = matches!(state.search_focus, crate::app::state::SearchFocus::Results);

    // Check if this is a rapid click (within 500ms) - if so, don't open on second click
    let is_rapid_click = state.scroll.search_click_time
        .map(|t| t.elapsed().as_millis() < 500)
        .unwrap_or(false);

    match state.search_tab {
        SearchTab::Global => {
            let section_counts = [
                results.artists.len(),
                results.albums.len(),
                results.playlists.len(),
                results.genres.len(),
                results.tracks.len(),
            ];
            let mut entries: Vec<Option<usize>> = Vec::new();
            let mut global_idx: usize = 0;
            for &count in &section_counts {
                if count > 0 {
                    entries.push(None); // section header
                    for _ in 0..count {
                        entries.push(Some(global_idx));
                        global_idx += 1;
                    }
                }
            }
            let display_selected = entries.iter()
                .position(|e| *e == Some(state.list_state.search_item_index))
                .unwrap_or(0);
            let scroll_offset = match state.scroll.search {
                Some(pinned) => pinned,
                None => NavigationService::calc_scroll_offset(
                    display_selected, results_height, entries.len(),
                ),
            };
            let abs_row = scroll_offset + visual_row;
            if let Some(Some(idx)) = entries.get(abs_row) {
                let already_selected = was_focused && *idx == prev_idx;
                state.search_focus = crate::app::state::SearchFocus::Results;
                state.list_state.search_item_index = *idx;
                state.scroll.search = Some(scroll_offset);
                state.scroll.search_click_time = Some(std::time::Instant::now());
                // Open in library only if already selected AND not a rapid click
                if already_selected && !is_rapid_click {
                    return vec![Action::SelectSearchResult];
                }
            }
        }
        _ => {
            let total = match state.search_tab {
                SearchTab::Artists => results.artists.len(),
                SearchTab::Albums => results.albums.len(),
                SearchTab::Playlists => results.playlists.len(),
                SearchTab::Tracks => results.tracks.len(),
                SearchTab::Genres => results.genres.len(),
                _ => 0,
            };
            let scroll_offset = match state.scroll.search {
                Some(pinned) => pinned,
                None => NavigationService::calc_scroll_offset(
                    state.list_state.search_item_index, results_height, total,
                ),
            };
            let actual_idx = scroll_offset + visual_row;
            if actual_idx < total {
                let already_selected = was_focused && actual_idx == prev_idx;
                state.search_focus = crate::app::state::SearchFocus::Results;
                state.list_state.search_item_index = actual_idx;
                state.scroll.search = Some(scroll_offset);
                state.scroll.search_click_time = Some(std::time::Instant::now());
                // Open in library only if already selected AND not a rapid click
                if already_selected && !is_rapid_click {
                    return vec![Action::SelectSearchResult];
                }
            }
        }
    }
    vec![]
}

/// Handle mouse click when the search popup is active.
fn handle_search_popup_click(click_row: u16, click_col: u16, shift_held: bool, state: &mut AppState) -> Vec<Action> {
    // Read registered regions (drop borrow before mutating state)
    let regions = {
        let hr = state.hit_regions.borrow();
        hr.search_popup.clone()
    };
    let Some(regions) = regions else { return vec![] };

    // Click outside popup → close
    if click_row < regions.outer.y || click_row >= regions.outer.bottom()
        || click_col < regions.outer.x || click_col >= regions.outer.right()
    {
        state.search_query.clear();
        state.search_results = None;
        state.search_focus = crate::app::state::SearchFocus::Input;
        return vec![Action::CloseSearchPopup];
    }

    // Tab area
    if click_row >= regions.tab_area.y && click_row < regions.tab_area.bottom() {
        let rel_col = click_col.saturating_sub(regions.tab_area.x);
        let actions = handle_search_tab_click(rel_col, state);
        if !actions.is_empty() {
            return actions;
        }
        return vec![];
    }

    // Search input area
    if click_row >= regions.input_area.y && click_row < regions.input_area.bottom() {
        state.search_focus = crate::app::state::SearchFocus::Input;
        return vec![];
    }

    // Results area
    if click_row >= regions.results_area.y && click_row < regions.results_area.bottom() {
        let visual_row = (click_row - regions.results_area.y) as usize;
        let results_height = regions.results_area.height as usize;
        let actions = handle_search_result_click(visual_row, results_height, state);
        if shift_held {
            return vec![Action::PlaySearchResult];
        }
        return actions;
    }

    vec![]
}

/// Handle click on the top tab bar (navigation tabs).
fn handle_tab_bar_click(click_col: u16, state: &mut AppState) -> Vec<Action> {
    // Read registered regions (drop borrow before mutating state)
    let regions = {
        let hr = state.hit_regions.borrow();
        hr.tab_bar.clone()
    };
    let Some(regions) = regions else { return vec![] };

    // Click on library name opens library picker
    if let Some(lib_rect) = &regions.library_label {
        if click_col >= lib_rect.x && click_col < lib_rect.right() {
            return vec![Action::OpenLibraryPicker];
        }
    }

    // Click on quit button
    if let Some(quit_rect) = &regions.quit_button {
        if click_col >= quit_rect.x && click_col < quit_rect.right() {
            use crate::app::state::{ConfirmDialog, ConfirmAction};
            state.popups.confirm_dialog = Some(ConfirmDialog {
                title: "Quit".to_string(),
                message: "Are you sure you want to quit?".to_string(),
                on_confirm: ConfirmAction::Quit,
                selected_yes: false,
            });
            return vec![];
        }
    }

    // Find which tab was clicked
    for (rect, idx) in &regions.tabs {
        if click_col >= rect.x && click_col < rect.right() {
            return tab_bar_action(*idx, state);
        }
    }

    vec![]
}

/// Handle click on the always-visible command bar (bottom 2 rows).
fn handle_command_bar_click(click_col: u16, state: &mut AppState, is_top_row: bool) -> Vec<Action> {
    use crate::app::handlers::key_input::{available_alt_commands, CommandModifier};

    // Read registered regions (drop borrow before mutating state)
    let regions = {
        let hr = state.hit_regions.borrow();
        hr.command_bar.clone()
    };
    let Some(regions) = regions else { return vec![] };

    let items = if is_top_row { &regions.top_row } else { &regions.bottom_row };

    // Find which command button was clicked
    for (rect, action_key) in items {
        if click_col >= rect.x && click_col < rect.right() {
            let parts: Vec<&str> = action_key.splitn(2, ':').collect();
            if parts.len() != 2 { continue; }
            let (mod_str, key_str) = (parts[0], parts[1]);

            let alt_cmds = available_alt_commands(state);

            // F-key commands use "fkey:F1" format
            if mod_str == "fkey" {
                for cmd in &alt_cmds {
                    if cmd.display_key == Some(key_str) {
                        if !cmd.enabled {
                            return vec![];
                        }
                        return alt_bar_item_action(cmd, state);
                    }
                }
                return vec![];
            }

            // Modifier+key commands use "ctrl:e" format
            for cmd in &alt_cmds {
                let cmd_mod_str = match cmd.modifier {
                    CommandModifier::Ctrl => "ctrl",
                    CommandModifier::Alt => "alt",
                    CommandModifier::None => "none",
                };
                let cmd_key_str = cmd.key.to_string();
                if cmd_mod_str == mod_str && cmd_key_str == key_str {
                    if !cmd.enabled {
                        return vec![];
                    }
                    return alt_bar_item_action(cmd, state);
                }
            }
        }
    }

    vec![]
}

/// Map a clicked alt bar command to an action.
fn alt_bar_item_action(cmd: &crate::app::handlers::key_input::AltCommand, state: &mut AppState) -> Vec<Action> {
    use crate::app::handlers::key_input::CommandModifier;
    match (cmd.modifier, cmd.key) {
        (CommandModifier::Ctrl, 'e') => vec![Action::EnqueueSelection],
        (CommandModifier::Ctrl, 'm') => super::key_input::get_similar_action(state),
        (CommandModifier::Ctrl, 'j') => super::key_input::navigate_to_album(state),
        (CommandModifier::Ctrl, 'w') => vec![Action::PromptSavePlaylist],
        (CommandModifier::Ctrl, 'x') => vec![Action::ClearQueue],
        (CommandModifier::Alt, 'f') => vec![Action::ActivateListFilter],
        (CommandModifier::Alt, 'r') => {
            if let Some(ref lib_key) = state.active_library {
                let key = format!("/library/sections/{}/stations/randomAlbum", lib_key);
                vec![Action::PlayStation(key)]
            } else {
                vec![]
            }
        }
        (CommandModifier::Ctrl, 's') => vec![Action::OpenSortPopup],
        (CommandModifier::Ctrl, 'f') => {
            if state.popups.search_active {
                vec![Action::CloseSearchPopup]
            } else {
                vec![Action::OpenSearchPopup]
            }
        }
        (CommandModifier::Ctrl, 'q') => {
            use crate::app::state::{ConfirmDialog, ConfirmAction};
            state.popups.confirm_dialog = Some(ConfirmDialog {
                title: "Quit".to_string(),
                message: "Are you sure you want to quit?".to_string(),
                on_confirm: ConfirmAction::Quit,
                selected_yes: false,
            });
            vec![]
        }
        (CommandModifier::None, _) => {
            // Function keys
            match cmd.display_key {
                Some("F1") => vec![Action::SetView(View::Help)],
                Some("F2") => vec![Action::OpenSettings],
                Some("F3") => vec![Action::OpenLibraryPicker],
                Some("F4") => {
                    if let Some((artist_key, artist_name)) = helpers::get_artist_for_bio(state) {
                        vec![Action::ShowArtistBio { artist_key, artist_name }]
                    } else {
                        vec![]
                    }
                }
                Some("F5") => helpers::refresh_current_view(state),
                _ => vec![],
            }
        }
        _ => vec![],
    }
}

/// Return the action for clicking a shortcut bar item (with cycling support).
fn tab_bar_action(idx: usize, _state: &AppState) -> Vec<Action> {
    match idx {
        0 => {
            // Library: just switch (no cycling)
            vec![Action::SetCategory(BrowseCategory::Library), Action::SetView(View::Browse)]
        }
        1 => {
            // Playlists: just switch (no cycling)
            vec![Action::SetCategory(BrowseCategory::Playlists), Action::SetView(View::Browse)]
        }
        2 => {
            // Genres: just switch (no cycling -- use tabs)
            vec![Action::SetCategory(BrowseCategory::Genres), Action::SetView(View::Browse)]
        }
        3 => {
            // Folders: just switch (no cycling)
            vec![Action::SetCategory(BrowseCategory::Folders), Action::SetView(View::Browse)]
        }
        4 => {
            // Queue
            vec![Action::SetView(View::Queue)]
        }
        5 => {
            // Now Playing (visualizer)
            vec![Action::SetView(View::NowPlaying), Action::LoadWaveform]
        }
        _ => vec![],
    }
}


/// Handle mouse down on the transport bar.
fn handle_transport_down(click_col: u16, _click_row: u16, _transport_start: u16, state: &mut AppState) -> Vec<Action> {
    // Read registered regions (drop borrow before mutating state)
    let regions = {
        let hr = state.hit_regions.borrow();
        hr.transport.clone()
    };
    let Some(regions) = regions else { return vec![] };

    // Volume slider click (inline, to the left of speaker icon)
    if let Some(ref slider_rect) = regions.volume_slider {
        if click_col >= slider_rect.x && click_col < slider_rect.right() {
            let relative_pos = click_col - slider_rect.x;
            let vol = (relative_pos as f32 / slider_rect.width as f32).clamp(0.0, 1.0);
            state.volume_slider_until = Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
            state.volume_drag = true;
            return vec![Action::SetVolume(vol)];
        }
    }

    // Speaker icon: toggle volume slider visibility (or mute if already showing)
    if let Some(ref speaker_rect) = regions.speaker_icon {
        if click_col >= speaker_rect.x && click_col < speaker_rect.right() {
            if state.volume_slider_until.map_or(false, |t| t > std::time::Instant::now()) {
                // Slider already visible: toggle mute
                return vec![Action::ToggleMute];
            } else {
                // Show volume slider
                state.volume_slider_until = Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
                return vec![];
            }
        }
    }

    // Search icon area
    if let Some(ref search_rect) = regions.search_icon {
        if state.view == View::Browse && click_col >= search_rect.x && click_col < search_rect.right() {
            if state.list_filter.active {
                return vec![Action::DeactivateListFilter];
            } else {
                return vec![Action::ActivateListFilter];
            }
        }
    }

    // Play/pause button
    if click_col >= regions.play_pause.x && click_col < regions.play_pause.right() {
        return vec![Action::TogglePlayPause];
    }

    // Seek bar
    let seekable_width = regions.seekbar.width;
    if state.playback.duration_ms > 0
        && click_col >= regions.seekbar.x && click_col < regions.seekbar.right()
    {
        let relative_pos = click_col - regions.seekbar.x;

        // Calculate where the indicator currently is
        let progress = state.playback.position_ms as f64 / state.playback.duration_ms as f64;
        let indicator_pos = (progress * seekable_width as f64) as u16;

        // Check if click is on or near the indicator (within 1 char)
        let on_indicator = relative_pos >= indicator_pos.saturating_sub(1)
            && relative_pos <= indicator_pos.saturating_add(1);

        if on_indicator {
            state.seeking_drag = true;
        }

        let seek_progress = (relative_pos as f64 / seekable_width as f64).clamp(0.0, 1.0);
        let seek_ms = (seek_progress * state.playback.duration_ms as f64) as u64;
        return vec![Action::Seek(seek_ms)];
    }

    // Previous track button
    if click_col >= regions.prev_track.x && click_col < regions.prev_track.right() {
        return vec![Action::Previous];
    }

    // Next track button
    if click_col >= regions.next_track.x && click_col < regions.next_track.right() {
        return vec![Action::Next];
    }

    // Track info area: navigate to Now Playing
    if let Some(ref info_rect) = regions.track_info {
        if click_col >= info_rect.x && click_col < info_rect.right() {
            return vec![Action::SetView(View::NowPlaying), Action::LoadWaveform];
        }
    }

    vec![]
}

/// Handle mouse drag on the volume slider.
fn handle_volume_drag(click_col: u16, state: &mut AppState) -> Vec<Action> {
    let slider = {
        let hr = state.hit_regions.borrow();
        hr.transport.as_ref().and_then(|t| t.volume_slider)
    };
    let Some(slider) = slider else { return vec![] };

    let clamped_col = click_col.max(slider.x).min(slider.x + slider.width);
    let relative_pos = clamped_col - slider.x;
    let vol = (relative_pos as f32 / slider.width as f32).clamp(0.0, 1.0);
    state.volume_slider_until = Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
    vec![Action::SetVolume(vol)]
}

/// Handle mouse drag on the transport bar (only when seeking_drag is true).
fn handle_transport_drag(click_col: u16, state: &AppState) -> Vec<Action> {
    if state.playback.duration_ms > 0 {
        // Read registered seekbar region
        let seekbar = {
            let hr = state.hit_regions.borrow();
            hr.transport.as_ref().map(|t| t.seekbar)
        };
        let Some(seekbar) = seekbar else {
            return vec![];
        };

        let clamped_col = click_col.max(seekbar.x).min(seekbar.x + seekbar.width);
        let relative_pos = clamped_col - seekbar.x;
        let progress = (relative_pos as f64 / seekbar.width as f64).clamp(0.0, 1.0);
        let seek_ms = (progress * state.playback.duration_ms as f64) as u64;
        return vec![Action::Seek(seek_ms)];
    }
    vec![]
}

// ============================================================================
// Miller Column Hit-Testing
// ============================================================================

/// Compute the full-width Miller column area for the Browse view.
/// Must replicate the layout from AppLayout (30 + remaining = full width).
/// Hit-test a click against a tab bar.
/// Returns Some(tab_index) if the click maps to a tab label.
/// Tab labels should include padding (e.g., " Playlists ").
fn tab_hit_test(click_col: u16, labels: &[&str]) -> Option<usize> {
    let mut x = 0u16;
    let divider_width = 3u16; // " │ "
    for (i, label) in labels.iter().enumerate() {
        let label_width = label.len() as u16;
        if click_col >= x && click_col < x + label_width {
            return Some(i);
        }
        x += label_width;
        if i < labels.len() - 1 {
            x += divider_width;
        }
    }
    None
}

/// Hit-test a click against the title bar (top border row) of Miller columns.
/// Returns Some(col_idx) if the click is on a column's title row.
fn miller_title_hit_test(
    click_col: u16,
    click_row: u16,
    state: &AppState,
) -> Option<usize> {
    let regions = {
        let hr = state.hit_regions.borrow();
        hr.miller_columns.clone()
    };
    let regions = regions?;

    for col_region in &regions.columns {
        if click_col < col_region.area.x || click_col >= col_region.area.right() {
            continue;
        }
        // Title bar is the top border row: area.y (before inner.y)
        if click_row == col_region.area.y {
            return Some(col_region.col_idx);
        }
    }
    None
}

/// Cycle the sort mode for a specific column (used by title bar click).
/// Cycles through modes only (always ascending): Default → ByTitle → ... → Shuffled → Default.
/// Use the sort popup (Ctrl+S) for ascending/descending control.
fn cycle_column_sort(state: &mut AppState, col_idx: usize) -> Vec<Action> {
    use crate::app::state::{BrowseItem, ColumnSortMode, SortColumnType};

    let (current_mode, column_type) = {
        let nav = match state.browse_nav() {
            Some(n) => n,
            None => return vec![],
        };
        let col = match nav.columns.get(col_idx) {
            Some(c) => c,
            None => return vec![],
        };

        let first_item = col.items.first();
        let ct = if first_item.map_or(false, |i| matches!(i, BrowseItem::Artist { .. }))
            || col.items.iter().take(3).any(|i| matches!(i, BrowseItem::Artist { .. }))
        {
            SortColumnType::Artist
        } else if first_item.map_or(false, |i| matches!(i, BrowseItem::Album { .. }))
            || col.items.iter().take(4).any(|i| matches!(i, BrowseItem::Album { .. }))
        {
            SortColumnType::Album
        } else if first_item.map_or(false, |i| matches!(i, BrowseItem::Track { .. })) {
            if state.is_special_track_column(nav, col_idx) {
                SortColumnType::AllTracks
            } else {
                SortColumnType::Track
            }
        } else {
            return vec![];
        };

        (col.sort_mode, ct)
    };

    let modes = match column_type {
        SortColumnType::Artist => vec![ColumnSortMode::Default, ColumnSortMode::Shuffled],
        SortColumnType::Album => vec![ColumnSortMode::Default, ColumnSortMode::ByTitle, ColumnSortMode::ByArtist, ColumnSortMode::Shuffled],
        SortColumnType::Track => vec![ColumnSortMode::Default, ColumnSortMode::ByTitle, ColumnSortMode::ByDuration, ColumnSortMode::Shuffled],
        SortColumnType::AllTracks => vec![ColumnSortMode::Default, ColumnSortMode::ByArtist, ColumnSortMode::ByAlbum, ColumnSortMode::ByTitle, ColumnSortMode::ByDuration, ColumnSortMode::Shuffled],
    };

    // Find current position and advance to next mode
    let current_pos = modes.iter().position(|m| *m == current_mode).unwrap_or(0);
    let next_mode = modes[(current_pos + 1) % modes.len()];

    super::key_input::sort_popup::apply_sort_for_column(state, col_idx, next_mode)
}

/// Hit-test a click against Miller column layout for BrowseNavigationState.
/// Returns Some((col_idx, item_idx, scroll_offset)) if the click maps to an item.
/// item_idx is always an index into the full col.items list (mapped through
/// matched_indices when the list filter is active on the clicked column).
fn miller_hit_test(
    click_col: u16,
    click_row: u16,
    nav: &BrowseNavigationState,
    state: &AppState,
) -> Option<(usize, usize, usize)> {
    // Read registered Miller column regions
    let regions = {
        let hr = state.hit_regions.borrow();
        hr.miller_columns.clone()
    };
    let regions = regions?;

    // Check overall area bounds
    if click_row < regions.area.y || click_row >= regions.area.bottom() {
        return None;
    }
    if click_col < regions.area.x || click_col >= regions.area.right() {
        return None;
    }

    // Check if filter is active on this category
    let filter_active = state.list_filter.active
        && state.list_filter.category == state.browse_category;

    // Find which registered column was clicked
    for col_region in &regions.columns {
        let col_idx = col_region.col_idx;
        if col_idx >= nav.columns.len() {
            continue;
        }

        if click_col < col_region.area.x || click_col >= col_region.area.right() {
            continue;
        }

        let col = &nav.columns[col_idx];
        if col.items.is_empty() {
            return None;
        }

        let inner_y = col_region.inner.y;
        let inner_height = col_region.inner.height;

        if click_row < inner_y || click_row >= inner_y + inner_height {
            return None;
        }

        let click_offset = (click_row - inner_y) as usize;

        // Use pinned scroll offset if set for this column
        let pinned = state.scroll.browse.and_then(|(pc, po)| if pc == col_idx { Some(po) } else { None });

        // Check if this column has artwork_visible enabled
        if col.artwork_visible {
            // Cover art mode: mixed row heights (one-row pinned items vs art-height items)
            let total_items = col.items.len();
            let art_count = col.items.iter().filter(|item| !is_one_row_item(item)).count().max(1);
            let target_visible = 3u16.max((art_count as u16).min(5));
            let art_row_height = (inner_height / target_visible).max(3) as usize;

            // Spacer: blank row between last one-row item and first art item
            let has_spacer_after = |idx: usize| -> bool {
                idx + 1 < total_items
                    && is_one_row_item(&col.items[idx])
                    && !is_one_row_item(&col.items[idx + 1])
            };

            // Compute visible count and scroll offset with mixed heights
            let count_visible = |offset: usize| -> usize {
                let mut y = 0usize;
                let mut count = 0;
                for i in offset..total_items {
                    let h = if is_one_row_item(&col.items[i]) { 1 } else { art_row_height };
                    let spacer = if has_spacer_after(i) { 1 } else { 0 };
                    if y + h + spacer > inner_height as usize { break; }
                    y += h + spacer;
                    count += 1;
                }
                count
            };

            let scroll_offset = if let Some(p) = pinned { p } else {
                let mut offset = 0;
                loop {
                    let visible = count_visible(offset);
                    if visible == 0 { break; }
                    if col.selected_index >= offset && col.selected_index < offset + visible { break; }
                    if col.selected_index < offset { offset = col.selected_index; break; }
                    offset += 1;
                }
                offset
            };

            // Walk through visible items to find which one was clicked
            let mut y = 0usize;
            for i in scroll_offset..total_items {
                let h = if is_one_row_item(&col.items[i]) { 1 } else { art_row_height };
                if y + h > inner_height as usize { break; }
                if click_offset >= y && click_offset < y + h {
                    return Some((col_idx, i, scroll_offset));
                }
                y += h;
                if has_spacer_after(i) { y += 1; }
            }
        } else if filter_active && state.list_filter.column == col_idx && state.list_filter.results.is_some() {
            // Filtered mode: only matched items are shown
            if let Some(ref results) = state.list_filter.results {
                if results.matched_indices.is_empty() {
                    return None;
                }
                let total_display = results.matched_indices.len();
                let visible_height = inner_height as usize;

                // display_selected_idx = position of col.selected_index in matched list
                let display_selected = results.matched_indices.iter()
                    .position(|&idx| idx == col.selected_index)
                    .unwrap_or(0);
                let scroll_offset = pinned.unwrap_or_else(|| helpers::calc_scroll_offset(display_selected, visible_height, total_display));
                let display_idx = scroll_offset + click_offset;

                if display_idx < total_display {
                    // Map through matched_indices to get actual item index
                    return Some((col_idx, results.matched_indices[display_idx], scroll_offset));
                }
            }
            return None;
        } else {
            // Check if this column uses 2-row display
            let is_two_row = is_two_row_browse_column(state, col, col_idx, nav);
            let rows_per_item = if is_two_row { 2 } else { 1 };

            let total_items = col.items.len();
            let visible_height = inner_height as usize;
            let visible_item_count = visible_height / rows_per_item;
            let scroll_offset = pinned.unwrap_or_else(|| helpers::calc_scroll_offset(col.selected_index, visible_item_count, total_items));
            let item_idx = scroll_offset + click_offset / rows_per_item;

            if item_idx < total_items {
                return Some((col_idx, item_idx, scroll_offset));
            }
        }

        return None;
    }

    None
}

/// Hit-test a click against Miller column layout for FolderNavigationState.
fn folder_hit_test(
    click_col: u16,
    click_row: u16,
    state: &AppState,
) -> Option<(usize, usize, usize)> {
    let folder_state = state.folder_state.as_ref()?;

    // Read registered Miller column regions
    let regions = {
        let hr = state.hit_regions.borrow();
        hr.miller_columns.clone()
    };
    let regions = regions?;

    if click_row < regions.area.y || click_row >= regions.area.bottom() {
        return None;
    }

    for col_region in &regions.columns {
        let col_idx = col_region.col_idx;
        if col_idx >= folder_state.columns.len() {
            continue;
        }

        if click_col < col_region.area.x || click_col >= col_region.area.right() {
            continue;
        }

        let col = &folder_state.columns[col_idx];
        if col.items.is_empty() {
            return None;
        }

        let inner_y = col_region.inner.y;
        let inner_height = col_region.inner.height;

        if click_row < inner_y || click_row >= inner_y + inner_height {
            return None;
        }

        let click_offset = (click_row - inner_y) as usize;
        let visible_height = inner_height as usize;

        // Use pinned scroll offset if set for this column
        let pinned = state.scroll.browse.and_then(|(pc, po)| if pc == col_idx { Some(po) } else { None });

        // Check if filter is active on this folder column with actual results
        let filter_on_col = state.list_filter.active
            && state.list_filter.category == BrowseCategory::Folders
            && state.list_filter.column == col_idx
            && state.list_filter.results.is_some();

        if filter_on_col {
            if let Some(ref results) = state.list_filter.results {
                if results.matched_indices.is_empty() {
                    return None;
                }
                let total_display = results.matched_indices.len();
                let display_selected = results.matched_indices.iter()
                    .position(|&idx| idx == col.selected_index)
                    .unwrap_or(0);
                let scroll_offset = pinned.unwrap_or_else(|| helpers::calc_scroll_offset(display_selected, visible_height, total_display));
                let display_idx = scroll_offset + click_offset;
                if display_idx < total_display {
                    return Some((col_idx, results.matched_indices[display_idx], scroll_offset));
                }
            }
            return None;
        }

        let total_items = col.items.len();
        let scroll_offset = pinned.unwrap_or_else(|| helpers::calc_scroll_offset(col.selected_index, visible_height, total_items));
        let item_idx = scroll_offset + click_offset;

        if item_idx < total_items {
            return Some((col_idx, item_idx, scroll_offset));
        }

        return None;
    }

    None
}

/// Identify which Miller column the cursor is over (for scroll events).
/// Returns the column index, or None if not over a column.
fn miller_column_at(click_col: u16, _nav: &BrowseNavigationState, state: &AppState) -> Option<usize> {
    let miller = {
        let hr = state.hit_regions.borrow();
        hr.miller_columns.clone()
    };
    let mr = miller?;

    for col_reg in &mr.columns {
        if click_col >= col_reg.area.x && click_col < col_reg.area.right() {
            return Some(col_reg.col_idx);
        }
    }

    None
}

// ============================================================================
// Browse Click Handlers
// ============================================================================

/// Handle click in Browse view using Miller column hit-testing.
fn handle_browse_click(click_row: u16, click_col: u16, shift_held: bool, state: &mut AppState) -> Vec<Action> {
    // Check scrollbar click first (before item selection)
    match state.browse_category {
        BrowseCategory::Folders => {
            if let Some(actions) = try_folder_scrollbar_click(click_col, click_row, state) {
                return actions;
            }
        }
        BrowseCategory::Library | BrowseCategory::Genres | BrowseCategory::Playlists => {
            if let Some(actions) = try_browse_scrollbar_click(click_col, click_row, state) {
                return actions;
            }
        }
    }

    match state.browse_category {
        BrowseCategory::Folders => {
            return handle_folder_click(click_row, click_col, state);
        }
        BrowseCategory::Library | BrowseCategory::Genres | BrowseCategory::Playlists => {
            // All use BrowseNavigationState
            let nav = match state.browse_category {
                BrowseCategory::Library => &state.artist_nav,
                BrowseCategory::Genres => &state.genre_nav,
                BrowseCategory::Playlists => &state.playlist_nav,
                _ => unreachable!(),
            };

            // Check if click is on a column title bar (border row)
            if let Some(col_idx) = miller_title_hit_test(click_col, click_row, state) {
                // Cycle to the next sort mode for this column
                return cycle_column_sort(state, col_idx);
            }

            if let Some((col_idx, item_idx, scroll_offset)) = miller_hit_test(click_col, click_row, nav, state) {
                // Check if this click is on a column with an active filter
                let filter_on_click_col = state.list_filter.active
                    && state.list_filter.category == state.browse_category
                    && state.list_filter.column == col_idx
                    && state.list_filter.results.is_some();

                let nav = match state.browse_category {
                    BrowseCategory::Library => &mut state.artist_nav,
                    BrowseCategory::Genres => &mut state.genre_nav,
                    BrowseCategory::Playlists => &mut state.playlist_nav,
                    _ => unreachable!(),
                };

                // If clicking a different column, change focus
                if col_idx != nav.focused_column {
                    state.scroll.browse = Some((col_idx, scroll_offset));
                    state.scroll.browse_click_time = Some(std::time::Instant::now());
                    nav.focused_column = col_idx;
                    nav.truncate_right();
                    if let Some(col) = nav.columns.get_mut(col_idx) {
                        col.selected_index = item_idx;
                    }
                    // Moving to a different column clears filter
                    if state.list_filter.active {
                        state.list_filter.deactivate();
                    }
                    return vec![];
                }

                // Determine if this click should drill down or just select
                let col_sel = nav.columns.get(col_idx).map(|c| c.selected_index).unwrap_or(0);
                let should_drill = if filter_on_click_col {
                    // Filter active: use helper (first click = select, second = drill)
                    let drill = handle_filtered_column_click(state, col_idx, item_idx, scroll_offset, col_sel);
                    // Always update column selection
                    let nav = match state.browse_category {
                        BrowseCategory::Library => &mut state.artist_nav,
                        BrowseCategory::Genres => &mut state.genre_nav,
                        BrowseCategory::Playlists => &mut state.playlist_nav,
                        _ => return vec![],
                    };
                    if let Some(col) = nav.columns.get_mut(col_idx) {
                        col.selected_index = item_idx;
                    }
                    drill
                } else {
                    // No filter: click highlights, Enter key activates
                    state.scroll.browse = Some((col_idx, scroll_offset));
                    state.scroll.browse_click_time = Some(std::time::Instant::now());
                    // Update selection
                    if let Some(col) = nav.columns.get_mut(col_idx) {
                        col.selected_index = item_idx;
                    }
                    // Shift+click: enqueue
                    if shift_held {
                        if let Some(item) = {
                            let nav = match state.browse_category {
                                BrowseCategory::Library => &state.artist_nav,
                                BrowseCategory::Genres => &state.genre_nav,
                                BrowseCategory::Playlists => &state.playlist_nav,
                                _ => return vec![],
                            };
                            nav.columns.get(col_idx).and_then(|c| c.items.get(item_idx)).cloned()
                        } {
                            return browse_enqueue_action(&item, col_idx, state);
                        }
                    }
                    // Click just highlights, never drills down
                    false
                };

                if should_drill {
                    if let Some(item) = {
                        let nav = match state.browse_category {
                            BrowseCategory::Library => &state.artist_nav,
                            BrowseCategory::Genres => &state.genre_nav,
                            BrowseCategory::Playlists => &state.playlist_nav,
                            _ => return vec![],
                        };
                        nav.columns.get(col_idx).and_then(|c| c.items.get(item_idx)).cloned()
                    } {
                        if shift_held {
                            return browse_enqueue_action(&item, col_idx, state);
                        }
                        return browse_drill_down_action(item, col_idx, item_idx, state);
                    }
                } else if !filter_on_click_col {
                    // Auto-drill: update child column so right panel
                    // reflects the highlighted item.
                    // Exception: Track items only play on second click
                    // (first click highlights, matching keyboard behavior).
                    let nav = match state.browse_category {
                        BrowseCategory::Library => &state.artist_nav,
                        BrowseCategory::Genres => &state.genre_nav,
                        BrowseCategory::Playlists => &state.playlist_nav,
                        _ => return vec![],
                    };
                    if let Some(item) = nav.columns.get(col_idx).and_then(|c| c.items.get(item_idx)).cloned() {
                        let is_track = matches!(item, BrowseItem::Track { .. });
                        if is_track {
                            // Track: only play if already highlighted (second click)
                            if col_sel == item_idx {
                                return browse_drill_down_action(item, col_idx, item_idx, state);
                            }
                            // First click on track: just highlight (already done above),
                            // truncate child columns
                            let nav = match state.browse_category {
                                BrowseCategory::Library => &mut state.artist_nav,
                                BrowseCategory::Genres => &mut state.genre_nav,
                                BrowseCategory::Playlists => &mut state.playlist_nav,
                                _ => return vec![],
                            };
                            nav.truncate_right();
                        } else {
                            let drill_actions = browse_drill_down_action(item, col_idx, item_idx, state);
                            if !drill_actions.is_empty() {
                                state.auto_drill_pending = true;
                                return drill_actions;
                            }
                        }
                    } else {
                        // Non-drillable item: truncate child columns
                        let nav = match state.browse_category {
                            BrowseCategory::Library => &mut state.artist_nav,
                            BrowseCategory::Genres => &mut state.genre_nav,
                            BrowseCategory::Playlists => &mut state.playlist_nav,
                            _ => return vec![],
                        };
                        nav.truncate_right();
                    }
                }
            }
        }
    }

    vec![]
}

/// Return the play+enqueue action for shift-clicking an item in a browse Miller column.
/// Artist/Album → load all tracks and play. Track → play track + following from column.
fn browse_enqueue_action(item: &BrowseItem, col_idx: usize, state: &AppState) -> Vec<Action> {
    match item {
        BrowseItem::Artist { key, .. } => {
            vec![Action::PlayArtistTracks { artist_key: key.clone() }]
        }
        BrowseItem::Album { key, .. } => {
            vec![Action::PlayAlbum { rating_key: key.clone() }]
        }
        BrowseItem::Track { .. } => {
            let nav = match state.browse_category {
                BrowseCategory::Library => &state.artist_nav,
                BrowseCategory::Genres => &state.genre_nav,
                BrowseCategory::Playlists => &state.playlist_nav,
                _ => return vec![],
            };
            let item_idx = nav.columns.get(col_idx).map(|c| c.selected_index).unwrap_or(0);
            match state.browse_category {
                BrowseCategory::Library => {
                    vec![Action::PlayTrackFromMiller { column_index: col_idx, track_index: item_idx, single_track: false }]
                }
                BrowseCategory::Genres => {
                    vec![Action::PlayGenreTrackFromMiller { column_index: col_idx, track_index: item_idx, single_track: false }]
                }
                BrowseCategory::Playlists => {
                    vec![Action::PlayPlaylistTrackFromMiller { column_index: col_idx, track_index: item_idx, single_track: false }]
                }
                _ => vec![],
            }
        }
        _ => vec![],
    }
}

/// Return the drill-down action for clicking an already-selected item in a browse Miller column.
fn browse_drill_down_action(item: BrowseItem, col_idx: usize, item_idx: usize, state: &mut AppState) -> Vec<Action> {
    match state.browse_category {
        BrowseCategory::Library => {
            match item {
                BrowseItem::Artist { key, title, .. } => {
                    state.selected_artist_name = title;
                    vec![Action::LoadArtistAlbumsForMiller { artist_key: key }]
                }
                BrowseItem::Album { key, title, .. } => {
                    state.selected_album_title = title;
                    vec![Action::LoadAlbumTracksForMiller { album_key: key }]
                }
                BrowseItem::AllTracks { artist_key, artist_name, .. } => {
                    if artist_key == "__all_library__" {
                        state.selected_album_title = "All Tracks".to_string();
                        vec![Action::LoadAllLibraryTracksForMiller]
                    } else if artist_key == "__all_comp__" {
                        state.selected_album_title = "All Tracks".to_string();
                        vec![Action::LoadAllCompilationTracksForMiller]
                    } else if let Some(real_key) = artist_key.strip_prefix("__comp_tracks:") {
                        vec![Action::LoadCompilationAllTracksForMiller {
                            artist_key: real_key.to_string(),
                            artist_name,
                        }]
                    } else {
                        state.selected_album_title = format!("All tracks by {}", artist_name);
                        vec![Action::LoadArtistAllTracksForMiller { artist_key }]
                    }
                }
                BrowseItem::AllArtists => {
                    vec![Action::LoadAllAlbumsForMiller]
                }
                BrowseItem::Compilations => {
                    vec![Action::LoadCompilationsForMiller]
                }
                BrowseItem::CompilationTracks { artist_key, artist_name } => {
                    vec![Action::LoadCompilationAlbumsForMiller { artist_key, artist_name }]
                }
                BrowseItem::Track { .. } => {
                    vec![Action::PlayTrackFromMiller { column_index: col_idx, track_index: item_idx, single_track: true }]
                }
                _ => vec![],
            }
        }
        BrowseCategory::Genres => {
            match item {
                BrowseItem::GenreCategory { key, .. } => {
                    vec![Action::DrillGenreCategory { category_key: key }]
                }
                BrowseItem::Genre { key, .. } => {
                    vec![Action::LoadGenreAlbumsForMiller { genre_key: key }]
                }
                BrowseItem::Album { key, .. } => {
                    vec![Action::LoadGenreTracksForMiller { album_key: key }]
                }
                BrowseItem::Track { .. } => {
                    vec![Action::PlayGenreTrackFromMiller { column_index: col_idx, track_index: item_idx, single_track: true }]
                }
                _ => vec![],
            }
        }
        BrowseCategory::Playlists => {
            match item {
                BrowseItem::Playlist { key, .. } => {
                    vec![Action::LoadPlaylistTracksForMiller { playlist_key: key }]
                }
                BrowseItem::Album { key, title, .. } => {
                    // Grouped-by-album: drill into local track group
                    if let Some(col) = state.playlist_nav.columns.get(col_idx) {
                        if col.grouped_by_album {
                            if let Some(new_col) = helpers::drill_grouped_album(col, item_idx) {
                                state.playlist_nav.push_column(new_col);
                                return vec![];
                            }
                        }
                    }
                    state.selected_album_title = title;
                    vec![Action::LoadAlbumTracksForMiller { album_key: key }]
                }
                BrowseItem::Track { .. } => {
                    vec![Action::PlayPlaylistTrackFromMiller { column_index: col_idx, track_index: item_idx, single_track: true }]
                }
                _ => vec![],
            }
        }
        _ => vec![],
    }
}

/// Handle click in Folder browse mode.
fn handle_folder_click(click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
    use crate::services::FolderItemType;

    if let Some((col_idx, item_idx, scroll_offset)) = folder_hit_test(click_col, click_row, state) {
        let filter_on_click_col = state.list_filter.active
            && state.list_filter.category == BrowseCategory::Folders
            && state.list_filter.column == col_idx
            && state.list_filter.results.is_some();

        let Some(folder_state) = state.folder_state.as_mut() else {
            return vec![];
        };

        // If clicking a different column, change focus (clears filter)
        if col_idx != folder_state.focused_column {
            state.scroll.browse = Some((col_idx, scroll_offset));
            state.scroll.browse_click_time = Some(std::time::Instant::now());
            folder_state.focused_column = col_idx;
            folder_state.truncate_right_columns();
            if let Some(col) = folder_state.columns.get_mut(col_idx) {
                col.selected_index = item_idx;
            }
            if state.list_filter.active {
                state.list_filter.deactivate();
            }
            return vec![];
        }

        // Determine if this click should drill down or just select
        let col_sel = folder_state.columns.get(col_idx).map(|c| c.selected_index).unwrap_or(0);
        let should_drill = if filter_on_click_col {
            let drill = handle_filtered_column_click(state, col_idx, item_idx, scroll_offset, col_sel);
            if let Some(ref mut fs) = state.folder_state {
                if let Some(col) = fs.columns.get_mut(col_idx) {
                    col.selected_index = item_idx;
                }
            }
            drill
        } else {
            state.scroll.browse = Some((col_idx, scroll_offset));
            state.scroll.browse_click_time = Some(std::time::Instant::now());
            if col_sel == item_idx {
                true
            } else {
                if let Some(ref mut fs) = state.folder_state {
                    if let Some(col) = fs.columns.get_mut(col_idx) {
                        col.selected_index = item_idx;
                    }
                    fs.truncate_right_columns();
                }
                false
            }
        };

        if should_drill {
            if let Some(item) = state.folder_state.as_ref()
                .and_then(|fs| fs.columns.get(col_idx))
                .and_then(|c| c.items.get(item_idx)).cloned()
            {
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
    }

    vec![]
}

/// Handle mouse down in Queue or NowPlaying view.
fn handle_now_playing_down(click_row: u16, click_col: u16, modifiers: crossterm::event::KeyModifiers, state: &mut AppState) -> Vec<Action> {
    use crate::app::state::NowPlayingFocus;

    match state.view {
        View::Queue => {
            // Check scrollbar clicks first
            if let Some(actions) = try_queue_scrollbar_click(click_col, click_row, state) {
                return actions;
            }
            if let Some(actions) = try_station_scrollbar_click(click_col, click_row, state) {
                return actions;
            }

            // Read registered queue regions (drop borrow before mutating state)
            let queue_regions = {
                let hr = state.hit_regions.borrow();
                hr.queue_content.clone()
            };
            let Some(qr) = queue_regions else { return vec![] };

            // Click in station panel area (left column, below artwork)
            if click_col < qr.station_panel.right() && click_row >= qr.station_panel.y {
                state.now_playing_focus = NowPlayingFocus::Stations;
                let inner_top = qr.station_inner.y;
                let inner_bottom = qr.station_inner.bottom();
                if click_row >= inner_top && click_row < inner_bottom {
                    // Non-root columns have a "← back" row at the top
                    let has_back_item = state.station_nav.focused().map(|c| c.key.is_some()).unwrap_or(false);

                    // Click on back item row
                    if has_back_item && click_row == inner_top {
                        return vec![Action::NavigateStationsBack];
                    }

                    let back_rows: u16 = if has_back_item { 1 } else { 0 };
                    let station_inner_top = inner_top + back_rows;
                    let click_offset = (click_row - station_inner_top) as usize;
                    let visible_height = (inner_bottom - station_inner_top) as usize;

                    // Compute scroll offset once, respecting existing pin
                    let scroll_offset = if let Some(col) = state.station_nav.focused() {
                        state.scroll.station.unwrap_or_else(|| {
                            helpers::calc_scroll_offset(
                                col.selected_index, visible_height, col.stations.len(),
                            )
                        })
                    } else {
                        0
                    };

                    let (already_selected, item_idx) = if let Some(col) = state.station_nav.focused() {
                        let idx = scroll_offset + click_offset;
                        if idx < col.stations.len() {
                            (col.selected_index == idx, Some(idx))
                        } else {
                            (false, None)
                        }
                    } else {
                        (false, None)
                    };

                    if let Some(idx) = item_idx {
                        if already_selected {
                            // Click already-selected: drill down / play (same as Enter)
                            if let Some(station) = state.station_nav.selected_station().cloned() {
                                if station.is_separator() {
                                    return vec![];
                                }
                                if station.key.starts_with("action:") {
                                    return match station.key.as_str() {
                                        "action:adventure" => vec![Action::OpenAdventureLauncher],
                                        "action:artist_radio" => vec![Action::OpenArtistRadioPicker],
                                        _ => vec![],
                                    };
                                }
                                if station.key.starts_with("remix:") {
                                    return match station.key.as_str() {
                                        "remix:gemini" => vec![Action::RemixGemini],
                                        "remix:twofer" => vec![Action::RemixTwofer],
                                        "remix:stretch" => vec![Action::RemixStretch],
                                        "remix:doppelganger" => vec![Action::RemixDoppelganger],
                                        "remix:shuffle" => {
                                            if state.shuffle_undo_queue.is_some() {
                                                vec![Action::RemixUndoShuffle]
                                            } else {
                                                vec![Action::RemixShuffle]
                                            }
                                        }
                                        _ => vec![],
                                    };
                                }
                                if station.is_dj_mode() {
                                    if let Some(mode) = crate::app::state::DjMode::from_key(&station.key) {
                                        return vec![Action::ToggleDjMode(mode)];
                                    }
                                    return vec![];
                                }
                                if station.is_category() {
                                    return vec![Action::DrillIntoStation(station.key.clone(), station.title.clone())];
                                }
                                return vec![Action::PlayStation(station.key.clone())];
                            }
                        } else {
                            // Pin current scroll offset to prevent viewport jump on selection change
                            state.scroll.station = Some(scroll_offset);
                            if let Some(col) = state.station_nav.focused_mut() {
                                col.selected_index = idx;
                            }
                        }
                    }
                }
                return vec![];
            }

            // Click on title bar (first content row) of track list toggles shuffle
            if click_row == qr.track_list.y && click_col >= qr.track_list.x && !state.queue.is_empty() {
                return vec![Action::ToggleQueueShuffle];
            }

            // Track list area (right column)
            if click_col >= qr.track_list.x {
                state.now_playing_focus = NowPlayingFocus::Tracks;
                // Visual row (accounting for border + 2-row layout per item)
                let visual_row = click_row.saturating_sub(qr.track_list_inner.y) as usize;
                let item_row = visual_row / 2;

                // Calculate visible item count (2 rows per item)
                let visible_item_count = qr.track_list_inner.height as usize / 2;

                // Track list
                let tracks_len = if state.playback_mode == PlaybackMode::Radio {
                    state.radio.tracks.len()
                } else {
                    state.queue.len()
                };

                // Match the renderer's scroll offset calculation
                let selected = state.list_state.queue_index;
                let scroll_offset = match state.scroll.queue {
                    Some(pinned) => pinned,
                    None => helpers::calc_scroll_offset(selected, visible_item_count, tracks_len),
                };
                let actual_idx = item_row + scroll_offset;

                if actual_idx < tracks_len {
                    // Shift+Click: toggle multi-select
                    if modifiers.contains(crossterm::event::KeyModifiers::SHIFT) {
                        if state.queue_selected.contains(&actual_idx) {
                            state.queue_selected.remove(&actual_idx);
                        } else {
                            state.queue_selected.insert(actual_idx);
                        }
                        state.scroll.queue = Some(scroll_offset);
                        state.list_state.queue_index = actual_idx;
                        return vec![];
                    }

                    // Normal click: clear multi-select
                    if !state.queue_selected.is_empty() {
                        state.queue_selected.clear();
                    }

                    let already_selected = state.list_state.queue_index == actual_idx;
                    state.scroll.queue = Some(scroll_offset);
                    state.list_state.queue_index = actual_idx;

                    // Click already-selected item: play it (same as Enter)
                    if already_selected {
                        match state.playback_mode {
                            PlaybackMode::Queue | PlaybackMode::None => {
                                if actual_idx < state.queue.len() {
                                    return vec![Action::JumpToQueueIndex(actual_idx)];
                                }
                            }
                            PlaybackMode::Radio => {
                                if actual_idx < state.radio.tracks.len() {
                                    return vec![Action::JumpToRadioTrack(actual_idx)];
                                }
                            }
                        }
                    }
                }
            }
        }
        View::NowPlaying => {
            // Read registered Now Playing regions (drop borrow before mutating state)
            let np_regions = {
                let hr = state.hit_regions.borrow();
                hr.now_playing_content.clone()
            };
            let Some(npr) = np_regions else { return vec![] };

            // Check if click is on the tab bar row
            if click_row >= npr.visualizer_tab_area.y
                && click_row < npr.visualizer_tab_area.bottom()
                && click_col >= npr.visualizer_tab_area.x
                && click_col < npr.visualizer_tab_area.right()
            {
                // Tab bar uses Tabs widget: " waveform  │  spectrum  │  spectrogram "
                let rel_col = click_col - npr.visualizer_tab_area.x;
                let tab_labels = [" waveform ", " spectrum ", " spectrogram "];
                if let Some(tab_idx) = tab_hit_test(rel_col, &tab_labels) {
                    let new_tab = match tab_idx {
                        0 => crate::app::state::VisualizerTab::Waveform,
                        1 => crate::app::state::VisualizerTab::Spectrum,
                        _ => crate::app::state::VisualizerTab::Spectrogram,
                    };
                    state.visualizer_tab = new_tab;
                }
                return vec![];
            }

            // Check if click is within the visualizer content area (for seeking)
            let ca = &npr.visualizer_content_area;
            if click_row >= ca.y
                && click_row < ca.bottom()
                && click_col >= ca.x
                && click_col < ca.right()
                && state.playback.duration_ms > 0
            {
                let inner_width = ca.width;

                // Calculate where the indicator currently is
                let progress = state.playback.position_ms as f64 / state.playback.duration_ms as f64;
                let indicator_col = ca.x + (progress * inner_width as f64) as u16;

                // Check if click is on or near the indicator (within 2 chars)
                let on_indicator = click_col >= indicator_col.saturating_sub(2)
                    && click_col <= indicator_col.saturating_add(2);

                if on_indicator {
                    // Enable drag mode
                    state.seeking_drag = true;
                }

                // Always seek on click
                let relative_col = click_col - ca.x;
                let seek_progress = relative_col as f64 / inner_width as f64;
                let seek_ms = (seek_progress * state.playback.duration_ms as f64) as u64;
                return vec![Action::Seek(seek_ms)];
            }
        }
        _ => {}
    }

    vec![]
}

/// Handle mouse drag on the visualizer seekbar.
fn handle_visualizer_drag(click_col: u16, state: &AppState) -> Vec<Action> {
    if state.playback.duration_ms > 0 {
        let np_regions = {
            let hr = state.hit_regions.borrow();
            hr.now_playing_content.clone()
        };
        if let Some(npr) = np_regions {
            let ca = &npr.visualizer_content_area;
            let inner_width = ca.width;

            if inner_width > 0 {
                // Clamp to valid range for smoother feel at edges
                let clamped_col = click_col.max(ca.x).min(ca.right().saturating_sub(1));
                let relative_col = clamped_col - ca.x;
                let progress = (relative_col as f64 / inner_width as f64).clamp(0.0, 1.0);
                let seek_ms = (progress * state.playback.duration_ms as f64) as u64;
                return vec![Action::Seek(seek_ms)];
            }
        }
    }
    vec![]
}

/// Handle click in Search view.
fn handle_search_click(click_row: u16, click_col: u16, shift_held: bool, state: &mut AppState) -> Vec<Action> {
    // Read registered search popup regions (drop borrow before mutating state)
    let regions = {
        let hr = state.hit_regions.borrow();
        hr.search_popup.clone()
    };
    let Some(sp) = regions else { return vec![] };

    // Check if click is within popup
    if click_row < sp.outer.y || click_row >= sp.outer.bottom()
        || click_col < sp.outer.x || click_col >= sp.outer.right()
    {
        return vec![];
    }

    // Tabs area
    if click_row >= sp.tab_area.y && click_row < sp.tab_area.bottom() {
        let inner_col = click_col.saturating_sub(sp.tab_area.x);
        let actions = handle_search_tab_click(inner_col, state);
        if !actions.is_empty() {
            return actions;
        }
        return vec![];
    }

    // Results area
    if click_row >= sp.results_area.y && click_row < sp.results_area.bottom() {
        let visual_row = (click_row - sp.results_area.y) as usize;
        let results_height = sp.results_area.height as usize;
        let actions = handle_search_result_click(visual_row, results_height, state);
        if shift_held {
            return vec![Action::PlaySearchResult];
        }
        return actions;
    }

    vec![]
}

/// Handle click in Auth view (login form, server selection).
fn handle_auth_click(click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
    use crate::app::state::AuthStep;

    match state.auth_state.step {
        AuthStep::Login => {
            // Login form is centered, 50 chars wide, 12 rows tall
            let form_width = 50u16.min(state.terminal_width.saturating_sub(4));
            let form_height = 12u16;
            let form_x = (state.terminal_width.saturating_sub(form_width)) / 2;
            let form_y = (state.terminal_height.saturating_sub(form_height)) / 2;

            // Check if click is within form bounds
            if click_col >= form_x && click_col < form_x + form_width
                && click_row >= form_y && click_row < form_y + form_height
            {
                let rel_row = click_row - form_y;

                // Form layout:
                // 0-1: Title (2 rows)
                // 2-4: Username field (3 rows)
                // 5-7: Password field (3 rows)
                // 8-9: Button (2 rows)

                if rel_row >= 2 && rel_row < 5 {
                    // Username field clicked
                    state.auth_state.field_index = 0;
                    state.auth_state.editing = true;
                } else if rel_row >= 5 && rel_row < 8 {
                    // Password field clicked
                    state.auth_state.field_index = 1;
                    state.auth_state.editing = true;
                } else if rel_row >= 8 && rel_row < 10 {
                    // Sign In button clicked
                    state.auth_state.field_index = 2;
                    state.auth_state.editing = false;
                    // Trigger login action
                    return vec![Action::AuthSignIn];
                }
            }
        }
        AuthStep::ServerSelect => {
            // Server list is centered, calculate bounds
            let list_width = 50u16.min(state.terminal_width.saturating_sub(4));
            let list_height = (state.available_servers.len() as u16).min(10) + 4;
            let list_x = (state.terminal_width.saturating_sub(list_width)) / 2;
            let list_y = (state.terminal_height.saturating_sub(list_height)) / 2;

            if click_col >= list_x && click_col < list_x + list_width
                && click_row >= list_y && click_row < list_y + list_height
            {
                let rel_row = click_row - list_y;

                // Layout: 2 rows instruction, then server list, 1 row hint
                if rel_row >= 2 && rel_row < list_height - 1 {
                    let server_index = (rel_row - 2) as usize;
                    if server_index < state.available_servers.len() {
                        let already_highlighted = state.auth_state.server_index == server_index;
                        if already_highlighted {
                            return vec![Action::AuthSelectServer];
                        } else {
                            state.auth_state.server_index = server_index;
                        }
                    }
                }
            }
        }
        _ => {}
    }

    vec![]
}

/// Handle click in Settings view.
fn handle_settings_click(click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
    use crate::app::state::{SettingsFocus, SettingsSection};

    // Settings layout: left panel (sections, 16 wide) | right panel (content)
    let left_panel_width = 16u16;

    // Account for content starting at row 1 (after tab bar) + top border
    let visual_row = click_row.saturating_sub(2) as usize;

    if click_col < left_panel_width {
        // Click on section list
        let sections = SettingsSection::all();
        if visual_row < sections.len() {
            state.settings_state.section = sections[visual_row];
            state.settings_state.item_index = 0;
            state.settings_state.focus = SettingsFocus::Sections;
        }
    } else {
        // Click on items in right panel - map visual row to item index
        // Each section has different header/blank line layouts before selectable items
        let item_index = settings_visual_row_to_item(visual_row, state);

        if let Some(idx) = item_index {
            let was_selected = state.settings_state.focus == SettingsFocus::Content
                && state.settings_state.item_index == idx;
            state.settings_state.focus = SettingsFocus::Content;
            state.settings_state.item_index = idx;

            if was_selected {
                // Double-click / click already-selected: activate
                return vec![Action::SettingsSelect];
            }
        }
    }

    vec![]
}

/// Map a visual row in the settings content panel to an item index.
/// Returns None if the row is a header, blank line, or out of bounds.
fn settings_visual_row_to_item(visual_row: usize, state: &AppState) -> Option<usize> {
    use crate::app::state::{ConnectionState, SettingsSection};

    match state.settings_state.section {
        SettingsSection::Account => {
            if state.settings_state.signing_in {
                // Row 0: "Sign In:" header
                // Row 1: Username → item 0
                // Row 2: Password → item 1
                // Row 3: Sign In button → item 2
                // Row 4: blank
                // Row 5: "Available servers:" header
                // Row 6+: servers → item 3+
                match visual_row {
                    1 => Some(0),
                    2 => Some(1),
                    3 => Some(2),
                    r if r >= 6 => {
                        let server_idx = r - 6;
                        if server_idx < state.available_servers.len() {
                            Some(3 + server_idx)
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            } else if matches!(state.connection, ConnectionState::Connected { .. }) {
                // Row 0: "Signed in as ..."
                // Row 1: "Plex Pass: ..."
                // Row 2: blank
                // Row 3: "Music libraries:" header
                // Row 4+: libraries (items 0..lib_count-1)
                // Then: blank
                // Then: action items (lib_count..lib_count+3) + Sign Out (lib_count+4)
                let header_rows = 4; // signed_in, plex_pass, blank, "Music libraries:"
                let lib_count = state.libraries.len();
                let lib_end = if lib_count == 0 { header_rows + 1 } else { header_rows + lib_count };
                if visual_row >= header_rows && visual_row < lib_end {
                    Some(visual_row - header_rows) // Library item
                } else if visual_row == lib_end {
                    None // Blank line
                } else if visual_row > lib_end && visual_row <= lib_end + 5 {
                    Some(lib_count + (visual_row - lib_end - 1)) // Action items + Sign Out
                } else {
                    None
                }
            } else {
                // Row 0: "Not signed in"
                // Row 1: blank
                // Row 2: "Music libraries:" header
                // Row 3: "(signed out)"
                // Row 4: blank
                // Row 5: Sign In → item 0
                if visual_row == 5 {
                    Some(0) // Sign In
                } else {
                    None
                }
            }
        }
        SettingsSection::Textamp => {
            // Row 0: "theme:" header
            // Row 1..T: theme items (items 0..T-1)
            // Row T+1: blank
            // Row T+2: "enter: apply theme" help
            // Row T+3: blank
            // Row T+4: "graphics:" header
            // Row T+5: protocol line
            // Row T+6: "artwork:" header
            // Row T+7..T+7+A-1: artwork mode items (items T..T+A-1)
            // Row T+7+A: blank
            // Row T+7+A+1: "playback output:" header
            // Row T+7+A+2: local (item T+A)
            // Row T+7+A+3..T+7+A+2+R: remote players (items T+A+1..T+A+R)
            // Row T+7+A+3+R: blank
            // Row T+7+A+4+R: refresh players (item T+A+R+1)
            let theme_count = crate::ui::theme::ThemeName::all().len();
            let artwork_count = crate::app::state::ArtworkMode::all().len();
            let output_offset = theme_count + artwork_count;
            let player_count = state.remote.players.len();

            // Theme items: rows 1..theme_count
            if visual_row >= 1 && visual_row < 1 + theme_count {
                Some(visual_row - 1)
            }
            // Artwork items: starts at row theme_count + 7 (after blank + help + blank + graphics: + protocol + artwork:)
            else {
                let artwork_start = 1 + theme_count + 6; // themes + blank + help + blank + graphics: + protocol + artwork:
                if visual_row >= artwork_start && visual_row < artwork_start + artwork_count {
                    Some(theme_count + (visual_row - artwork_start))
                }
                // Output items: after artwork + blank + "playback output:" header
                else {
                    let local_row = artwork_start + artwork_count + 2; // blank + header
                    if visual_row == local_row {
                        Some(output_offset) // Local
                    } else if visual_row > local_row && visual_row <= local_row + player_count {
                        Some(output_offset + (visual_row - local_row)) // Remote player
                    } else if visual_row == local_row + player_count + 2 {
                        // blank + refresh players
                        Some(output_offset + 1 + player_count) // Refresh Players
                    } else {
                        None
                    }
                }
            }
        }
        SettingsSection::About => {
            // Display-only section, no selectable items
            None
        }
    }
}

/// Handle click in Help view.
fn handle_help_click(click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
    // Check scrollbar click
    if let Some(actions) = try_help_scrollbar_click(click_col, click_row, state) {
        return actions;
    }
    vec![]
}

/// Handle click in Similar view (popup overlay).
fn handle_similar_click(click_row: u16, click_col: u16, shift_held: bool, state: &mut AppState) -> Vec<Action> {
    use crate::app::state::SimilarMode;

    // Read registered regions (drop borrow before mutating state)
    let regions = {
        let hr = state.hit_regions.borrow();
        hr.similar_content.clone()
    };
    let Some(regions) = regions else { return vec![] };

    // Click outside popup: close similar view
    if click_col < regions.outer.x || click_col >= regions.outer.right()
        || click_row < regions.outer.y || click_row >= regions.outer.bottom()
    {
        return vec![Action::SetView(state.previous_view.unwrap_or(View::Browse))];
    }

    // Check scrollbar click first
    if let Some(actions) = try_similar_scrollbar_click(click_col, click_row, state) {
        return actions;
    }

    // Content area: inner minus footer (1 row at bottom)
    let content_bottom = regions.inner.y + regions.inner.height.saturating_sub(1);
    if click_row < regions.inner.y || click_row >= content_bottom {
        return vec![];
    }

    let inner_row = (click_row - regions.inner.y) as usize;
    let rows_per_item = regions.rows_per_item as usize;
    let inner_height = (content_bottom - regions.inner.y) as usize;
    let visible_item_count = inner_height / rows_per_item;

    let total = match state.similar.mode {
        SimilarMode::Albums => state.similar.albums.len(),
        SimilarMode::Tracks => state.similar.tracks.len(),
    };

    if total == 0 {
        return vec![];
    }

    let scroll_offset = match state.scroll.similar {
        Some(pinned) => pinned,
        None => helpers::calc_scroll_offset(state.list_state.similar_index, visible_item_count, total),
    };
    let clicked_idx = scroll_offset + inner_row / rows_per_item;

    if clicked_idx >= total {
        return vec![];
    }

    // Shift+click: add to queue and play (like Shift+Enter)
    if shift_held {
        state.list_state.similar_index = clicked_idx;
        state.scroll.similar = Some(scroll_offset);
        match state.similar.mode {
            SimilarMode::Albums => {
                // Shift+click: play this album and enqueue all following albums
                let albums_to_enqueue: Vec<_> = state.similar.albums[clicked_idx..].to_vec();
                if albums_to_enqueue.is_empty() {
                    return vec![];
                }
                let mut actions = Vec::new();
                // First album: play it
                let first = &albums_to_enqueue[0];
                actions.push(Action::PlayAlbum {
                    rating_key: first.rating_key.clone(),
                });
                // Remaining albums: append to end of queue
                for album in albums_to_enqueue.iter().skip(1) {
                    actions.push(Action::EnqueueAlbum {
                        rating_key: album.rating_key.clone(),
                        title: album.title.clone(),
                    });
                }
                return actions;
            }
            SimilarMode::Tracks => {
                // Shift+click: play this track and all following tracks
                let tracks: Vec<_> = state.similar.tracks[clicked_idx..].to_vec();
                if !tracks.is_empty() {
                    return vec![Action::EnqueueTracksNext(tracks)];
                }
            }
        }
        return vec![];
    }

    // Click highlights; second click on already-highlighted item activates
    let already_selected = state.list_state.similar_index == clicked_idx;
    state.scroll.similar = Some(scroll_offset);
    state.list_state.similar_index = clicked_idx;

    if already_selected {
        return super::key_input::similar::activate_similar_item(state);
    }

    vec![]
}

/// Handle scroll wheel events.
fn handle_scroll(up: bool, click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
    // Coalesce rapid scroll events (normal mouse wheel can fire multiple events per tick)
    let now = std::time::Instant::now();
    if let Some(last) = state.scroll.scroll_cooldown {
        if now.duration_since(last).as_millis() < 50 {
            return vec![];
        }
    }
    state.scroll.scroll_cooldown = Some(now);

    match state.view {
        View::Browse => {
            return handle_browse_scroll(up, click_row, click_col, state);
        }
        View::Queue => {
            // Check if scrolling in station panel area (left column),
            // or if station panel is focused (scroll it from anywhere)
            let content_y = 1u16;
            let content_height = state.terminal_height.saturating_sub(6);
            let art_height = (content_height * 40 / 100).max(8);
            let art_width = (art_height * 2).min(state.terminal_width * 40 / 100).max(25);

            let station_top = content_y + art_height;
            let in_station_area = click_col < art_width && click_row >= station_top;
            let station_focused = state.now_playing_focus == crate::app::state::NowPlayingFocus::Stations;
            if in_station_area || station_focused {
                // Station panel scroll: clear pin so view follows selection
                state.scroll.station = None;
                let delta: i32 = if up { -1 } else { 1 };
                if let Some(col) = state.station_nav.focused_mut() {
                    let max = col.stations.len().saturating_sub(1);
                    let new_idx = (col.selected_index as i32 + delta).clamp(0, max as i32) as usize;
                    col.selected_index = new_idx;
                }
                state.station_nav.truncate_right_columns();
                return vec![];
            }

            // Scroll queue (includes play history + current tracks)
            state.scroll.queue = None;
            let delta: i32 = if up { -1 } else { 1 };
            let tracks_len = if state.playback_mode == PlaybackMode::Radio {
                state.radio.tracks.len()
            } else {
                state.queue.len()
            };
            let max = tracks_len.saturating_sub(1);
            let new_idx = (state.list_state.queue_index as i32 + delta).clamp(0, max as i32) as usize;
            state.list_state.queue_index = new_idx;
        }
        View::Search => {
            // Search scrolling handled via keyboard for now
        }
        View::Help => {
            // Scroll help content
            let delta: i32 = if up { -1 } else { 1 };
            let new_scroll = (state.help_scroll as i32 + delta).max(0) as u16;
            state.help_scroll = new_scroll;
        }
        View::Similar => {
            state.scroll.similar = None;
            let delta: i32 = if up { -1 } else { 1 };
            let total = match state.similar.mode {
                crate::app::state::SimilarMode::Albums => state.similar.albums.len(),
                crate::app::state::SimilarMode::Tracks => state.similar.tracks.len(),
            };
            let max = total.saturating_sub(1);
            let new_idx = (state.list_state.similar_index as i32 + delta).clamp(0, max as i32) as usize;
            state.list_state.similar_index = new_idx;
        }
        _ => {}
    }

    vec![]
}

/// Handle scroll in Browse view using Miller column awareness.
fn handle_browse_scroll(up: bool, click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
    // If a mouse click recently set the scroll pin, ignore scroll events
    // to prevent trackpad inertia from clearing the pin and re-centering.
    if let Some(click_time) = state.scroll.browse_click_time {
        if click_time.elapsed() < std::time::Duration::from_millis(400) {
            return vec![];
        }
        state.scroll.browse_click_time = None;
    }
    // Clear scroll pin — scrolling should use fresh calc_scroll_offset
    state.scroll.browse = None;

    match state.browse_category {
        BrowseCategory::Folders => {
            if let Some((col_idx, _, _)) = folder_hit_test(click_col, click_row, state) {
                let delta: i32 = if up { -1 } else { 1 };

                // Check if filter is active on this folder column
                let filter_on_col = state.list_filter.active
                    && state.list_filter.category == BrowseCategory::Folders
                    && state.list_filter.column == col_idx;

                if filter_on_col {
                    if let Some(ref results) = state.list_filter.results {
                        if !results.matched_indices.is_empty() {
                            let max = results.matched_indices.len().saturating_sub(1);
                            let new_sel = (state.list_filter.selected as i32 + delta).clamp(0, max as i32) as usize;
                            state.list_filter.selected = new_sel;
                            if let Some(&item_idx) = results.matched_indices.get(new_sel) {
                                if let Some(folder_state) = &mut state.folder_state {
                                    if let Some(col) = folder_state.columns.get_mut(col_idx) {
                                        col.selected_index = item_idx;
                                    }
                                }
                            }
                        }
                    }
                } else {
                    if let Some(folder_state) = &mut state.folder_state {
                        if let Some(col) = folder_state.columns.get_mut(col_idx) {
                            let max = col.items.len().saturating_sub(1);
                            let new_idx = (col.selected_index as i32 + delta).clamp(0, max as i32) as usize;
                            col.selected_index = new_idx;
                        }
                        if col_idx != folder_state.focused_column {
                            folder_state.focused_column = col_idx;
                            folder_state.truncate_right_columns();
                        }
                    }
                }
            }
        }
        BrowseCategory::Library | BrowseCategory::Genres | BrowseCategory::Playlists => {
            let nav = match state.browse_category {
                BrowseCategory::Library => &state.artist_nav,
                BrowseCategory::Genres => &state.genre_nav,
                BrowseCategory::Playlists => &state.playlist_nav,
                _ => unreachable!(),
            };

            if let Some(col_idx) = miller_column_at(click_col, nav, state) {
                // Check if column has artwork visible (scroll 1 at a time in art mode)
                let is_art_scroll = nav.columns.get(col_idx).map_or(false, |c| c.artwork_visible);

                // Throttle cover art scrolling to prevent trackpad momentum
                if is_art_scroll {
                    let now = std::time::Instant::now();
                    if let Some(last) = state.scroll.art_cooldown {
                        if now.duration_since(last).as_millis() < 120 {
                            return vec![];
                        }
                    }
                    state.scroll.art_cooldown = Some(now);
                }

                let delta: i32 = if up { -1 } else { 1 };

                // Check if filter is active on this column
                let filter_on_col = state.list_filter.active
                    && state.list_filter.category == state.browse_category
                    && state.list_filter.column == col_idx;

                if filter_on_col {
                    // Navigate through filtered items
                    if let Some(ref results) = state.list_filter.results {
                        if !results.matched_indices.is_empty() {
                            let max = results.matched_indices.len().saturating_sub(1);
                            let new_sel = (state.list_filter.selected as i32 + delta).clamp(0, max as i32) as usize;
                            state.list_filter.selected = new_sel;
                            if let Some(&item_idx) = results.matched_indices.get(new_sel) {
                                let nav = match state.browse_category {
                                    BrowseCategory::Library => &mut state.artist_nav,
                                    BrowseCategory::Genres => &mut state.genre_nav,
                                    BrowseCategory::Playlists => &mut state.playlist_nav,
                                    _ => unreachable!(),
                                };
                                if let Some(col) = nav.columns.get_mut(col_idx) {
                                    col.selected_index = item_idx;
                                }
                            }
                        }
                    }
                } else {
                    let nav = match state.browse_category {
                        BrowseCategory::Library => &mut state.artist_nav,
                        BrowseCategory::Genres => &mut state.genre_nav,
                        BrowseCategory::Playlists => &mut state.playlist_nav,
                        _ => unreachable!(),
                    };

                    if let Some(col) = nav.columns.get_mut(col_idx) {
                        let max = col.items.len().saturating_sub(1);
                        let new_idx = (col.selected_index as i32 + delta).clamp(0, max as i32) as usize;
                        col.selected_index = new_idx;
                    }

                    if col_idx != nav.focused_column {
                        nav.focused_column = col_idx;
                        nav.truncate_right();
                    }

                    // Auto-drill: always update child column on scroll so the right
                    // panel reflects the highlighted item (matches keyboard Up/Down)
                    {
                        use super::key_input::{auto_drill_artist_action, auto_drill_genre_action, auto_drill_playlist_action};
                        let drill = match state.browse_category {
                            BrowseCategory::Library => auto_drill_artist_action(state),
                            BrowseCategory::Genres => auto_drill_genre_action(state),
                            BrowseCategory::Playlists => auto_drill_playlist_action(state),
                            _ => None,
                        };
                        if let Some(action) = drill {
                            state.auto_drill_pending = true;
                            return vec![action];
                        } else {
                            // Non-drillable item: truncate child columns
                            match state.browse_category {
                                BrowseCategory::Library => state.artist_nav.truncate_right(),
                                BrowseCategory::Genres => state.genre_nav.truncate_right(),
                                BrowseCategory::Playlists => state.playlist_nav.truncate_right(),
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }

    // After scroll, lazily load album art for newly visible items
    let mut actions = vec![];
    let art_batch = super::dispatch_miller::collect_viewport_art(state);
    if !art_batch.is_empty() {
        actions.push(Action::LoadAlbumArt(art_batch));
    }
    actions
}

/// Handle mouse click when the artist radio picker popup is active.
fn handle_artist_radio_picker_click(click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
    use crate::app::state::{ArtistRadioPickerStep, SearchFocus};

    // Read registered regions (drop borrow before mutating state)
    let regions = {
        let hr = state.hit_regions.borrow();
        hr.artist_radio_picker.clone()
    };
    let Some(regions) = regions else { return vec![] };

    // Click outside popup — close
    if click_col < regions.outer.x || click_col >= regions.outer.right()
        || click_row < regions.outer.y || click_row >= regions.outer.bottom()
    {
        return vec![Action::CloseArtistRadioPicker];
    }

    let picker = match &state.popups.artist_radio_picker {
        Some(p) => p,
        None => return vec![],
    };

    // Only handle clicks in SelectArtists step (results list)
    if !matches!(picker.step, ArtistRadioPickerStep::SelectArtists) {
        return vec![];
    }

    // Check if click is in the results area
    if click_row < regions.items_area.y || click_row >= regions.items_area.bottom() {
        return vec![];
    }

    let click_offset = (click_row - regions.items_area.y) as usize;
    let results_height = regions.items_area.height as usize;
    let total = picker.filtered_artists.len();
    let scroll_offset = match picker.scroll_pin {
        Some(pinned) => pinned,
        None => helpers::calc_scroll_offset(picker.item_index, results_height, total),
    };

    let clicked_idx = scroll_offset + click_offset;
    if clicked_idx >= total {
        return vec![];
    }

    let already_selected = picker.item_index == clicked_idx && picker.focus == SearchFocus::Results;

    if already_selected {
        // Second click on same item — toggle selection
        return vec![Action::ArtistRadioPickerToggleArtist];
    }

    // First click — highlight (set scroll pin)
    if let Some(ref mut picker) = state.popups.artist_radio_picker {
        picker.scroll_pin = Some(scroll_offset);
        picker.item_index = clicked_idx;
        picker.focus = SearchFocus::Results;
    }
    vec![]
}

/// Handle mouse click when the adventure launcher popup is active.
fn handle_adventure_launcher_click(click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
    use crate::app::state::{SearchFocus, AdventureDrillLevel};

    // Read registered regions (drop borrow before mutating state)
    let regions = {
        let hr = state.hit_regions.borrow();
        hr.adventure_launcher.clone()
    };
    let Some(regions) = regions else { return vec![] };

    // Click outside popup — close
    if click_col < regions.outer.x || click_col >= regions.outer.right()
        || click_row < regions.outer.y || click_row >= regions.outer.bottom()
    {
        return vec![Action::CloseAdventureLauncher];
    }

    let launcher = match &state.popups.adventure_launcher {
        Some(l) => l,
        None => return vec![],
    };

    // Results area from registered regions
    let results_y = regions.inner.y + regions.results_y_offset;
    let results_height = regions.inner.height.saturating_sub(regions.results_y_offset) as usize;

    if click_row < results_y || click_row >= results_y + results_height as u16 {
        return vec![];
    }

    let click_offset = (click_row - results_y) as usize;
    let total = match &launcher.drill {
        AdventureDrillLevel::Search => {
            if let Some(ref results) = launcher.results {
                results.artists.len() + results.albums.len() + results.tracks.len()
            } else {
                0
            }
        }
        AdventureDrillLevel::ArtistAlbums { albums, .. } => albums.len(),
        AdventureDrillLevel::AlbumTracks { tracks, .. } => tracks.len(),
    };

    let scroll_offset = match launcher.scroll_pin {
        Some(pinned) => pinned,
        None => helpers::calc_scroll_offset(launcher.item_index, results_height, total),
    };

    let clicked_display_idx = scroll_offset + click_offset;

    // For Search level with section headers, we need to map display index to item index
    let clicked_item_idx = if matches!(launcher.drill, AdventureDrillLevel::Search) {
        if let Some(ref results) = launcher.results {
            let mut item_idx = 0usize;
            let mut display_idx = 0usize;
            // Artists section
            if !results.artists.is_empty() {
                if display_idx == clicked_display_idx { return vec![]; } // clicked header
                display_idx += 1;
                for _ in &results.artists {
                    if display_idx == clicked_display_idx { break; }
                    display_idx += 1;
                    item_idx += 1;
                }
                if display_idx == clicked_display_idx { Some(item_idx) } else { None }
            } else { None }
            .or_else(|| {
                if !results.albums.is_empty() {
                    if display_idx == clicked_display_idx { return None; } // header
                    display_idx += 1;
                    for _ in &results.albums {
                        if display_idx == clicked_display_idx { return Some(item_idx); }
                        display_idx += 1;
                        item_idx += 1;
                    }
                }
                None
            })
            .or_else(|| {
                if !results.tracks.is_empty() {
                    if display_idx == clicked_display_idx { return None; } // header
                    display_idx += 1;
                    for _ in &results.tracks {
                        if display_idx == clicked_display_idx { return Some(item_idx); }
                        display_idx += 1;
                        item_idx += 1;
                    }
                }
                None
            })
        } else {
            None
        }
    } else {
        if clicked_display_idx < total { Some(clicked_display_idx) } else { None }
    };

    let Some(item_idx) = clicked_item_idx else {
        return vec![];
    };

    let already_selected = launcher.item_index == item_idx && launcher.focus == SearchFocus::Results;

    if already_selected {
        // Second click — same as Enter: drill into artist/album or select track
        match &launcher.drill {
            AdventureDrillLevel::Search => {
                if let Some(ref results) = launcher.results {
                    let artist_count = results.artists.len();
                    let album_count = results.albums.len();
                    if item_idx < artist_count {
                        let artist = &results.artists[item_idx];
                        return vec![Action::AdventureLauncherDrillArtist {
                            key: artist.rating_key.clone(),
                            name: artist.title.clone(),
                        }];
                    } else if item_idx < artist_count + album_count {
                        let album = &results.albums[item_idx - artist_count];
                        return vec![Action::AdventureLauncherDrillAlbum {
                            key: album.rating_key.clone(),
                            title: album.title.clone(),
                            artist_name: album.artist_name().to_string(),
                        }];
                    } else {
                        return vec![Action::AdventureLauncherSelectTrack];
                    }
                }
            }
            AdventureDrillLevel::ArtistAlbums { albums, artist_name, .. } => {
                if let Some(album) = albums.get(item_idx) {
                    return vec![Action::AdventureLauncherDrillAlbum {
                        key: album.rating_key.clone(),
                        title: album.title.clone(),
                        artist_name: artist_name.clone(),
                    }];
                }
            }
            AdventureDrillLevel::AlbumTracks { .. } => {
                return vec![Action::AdventureLauncherSelectTrack];
            }
        }
        return vec![];
    }

    // First click — highlight (set scroll pin)
    if let Some(ref mut launcher) = state.popups.adventure_launcher {
        launcher.scroll_pin = Some(scroll_offset);
        launcher.item_index = item_idx;
        launcher.focus = SearchFocus::Results;
    }
    vec![]
}

// ============================================================================
// Scrollbar click-and-drag support
// ============================================================================

/// Check if a click is on the scrollbar (right border column) of a bordered area.
/// Returns Some((track_y_start, track_height, thumb_pos, thumb_size, scroll_offset))
/// if the click is on the scrollbar column within the track area.
fn scrollbar_hit_test_bordered(
    click_col: u16,
    click_row: u16,
    col_area_x: u16,
    col_area_width: u16,
    col_area_y: u16,
    col_area_height: u16,
    total_items: usize,
    visible_items: usize,
    scroll_offset: usize,
) -> Option<(u16, u16, usize, usize, usize)> {
    // Scrollbar is on the rightmost column of the bordered area
    let bar_x = col_area_x + col_area_width.saturating_sub(1);
    if click_col != bar_x {
        return None;
    }

    let track_y_start = col_area_y + 1; // skip top border
    let track_height = col_area_height.saturating_sub(2); // exclude top/bottom border

    if track_height == 0 || total_items == 0 || visible_items >= total_items {
        return None;
    }

    if click_row < track_y_start || click_row >= track_y_start + track_height {
        return None;
    }

    let (thumb_size, thumb_pos) = calc_thumb(total_items, visible_items, scroll_offset, track_height as usize);
    Some((track_y_start, track_height, thumb_pos, thumb_size, scroll_offset))
}

/// Start a scrollbar drag from a click. Sets `scrollbar_drag` on state.
/// If the click is on the thumb, grab_offset is the offset within the thumb.
/// If the click is on the track (not thumb), jump to position and center grab.
fn start_scrollbar_drag(
    click_row: u16,
    track_y_start: u16,
    track_height: u16,
    thumb_pos: usize,
    thumb_size: usize,
    total_items: usize,
    visible_items: usize,
    view: ScrollbarView,
    col_idx: usize,
    state: &mut AppState,
) -> usize {
    let rel_row = (click_row - track_y_start) as usize;
    let on_thumb = rel_row >= thumb_pos && rel_row < thumb_pos + thumb_size;

    let grab_offset = if on_thumb {
        (rel_row - thumb_pos) as u16
    } else {
        (thumb_size / 2) as u16
    };

    // Compute the scroll offset for the target position
    let new_offset = if on_thumb {
        // Don't move — just start dragging
        scroll_offset_from_y(click_row, track_y_start, track_height, total_items, visible_items, grab_offset)
    } else {
        // Jump: position thumb centered on click
        scroll_offset_from_y(click_row, track_y_start, track_height, total_items, visible_items, grab_offset)
    };

    state.scroll.scrollbar_drag = Some(ScrollbarDrag {
        view,
        col_idx,
        total_items,
        visible_items,
        track_y_start,
        track_height,
        grab_offset,
    });

    new_offset
}

/// Handle scrollbar drag events (MouseDrag while scrollbar_drag is active).
fn handle_scrollbar_drag(mouse_y: u16, state: &mut AppState) -> Vec<Action> {
    let drag = match state.scroll.scrollbar_drag.as_ref() {
        Some(d) => d.clone(),
        None => return vec![],
    };

    let new_offset = scroll_offset_from_y(
        mouse_y,
        drag.track_y_start,
        drag.track_height,
        drag.total_items,
        drag.visible_items,
        drag.grab_offset,
    );

    match drag.view {
        ScrollbarView::Browse | ScrollbarView::Folder => {
            state.scroll.browse = Some((drag.col_idx, new_offset));
            state.scroll.browse_click_time = Some(std::time::Instant::now());

            // Adjust selected index to stay within the visible range
            let first_visible = new_offset;
            let last_visible = new_offset + drag.visible_items.saturating_sub(1);
            match drag.view {
                ScrollbarView::Browse => {
                    let nav = match state.browse_category {
                        BrowseCategory::Library => &mut state.artist_nav,
                        BrowseCategory::Genres => &mut state.genre_nav,
                        BrowseCategory::Playlists => &mut state.playlist_nav,
                        _ => return vec![],
                    };
                    if let Some(col) = nav.columns.get_mut(drag.col_idx) {
                        if col.selected_index < first_visible {
                            col.selected_index = first_visible;
                        } else if col.selected_index > last_visible {
                            col.selected_index = last_visible;
                        }
                    }
                }
                ScrollbarView::Folder => {
                    if let Some(folder_state) = &mut state.folder_state {
                        if let Some(col) = folder_state.columns.get_mut(drag.col_idx) {
                            if col.selected_index < first_visible {
                                col.selected_index = first_visible;
                            } else if col.selected_index > last_visible {
                                col.selected_index = last_visible;
                            }
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
        ScrollbarView::Queue => {
            state.scroll.queue = Some(new_offset);
        }
        ScrollbarView::Station => {
            state.scroll.station = Some(new_offset);
        }
        ScrollbarView::Similar => {
            state.scroll.similar = Some(new_offset);
        }
        ScrollbarView::Help => {
            state.help_scroll = new_offset as u16;
        }
    }

    vec![]
}

/// Try to handle a scrollbar click in a browse Miller column.
/// Returns Some(actions) if the click was on a scrollbar, None otherwise.
fn try_browse_scrollbar_click(
    click_col: u16,
    click_row: u16,
    state: &mut AppState,
) -> Option<Vec<Action>> {
    let miller = {
        let hr = state.hit_regions.borrow();
        hr.miller_columns.clone()
    };
    let mr = miller?;

    let nav = match state.browse_category {
        BrowseCategory::Library => &state.artist_nav,
        BrowseCategory::Genres => &state.genre_nav,
        BrowseCategory::Playlists => &state.playlist_nav,
        _ => return None,
    };

    for col_reg in &mr.columns {
        let col_idx = col_reg.col_idx;
        if col_idx >= nav.columns.len() {
            continue;
        }

        let col = &nav.columns[col_idx];
        let total_items = col.items.len();
        if total_items == 0 {
            continue;
        }

        let inner_height = col_reg.inner.height as usize;

        let visible_items = if col_reg.is_art_mode {
            let art_count = col.items.iter().filter(|item| !is_one_row_item(item)).count().max(1);
            let target_visible = 3u16.max((art_count as u16).min(5));
            let art_row_height = (inner_height as u16 / target_visible).max(3) as usize;
            let mut y = 0usize;
            let mut count = 0;
            let offset = state.scroll.browse.and_then(|(pc, po)| if pc == col_idx { Some(po) } else { None }).unwrap_or(0);
            for i in offset..total_items {
                let h = if is_one_row_item(&col.items[i]) { 1 } else { art_row_height };
                let spacer = if i + 1 < total_items && is_one_row_item(&col.items[i]) && !is_one_row_item(&col.items[i + 1]) { 1 } else { 0 };
                if y + h + spacer > inner_height { break; }
                y += h + spacer;
                count += 1;
            }
            count.max(1)
        } else {
            let rows_per_item = col_reg.rows_per_item as usize;
            inner_height / rows_per_item
        };

        let pinned = state.scroll.browse.and_then(|(pc, po)| if pc == col_idx { Some(po) } else { None });
        let scroll_offset = pinned.unwrap_or_else(|| helpers::calc_scroll_offset(col.selected_index, visible_items, total_items));

        if let Some((track_y_start, track_height, thumb_pos, thumb_size, _)) =
            scrollbar_hit_test_bordered(click_col, click_row, col_reg.area.x, col_reg.area.width, col_reg.area.y, col_reg.area.height, total_items, visible_items, scroll_offset)
        {
            let new_offset = start_scrollbar_drag(
                click_row, track_y_start, track_height, thumb_pos, thumb_size,
                total_items, visible_items, ScrollbarView::Browse, col_idx, state,
            );
            state.scroll.browse = Some((col_idx, new_offset));
            state.scroll.browse_click_time = Some(std::time::Instant::now());
            return Some(vec![]);
        }
    }

    None
}

/// Try to handle a scrollbar click in a folder Miller column.
fn try_folder_scrollbar_click(
    click_col: u16,
    click_row: u16,
    state: &mut AppState,
) -> Option<Vec<Action>> {
    let folder_state = state.folder_state.as_ref()?;

    let miller = {
        let hr = state.hit_regions.borrow();
        hr.miller_columns.clone()
    };
    let mr = miller?;

    for col_reg in &mr.columns {
        let col_idx = col_reg.col_idx;
        if col_idx >= folder_state.columns.len() {
            continue;
        }

        let col = &folder_state.columns[col_idx];
        let total_items = col.items.len();
        if total_items == 0 {
            continue;
        }

        let inner_height = col_reg.inner.height as usize;
        let visible_items = inner_height;
        let pinned = state.scroll.browse.and_then(|(pc, po)| if pc == col_idx { Some(po) } else { None });
        let scroll_offset = pinned.unwrap_or_else(|| helpers::calc_scroll_offset(col.selected_index, visible_items, total_items));

        if let Some((track_y_start, track_height, thumb_pos, thumb_size, _)) =
            scrollbar_hit_test_bordered(click_col, click_row, col_reg.area.x, col_reg.area.width, col_reg.area.y, col_reg.area.height, total_items, visible_items, scroll_offset)
        {
            let new_offset = start_scrollbar_drag(
                click_row, track_y_start, track_height, thumb_pos, thumb_size,
                total_items, visible_items, ScrollbarView::Folder, col_idx, state,
            );
            state.scroll.browse = Some((col_idx, new_offset));
            state.scroll.browse_click_time = Some(std::time::Instant::now());
            return Some(vec![]);
        }
    }

    None
}

/// Try to handle a scrollbar click in the queue track list.
fn try_queue_scrollbar_click(
    click_col: u16,
    click_row: u16,
    state: &mut AppState,
) -> Option<Vec<Action>> {
    let qr = {
        let hr = state.hit_regions.borrow();
        hr.queue_content.clone()
    }?;

    // Track list is the right column, bordered
    if click_col < qr.track_list.x {
        return None;
    }

    let tracks_len = if state.playback_mode == PlaybackMode::Radio {
        state.radio.tracks.len()
    } else {
        state.queue.len()
    };

    // 2-row items
    let inner_height = qr.track_list_inner.height as usize;
    let visible_items = inner_height / 2;
    let selected = state.list_state.queue_index;
    let scroll_offset = state.scroll.queue.unwrap_or_else(|| helpers::calc_scroll_offset(selected, visible_items, tracks_len));

    if let Some((track_y_start, track_height, thumb_pos, thumb_size, _)) =
        scrollbar_hit_test_bordered(click_col, click_row, qr.track_list.x, qr.track_list.width, qr.track_list.y, qr.track_list.height, tracks_len, visible_items, scroll_offset)
    {
        let new_offset = start_scrollbar_drag(
            click_row, track_y_start, track_height, thumb_pos, thumb_size,
            tracks_len, visible_items, ScrollbarView::Queue, 0, state,
        );
        state.scroll.queue = Some(new_offset);
        return Some(vec![]);
    }

    None
}

/// Try to handle a scrollbar click in the station panel.
fn try_station_scrollbar_click(
    click_col: u16,
    click_row: u16,
    state: &mut AppState,
) -> Option<Vec<Action>> {
    let qr = {
        let hr = state.hit_regions.borrow();
        hr.queue_content.clone()
    }?;

    // Station panel is in the left column, below artwork
    if click_col >= qr.station_panel.right() || click_row < qr.station_panel.y {
        return None;
    }

    let total_items = state.station_nav.focused().map(|c| c.stations.len()).unwrap_or(0);
    if total_items == 0 {
        return None;
    }

    let inner_height = qr.station_inner.height as usize;
    let visible_items = inner_height;
    let selected = state.station_nav.focused().map(|c| c.selected_index).unwrap_or(0);
    let scroll_offset = state.scroll.station.unwrap_or_else(|| helpers::calc_scroll_offset(selected, visible_items, total_items));

    if let Some((track_y_start, track_height, thumb_pos, thumb_size, _)) =
        scrollbar_hit_test_bordered(click_col, click_row, qr.station_panel.x, qr.station_panel.width, qr.station_panel.y, qr.station_panel.height, total_items, visible_items, scroll_offset)
    {
        let new_offset = start_scrollbar_drag(
            click_row, track_y_start, track_height, thumb_pos, thumb_size,
            total_items, visible_items, ScrollbarView::Station, 0, state,
        );
        state.scroll.station = Some(new_offset);
        return Some(vec![]);
    }

    None
}

/// Try to handle a scrollbar click in the similar popup.
fn try_similar_scrollbar_click(
    click_col: u16,
    click_row: u16,
    state: &mut AppState,
) -> Option<Vec<Action>> {
    use crate::app::state::SimilarMode;

    let sr = {
        let hr = state.hit_regions.borrow();
        hr.similar_content.clone()
    }?;

    let total_items = match state.similar.mode {
        SimilarMode::Albums => state.similar.albums.len(),
        SimilarMode::Tracks => state.similar.tracks.len(),
    };
    if total_items == 0 {
        return None;
    }

    let rows_per_item = sr.rows_per_item as usize;
    let inner_height = sr.inner.height.saturating_sub(1) as usize; // -1 for footer
    let visible_items = inner_height / rows_per_item;
    let scroll_offset = state.scroll.similar.unwrap_or_else(|| helpers::calc_scroll_offset(state.list_state.similar_index, visible_items, total_items));

    if let Some((track_y_start, track_height, thumb_pos, thumb_size, _)) =
        scrollbar_hit_test_bordered(click_col, click_row, sr.outer.x, sr.outer.width, sr.outer.y, sr.outer.height, total_items, visible_items, scroll_offset)
    {
        let new_offset = start_scrollbar_drag(
            click_row, track_y_start, track_height, thumb_pos, thumb_size,
            total_items, visible_items, ScrollbarView::Similar, 0, state,
        );
        state.scroll.similar = Some(new_offset);
        return Some(vec![]);
    }

    None
}

/// Try to handle a scrollbar click in the help view.
fn try_help_scrollbar_click(
    click_col: u16,
    click_row: u16,
    state: &mut AppState,
) -> Option<Vec<Action>> {
    let content_height = state.terminal_height.saturating_sub(6);
    let area_x = 0u16;
    let area_width = state.terminal_width;
    let area_y = 1u16; // Content starts after tab bar
    let area_height = content_height;

    // Help uses help_scroll (u16 offset), total_lines estimated from render
    // The help screen renders keybinding lines; we estimate total from the help content
    let total_lines = crate::ui::screens::help::help_total_lines();
    let visible_items = area_height.saturating_sub(2) as usize;
    if total_lines == 0 || visible_items >= total_lines {
        return None;
    }
    let scroll_offset = state.help_scroll as usize;

    if let Some((track_y_start, track_height, thumb_pos, thumb_size, _)) =
        scrollbar_hit_test_bordered(click_col, click_row, area_x, area_width, area_y, area_height, total_lines, visible_items, scroll_offset)
    {
        let new_offset = start_scrollbar_drag(
            click_row, track_y_start, track_height, thumb_pos, thumb_size,
            total_lines, visible_items, ScrollbarView::Help, 0, state,
        );
        state.help_scroll = new_offset as u16;
        return Some(vec![]);
    }

    None
}

/// Handle mouse click on the sort popup overlay.
fn handle_sort_popup_click(click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
    if state.popups.sort.is_none() {
        return vec![];
    }

    // Read registered regions (drop borrow before mutating state)
    let regions = {
        let hr = state.hit_regions.borrow();
        hr.sort_popup.clone()
    };
    let Some(regions) = regions else { return vec![] };

    // Click outside popup → close
    if click_col < regions.outer.x || click_col >= regions.outer.right()
        || click_row < regions.outer.y || click_row >= regions.outer.bottom()
    {
        return vec![Action::CloseSortPopup];
    }

    // Options occupy inner area minus footer (last row)
    let options_end = regions.inner.y + regions.inner.height.saturating_sub(1);
    if click_row >= regions.inner.y && click_row < options_end {
        let option_idx = (click_row - regions.inner.y) as usize;
        if option_idx < regions.option_count {
            let already_selected = state.popups.sort.as_ref()
                .map(|p| p.selected_index == option_idx)
                .unwrap_or(false);
            if let Some(p) = &mut state.popups.sort {
                p.selected_index = option_idx;
            }
            if already_selected {
                return super::key_input::sort_popup::apply_selected_option(state);
            }
        }
    }

    vec![]
}
