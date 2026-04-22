//! Root UI rendering (musikcube-style).
//!
//! Layout:
//! ┌──────────────────────────────────────────────────────────────┐
//! │ ┌─ artists ─────────┬─ track list ─────────────────────────┐ │
//! │ │ Artist 1          │ Album Header                         │ │
//! │ │ Artist 2          │   1  Track Name        4:32  Artist  │ │
//! │ │ > Artist 3        │   2  Track Name        3:21  Artist  │ │
//! │ │ Artist 4          │ Album Header                         │ │
//! │ │                   │   1  Track Name        5:02  Artist  │ │
//! │ └───────────────────┴──────────────────────────────────────┘ │
//! ├──────────────────────────────────────────────────────────────┤
//! │ playing Track Name by Artist from Album       vol ─■── 80%   │
//! ├──────────────────────────────────────────────────────────────┤
//! │ ^A artists │ ^P playlists │ ^N queue │ ^S similar │ ? │
//! └──────────────────────────────────────────────────────────────┘

use std::cell::RefCell;

use crate::app::state::{View, BrowseCategory, InputDialog, ConfirmDialog};
use crate::app::AppState;
use crate::services::NavigationService;
use super::artwork::ArtworkRenderer;
use super::layout::{AppLayout, FullScreenLayout, centered_rect};
use super::screens;
use super::theme::theme;
use super::widgets;
use super::widgets::scrollbar::render_scrollbar;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

thread_local! {
    static BIO_ARTWORK_RENDERER: RefCell<ArtworkRenderer> = RefCell::new(ArtworkRenderer::new());
}

/// Initialize the bio popup artwork renderer with a pre-detected picker.
/// Must be called before the event reader task starts consuming stdin.
pub fn init_bio_artwork_renderer(picker: ratatui_image::picker::Picker) {
    BIO_ARTWORK_RENDERER.with(|r| {
        *r.borrow_mut() = ArtworkRenderer::new_with_picker(picker);
    });
}

/// Set the bio artwork renderer mode.
pub fn set_bio_artwork_mode(mode: crate::app::state::ArtworkMode) {
    BIO_ARTWORK_RENDERER.with(|r| r.borrow_mut().set_mode(mode));
}

/// Set the bio artwork renderer protocol type.
pub fn set_bio_artwork_protocol_type(protocol_type: ratatui_image::picker::ProtocolType) {
    BIO_ARTWORK_RENDERER.with(|r| r.borrow_mut().set_protocol_type(protocol_type));
}

/// Restore the bio artwork renderer's native protocol.
pub fn restore_bio_artwork_native_protocol() {
    BIO_ARTWORK_RENDERER.with(|r| r.borrow_mut().restore_native_protocol());
}

/// Render the entire UI.
pub fn render(frame: &mut Frame, state: &AppState) {
    // Clear hit-test registry for this frame
    state.hit_regions.borrow_mut().clear();

    // Fill entire background with theme color
    let t = theme();
    frame.render_widget(Block::default().style(Style::default().bg(t.colors.bg_primary)), frame.area());

    match state.view {
        View::Auth => render_auth(frame, state),
        View::Browse => render_browse(frame, state),
        View::Queue => render_queue(frame, state),
        View::NowPlaying => render_now_playing(frame, state),
        View::Search => render_search(frame, state),
        View::Similar => render_similar(frame, state),
        View::Related => render_related(frame, state),
        View::Help => render_help(frame, state),
        View::Settings => render_settings(frame, state),
    }

    // Render search popup if active (floating dialog)
    if state.popups.search_active {
        screens::filter::render(frame, state, frame.area());
    }

    // Render radio launcher popup if active
    if state.popups.radio_launcher.is_some() {
        screens::radio_launcher::render(frame, state, frame.area());
    }

    // Render adventure launcher popup if active
    if state.popups.adventure_launcher.is_some() {
        screens::adventure_launcher::render(frame, state, frame.area());
    }

    // Render artist radio picker popup if active
    if state.popups.artist_radio_picker.is_some() {
        screens::artist_radio_picker::render(frame, state, frame.area());
    }

    // Render library picker popup if active
    if state.popups.library_picker_active {
        render_library_picker(frame, state);
    }

    // Render sort popup if active
    if state.popups.sort.is_some() {
        screens::sort_popup::render(frame, state, frame.area());
    }

    // Render artist bio popup if active
    if state.popups.artist_bio.is_some() {
        render_artist_bio_popup(frame, state);
    }

    // Render error popup if present
    if let Some(ref error) = state.notifications.last_error {
        render_error_popup(frame, error);
    }

    // Render input dialog if present
    if let Some(ref dialog) = state.popups.input_dialog {
        render_input_dialog(frame, dialog);
    }

    // Render confirm dialog if present
    if let Some(ref dialog) = state.popups.confirm_dialog {
        render_confirm_dialog(frame, state, dialog);
    }

    // Render toast notification if present (bottom-right, non-blocking)
    if let Some(ref toast) = state.notifications.toast_message {
        render_toast(frame, toast, frame.area());
    }
}

fn render_auth(frame: &mut Frame, state: &AppState) {
    screens::auth::render(frame, state, frame.area());
}

fn render_browse(frame: &mut Frame, state: &AppState) {
    use crate::app::state::BrowseCategory;

    let layout = AppLayout::new(frame.area());

    // Full area for all columns (combine left + right panels)
    let full_area = Rect {
        x: layout.left_panel.x,
        y: layout.left_panel.y,
        width: layout.left_panel.width + layout.right_panel.width,
        height: layout.left_panel.height,
    };

    // Column width: always divide into 3 equal slots (the minimum visible)
    // Columns never get narrower than this; they slide off the left edge instead.
    let col_width = full_area.width / 3;

    // Virtual column model:
    //   virtual 0 = category selector column
    //   virtual 1..N = content columns from the active category's nav state
    let content_focused = if state.category_column_focused {
        0
    } else {
        match state.browse_category {
            BrowseCategory::Folders => state.folder_state.as_ref().map_or(0, |fs| fs.focused_column),
            _ => state.browse_nav().map_or(0, |nav| nav.focused_column),
        }
    };

    let virtual_focus = if state.category_column_focused { 0 } else { 1 + content_focused };

    // Total virtual columns: 1 (category) + content columns
    let content_column_count = match state.browse_category {
        BrowseCategory::Folders => state.folder_state.as_ref().map_or(0, |fs| fs.columns.len()),
        _ => state.browse_nav().map_or(0, |nav| nav.columns.len()),
    };
    let total_virtual = 1 + content_column_count;

    // Sliding window: show 3 columns at a time.
    // The window only slides right when columns extend beyond what's visible.
    // It slides based on the DEEPEST column with content, not the focused column —
    // this prevents jumps when clicking between already-visible columns.
    // The focused column is guaranteed visible by clamping.
    let visible_count: usize = 3;

    // The rightmost column we want visible: the deepest column with content,
    // or at minimum the focused column.
    let rightmost = total_virtual.saturating_sub(1).max(virtual_focus);

    // Slide window right only as far as needed to show the rightmost column
    let virtual_start = if rightmost + 1 > visible_count {
        rightmost + 1 - visible_count
    } else {
        0
    };

    // But also ensure the focused column is visible (clamp left if needed)
    let virtual_start = virtual_start.min(virtual_focus);

    let t = theme();
    let category_items = BrowseCategory::all();

    // Determine if category column is visible
    let cat_col_visible = virtual_start == 0;

    // Render the category column if visible
    if cat_col_visible {
        let col_area = Rect {
            x: full_area.x,
            y: full_area.y,
            width: col_width,
            height: full_area.height,
        };

        let is_focused = state.category_column_focused;
        let border_color = if is_focused { t.colors.title_focused } else { t.colors.border };
        let block = Block::default()
            .title(" browse ")
            .title_style(Style::default().fg(if is_focused { t.colors.title_focused } else { t.colors.fg_accent }))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(t.colors.bg_primary));
        let inner = block.inner(col_area);
        frame.render_widget(block, col_area);

        // Register category column hit region
        {
            let mut hr = state.hit_regions.borrow_mut();
            hr.category_column = Some(crate::ui::hit_regions::CategoryColumnRegion {
                area: col_area,
                inner,
                item_count: category_items.len(),
            });
        }

        for (i, cat) in category_items.iter().enumerate() {
            if i as u16 >= inner.height { break; }
            let is_selected = i == state.category_column_index;
            let style = if is_selected && is_focused {
                Style::default().fg(t.colors.fg_primary).bg(t.colors.bg_selection)
            } else if is_selected {
                Style::default().fg(t.colors.fg_primary).bg(t.colors.bg_highlight)
            } else {
                Style::default().fg(t.colors.fg_primary)
            };

            let label = match cat {
                BrowseCategory::Library => "Library",
                BrowseCategory::Playlists => "Playlists",
                BrowseCategory::Genres => "Genres",
                BrowseCategory::Folders => "Folders",
            };
            let indicator = if is_selected { "\u{25b8} " } else { "  " };
            let text = format!("{}{}", indicator, label);
            let line_area = Rect { x: inner.x, y: inner.y + i as u16, width: inner.width, height: 1 };
            frame.render_widget(Paragraph::new(text).style(style), line_area);
        }
    }

    // Compute content area (to the right of the category column, or full width if cat col scrolled off)
    let content_area = if cat_col_visible {
        Rect {
            x: full_area.x + col_width,
            y: full_area.y,
            width: full_area.width.saturating_sub(col_width),
            height: full_area.height,
        }
    } else {
        full_area
    };

    // Pass content area to existing category renderers.
    // They compute their own internal column layout from the given area.
    // When category column is visible, they get 2/3 of the width (2 content column slots).
    // When it's scrolled off, they get full width (3 content column slots).
    let current_track_key = state.current_track().map(|t| t.rating_key.as_str());

    // When category column is focused, anchor the inner content viewport to
    // column 0 so the root column (e.g. Artists) is always visible next to the
    // category column.
    let content_focus_override = if state.category_column_focused { Some(0) } else { None };

    match state.browse_category {
        BrowseCategory::Library => {
            let (filter_results, filter_column) = if state.list_filter.active
                && state.list_filter.category == BrowseCategory::Library {
                (state.list_filter.results.as_ref(), Some(state.list_filter.column))
            } else { (None, None) };
            render_browse_miller_columns(
                frame, state, &state.artist_nav, "artists", current_track_key,
                filter_results, filter_column, false,
                content_area, Rect { x: 0, y: 0, width: 0, height: 0 },
                Some(col_width), content_focus_override,
            );
        }
        BrowseCategory::Playlists => {
            let (filter_results, filter_column) = if state.list_filter.active
                && state.list_filter.category == BrowseCategory::Playlists {
                (state.list_filter.results.as_ref(), Some(state.list_filter.column))
            } else { (None, None) };
            render_browse_miller_columns(
                frame, state, &state.playlist_nav, "playlists", current_track_key,
                filter_results, filter_column, true,
                content_area, Rect { x: 0, y: 0, width: 0, height: 0 },
                Some(col_width), content_focus_override,
            );
        }
        BrowseCategory::Genres => {
            let (filter_results, filter_column) = if state.list_filter.active
                && state.list_filter.category == BrowseCategory::Genres {
                (state.list_filter.results.as_ref(), Some(state.list_filter.column))
            } else { (None, None) };
            render_browse_miller_columns(
                frame, state, &state.genre_nav, "genres", current_track_key,
                filter_results, filter_column, false,
                content_area, Rect { x: 0, y: 0, width: 0, height: 0 },
                Some(col_width), content_focus_override,
            );
        }
        BrowseCategory::Folders => {
            let (filter_results, filter_column) = if state.list_filter.active
                && state.list_filter.category == BrowseCategory::Folders {
                (state.list_filter.results.as_ref(), Some(state.list_filter.column))
            } else { (None, None) };
            render_folder_view(frame, state, filter_results, filter_column,
                content_area, Rect { x: 0, y: 0, width: 0, height: 0 },
                Some(col_width), content_focus_override);
        }
    }

    // Chrome: tab bar, transport, command bar
    render_transport(frame, state, layout.transport);
    render_commands(frame, state, layout.commands);
}

