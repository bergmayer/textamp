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
    let current_track_key = state.current_track().map(|t| t.rating_key.as_str());

    // All browse categories use dynamic Miller columns
    match state.browse_category {
        BrowseCategory::Library => {
            // Get filter info if filter applies to this category
            let (filter_results, filter_column) = if state.list_filter.active
                && state.list_filter.category == BrowseCategory::Library {
                (state.list_filter.results.as_ref(), Some(state.list_filter.column))
            } else {
                (None, None)
            };

            // Artists view with dynamic Miller columns
            let title = "artists";
            render_browse_miller_columns(
                frame,
                state,
                &state.artist_nav,
                title,
                current_track_key,
                filter_results,
                filter_column,
                false,
                layout.left_panel,
                layout.right_panel,
            );
        }
        BrowseCategory::Playlists => {
            // Get filter info if filter applies to this category
            let (filter_results, filter_column) = if state.list_filter.active
                && state.list_filter.category == BrowseCategory::Playlists {
                (state.list_filter.results.as_ref(), Some(state.list_filter.column))
            } else {
                (None, None)
            };

            // Playlists view with dynamic Miller columns (no tab bar)
            render_browse_miller_columns(
                frame,
                state,
                &state.playlist_nav,
                "playlists",
                current_track_key,
                filter_results,
                filter_column,
                true,
                layout.left_panel,
                layout.right_panel,
            );
        }
        BrowseCategory::Genres => {
            // Get filter info if filter applies to this category
            let (filter_results, filter_column) = if state.list_filter.active
                && state.list_filter.category == BrowseCategory::Genres {
                (state.list_filter.results.as_ref(), Some(state.list_filter.column))
            } else {
                (None, None)
            };

            // Genres view with dynamic Miller columns (category selector in column 0)
            render_browse_miller_columns(
                frame,
                state,
                &state.genre_nav,
                "genres",
                current_track_key,
                filter_results,
                filter_column,
                false,
                layout.left_panel,
                layout.right_panel,
            );
        }
        BrowseCategory::Folders => {
            // Get filter info if filter applies to this category
            let (filter_results, filter_column) = if state.list_filter.active
                && state.list_filter.category == BrowseCategory::Folders {
                (state.list_filter.results.as_ref(), Some(state.list_filter.column))
            } else {
                (None, None)
            };

            // Folder browsing mode - existing Miller columns implementation
            render_folder_view(frame, state, filter_results, filter_column, layout.left_panel, layout.right_panel);
        }
    }

    // Chrome: tab bar, transport, command bar
    render_tab_bar_nav(frame, state, layout.tab_bar);
    render_transport(frame, state, layout.transport);
    render_commands(frame, state, layout.commands);
}

fn render_queue(frame: &mut Frame, state: &AppState) {
    let layout = FullScreenLayout::new(frame.area());

    render_tab_bar_nav(frame, state, layout.tab_bar);
    screens::now_playing::render_queue_mode(frame, state, layout.content);
    render_transport(frame, state, layout.transport);
    render_commands(frame, state, layout.commands);
}

/// Compute the queue right panel (track list) area for popup centering.
/// Replicates the layout calculation from render_queue_mode.
fn render_now_playing(frame: &mut Frame, state: &AppState) {
    let layout = FullScreenLayout::new(frame.area());

    render_tab_bar_nav(frame, state, layout.tab_bar);
    screens::now_playing::render_visualizer_mode(frame, state, layout.content);
    render_transport(frame, state, layout.transport);
    render_commands(frame, state, layout.commands);
}

fn render_search(frame: &mut Frame, state: &AppState) {
    let layout = FullScreenLayout::new(frame.area());

    render_tab_bar_nav(frame, state, layout.tab_bar);
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

    render_tab_bar_nav(frame, state, layout.tab_bar);
    screens::help::render(frame, state, layout.content);
    render_transport(frame, state, layout.transport);
    render_commands(frame, state, layout.commands);
}

