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

use crate::app::state::{View, Focus, BrowseCategory, InputDialog, ConfirmDialog};
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

    // Special Miller columns views for certain categories
    match state.browse_category {
        BrowseCategory::Folders => {
            // Folder browsing mode - Miller columns with folder tree
            render_folder_view(frame, state, layout.left_panel, layout.right_panel);
        }
        BrowseCategory::Genres => {
            // Genres use 3-column Miller columns: Genre | Albums | Tracks
            render_genre_miller_columns(frame, state, layout.left_panel, layout.right_panel);
        }
        BrowseCategory::Stations => {
            // Stations use 2-column view: Stations | Info/Preview
            render_station_view(frame, state, layout.left_panel, layout.right_panel);
        }
        _ => {
            // Standard 2-panel layout for other categories
            render_category_list(frame, state, layout.left_panel);
            render_right_panel(frame, state, layout.right_panel);
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
fn render_folder_view(frame: &mut Frame, state: &AppState, left_area: Rect, right_area: Rect) {
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

            let border_color = if is_focused { t.colors.fg_accent } else { t.colors.border };
            let title = format!(" {} ", col.title);

            let block = Block::default()
                .title(title)
                .title_style(if is_focused {
                    Style::default().fg(t.colors.fg_accent)
                } else {
                    Style::default().fg(t.colors.fg_accent)
                })
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(t.colors.bg_primary));

            let inner = block.inner(col_area);
            frame.render_widget(block, col_area);

            if col.items.is_empty() {
                let empty = Paragraph::new("(empty)")
                    .style(Style::default().fg(t.colors.fg_muted));
                frame.render_widget(empty, inner);
            } else {
                // LAZY LOADING: Only render visible items
                let visible_height = inner.height as usize;
                let total_items = col.items.len();
                let selected_idx = col.selected_index;

                // Calculate scroll offset to keep selection visible
                let scroll_offset = NavigationService::calc_scroll_offset(selected_idx, visible_height, total_items);

                // Only create ListItems for visible range
                let visible_items: Vec<ListItem> = col.items.iter()
                    .enumerate()
                    .skip(scroll_offset)
                    .take(visible_height)
                    .map(|(i, item)| {
                        let is_selected = i == selected_idx;

                        // Check if this item is the currently playing track
                        let is_now_playing = matches!(item.item_type, FolderItemType::Track)
                            && current_track_key.map(|k| item.key == k).unwrap_or(false);

                        let prefix = match item.item_type {
                            FolderItemType::Folder => "▸ ",
                            FolderItemType::Track if is_now_playing => "♪ ",
                            FolderItemType::Track => "  ",
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
                        ListItem::new(format!("{}{}", prefix, item.title)).style(style)
                    })
                    .collect();

                let list = List::new(visible_items);
                frame.render_widget(list, inner);

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

/// Render Genre view with 3-column Miller columns: Genres | Albums | Tracks
fn render_genre_miller_columns(frame: &mut Frame, state: &AppState, left_area: Rect, right_area: Rect) {
    let t = theme();

    // Combine left and right panels for 3-column layout
    let area = Rect {
        x: left_area.x,
        y: left_area.y,
        width: left_area.width + right_area.width,
        height: left_area.height,
    };

    // Split into 3 columns
    let col_width = area.width / 3;
    let columns = [
        Rect::new(area.x, area.y, col_width, area.height),
        Rect::new(area.x + col_width, area.y, col_width, area.height),
        Rect::new(area.x + col_width * 2, area.y, area.width - col_width * 2, area.height),
    ];

    // Column 0: Genres list
    let genres_title = format!(" {} ", state.genre_content_type.name());
    let genres_focused = state.genre_focus_column == 0;
    let genres_block = Block::default()
        .title(genres_title)
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(if genres_focused {
            Style::default().fg(t.colors.border_focused)
        } else {
            Style::default().fg(t.colors.border)
        })
        .style(Style::default().bg(t.colors.bg_primary));

    let genres_inner = genres_block.inner(columns[0]);
    frame.render_widget(genres_block, columns[0]);

    let genre_list = state.current_genre_list();
    if genre_list.is_empty() {
        let msg = if state.genres_loading || state.artist_genres_loading || state.album_genres_loading || state.moods_loading || state.styles_loading {
            "Loading..."
        } else {
            "No items"
        };
        frame.render_widget(
            Paragraph::new(msg).style(Style::default().fg(t.colors.fg_muted)),
            genres_inner,
        );
    } else {
        let visible_height = genres_inner.height as usize;
        let total = genre_list.len();
        let selected = state.genres_index;
        let scroll_offset = NavigationService::calc_scroll_offset(selected, visible_height, total);

        let items: Vec<ListItem> = genre_list.iter()
            .enumerate()
            .skip(scroll_offset)
            .take(visible_height)
            .map(|(i, genre)| {
                let is_selected = i == selected;
                let style = if is_selected && genres_focused {
                    Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                } else if is_selected {
                    Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                } else {
                    Style::default().fg(t.colors.fg_primary)
                };
                ListItem::new(genre.title.as_str()).style(style)
            })
            .collect();

        frame.render_widget(List::new(items), genres_inner);

        // Position indicator
        if total > visible_height {
            let footer = format!("{}/{}", selected + 1, total);
            let footer_area = Rect::new(
                columns[0].x + columns[0].width.saturating_sub(footer.len() as u16 + 2),
                columns[0].y + columns[0].height - 1,
                footer.len() as u16 + 1,
                1,
            );
            frame.render_widget(
                Paragraph::new(footer).style(Style::default().fg(t.colors.fg_muted)),
                footer_area,
            );
        }
    }

    // Column 1: Albums in selected genre
    let albums_title = format!(" albums (by {}) ", state.genre_sort_mode.name());
    let albums_focused = state.genre_focus_column == 1;
    let albums_block = Block::default()
        .title(albums_title)
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(if albums_focused {
            Style::default().fg(t.colors.border_focused)
        } else {
            Style::default().fg(t.colors.border)
        })
        .style(Style::default().bg(t.colors.bg_primary));

    let albums_inner = albums_block.inner(columns[1]);
    frame.render_widget(albums_block, columns[1]);

    if state.genre_albums.is_empty() {
        let msg = if state.right_panel_loading {
            "Loading..."
        } else {
            "Select genre"
        };
        frame.render_widget(
            Paragraph::new(msg).style(Style::default().fg(t.colors.fg_muted)),
            albums_inner,
        );
    } else {
        let visible_height = albums_inner.height as usize;
        let total = state.genre_albums.len();
        let selected = state.genre_albums_index;
        let scroll_offset = NavigationService::calc_scroll_offset(selected, visible_height, total);

        let items: Vec<ListItem> = state.genre_albums.iter()
            .enumerate()
            .skip(scroll_offset)
            .take(visible_height)
            .map(|(i, album)| {
                let is_selected = i == selected;
                let style = if is_selected && albums_focused {
                    Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                } else if is_selected {
                    Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                } else {
                    Style::default().fg(t.colors.fg_primary)
                };
                let year = album.year.map(|y| format!(" ({})", y)).unwrap_or_default();
                let artist = album.parent_title.as_deref().unwrap_or("Unknown");
                ListItem::new(format!("{} - {}{}", album.title, artist, year)).style(style)
            })
            .collect();

        frame.render_widget(List::new(items), albums_inner);

        // Position indicator
        if total > visible_height {
            let footer = format!("{}/{}", selected + 1, total);
            let footer_area = Rect::new(
                columns[1].x + columns[1].width.saturating_sub(footer.len() as u16 + 2),
                columns[1].y + columns[1].height - 1,
                footer.len() as u16 + 1,
                1,
            );
            frame.render_widget(
                Paragraph::new(footer).style(Style::default().fg(t.colors.fg_muted)),
                footer_area,
            );
        }
    }

    // Column 2: Tracks in selected album
    let tracks_focused = state.genre_focus_column == 2;
    let tracks_block = Block::default()
        .title(" tracks ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(if tracks_focused {
            Style::default().fg(t.colors.border_focused)
        } else {
            Style::default().fg(t.colors.border)
        })
        .style(Style::default().bg(t.colors.bg_primary));

    let tracks_inner = tracks_block.inner(columns[2]);
    frame.render_widget(tracks_block, columns[2]);

    if state.genre_tracks.is_empty() {
        let msg = "Select album";
        frame.render_widget(
            Paragraph::new(msg).style(Style::default().fg(t.colors.fg_muted)),
            tracks_inner,
        );
    } else {
        let current_track_key = state.current_track().map(|t| t.rating_key.as_str());
        widgets::track_list::render(
            frame,
            &state.genre_tracks,
            state.genre_tracks_index,
            current_track_key,
            tracks_inner,
        );
    }
}

/// Render Station view with Miller columns navigation
fn render_station_view(frame: &mut Frame, state: &AppState, left_area: Rect, right_area: Rect) {
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

        let border_color = if is_focused { t.colors.fg_accent } else { t.colors.border };
        let title = format!(" {} ", col.title);

        let block = Block::default()
            .title(title)
            .title_style(if is_focused {
                Style::default().fg(t.colors.fg_accent)
            } else {
                Style::default().fg(t.colors.fg_accent)
            })
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(t.colors.bg_primary));

        let inner = block.inner(col_area);
        frame.render_widget(block, col_area);

        if col.stations.is_empty() {
            let empty = Paragraph::new("(empty)")
                .style(Style::default().fg(t.colors.fg_muted));
            frame.render_widget(empty, inner);
        } else {
            // LAZY LOADING: Only render visible items
            let visible_height = inner.height as usize;
            let total_items = col.stations.len();
            let selected_idx = col.selected_index;

            // Calculate scroll offset to keep selection visible
            let scroll_offset = NavigationService::calc_scroll_offset(selected_idx, visible_height, total_items);

            // Only create ListItems for visible range
            let visible_items: Vec<ListItem> = col.stations.iter()
                .enumerate()
                .skip(scroll_offset)
                .take(visible_height)
                .map(|(i, station)| {
                    let is_selected = i == selected_idx;
                    let is_category = station.is_category();

                    // Show "›" suffix for categories (drillable)
                    let suffix = if is_category { " ›" } else { "" };

                    let style = if is_selected && is_focused {
                        Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                    } else if is_selected {
                        Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                    } else {
                        Style::default().fg(t.colors.fg_primary)
                    };
                    ListItem::new(format!("{}{}", station.title, suffix)).style(style)
                })
                .collect();

            let list = List::new(visible_items);
            frame.render_widget(list, inner);

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

/// Render the category list (left panel).
fn render_category_list(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();
    let is_focused = state.focus == Focus::Left;
    let category = state.browse_category;

    // Title shows category type, with mode indicator for genres, artists, and playlists
    let title = match category {
        BrowseCategory::Genres => {
            format!(" {} ", state.genre_content_type.name())
        }
        BrowseCategory::Artists => {
            format!(" {} ", state.artist_view_mode.name())
        }
        BrowseCategory::Playlists => {
            format!(" {} ", state.playlists_mode.name())
        }
        _ => format!(" {} ", category.name()),
    };

    let border_style = if is_focused {
        Style::default().fg(t.colors.border_focused)
    } else {
        Style::default().fg(t.colors.border)
    };

    let block = Block::default()
        .title(title)
        .title_style(if is_focused {
            Style::default().fg(t.colors.fg_accent)
        } else {
            Style::default().fg(t.colors.fg_accent)
        })
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Get items for current category
    let (items, selected) = match category {
        BrowseCategory::Artists => {
            use crate::app::state::ArtistViewMode;
            // Show artists or albums based on view mode
            match state.artist_view_mode {
                ArtistViewMode::Artist | ArtistViewMode::AlbumArtist => {
                    let items: Vec<ListItem> = state.artists.iter().enumerate().map(|(i, artist)| {
                        let is_selected = i == state.list_state.artists_index;
                        let style = if is_selected && is_focused {
                            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                        } else if is_selected {
                            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                        } else {
                            Style::default().fg(t.colors.fg_primary)
                        };
                        ListItem::new(artist.title.as_str()).style(style)
                    }).collect();
                    (items, state.list_state.artists_index)
                }
                ArtistViewMode::Album => {
                    // Show albums by title
                    let items: Vec<ListItem> = state.albums.iter().enumerate().map(|(i, album)| {
                        let is_selected = i == state.list_state.albums_index;
                        let style = if is_selected && is_focused {
                            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                        } else if is_selected {
                            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                        } else {
                            Style::default().fg(t.colors.fg_primary)
                        };
                        // Show album title with artist
                        let artist = album.parent_title.as_deref().unwrap_or("Unknown");
                        ListItem::new(format!("{} - {}", album.title, artist)).style(style)
                    }).collect();
                    (items, state.list_state.albums_index)
                }
            }
        }
        BrowseCategory::Playlists => {
            use crate::app::state::PlaylistsMode;
            match state.playlists_mode {
                PlaylistsMode::All => {
                    let items: Vec<ListItem> = state.playlists.iter().enumerate().map(|(i, playlist)| {
                        let is_selected = i == state.list_state.playlists_index;
                        let style = if is_selected && is_focused {
                            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                        } else if is_selected {
                            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                        } else {
                            Style::default().fg(t.colors.fg_primary)
                        };
                        ListItem::new(playlist.title.as_str()).style(style)
                    }).collect();
                    (items, state.list_state.playlists_index)
                }
                PlaylistsMode::RecentlyAdded => {
                    let items: Vec<ListItem> = state.recently_added_albums.iter().enumerate().map(|(i, album)| {
                        let is_selected = i == state.list_state.playlists_index;
                        let style = if is_selected && is_focused {
                            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                        } else if is_selected {
                            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                        } else {
                            Style::default().fg(t.colors.fg_primary)
                        };
                        let artist = album.artist_name();
                        ListItem::new(format!("{} - {}", album.title, artist)).style(style)
                    }).collect();
                    (items, state.list_state.playlists_index)
                }
                PlaylistsMode::RecentPlaylists => {
                    let items: Vec<ListItem> = state.recent_playlists.iter().enumerate().map(|(i, playlist)| {
                        let is_selected = i == state.list_state.playlists_index;
                        let style = if is_selected && is_focused {
                            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                        } else if is_selected {
                            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                        } else {
                            Style::default().fg(t.colors.fg_primary)
                        };
                        ListItem::new(playlist.title.as_str()).style(style)
                    }).collect();
                    (items, state.list_state.playlists_index)
                }
            }
        }
        BrowseCategory::Stations => {
            let items: Vec<ListItem> = state.stations.iter().enumerate().map(|(i, station)| {
                let is_selected = i == state.stations_index;
                let is_category = station.is_category();
                let suffix = if is_category { " ›" } else { "" };
                let style = if is_selected && is_focused {
                    Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                } else if is_selected {
                    Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                } else {
                    Style::default().fg(t.colors.fg_primary)
                };
                ListItem::new(format!("{}{}", station.title, suffix)).style(style)
            }).collect();
            (items, state.stations_index)
        }
        BrowseCategory::Genres => {
            // Show genres, normalized genres, or moods based on current content type
            let items: Vec<ListItem> = state.current_genre_list().iter().enumerate().map(|(i, genre)| {
                let is_selected = i == state.genres_index;
                let style = if is_selected && is_focused {
                    Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                } else if is_selected {
                    Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                } else {
                    Style::default().fg(t.colors.fg_primary)
                };
                ListItem::new(genre.title.as_str()).style(style)
            }).collect();
            (items, state.genres_index)
        }
        BrowseCategory::Folders => {
            // Folders category uses the folder view, not category list
            // Return empty - folder view is rendered separately
            (vec![], 0)
        }
    };

    if items.is_empty() {
        let msg = if state.artists_loading || state.albums_loading || state.playlists_loading || state.genres_loading {
            "Loading..."
        } else if category == BrowseCategory::Folders {
            "" // Folders handled separately
        } else {
            "No items"
        };
        let empty = Paragraph::new(msg)
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
    } else {
        // Calculate visible window
        let visible_height = inner.height as usize;
        let total = items.len();
        let scroll_offset = NavigationService::calc_scroll_offset(selected, visible_height, total);

        let visible_items: Vec<ListItem> = items.into_iter()
            .skip(scroll_offset)
            .take(visible_height)
            .collect();

        let list = List::new(visible_items);
        frame.render_widget(list, inner);

        // Position indicator
        if total > visible_height {
            let footer = format!("{}/{}", selected + 1, total);
            let footer_area = Rect::new(
                area.x + area.width.saturating_sub(footer.len() as u16 + 2),
                area.y + area.height - 1,
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

/// Render the right panel (albums or tracks depending on mode).
fn render_right_panel(frame: &mut Frame, state: &AppState, area: Rect) {
    use crate::app::state::RightPanelMode;

    let t = theme();
    let is_focused = state.focus == Focus::Right;

    // Determine title based on mode
    let title = match state.right_panel_mode {
        RightPanelMode::Empty => " select item ".to_string(),
        RightPanelMode::ArtistAlbums => format!(" {} albums ", state.selected_artist_name),
        RightPanelMode::AlbumTracks => format!(" {} ", state.selected_album_title),
        RightPanelMode::CategoryTracks => " tracks ".to_string(),
        RightPanelMode::CategoryAlbums => {
            // Show sort mode in title for genre albums
            format!(" albums (by {}) ", state.genre_sort_mode.name())
        }
    };

    let border_style = if is_focused {
        Style::default().fg(t.colors.border_focused)
    } else {
        Style::default().fg(t.colors.border)
    };

    let block = Block::default()
        .title(title)
        .title_style(if is_focused {
            Style::default().fg(t.colors.fg_accent)
        } else {
            Style::default().fg(t.colors.fg_accent)
        })
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.right_panel_loading {
        let loading = Paragraph::new("Loading...")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(loading, inner);
        return;
    }

    match state.right_panel_mode {
        RightPanelMode::Empty => {
            let msg = format!("Select {} to view content", state.browse_category.name());
            let empty = Paragraph::new(msg)
                .style(Style::default().fg(t.colors.fg_muted))
                .alignment(Alignment::Center);
            frame.render_widget(empty, inner);
        }
        RightPanelMode::ArtistAlbums => {
            render_album_list(frame, state, inner, is_focused);
        }
        RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
            let tracks = if state.right_panel_mode == RightPanelMode::AlbumTracks {
                &state.selected_album_tracks
            } else {
                &state.selected_album_tracks // For category tracks too
            };

            if tracks.is_empty() {
                let empty = Paragraph::new("No tracks")
                    .style(Style::default().fg(t.colors.fg_muted))
                    .alignment(Alignment::Center);
                frame.render_widget(empty, inner);
            } else {
                let current_track_key = state.current_track().map(|t| t.rating_key.as_str());
                widgets::track_list::render(
                    frame,
                    tracks,
                    state.list_state.tracks_index,
                    current_track_key,
                    inner,
                );
            }
        }
        RightPanelMode::CategoryAlbums => {
            render_genre_album_list(frame, state, inner, is_focused);
        }
    }
}

/// Render album list in the right panel.
fn render_album_list(frame: &mut Frame, state: &AppState, area: Rect, is_focused: bool) {
    let t = theme();

    // Total includes "All Tracks" entry at index 0
    let total = state.selected_artist_albums.len() + 1;
    let selected_idx = state.list_state.right_albums_index;
    let visible_height = area.height as usize;

    let scroll_offset = NavigationService::calc_scroll_offset(selected_idx, visible_height, total);

    let mut items: Vec<ListItem> = Vec::with_capacity(visible_height);

    // Build items for visible range
    for i in scroll_offset..(scroll_offset + visible_height).min(total) {
        let is_selected = i == selected_idx;

        let line = if i == 0 {
            // "All Tracks" entry
            "► All Tracks".to_string()
        } else {
            // Album entry (index - 1 to get actual album)
            let album = &state.selected_artist_albums[i - 1];
            let year = album.year.map(|y| format!(" ({})", y)).unwrap_or_default();
            let track_count = album.leaf_count.map(|c| format!(" [{} tracks]", c)).unwrap_or_default();
            format!("{}{}{}", album.title, year, track_count)
        };

        let style = if is_selected && is_focused {
            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
        } else if is_selected {
            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
        } else {
            Style::default().fg(t.colors.fg_primary)
        };

        items.push(ListItem::new(line).style(style));
    }

    let list = List::new(items);
    frame.render_widget(list, area);

    // Position indicator
    if total > visible_height {
        let footer = format!("{}/{}", selected_idx + 1, total);
        let footer_area = Rect::new(
            area.x + area.width.saturating_sub(footer.len() as u16 + 2),
            area.y + area.height.saturating_sub(1),
            footer.len() as u16 + 1,
            1,
        );
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().fg(t.colors.fg_muted)),
            footer_area,
        );
    }
}

/// Render album list for genre/mood view.
fn render_genre_album_list(frame: &mut Frame, state: &AppState, area: Rect, is_focused: bool) {
    let t = theme();

    let total = state.genre_albums.len();
    let selected_idx = state.genre_albums_index;
    let visible_height = area.height as usize;

    if total == 0 {
        let empty = Paragraph::new("No albums")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, area);
        return;
    }

    let scroll_offset = NavigationService::calc_scroll_offset(selected_idx, visible_height, total);

    let mut items: Vec<ListItem> = Vec::with_capacity(visible_height);

    for i in scroll_offset..(scroll_offset + visible_height).min(total) {
        let is_selected = i == selected_idx;
        let album = &state.genre_albums[i];

        // Format: Album Title - Artist (Year)
        let year = album.year.map(|y| format!(" ({})", y)).unwrap_or_default();
        let artist = album.parent_title.as_deref().unwrap_or("Unknown");
        let line = format!("{} - {}{}", album.title, artist, year);

        let style = if is_selected && is_focused {
            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
        } else if is_selected {
            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
        } else {
            Style::default().fg(t.colors.fg_primary)
        };

        items.push(ListItem::new(line).style(style));
    }

    let list = List::new(items);
    frame.render_widget(list, area);

    // Position indicator
    if total > visible_height {
        let footer = format!("{}/{}", selected_idx + 1, total);
        let footer_area = Rect::new(
            area.x + area.width.saturating_sub(footer.len() as u16 + 2),
            area.y + area.height.saturating_sub(1),
            footer.len() as u16 + 1,
            1,
        );
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().fg(t.colors.fg_muted)),
            footer_area,
        );
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

    // Define all shortcuts - consistent across all views
    // Format: (key, label, is_current_view_or_category)
    // Labels change based on current mode within each category
    let artists_label = state.artist_view_mode.name();
    let playlists_label = state.playlists_mode.name();
    let genres_label = state.genre_content_type.name();
    let now_playing_label = state.now_playing_mode.name();

    let shortcuts: Vec<(&str, &str, bool)> = vec![
        ("^A", artists_label, state.view == View::Browse && state.browse_category == BrowseCategory::Artists),
        ("^P", playlists_label, state.view == View::Browse && state.browse_category == BrowseCategory::Playlists),
        ("^G", genres_label, state.view == View::Browse && state.browse_category == BrowseCategory::Genres),
        ("^O", "folders", state.view == View::Browse && state.browse_category == BrowseCategory::Folders),
        ("^T", "stations", state.view == View::Browse && state.browse_category == BrowseCategory::Stations),
        ("^N", now_playing_label, state.view == View::NowPlaying),
        ("^F", "search", state.view == View::Search),
        ("F1", "help", state.view == View::Help),
        ("F2", "settings", state.view == View::Settings),
    ];

    // Build spans with highlighting for current view
    let mut spans: Vec<Span> = Vec::new();

    for (i, (key, label, is_current)) in shortcuts.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("|", Style::default().fg(t.colors.fg_muted).bg(t.colors.bg_secondary)));
        }

        if *is_current {
            // Highlighted: use accent color for both key and label
            spans.push(Span::styled(
                format!(" {} {} ", key, label),
                Style::default().fg(t.colors.fg_accent).bg(t.colors.bg_secondary).add_modifier(ratatui::style::Modifier::BOLD),
            ));
        } else {
            // Normal: key in shortcut_key color, label in shortcut_text color
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

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line)
        .style(Style::default().bg(t.colors.bg_secondary))
        .alignment(Alignment::Center);

    frame.render_widget(paragraph, area);
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
    let width = (message.len() + 4).min(40) as u16;
    let toast_area = Rect {
        x: area.width.saturating_sub(width + 2),
        y: area.height.saturating_sub(4),
        width,
        height: 3,
    };

    frame.render_widget(Clear, toast_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.fg_accent))
        .style(Style::default().bg(t.colors.bg_secondary));
    let text = Paragraph::new(message)
        .style(Style::default().fg(t.colors.fg_primary))
        .block(block);
    frame.render_widget(text, toast_area);
}
