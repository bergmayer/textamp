//! Mouse input handler functions.
//!
//! All mouse event processing extracted from the event loop as free functions.

use crate::app::Action;
use crate::app::state::{
    BrowseCategory, BrowseItem, BrowseNavigationState, PlaybackMode,
    SearchTab, View,
};
use crate::app::AppState;
use crate::ui::layout::centered_rect;
use super::helpers;

/// Handle mouse events.
pub fn handle_mouse(event: crossterm::event::MouseEvent, state: &mut AppState) -> Vec<Action> {
    use crossterm::event::{MouseEventKind, MouseButton};

    let click_row = event.row;
    let click_col = event.column;

    // Calculate layout regions
    let transport_row = state.terminal_height.saturating_sub(3);
    let shortcuts_row = state.terminal_height.saturating_sub(1);

    // Library picker popup intercepts all mouse events when active
    if state.library_picker_active {
        return handle_library_picker_mouse(event, state);
    }

    match event.kind {
        // Left click
        MouseEventKind::Down(MouseButton::Left) => {
            // Check shortcut bar (bottom row)
            if click_row == shortcuts_row {
                return handle_shortcut_bar_click(click_col, state);
            }

            // Check transport bar
            if click_row >= transport_row && click_row < shortcuts_row {
                return handle_transport_down(click_col, state);
            }

            // Content area clicks depend on view
            match state.view {
                View::Auth => {
                    return handle_auth_click(click_row, click_col, state);
                }
                View::Browse => {
                    return handle_browse_click(click_row, click_col, state);
                }
                View::NowPlaying => {
                    return handle_now_playing_down(click_row, click_col, state);
                }
                View::Search => {
                    return handle_search_click(click_row, click_col, state);
                }
                View::Settings => {
                    return handle_settings_click(click_row, click_col, state);
                }
                View::Help => {
                    return handle_help_click(click_row, state);
                }
                _ => {}
            }
        }

        // Mouse drag - only seek if we started dragging on the indicator
        MouseEventKind::Drag(MouseButton::Left) => {
            if state.seeking_drag {
                // When dragging, respond to either transport bar or visualizer area
                // This allows smooth dragging even if mouse moves between areas

                // Dragging in transport bar area
                if click_row >= transport_row && click_row < shortcuts_row {
                    return handle_transport_drag(click_col, state);
                }

                // Dragging in visualizer seekbar (Now Playing view)
                if state.view == View::NowPlaying {
                    if let crate::app::state::NowPlayingMode::NowPlaying = state.now_playing_mode {
                        return handle_visualizer_drag(click_col, state);
                    }
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
    use ratatui::layout::Rect;

    let click_row = event.row;
    let click_col = event.column;

    // Calculate popup bounds (must match render_library_picker: 40% width, 30% height)
    let frame_area = Rect::new(0, 0, state.terminal_width, state.terminal_height);
    let popup = centered_rect(40, 30, frame_area);

    let lib_count = state.libraries.len();

    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // Check if click is inside the popup
            if click_row >= popup.y && click_row < popup.y + popup.height
                && click_col >= popup.x && click_col < popup.x + popup.width
            {
                // Inside popup - map click to library index
                // Layout: border(1) + title(1) = content starts at row 2 inside popup
                let inner_top = popup.y + 2; // After top border + title line
                let inner_bottom = popup.y + popup.height.saturating_sub(2); // Before help line + bottom border

                if click_row >= inner_top && click_row < inner_bottom {
                    let clicked_idx = (click_row - inner_top) as usize;
                    if clicked_idx < lib_count {
                        if state.library_picker_index == clicked_idx {
                            // Click already-selected: activate
                            if let Some(lib) = state.libraries.get(clicked_idx) {
                                let key = lib.key.clone();
                                return vec![Action::SelectLibrary(key), Action::CloseLibraryPicker];
                            }
                        } else {
                            state.library_picker_index = clicked_idx;
                        }
                    }
                }
            } else {
                // Click outside popup: close
                return vec![Action::CloseLibraryPicker];
            }
        }
        MouseEventKind::ScrollUp => {
            // Scroll up inside popup
            if click_row >= popup.y && click_row < popup.y + popup.height
                && click_col >= popup.x && click_col < popup.x + popup.width
            {
                if state.library_picker_index > 0 {
                    state.library_picker_index -= 1;
                }
            }
        }
        MouseEventKind::ScrollDown => {
            // Scroll down inside popup
            if click_row >= popup.y && click_row < popup.y + popup.height
                && click_col >= popup.x && click_col < popup.x + popup.width
            {
                if state.library_picker_index + 1 < lib_count {
                    state.library_picker_index += 1;
                }
            }
        }
        _ => {}
    }

    vec![]
}

/// Handle click on the shortcut bar at the bottom.
fn handle_shortcut_bar_click(click_col: u16, state: &AppState) -> Vec<Action> {
    // Shortcut bar items (must match render_shortcuts in ui/app.rs):
    // ^A artists | ^P playlists | ^G genres | ^O folders | ^N queue | F1 help | F2 settings
    //
    // These are centered, so we need to calculate positions based on terminal width.
    // Each item is roughly: " ^X label " with separators "|"
    //
    // Clicking an already-active item cycles its mode (like the keyboard shortcut does).
    // Note: Genres cycle includes Stations (via Ctrl+G).

    let shortcuts: [(&str, &str, usize); 8] = [
        ("^A", state.artist_view_mode.name(), 0),   // Artists
        ("^P", state.playlists_mode.name(), 1),     // Playlists
        ("^G", state.genre_content_type.name(), 2), // Genres (cycles through genres/moods/styles/stations)
        ("^O", "folders", 3),                       // Folders
        ("^N", state.now_playing_mode.name(), 4),   // Now Playing
        ("^F", "search", 5),                        // Search
        ("F1", "help", 6),                          // Help
        ("F2", "settings", 7),                      // Settings
    ];

    // Calculate total width of shortcut bar
    let mut total_width: u16 = 0;
    let mut item_ranges: Vec<(u16, u16, usize)> = Vec::new();

    for (i, (key, label, idx)) in shortcuts.iter().enumerate() {
        let separator_width = if i > 0 { 1 } else { 0 }; // "|"
        let item_width = 1 + key.len() as u16 + 1 + label.len() as u16 + 1; // " ^X label "

        let start = total_width + separator_width;
        let end = start + item_width;
        item_ranges.push((start, end, *idx));
        total_width = end;
    }

    // Center offset
    let center_offset = state.terminal_width.saturating_sub(total_width) / 2;

    // Find which item was clicked
    for (start, end, idx) in item_ranges {
        let abs_start = center_offset + start;
        let abs_end = center_offset + end;
        if click_col >= abs_start && click_col < abs_end {
            return shortcut_bar_action(idx, state);
        }
    }

    vec![]
}

/// Return the action for clicking a shortcut bar item (with cycling support).
fn shortcut_bar_action(idx: usize, state: &AppState) -> Vec<Action> {
    match idx {
        0 => {
            // Artists: cycle mode if already there, else switch
            if state.view == View::Browse && state.browse_category == BrowseCategory::Artists {
                return vec![Action::CycleArtistViewMode];
            }
            vec![Action::SetCategory(BrowseCategory::Artists), Action::SetView(View::Browse)]
        }
        1 => {
            // Playlists: cycle mode if already there, else switch
            if state.view == View::Browse && state.browse_category == BrowseCategory::Playlists {
                return vec![Action::CyclePlaylistsMode];
            }
            vec![Action::SetCategory(BrowseCategory::Playlists), Action::SetView(View::Browse)]
        }
        2 => {
            // Genres: cycle content type if already there, else switch (includes Stations)
            if state.view == View::Browse && state.browse_category == BrowseCategory::Genres {
                return vec![Action::CycleGenreContentType];
            }
            vec![Action::SetCategory(BrowseCategory::Genres), Action::SetView(View::Browse)]
        }
        3 => {
            // Folders: just switch (no cycling)
            vec![Action::SetCategory(BrowseCategory::Folders), Action::SetView(View::Browse)]
        }
        4 => {
            // Now Playing: cycle mode if already there, else switch
            if state.view == View::NowPlaying {
                return vec![Action::CycleNowPlayingMode];
            }
            vec![Action::SetView(View::NowPlaying)]
        }
        5 => {
            // Search: toggle popup
            if state.search_popup_active {
                vec![Action::CloseSearchPopup]
            } else {
                vec![Action::OpenSearchPopup]
            }
        }
        6 => {
            // Help
            vec![Action::SetView(View::Help)]
        }
        7 => {
            // Settings
            vec![Action::SetView(View::Settings)]
        }
        _ => vec![],
    }
}


/// Handle mouse down on the transport bar.
fn handle_transport_down(click_col: u16, state: &mut AppState) -> Vec<Action> {
    // Transport bar layout (controls on left):
    // [⏸] [MM:SS] [━━━●───────] [MM:SS] [ │ ] [track info...] [...] [🔍]
    // ^0  ^2      ^8           ^28      ^34                          ^end
    //
    // Fixed positions at the start:
    // - Play/pause: cols 0-1 (icon + space)
    // - Position time: cols 2-6 (5 chars MM:SS)
    // - Space: col 7
    // - Seek bar: cols 8-27 (20 chars)
    // - Space: col 28
    // - Duration time: cols 29-33 (5 chars)
    // - Separator: cols 34-38 ("  │  ")
    // - Search emoji at the far right

    // Search icon (🔍) in the right section of the transport bar.
    // Right content is ~12-15 chars from the right edge, 🔍 is first.
    // Make the entire right section clickable to toggle inline filter.
    if state.view == View::Browse && click_col >= state.terminal_width.saturating_sub(15) {
        if state.list_filter.active {
            return vec![Action::DeactivateListFilter];
        } else {
            return vec![Action::ActivateListFilter];
        }
    }

    // Play/pause button at columns 0-1
    if click_col < 2 {
        return vec![Action::TogglePlayPause];
    }

    // Seek bar at columns 8-27 (20 chars)
    let seekbar_start = 8u16;
    let seekbar_end = 28u16;
    let seekable_width = 20u16;

    if state.playback.duration_ms > 0 && click_col >= seekbar_start && click_col < seekbar_end {
        let relative_pos = click_col - seekbar_start;

        // Calculate where the indicator currently is
        let progress = state.playback.position_ms as f64 / state.playback.duration_ms as f64;
        let indicator_pos = (progress * seekable_width as f64) as u16;

        // Check if click is on or near the indicator (within 1 char)
        let on_indicator = relative_pos >= indicator_pos.saturating_sub(1)
            && relative_pos <= indicator_pos.saturating_add(1);

        if on_indicator {
            // Start drag mode
            state.seeking_drag = true;
        }

        // Always seek on click
        let seek_progress = (relative_pos as f64 / seekable_width as f64).clamp(0.0, 1.0);
        let seek_ms = (seek_progress * state.playback.duration_ms as f64) as u64;
        return vec![Action::Seek(seek_ms)];
    }

    vec![]
}

/// Handle mouse drag on the transport bar (only when seeking_drag is true).
fn handle_transport_drag(click_col: u16, state: &AppState) -> Vec<Action> {
    if state.playback.duration_ms > 0 {
        let seekbar_start = 8u16;
        let seekable_width = 20u16;

        // Allow dragging slightly outside the bar bounds for smoother interaction
        let clamped_col = click_col.max(seekbar_start).min(seekbar_start + seekable_width);
        let relative_pos = clamped_col - seekbar_start;
        let progress = (relative_pos as f64 / seekable_width as f64).clamp(0.0, 1.0);
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
fn miller_area(state: &AppState) -> (u16, u16, u16, u16) {
    // AppLayout: vertical split [Min(5), Length(2), Length(1)]
    // Content area: from y=0, height = terminal_height - 3
    // Horizontal: left_panel (30) + right_panel (remaining)
    // Miller columns combine them: x=0, width=terminal_width
    let area_x = 0u16;
    let area_y = 0u16;
    let area_width = state.terminal_width;
    let area_height = state.terminal_height.saturating_sub(3);
    (area_x, area_y, area_width, area_height)
}

/// Compute visible column layout parameters for a BrowseNavigationState.
/// Returns (max_visible, col_width, start_col, effective_columns).
fn browse_column_layout(nav: &BrowseNavigationState, area_width: u16) -> (usize, u16, usize, usize) {
    let num_columns = nav.columns.len();
    if num_columns == 0 {
        return (0, 0, 0, 0);
    }

    let last_meaningful = (0..num_columns)
        .rev()
        .find(|&i| !nav.columns[i].items.is_empty() || i <= nav.focused_column)
        .unwrap_or(0);
    let effective_columns = last_meaningful + 1;

    let max_visible = 3.min(effective_columns);
    let col_width = if max_visible > 0 { area_width / max_visible as u16 } else { 0 };

    let start_col = if nav.focused_column + 1 > max_visible {
        nav.focused_column + 1 - max_visible
    } else {
        0
    };

    (max_visible, col_width, start_col, effective_columns)
}

/// Hit-test a click against Miller column layout for BrowseNavigationState.
/// Returns Some((col_idx, item_idx)) if the click maps to an item.
/// item_idx is always an index into the full col.items list (mapped through
/// matched_indices when the list filter is active on the clicked column).
fn miller_hit_test(
    click_col: u16,
    click_row: u16,
    nav: &BrowseNavigationState,
    state: &AppState,
) -> Option<(usize, usize, usize)> {
    let (area_x, area_y, area_width, area_height) = miller_area(state);

    if click_row < area_y || click_row >= area_y + area_height {
        return None;
    }
    if click_col < area_x || click_col >= area_x + area_width {
        return None;
    }

    let (max_visible, col_width, start_col, effective_columns) =
        browse_column_layout(nav, area_width);

    if max_visible == 0 || col_width == 0 {
        return None;
    }

    // Check if filter is active on this category
    let filter_active = state.list_filter.active
        && state.list_filter.category == state.browse_category;

    // Find which visible column was clicked
    for vis_idx in 0..max_visible {
        let col_idx = start_col + vis_idx;
        if col_idx >= effective_columns || col_idx >= nav.columns.len() {
            continue;
        }

        let col_x = area_x + (vis_idx as u16 * col_width);
        let col_w = if vis_idx == max_visible - 1 {
            area_width - (vis_idx as u16 * col_width)
        } else {
            col_width
        };

        if click_col < col_x || click_col >= col_x + col_w {
            continue;
        }

        let col = &nav.columns[col_idx];
        if col.items.is_empty() {
            return None;
        }

        // Inner area: subtract 1 row for top border, 1 row for bottom border
        let inner_y = area_y + 1;
        let inner_height = area_height.saturating_sub(2);

        if click_row < inner_y || click_row >= inner_y + inner_height {
            return None;
        }

        let click_offset = (click_row - inner_y) as usize;

        // Use pinned scroll offset if set for this column
        let pinned = state.browse_scroll_pin.and_then(|(pc, po)| if pc == col_idx { Some(po) } else { None });

        // Check if this column has albums and cover art view is active
        let has_albums = col.items.iter().any(|item| matches!(item, BrowseItem::Album { .. }));
        if state.album_art_view && has_albums {
            // Cover art mode: multi-row items (filter not applied in art mode)
            let total_items = col.items.len();
            let target_visible = 3u16.max((col.items.len() as u16).min(5));
            let row_height = (inner_height / target_visible).max(3) as usize;
            let visible_rows = (inner_height as usize / row_height).max(1);

            let scroll_offset = pinned.unwrap_or_else(|| helpers::calc_scroll_offset(col.selected_index, visible_rows, total_items));
            let vis_row = click_offset / row_height;
            let item_idx = scroll_offset + vis_row;

            if item_idx < total_items {
                return Some((col_idx, item_idx, scroll_offset));
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
            // Normal mode: 1 row per item
            let total_items = col.items.len();
            let visible_height = inner_height as usize;
            let scroll_offset = pinned.unwrap_or_else(|| helpers::calc_scroll_offset(col.selected_index, visible_height, total_items));
            let item_idx = scroll_offset + click_offset;

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
    let (area_x, area_y, area_width, area_height) = miller_area(state);

    if click_row < area_y || click_row >= area_y + area_height {
        return None;
    }

    let num_columns = folder_state.columns.len();
    if num_columns == 0 {
        return None;
    }

    let last_meaningful = (0..num_columns)
        .rev()
        .find(|&i| !folder_state.columns[i].items.is_empty() || i <= folder_state.focused_column)
        .unwrap_or(0);
    let effective_columns = last_meaningful + 1;
    let max_visible = 3.min(effective_columns);
    let col_width = if max_visible > 0 { area_width / max_visible as u16 } else { return None; };
    let start_col = if folder_state.focused_column + 1 > max_visible {
        folder_state.focused_column + 1 - max_visible
    } else {
        0
    };

    for vis_idx in 0..max_visible {
        let col_idx = start_col + vis_idx;
        if col_idx >= effective_columns || col_idx >= folder_state.columns.len() {
            continue;
        }

        let col_x = area_x + (vis_idx as u16 * col_width);
        let col_w = if vis_idx == max_visible - 1 {
            area_width - (vis_idx as u16 * col_width)
        } else {
            col_width
        };

        if click_col < col_x || click_col >= col_x + col_w {
            continue;
        }

        let col = &folder_state.columns[col_idx];
        if col.items.is_empty() {
            return None;
        }

        let inner_y = area_y + 1;
        let inner_height = area_height.saturating_sub(2);

        if click_row < inner_y || click_row >= inner_y + inner_height {
            return None;
        }

        let click_offset = (click_row - inner_y) as usize;
        let visible_height = inner_height as usize;

        // Use pinned scroll offset if set for this column
        let pinned = state.browse_scroll_pin.and_then(|(pc, po)| if pc == col_idx { Some(po) } else { None });

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

/// Hit-test a click against Miller column layout for StationNavigationState.
fn station_hit_test(
    click_col: u16,
    click_row: u16,
    state: &AppState,
) -> Option<(usize, usize, usize)> {
    let (area_x, area_y, area_width, area_height) = miller_area(state);

    if click_row < area_y || click_row >= area_y + area_height {
        return None;
    }

    let num_columns = state.station_nav.columns.len();
    if num_columns == 0 {
        return None;
    }

    let last_meaningful = (0..num_columns)
        .rev()
        .find(|&i| !state.station_nav.columns[i].stations.is_empty() || i <= state.station_nav.focused_column)
        .unwrap_or(0);
    let effective_columns = last_meaningful + 1;
    let max_visible = 3.min(effective_columns);
    let col_width = if max_visible > 0 { area_width / max_visible as u16 } else { return None; };
    let start_col = if state.station_nav.focused_column + 1 > max_visible {
        state.station_nav.focused_column + 1 - max_visible
    } else {
        0
    };

    for vis_idx in 0..max_visible {
        let col_idx = start_col + vis_idx;
        if col_idx >= effective_columns || col_idx >= state.station_nav.columns.len() {
            continue;
        }

        let col_x = area_x + (vis_idx as u16 * col_width);
        let col_w = if vis_idx == max_visible - 1 {
            area_width - (vis_idx as u16 * col_width)
        } else {
            col_width
        };

        if click_col < col_x || click_col >= col_x + col_w {
            continue;
        }

        let col = &state.station_nav.columns[col_idx];
        if col.stations.is_empty() {
            return None;
        }

        let inner_y = area_y + 1;
        let inner_height = area_height.saturating_sub(2);

        if click_row < inner_y || click_row >= inner_y + inner_height {
            return None;
        }

        let click_offset = (click_row - inner_y) as usize;
        let visible_height = inner_height as usize;

        // Use pinned scroll offset if set for this column
        let pinned = state.browse_scroll_pin.and_then(|(pc, po)| if pc == col_idx { Some(po) } else { None });

        // Check if filter is active on this station column with actual results
        let filter_on_col = state.list_filter.active
            && state.list_filter.column == col_idx
            && (state.list_filter.category == BrowseCategory::Genres
                || state.list_filter.category == BrowseCategory::Playlists)
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

        let total_items = col.stations.len();
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
fn miller_column_at(click_col: u16, nav: &BrowseNavigationState, state: &AppState) -> Option<usize> {
    let (area_x, _, area_width, _) = miller_area(state);
    let (max_visible, col_width, start_col, effective_columns) =
        browse_column_layout(nav, area_width);

    if max_visible == 0 || col_width == 0 {
        return None;
    }

    for vis_idx in 0..max_visible {
        let col_idx = start_col + vis_idx;
        if col_idx >= effective_columns || col_idx >= nav.columns.len() {
            continue;
        }

        let col_x = area_x + (vis_idx as u16 * col_width);
        let col_w = if vis_idx == max_visible - 1 {
            area_width - (vis_idx as u16 * col_width)
        } else {
            col_width
        };

        if click_col >= col_x && click_col < col_x + col_w {
            return Some(col_idx);
        }
    }

    None
}

// ============================================================================
// Browse Click Handlers
// ============================================================================

/// Handle click in Browse view using Miller column hit-testing.
fn handle_browse_click(click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
    // Determine whether we're in stations mode
    let is_stations = matches!(
        (&state.browse_category, &state.genre_content_type, &state.playlists_mode),
        (BrowseCategory::Genres, crate::app::state::GenreContentType::Stations, _)
        | (BrowseCategory::Playlists, _, crate::app::state::PlaylistsMode::Stations)
    );

    if is_stations {
        return handle_station_click(click_row, click_col, state);
    }

    match state.browse_category {
        BrowseCategory::Folders => {
            return handle_folder_click(click_row, click_col, state);
        }
        BrowseCategory::Artists | BrowseCategory::Genres | BrowseCategory::Playlists => {
            // All use BrowseNavigationState
            let nav = match state.browse_category {
                BrowseCategory::Artists => &state.artist_nav,
                BrowseCategory::Genres => &state.genre_nav,
                BrowseCategory::Playlists => &state.playlist_nav,
                _ => unreachable!(),
            };

            if let Some((col_idx, item_idx, scroll_offset)) = miller_hit_test(click_col, click_row, nav, state) {
                let nav = match state.browse_category {
                    BrowseCategory::Artists => &mut state.artist_nav,
                    BrowseCategory::Genres => &mut state.genre_nav,
                    BrowseCategory::Playlists => &mut state.playlist_nav,
                    _ => unreachable!(),
                };

                // If clicking a different column, change focus
                if col_idx != nav.focused_column {
                    // Pin viewport so the clicked column doesn't re-center
                    state.browse_scroll_pin = Some((col_idx, scroll_offset));
                    state.browse_click_time = Some(std::time::Instant::now());
                    // Pop to the clicked column (truncate right)
                    nav.focused_column = col_idx;
                    nav.truncate_right();
                    if let Some(col) = nav.columns.get_mut(col_idx) {
                        col.selected_index = item_idx;
                    }
                    // Sync filter selection so SelectFilteredItem drills into the right item
                    if state.list_filter.active
                        && state.list_filter.category == state.browse_category
                        && state.list_filter.column == col_idx
                    {
                        if let Some(ref results) = state.list_filter.results {
                            if let Some(pos) = results.matched_indices.iter().position(|&idx| idx == item_idx) {
                                state.list_filter.selected = pos;
                            }
                        }
                    }
                    return vec![];
                }

                // Same column: check for drill-down (click already-selected item)
                let already_selected = nav.columns.get(col_idx)
                    .map(|c| c.selected_index == item_idx)
                    .unwrap_or(false);

                if already_selected {
                    // Drill down - preserve parent column viewport position
                    state.browse_scroll_pin = Some((col_idx, scroll_offset));
                    state.browse_click_time = Some(std::time::Instant::now());
                    // When filter is active with results, use SelectFilteredItem for proper drill-down
                    let filter_on_col = state.list_filter.active
                        && state.list_filter.category == state.browse_category
                        && state.list_filter.column == col_idx
                        && state.list_filter.results.is_some();
                    if filter_on_col {
                        return vec![Action::SelectFilteredItem];
                    }
                    if let Some(item) = nav.columns.get(col_idx).and_then(|c| c.items.get(item_idx)).cloned() {
                        return browse_drill_down_action(item, col_idx, item_idx, state);
                    }
                } else {
                    // Pin the current scroll offset so the viewport doesn't jump
                    state.browse_scroll_pin = Some((col_idx, scroll_offset));
                    state.browse_click_time = Some(std::time::Instant::now());

                    // Select the item, truncate columns to the right
                    if let Some(col) = nav.columns.get_mut(col_idx) {
                        col.selected_index = item_idx;
                    }
                    nav.truncate_right();

                    // Update filter selection if filter is active on this column
                    if state.list_filter.active
                        && state.list_filter.category == state.browse_category
                        && state.list_filter.column == col_idx
                    {
                        if let Some(ref results) = state.list_filter.results {
                            if let Some(pos) = results.matched_indices.iter().position(|&idx| idx == item_idx) {
                                state.list_filter.selected = pos;
                            }
                        }
                    }
                }
            }
        }
    }

    vec![]
}

/// Return the drill-down action for clicking an already-selected item in a browse Miller column.
fn browse_drill_down_action(item: BrowseItem, col_idx: usize, item_idx: usize, state: &mut AppState) -> Vec<Action> {
    match state.browse_category {
        BrowseCategory::Artists => {
            match item {
                BrowseItem::Artist { key, title } => {
                    state.selected_artist_name = title;
                    vec![Action::LoadArtistAlbumsForMiller { artist_key: key }]
                }
                BrowseItem::Album { key, title, .. } => {
                    state.selected_album_title = title;
                    vec![Action::LoadAlbumTracksForMiller { album_key: key }]
                }
                BrowseItem::AllTracks { artist_key, artist_name, .. } => {
                    state.selected_album_title = format!("All tracks by {}", artist_name);
                    vec![Action::LoadArtistAllTracksForMiller { artist_key }]
                }
                BrowseItem::Track { .. } => {
                    vec![Action::PlayTrackFromMiller { column_index: col_idx, track_index: item_idx }]
                }
                _ => vec![],
            }
        }
        BrowseCategory::Genres => {
            match item {
                BrowseItem::Genre { key, .. } => {
                    vec![Action::LoadGenreAlbumsForMiller { genre_key: key }]
                }
                BrowseItem::Album { key, .. } => {
                    vec![Action::LoadGenreTracksForMiller { album_key: key }]
                }
                BrowseItem::Track { .. } => {
                    vec![Action::PlayGenreTrackFromMiller { column_index: col_idx, track_index: item_idx }]
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
                    state.selected_album_title = title;
                    vec![Action::LoadAlbumTracksForPlaylistMiller { album_key: key }]
                }
                BrowseItem::Track { .. } => {
                    vec![Action::PlayPlaylistTrackFromMiller { column_index: col_idx, track_index: item_idx }]
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
        let folder_state = state.folder_state.as_mut().unwrap();

        // If clicking a different column, change focus
        if col_idx != folder_state.focused_column {
            // Pin viewport so the clicked column doesn't re-center
            state.browse_scroll_pin = Some((col_idx, scroll_offset));
            state.browse_click_time = Some(std::time::Instant::now());
            folder_state.focused_column = col_idx;
            folder_state.truncate_right_columns();
            if let Some(col) = folder_state.columns.get_mut(col_idx) {
                col.selected_index = item_idx;
            }
            // Sync filter selection
            if state.list_filter.active
                && state.list_filter.category == BrowseCategory::Folders
                && state.list_filter.column == col_idx
            {
                if let Some(ref results) = state.list_filter.results {
                    if let Some(pos) = results.matched_indices.iter().position(|&idx| idx == item_idx) {
                        state.list_filter.selected = pos;
                    }
                }
            }
            return vec![];
        }

        // Same column: check for drill-down
        let already_selected = folder_state.columns.get(col_idx)
            .map(|c| c.selected_index == item_idx)
            .unwrap_or(false);

        if already_selected {
            // Drill down - preserve parent column viewport position
            state.browse_scroll_pin = Some((col_idx, scroll_offset));
            state.browse_click_time = Some(std::time::Instant::now());
            // When filter is active with results, use SelectFilteredItem for drill-down
            let filter_on_col = state.list_filter.active
                && state.list_filter.category == BrowseCategory::Folders
                && state.list_filter.column == col_idx
                && state.list_filter.results.is_some();
            if filter_on_col {
                return vec![Action::SelectFilteredItem];
            }
            if let Some(item) = folder_state.columns.get(col_idx)
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
        } else {
            // Pin the current scroll offset so the viewport doesn't jump
            state.browse_scroll_pin = Some((col_idx, scroll_offset));
            state.browse_click_time = Some(std::time::Instant::now());

            if let Some(col) = folder_state.columns.get_mut(col_idx) {
                col.selected_index = item_idx;
            }
            folder_state.truncate_right_columns();

            // Update filter selection if active on this column
            if state.list_filter.active
                && state.list_filter.category == BrowseCategory::Folders
                && state.list_filter.column == col_idx
            {
                if let Some(ref results) = state.list_filter.results {
                    if let Some(pos) = results.matched_indices.iter().position(|&idx| idx == item_idx) {
                        state.list_filter.selected = pos;
                    }
                }
            }
        }
    }

    vec![]
}

/// Handle click in Station browse mode.
fn handle_station_click(click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
    if let Some((col_idx, item_idx, scroll_offset)) = station_hit_test(click_col, click_row, state) {
        // If clicking a different column, change focus
        if col_idx != state.station_nav.focused_column {
            // Pin viewport so the clicked column doesn't re-center
            state.browse_scroll_pin = Some((col_idx, scroll_offset));
            state.browse_click_time = Some(std::time::Instant::now());
            state.station_nav.focused_column = col_idx;
            state.station_nav.truncate_right_columns();
            if let Some(col) = state.station_nav.columns.get_mut(col_idx) {
                col.selected_index = item_idx;
            }
            // Update legacy state
            if let Some(col) = state.station_nav.focused() {
                state.stations = col.stations.clone();
            }
            // Sync filter selection
            let filter_on_col = state.list_filter.active
                && state.list_filter.column == col_idx
                && (state.list_filter.category == BrowseCategory::Genres
                    || state.list_filter.category == BrowseCategory::Playlists);
            if filter_on_col {
                if let Some(ref results) = state.list_filter.results {
                    if let Some(pos) = results.matched_indices.iter().position(|&idx| idx == item_idx) {
                        state.list_filter.selected = pos;
                    }
                }
            }
            return vec![];
        }

        // Same column: check for drill-down
        let already_selected = state.station_nav.columns.get(col_idx)
            .map(|c| c.selected_index == item_idx)
            .unwrap_or(false);

        if already_selected {
            // Drill down - preserve parent column viewport position
            state.browse_scroll_pin = Some((col_idx, scroll_offset));
            state.browse_click_time = Some(std::time::Instant::now());
            // When filter is active with results, use SelectFilteredItem for drill-down
            let filter_on_col = state.list_filter.active
                && state.list_filter.column == col_idx
                && (state.list_filter.category == BrowseCategory::Genres
                    || state.list_filter.category == BrowseCategory::Playlists)
                && state.list_filter.results.is_some();
            if filter_on_col {
                return vec![Action::SelectFilteredItem];
            }
            if let Some(station) = state.station_nav.columns.get(col_idx)
                .and_then(|c| c.stations.get(item_idx)).cloned()
            {
                if station.is_category() {
                    return vec![Action::DrillIntoStation(station.key.clone(), station.title.clone())];
                } else {
                    return vec![Action::PlayStation(station.key.clone())];
                }
            }
        } else {
            // Pin the current scroll offset so the viewport doesn't jump
            state.browse_scroll_pin = Some((col_idx, scroll_offset));
            state.browse_click_time = Some(std::time::Instant::now());

            if let Some(col) = state.station_nav.columns.get_mut(col_idx) {
                col.selected_index = item_idx;
            }
            state.station_nav.truncate_right_columns();

            // Update filter selection if active on this column
            let filter_on_col = state.list_filter.active
                && state.list_filter.column == col_idx
                && (state.list_filter.category == BrowseCategory::Genres
                    || state.list_filter.category == BrowseCategory::Playlists);
            if filter_on_col {
                if let Some(ref results) = state.list_filter.results {
                    if let Some(pos) = results.matched_indices.iter().position(|&idx| idx == item_idx) {
                        state.list_filter.selected = pos;
                    }
                }
            }
        }
    }

    vec![]
}

/// Handle mouse down in Now Playing view.
fn handle_now_playing_down(click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
    use crate::app::state::NowPlayingMode;

    match state.now_playing_mode {
        NowPlayingMode::Queue => {
            // Queue mode layout: optional artwork (25 cols) + track list
            let artwork_width = if state.artwork_data.is_some() && state.terminal_width > 60 {
                25u16
            } else {
                0u16
            };

            // Click on title bar (row 0) of track list toggles shuffle
            if click_row == 0 && click_col >= artwork_width && !state.queue.is_empty() {
                return vec![Action::ToggleQueueShuffle];
            }

            // Track list starts after artwork
            if click_col >= artwork_width {
                // Visual row (accounting for border + 2-row layout per item)
                let visual_row = click_row.saturating_sub(1) as usize;
                let item_row = visual_row / 2;

                // Calculate visible item count (2 rows per item)
                let content_height = state.terminal_height.saturating_sub(5) as usize;
                let visible_item_count = content_height / 2;

                // Combined list: play_history + queue tracks
                let history_len = state.play_history.len();
                let tracks_len = if state.playback_mode == PlaybackMode::Radio {
                    state.radio.tracks.len()
                } else {
                    state.queue.len()
                };
                let total_len = history_len + tracks_len;

                // Match the renderer's scroll offset calculation:
                // display_selected = history_len + queue_index
                let selected = state.list_state.queue_index;
                let display_selected = history_len + selected;
                let scroll_offset = helpers::calc_scroll_offset(display_selected, visible_item_count, total_len);
                let actual_idx = item_row + scroll_offset;

                if actual_idx < total_len {
                    let current = state.list_state.queue_index;
                    if current == actual_idx {
                        // Double-click: if this is the currently playing track, switch to NowPlaying view
                        if actual_idx >= history_len {
                            let queue_idx = actual_idx - history_len;
                            let is_current = match state.playback_mode {
                                PlaybackMode::Queue | PlaybackMode::None => state.queue_index == Some(queue_idx),
                                PlaybackMode::Radio => state.radio.track_index == Some(queue_idx),
                            };
                            if is_current {
                                state.now_playing_mode = crate::app::state::NowPlayingMode::NowPlaying;
                                return vec![Action::LoadWaveform];
                            }
                            return vec![Action::JumpToQueueIndex(queue_idx)];
                        }
                    }
                    state.list_state.queue_index = actual_idx;
                }
            }
        }
        NowPlayingMode::NowPlaying => {
            // Visualizer mode: click to seek (enable drag only on indicator)
            let content_height = state.terminal_height.saturating_sub(3);
            let track_info_height = 5u16;
            let visualizer_top = track_info_height;
            let visualizer_bottom = content_height;
            let visualizer_inner_top = visualizer_top + 1;
            let visualizer_inner_bottom = visualizer_bottom.saturating_sub(1);
            let visualizer_inner_left = 1u16;
            let visualizer_inner_right = state.terminal_width.saturating_sub(1);

            // Check if click is within the visualizer inner area (for seeking)
            if click_row >= visualizer_inner_top
                && click_row < visualizer_inner_bottom
                && click_col >= visualizer_inner_left
                && click_col < visualizer_inner_right
                && state.playback.duration_ms > 0
            {
                let inner_width = visualizer_inner_right - visualizer_inner_left;

                // Calculate where the indicator currently is
                let progress = state.playback.position_ms as f64 / state.playback.duration_ms as f64;
                let indicator_col = visualizer_inner_left + (progress * inner_width as f64) as u16;

                // Check if click is on or near the indicator (within 2 chars)
                let on_indicator = click_col >= indicator_col.saturating_sub(2)
                    && click_col <= indicator_col.saturating_add(2);

                if on_indicator {
                    // Enable drag mode
                    state.seeking_drag = true;
                }

                // Always seek on click
                let relative_col = click_col - visualizer_inner_left;
                let seek_progress = relative_col as f64 / inner_width as f64;
                let seek_ms = (seek_progress * state.playback.duration_ms as f64) as u64;
                return vec![Action::Seek(seek_ms)];
            }
        }
    }

    vec![]
}

/// Handle mouse drag on the visualizer seekbar.
fn handle_visualizer_drag(click_col: u16, state: &AppState) -> Vec<Action> {
    if state.playback.duration_ms > 0 {
        let visualizer_inner_left = 1u16;
        let visualizer_inner_right = state.terminal_width.saturating_sub(1);
        let inner_width = visualizer_inner_right.saturating_sub(visualizer_inner_left);

        if inner_width > 0 {
            // Clamp to valid range for smoother feel at edges
            let clamped_col = click_col.max(visualizer_inner_left).min(visualizer_inner_right);
            let relative_col = clamped_col - visualizer_inner_left;
            let progress = (relative_col as f64 / inner_width as f64).clamp(0.0, 1.0);
            let seek_ms = (progress * state.playback.duration_ms as f64) as u64;
            return vec![Action::Seek(seek_ms)];
        }
    }
    vec![]
}

/// Handle click in Search view.
fn handle_search_click(click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
    // Search view is a centered popup: 60% width, 70% height
    // Calculate popup bounds
    let content_height = state.terminal_height.saturating_sub(3); // Minus transport + shortcuts
    let content_width = state.terminal_width;

    let popup_height = content_height * 70 / 100;
    let popup_width = content_width * 60 / 100;
    let popup_top = (content_height - popup_height) / 2;
    let popup_left = (content_width - popup_width) / 2;

    // Check if click is within popup
    if click_row < popup_top || click_row >= popup_top + popup_height {
        return vec![];
    }
    if click_col < popup_left || click_col >= popup_left + popup_width {
        return vec![];
    }

    // Convert to popup-relative coordinates
    let rel_row = click_row - popup_top;
    let rel_col = click_col - popup_left;

    // Popup layout: border (1) + tabs (2 rows) + search input (3 rows) + results
    // Tabs are at rows 1-2 (after top border at row 0)
    let tabs_start_row = 1u16;
    let tabs_end_row = 3u16;

    if rel_row >= tabs_start_row && rel_row < tabs_end_row {
        // Click is in tabs area
        // Tabs rendered with Ratatui Tabs widget: "All | Artists | Album Artists | ..."
        // Format: "tab1 | tab2 | tab3" with spaces around separators
        let tab_names = ["All", "Artists", "Albums", "Playlists", "Tracks", "Genres"];
        let tabs_with_enum = [
            SearchTab::Global,
            SearchTab::Artists,
            SearchTab::Albums,
            SearchTab::Playlists,
            SearchTab::Tracks,
            SearchTab::Genres,
        ];

        // Calculate tab positions (accounting for left border)
        let mut x: u16 = 1; // After left border
        for (i, name) in tab_names.iter().enumerate() {
            let tab_width = name.len() as u16;
            if rel_col >= x && rel_col < x + tab_width {
                state.search_tab = tabs_with_enum[i];
                return vec![];
            }
            x += tab_width;
            // Add separator width " | " = 3 chars
            if i < tab_names.len() - 1 {
                x += 3;
            }
        }
        return vec![];
    }

    // Results area starts after tabs (3) + search input (3) + border (1) = row 7
    let results_start_row = 7u16;
    if rel_row >= results_start_row {
        let result_row = (rel_row - results_start_row) as usize;

        // Handle based on current tab
        match state.search_tab {
            SearchTab::Global => {
                // 3-column layout: Artists | Albums | Tracks
                // Each column is roughly 1/3 of the popup inner width
                let inner_width = popup_width.saturating_sub(2); // Subtract borders
                let col_width = inner_width / 3;
                let rel_inner_col = rel_col.saturating_sub(1); // Subtract left border

                if let Some(ref results) = state.filter_results {
                    if rel_inner_col < col_width {
                        // Artists column
                        if result_row < results.artists.len() {
                            state.list_state.search_section = crate::app::state::SearchSection::Artists;
                            state.list_state.search_item_index = result_row;
                        }
                    } else if rel_inner_col < col_width * 2 {
                        // Albums column
                        if result_row < results.albums.len() {
                            state.list_state.search_section = crate::app::state::SearchSection::Albums;
                            state.list_state.search_item_index = result_row;
                        }
                    } else {
                        // Tracks column
                        if result_row < results.tracks.len() {
                            state.list_state.search_section = crate::app::state::SearchSection::Tracks;
                            state.list_state.search_item_index = result_row;
                        }
                    }
                }
            }
            SearchTab::Artists | SearchTab::AlbumArtists => {
                if let Some(ref results) = state.filter_results {
                    if result_row < results.artists.len() {
                        state.list_state.search_item_index = result_row;
                    }
                }
            }
            SearchTab::Albums => {
                if let Some(ref results) = state.filter_results {
                    if result_row < results.albums.len() {
                        state.list_state.search_item_index = result_row;
                    }
                }
            }
            SearchTab::Playlists => {
                if let Some(ref results) = state.filter_results {
                    if result_row < results.playlists.len() {
                        state.list_state.search_item_index = result_row;
                    }
                }
            }
            SearchTab::Tracks => {
                if let Some(ref results) = state.filter_results {
                    if result_row < results.tracks.len() {
                        state.list_state.search_item_index = result_row;
                    }
                }
            }
            SearchTab::Genres => {
                // Genres are filtered from state.genres
                let query_lower = state.search_query.to_lowercase();
                let filtered: Vec<_> = state.genres.iter()
                    .filter(|g| g.title.to_lowercase().contains(&query_lower))
                    .collect();
                if result_row < filtered.len() {
                    state.list_state.search_item_index = result_row;
                }
            }
        }
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
                        state.auth_state.server_index = server_index;
                        // Double-click or single click to select
                        return vec![Action::AuthSelectServer];
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

    // Account for top border
    let visual_row = click_row.saturating_sub(1) as usize;

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
        SettingsSection::About => {
            // Logo lines + blank + version + description + author + url + blank + "Theme:" header
            // Theme items are selectable
            // Count logo lines
            let logo_lines = crate::ui::screens::settings::ansi_logo_line_count();
            // logo + blank + version + description + author + url + blank + "Theme:" = logo + 7
            let theme_start = logo_lines + 7;
            let theme_count = crate::ui::theme::ThemeName::all().len();
            if visual_row >= theme_start && visual_row < theme_start + theme_count {
                Some(visual_row - theme_start)
            } else {
                None
            }
        }
    }
}

/// Handle click in Help view.
fn handle_help_click(click_row: u16, state: &mut AppState) -> Vec<Action> {
    // Help view is scrollable - clicking just sets focus, scroll wheel handles scrolling
    // Could implement click-to-scroll-to-position but for now just acknowledge the click
    let _ = click_row;
    let _ = state;
    vec![]
}

/// Handle scroll wheel events.
fn handle_scroll(up: bool, click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
    match state.view {
        View::Browse => {
            handle_browse_scroll(up, click_row, click_col, state);
        }
        View::NowPlaying => {
            // Scroll queue (includes play history + current tracks)
            let delta: i32 = if up { -1 } else { 1 };
            let tracks_len = if state.playback_mode == PlaybackMode::Radio {
                state.radio.tracks.len()
            } else {
                state.queue.len()
            };
            let total = state.play_history.len() + tracks_len;
            let max = total.saturating_sub(1);
            let new_idx = (state.list_state.queue_index as i32 + delta).clamp(0, max as i32) as usize;
            state.list_state.queue_index = new_idx;
        }
        View::Search => {
            // Search scrolling handled via keyboard for now
            // (requires proper handling of optional filter_results)
        }
        View::Help => {
            // Scroll help content
            let delta: i32 = if up { -1 } else { 1 };
            let new_scroll = (state.help_scroll as i32 + delta).max(0) as u16;
            state.help_scroll = new_scroll;
        }
        _ => {}
    }

    vec![]
}

/// Handle scroll in Browse view using Miller column awareness.
fn handle_browse_scroll(up: bool, click_row: u16, click_col: u16, state: &mut AppState) {
    // If a mouse click recently set the scroll pin, ignore scroll events
    // to prevent trackpad inertia from clearing the pin and re-centering.
    if let Some(click_time) = state.browse_click_time {
        if click_time.elapsed() < std::time::Duration::from_millis(400) {
            return;
        }
        state.browse_click_time = None;
    }
    // Clear scroll pin — scrolling should use fresh calc_scroll_offset
    state.browse_scroll_pin = None;
    // Determine whether we're in stations mode
    let is_stations = matches!(
        (&state.browse_category, &state.genre_content_type, &state.playlists_mode),
        (BrowseCategory::Genres, crate::app::state::GenreContentType::Stations, _)
        | (BrowseCategory::Playlists, _, crate::app::state::PlaylistsMode::Stations)
    );

    if is_stations {
        // Stations scroll
        if let Some((col_idx, _, _)) = station_hit_test(click_col, click_row, state) {
            let delta: i32 = if up { -1 } else { 1 };

            // Check if filter is active on this column
            let filter_on_col = state.list_filter.active
                && state.list_filter.column == col_idx
                && (state.list_filter.category == BrowseCategory::Genres
                    || state.list_filter.category == BrowseCategory::Playlists);

            if filter_on_col {
                if let Some(ref results) = state.list_filter.results {
                    if !results.matched_indices.is_empty() {
                        let max = results.matched_indices.len().saturating_sub(1);
                        let new_sel = (state.list_filter.selected as i32 + delta).clamp(0, max as i32) as usize;
                        state.list_filter.selected = new_sel;
                        if let Some(&item_idx) = results.matched_indices.get(new_sel) {
                            if let Some(col) = state.station_nav.columns.get_mut(col_idx) {
                                col.selected_index = item_idx;
                            }
                        }
                    }
                }
            } else {
                if let Some(col) = state.station_nav.columns.get_mut(col_idx) {
                    let max = col.stations.len().saturating_sub(1);
                    let new_idx = (col.selected_index as i32 + delta).clamp(0, max as i32) as usize;
                    col.selected_index = new_idx;
                }
                if col_idx != state.station_nav.focused_column {
                    state.station_nav.focused_column = col_idx;
                    state.station_nav.truncate_right_columns();
                }
            }
        }
        return;
    }

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
        BrowseCategory::Artists | BrowseCategory::Genres | BrowseCategory::Playlists => {
            let nav = match state.browse_category {
                BrowseCategory::Artists => &state.artist_nav,
                BrowseCategory::Genres => &state.genre_nav,
                BrowseCategory::Playlists => &state.playlist_nav,
                _ => unreachable!(),
            };

            if let Some(col_idx) = miller_column_at(click_col, nav, state) {
                // Check if column has albums and cover art is active (scroll 1 at a time)
                let has_albums = nav.columns.get(col_idx)
                    .map(|c| c.items.iter().any(|item| matches!(item, BrowseItem::Album { .. })))
                    .unwrap_or(false);
                let is_art_scroll = state.album_art_view && has_albums;

                // Throttle cover art scrolling to prevent trackpad momentum
                if is_art_scroll {
                    let now = std::time::Instant::now();
                    if let Some(last) = state.art_scroll_cooldown {
                        if now.duration_since(last).as_millis() < 120 {
                            return;
                        }
                    }
                    state.art_scroll_cooldown = Some(now);
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
                                    BrowseCategory::Artists => &mut state.artist_nav,
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
                        BrowseCategory::Artists => &mut state.artist_nav,
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
                }
            }
        }
    }
}
