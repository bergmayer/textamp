//! Adventure launcher popup (Sonic Adventure from Radio section).
//!
//! Three-step self-contained popup:
//! 1. Find start track (search + drill into artist → albums → tracks)
//! 2. Enter track count (5-100)
//! 3. Find end track (same search + drill)

use crate::app::state::{AdventureDrillLevel, AdventureLauncherState, AdventureStep, SearchFocus, SearchTab};
use crate::app::AppState;
use crate::services::NavigationService;
use crate::ui::layout::centered_rect;
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs};

/// Render the adventure launcher popup as an overlay.
pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let launcher = match &state.popups.adventure_launcher {
        Some(l) => l,
        None => return,
    };

    // Register hit regions for mouse handler
    {
        let popup_area = centered_rect(50, 70, area);
        let block_tmp = ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::ALL);
        let inner_tmp = block_tmp.inner(popup_area);
        // Results Y offset depends on drill level
        let results_y_offset: u16 = match launcher.step {
            AdventureStep::EnterTrackCount => 0, // count input popup — no results list
            _ => match &launcher.drill {
                AdventureDrillLevel::Search => 2 + 3, // tabs (2) + search input (3)
                AdventureDrillLevel::ArtistAlbums { .. } => 1, // breadcrumb (1)
                AdventureDrillLevel::AlbumTracks { .. } => 1,  // breadcrumb (1)
            },
        };
        let item_count = match launcher.step {
            AdventureStep::EnterTrackCount => 0,
            _ => match &launcher.drill {
                AdventureDrillLevel::Search => {
                    launcher.results.as_ref().map_or(0, |r| {
                        r.artists.len() + r.albums.len() + r.tracks.len()
                    })
                }
                AdventureDrillLevel::ArtistAlbums { albums, .. } => albums.len(),
                AdventureDrillLevel::AlbumTracks { tracks, .. } => tracks.len(),
            },
        };
        let mut hr = state.hit_regions.borrow_mut();
        hr.adventure_launcher = Some(crate::ui::hit_regions::AdventureLauncherRegions {
            outer: popup_area,
            inner: inner_tmp,
            item_count,
            results_y_offset,
        });
    }

    match launcher.step {
        AdventureStep::FindStartTrack | AdventureStep::FindEndTrack => {
            render_track_finder(frame, launcher, area);
        }
        AdventureStep::EnterTrackCount => {
            render_track_count(frame, launcher, area);
        }
    }
}

/// Render step 1/3: track finder with search and drill-down.
fn render_track_finder(frame: &mut Frame, launcher: &AdventureLauncherState, area: Rect) {
    let t = theme();

    let popup_area = centered_rect(50, 70, area);
    frame.render_widget(Clear, popup_area);

    let title = match launcher.step {
        AdventureStep::FindStartTrack => " sonic adventure — select start track ",
        AdventureStep::FindEndTrack => " sonic adventure — select end track ",
        _ => " sonic adventure ",
    };

    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.fg_accent))
        .style(Style::default().bg(t.colors.bg_primary));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    match &launcher.drill {
        AdventureDrillLevel::Search => {
            render_search_level(frame, launcher, inner);
        }
        AdventureDrillLevel::ArtistAlbums { artist_name, albums, .. } => {
            render_artist_albums_level(frame, launcher, artist_name, albums, inner);
        }
        AdventureDrillLevel::AlbumTracks { album_title, artist_name, tracks, .. } => {
            render_album_tracks_level(frame, launcher, artist_name, album_title, tracks, inner);
        }
    }
}

/// Render the search level: tabs + search input + results.
fn render_search_level(frame: &mut Frame, launcher: &AdventureLauncherState, area: Rect) {
    let t = theme();

    let chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(2), // tabs + step hint
            ratatui::layout::Constraint::Length(3), // search input
            ratatui::layout::Constraint::Min(3),    // results
        ])
        .split(area);

    // Tab bar + step hint
    let tab_hint_chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(1), // tabs
            ratatui::layout::Constraint::Length(1), // step hint
        ])
        .split(chunks[0]);

    render_adventure_tabs(frame, launcher, tab_hint_chunks[0]);

    let step_hint = match launcher.step {
        AdventureStep::FindStartTrack => "Step 1/3: Search for a track to START the adventure.",
        AdventureStep::FindEndTrack => "Step 3/3: Search for a track to END the adventure.",
        _ => "",
    };
    let hint = Paragraph::new(Span::styled(step_hint, Style::default().fg(t.colors.fg_muted)));
    frame.render_widget(hint, tab_hint_chunks[1]);

    // Search input
    render_search_input(frame, &launcher.query, launcher.focus == SearchFocus::Input, chunks[1]);

    // Results
    render_search_results(frame, launcher, chunks[2]);
}