fn render_queue(frame: &mut Frame, state: &AppState) {
    let layout = FullScreenLayout::new(frame.area());

    screens::now_playing::render_queue_mode(frame, state, layout.content);
    render_transport(frame, state, layout.transport);
    render_commands(frame, state, layout.commands);
}

/// Compute the queue right panel (track list) area for popup centering.
/// Replicates the layout calculation from render_queue_mode.
fn render_now_playing(frame: &mut Frame, state: &AppState) {
    let layout = FullScreenLayout::new(frame.area());

    screens::now_playing::render_visualizer_mode(frame, state, layout.content);
    render_transport(frame, state, layout.transport);
    render_commands(frame, state, layout.commands);
}

fn render_search(frame: &mut Frame, state: &AppState) {
    let layout = FullScreenLayout::new(frame.area());

    // Unified search/filter screen handles all tabs including Global (with 3-column layout)
    screens::filter::render(frame, state, layout.content);
    render_transport(frame, state, layout.transport);
    render_commands(frame, state, layout.commands);
}

fn render_similar(frame: &mut Frame, state: &AppState) {
    // Render the previous view behind the popup
    let prev = state.previous_view.unwrap_or(View::Browse);
    match prev {
        View::Queue => render_queue(frame, state),
        View::NowPlaying => render_now_playing(frame, state),
        View::Browse => render_browse(frame, state),
        _ => render_browse(frame, state),
    }

    // Overlay the similar popup
    screens::similar::render(frame, state, frame.area());
}

fn render_related(frame: &mut Frame, state: &AppState) {
    // Render the previous view behind the popup
    let prev = state.previous_view.unwrap_or(View::Browse);
    match prev {
        View::Queue => render_queue(frame, state),
        View::NowPlaying => render_now_playing(frame, state),
        View::Browse => render_browse(frame, state),
        _ => render_browse(frame, state),
    }

    // Overlay the related popup
    screens::related::render(frame, state, frame.area());
}

fn render_help(frame: &mut Frame, state: &AppState) {
    let layout = FullScreenLayout::new(frame.area());

    screens::help::render(frame, state, layout.content);
    render_transport(frame, state, layout.transport);
    render_commands(frame, state, layout.commands);
}

fn render_settings(frame: &mut Frame, state: &AppState) {
    let layout = FullScreenLayout::new(frame.area());

    screens::settings::render(frame, state, layout.content);
    render_transport(frame, state, layout.transport);
    render_commands(frame, state, layout.commands);
}

/// Render folder browsing view (Miller columns style) with lazy/windowed rendering.
/// Truncate a path from the left, keeping the end visible.
/// E.g. "D:\music\artist\album" with max 15 → "…\artist\album"
fn truncate_path_left(path: &str, max_width: usize) -> String {
    let char_count = path.chars().count();
    if char_count <= max_width {
        return path.to_string();
    }
    if max_width <= 1 {
        return "…".to_string();
    }
    let keep = max_width - 1; // 1 char for "…"
    let skip = char_count - keep;
    let tail: String = path.chars().skip(skip).collect();
    format!("…{}", tail)
}