fn render_settings(frame: &mut Frame, state: &AppState) {
    let layout = FullScreenLayout::new(frame.area());

    render_tab_bar_nav(frame, state, layout.tab_bar);
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
        let effective_columns = last_meaningful + 1;

        // Calculate column width - show up to 3 columns
        let max_visible = 3.min(effective_columns);
        let col_width = area.width / max_visible as u16;

        // Determine which columns to show (always include focused column)
        let start_col = if folder_state.focused_column + 1 > max_visible {
            folder_state.focused_column + 1 - max_visible
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
            let is_focused = col_idx == folder_state.focused_column;

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
            && state.library_sub_mode != crate::app::state::LibrarySubMode::Normal
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

    // Show up to 3 columns at a time; always at least 2 (Library/Genre/Playlist always
    // show a child column, even if empty — Folders is the exception, handled separately)
    let max_visible = 3.min(layout_columns).max(2);
    let col_width = area.width / max_visible as u16;

    // Determine which columns to show (always include focused column)
    let start_col = if nav.focused_column + 1 > max_visible {
        nav.focused_column + 1 - max_visible
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
        let is_focused = col_idx == nav.focused_column;
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

/// Render the top tab bar with library name and navigation tabs.
fn render_tab_bar_nav(frame: &mut Frame, state: &AppState, area: Rect) {
    use crate::app::state::AuthStep;
    let t = theme();

    // Auth view: show auth hints instead of tabs
    if state.view == View::Auth {
        let hint = match state.auth_state.step {
            AuthStep::Login => {
                if state.auth_state.editing {
                    "Enter: done | Esc: cancel | Tab: next field"
                } else {
                    "Enter: edit/submit | Tab/Arrows: navigate"
                }
            }
            AuthStep::ServerSelect => "Enter: connect | Arrows: select",
            _ => "",
        };
        let paragraph = Paragraph::new(hint)
            .style(Style::default().fg(t.colors.fg_muted).bg(t.colors.bg_secondary))
            .alignment(Alignment::Center);
        frame.render_widget(paragraph, area);
        return;
    }

    // Library name (left-aligned)
    let lib_label = if let Some(lib_name) = state.active_library.as_ref()
        .and_then(|key| state.libraries.iter().find(|l| &l.key == key))
    {
        if state.has_multiple_servers() {
            if let Some(server_name) = state.active_server_name() {
                format!(" {} ({}) ", lib_name.title, server_name)
            } else {
                format!(" {} ", lib_name.title)
            }
        } else {
            format!(" {} ", lib_name.title)
        }
    } else {
        String::new()
    };

    // Navigation tabs (centered in remaining space)
    let tabs: Vec<(&str, &str, bool)> = vec![
        ("^L", "library", state.view == View::Browse && state.browse_category == BrowseCategory::Library),
        ("^P", "playlists", state.view == View::Browse && state.browse_category == BrowseCategory::Playlists),
        ("^G", "genres", state.view == View::Browse && state.browse_category == BrowseCategory::Genres),
        ("^O", "folders", state.view == View::Browse && state.browse_category == BrowseCategory::Folders),
        ("^U", "queue", state.view == View::Queue),
        ("^N", "now playing", state.view == View::NowPlaying),
    ];

    // Render library name + quit button on the left
    let mut tab_bar_regions = crate::ui::hit_regions::TabBarRegions {
        library_label: None,
        quit_button: None,
        tabs: Vec::new(),
    };

    {
        // Fill background first
        let bg = Paragraph::new("").style(Style::default().bg(t.colors.bg_secondary));
        frame.render_widget(bg, area);

        if !lib_label.is_empty() {
            let lib_span = Paragraph::new(
                Span::styled(&lib_label, Style::default().fg(t.colors.fg_accent_dim).bg(t.colors.bg_secondary))
            ).style(Style::default().bg(t.colors.bg_secondary));
            let lib_width = lib_label.len() as u16;
            let lib_area = Rect { width: lib_width.min(area.width), ..area };
            frame.render_widget(lib_span, lib_area);

            tab_bar_regions.library_label = Some(lib_area);

            // Quit button (styled like bottom bar shortcut buttons)
            let quit_key = " ^Q ";
            let quit_label = "quit ";
            let quit_x = area.x + lib_width;
            let quit_total_width = (quit_key.len() + quit_label.len()) as u16;
            let quit_key_area = Rect { x: quit_x, y: area.y, width: quit_key.len() as u16, height: 1 };
            let quit_label_area = Rect { x: quit_x + quit_key.len() as u16, y: area.y, width: quit_label.len() as u16, height: 1 };
            if quit_x + quit_total_width <= area.x + area.width {
                frame.render_widget(
                    Paragraph::new(quit_key).style(Style::default().fg(t.colors.shortcut_key).bg(t.colors.bg_primary)),
                    quit_key_area,
                );
                frame.render_widget(
                    Paragraph::new(quit_label).style(Style::default().fg(t.colors.shortcut_text).bg(t.colors.bg_secondary)),
                    quit_label_area,
                );
                tab_bar_regions.quit_button = Some(Rect {
                    x: quit_x, y: area.y, width: quit_total_width, height: 1,
                });
            }
        }
    }

    // Build tab spans and compute absolute tab positions for hit regions
    let mut spans: Vec<Span> = Vec::new();
    let mut tab_total_width: u16 = 0;
    let mut tab_positions: Vec<(u16, u16, usize)> = Vec::new(); // (rel_start, width, tab_idx)

    for (i, (key, label, is_current)) in tabs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" ", Style::default().bg(t.colors.bg_secondary)));
            tab_total_width += 1; // separator
        }

        let tab_text = format!(" {} {} ", key, label);
        let tab_width = tab_text.len() as u16;
        tab_positions.push((tab_total_width, tab_width, i));
        tab_total_width += tab_width;

        if *is_current {
            spans.push(Span::styled(
                tab_text,
                Style::default()
                    .fg(t.colors.fg_accent)
                    .bg(t.colors.bg_primary)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                tab_text,
                Style::default().fg(t.colors.fg_muted).bg(t.colors.bg_secondary),
            ));
        }
    }

    // Compute centered absolute positions for tab hit regions
    let tab_center_offset = area.x + area.width.saturating_sub(tab_total_width) / 2;
    for (rel_start, width, idx) in &tab_positions {
        tab_bar_regions.tabs.push((
            Rect { x: tab_center_offset + rel_start, y: area.y, width: *width, height: 1 },
            *idx,
        ));
    }

    // Register tab bar hit regions
    {
        let mut hr = state.hit_regions.borrow_mut();
        hr.tab_bar = Some(tab_bar_regions);
    }

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line)
        .style(Style::default().bg(t.colors.bg_secondary))
        .alignment(Alignment::Center);
    frame.render_widget(paragraph, area);
}

