//! Search popup screen (Ctrl+F).
//!
//! Local-first search with tabs: All | Artists | Albums | Playlists | Tracks | Genres

use crate::app::state::{SearchFocus, SearchTab};
use crate::library::models::SearchResults;
use crate::services::NavigationService;
use crate::ui::RenderState as AppState;
use crate::ui::layout::centered_rect;
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs};

/// Render the search popup as an overlay.
pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    state.hit_regions.borrow_mut().artist_links.clear();
    let t = theme();

    let popup_area = centered_rect(65, 75, area);
    state.hit_regions.borrow_mut().track_pane = None;

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);
    crate::ui::widgets::register_popup_close(state, popup_area);

    let block = Block::default()
        .title(" search ")
        .title_bottom(" Enter: show in Library · Esc: close ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.fg_accent))
        .style(Style::default().bg(t.colors.bg_primary));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // Split inner area: tabs, search input, results
    let chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(2),
            ratatui::layout::Constraint::Length(3),
            ratatui::layout::Constraint::Min(3),
        ])
        .split(inner);

    let results_area = chunks[2];

    // Register hit regions for mouse handler
    {
        let mut hr = state.hit_regions.borrow_mut();
        hr.search_popup = Some(crate::app::presentation::SearchPopupRegions {
            outer: popup_area,
            tab_area: chunks[0],
            input_area: chunks[1],
            results_area,
        });
    }

    // Tabs
    render_tabs(frame, state, chunks[0]);

    // Search input
    render_search_input(frame, state, chunks[1]);

    // Results
    render_results(frame, state, results_area);
}

fn render_tabs(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();
    if state.sources.active.folder().is_some() {
        frame.render_widget(
            Paragraph::new("Audio files in current folder · / filters folders")
                .style(Style::default().fg(t.colors.fg_muted)),
            area,
        );
        return;
    }

    let labels = SearchTab::all();
    let selected_idx = match state.search.tab {
        SearchTab::Global => 0,
        SearchTab::Artists => 1,
        SearchTab::Albums => 2,
        SearchTab::Playlists => 3,
        SearchTab::Tracks => 4,
        SearchTab::Genres => 5,
    };

    let is_tab_focused = state.search.focus == SearchFocus::Input;

    let titles: Vec<Line> = labels
        .iter()
        .enumerate()
        .map(|(i, tab)| {
            if i == selected_idx && is_tab_focused {
                Line::from(Span::styled(
                    format!(" {} ", tab.name()),
                    Style::default()
                        .fg(t.colors.selection_text)
                        .bg(t.colors.selection_bar_bg),
                ))
            } else if i == selected_idx {
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
        })
        .collect();

    let tabs = Tabs::new(titles)
        .select(selected_idx)
        .highlight_style(Style::default())
        .style(
            Style::default()
                .bg(t.colors.bg_primary)
                .fg(t.colors.fg_muted),
        )
        .divider(Span::styled(" │ ", Style::default().fg(t.colors.fg_muted)))
        .padding("", "");

    frame.render_widget(tabs, area);
}

fn render_search_input(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    let is_input_focused = state.search.focus == SearchFocus::Input;

    let input_block = Block::default()
        .title(" search ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if is_input_focused {
            t.colors.border_focused
        } else {
            t.colors.border
        }))
        .style(Style::default().bg(t.colors.bg_primary));

    let input_inner = input_block.inner(area);
    frame.render_widget(input_block, area);

    let query_text = state.search.query.clone();
    let fg = if is_input_focused {
        t.colors.fg_primary
    } else {
        t.colors.fg_muted
    };
    let input = Paragraph::new(query_text).style(Style::default().fg(fg));
    frame.render_widget(input, input_inner);
    if is_input_focused {
        crate::ui::widgets::text_input::render(
            frame,
            state,
            input_inner,
            crate::app::terminal_text::Target::Search,
            Style::default().fg(fg).bg(t.colors.bg_primary),
        );
    }
}

fn render_results(frame: &mut Frame, state: &AppState, area: Rect) {
    let scroll_pin = state.scroll.search;
    let t = theme();

    let results = match &state.search.results {
        Some(r) => r,
        None => {
            let msg = if state.search.query.is_empty() {
                "Type to search library"
            } else {
                "Searching..."
            };
            let empty = Paragraph::new(msg)
                .style(Style::default().fg(t.colors.fg_muted))
                .alignment(Alignment::Center);
            frame.render_widget(empty, area);
            return;
        }
    };

    let is_results_focused = state.search.focus == SearchFocus::Results;
    let selected_idx = state.list_state.search_item_index;

    match state.search.tab {
        SearchTab::Global => render_all_tab(
            frame,
            results,
            is_results_focused,
            selected_idx,
            scroll_pin,
            area,
        ),
        SearchTab::Artists => render_single_section(
            frame,
            &results.artists,
            |a| {
                if a.title.is_empty() {
                    "Unknown Artist".to_string()
                } else {
                    a.title.clone()
                }
            },
            is_results_focused,
            selected_idx,
            scroll_pin,
            area,
        ),
        SearchTab::Albums => render_single_section(
            frame,
            &results.albums,
            |a| {
                let artist = a.artist_name();
                let title = if a.title.is_empty() {
                    format!("Unknown Album ({})", artist)
                } else if let Some(year) = a.year {
                    format!("{} ({})", a.title, year)
                } else {
                    a.title.clone()
                };
                format!("{} - {}", title, artist)
            },
            is_results_focused,
            selected_idx,
            scroll_pin,
            area,
        ),
        SearchTab::Playlists => render_single_section(
            frame,
            &results.playlists,
            |p| p.title.clone(),
            is_results_focused,
            selected_idx,
            scroll_pin,
            area,
        ),
        SearchTab::Tracks => {
            render_single_section(
                frame,
                &results.tracks,
                |tr| {
                    let title = if tr.title.is_empty() {
                        tr.file_name().unwrap_or("Unknown Track")
                    } else {
                        &tr.title
                    };
                    format!("{} - {}", title, tr.track_artist())
                },
                is_results_focused,
                selected_idx,
                scroll_pin,
                area,
            );
        }
        SearchTab::Genres => render_single_section(
            frame,
            &results.genres,
            |g| g.title.clone(),
            is_results_focused,
            selected_idx,
            scroll_pin,
            area,
        ),
    }
    register_result_artist_links(frame, state, results, area, scroll_pin, selected_idx);
}