fn render_folder_view(
    frame: &mut Frame,
    state: &AppState,
    filter_results: Option<&crate::app::state::ListFilterResults>,
    filter_column: Option<usize>,
    left_area: Rect,
    right_area: Rect,
    fixed_col_width: Option<u16>,
    focus_override: Option<usize>,
) {
    use crate::services::FolderItemType;

    let t = theme();

    // Combine left and right panels for folder view
    let area = Rect {
        x: left_area.x,
        y: left_area.y,
        width: left_area.width + right_area.width,
        height: left_area.height,
    };

    if let Some(ref folder_state) = state.folder_state {
        if folder_state.loading {
            let block = Block::default()
                .title(" folders ")
                .title_style(Style::default().fg(t.colors.fg_accent))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(t.colors.border_focused))
                .style(Style::default().bg(t.colors.bg_primary));
            let inner = block.inner(area);
            frame.render_widget(block, area);
            let loading = Paragraph::new("Loading...")
                .style(Style::default().fg(t.colors.fg_muted));
            frame.render_widget(loading, inner);
            return;
        }

        let num_columns = folder_state.columns.len();
        if num_columns == 0 {
            return;
        }

        // Don't show empty trailing columns
        // Find the last non-empty column (or focused column, whichever is greater)
        let last_meaningful = (0..num_columns)
            .rev()
            .find(|&i| !folder_state.columns[i].items.is_empty() || i <= folder_state.focused_column)
            .unwrap_or(0);
        let effective_columns = (last_meaningful + 1).max(num_columns.min(2));

        // Calculate column width - use fixed width when provided (from outer browse layout)
        let (max_visible, col_width) = if let Some(fixed_w) = fixed_col_width {
            let max_vis = (area.width / fixed_w).max(1) as usize;
            (max_vis, fixed_w)
        } else {
            let max_vis = 3.min(effective_columns).max(2);
            (max_vis, area.width / max_vis as u16)
        };

        // Determine which columns to show.
        // Slide based on deepest column, not focus — prevents jumps when clicking
        // between already-visible columns.
        // When focus_override is provided (category column focused), anchor viewport left.
        let viewport_focus = focus_override.unwrap_or(folder_state.focused_column);
        let rightmost_col = effective_columns.saturating_sub(1).max(viewport_focus);
        let start_col = if rightmost_col + 1 > max_visible {
            let s = rightmost_col + 1 - max_visible;
            s.min(viewport_focus)
        } else {
            0
        };

        // Get currently playing track key once for all columns
        let current_track_key = state.current_track().map(|t| t.rating_key.as_str());

        // Register folder Miller column regions for hit-testing
        {
            let mut column_regions = Vec::new();
            for (vis_idx, col_idx) in (start_col..effective_columns.min(start_col + max_visible)).enumerate() {
                let col_area = Rect {
                    x: area.x + (vis_idx as u16 * col_width),
                    y: area.y,
                    width: if vis_idx == max_visible - 1 {
                        area.width - (vis_idx as u16 * col_width)
                    } else {
                        col_width
                    },
                    height: area.height,
                };
                let block_tmp = Block::default().borders(Borders::ALL);
                let inner_tmp = block_tmp.inner(col_area);
                column_regions.push(crate::ui::hit_regions::MillerColumnRegion {
                    col_idx,
                    area: col_area,
                    inner: inner_tmp,
                    rows_per_item: 1, // Folder items are always 1-row
                    is_art_mode: false,
                });
            }
            let mut hr = state.hit_regions.borrow_mut();
            hr.miller_columns = Some(crate::ui::hit_regions::MillerRegions {
                area,
                columns: column_regions,
            });
        }

        for (vis_idx, col_idx) in (start_col..effective_columns.min(start_col + max_visible)).enumerate() {
            let col = &folder_state.columns[col_idx];
            let is_focused = focus_override.is_none() && col_idx == folder_state.focused_column;

            let col_area = Rect {
                x: area.x + (vis_idx as u16 * col_width),
                y: area.y,
                width: if vis_idx == max_visible - 1 {
                    area.width - (vis_idx as u16 * col_width) // Last column gets remaining width
                } else {
                    col_width
                },
                height: area.height,
            };

            use crate::util::truncate_middle;

            let border_color = if is_focused { t.colors.title_focused } else { t.colors.border };
            let is_root = col_idx == 0;

            // Show title for all columns; folder paths truncate from the left
            let max_title_width = col_area.width.saturating_sub(4) as usize; // borders + padding
            let title = if is_root && col.is_shuffled() {
                " folders (shuffled) ".to_string()
            } else if is_root {
                " folders ".to_string()
            } else if col.is_shuffled() {
                let t = truncate_path_left(&col.title, max_title_width);
                format!(" {} (shuffled) ", t)
            } else if !col.title.is_empty() {
                let t = truncate_path_left(&col.title, max_title_width);
                format!(" {} ", t)
            } else {
                String::new()
            };

            let mut block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(t.colors.bg_primary));

            if !title.is_empty() {
                let title_color = if is_focused { t.colors.title_focused } else { t.colors.fg_accent };
                block = block
                    .title(title)
                    .title_style(Style::default().fg(title_color));
            }

            let inner = block.inner(col_area);
            frame.render_widget(block, col_area);

            if col.items.is_empty() {
                let empty = Paragraph::new("(empty)")
                    .style(Style::default().fg(t.colors.fg_muted));
                frame.render_widget(empty, inner);
            } else {
                // LAZY LOADING: Only render visible items
                let visible_height = inner.height as usize;
                let selected_idx = col.selected_index;

                // Calculate max width for text (minus prefix and padding)
                let max_text_width = inner.width.saturating_sub(4) as usize;

                // Check if filter is active on this column
                let is_filter_column = filter_column == Some(col_idx);
                let (items_to_show, total_items, filter_active_on_col): (Vec<(usize, &crate::services::FolderItem)>, usize, bool) =
                    if let Some(results) = filter_results.filter(|_| is_filter_column) {
                        if results.matched_indices.is_empty() {
                            (vec![], 0, true)
                        } else {
                            let items: Vec<_> = results.matched_indices.iter()
                                .filter_map(|&idx| col.items.get(idx).map(|item| (idx, item)))
                                .collect();
                            let len = items.len();
                            (items, len, true)
                        }
                    } else {
                        let items: Vec<_> = col.items.iter().enumerate().collect();
                        let len = items.len();
                        (items, len, false)
                    };

                // Calculate scroll offset (needed for both rendering and scrollbar)
                let display_selected_idx = if let Some(results) = filter_results.filter(|_| filter_active_on_col) {
                    results.matched_indices.iter()
                        .position(|&idx| idx == selected_idx)
                        .unwrap_or(0)
                } else {
                    selected_idx
                };
                let scroll_offset = match state.scroll.browse {
                    Some((pin_col, pinned)) if pin_col == col_idx => pinned,
                    _ => NavigationService::calc_scroll_offset(display_selected_idx, visible_height, total_items),
                };

                if items_to_show.is_empty() && filter_active_on_col {
                    let empty = Paragraph::new("no matches")
                        .style(Style::default().fg(t.colors.fg_muted));
                    frame.render_widget(empty, inner);
                } else {
                    // Only create ListItems for visible range
                    let visible_items: Vec<ListItem> = items_to_show.into_iter()
                        .skip(scroll_offset)
                        .take(visible_height)
                        .map(|(orig_idx, item)| {
                            let is_selected = orig_idx == selected_idx;

                            // Check if this item is the currently playing track
                            let is_now_playing = matches!(item.item_type, FolderItemType::Track)
                                && current_track_key.map(|k| item.key == k).unwrap_or(false);

                            let prefix = match item.item_type {
                                FolderItemType::Folder => "▸ ",
                                FolderItemType::Track if is_now_playing => "♪ ",
                                FolderItemType::Track => "  ",
                            };

                            // Use middle truncation for long titles
                            let display_title = truncate_middle(&item.title, max_text_width);

                            let style = if is_now_playing {
                                Style::default().fg(t.colors.fg_accent).add_modifier(ratatui::style::Modifier::BOLD)
                            } else if is_selected && is_focused {
                                Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                            } else if is_selected {
                                Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                            } else {
                                Style::default().fg(t.colors.fg_primary)
                            };
                            ListItem::new(format!("{}{}", prefix, display_title)).style(style)
                        })
                        .collect();

                    let list = List::new(visible_items);
                    frame.render_widget(list, inner);
                }

                // Scrollbar + position indicator for long lists
                if total_items > visible_height {
                    render_scrollbar(frame, col_area, total_items, visible_height, scroll_offset, Some(border_color));

                    let footer = format!("{}/{}", selected_idx + 1, total_items);
                    let footer_area = Rect::new(
                        col_area.x + col_area.width.saturating_sub(footer.len() as u16 + 2),
                        col_area.y + col_area.height - 1,
                        footer.len() as u16 + 1,
                        1,
                    );
                    frame.render_widget(
                        Paragraph::new(footer).style(Style::default().fg(t.colors.fg_muted)),
                        footer_area,
                    );
                }
            }
        }
    } else {
        let block = Block::default()
            .title(" folders ")
            .title_style(Style::default().fg(t.colors.fg_accent))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.colors.border_focused))
            .style(Style::default().bg(t.colors.bg_primary));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let msg = Paragraph::new("Loading folders...")
            .style(Style::default().fg(t.colors.fg_muted));
        frame.render_widget(msg, inner);
    }
}

/// Determine if a Miller column should use 2-row display.
///
/// Returns true for:
/// - Special track columns (playlist tracks, all-tracks, compilation tracks, etc.)
/// - Album-grouped columns (TracksByAlbum / TracksByArtist mode)
/// - Album columns in "All Artists" mode
/// - Genre/mood album columns
fn is_two_row_column(
    state: &AppState,
    col: &crate::app::state::BrowseColumn,
    col_idx: usize,
    nav: &crate::app::state::BrowseNavigationState,
    _two_row_tracks: bool,
) -> bool {
    use crate::app::state::BrowseItem;

    let first_is_track = col.items.first().map_or(false, |item| matches!(item, BrowseItem::Track { .. }));
    let first_is_album = col.items.first().map_or(false, |item| matches!(item, BrowseItem::Album { .. }));

    // Special track columns always get two-row display
    if first_is_track && state.is_special_track_column(nav, col_idx) {
        return true;
    }

    // Album columns in "All Artists" mode (shows artist on 2nd row)
    if first_is_album && (nav.columns.first()
        .and_then(|c| c.selected_item())
        .map_or(false, |item| matches!(item, BrowseItem::AllArtists))
        || (state.browse_category == crate::app::state::BrowseCategory::Library
            && state.library.library_sub_mode != crate::app::state::LibrarySubMode::Normal
            && col_idx == 0))
    {
        return true;
    }

    // Genre/mood album columns
    if first_is_album && state.browse_category == BrowseCategory::Genres {
        return true;
    }

    // Grouped-by-album playlist columns (albums with artist on 2nd row)
    if first_is_album && col.grouped_by_album {
        return true;
    }

    false
}