/// Render the tab bar for adventure launcher search.
fn render_adventure_tabs(frame: &mut Frame, launcher: &AdventureLauncherState, area: Rect) {
    let t = theme();

    let labels = SearchTab::all();
    let selected_idx = match launcher.search_tab {
        SearchTab::Global => 0,
        SearchTab::Artists => 1,
        SearchTab::Albums => 2,
        SearchTab::Playlists => 3,
        SearchTab::Tracks => 4,
        SearchTab::Genres => 5,
    };

    let titles: Vec<Line> = labels.iter().enumerate().map(|(i, tab)| {
        if i == selected_idx {
            Line::from(Span::styled(
                format!(" {} ", tab.name()),
                Style::default()
                    .fg(t.colors.fg_accent)
                    .add_modifier(Modifier::BOLD),
            ))
        } else {
            Line::from(Span::styled(
                format!(" {} ", tab.name()),
                Style::default().fg(t.colors.fg_muted),
            ))
        }
    }).collect();

    let tabs = Tabs::new(titles)
        .select(selected_idx)
        .highlight_style(Style::default())
        .style(Style::default().bg(t.colors.bg_primary).fg(t.colors.fg_muted))
        .divider(Span::styled(" │ ", Style::default().fg(t.colors.fg_muted)))
        .padding("", "");

    frame.render_widget(tabs, area);
}

/// Render the artist albums drill level.
fn render_artist_albums_level(
    frame: &mut Frame,
    launcher: &AdventureLauncherState,
    artist_name: &str,
    albums: &[crate::plex::models::Album],
    area: Rect,
) {
    let t = theme();

    let chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(1), // breadcrumb
            ratatui::layout::Constraint::Min(3),    // album list
        ])
        .split(area);

    // Breadcrumb
    let breadcrumb = Line::from(vec![
        Span::styled("← ", Style::default().fg(t.colors.fg_accent)),
        Span::styled(artist_name, Style::default().fg(t.colors.fg_accent).bold()),
    ]);
    frame.render_widget(Paragraph::new(breadcrumb), chunks[0]);

    // Album list
    if albums.is_empty() {
        let msg = if launcher.loading { "Loading..." } else { "No albums found" };
        let empty = Paragraph::new(msg)
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, chunks[1]);
        return;
    }

    let is_focused = launcher.focus == SearchFocus::Results;
    let visible_height = chunks[1].height as usize;
    let scroll_offset = match launcher.scroll_pin {
        Some(pinned) => pinned,
        None => NavigationService::calc_scroll_offset(launcher.item_index, visible_height, albums.len()),
    };

    let items: Vec<ListItem> = albums.iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(i, album)| {
            let is_selected = is_focused && i == launcher.item_index;
            let text = if let Some(year) = album.year {
                format!("  {} ({})", album.title, year)
            } else {
                format!("  {}", album.title)
            };
            let style = if is_selected {
                Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
            } else {
                Style::default().fg(t.colors.fg_primary)
            };
            ListItem::new(text).style(style)
        })
        .collect();

    frame.render_widget(List::new(items), chunks[1]);

    // Scrollbar for long album lists
    if albums.len() > visible_height {
        crate::ui::widgets::render_scrollbar_borderless(frame, chunks[1], albums.len(), visible_height, scroll_offset);
    }
}

/// Render the album tracks drill level.
fn render_album_tracks_level(
    frame: &mut Frame,
    launcher: &AdventureLauncherState,
    artist_name: &str,
    album_title: &str,
    tracks: &[crate::plex::models::Track],
    area: Rect,
) {
    let t = theme();

    let chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(2), // breadcrumb (2 lines)
            ratatui::layout::Constraint::Min(3),    // track list
        ])
        .split(area);

    // Breadcrumb
    let breadcrumb = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("← ", Style::default().fg(t.colors.fg_accent)),
            Span::styled(artist_name, Style::default().fg(t.colors.fg_accent).bold()),
            Span::styled(" > ", Style::default().fg(t.colors.fg_muted)),
            Span::styled(album_title, Style::default().fg(t.colors.fg_accent)),
        ]),
        Line::from(Span::styled(
            "Select a track for the adventure.",
            Style::default().fg(t.colors.fg_muted).italic(),
        )),
    ]);
    frame.render_widget(breadcrumb, chunks[0]);

    // Track list
    if tracks.is_empty() {
        let msg = if launcher.loading { "Loading..." } else { "No tracks found" };
        let empty = Paragraph::new(msg)
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, chunks[1]);
        return;
    }

    let is_focused = launcher.focus == SearchFocus::Results;
    let visible_height = chunks[1].height as usize;
    let scroll_offset = match launcher.scroll_pin {
        Some(pinned) => pinned,
        None => NavigationService::calc_scroll_offset(launcher.item_index, visible_height, tracks.len()),
    };

    let items: Vec<ListItem> = tracks.iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(i, track)| {
            let is_selected = is_focused && i == launcher.item_index;
            let track_num = track.index.map(|n| format!("{}. ", n)).unwrap_or_default();
            let duration = format_duration(track.duration_ms());
            let text = format!("  {}{} [{}]", track_num, track.title, duration);
            let style = if is_selected {
                Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
            } else {
                Style::default().fg(t.colors.fg_primary)
            };
            ListItem::new(text).style(style)
        })
        .collect();

    frame.render_widget(List::new(items), chunks[1]);

    // Scrollbar for long track lists
    if tracks.len() > visible_height {
        crate::ui::widgets::render_scrollbar_borderless(frame, chunks[1], tracks.len(), visible_height, scroll_offset);
    }
}

