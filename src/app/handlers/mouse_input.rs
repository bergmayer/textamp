//! Mouse input handler functions.
//!
//! All mouse event processing extracted from the event loop as free functions.

use crate::app::Action;
use crate::app::state::{
    BrowseCategory, Focus, PlaybackMode, RightPanelMode, SearchTab, View,
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
    let left_panel_width = 30u16;

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
                    // Left panel (categories)
                    if click_col < left_panel_width {
                        return handle_left_panel_click(click_row, state);
                    }
                    // Right panel (albums/tracks)
                    return handle_right_panel_click(click_row, click_col, state);
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
            return handle_scroll(true, click_col, state);
        }
        MouseEventKind::ScrollDown => {
            return handle_scroll(false, click_col, state);
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

    let shortcuts: [(&str, &str, usize); 7] = [
        ("^A", state.artist_view_mode.name(), 0),   // Artists
        ("^P", state.playlists_mode.name(), 1),     // Playlists
        ("^G", state.genre_content_type.name(), 2), // Genres (cycles through genres/moods/styles/stations)
        ("^O", "folders", 3),                       // Folders
        ("^N", state.now_playing_mode.name(), 4),   // Now Playing
        ("F1", "help", 5),                          // Help
        ("F2", "settings", 6),                      // Settings
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
            // Help
            vec![Action::SetView(View::Help)]
        }
        6 => {
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

    // Search emoji at far right (last 4 columns to account for emoji width)
    // Only activate filter in Browse view
    if state.view == View::Browse && click_col >= state.terminal_width.saturating_sub(4) {
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

/// Handle click on the left panel (category list).
fn handle_left_panel_click(click_row: u16, state: &mut AppState) -> Vec<Action> {
    // Left panel has a 1-row border at top
    // Visual row within the list (0-indexed from first visible item)
    let visual_row = click_row.saturating_sub(1) as usize;

    // Calculate visible height for left panel (30 width, content area height minus borders)
    let content_height = state.terminal_height.saturating_sub(4) as usize;
    let visible_height = content_height.saturating_sub(1);

    // Set focus to left panel
    state.focus = Focus::Left;

    // Update the appropriate index based on category
    match state.browse_category {
        BrowseCategory::Artists => {
            let len = state.category_len();
            let selected = state.category_index();
            let scroll_offset = helpers::calc_scroll_offset(selected, visible_height, len);
            let actual_idx = visual_row + scroll_offset;

            if actual_idx < len {
                state.set_category_index(actual_idx);
                return vec![Action::LoadArtistAlbums];
            }
        }
        BrowseCategory::Playlists => {
            if state.playlists_mode == crate::app::state::PlaylistsMode::Stations {
                // Stations mode - select item in station_nav focused column
                if let Some(column) = state.station_nav.columns.get(state.station_nav.focused_column) {
                    let len = column.stations.len();
                    let selected = column.selected_index;
                    let scroll_offset = helpers::calc_scroll_offset(selected, visible_height, len);
                    let actual_idx = visual_row + scroll_offset;

                    if actual_idx < len {
                        if let Some(col) = state.station_nav.columns.get_mut(state.station_nav.focused_column) {
                            col.selected_index = actual_idx;
                        }
                    }
                }
            } else {
                let len = state.category_len();
                let selected = state.category_index();
                let scroll_offset = helpers::calc_scroll_offset(selected, visible_height, len);
                let actual_idx = visual_row + scroll_offset;

                if actual_idx < len {
                    state.set_category_index(actual_idx);
                    return vec![Action::LoadCategoryTracks];
                }
            }
        }
        BrowseCategory::Genres => {
            // Stations are now part of the genre content type cycle
            if state.genre_content_type == crate::app::state::GenreContentType::Stations {
                // Stations use station_nav - select item in focused column
                if let Some(column) = state.station_nav.columns.get(state.station_nav.focused_column) {
                    let len = column.stations.len();
                    let selected = column.selected_index;
                    let scroll_offset = helpers::calc_scroll_offset(selected, visible_height, len);
                    let actual_idx = visual_row + scroll_offset;

                    if actual_idx < len {
                        if let Some(col) = state.station_nav.columns.get_mut(state.station_nav.focused_column) {
                            col.selected_index = actual_idx;
                        }
                    }
                }
            } else {
                let len = state.current_genre_list().len();
                let selected = state.genres_index;
                let scroll_offset = helpers::calc_scroll_offset(selected, visible_height, len);
                let actual_idx = visual_row + scroll_offset;

                if actual_idx < len {
                    state.genres_index = actual_idx;
                    // Load albums for this genre
                    return match state.genre_content_type {
                        crate::app::state::GenreContentType::Genres => vec![Action::LoadGenreAlbums],
                        crate::app::state::GenreContentType::ArtistGenres => vec![Action::LoadArtistGenreAlbums],
                        crate::app::state::GenreContentType::AlbumGenres => vec![Action::LoadAlbumGenreAlbums],
                        crate::app::state::GenreContentType::Moods => vec![Action::LoadMoodAlbums],
                        crate::app::state::GenreContentType::Styles => vec![Action::LoadStyleAlbums],
                        crate::app::state::GenreContentType::Stations => vec![], // Handled above
                    };
                }
            }
        }
        BrowseCategory::Folders => {
            // Folders use folder_state
            if let Some(folder_state) = &mut state.folder_state {
                if let Some(column) = folder_state.columns.get(folder_state.focused_column) {
                    let len = column.items.len();
                    let selected = column.selected_index;
                    let scroll_offset = helpers::calc_scroll_offset(selected, visible_height, len);
                    let actual_idx = visual_row + scroll_offset;

                    if actual_idx < len {
                        if let Some(col) = folder_state.columns.get_mut(folder_state.focused_column) {
                            col.selected_index = actual_idx;
                        }
                    }
                }
            }
        }
    }

    vec![]
}

/// Handle click on the right panel (albums/tracks).
fn handle_right_panel_click(click_row: u16, _click_col: u16, state: &mut AppState) -> Vec<Action> {
    // Right panel has a 1-row border at top
    // Visual row within the list (0-indexed from first visible item)
    let visual_row = click_row.saturating_sub(1) as usize;

    // Calculate visible height (content area minus transport and shortcuts, minus borders)
    let content_height = state.terminal_height.saturating_sub(4) as usize; // -3 for transport/shortcuts, -1 for top border
    let visible_height = content_height.saturating_sub(1); // Account for bottom border

    // Set focus to right panel
    state.focus = Focus::Right;

    // Handle based on current right panel mode
    match state.right_panel_mode {
        RightPanelMode::ArtistAlbums => {
            // Note: total includes "All Tracks" entry at index 0
            let len = state.selected_artist_albums.len() + 1;
            let selected = state.list_state.right_albums_index;
            let scroll_offset = helpers::calc_scroll_offset(selected, visible_height, len);
            let actual_idx = visual_row + scroll_offset;

            if actual_idx < len {
                let current = state.list_state.right_albums_index;
                if current == actual_idx {
                    // Double-click behavior: drill into album or All Tracks
                    if actual_idx == 0 {
                        return vec![Action::LoadArtistAllTracks];
                    } else {
                        return vec![Action::LoadSelectedAlbumTracks];
                    }
                }
                state.list_state.right_albums_index = actual_idx;
            }
        }
        RightPanelMode::CategoryAlbums => {
            let len = state.genre_albums.len();
            let selected = state.genre_albums_index;
            let scroll_offset = helpers::calc_scroll_offset(selected, visible_height, len);
            let actual_idx = visual_row + scroll_offset;

            if actual_idx < len {
                let current = state.genre_albums_index;
                if current == actual_idx {
                    // Double-click: drill into album
                    return vec![Action::LoadSelectedAlbumTracks];
                }
                state.genre_albums_index = actual_idx;
            }
        }
        RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
            let len = state.selected_album_tracks.len();
            let selected = state.list_state.tracks_index;
            let scroll_offset = helpers::calc_scroll_offset(selected, visible_height, len);
            let actual_idx = visual_row + scroll_offset;

            if actual_idx < len {
                let current = state.list_state.tracks_index;
                if current == actual_idx {
                    // Double-click: play track
                    return vec![Action::PlayTrackFromCategory(actual_idx)];
                }
                state.list_state.tracks_index = actual_idx;
            }
        }
        RightPanelMode::Empty => {}
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
                // Visual row (accounting for border)
                let visual_row = click_row.saturating_sub(1) as usize;

                // Calculate visible height
                let content_height = state.terminal_height.saturating_sub(5) as usize;
                let visible_height = content_height;

                // Combined list: play_history + queue tracks
                let history_len = state.play_history.len();
                let tracks_len = if state.playback_mode == PlaybackMode::Radio {
                    state.radio.tracks.len()
                } else {
                    state.queue.len()
                };
                let total_len = history_len + tracks_len;

                let selected = state.list_state.queue_index;
                let scroll_offset = helpers::calc_scroll_offset(selected, visible_height, total_len);
                let actual_idx = visual_row + scroll_offset;

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
        let tab_names = ["All", "Artists", "Album Artists", "Albums", "Playlists", "Tracks", "Genres"];
        let tabs_with_enum = [
            SearchTab::Global,
            SearchTab::Artists,
            SearchTab::AlbumArtists,
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
                // Row 3: "Library: ..." (if active_library) or Clear Cache
                // Row 4: blank (if active_library) or Sign Out
                // Row 5: Clear Cache (if active_library)
                // Row 6: Sign Out (if active_library)
                let offset = if state.active_library.is_some() { 5 } else { 3 };
                if visual_row == offset {
                    Some(0) // Clear Cache & Reload
                } else if visual_row == offset + 1 {
                    Some(1) // Sign Out
                } else {
                    None
                }
            } else {
                // Row 0: "Not signed in"
                // Row 1: blank
                // Row 2: Sign In → item 0
                if visual_row == 2 {
                    Some(0)
                } else {
                    None
                }
            }
        }
        SettingsSection::Libraries => {
            // Row 0: "Music libraries:" header
            // Row 1: blank
            // Row 2+: library items → item 0+
            if visual_row >= 2 {
                let idx = visual_row - 2;
                if idx < state.libraries.len() {
                    Some(idx)
                } else {
                    None
                }
            } else {
                None
            }
        }
        SettingsSection::Interface => {
            // Row 0: "Theme:" header
            // Row 1: blank
            // Row 2+: theme items → item 0+
            if visual_row >= 2 {
                let idx = visual_row - 2;
                if idx < crate::ui::theme::ThemeName::all().len() {
                    Some(idx)
                } else {
                    None
                }
            } else {
                None
            }
        }
        SettingsSection::Playback | SettingsSection::About => {
            // No selectable items
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

/// Handle scroll wheel events.
fn handle_scroll(up: bool, click_col: u16, state: &mut AppState) -> Vec<Action> {
    let delta: i32 = if up { -3 } else { 3 }; // Scroll 3 items at a time

    match state.view {
        View::Browse => {
            let left_panel_width = 30u16;
            if click_col < left_panel_width {
                // Scroll left panel
                let max = state.category_len().saturating_sub(1);
                let current = state.category_index();
                let new_idx = (current as i32 + delta).clamp(0, max as i32) as usize;
                state.set_category_index(new_idx);
            } else {
                // Scroll right panel
                match state.right_panel_mode {
                    RightPanelMode::ArtistAlbums => {
                        let max = state.selected_artist_albums.len().saturating_sub(1);
                        let new_idx = (state.list_state.right_albums_index as i32 + delta).clamp(0, max as i32) as usize;
                        state.list_state.right_albums_index = new_idx;
                    }
                    RightPanelMode::CategoryAlbums => {
                        let max = state.genre_albums.len().saturating_sub(1);
                        let new_idx = (state.genre_albums_index as i32 + delta).clamp(0, max as i32) as usize;
                        state.genre_albums_index = new_idx;
                    }
                    RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                        let max = state.selected_album_tracks.len().saturating_sub(1);
                        let new_idx = (state.list_state.tracks_index as i32 + delta).clamp(0, max as i32) as usize;
                        state.list_state.tracks_index = new_idx;
                    }
                    RightPanelMode::Empty => {}
                }
            }
        }
        View::NowPlaying => {
            // Scroll queue
            let max = state.queue.len().saturating_sub(1);
            let new_idx = (state.list_state.queue_index as i32 + delta).clamp(0, max as i32) as usize;
            state.list_state.queue_index = new_idx;
        }
        View::Search => {
            // Search scrolling handled via keyboard for now
            // (requires proper handling of optional filter_results)
        }
        View::Help => {
            // Scroll help content
            let new_scroll = (state.help_scroll as i32 + delta).max(0) as u16;
            state.help_scroll = new_scroll;
        }
        _ => {}
    }

    vec![]
}