/// Render a BrowseNavigationState as dynamic Miller columns.
/// Used for Artists, Playlists, and Genres views.
/// When filter_results is Some, only show items at the matched indices in the filter_column.
fn render_browse_miller_columns(
    frame: &mut Frame,
    state: &AppState,
    nav: &crate::app::state::BrowseNavigationState,
    root_title: &str,
    current_track_key: Option<&str>,
    filter_results: Option<&crate::app::state::ListFilterResults>,
    filter_column: Option<usize>,
    two_row_tracks: bool,
    left_area: Rect,
    right_area: Rect,
    fixed_col_width: Option<u16>,
    focus_override: Option<usize>,
) {
    use crate::app::state::BrowseItem;
    use crate::util::truncate_middle;

    let t = theme();

    // Combine left and right panels for full-width Miller columns
    let area = Rect {
        x: left_area.x,
        y: left_area.y,
        width: left_area.width + right_area.width,
        height: left_area.height,
    };

    // Loading with no columns yet: show full loading state
    if nav.loading && nav.columns.is_empty() {
        let block = Block::default()
            .title(format!(" {} ", root_title))
            .title_style(Style::default().fg(t.colors.fg_accent))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.colors.border_focused))
            .style(Style::default().bg(t.colors.bg_primary));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let loading = Paragraph::new("Loading...")
            .style(Style::default().fg(t.colors.fg_muted));
        frame.render_widget(loading, inner);
        return;
    }

    let num_columns = nav.columns.len();
    if num_columns == 0 {
        // Empty state - show single column with message
        let block = Block::default()
            .title(format!(" {} ", root_title))
            .title_style(Style::default().fg(t.colors.fg_accent))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.colors.border_focused))
            .style(Style::default().bg(t.colors.bg_primary));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let msg = Paragraph::new("No items")
            .style(Style::default().fg(t.colors.fg_muted));
        frame.render_widget(msg, inner);
        return;
    }

    // Find last non-empty column (or focused column, whichever is greater)
    let last_meaningful = (0..num_columns)
        .rev()
        .find(|&i| !nav.columns[i].items.is_empty() || i <= nav.focused_column)
        .unwrap_or(0);
    let effective_columns = last_meaningful + 1;
    // When loading with existing columns, reserve space for a loading indicator column
    let layout_columns = if nav.loading { effective_columns + 1 } else { effective_columns };

    // When a fixed column width is provided (from the outer browse layout), use it
    // to keep column widths consistent with the category column. Otherwise fall back
    // to the traditional calculation.
    let (max_visible, col_width) = if let Some(fixed_w) = fixed_col_width {
        let max_vis = (area.width / fixed_w).max(1) as usize;
        (max_vis, fixed_w)
    } else {
        let max_vis = 3.min(layout_columns).max(2);
        (max_vis, area.width / max_vis as u16)
    };

    // Determine which columns to show.
    // Slide based on the deepest column with content, not focus — prevents jumps
    // when clicking between already-visible columns.
    // When focus_override is provided (e.g. category column is focused), use that
    // instead of nav.focused_column so the viewport stays anchored to the left.
    let viewport_focus = focus_override.unwrap_or(nav.focused_column);
    let rightmost_col = effective_columns.saturating_sub(1).max(viewport_focus);
    let start_col = if rightmost_col + 1 > max_visible {
        let s = rightmost_col + 1 - max_visible;
        s.min(viewport_focus) // ensure focused column stays visible
    } else {
        0
    };

    // Register Miller column regions for hit-testing
    {
        let mut column_regions = Vec::new();
        for (vis_idx, col_idx) in (start_col..effective_columns.min(start_col + max_visible)).enumerate() {
            let col = &nav.columns[col_idx];
            let col_area = Rect {
                x: area.x + (vis_idx as u16 * col_width),
                y: area.y,
                width: if vis_idx == max_visible - 1 {
                    area.width - (vis_idx as u16 * col_width)
                } else {
                    col_width
                },
                height: area.height,
            };
            let block_tmp = Block::default().borders(Borders::ALL);
            let inner_tmp = block_tmp.inner(col_area);
            let is_two_row = is_two_row_column(state, col, col_idx, nav, two_row_tracks);
            column_regions.push(crate::ui::hit_regions::MillerColumnRegion {
                col_idx,
                area: col_area,
                inner: inner_tmp,
                rows_per_item: if is_two_row { 2 } else { 1 },
                is_art_mode: col.artwork_visible,
            });
        }
        let mut hr = state.hit_regions.borrow_mut();
        hr.miller_columns = Some(crate::ui::hit_regions::MillerRegions {
            area,
            columns: column_regions,
        });
    }

    for (vis_idx, col_idx) in (start_col..effective_columns.min(start_col + max_visible)).enumerate() {
        let col = &nav.columns[col_idx];
        let is_focused = focus_override.is_none() && col_idx == nav.focused_column;
        let is_root = col_idx == 0;

        let col_area = Rect {
            x: area.x + (vis_idx as u16 * col_width),
            y: area.y,
            width: if vis_idx == max_visible - 1 {
                area.width - (vis_idx as u16 * col_width) // Last column gets remaining width
            } else {
                col_width
            },
            height: area.height,
        };

        let border_color = if is_focused { t.colors.title_focused } else { t.colors.border };

        // Show title for all columns with sort suffix
        let sort_suffix = {
            let suffix = col.sort_mode.header_suffix(!col.sort_ascending);
            if suffix.is_empty() { String::new() } else { format!(" ({})", suffix) }
        };

        let title = if is_root {
            format!(" {}{} ", root_title, sort_suffix)
        } else if !col.title.is_empty() {
            if col.grouped_by_album {
                format!(" albums - {}{} ", col.title, sort_suffix)
            } else {
                format!(" {}{} ", col.title, sort_suffix)
            }
        } else {
            String::new()
        };

        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(t.colors.bg_primary));

        if !title.is_empty() {
            let title_color = if is_focused { t.colors.title_focused } else { t.colors.fg_accent };
            block = block
                .title(title)
                .title_style(Style::default().fg(title_color));
        }

        let inner = block.inner(col_area);
        frame.render_widget(block, col_area);

        if col.items.is_empty() {
            let empty = Paragraph::new("(empty)")
                .style(Style::default().fg(t.colors.fg_muted));
            frame.render_widget(empty, inner);
            continue;
        }

        // Check if this column has artwork_visible enabled
        let is_filter_column = filter_column == Some(col_idx);
        if col.artwork_visible {
            let col_filter = if is_filter_column { filter_results } else { None };
            render_album_art_grid(frame, state, col, is_focused, inner, col_area, col_idx, col_filter);
            continue;
        }

        {
            // Calculate visible range for lazy loading
            let visible_height = inner.height as usize;
            let selected_idx = col.selected_index;

            // Calculate max width for text (minus prefix and padding)
            let max_text_width = inner.width.saturating_sub(4) as usize;

            let is_two_row = is_two_row_column(state, col, col_idx, nav, two_row_tracks);
            let rows_per_item = if is_two_row { 2 } else { 1 };
            let visible_item_count = visible_height / rows_per_item;

            // When filter is active on this column, only show filtered items
            let (items_to_show, total_display_items, filter_active_on_col): (Vec<(usize, &BrowseItem)>, usize, bool) =
                if let Some(results) = filter_results.filter(|_| is_filter_column) {
                    if results.matched_indices.is_empty() {
                        (vec![], 0, true)
                    } else {
                        let items: Vec<_> = results.matched_indices.iter()
                            .filter_map(|&idx| col.items.get(idx).map(|item| (idx, item)))
                            .collect();
                        let len = items.len();
                        (items, len, true)
                    }
                } else {
                    let items: Vec<_> = col.items.iter().enumerate().collect();
                    let len = items.len();
                    (items, len, false)
                };

            if items_to_show.is_empty() && filter_active_on_col {
                let empty = Paragraph::new("no matches")
                    .style(Style::default().fg(t.colors.fg_muted));
                frame.render_widget(empty, inner);
            } else {
                // Calculate scroll offset based on display items
                let display_selected_idx = if let Some(results) = filter_results.filter(|_| filter_active_on_col) {
                    results.matched_indices.iter()
                        .position(|&idx| idx == selected_idx)
                        .unwrap_or(0)
                } else {
                    selected_idx
                };
                let scroll_offset = match state.scroll.browse {
                    Some((pin_col, pinned)) if pin_col == col_idx => pinned,
                    _ => NavigationService::calc_scroll_offset(display_selected_idx, visible_item_count, total_display_items),
                };

                let visible_items: Vec<ListItem> = items_to_show.into_iter()
                    .skip(scroll_offset)
                    .take(visible_item_count)
                    .map(|(orig_idx, item)| {
                        let is_selected = orig_idx == selected_idx;

                        // Check if this is the currently playing track
                        let is_now_playing = matches!(item, BrowseItem::Track { key, .. } if current_track_key == Some(key.as_str()));

                        // Prefix based on item type
                        let is_pinned = matches!(item,
                            BrowseItem::AllArtists | BrowseItem::Compilations |
                            BrowseItem::AllTracks { .. } | BrowseItem::ArtistRadio { .. } |
                            BrowseItem::CompilationTracks { .. }
                        );
                        let prefix = match item {
                            BrowseItem::Track { .. } if is_now_playing => "♪ ",
                            BrowseItem::Track { .. } => "  ",
                            _ if is_pinned => "  ", // No arrow for pinned items
                            _ => "▸ ", // Drillable items get arrow
                        };

                        // Full text for line 1 (before truncation)
                        let full_text = match item {
                            BrowseItem::Album { title, year, .. } => {
                                if let Some(y) = year {
                                    format!("{} ({})", title, y)
                                } else {
                                    title.clone()
                                }
                            }
                            BrowseItem::Track { title, track_number, .. } => {
                                // Show track numbers only in album drill-downs (1-row mode)
                                if !is_two_row {
                                    if let Some(num) = track_number {
                                        format!("{:02}. {}", num, title)
                                    } else {
                                        title.clone()
                                    }
                                } else {
                                    title.clone()
                                }
                            }
                            _ => item.title().to_string(),
                        };

                        // Duration string for tracks (right-aligned)
                        let dur_str = match item {
                            BrowseItem::Track { duration_ms, .. } if *duration_ms > 0 => {
                                Some(crate::util::format_duration(*duration_ms))
                            }
                            _ => None,
                        };
                        // Reduce title width to make room for duration
                        let title_width = if let Some(ref d) = dur_str {
                            max_text_width.saturating_sub(d.len() + 1)
                        } else {
                            max_text_width
                        };

                        // Marquee for selected+focused item, or truncate normally
                        let display_text = if is_selected && is_focused {
                            let marquee_key = format!("miller:{}:{}", col_idx, orig_idx);
                            let mut marquee = state.marquee.borrow_mut();
                            if marquee.selection_key != marquee_key {
                                marquee.reset(marquee_key, full_text.clone(), title_width);
                            }
                            if marquee.phase == crate::app::state::MarqueePhase::Inactive {
                                truncate_middle(&full_text, title_width)
                            } else {
                                let text = marquee.display_text();
                                drop(marquee);
                                // Trim to title_width (display_text already pads)
                                text.chars().take(title_width).collect()
                            }
                        } else {
                            truncate_middle(&full_text, title_width)
                        };

                        // Build ListItem — 2-row for playlist tracks or All Artists albums, 1-row otherwise
                        if is_two_row {
                            // Determine subtitle content based on item type
                            let subtitle_content = match item {
                                BrowseItem::Track { artist_name, album_name, year, .. } => {
                                    match (artist_name.as_ref(), album_name.as_ref()) {
                                        (Some(a), Some(b)) => {
                                            if let Some(y) = year {
                                                format!("{} — {} ({})", a, b, y)
                                            } else {
                                                format!("{} — {}", a, b)
                                            }
                                        }
                                        (Some(a), None) => a.clone(),
                                        (None, Some(b)) => {
                                            if let Some(y) = year {
                                                format!("{} ({})", b, y)
                                            } else {
                                                b.clone()
                                            }
                                        }
                                        (None, None) => String::new(),
                                    }
                                }
                                BrowseItem::Album { artist, .. } => {
                                    // All Artists mode: show artist on second row
                                    artist.clone()
                                }
                                _ => String::new(),
                            };

                            if !subtitle_content.is_empty() || matches!(item, BrowseItem::Track { .. } | BrowseItem::Album { .. }) {
                                // Subtitle display width (5 indent + 2 padding = 7 overhead)
                                let subtitle_width = (inner.width as usize).saturating_sub(7);

                                // Marquee for subtitle row (independent of title)
                                let subtitle_display = if is_selected && is_focused && !subtitle_content.is_empty() {
                                    let sub_key = format!("miller:{}:{}:sub", col_idx, orig_idx);
                                    let mut sub_marquee = state.marquee_subtitle.borrow_mut();
                                    if sub_marquee.selection_key != sub_key {
                                        sub_marquee.reset(sub_key, subtitle_content.clone(), subtitle_width);
                                    }
                                    if sub_marquee.phase == crate::app::state::MarqueePhase::Inactive {
                                        truncate_middle(&subtitle_content, subtitle_width)
                                    } else {
                                        let text = sub_marquee.display_text();
                                        drop(sub_marquee);
                                        text
                                    }
                                } else {
                                    truncate_middle(&subtitle_content, subtitle_width)
                                };

                                let (line1_fg, line2_fg, item_bg) = if is_now_playing {
                                    (
                                        Style::default().fg(t.colors.fg_accent).add_modifier(ratatui::style::Modifier::BOLD),
                                        Style::default().fg(t.colors.fg_accent),
                                        Style::default(),
                                    )
                                } else if is_selected {
                                    (
                                        Style::default().fg(t.colors.selection_text),
                                        Style::default().fg(t.colors.selection_text),
                                        Style::default().bg(t.colors.selection_bar_bg),
                                    )
                                } else {
                                    (
                                        Style::default().fg(t.colors.fg_primary),
                                        Style::default().fg(t.colors.fg_muted),
                                        Style::default(),
                                    )
                                };

                                // Build line 1 with optional right-aligned duration
                                let line1 = if let Some(ref dur) = dur_str {
                                    let title_chars = display_text.chars().count();
                                    let pad = title_width.saturating_sub(title_chars);
                                    Line::from(Span::styled(
                                        format!("{}{}{} {}", prefix, display_text, " ".repeat(pad), dur),
                                        line1_fg,
                                    ))
                                } else {
                                    Line::from(Span::styled(format!("{}{}", prefix, display_text), line1_fg))
                                };

                                let text = Text::from(vec![
                                    line1,
                                    Line::from(Span::styled(format!("     {}", subtitle_display), line2_fg)),
                                ]);
                                ListItem::new(text).style(item_bg)
                            } else {
                                // Non-track/album item in a two-row column (handle gracefully)
                                let style = if is_selected {
                                    Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                                } else if is_pinned {
                                    Style::default().fg(t.colors.fg_accent)
                                } else {
                                    Style::default().fg(t.colors.fg_primary)
                                };
                                ListItem::new(format!("{}{}", prefix, display_text)).style(style)
                            }
                        } else {
                            let style = if is_now_playing {
                                Style::default().fg(t.colors.fg_accent).add_modifier(ratatui::style::Modifier::BOLD)
                            } else if is_selected {
                                Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                            } else if is_pinned {
                                Style::default().fg(t.colors.fg_accent)
                            } else {
                                Style::default().fg(t.colors.fg_primary)
                            };

                            // Build display with optional right-aligned duration
                            if let Some(ref dur) = dur_str {
                                let title_chars = display_text.chars().count();
                                let pad = title_width.saturating_sub(title_chars);
                                ListItem::new(format!("{}{}{} {}", prefix, display_text, " ".repeat(pad), dur)).style(style)
                            } else {
                                ListItem::new(format!("{}{}", prefix, display_text)).style(style)
                            }
                        }
                    })
                    .collect();

                let list = List::new(visible_items);
                frame.render_widget(list, inner);

                // Scrollbar + position indicator for long lists
                if total_display_items > visible_item_count {
                    // Render scrollbar on right edge of column
                    render_scrollbar(
                        frame,
                        col_area,
                        total_display_items,
                        visible_item_count,
                        scroll_offset,
                        Some(border_color),
                    );

                    let footer = format!("{}/{}", display_selected_idx + 1, total_display_items);
                    let footer_area = Rect::new(
                        col_area.x + col_area.width.saturating_sub(footer.len() as u16 + 2),
                        col_area.y + col_area.height - 1,
                        footer.len() as u16 + 1,
                        1,
                    );
                    frame.render_widget(
                        Paragraph::new(footer).style(Style::default().fg(t.colors.fg_muted)),
                        footer_area,
                    );
                }
            }
        }
    }

    // Render placeholder columns when fewer real columns than max_visible
    // (loading indicator or empty column to maintain 2-column minimum)
    let real_rendered = effective_columns.min(start_col + max_visible).saturating_sub(start_col);
    if real_rendered < max_visible {
        let vis_idx = real_rendered;
        let placeholder_area = Rect {
            x: area.x + (vis_idx as u16 * col_width),
            y: area.y,
            width: area.width - (vis_idx as u16 * col_width),
            height: area.height,
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.colors.border))
            .style(Style::default().bg(t.colors.bg_primary));
        let placeholder_inner = block.inner(placeholder_area);
        frame.render_widget(block, placeholder_area);
        if nav.loading {
            let loading = Paragraph::new("Loading...")
                .style(Style::default().fg(t.colors.fg_muted));
            frame.render_widget(loading, placeholder_inner);
        }
    }
}