/// Render the always-visible command bar (3 rows: function keys + spacer + contextual commands).
fn render_commands(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Fill background
    let bg = Paragraph::new("").style(Style::default().bg(t.colors.bg_secondary));
    frame.render_widget(bg, area);

    // Split into 3 rows: top commands, spacer, bottom commands
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let top_items = render_command_row(frame, state, rows[0], true);  // Function keys
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

/// Render one row of the command bar (top = function keys, bottom = contextual commands).
/// Returns (Rect, action_key) pairs for hit-test registration.
fn render_command_row(frame: &mut Frame, state: &AppState, area: Rect, is_top_row: bool) -> Vec<(Rect, String)> {
    let t = theme();
    let alt_cmds = crate::app::available_alt_commands(state);

    let mut spans: Vec<Span> = Vec::new();

    // Split commands: function keys (display_key set) go on top, others on bottom
    let mut filtered: Vec<&crate::app::AltCommand> = alt_cmds.iter()
        .filter(|cmd| {
            if is_top_row {
                cmd.display_key.is_some()
            } else {
                cmd.display_key.is_none()
            }
        })
        .collect();

    // Sort bottom row by modifier (Ctrl < Alt < None) then alphabetically by key
    if !is_top_row {
        filtered.sort_by(|a, b| {
            let mod_order = |m: &crate::app::CommandModifier| match m {
                crate::app::CommandModifier::Ctrl => 0,
                crate::app::CommandModifier::Alt => 1,
                crate::app::CommandModifier::None => 2,
            };
            mod_order(&a.modifier).cmp(&mod_order(&b.modifier))
                .then(a.key.cmp(&b.key))
        });
    }

    // Track positions for hit-test registration
    let mut total_width: u16 = 0;
    let mut item_positions: Vec<(u16, u16, String)> = Vec::new(); // (rel_start, width, action_key)

    for (i, cmd) in filtered.iter().enumerate() {
        let separator_width: u16 = if i > 0 { 1 } else { 0 };
        if i > 0 {
            spans.push(Span::styled(" ", Style::default().bg(t.colors.bg_secondary)));
        }
        // Button-styled: key has a contrasting background
        let key_str = if let Some(dk) = cmd.display_key {
            format!(" {} ", dk)
        } else {
            match cmd.modifier {
                crate::app::CommandModifier::Ctrl => format!(" ^{} ", cmd.key.to_ascii_uppercase()),
                crate::app::CommandModifier::Alt => format!(" \u{2325}{} ", cmd.key.to_ascii_uppercase()),
                crate::app::CommandModifier::None => format!(" {} ", cmd.key.to_ascii_uppercase()),
            }
        };
        let label_str = format!("{} ", cmd.label);
        let item_width = key_str.len() as u16 + label_str.len() as u16;

        // Build action key for lookup (use display_key for F-keys to disambiguate)
        let action_key = if let Some(dk) = cmd.display_key {
            format!("fkey:{}", dk)
        } else {
            format!("{}:{}", match cmd.modifier {
                crate::app::CommandModifier::Ctrl => "ctrl",
                crate::app::CommandModifier::Alt => "alt",
                crate::app::CommandModifier::None => "none",
            }, cmd.key)
        };

        let rel_start = total_width + separator_width;
        item_positions.push((rel_start, item_width, action_key));
        total_width = rel_start + item_width;

        if cmd.enabled {
            spans.push(Span::styled(
                key_str,
                Style::default().fg(t.colors.shortcut_key).bg(t.colors.bg_primary),
            ));
            spans.push(Span::styled(
                label_str,
                Style::default().fg(t.colors.shortcut_text).bg(t.colors.bg_secondary),
            ));
        } else {
            // Disabled: dimmed key and label
            spans.push(Span::styled(
                key_str,
                Style::default().fg(t.colors.fg_muted).bg(t.colors.bg_secondary),
            ));
            spans.push(Span::styled(
                label_str,
                Style::default().fg(t.colors.fg_muted).bg(t.colors.bg_secondary),
            ));
        }
    }

    // Compute absolute positions (centered)
    let center_offset = area.x + area.width.saturating_sub(total_width) / 2;
    let hit_items: Vec<(Rect, String)> = item_positions.into_iter().map(|(rel_start, width, key)| {
        (Rect { x: center_offset + rel_start, y: area.y, width, height: 1 }, key)
    }).collect();

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line)
        .style(Style::default().bg(t.colors.bg_secondary))
        .alignment(Alignment::Center);
    frame.render_widget(paragraph, area);

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
