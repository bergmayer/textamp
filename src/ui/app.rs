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

use crate::app::state::{View, BrowseCategory, GenreContentType, InputDialog, ConfirmDialog};
use crate::app::AppState;
use crate::services::NavigationService;
use super::layout::{AppLayout, FullScreenLayout, centered_rect};
use super::screens;
use super::theme::theme;
use super::widgets;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

/// Render the entire UI.
pub fn render(frame: &mut Frame, state: &AppState) {
    // Fill entire background with theme color
    let t = theme();
    frame.render_widget(Block::default().style(Style::default().bg(t.colors.bg_primary)), frame.area());

    match state.view {
        View::Auth => render_auth(frame, state),
        View::Browse => render_browse(frame, state),
        View::NowPlaying => render_now_playing(frame, state),
        View::Search => render_search(frame, state),
        View::Similar => render_similar(frame, state),
        View::Help => render_help(frame, state),
        View::Settings => render_settings(frame, state),
    }

    // Render search popup if active (floating dialog)
    if state.search_popup_active {
        render_search_popup(frame, state);
    }

    // Render library picker popup if active
    if state.library_picker_active {
        render_library_picker(frame, state);
    }

    // Render error popup if present
    if let Some(ref error) = state.last_error {
        render_error_popup(frame, error);
    }

    // Render input dialog if present
    if let Some(ref dialog) = state.input_dialog {
        render_input_dialog(frame, dialog);
    }

    // Render confirm dialog if present
    if let Some(ref dialog) = state.confirm_dialog {
        render_confirm_dialog(frame, dialog);
    }

    // Render toast notification if present (bottom-right, non-blocking)
    if let Some(ref toast) = state.toast_message {
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
        BrowseCategory::Artists => {
            // Get filter info if filter applies to this category
            let (filter_results, filter_column) = if state.list_filter.active
                && state.list_filter.category == BrowseCategory::Artists {
                (state.list_filter.results.as_ref(), Some(state.list_filter.column))
            } else {
                (None, None)
            };

            // Artists view with dynamic Miller columns
            let title = state.artist_view_mode.name();
            render_browse_miller_columns(
                frame,
                state,
                &state.artist_nav,
                title,
                current_track_key,
                filter_results,
                filter_column,
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

            if state.playlists_mode == crate::app::state::PlaylistsMode::Stations {
                // Stations mode - render station_nav Miller columns
                render_station_view(frame, state, filter_results, filter_column, layout.left_panel, layout.right_panel);
            } else {
                // Playlists view with dynamic Miller columns
                let title = state.playlists_mode.name();
                render_browse_miller_columns(
                    frame,
                    state,
                    &state.playlist_nav,
                    title,
                    current_track_key,
                    filter_results,
                    filter_column,
                    layout.left_panel,
                    layout.right_panel,
                );
            }
        }
        BrowseCategory::Genres => {
            // Genres cycle includes Stations
            if state.genre_content_type == GenreContentType::Stations {
                // Get filter info if filter applies to stations (Genres category with Stations type)
                let (filter_results, filter_column) = if state.list_filter.active
                    && state.list_filter.category == BrowseCategory::Genres {
                    (state.list_filter.results.as_ref(), Some(state.list_filter.column))
                } else {
                    (None, None)
                };

                // Stations view
                render_station_view(frame, state, filter_results, filter_column, layout.left_panel, layout.right_panel);
            } else {
                // Get filter info if filter applies to this category
                let (filter_results, filter_column) = if state.list_filter.active
                    && state.list_filter.category == BrowseCategory::Genres {
                    (state.list_filter.results.as_ref(), Some(state.list_filter.column))
                } else {
                    (None, None)
                };

                // Genres with dynamic Miller columns
                let title = state.genre_content_type.name();
                render_browse_miller_columns(
                    frame,
                    state,
                    &state.genre_nav,
                    title,
                    current_track_key,
                    filter_results,
                    filter_column,
                    layout.left_panel,
                    layout.right_panel,
                );
            }
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

    // Transport bar
    render_transport(frame, state, layout.transport);

    // Shortcut bar
    render_shortcuts(frame, state, layout.shortcuts);
}

fn render_now_playing(frame: &mut Frame, state: &AppState) {
    let layout = FullScreenLayout::new(frame.area());

    screens::now_playing::render(frame, state, layout.content);
    render_transport(frame, state, layout.transport);
    render_shortcuts(frame, state, layout.shortcuts);
}

fn render_search(frame: &mut Frame, state: &AppState) {
    let layout = FullScreenLayout::new(frame.area());

    // Unified search/filter screen handles all tabs including Global (with 3-column layout)
    screens::filter::render(frame, state, layout.content);
    render_transport(frame, state, layout.transport);
    render_shortcuts(frame, state, layout.shortcuts);
}

fn render_similar(frame: &mut Frame, state: &AppState) {
    let layout = FullScreenLayout::new(frame.area());

    screens::similar::render(frame, state, layout.content);
    render_transport(frame, state, layout.transport);
    render_shortcuts(frame, state, layout.shortcuts);
}

fn render_help(frame: &mut Frame, state: &AppState) {
    let layout = FullScreenLayout::new(frame.area());

    screens::help::render(frame, state, layout.content);
    render_transport(frame, state, layout.transport);
    render_shortcuts(frame, state, layout.shortcuts);
}

fn render_settings(frame: &mut Frame, state: &AppState) {
    let layout = FullScreenLayout::new(frame.area());

    screens::settings::render(frame, state, layout.content);
    render_transport(frame, state, layout.transport);
    render_shortcuts(frame, state, layout.shortcuts);
}

/// Render folder browsing view (Miller columns style) with lazy/windowed rendering.
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

            let border_color = if is_focused { t.colors.border_focused } else { t.colors.border };
            let is_root = col_idx == 0;

            // Show title for root column, or any shuffled column
            let title = if is_root && col.is_shuffled() {
                " folders (shuffled) ".to_string()
            } else if is_root {
                " folders ".to_string()
            } else if col.is_shuffled() {
                format!(" {} (shuffled) ", col.title)
            } else {
                String::new()
            };

            let mut block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(t.colors.bg_primary));

            if !title.is_empty() {
                block = block
                    .title(title)
                    .title_style(Style::default().fg(t.colors.fg_accent));
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
                    if is_filter_column && filter_results.is_some() {
                        let results = filter_results.unwrap();
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
                    let display_selected_idx = if filter_active_on_col {
                        filter_results.unwrap().matched_indices.iter()
                            .position(|&idx| idx == selected_idx)
                            .unwrap_or(0)
                    } else {
                        selected_idx
                    };
                    let scroll_offset = match state.browse_scroll_pin {
                        Some((pin_col, pinned)) if pin_col == col_idx => pinned,
                        _ => NavigationService::calc_scroll_offset(display_selected_idx, visible_height, total_items),
                    };

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

                // Position indicator for long lists
                if total_items > visible_height {
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

    if nav.loading {
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

    // Show up to 3 columns at a time
    let max_visible = 3.min(effective_columns);
    let col_width = area.width / max_visible as u16;

    // Determine which columns to show (always include focused column)
    let start_col = if nav.focused_column + 1 > max_visible {
        nav.focused_column + 1 - max_visible
    } else {
        0
    };

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

        let border_color = if is_focused { t.colors.border_focused } else { t.colors.border };

        // Show title for root column, or any shuffled column
        let title = if is_root && col.is_shuffled() {
            format!(" {} (shuffled) ", root_title)
        } else if is_root {
            format!(" {} ", root_title)
        } else if col.is_shuffled() {
            format!(" {} (shuffled) ", col.title)
        } else {
            String::new()
        };

        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(t.colors.bg_primary));

        if !title.is_empty() {
            block = block
                .title(title)
                .title_style(Style::default().fg(t.colors.fg_accent));
        }

        let inner = block.inner(col_area);
        frame.render_widget(block, col_area);

        if col.items.is_empty() {
            let empty = Paragraph::new("(empty)")
                .style(Style::default().fg(t.colors.fg_muted));
            frame.render_widget(empty, inner);
            continue;
        }

        // Check if this column has albums and cover art view is active
        let has_albums = col.items.iter().any(|item| matches!(item, BrowseItem::Album { .. }));
        let is_filter_column = filter_column == Some(col_idx);
        if state.album_art_view && has_albums {
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

            // When filter is active on this column, only show filtered items
            let (items_to_show, total_display_items, filter_active_on_col): (Vec<(usize, &BrowseItem)>, usize, bool) =
                if is_filter_column && filter_results.is_some() {
                    let results = filter_results.unwrap();
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
                let display_selected_idx = if filter_active_on_col {
                    // Find the position of selected_idx in filtered results
                    filter_results.unwrap().matched_indices.iter()
                        .position(|&idx| idx == selected_idx)
                        .unwrap_or(0)
                } else {
                    selected_idx
                };
                let scroll_offset = match state.browse_scroll_pin {
                    Some((pin_col, pinned)) if pin_col == col_idx => pinned,
                    _ => NavigationService::calc_scroll_offset(display_selected_idx, visible_height, total_display_items),
                };

                let visible_items: Vec<ListItem> = items_to_show.into_iter()
                    .skip(scroll_offset)
                    .take(visible_height)
                    .map(|(orig_idx, item)| {
                        let is_selected = orig_idx == selected_idx;

                        // Check if this is the currently playing track
                        let is_now_playing = matches!(item, BrowseItem::Track { key, .. } if current_track_key == Some(key.as_str()));

                        // Prefix based on item type
                        let prefix = match item {
                            BrowseItem::Track { .. } if is_now_playing => "♪ ",
                            BrowseItem::Track { .. } => "  ",
                            _ => "▸ ", // Drillable items get arrow
                        };

                        // Full text (before truncation)
                        let full_text = match item {
                            BrowseItem::Album { title, artist, year, .. } => {
                                let name = if title.is_empty() {
                                    artist.clone()
                                } else if artist.is_empty() {
                                    title.clone()
                                } else {
                                    format!("{} - {}", title, artist)
                                };
                                if let Some(y) = year {
                                    format!("{} ({})", name, y)
                                } else {
                                    name
                                }
                            }
                            BrowseItem::Track { title, track_number, .. } => {
                                if let Some(num) = track_number {
                                    format!("{:02}. {}", num, title)
                                } else {
                                    title.clone()
                                }
                            }
                            _ => item.title().to_string(),
                        };

                        // Marquee for selected+focused item, or truncate normally
                        let display_text = if is_selected && is_focused {
                            let marquee_key = format!("miller:{}:{}", col_idx, orig_idx);
                            let mut marquee = state.marquee.borrow_mut();
                            if marquee.selection_key != marquee_key {
                                marquee.reset(marquee_key, full_text.clone(), max_text_width);
                            }
                            if marquee.phase == crate::app::state::MarqueePhase::Inactive {
                                truncate_middle(&full_text, max_text_width)
                            } else {
                                let text = marquee.display_text();
                                drop(marquee);
                                // Trim to max_text_width (display_text already pads)
                                text.chars().take(max_text_width).collect()
                            }
                        } else {
                            truncate_middle(&full_text, max_text_width)
                        };

                        let style = if is_now_playing {
                            Style::default().fg(t.colors.fg_accent).add_modifier(ratatui::style::Modifier::BOLD)
                        } else if is_selected && is_focused {
                            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                        } else if is_selected {
                            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                        } else {
                            Style::default().fg(t.colors.fg_primary)
                        };

                        ListItem::new(format!("{}{}", prefix, display_text)).style(style)
                    })
                    .collect();

                let list = List::new(visible_items);
                frame.render_widget(list, inner);

                // Position indicator for long lists
                if total_display_items > visible_height {
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

    // Each list row: artwork on left, text on right
    // Size rows to fill available vertical space with at least 3 visible items.
    // Row height is derived from panel height, then art_width from row_height.
    let target_visible = 3u16.max((total_items as u16).min(5));
    let row_height = (inner.height / target_visible).max(3);
    // Art width: 2x row_height (terminal chars are ~2:1 aspect), capped at half column width
    let max_art = inner.width / 2;
    let art_width = (row_height * 2).min(max_art).max(6);

    if row_height == 0 {
        return;
    }

    let visible_rows = (inner.height / row_height).max(1) as usize;
    let selected_idx = col.selected_index;

    // Convert selected_idx to display position within the (possibly filtered) list
    let display_selected = if filter_results.is_some() {
        items_with_indices.iter().position(|(idx, _)| *idx == selected_idx).unwrap_or(0)
    } else {
        selected_idx
    };

    // Standard scroll offset (1 item per row), with pin support for mouse clicks
    let scroll_offset = match state.browse_scroll_pin {
        Some((pin_col, pinned)) if pin_col == col_idx => pinned,
        _ => NavigationService::calc_scroll_offset(display_selected, visible_rows, total_items),
    };

    for vis_row in 0..visible_rows {
        let display_idx = scroll_offset + vis_row;
        if display_idx >= total_items {
            break;
        }

        let row_y = inner.y + (vis_row as u16 * row_height);
        if row_y + row_height > inner.y + inner.height {
            break;
        }

        let (orig_idx, item) = items_with_indices[display_idx];
        let is_selected = orig_idx == selected_idx;

        // Artwork area (left side)
        let image_area = Rect {
            x: inner.x,
            y: row_y,
            width: art_width,
            height: row_height,
        };

        // Text area (right side)
        let text_x = inner.x + art_width + 1;
        let text_width = inner.width.saturating_sub(art_width + 1);

        // Selection highlight background across the full row
        if is_selected {
            let row_area = Rect {
                x: inner.x,
                y: row_y,
                width: inner.width,
                height: row_height,
            };
            let bg_style = if is_focused {
                Style::default().bg(t.colors.selection_bar_bg)
            } else {
                Style::default().bg(t.colors.selection_bar_bg)
            };
            frame.render_widget(Block::default().style(bg_style), row_area);
        }

        // Render album/artist art image or placeholder
        let mut rendered_image = false;
        let art_key = match item {
            BrowseItem::Album { key, .. } => Some(key.as_str()),
            BrowseItem::AllTracks { artist_key, .. } => Some(artist_key.as_str()),
            _ => None,
        };
        if let Some(key) = art_key {
            if let Some(data) = state.album_art_cache.get(key) {
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
            let placeholder_text = if state.album_art_pending.contains(item.key()) {
                "...".to_string()
            } else if initials.is_empty() {
                "?".to_string()
            } else {
                initials
            };

            let text_y = image_area.y + image_area.height / 2;
            let text_x_p = image_area.x + (image_area.width.saturating_sub(placeholder_text.len() as u16)) / 2;
            if text_y < image_area.y + image_area.height {
                frame.render_widget(
                    Paragraph::new(placeholder_text).style(Style::default().fg(t.colors.fg_muted)),
                    Rect { x: text_x_p, y: text_y, width: image_area.width, height: 1 },
                );
            }
        }

        // Text content to the right of artwork
        if text_width > 2 {
            let max_text = text_width.saturating_sub(1) as usize;

            // Title (line 1, vertically centered in row)
            // Use album title, falling back to artist if title is empty
            let display_title = if let BrowseItem::Album { title, artist, .. } = item {
                if title.is_empty() { artist.as_str() } else { title.as_str() }
            } else {
                item.title()
            };
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
            if let BrowseItem::Album { title, artist, year, .. } = item {
                // Show artist on line 2 (or title if title was empty and artist was shown on line 1)
                let sub_name = if title.is_empty() { "" } else { artist.as_str() };
                let subtitle = if let Some(y) = year {
                    if sub_name.is_empty() {
                        format!("({})", y)
                    } else {
                        truncate_middle(&format!("{} ({})", sub_name, y), max_text)
                    }
                } else if sub_name.is_empty() {
                    String::new()
                } else {
                    truncate_middle(sub_name, max_text)
                };
                let sub_style = if is_selected {
                    Style::default().fg(t.colors.fg_muted)
                } else {
                    Style::default().fg(t.colors.fg_muted)
                };
                frame.render_widget(
                    Paragraph::new(subtitle).style(sub_style),
                    Rect { x: text_x, y: title_y + 1, width: text_width, height: 1 },
                );
            }
        }
    }

    // Position indicator
    if total_items > visible_rows {
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
/// Render Station view with Miller columns navigation
fn render_station_view(
    frame: &mut Frame,
    state: &AppState,
    filter_results: Option<&crate::app::state::ListFilterResults>,
    filter_column: Option<usize>,
    left_area: Rect,
    right_area: Rect,
) {
    use crate::util::truncate_middle;
    let t = theme();

    // Combine left and right panels for Miller columns view
    let area = Rect {
        x: left_area.x,
        y: left_area.y,
        width: left_area.width + right_area.width,
        height: left_area.height,
    };

    // Loading state
    if state.station_nav.loading || state.stations_loading {
        let block = Block::default()
            .title(" stations ")
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

    let num_columns = state.station_nav.columns.len();
    if num_columns == 0 {
        // No columns yet - show empty state
        let block = Block::default()
            .title(" stations ")
            .title_style(Style::default().fg(t.colors.fg_accent))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.colors.border_focused))
            .style(Style::default().bg(t.colors.bg_primary));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let msg = Paragraph::new("No stations loaded")
            .style(Style::default().fg(t.colors.fg_muted));
        frame.render_widget(msg, inner);
        return;
    }

    // Don't show empty trailing columns
    let last_meaningful = (0..num_columns)
        .rev()
        .find(|&i| !state.station_nav.columns[i].stations.is_empty() || i <= state.station_nav.focused_column)
        .unwrap_or(0);
    let effective_columns = last_meaningful + 1;

    // Calculate column width - show up to 3 columns
    let max_visible = 3.min(effective_columns);
    let col_width = area.width / max_visible as u16;

    // Determine which columns to show (always include focused column)
    let start_col = if state.station_nav.focused_column + 1 > max_visible {
        state.station_nav.focused_column + 1 - max_visible
    } else {
        0
    };

    for (vis_idx, col_idx) in (start_col..effective_columns.min(start_col + max_visible)).enumerate() {
        let col = &state.station_nav.columns[col_idx];
        let is_focused = col_idx == state.station_nav.focused_column;

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

        let border_color = if is_focused { t.colors.border_focused } else { t.colors.border };

        // Show title for root column, or any shuffled column
        let title = if col_idx == 0 && col.is_shuffled() {
            format!(" {} (shuffled) ", col.title)
        } else if col_idx == 0 {
            format!(" {} ", col.title)
        } else if col.is_shuffled() {
            format!(" {} (shuffled) ", col.title)
        } else {
            String::new()
        };

        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(t.colors.bg_primary));

        if !title.is_empty() {
            block = block
                .title(title)
                .title_style(Style::default().fg(t.colors.fg_accent));
        }

        let inner = block.inner(col_area);
        frame.render_widget(block, col_area);

        if col.stations.is_empty() {
            let empty = Paragraph::new("(empty)")
                .style(Style::default().fg(t.colors.fg_muted));
            frame.render_widget(empty, inner);
        } else {
            // LAZY LOADING: Only render visible items
            let visible_height = inner.height as usize;
            let selected_idx = col.selected_index;

            // Calculate max text width for middle truncation
            let max_text_width = inner.width.saturating_sub(3) as usize; // Leave room for " ›" suffix

            // Check if filter is active on this column
            let is_filter_column = filter_column == Some(col_idx);
            let (items_to_show, total_items, filter_active_on_col): (Vec<(usize, &crate::api::models::Station)>, usize, bool) =
                if is_filter_column && filter_results.is_some() {
                    let results = filter_results.unwrap();
                    if results.matched_indices.is_empty() {
                        (vec![], 0, true)
                    } else {
                        let items: Vec<_> = results.matched_indices.iter()
                            .filter_map(|&idx| col.stations.get(idx).map(|s| (idx, s)))
                            .collect();
                        let len = items.len();
                        (items, len, true)
                    }
                } else {
                    let items: Vec<_> = col.stations.iter().enumerate().collect();
                    let len = items.len();
                    (items, len, false)
                };

            if items_to_show.is_empty() && filter_active_on_col {
                let empty = Paragraph::new("no matches")
                    .style(Style::default().fg(t.colors.fg_muted));
                frame.render_widget(empty, inner);
            } else {
                // Calculate scroll offset based on display items
                let display_selected_idx = if filter_active_on_col {
                    filter_results.unwrap().matched_indices.iter()
                        .position(|&idx| idx == selected_idx)
                        .unwrap_or(0)
                } else {
                    selected_idx
                };
                let scroll_offset = match state.browse_scroll_pin {
                    Some((pin_col, pinned)) if pin_col == col_idx => pinned,
                    _ => NavigationService::calc_scroll_offset(display_selected_idx, visible_height, total_items),
                };

                // Only create ListItems for visible range
                let visible_items: Vec<ListItem> = items_to_show.into_iter()
                    .skip(scroll_offset)
                    .take(visible_height)
                    .map(|(orig_idx, station)| {
                        let is_selected = orig_idx == selected_idx;
                        let is_category = station.is_category();

                        // Show "›" suffix for categories (drillable)
                        let suffix = if is_category { " ›" } else { "" };

                        // Apply middle truncation for long titles
                        let display_title = truncate_middle(&station.title, max_text_width);

                        let style = if is_selected && is_focused {
                            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                        } else if is_selected {
                            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                        } else {
                            Style::default().fg(t.colors.fg_primary)
                        };
                        ListItem::new(format!("{}{}", display_title, suffix)).style(style)
                    })
                    .collect();

                let list = List::new(visible_items);
                frame.render_widget(list, inner);
            }

            // Position indicator for long lists
            if total_items > visible_height {
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
}

/// Render the transport bar (now playing info).
fn render_transport(frame: &mut Frame, state: &AppState, area: Rect) {
    widgets::transport::render(frame, state, area);
}

/// Render the shortcut bar with consistent layout and current view highlighted.
fn render_shortcuts(frame: &mut Frame, state: &AppState, area: Rect) {
    use crate::app::state::AuthStep;
    let t = theme();

    // Show auth-specific shortcuts for Auth view
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

    // Library name indicator (left-aligned, under transport time)
    if let Some(lib_name) = state.active_library.as_ref()
        .and_then(|key| state.libraries.iter().find(|l| &l.key == key))
    {
        let lib_label = Paragraph::new(
            Span::styled(
                format!(" [{}]", lib_name.title),
                Style::default().fg(t.colors.fg_accent_dim).bg(t.colors.bg_secondary),
            )
        ).style(Style::default().bg(t.colors.bg_secondary));
        frame.render_widget(lib_label, area);
    }

    // Three bar modes based on held modifier keys
    let mut spans: Vec<Span> = Vec::new();

    let show_ctrl_alt_bar = state.ctrl_alt_bar_until.is_some();
    let show_alt_bar = state.alt_bar_until.is_some();

    if show_ctrl_alt_bar {
        // Ctrl+Alt bar: global station shortcuts
        let shortcuts: Vec<(&str, &str)> = vec![
            ("^⌥L", "library radio"),
            ("^⌥R", "random album"),
            ("^⌥S", "switch library"),
        ];

        for (i, (key, label)) in shortcuts.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled("|", Style::default().fg(t.colors.fg_muted).bg(t.colors.bg_secondary)));
            }
            spans.push(Span::styled(
                format!(" {} ", key),
                Style::default().fg(t.colors.shortcut_key).bg(t.colors.bg_secondary),
            ));
            spans.push(Span::styled(
                format!("{} ", label),
                Style::default().fg(t.colors.shortcut_text).bg(t.colors.bg_secondary),
            ));
        }
    } else if show_alt_bar {
        // Alt bar: contextual command shortcuts (only available commands shown)
        let alt_cmds = crate::app::available_alt_commands(state);

        if alt_cmds.is_empty() {
            spans.push(Span::styled(
                " no commands available ",
                Style::default().fg(t.colors.fg_muted).bg(t.colors.bg_secondary),
            ));
        } else {
            for (i, cmd) in alt_cmds.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::styled("|", Style::default().fg(t.colors.fg_muted).bg(t.colors.bg_secondary)));
                }
                let key_str = format!(" ⌥{} ", cmd.key.to_ascii_uppercase());
                spans.push(Span::styled(
                    key_str,
                    Style::default().fg(t.colors.shortcut_key).bg(t.colors.bg_secondary),
                ));
                spans.push(Span::styled(
                    format!("{} ", cmd.label),
                    Style::default().fg(t.colors.shortcut_text).bg(t.colors.bg_secondary),
                ));
            }
        }
    } else {
        // Default: navigation shortcuts
        let artists_label = state.artist_view_mode.name();
        let playlists_label = state.playlists_mode.name();
        let genres_label = state.genre_content_type.name();
        let now_playing_label = state.now_playing_mode.name();

        let shortcuts: Vec<(&str, &str, bool)> = vec![
            ("^A", artists_label, state.view == View::Browse && state.browse_category == BrowseCategory::Artists),
            ("^P", playlists_label, state.view == View::Browse && state.browse_category == BrowseCategory::Playlists),
            ("^G", genres_label, state.view == View::Browse && state.browse_category == BrowseCategory::Genres),
            ("^O", "folders", state.view == View::Browse && state.browse_category == BrowseCategory::Folders),
            ("^N", now_playing_label, state.view == View::NowPlaying),
            ("^F", "search", state.search_popup_active),
            ("F1", "help", state.view == View::Help),
            ("F2", "settings", state.view == View::Settings),
        ];

        for (i, (key, label, is_current)) in shortcuts.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled("|", Style::default().fg(t.colors.fg_muted).bg(t.colors.bg_secondary)));
            }

            if *is_current {
                spans.push(Span::styled(
                    format!(" {} {} ", key, label),
                    Style::default().fg(t.colors.fg_accent).bg(t.colors.bg_secondary).add_modifier(ratatui::style::Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(
                    format!(" {} ", key),
                    Style::default().fg(t.colors.shortcut_key).bg(t.colors.bg_secondary),
                ));
                spans.push(Span::styled(
                    format!("{} ", label),
                    Style::default().fg(t.colors.shortcut_text).bg(t.colors.bg_secondary),
                ));
            }
        }
    }

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line)
        .style(Style::default().bg(t.colors.bg_secondary))
        .alignment(Alignment::Center);

    frame.render_widget(paragraph, area);
}

/// Render the search popup as a floating dialog.
fn render_search_popup(frame: &mut Frame, state: &AppState) {
    // Use 80% width and 70% height for the search popup
    let area = centered_rect(80, 70, frame.area());

    // Render the search screen content inside the popup area
    // (filter.rs handles its own Clear and background)
    screens::filter::render(frame, state, area);
}

/// Render the library picker popup (Ctrl+Alt+S).
fn render_library_picker(frame: &mut Frame, state: &AppState) {
    let t = theme();
    let area = centered_rect(40, 30, frame.area());

    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" switch library ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border_focused))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.libraries.is_empty() {
        let msg = Paragraph::new("No libraries available")
            .style(Style::default().fg(t.colors.fg_muted));
        frame.render_widget(msg, inner);
        return;
    }

    // Split inner area: library list + help line
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // library list
            Constraint::Length(1), // help line
        ])
        .split(inner);

    // Build library list items
    let items: Vec<ListItem> = state.libraries.iter().enumerate().map(|(i, lib)| {
        let is_selected = i == state.library_picker_index;
        let is_active = state.active_library.as_deref() == Some(&lib.key);

        let prefix = if is_selected { "> " } else { "  " };
        let suffix = if is_active { " *" } else { "" };
        let text = format!("{}{}{}", prefix, lib.title, suffix);

        let style = if is_selected {
            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
        } else {
            Style::default().fg(t.colors.fg_primary)
        };

        ListItem::new(text).style(style)
    }).collect();

    let list = List::new(items);
    frame.render_widget(list, chunks[0]);

    // Help line
    let help = Paragraph::new("Enter: switch | Esc: close")
        .style(Style::default().fg(t.colors.fg_muted))
        .alignment(Alignment::Center);
    frame.render_widget(help, chunks[1]);
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
    let input_text = format!("{}█", dialog.input);
    let input = Paragraph::new(input_text)
        .style(Style::default().fg(t.colors.fg_primary));
    frame.render_widget(input, chunks[1]);

    // Hint text
    let hint = Paragraph::new("Enter: Save  |  Esc: Cancel")
        .style(Style::default().fg(t.colors.fg_muted))
        .alignment(Alignment::Center);
    frame.render_widget(hint, chunks[2]);
}

fn render_confirm_dialog(frame: &mut Frame, dialog: &ConfirmDialog) {
    let t = theme();
    let area = centered_rect(50, 25, frame.area());

    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(format!(" {} ", dialog.title))
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border_focused))
        .style(Style::default().bg(t.colors.bg_primary));

    let text = format!("{}\n\n[Y] Yes  [N] No", dialog.message);
    let paragraph = Paragraph::new(text)
        .style(Style::default().fg(t.colors.fg_primary))
        .wrap(Wrap { trim: true })
        .block(block);

    frame.render_widget(paragraph, area);
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