/// Render album art list for a column in cover art view mode.
/// Each row: artwork thumbnail on the left, title/artist text on the right.
fn render_album_art_grid(
    frame: &mut Frame,
    state: &AppState,
    col: &crate::app::state::BrowseColumn,
    is_focused: bool,
    inner: Rect,
    col_area: Rect,
    col_idx: usize,
    filter_results: Option<&crate::app::state::ListFilterResults>,
) {
    use crate::app::state::BrowseItem;
    use crate::util::truncate_middle;
    let t = theme();

    // Build the list of items to display (filtered or full)
    let items_with_indices: Vec<(usize, &BrowseItem)> = if let Some(results) = filter_results {
        if results.matched_indices.is_empty() {
            let empty = Paragraph::new("no matches")
                .style(Style::default().fg(t.colors.fg_muted));
            frame.render_widget(empty, inner);
            return;
        }
        results.matched_indices.iter()
            .filter_map(|&idx| col.items.get(idx).map(|item| (idx, item)))
            .collect()
    } else {
        col.items.iter().enumerate().collect()
    };

    let total_items = items_with_indices.len();
    if total_items == 0 {
        return;
    }

    // Classify items: "one-row" pinned items vs normal art-height items
    fn is_one_row(item: &BrowseItem) -> bool {
        matches!(item,
            BrowseItem::ArtistRadio { .. } |
            BrowseItem::CompilationTracks { .. } |
            BrowseItem::Compilations
        )
    }

    // Count art items to size rows (one-row items don't affect art sizing)
    let art_item_count = items_with_indices.iter().filter(|(_, item)| !is_one_row(item)).count();

    // Each list row: artwork on left, text on right
    // Size rows to fill available vertical space with at least 3 visible items.
    // Row height is derived from panel height, then art_width from row_height.
    let target_visible = 3u16.max((art_item_count.max(1) as u16).min(5));
    let art_row_height = (inner.height / target_visible).max(3);
    // Art width: 2x art_row_height (terminal chars are ~2:1 aspect), capped at half column width
    let max_art = inner.width / 2;
    let art_width = (art_row_height * 2).min(max_art).max(6);

    if art_row_height == 0 {
        return;
    }

    // Check if there's a spacer between item at `idx` and the next item
    // (spacer appears after the last consecutive one-row item before an art item)
    let has_spacer_after = |idx: usize| -> bool {
        idx + 1 < total_items
            && is_one_row(items_with_indices[idx].1)
            && !is_one_row(items_with_indices[idx + 1].1)
    };

    // Compute how many items are visible from a given scroll offset
    let count_visible_from = |offset: usize| -> usize {
        let mut y = 0u16;
        let mut count = 0;
        for i in offset..total_items {
            let h = if is_one_row(items_with_indices[i].1) { 1 } else { art_row_height };
            // Account for spacer row after last one-row item
            let spacer = if has_spacer_after(i) { 1u16 } else { 0 };
            if y + h + spacer > inner.height { break; }
            y += h + spacer;
            count += 1;
        }
        count
    };

    let selected_idx = col.selected_index;

    // Convert selected_idx to display position within the (possibly filtered) list
    let display_selected = if filter_results.is_some() {
        items_with_indices.iter().position(|(idx, _)| *idx == selected_idx).unwrap_or(0)
    } else {
        selected_idx
    };

    // Scroll offset: respect pin, otherwise ensure selected item is visible
    let scroll_offset = match state.scroll.browse {
        Some((pin_col, pinned)) if pin_col == col_idx => pinned,
        _ => {
            // Simple approach: start from 0, advance until selected is visible
            let mut offset = 0;
            loop {
                let visible = count_visible_from(offset);
                if visible == 0 { break; }
                if display_selected >= offset && display_selected < offset + visible {
                    break;
                }
                if display_selected < offset {
                    offset = display_selected;
                    break;
                }
                offset += 1;
            }
            offset
        }
    };

    let visible_count = count_visible_from(scroll_offset);

    let mut row_y = inner.y;
    for vis_row in 0..visible_count {
        let display_idx = scroll_offset + vis_row;
        if display_idx >= total_items {
            break;
        }

        let (orig_idx, item) = items_with_indices[display_idx];
        let is_selected = orig_idx == selected_idx;
        let one_row = is_one_row(item);
        let row_height = if one_row { 1 } else { art_row_height };

        if row_y + row_height > inner.y + inner.height {
            break;
        }

        // Selection highlight background across the full row
        if is_selected {
            let row_area = Rect {
                x: inner.x,
                y: row_y,
                width: inner.width,
                height: row_height,
            };
            let bg_style = Style::default().bg(t.colors.selection_bar_bg);
            frame.render_widget(Block::default().style(bg_style), row_area);
        }

        if one_row {
            // One-row item: full-width text, no art area
            let max_text = inner.width.saturating_sub(2) as usize;
            let display_title = item.title();
            let title_text = truncate_middle(display_title, max_text);
            let title_style = if is_selected {
                Style::default().fg(t.colors.selection_text)
            } else {
                Style::default().fg(t.colors.fg_muted)
            };
            frame.render_widget(
                Paragraph::new(format!(" {}", title_text)).style(title_style),
                Rect { x: inner.x, y: row_y, width: inner.width, height: 1 },
            );
        } else {
            // Art-height item: artwork on left, text on right
            let image_area = Rect {
                x: inner.x,
                y: row_y,
                width: art_width,
                height: row_height,
            };
            let text_x = inner.x + art_width + 1;
            let text_width = inner.width.saturating_sub(art_width + 1);

            // Render album/artist art image or placeholder
            let mut rendered_image = false;
            let art_key = match item {
                BrowseItem::Album { key, .. } => Some(key.as_str()),
                BrowseItem::Artist { key, .. } => Some(key.as_str()),
                BrowseItem::ArtistRadio { artist_key, .. } => Some(artist_key.as_str()),
                BrowseItem::AllTracks { artist_key, .. } => Some(artist_key.as_str()),
                _ => None,
            };
            if let Some(key) = art_key {
                if let Some(data) = state.artwork.grid_cache.get(key) {
                    rendered_image = super::artwork::render_grid_image(frame, image_area, key, data);
                }
            }

            if !rendered_image {
                // Placeholder: centered initials in art area
                let initials: String = item.title()
                    .split_whitespace()
                    .filter_map(|w| w.chars().next())
                    .take(3)
                    .collect();
                let placeholder_text = if state.artwork.grid_pending.contains(item.key()) {
                    "...".to_string()
                } else if initials.is_empty() {
                    "?".to_string()
                } else {
                    initials
                };

                let text_y_p = image_area.y + image_area.height / 2;
                let text_x_p = image_area.x + (image_area.width.saturating_sub(placeholder_text.len() as u16)) / 2;
                if text_y_p < image_area.y + image_area.height {
                    frame.render_widget(
                        Paragraph::new(placeholder_text).style(Style::default().fg(t.colors.fg_muted)),
                        Rect { x: text_x_p, y: text_y_p, width: image_area.width, height: 1 },
                    );
                }
            }

            // Text content to the right of artwork
            if text_width > 2 {
                let max_text = text_width.saturating_sub(1) as usize;

                // Title (line 1, vertically centered in row)
                let display_title = item.title();
                let title_text = truncate_middle(display_title, max_text);
                let title_y = row_y + (row_height / 2).saturating_sub(1);
                let title_style = if is_selected {
                    Style::default().fg(t.colors.selection_text)
                } else {
                    Style::default().fg(t.colors.fg_primary)
                };
                frame.render_widget(
                    Paragraph::new(title_text).style(title_style),
                    Rect { x: text_x, y: title_y, width: text_width, height: 1 },
                );

                // Artist and year (line 2)
                if let BrowseItem::Album { artist, year, .. } = item {
                    let subtitle = if let Some(y) = year {
                        truncate_middle(&format!("{} ({})", artist, y), max_text)
                    } else {
                        truncate_middle(artist, max_text)
                    };
                    let sub_style = Style::default().fg(t.colors.fg_muted);
                    frame.render_widget(
                        Paragraph::new(subtitle).style(sub_style),
                        Rect { x: text_x, y: title_y + 1, width: text_width, height: 1 },
                    );
                }
            }
        }

        row_y += row_height;

        // Add spacer row after last one-row item before art items
        if has_spacer_after(display_idx) {
            row_y += 1;
        }
    }

    // Scrollbar + position indicator
    if total_items > visible_count {
        let sb_border = if is_focused { Some(t.colors.title_focused) } else { None };
        render_scrollbar(frame, col_area, total_items, visible_count, scroll_offset, sb_border);

        let footer = format!("{}/{}", display_selected + 1, total_items);
        let footer_area = Rect::new(
            col_area.x + col_area.width.saturating_sub(footer.len() as u16 + 2),
            col_area.y + col_area.height - 1,
            footer.len() as u16 + 1,
            1,
        );
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().fg(t.colors.fg_muted)),
            footer_area,
        );
    }
}
/// Render the transport bar (always visible, never hijacked by alt bar).
fn render_transport(frame: &mut Frame, state: &AppState, area: Rect) {
    widgets::transport::render(frame, state, area);
}