/// Render the All tab with section headers.
fn render_all_tab(
    frame: &mut Frame,
    results: &SearchResults,
    is_focused: bool,
    selected_idx: usize,
    scroll_pin: Option<usize>,
    area: Rect,
) {
    let rows: Vec<_> = (0..results.total_count()).collect();
    render_single_section(
        frame,
        &rows,
        |&i| {
            let (tab, index) =
                crate::app::handlers::dispatch_search::resolve_global_index(results, i);
            match tab {
                SearchTab::Artists => format!("Artist · {}", results.artists[index].title),
                SearchTab::Albums => format!(
                    "Album · {} — {}",
                    results.albums[index].title,
                    results.albums[index].artist_name()
                ),
                SearchTab::Tracks => format!(
                    "Track · {} — {}",
                    results.tracks[index].title,
                    results.tracks[index].track_artist()
                ),
                SearchTab::Playlists => format!("Playlist · {}", results.playlists[index].title),
                SearchTab::Genres => format!("Genre · {}", results.genres[index].title),
                SearchTab::Global => unreachable!(),
            }
        },
        is_focused,
        selected_idx,
        scroll_pin,
        area,
    );
}

/// Render a single-section list (for Artists, Albums, Playlists, Tracks, Genres tabs).
fn render_single_section<T, F>(
    frame: &mut Frame,
    items: &[T],
    format_item: F,
    is_focused: bool,
    selected_idx: usize,
    scroll_pin: Option<usize>,
    area: Rect,
) where
    F: Fn(&T) -> String,
{
    let t = theme();

    if items.is_empty() {
        let msg = "No matches";
        let empty = Paragraph::new(msg)
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, area);
        return;
    }

    let visible_height = area.height as usize;
    let total = items.len();
    let scroll_offset = match scroll_pin {
        Some(pinned) => pinned,
        None => NavigationService::calc_scroll_offset(selected_idx, visible_height, total),
    };

    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(i, item)| {
            let is_selected = is_focused && i == selected_idx;
            let style = if is_selected {
                Style::default()
                    .fg(t.colors.selection_text)
                    .bg(t.colors.selection_bar_bg)
            } else if i == selected_idx {
                t.colors.inactive_row()
            } else {
                Style::default().fg(t.colors.fg_primary)
            };
            ListItem::new(format_item(item)).style(style)
        })
        .collect();

    frame.render_widget(List::new(list_items), area);

    // Scrollbar for long lists
    if total > visible_height {
        crate::ui::widgets::render_scrollbar_borderless(
            frame,
            area,
            total,
            visible_height,
            scroll_offset,
        );
    }
}

fn register_result_artist_links(
    frame: &mut Frame,
    state: &AppState,
    results: &SearchResults,
    area: Rect,
    pin: Option<usize>,
    selected: usize,
) {
    let artist = |artist: &crate::library::models::Artist| {
        Some((artist.title.clone(), Some(artist.rating_key.clone())))
    };
    let album = |album: &crate::library::models::Album| {
        Some((
            album.artist_name().to_owned(),
            album.parent_rating_key.clone(),
        ))
    };
    let track = |track: &crate::library::models::Track| {
        Some((
            track.track_artist().to_owned(),
            track.grandparent_rating_key.clone(),
        ))
    };
    let mut rows = Vec::new();
    if state.search.tab == SearchTab::Global {
        for i in 0..results.total_count() {
            let (tab, index) =
                crate::app::handlers::dispatch_search::resolve_global_index(results, i);
            let link = match tab {
                SearchTab::Artists => artist(&results.artists[index]),
                SearchTab::Albums => album(&results.albums[index]),
                SearchTab::Tracks => track(&results.tracks[index]),
                _ => None,
            };
            rows.push((true, link));
        }
    } else {
        let entries = match state.search.tab {
            SearchTab::Artists => results.artists.iter().map(artist).collect(),
            SearchTab::Albums => results.albums.iter().map(album).collect(),
            SearchTab::Tracks => results.tracks.iter().map(track).collect(),
            _ => Vec::new(),
        };
        rows.extend(entries.into_iter().map(|entry| (true, entry)));
    }
    let mut index = 0;
    let selected_display = rows
        .iter()
        .position(|(selectable, _)| {
            if !selectable {
                return false;
            }
            let matches = index == selected;
            index += 1;
            matches
        })
        .unwrap_or(0);
    let offset = pin.unwrap_or_else(|| {
        NavigationService::calc_scroll_offset(selected_display, area.height as usize, rows.len())
    });
    for (line, (_, artist)) in rows
        .into_iter()
        .skip(offset)
        .take(area.height as usize)
        .enumerate()
    {
        if let Some((name, key)) = artist {
            crate::ui::artist_links::visible(
                frame,
                state,
                Rect::new(area.x, area.y + line as u16, area.width, 1),
                &name,
                key.as_deref(),
            );
        }
    }
}
