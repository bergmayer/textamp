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

    // Search popup intercepts mouse clicks when active
    if state.search_popup_active {
        if let crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) = event.kind {
            return handle_search_popup_click(click_row, click_col, state);
        }
        return vec![];
    }

    // Artist radio picker popup intercepts mouse clicks when active
    if state.artist_radio_picker.is_some() {
        if let crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) = event.kind {
            return handle_artist_radio_picker_click(click_row, click_col, state);
        }
        return vec![];
    }

    // Adventure launcher popup intercepts mouse clicks when active
    if state.adventure_launcher.is_some() {
        if let crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) = event.kind {
            return handle_adventure_launcher_click(click_row, click_col, state);
        }
        return vec![];
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
                View::Queue | View::NowPlaying => {
                    return handle_now_playing_down(click_row, click_col, event.modifiers, state);
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
                View::Similar => {
                    return handle_similar_click(click_row, state);
                }
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
        state.search_scroll_pin = None;
        if !state.search_query.is_empty() {
            return vec![Action::ExecuteLocalSearch];
        }
    }
    vec![]
}

/// Shared: handle a click in the search results area.
fn handle_search_result_click(visual_row: usize, results_height: usize, state: &mut AppState) {
    use crate::services::NavigationService;

    if visual_row >= results_height {
        return;
    }

    let Some(ref results) = state.search_results else { return };

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
            let scroll_offset = match state.search_scroll_pin {
                Some(pinned) => pinned,
                None => NavigationService::calc_scroll_offset(
                    display_selected, results_height, entries.len(),
                ),
            };
            let abs_row = scroll_offset + visual_row;
            if let Some(Some(idx)) = entries.get(abs_row) {
                state.search_focus = crate::app::state::SearchFocus::Results;
                state.list_state.search_item_index = *idx;
                state.search_scroll_pin = Some(scroll_offset);
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
            let scroll_offset = match state.search_scroll_pin {
                Some(pinned) => pinned,
                None => NavigationService::calc_scroll_offset(
                    state.list_state.search_item_index, results_height, total,
                ),
            };
            let actual_idx = scroll_offset + visual_row;
            if actual_idx < total {
                state.search_focus = crate::app::state::SearchFocus::Results;
                state.list_state.search_item_index = actual_idx;
                state.search_scroll_pin = Some(scroll_offset);
            }
        }
    }
}

/// Handle mouse click when the search popup is active.
fn handle_search_popup_click(click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
    use ratatui::layout::Rect;

    // Match the single centered_rect(50, 70, frame.area()) used by filter::render
    let frame_area = Rect::new(0, 0, state.terminal_width, state.terminal_height);
    let popup = centered_rect(50, 70, frame_area);

    // Check if click is outside popup — close it
    if click_row < popup.y || click_row >= popup.y + popup.height
        || click_col < popup.x || click_col >= popup.x + popup.width
    {
        state.search_query.clear();
        state.search_results = None;
        state.search_focus = crate::app::state::SearchFocus::Input;
        return vec![Action::CloseSearchPopup];
    }

    // Inner area (inside block border)
    let inner_x = popup.x + 1;
    let inner_y = popup.y + 1;
    let _inner_width = popup.width.saturating_sub(2);
    let _inner_height = popup.height.saturating_sub(2);

    // Convert to popup-inner-relative coordinates
    let rel_row = click_row.saturating_sub(inner_y);
    let rel_col = click_col.saturating_sub(inner_x);

    // Layout inside popup: tabs (2 rows) | search input (3 rows) | results
    // Tabs are at rel_row 0-1
    if rel_row < 2 {
        let actions = handle_search_tab_click(rel_col, state);
        if !actions.is_empty() {
            return actions;
        }
        return vec![];
    }

    // Search input area: rel_row 2-4
    if rel_row >= 2 && rel_row < 5 {
        state.search_focus = crate::app::state::SearchFocus::Input;
        return vec![];
    }

    // Results area: rel_row 5+ (tabs=2 + search_input=3)
    if rel_row >= 5 {
        let visual_row = (rel_row - 5) as usize;
        let results_height = popup.height.saturating_sub(2 + 2 + 3) as usize;
        handle_search_result_click(visual_row, results_height, state);
    }

    vec![]
}