/// Render the command bar (3 rows: top info/tabs + spacer + contextual commands).
///
/// Top row layout: [library name] [^Q quit] ... [F-keys] [^L library] [^U queue] [^N now playing]
fn render_commands(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Fill background
    let bg = Paragraph::new("").style(Style::default().bg(t.colors.bg_secondary));
    frame.render_widget(bg, area);

    // Split into 3 rows: top (info + F-keys + tabs), spacer, bottom (contextual)
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    // === Top row: all items evenly spaced and centered ===
    //
    // Items: [lib name] [^Q quit] [^L library] [^U queue] [^N now playing] [F1 help] [F2 settings] ...
    //        ╰──────────────────────── evenly spaced across full width ─────────────────────────────╯
    let top_row = rows[0];
    let mut top_items: Vec<(Rect, String)> = Vec::new();
    let mut tab_bar_regions = crate::ui::hit_regions::TabBarRegions {
        library_label: None,
        quit_button: None,
        tabs: Vec::new(),
    };

    if state.view != View::Auth {
        #[derive(Clone, Copy, PartialEq)]
        enum ItemKind { LibLabel, Quit, ViewTab(usize), FKey }
        struct BarItem { key: String, label: String, active: bool, kind: ItemKind }

        let mut all_items: Vec<BarItem> = Vec::new();

        // Library name
        let lib_label = state.active_library.as_ref()
            .and_then(|key| state.libraries.iter().find(|l| &l.key == key))
            .map(|lib_name| {
                if state.has_multiple_servers() {
                    state.active_server_name()
                        .map(|sn| format!("{} ({})", lib_name.title, sn))
                        .unwrap_or_else(|| lib_name.title.clone())
                } else {
                    lib_name.title.clone()
                }
            });
        if let Some(ref name) = lib_label {
            all_items.push(BarItem { key: String::new(), label: format!(" {}", name), active: false, kind: ItemKind::LibLabel });
        }

        // Quit
        all_items.push(BarItem { key: "^Q".into(), label: "quit".into(), active: false, kind: ItemKind::Quit });

        // View tabs
        let view_tabs: [(&str, &str, bool); 3] = [
            ("^L", "library", state.view == View::Browse),
            ("^U", "queue", state.view == View::Queue),
            ("^N", "now playing", state.view == View::NowPlaying),
        ];
        for (i, (key, label, is_active)) in view_tabs.iter().enumerate() {
            all_items.push(BarItem { key: key.to_string(), label: label.to_string(), active: *is_active, kind: ItemKind::ViewTab(i) });
        }

        // F-keys (always show all; highlight active state)
        let f_active = |label: &str| -> bool {
            match label {
                "help" => state.view == View::Help,
                "settings" => state.view == View::Settings,
                _ => false,
            }
        };
        let alt_cmds = crate::app::available_alt_commands(state);
        for cmd in alt_cmds.iter().filter(|c| c.display_key.is_some()) {
            let dk = cmd.display_key.unwrap();
            all_items.push(BarItem { key: dk.to_string(), label: cmd.label.to_string(), active: f_active(cmd.label), kind: ItemKind::FKey });
        }

        // Calculate natural width of each item: " key label " with padding
        let item_widths: Vec<u16> = all_items.iter().map(|item| {
            let k = if item.key.is_empty() { 0 } else { item.key.len() as u16 + 2 }; // " key "
            let l = item.label.len() as u16 + 1; // "label "
            k + l
        }).collect();
        let content_width: u16 = item_widths.iter().sum();
        let n = all_items.len() as u16;

        // Distribute items evenly across the row
        let total_gap = top_row.width.saturating_sub(content_width);
        let gap = if n > 1 { total_gap / (n - 1) } else { 0 };
        let extra = if n > 1 { (total_gap % (n - 1)) as usize } else { 0 };
        // Center the whole block
        let block_width = content_width + gap * (n.saturating_sub(1)) + extra as u16;
        let start_x = top_row.x + top_row.width.saturating_sub(block_width) / 2;

        let mut cx = start_x;
        for (i, item) in all_items.iter().enumerate() {
            // Add gap before items (except the first)
            if i > 0 {
                let g = gap + if i <= extra { 1 } else { 0 };
                cx += g;
            }

            let has_key = !item.key.is_empty();
            let key_text = if has_key { format!(" {} ", item.key) } else { String::new() };
            let label_text = format!("{} ", item.label);
            let kw = key_text.len() as u16;
            let lw = label_text.len() as u16;

            if cx + kw + lw > top_row.x + top_row.width { break; }

            let (key_style, label_style) = if item.active {
                (
                    Style::default().fg(t.colors.fg_accent).bg(t.colors.bg_primary).add_modifier(ratatui::style::Modifier::BOLD),
                    Style::default().fg(t.colors.fg_accent).bg(t.colors.bg_primary).add_modifier(ratatui::style::Modifier::BOLD),
                )
            } else if matches!(item.kind, ItemKind::LibLabel) {
                (
                    Style::default().fg(t.colors.fg_accent_dim).bg(t.colors.bg_secondary),
                    Style::default().fg(t.colors.fg_accent_dim).bg(t.colors.bg_secondary),
                )
            } else {
                (
                    Style::default().fg(t.colors.shortcut_key).bg(t.colors.bg_primary),
                    Style::default().fg(t.colors.shortcut_text).bg(t.colors.bg_secondary),
                )
            };

            if kw > 0 {
                frame.render_widget(Paragraph::new(key_text.as_str()).style(key_style),
                    Rect { x: cx, y: top_row.y, width: kw, height: 1 });
            }
            frame.render_widget(Paragraph::new(label_text.as_str()).style(label_style),
                Rect { x: cx + kw, y: top_row.y, width: lw, height: 1 });

            let full_area = Rect { x: cx, y: top_row.y, width: kw + lw, height: 1 };
            match item.kind {
                ItemKind::LibLabel => { tab_bar_regions.library_label = Some(full_area); }
                ItemKind::Quit => { tab_bar_regions.quit_button = Some(full_area); }
                ItemKind::ViewTab(idx) => { tab_bar_regions.tabs.push((full_area, idx)); }
                ItemKind::FKey => { top_items.push((full_area, format!("fkey:{}", item.key))); }
            }
            cx += kw + lw;
        }
    }

    // Register tab bar hit regions
    {
        let mut hr = state.hit_regions.borrow_mut();
        hr.tab_bar = Some(tab_bar_regions);
    }

    // rows[1] is spacer — already filled by bg
    let bottom_items = render_command_row(frame, state, rows[2], false); // Contextual commands

    // Register command bar hit regions
    {
        let mut hr = state.hit_regions.borrow_mut();
        hr.command_bar = Some(crate::ui::hit_regions::CommandBarRegions {
            top_row: top_items,
            bottom_row: bottom_items,
        });
    }
}