/// Render search input box.
fn render_search_input(frame: &mut Frame, query: &str, is_focused: bool, area: Rect) {
    let t = theme();

    let input_block = Block::default()
        .title(" search ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if is_focused {
            t.colors.border_focused
        } else {
            t.colors.border
        }))
        .style(Style::default().bg(t.colors.bg_primary));

    let input_inner = input_block.inner(area);
    frame.render_widget(input_block, area);

    let query_text = if is_focused {
        format!("{}▋", query)
    } else {
        query.to_string()
    };
    let fg = if is_focused { t.colors.fg_primary } else { t.colors.fg_muted };
    let input = Paragraph::new(query_text).style(Style::default().fg(fg));
    frame.render_widget(input, input_inner);
}

/// Render search results (tab-aware: Global shows all with headers, other tabs show per-category).
fn render_search_results(frame: &mut Frame, launcher: &AdventureLauncherState, area: Rect) {
    let t = theme();

    let results = match &launcher.results {
        Some(r) => r,
        None => {
            let msg = if launcher.query.is_empty() {
                "Type to search your library"
            } else if launcher.loading {
                "Searching..."
            } else {
                "No results"
            };
            let empty = Paragraph::new(msg)
                .style(Style::default().fg(t.colors.fg_muted))
                .alignment(Alignment::Center);
            frame.render_widget(empty, area);
            return;
        }
    };

    let is_focused = launcher.focus == SearchFocus::Results;
    let selected_idx = launcher.item_index;
    let visible_height = area.height as usize;

    match launcher.search_tab {
        SearchTab::Global => {
            // Global tab: show all results with section headers
            let has_any = !results.artists.is_empty() || !results.albums.is_empty() || !results.tracks.is_empty();
            if !has_any {
                let msg = if launcher.loading { "Searching..." } else { "No matches found" };
                let empty = Paragraph::new(msg)
                    .style(Style::default().fg(t.colors.fg_muted))
                    .alignment(Alignment::Center);
                frame.render_widget(empty, area);
                return;
            }

            let mut entries: Vec<(String, bool, Option<usize>)> = Vec::new();
            let mut global_idx: usize = 0;

            if !results.artists.is_empty() {
                entries.push((format!("── Artists ({}) ──", results.artists.len()), true, None));
                for a in &results.artists {
                    entries.push((format!("  {}", a.title), false, Some(global_idx)));
                    global_idx += 1;
                }
            }
            if !results.albums.is_empty() {
                entries.push((format!("── Albums ({}) ──", results.albums.len()), true, None));
                for a in &results.albums {
                    let artist = a.artist_name();
                    let text = if let Some(year) = a.year {
                        format!("  {} ({}) - {}", a.title, year, artist)
                    } else {
                        format!("  {} - {}", a.title, artist)
                    };
                    entries.push((text, false, Some(global_idx)));
                    global_idx += 1;
                }
            }
            if !results.tracks.is_empty() {
                entries.push((format!("── Tracks ({}) ──", results.tracks.len()), true, None));
                for tr in &results.tracks {
                    entries.push((format!("  {} - {}", tr.title, tr.track_artist()), false, Some(global_idx)));
                    global_idx += 1;
                }
            }

            let display_selected = entries.iter()
                .position(|(_, _, idx)| *idx == Some(selected_idx))
                .unwrap_or(0);
            let scroll_offset = match launcher.scroll_pin {
                Some(pinned) => pinned,
                None => NavigationService::calc_scroll_offset(display_selected, visible_height, entries.len()),
            };

            let items: Vec<ListItem> = entries.iter()
                .skip(scroll_offset)
                .take(visible_height)
                .map(|(text, is_header, sel_idx)| {
                    if *is_header {
                        ListItem::new(text.as_str())
                            .style(Style::default().fg(t.colors.fg_accent))
                    } else {
                        let is_selected = is_focused && *sel_idx == Some(selected_idx);
                        let style = if is_selected {
                            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                        } else {
                            Style::default().fg(t.colors.fg_primary)
                        };
                        ListItem::new(text.as_str()).style(style)
                    }
                })
                .collect();

            frame.render_widget(List::new(items), area);

            // Scrollbar for long lists
            if entries.len() > visible_height {
                crate::ui::widgets::render_scrollbar_borderless(frame, area, entries.len(), visible_height, scroll_offset);
            }
        }
        SearchTab::Artists => {
            render_simple_list(frame, &results.artists, |a| a.title.clone(), selected_idx, is_focused, visible_height, launcher.scroll_pin, area);
        }
        SearchTab::Albums => {
            render_simple_list(frame, &results.albums, |a| {
                let artist = a.artist_name();
                if let Some(year) = a.year {
                    format!("{} ({}) - {}", a.title, year, artist)
                } else {
                    format!("{} - {}", a.title, artist)
                }
            }, selected_idx, is_focused, visible_height, launcher.scroll_pin, area);
        }
        SearchTab::Tracks => {
            render_simple_list(frame, &results.tracks, |tr| {
                format!("{} - {}", tr.title, tr.track_artist())
            }, selected_idx, is_focused, visible_height, launcher.scroll_pin, area);
        }
        SearchTab::Playlists => {
            render_simple_list(frame, &results.playlists, |p| p.title.clone(), selected_idx, is_focused, visible_height, launcher.scroll_pin, area);
        }
        SearchTab::Genres => {
            render_simple_list(frame, &results.genres, |g| g.title.clone(), selected_idx, is_focused, visible_height, launcher.scroll_pin, area);
        }
    }
}