/// Handle click on the shortcut bar at the bottom.
fn handle_shortcut_bar_click(click_col: u16, state: &AppState) -> Vec<Action> {
    // Shortcut bar items (must match render_shortcuts in ui/app.rs):
    // ^L library | ^P playlists | ^G genres | ^O folders | ^N queue
    //
    // These are centered, so we need to calculate positions based on terminal width.
    // Each item is roughly: " ^X label " with separators "|"
    //
    // Clicking an already-active item cycles its mode (like the keyboard shortcut does).

    let shortcuts: [(&str, &str, usize); 6] = [
        ("^L", "library", 0),                       // Library
        ("^P", "playlists", 1),                     // Playlists
        ("^G", "genres", 2),                        // Genres
        ("^O", "folders", 3),                       // Folders
        ("^U", "queue", 4),                         // Queue
        ("^N", "now playing", 5),                   // Now Playing
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
fn shortcut_bar_action(idx: usize, _state: &AppState) -> Vec<Action> {
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
    // Genres has a 1-row tab bar above the content
    let has_tab_bar = matches!(state.browse_category, BrowseCategory::Genres);
    let tab_offset = if has_tab_bar { 1u16 } else { 0 };
    let area_y = tab_offset;
    let area_width = state.terminal_width;
    let area_height = state.terminal_height.saturating_sub(3).saturating_sub(tab_offset);
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

        // Check if this column has albums/artists and the appropriate art view is active
        let has_albums = col.items.iter().any(|item| matches!(item, BrowseItem::Album { .. }));
        let has_artists = col.items.iter().any(|item| matches!(item, BrowseItem::Artist { .. }));
        if (state.album_art_view && has_albums) || (state.artist_art_view && has_artists) {
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
            // Check if this column uses 2-row display (playlist tracks or All Artists albums)
            let is_two_row_track = state.browse_category == BrowseCategory::Playlists
                && col.items.first().map_or(false, |item| matches!(item, BrowseItem::Track { .. }));
            let is_all_artists_albums = col.items.first().map_or(false, |item| matches!(item, BrowseItem::Album { .. }))
                && (nav.columns.first()
                    .and_then(|c| c.selected_item())
                    .map_or(false, |item| matches!(item, BrowseItem::AllArtists))
                || (state.browse_category == BrowseCategory::Library
                    && state.library_sub_mode != crate::app::state::LibrarySubMode::Normal
                    && col_idx == 0));
            let rows_per_item = if is_two_row_track || is_all_artists_albums { 2 } else { 1 };

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
    // Tab bar click handling for Genres (row 0 of content area)
    if click_row == 0 {
        if state.browse_category == BrowseCategory::Genres {
            // Genre tab bar: "All | Library | Artist | Album | Mood | Style"
            let tab_labels = [" all ", " library ", " artist ", " album ", " mood ", " style "];
            if let Some(tab_idx) = tab_hit_test(click_col, &tab_labels) {
                use crate::app::state::GenreTab;
                let new_tab = match tab_idx {
                    0 => GenreTab::All,
                    1 => GenreTab::Library,
                    2 => GenreTab::Artist,
                    3 => GenreTab::Album,
                    4 => GenreTab::Mood,
                    _ => GenreTab::Style,
                };
                if new_tab != state.genre_tab {
                    return vec![Action::SetGenreTab(new_tab)];
                }
            }
            return vec![];
        }
    }

    // Column header click: cycle view mode (same as Alt+V)
    // Header is at area_y (row 0 for Library/Playlists, row 1 for Genres due to tab bar)
    let (_, area_y, _, _) = miller_area(state);
    if click_row == area_y && state.browse_category != BrowseCategory::Folders {
        let nav = match state.browse_category {
            BrowseCategory::Library => &state.artist_nav,
            BrowseCategory::Genres => &state.genre_nav,
            BrowseCategory::Playlists => &state.playlist_nav,
            _ => return vec![],
        };
        if let Some(col_idx) = miller_column_at(click_col, nav, state) {
            // Skip playlist root column (no need to cycle playlist listing)
            if state.browse_category == BrowseCategory::Playlists && col_idx == 0 {
                return vec![];
            }
            let nav_mut = match state.browse_category {
                BrowseCategory::Library => &mut state.artist_nav,
                BrowseCategory::Genres => &mut state.genre_nav,
                BrowseCategory::Playlists => &mut state.playlist_nav,
                _ => return vec![],
            };
            nav_mut.focused_column = col_idx;
            return super::key_input::handle_cycle_view(state);
        }
        return vec![];
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

            if let Some((col_idx, item_idx, scroll_offset)) = miller_hit_test(click_col, click_row, nav, state) {
                let nav = match state.browse_category {
                    BrowseCategory::Library => &mut state.artist_nav,
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
                    state.selected_album_title = format!("All tracks by {}", artist_name);
                    vec![Action::LoadArtistAllTracksForMiller { artist_key }]
                }
                BrowseItem::AllArtists => {
                    vec![Action::LoadAllAlbumsForMiller]
                }
                BrowseItem::Compilations => {
                    vec![Action::LoadCompilationsForMiller]
                }
                BrowseItem::CompilationTracks { artist_key, artist_name } => {
                    vec![Action::LoadCompilationTracksForMiller { artist_key, artist_name }]
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
                    // Reset album grouping when selecting a new playlist
                    state.playlist_view_mode = crate::app::state::PlaylistViewMode::Tracks;
                    state.playlist_album_groups.clear();
                    state.playlist_original_items = None;
                    state.playlist_original_tracks = None;
                    vec![Action::LoadPlaylistTracksForMiller { playlist_key: key }]
                }
                BrowseItem::Album { .. }
                    if state.playlist_view_mode == crate::app::state::PlaylistViewMode::TracksByAlbum =>
                {
                    // Drill into playlist album group
                    if let Some(tracks) = state.playlist_album_groups.get(item_idx) {
                        let items = BrowseItem::from_tracks(tracks);
                        let title = match &state.playlist_nav.columns.get(col_idx)
                            .and_then(|c| c.items.get(item_idx))
                        {
                            Some(BrowseItem::Album { title, .. }) => title.clone(),
                            _ => "tracks".to_string(),
                        };
                        let new_col = crate::app::state::BrowseColumn::new_with_tracks(
                            title, items, tracks.clone(),
                        );
                        state.playlist_nav.push_column(new_col);
                    }
                    vec![]
                }
                BrowseItem::Album { key, title, .. } => {
                    state.selected_album_title = title;
                    vec![Action::LoadAlbumTracksForMiller { album_key: key }]
                }
                BrowseItem::Track { .. } => {
                    // When playing from a TracksByAlbum drill-down, queue all grouped tracks
                    if state.playlist_view_mode == crate::app::state::PlaylistViewMode::TracksByAlbum
                        && !state.playlist_album_groups.is_empty()
                    {
                        let parent_col_idx = col_idx.saturating_sub(1);
                        let album_group_idx = state.playlist_nav.columns.get(parent_col_idx)
                            .map(|c| c.selected_index)
                            .unwrap_or(0);
                        let offset: usize = state.playlist_album_groups.iter()
                            .take(album_group_idx)
                            .map(|g| g.len())
                            .sum();
                        let abs_idx = offset + item_idx;
                        vec![Action::PlayPlaylistAlbumGroupTrack { track_index: abs_idx }]
                    } else {
                        vec![Action::PlayPlaylistTrackFromMiller { column_index: col_idx, track_index: item_idx }]
                    }
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
        let Some(folder_state) = state.folder_state.as_mut() else {
            return vec![];
        };

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

/// Handle mouse down in Queue or NowPlaying view.
fn handle_now_playing_down(click_row: u16, click_col: u16, modifiers: crossterm::event::KeyModifiers, state: &mut AppState) -> Vec<Action> {
    use crate::app::state::NowPlayingFocus;

    match state.view {
        View::Queue => {
            // Queue mode layout: left column (artwork + stations) + right column (track list)
            // Must match render_queue_mode layout in now_playing.rs
            let content_height = state.terminal_height.saturating_sub(3);
            let art_height = (content_height * 40 / 100).max(8);
            let art_width = (art_height * 2).min(state.terminal_width * 40 / 100).max(25);

            // Click on clear button row (left column, between artwork and stations)
            if click_col < art_width && click_row == art_height {
                return vec![Action::ClearQueue];
            }

            // Click in station panel area (left column, below artwork + clear button)
            let station_top = art_height + 1;
            if click_col < art_width && click_row >= station_top {
                state.now_playing_focus = NowPlayingFocus::Stations;
                // Map click to station item (accounting for border)
                let inner_top = station_top + 1;
                let inner_bottom = content_height.saturating_sub(1);
                if click_row >= inner_top && click_row < inner_bottom {
                    let click_offset = (click_row - inner_top) as usize;
                    let visible_height = (inner_bottom - inner_top) as usize;
                    let (already_selected, item_idx) = if let Some(col) = state.station_nav.focused() {
                        let scroll_offset = helpers::calc_scroll_offset(
                            col.selected_index, visible_height, col.stations.len(),
                        );
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
                                if station.is_category() {
                                    return vec![Action::DrillIntoStation(station.key.clone(), station.title.clone())];
                                }
                                return vec![Action::PlayStation(station.key.clone())];
                            }
                        } else {
                            // Pin scroll offset before changing selection to prevent viewport jump
                            if let Some(col) = state.station_nav.focused() {
                                let current_offset = helpers::calc_scroll_offset(
                                    col.selected_index, visible_height, col.stations.len(),
                                );
                                state.station_scroll_pin = Some(current_offset);
                            }
                            if let Some(col) = state.station_nav.focused_mut() {
                                col.selected_index = idx;
                            }
                        }
                    }
                }
                return vec![];
            }

            // Click on title bar (row 0) of track list toggles shuffle
            if click_row == 0 && click_col >= art_width && !state.queue.is_empty() {
                return vec![Action::ToggleQueueShuffle];
            }

            // Track list area (right column)
            if click_col >= art_width {
                state.now_playing_focus = NowPlayingFocus::Tracks;
                // Visual row (accounting for border + 2-row layout per item)
                let visual_row = click_row.saturating_sub(1) as usize;
                let item_row = visual_row / 2;

                // Calculate visible item count (2 rows per item)
                let content_height_usize = content_height.saturating_sub(2) as usize;
                let visible_item_count = content_height_usize / 2;

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
                let scroll_offset = match state.queue_scroll_pin {
                    Some(pinned) => pinned,
                    None => helpers::calc_scroll_offset(display_selected, visible_item_count, total_len),
                };
                let actual_idx = item_row + scroll_offset;

                if actual_idx < total_len {
                    // Shift+Click: toggle multi-select (queue items only)
                    if modifiers.contains(crossterm::event::KeyModifiers::SHIFT) && actual_idx >= history_len {
                        let queue_idx = actual_idx - history_len;
                        if state.queue_selected.contains(&queue_idx) {
                            state.queue_selected.remove(&queue_idx);
                        } else {
                            state.queue_selected.insert(queue_idx);
                        }
                        state.queue_scroll_pin = Some(scroll_offset);
                        state.list_state.queue_index = actual_idx;
                        return vec![];
                    }

                    // Normal click: clear multi-select
                    if !state.queue_selected.is_empty() {
                        state.queue_selected.clear();
                    }

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
                                return vec![Action::SetView(View::NowPlaying), Action::LoadWaveform];
                            }
                            return match state.playback_mode {
                                PlaybackMode::Radio => vec![Action::JumpToRadioTrack(queue_idx)],
                                _ => vec![Action::JumpToQueueIndex(queue_idx)],
                            };
                        }
                    }
                    state.queue_scroll_pin = Some(scroll_offset);
                    state.list_state.queue_index = actual_idx;
                }
            }
        }
        View::NowPlaying => {
            // Visualizer mode layout depends on whether artwork is shown
            let content_height = state.terminal_height.saturating_sub(3);
            let show_artwork = state.artwork_data.is_some() && state.terminal_width > 50;

            let visualizer_top = if show_artwork {
                // Top panel: 40% of height (min 12) for artwork + track info
                (content_height * 40 / 100).max(12)
            } else {
                // Narrow layout: 5 rows for track info
                5u16
            };
            let visualizer_bottom = content_height;
            let visualizer_inner_top = visualizer_top + 1; // After top border
            let visualizer_inner_bottom = visualizer_bottom.saturating_sub(1);
            let visualizer_inner_left = 1u16;
            let visualizer_inner_right = state.terminal_width.saturating_sub(1);

            // Tab bar is the first row of the visualizer inner area
            let tab_bar_row = visualizer_inner_top;

            // Check if click is on the tab bar row
            if click_row == tab_bar_row
                && click_col >= visualizer_inner_left
                && click_col < visualizer_inner_right
            {
                // Tab bar uses Tabs widget: " waveform  │  spectrum  │  spectrogram "
                let rel_col = click_col - visualizer_inner_left;
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

            // Content area starts after tab bar row
            let content_top = tab_bar_row + 1;

            // Check if click is within the visualizer content area (for seeking)
            if click_row >= content_top
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
        _ => {}
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
    use ratatui::layout::Rect;

    // Use the same layout calculation as the renderer for accurate bounds
    let content_area = Rect::new(0, 0, state.terminal_width, state.terminal_height.saturating_sub(3));
    let popup_area = centered_rect(50, 70, content_area);

    // Check if click is within popup
    if click_row < popup_area.y || click_row >= popup_area.y + popup_area.height {
        return vec![];
    }
    if click_col < popup_area.x || click_col >= popup_area.x + popup_area.width {
        return vec![];
    }

    // Convert to popup-relative coordinates
    let rel_row = click_row - popup_area.y;
    let rel_col = click_col - popup_area.x;

    // Popup layout (rel_row):
    //   0: top border
    //   1-2: tabs (Length 2)
    //   3-5: search input (Length 3)
    //   6+: results
    //   last: bottom border

    // Tabs area (rel_rows 1-2)
    if rel_row >= 1 && rel_row < 3 {
        let inner_col = rel_col.saturating_sub(1);
        let actions = handle_search_tab_click(inner_col, state);
        if !actions.is_empty() {
            return actions;
        }
        return vec![];
    }

    // Results area starts at rel_row 6 (border=1 + tabs=2 + search=3)
    if rel_row >= 6 {
        let visual_row = (rel_row - 6) as usize;
        let results_height = popup_area.height.saturating_sub(2 + 2 + 3) as usize;
        handle_search_result_click(visual_row, results_height, state);
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
            let player_count = state.remote_players.len();

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
fn handle_help_click(click_row: u16, state: &mut AppState) -> Vec<Action> {
    // Help view is scrollable - clicking just sets focus, scroll wheel handles scrolling
    // Could implement click-to-scroll-to-position but for now just acknowledge the click
    let _ = click_row;
    let _ = state;
    vec![]
}

/// Handle click in Similar view.
fn handle_similar_click(click_row: u16, state: &mut AppState) -> Vec<Action> {
    use crate::app::state::SimilarMode;

    // Layout: border row at top (row 0), items start at row 1, border at bottom
    let content_height = state.terminal_height.saturating_sub(3); // nav bar + transport + shortcut
    if click_row == 0 || click_row >= content_height {
        return vec![];
    }

    let inner_row = (click_row - 1) as usize; // offset by top border
    let visible_height = content_height.saturating_sub(2) as usize; // minus top+bottom borders

    let total = match state.similar_mode {
        SimilarMode::Albums => state.similar_albums.len(),
        SimilarMode::Tracks => state.similar_tracks.len(),
    };

    if total == 0 {
        return vec![];
    }

    let scroll_offset = match state.similar_scroll_pin {
        Some(pinned) => pinned,
        None => helpers::calc_scroll_offset(state.list_state.similar_index, visible_height, total),
    };
    let clicked_idx = scroll_offset + inner_row;

    if clicked_idx >= total {
        return vec![];
    }

    if state.list_state.similar_index == clicked_idx {
        // Double-click: activate (same as Enter)
        match state.similar_mode {
            SimilarMode::Albums => {
                if let Some(album) = state.similar_albums.get(clicked_idx).cloned() {
                    state.pending_album_key = Some(album.rating_key.clone());
                    state.selected_album_title = album.title.clone();
                    state.selected_artist_name = album.artist_name().to_string();
                    state.view = View::Browse;
                    state.browse_category = BrowseCategory::Library;
                    if let Some(artist_key) = &album.parent_rating_key {
                        if let Some(idx) = state.artists.iter().position(|a| &a.rating_key == artist_key) {
                            state.list_state.artists_index = idx;
                        }
                    }
                    return vec![Action::LoadArtistAlbums];
                }
            }
            SimilarMode::Tracks => {
                if let Some(track) = state.similar_tracks.get(clicked_idx).cloned() {
                    return vec![Action::PlayTrack(track)];
                }
            }
        }
    } else {
        state.similar_scroll_pin = Some(scroll_offset);
        state.list_state.similar_index = clicked_idx;
    }

    vec![]
}

/// Handle scroll wheel events.
fn handle_scroll(up: bool, click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
    match state.view {
        View::Browse => {
            return handle_browse_scroll(up, click_row, click_col, state);
        }
        View::Queue => {
            // Check if scrolling in station panel area (left column)
            let content_height = state.terminal_height.saturating_sub(3);
            let art_height = (content_height * 40 / 100).max(8);
            let art_width = (art_height * 2).min(state.terminal_width * 40 / 100).max(25);

            let station_top = art_height + 1; // +1 for clear button row
            if click_col < art_width && click_row >= station_top {
                // Station panel scroll
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
            state.queue_scroll_pin = None;
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
        }
        View::Help => {
            // Scroll help content
            let delta: i32 = if up { -1 } else { 1 };
            let new_scroll = (state.help_scroll as i32 + delta).max(0) as u16;
            state.help_scroll = new_scroll;
        }
        View::Similar => {
            state.similar_scroll_pin = None;
            let delta: i32 = if up { -1 } else { 1 };
            let total = match state.similar_mode {
                crate::app::state::SimilarMode::Albums => state.similar_albums.len(),
                crate::app::state::SimilarMode::Tracks => state.similar_tracks.len(),
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
    if let Some(click_time) = state.browse_click_time {
        if click_time.elapsed() < std::time::Duration::from_millis(400) {
            return vec![];
        }
        state.browse_click_time = None;
    }
    // Clear scroll pin — scrolling should use fresh calc_scroll_offset
    state.browse_scroll_pin = None;

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
                // Check if column has albums and cover art is active (scroll 1 at a time)
                let has_albums = nav.columns.get(col_idx)
                    .map(|c| c.items.iter().any(|item| matches!(item, BrowseItem::Album { .. })))
                    .unwrap_or(false);
                let has_artists = nav.columns.get(col_idx)
                    .map(|c| c.items.iter().any(|item| matches!(item, BrowseItem::Artist { .. })))
                    .unwrap_or(false);
                let is_art_scroll = (state.album_art_view && has_albums) || (state.artist_art_view && has_artists);

                // Throttle cover art scrolling to prevent trackpad momentum
                if is_art_scroll {
                    let now = std::time::Instant::now();
                    if let Some(last) = state.art_scroll_cooldown {
                        if now.duration_since(last).as_millis() < 120 {
                            return vec![];
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
                }
            }
        }
    }

    // After scroll, lazily load album art for newly visible items
    let art_batch = super::dispatch_miller::collect_viewport_art(state);
    if !art_batch.is_empty() {
        return vec![Action::LoadAlbumArt(art_batch)];
    }
    vec![]
}

/// Handle mouse click when the artist radio picker popup is active.
fn handle_artist_radio_picker_click(click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
    use ratatui::layout::Rect;
    use crate::app::state::{ArtistRadioPickerStep, SearchFocus};

    let frame_area = Rect::new(0, 0, state.terminal_width, state.terminal_height);
    let popup = centered_rect(50, 70, frame_area);

    // Click outside popup — close
    if click_row < popup.y || click_row >= popup.y + popup.height
        || click_col < popup.x || click_col >= popup.x + popup.width
    {
        return vec![Action::CloseArtistRadioPicker];
    }

    let picker = match &state.artist_radio_picker {
        Some(p) => p,
        None => return vec![],
    };

    // Only handle clicks in SelectArtists step (results list)
    if !matches!(picker.step, ArtistRadioPickerStep::SelectArtists) {
        return vec![];
    }

    // Inner area (inside border)
    let inner_y = popup.y + 1;
    let inner_height = popup.height.saturating_sub(2);

    // Layout: selected (2) + input (3) + results (rest)
    let results_y = inner_y + 2 + 3;
    let results_height = inner_height.saturating_sub(5) as usize;

    if click_row < results_y || click_row >= results_y + results_height as u16 {
        return vec![];
    }

    let click_offset = (click_row - results_y) as usize;
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
    if let Some(ref mut picker) = state.artist_radio_picker {
        picker.scroll_pin = Some(scroll_offset);
        picker.item_index = clicked_idx;
        picker.focus = SearchFocus::Results;
    }
    vec![]
}

/// Handle mouse click when the adventure launcher popup is active.
fn handle_adventure_launcher_click(click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
    use ratatui::layout::Rect;
    use crate::app::state::{SearchFocus, AdventureDrillLevel};

    let frame_area = Rect::new(0, 0, state.terminal_width, state.terminal_height);
    let popup = centered_rect(50, 70, frame_area);

    // Click outside popup — close
    if click_row < popup.y || click_row >= popup.y + popup.height
        || click_col < popup.x || click_col >= popup.x + popup.width
    {
        return vec![Action::CloseAdventureLauncher];
    }

    let launcher = match &state.adventure_launcher {
        Some(l) => l,
        None => return vec![],
    };

    // Inner area (inside border)
    let inner_y = popup.y + 1;
    let inner_height = popup.height.saturating_sub(2);

    // Layout depends on drill level
    let (results_y, results_height) = match &launcher.drill {
        AdventureDrillLevel::Search => {
            // search input (3) + results (rest)
            let ry = inner_y + 3;
            let rh = inner_height.saturating_sub(3) as usize;
            (ry, rh)
        }
        AdventureDrillLevel::ArtistAlbums { .. } => {
            // breadcrumb (1) + album list (rest)
            let ry = inner_y + 1;
            let rh = inner_height.saturating_sub(1) as usize;
            (ry, rh)
        }
        AdventureDrillLevel::AlbumTracks { .. } => {
            // breadcrumb (1) + track list (rest)
            let ry = inner_y + 1;
            let rh = inner_height.saturating_sub(1) as usize;
            (ry, rh)
        }
    };

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
    if let Some(ref mut launcher) = state.adventure_launcher {
        launcher.scroll_pin = Some(scroll_offset);
        launcher.item_index = item_idx;
        launcher.focus = SearchFocus::Results;
    }
    vec![]
}