/// Render the bottom command row — always shows all commands, evenly spaced.
/// Returns (Rect, action_key) pairs for hit-test registration.
fn render_command_row(frame: &mut Frame, state: &AppState, area: Rect, _is_top_row: bool) -> Vec<(Rect, String)> {
    let t = theme();
    let alt_cmds = crate::app::available_alt_commands(state);

    // All non-F-key commands, always shown (not hidden based on context)
    let mut filtered: Vec<&crate::app::AltCommand> = alt_cmds.iter()
        .filter(|cmd| cmd.display_key.is_none())
        .collect();

    filtered.sort_by(|a, b| {
        let mod_order = |m: &crate::app::CommandModifier| match m {
            crate::app::CommandModifier::Ctrl => 0,
            crate::app::CommandModifier::Alt => 1,
            crate::app::CommandModifier::None => 2,
        };
        mod_order(&a.modifier).cmp(&mod_order(&b.modifier))
            .then(a.key.cmp(&b.key))
    });

    if filtered.is_empty() {
        return vec![];
    }

    struct CmdItem { key_str: String, label_str: String, action_key: String }
    let items: Vec<CmdItem> = filtered.iter().map(|cmd| {
        let key_str = match cmd.modifier {
            crate::app::CommandModifier::Ctrl => format!(" ^{} ", cmd.key.to_ascii_uppercase()),
            crate::app::CommandModifier::Alt => format!(" \u{2325}{} ", cmd.key.to_ascii_uppercase()),
            crate::app::CommandModifier::None => format!(" {} ", cmd.key.to_ascii_uppercase()),
        };
        let label_str = format!("{} ", cmd.label);
        let action_key = format!("{}:{}", match cmd.modifier {
            crate::app::CommandModifier::Ctrl => "ctrl",
            crate::app::CommandModifier::Alt => "alt",
            crate::app::CommandModifier::None => "none",
        }, cmd.key);
        CmdItem { key_str, label_str, action_key }
    }).collect();

    // Evenly distribute across the row
    let item_widths: Vec<u16> = items.iter().map(|i| i.key_str.len() as u16 + i.label_str.len() as u16).collect();
    let content_width: u16 = item_widths.iter().sum();
    let n = items.len() as u16;
    let total_gap = area.width.saturating_sub(content_width);
    let gap = if n > 1 { total_gap / (n - 1) } else { 0 };
    let extra = if n > 1 { (total_gap % (n - 1)) as usize } else { 0 };
    let block_width = content_width + gap * n.saturating_sub(1) + extra as u16;
    let start_x = area.x + area.width.saturating_sub(block_width) / 2;

    let mut hit_items: Vec<(Rect, String)> = Vec::new();
    let mut cx = start_x;

    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            cx += gap + if i <= extra { 1 } else { 0 };
        }

        let kw = item.key_str.len() as u16;
        let lw = item.label_str.len() as u16;
        if cx + kw + lw > area.x + area.width { break; }

        // All commands always shown with normal styling (not context-dimmed)
        frame.render_widget(Paragraph::new(item.key_str.as_str())
            .style(Style::default().fg(t.colors.shortcut_key).bg(t.colors.bg_primary)),
            Rect { x: cx, y: area.y, width: kw, height: 1 });
        frame.render_widget(Paragraph::new(item.label_str.as_str())
            .style(Style::default().fg(t.colors.shortcut_text).bg(t.colors.bg_secondary)),
            Rect { x: cx + kw, y: area.y, width: lw, height: 1 });

        hit_items.push((Rect { x: cx, y: area.y, width: kw + lw, height: 1 }, item.action_key.clone()));
        cx += kw + lw;
    }

    hit_items
}