/// Render a simple list of items (no section headers) for per-tab views.
fn render_simple_list<T, F>(
    frame: &mut Frame,
    items: &[T],
    format_fn: F,
    selected_idx: usize,
    is_focused: bool,
    visible_height: usize,
    scroll_pin: Option<usize>,
    area: Rect,
)
where
    F: Fn(&T) -> String,
{
    let t = theme();

    if items.is_empty() {
        let empty = Paragraph::new("No matches")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, area);
        return;
    }

    let scroll_offset = match scroll_pin {
        Some(pinned) => pinned,
        None => NavigationService::calc_scroll_offset(selected_idx, visible_height, items.len()),
    };

    let list_items: Vec<ListItem> = items.iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(i, item)| {
            let is_selected = is_focused && i == selected_idx;
            let text = format!("  {}", format_fn(item));
            let style = if is_selected {
                Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
            } else {
                Style::default().fg(t.colors.fg_primary)
            };
            ListItem::new(text).style(style)
        })
        .collect();

    frame.render_widget(List::new(list_items), area);

    // Scrollbar for long lists
    if items.len() > visible_height {
        crate::ui::widgets::render_scrollbar_borderless(frame, area, items.len(), visible_height, scroll_offset);
    }
}

/// Render step 2: track count input.
fn render_track_count(frame: &mut Frame, launcher: &AdventureLauncherState, area: Rect) {
    let t = theme();

    let popup_area = centered_rect(40, 20, area);
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" adventure length ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.fg_accent))
        .style(Style::default().bg(t.colors.bg_primary));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(2), // start track info
            ratatui::layout::Constraint::Length(3), // input
            ratatui::layout::Constraint::Min(1),    // hint
        ])
        .split(inner);

    // Start track info
    let start_info = if let Some(ref track) = launcher.start_track {
        format!("Start: {} — {}", track.title, track.track_artist())
    } else {
        "Start: (none)".to_string()
    };
    let info = Paragraph::new(vec![
        Line::from(Span::styled("Step 2/3", Style::default().fg(t.colors.fg_muted))),
        Line::from(Span::styled(start_info, Style::default().fg(t.colors.fg_primary))),
    ]);
    frame.render_widget(info, chunks[0]);

    // Input
    let input_block = Block::default()
        .title(" tracks ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border_focused))
        .style(Style::default().bg(t.colors.bg_primary));
    let input_inner = input_block.inner(chunks[1]);
    frame.render_widget(input_block, chunks[1]);

    let input_text = format!("{}▋", launcher.track_count_input);
    let input = Paragraph::new(input_text).style(Style::default().fg(t.colors.fg_primary));
    frame.render_widget(input, input_inner);

    // Hint
    let hint = Paragraph::new(Span::styled(
        "Enter number of tracks (5-100). Enter to continue, Esc to go back.",
        Style::default().fg(t.colors.fg_muted).italic(),
    ));
    frame.render_widget(hint, chunks[2]);
}

/// Format duration from milliseconds to "m:ss".
fn format_duration(ms: u64) -> String {
    let secs = ms / 1000;
    let m = secs / 60;
    let s = secs % 60;
    format!("{}:{:02}", m, s)
}