/// Render the search popup as a floating dialog.
/// Render the library picker popup (F3).
fn render_library_picker(frame: &mut Frame, state: &AppState) {
    let t = theme();
    let area = centered_rect(50, 30, frame.area());

    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" switch library ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border_focused))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Build flat list of all libraries across servers
    let multi_server = state.has_multiple_servers();
    let all_libs = if multi_server {
        state.all_libraries_with_servers()
    } else {
        // Single server — use current libraries
        let server_id = state.active_server_id.as_deref().unwrap_or("");
        let server_name = state.active_server_name().unwrap_or("");
        state.libraries.iter()
            .map(|lib| (server_id, server_name, lib))
            .collect()
    };

    if all_libs.is_empty() {
        let msg = Paragraph::new("No libraries available")
            .style(Style::default().fg(t.colors.fg_muted));
        frame.render_widget(msg, inner);
        return;
    }

    // Register hit regions for mouse handler
    {
        let mut hr = state.hit_regions.borrow_mut();
        hr.library_picker = Some(crate::ui::hit_regions::PopupListRegions {
            outer: area,
            items_area: inner,
            item_count: all_libs.len(),
        });
    }

    // Build library list items
    let items: Vec<ListItem> = all_libs.iter().enumerate().map(|(i, (server_id, server_name, lib))| {
        let is_selected = i == state.popups.library_picker_index;
        let is_active = state.active_library.as_deref() == Some(lib.key.as_str())
            && state.active_server_id.as_deref() == Some(*server_id);

        let prefix = if is_selected { "\u{266a} " } else { "  " };
        let suffix = if is_active { " *" } else { "" };
        let text = if multi_server {
            format!("{}{} ({}){}", prefix, lib.title, server_name, suffix)
        } else {
            format!("{}{}{}", prefix, lib.title, suffix)
        };

        let style = if is_selected {
            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
        } else {
            Style::default().fg(t.colors.fg_primary)
        };

        ListItem::new(text).style(style)
    }).collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Render the artist bio popup (F4).
fn render_artist_bio_popup(frame: &mut Frame, state: &AppState) {
    let popup = match &state.popups.artist_bio {
        Some(p) => p,
        None => return,
    };

    let t = theme();
    let area = centered_rect(70, 60, frame.area());

    frame.render_widget(Clear, area);

    let title = format!(" {} ", popup.artist_name);
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border_focused))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if popup.loading {
        let loading = Paragraph::new("Loading biography...")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(loading, inner);
        return;
    }

    let bio_area = inner;

    // Determine artwork size and whether to show it
    let has_artwork = popup.artwork_data.is_some() && popup.artwork_thumb.is_some();
    // Drive layout from height: fill ~60% of bio area vertically, then derive width
    // from height so a square image fills the rect exactly (terminal cells ≈ 2:1 aspect).
    let art_h = if has_artwork {
        let target = (bio_area.height * 3) / 5; // ~60% of bio area
        target.max(6).min(bio_area.height.saturating_sub(2))
    } else { 0 };
    let art_w = if has_artwork { (art_h * 2).min(bio_area.width / 2) } else { 0 };
    // 1 col gap between text and artwork
    let gap = if has_artwork && art_w > 0 { 1u16 } else { 0 };

    // Word-wrap bio text: narrow lines next to artwork, full-width lines below
    let full_width = bio_area.width as usize;
    let narrow_width = bio_area.width.saturating_sub(art_w + gap) as usize;
    let art_rows = art_h as usize;
    let wrapped = wrap_bio_text(&popup.bio, narrow_width, full_width, art_rows);
    let total_lines = wrapped.len() as u16;
    let visible = bio_area.height;
    let scroll = popup.scroll.min(total_lines.saturating_sub(visible));

    // Render artwork scrolling with text: crop top rows as user scrolls down.
    let art_visible_h = art_h.saturating_sub(scroll);
    if has_artwork && art_w > 0 && art_visible_h > 0 {
        let art_rect = Rect {
            x: bio_area.x + bio_area.width - art_w,
            y: bio_area.y,
            width: art_w,
            height: art_visible_h,
        };
        if let (Some(ref data), Some(ref thumb)) = (&popup.artwork_data, &popup.artwork_thumb) {
            BIO_ARTWORK_RENDERER.with(|renderer| {
                let mut renderer = renderer.borrow_mut();
                let crop_fraction = if scroll > 0 { scroll as f32 / art_h as f32 } else { 0.0 };
                if renderer.load_image_cropped(data, thumb, crop_fraction) {
                    renderer.render(frame, art_rect);
                }
            });
        }
    }

    // Render visible text lines
    let style = Style::default().fg(t.colors.fg_primary);
    for (screen_row, line_text) in wrapped.iter().skip(scroll as usize).take(visible as usize).enumerate() {
        let y = bio_area.y + screen_row as u16;
        // Narrow width only when artwork is visible on this screen row
        let in_art_zone = has_artwork && (screen_row as u16) < art_visible_h;
        let line_width = if in_art_zone { narrow_width as u16 } else { bio_area.width };
        let line_rect = Rect {
            x: bio_area.x,
            y,
            width: line_width,
            height: 1,
        };
        let p = Paragraph::new(line_text.as_str()).style(style);
        frame.render_widget(p, line_rect);
    }

    // Scrollbar
    if total_lines > visible {
        render_scrollbar(frame, area, total_lines as usize, visible as usize, scroll as usize, None);
    }
}

/// Word-wrap bio text with a narrow region (next to artwork) and full-width below.
/// The first `narrow_rows` output lines are wrapped at `narrow_width`;
/// subsequent lines are wrapped at `full_width`.
fn wrap_bio_text(text: &str, narrow_width: usize, full_width: usize, narrow_rows: usize) -> Vec<String> {
    if full_width == 0 {
        return vec![];
    }
    // If no artwork, everything is full width
    let narrow_width = if narrow_width == 0 || narrow_width >= full_width { full_width } else { narrow_width };

    let mut lines = Vec::new();

    for paragraph in text.split('\n') {
        if paragraph.trim().is_empty() {
            lines.push(String::new());
            continue;
        }

        let words: Vec<&str> = paragraph.split_whitespace().collect();
        let mut line = String::new();

        for word in &words {
            let max_w = if lines.len() < narrow_rows { narrow_width } else { full_width };
            if line.is_empty() {
                line.push_str(word);
            } else if line.len() + 1 + word.len() <= max_w {
                line.push(' ');
                line.push_str(word);
            } else {
                lines.push(line);
                line = word.to_string();
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }

    lines
}

fn render_error_popup(frame: &mut Frame, error: &str) {
    let t = theme();
    let area = centered_rect(60, 20, frame.area());

    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Error ")
        .title_style(Style::default().fg(t.colors.error))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.error))
        .style(Style::default().bg(t.colors.bg_primary));

    let text = Paragraph::new(error)
        .style(Style::default().fg(t.colors.error))
        .wrap(Wrap { trim: true })
        .block(block);

    frame.render_widget(text, area);
}

fn render_input_dialog(frame: &mut Frame, dialog: &InputDialog) {
    let t = theme();
    // Use 50% width and 25% height to ensure the dialog is visible
    let area = centered_rect(50, 25, frame.area());

    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(format!(" {} ", dialog.title))
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split inner area for input and hint
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // input label
            Constraint::Length(1),  // input field
            Constraint::Length(1),  // hint
        ])
        .split(inner);

    // Input field with cursor
    let input_text = format!("{}▋", dialog.input);
    let input = Paragraph::new(input_text)
        .style(Style::default().fg(t.colors.fg_primary));
    frame.render_widget(input, chunks[1]);

    // Hint text
    let hint = Paragraph::new("Enter: Save  |  Esc: Cancel")
        .style(Style::default().fg(t.colors.fg_muted))
        .alignment(Alignment::Center);
    frame.render_widget(hint, chunks[2]);
}

fn render_confirm_dialog(frame: &mut Frame, state: &AppState, dialog: &ConfirmDialog) {
    let t = theme();
    let area = centered_rect(50, 25, frame.area());

    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(format!(" {} ", dialog.title))
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border_focused))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Message
    let msg = Paragraph::new(dialog.message.as_str())
        .style(Style::default().fg(t.colors.fg_primary))
        .wrap(Wrap { trim: true });
    let msg_area = Rect { height: inner.height.saturating_sub(2), ..inner };
    frame.render_widget(msg, msg_area);

    // Button row at bottom of inner area
    let btn_y = inner.y + inner.height.saturating_sub(1);
    let yes_text = "  Yes  ";
    let no_text = "  No  ";
    let yes_x = inner.x + 1;
    let no_x = yes_x + yes_text.len() as u16 + 2;

    let yes_area = Rect { x: yes_x, y: btn_y, width: yes_text.len() as u16, height: 1 };
    let no_area = Rect { x: no_x, y: btn_y, width: no_text.len() as u16, height: 1 };

    // Register hit regions
    {
        let mut hr = state.hit_regions.borrow_mut();
        hr.confirm_dialog = Some(crate::ui::hit_regions::DialogRegions {
            outer: area,
            yes_button: yes_area,
            no_button: no_area,
        });
    }

    // Highlight the selected button with accent, dim the other
    let (yes_style, no_style) = if dialog.selected_yes {
        (
            Style::default().fg(t.colors.bg_primary).bg(t.colors.fg_accent),
            Style::default().fg(t.colors.fg_muted).bg(t.colors.bg_secondary),
        )
    } else {
        (
            Style::default().fg(t.colors.fg_muted).bg(t.colors.bg_secondary),
            Style::default().fg(t.colors.bg_primary).bg(t.colors.fg_accent),
        )
    };

    frame.render_widget(Paragraph::new(yes_text).style(yes_style), yes_area);
    frame.render_widget(Paragraph::new(no_text).style(no_style), no_area);

    // Hint text
    let hint_y = btn_y.saturating_sub(1);
    if hint_y > inner.y + 1 {
        let hint = Paragraph::new("Y/N or Enter to confirm")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        let hint_area = Rect { x: inner.x, y: hint_y, width: inner.width, height: 1 };
        frame.render_widget(hint, hint_area);
    }
}

/// Check if a mouse click hit a confirm dialog button. Returns Some(true) for Yes, Some(false) for No, None for miss.
pub fn confirm_dialog_hit_test(dialog: &ConfirmDialog, frame_area: Rect, col: u16, row: u16) -> Option<bool> {
    let area = centered_rect(50, 25, frame_area);
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(area);

    let btn_y = inner.y + inner.height.saturating_sub(1);
    if row != btn_y { return None; }

    let yes_text = "  Yes  ";
    let no_text = "  No  ";
    let yes_x = inner.x + 1;
    let no_x = yes_x + yes_text.len() as u16 + 2;

    let _ = dialog; // used for lifetime/future extensibility
    if col >= yes_x && col < yes_x + yes_text.len() as u16 {
        Some(true)
    } else if col >= no_x && col < no_x + no_text.len() as u16 {
        Some(false)
    } else {
        None
    }
}

fn render_toast(frame: &mut Frame, message: &str, area: Rect) {
    let t = theme();
    let padded_message = format!(" {} ", message);
    let width = (padded_message.len().min(50)) as u16;

    let toast_area = Rect {
        x: area.width.saturating_sub(width + 1),
        y: area.height.saturating_sub(4), // Above transport bar
        width,
        height: 1,
    };

    frame.render_widget(Clear, toast_area);
    let text = Paragraph::new(padded_message)
        .style(Style::default()
            .fg(t.colors.fg_primary)
            .bg(t.colors.fg_accent));
    frame.render_widget(text, toast_area);
}
