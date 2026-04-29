//! Queue and Now Playing screens.
//!
//! Queue view: shows track list with stations panel and artwork.
//! Now Playing view: shows artwork, track info, and waveform seekbar.

use crate::app::state::{NowPlayingFocus, PlaybackMode, PlayStatus, QueueSortMode, VisualizerTab};
use crate::app::AppState;
use crate::services::NavigationService;
use crate::ui::theme::theme;
use crate::ui::artwork::ArtworkRenderer;
use crate::util::format_duration;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Tabs};
use ratatui_image::picker::Picker;
use std::cell::RefCell;

thread_local! {
    static ARTWORK_RENDERER: RefCell<ArtworkRenderer> = RefCell::new(ArtworkRenderer::new());
}

/// Initialize the artwork renderer with a pre-detected picker.
/// Must be called before the event reader task starts consuming stdin.
pub fn init_artwork_renderer(picker: Picker) {
    ARTWORK_RENDERER.with(|r| {
        *r.borrow_mut() = ArtworkRenderer::new_with_picker(picker);
    });
}

/// Get the name of the detected graphics protocol (for settings display).
pub fn artwork_protocol_name() -> &'static str {
    ARTWORK_RENDERER.with(|r| r.borrow().protocol_name())
}

/// Change the artwork renderer's graphics protocol.
pub fn set_artwork_protocol_type(protocol_type: ratatui_image::picker::ProtocolType) {
    ARTWORK_RENDERER.with(|r| r.borrow_mut().set_protocol_type(protocol_type));
}

/// Set the artwork rendering mode and clear caches.
pub fn set_artwork_mode(mode: crate::app::state::ArtworkMode) {
    ARTWORK_RENDERER.with(|r| r.borrow_mut().set_mode(mode));
}

/// Restore the artwork renderer's native protocol detected at startup.
pub fn restore_artwork_native_protocol() {
    ARTWORK_RENDERER.with(|r| r.borrow_mut().restore_native_protocol());
}

/// Clear the artwork cache to force re-rendering on next frame.
/// Call this when transitioning from a view that overlaid the artwork (e.g., Similar popup).
pub fn clear_artwork_cache() {
    ARTWORK_RENDERER.with(|r| r.borrow_mut().clear());
}

/// Format "Artist — Album (Year)" for queue display.
/// Uses helper methods that handle empty/None fields with fallbacks.
fn format_artist_album(track: &crate::plex::models::Track) -> String {
    let artist = track.track_artist();
    let album = track.album_name();
    let year = track.year.or(track.parent_year);

    let album_part = if let Some(y) = year {
        format!("{} ({})", album, y)
    } else {
        album.to_string()
    };

    format!("{} — {}", artist, album_part)
}

/// Render the queue view — left sidebar (Radio / DJ Modes / Remix
/// Tools / Clear Queue), middle queue list + stations, right
/// artwork. Mirrors the GUI's Queue layout where the row of action
/// buttons sits beside the dominant queue list and the artwork is
/// the right-hand element.
pub fn render_queue_mode(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    frame.render_widget(
        Block::default().style(Style::default().bg(t.colors.bg_primary)),
        area,
    );

    let sidebar_w: u16 = 18;
    // Make the artwork box square in pixels: terminal cells are
    // ~2:1 (height:width) so width = 2 × height in cells gives a
    // pixel-square. Bound by the available column width so it
    // can't squeeze the queue list, with a sensible floor.
    let art_width = (area.height.saturating_mul(2))
        .min(area.width.saturating_sub(sidebar_w + 24))
        .max(20);

    // Three-column horizontal split: sidebar | queue (shrunken to
    // Min(24) so the artwork can grow) | artwork.
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(sidebar_w),
            Constraint::Min(24),
            Constraint::Length(art_width),
        ])
        .split(area);

    // Queue takes the full middle column — the old inline Radio
    // station panel that lived under it is gone (its features are
    // reachable via the Radio sidebar button → command palette,
    // mirroring the GUI's "stations live in a popup" pattern).
    let queue_area = chunks[1];

    {
        let track_block = ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::ALL);
        let track_inner = track_block.inner(queue_area);
        let mut hr = state.hit_regions.borrow_mut();
        hr.queue_content = Some(crate::ui::hit_regions::QueueRegions {
            // Stations panel removed; alias to the queue area so any
            // residual click-routing logic that consults this rect
            // doesn't dereference a stale region.
            station_panel: Rect { x: 0, y: 0, width: 0, height: 0 },
            station_inner: Rect { x: 0, y: 0, width: 0, height: 0 },
            track_list: queue_area,
            track_list_inner: track_inner,
            art_area: chunks[2],
        });
    }

    // Artwork box fills the entire right column. Outer layout
    // already sized the column so width = 2 × height in cells,
    // i.e. a pixel-square box — no wasted rows beneath the image.
    render_now_playing_sidebar(frame, state, chunks[0]);
    render_track_list(frame, state, queue_area);
    render_artwork(frame, state, chunks[2]);
}

/// Render the left sidebar with the four GUI-parity buttons:
/// Radio / DJ Modes / Remix Tools / Clear Queue. Each button has a
/// hit region; the mouse handler dispatches the right action (most
/// open the command palette pre-filtered to the relevant subset).
fn render_now_playing_sidebar(frame: &mut Frame, state: &AppState, area: Rect) {
    use ratatui::widgets::{Block, Borders};
    use crate::ui::theme::theme;
    let t = theme();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border))
        .style(Style::default().bg(t.colors.bg_primary).fg(t.colors.fg_primary));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let buttons: [(&str, NpSidebarButton); 4] = [
        ("Radio",       NpSidebarButton::Radio),
        ("DJ Modes",    NpSidebarButton::DjModes),
        ("Remix Tools", NpSidebarButton::Remix),
        ("Clear Queue", NpSidebarButton::ClearQueue),
    ];

    let sidebar_focused = matches!(state.now_playing_focus, crate::app::state::NowPlayingFocus::Sidebar);
    let mut regions: Vec<(Rect, NpSidebarButton)> = Vec::with_capacity(4);
    for (i, (label, btn)) in buttons.iter().copied().enumerate() {
        // Two rows per button: the label, then a blank spacer.
        let y = inner.y + (i as u16) * 2;
        if y >= inner.y + inner.height {
            break;
        }
        let row = Rect { x: inner.x, y, width: inner.width, height: 1 };

        // Highlight precedence: keyboard-focused selection > active
        // state (radio playing / DJ on) > inactive.
        let is_focus_target = sidebar_focused && state.now_playing_sidebar_index == i;
        let active = btn == NpSidebarButton::Radio
            && matches!(state.playback_mode, crate::app::state::PlaybackMode::Radio);
        let active = active
            || (btn == NpSidebarButton::DjModes && state.dj.active_mode.is_some());

        let style = if is_focus_target {
            Style::default()
                .fg(t.colors.fg_primary)
                .bg(t.colors.bg_selection)
                .add_modifier(ratatui::style::Modifier::BOLD)
        } else if active {
            Style::default()
                .fg(t.colors.fg_primary)
                .bg(t.colors.bg_highlight)
        } else {
            Style::default().fg(t.colors.fg_primary).bg(t.colors.bg_primary)
        };
        let prefix = if is_focus_target { "▶ " } else { "  " };
        let label = format!("{}{}", prefix, label);
        frame.render_widget(
            ratatui::widgets::Paragraph::new(label).style(style),
            row,
        );
        regions.push((row, btn));
    }

    state.hit_regions.borrow_mut().now_playing_sidebar = Some(regions);
}

/// Identifies which sidebar button was clicked; the mouse handler
/// resolves it to an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NpSidebarButton {
    Radio,
    DjModes,
    Remix,
    ClearQueue,
}

/// Render the album artwork.
fn render_artwork(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    let is_focused = state.now_playing_focus == NowPlayingFocus::Artwork;
    let border_color = if is_focused { t.colors.title_focused } else { t.colors.fg_accent };
    let title_color = if is_focused { t.colors.title_focused } else { t.colors.fg_accent };
    let block = Block::default()
        .title(" artwork ")
        .title_style(Style::default().fg(title_color))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let (Some(ref data), Some(ref thumb)) = (&state.artwork.current_data, &state.artwork.current_thumb) {
        ARTWORK_RENDERER.with(|renderer| {
            let mut renderer = renderer.borrow_mut();
            if renderer.load_image(data, thumb) {
                renderer.render(frame, inner);
            } else {
                render_artwork_placeholder(frame, inner, "Image load failed");
            }
        });
    } else if state.artwork.loading {
        render_artwork_placeholder(frame, inner, "Loading...");
    } else {
        render_artwork_placeholder(frame, inner, "No artwork");
    }
}

fn render_artwork_placeholder(frame: &mut Frame, area: Rect, message: &str) {
    let t = theme();

    let placeholder = Paragraph::new(message)
        .style(Style::default().fg(t.colors.fg_muted))
        .alignment(Alignment::Center);

    let y_offset = area.height / 2;
    let centered = Rect {
        x: area.x,
        y: area.y + y_offset,
        width: area.width,
        height: 1,
    };
    frame.render_widget(placeholder, centered);
}


/// Render the stations panel below artwork in queue mode.
fn render_station_panel(frame: &mut Frame, state: &AppState, area: Rect) {
    use crate::util::truncate_middle;
    let t = theme();

    let is_focused = state.now_playing_focus == NowPlayingFocus::Stations;
    let border_color = if is_focused { t.colors.border_focused } else { t.colors.border };

    let title = state.station_nav.current_title();
    let block = Block::default()
        .title(format!(" {} ", title))
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.station_nav.loading || state.stations_loading {
        let loading = Paragraph::new("Loading...")
            .style(Style::default().fg(t.colors.fg_muted));
        frame.render_widget(loading, inner);
        return;
    }

    let col = match state.station_nav.focused() {
        Some(col) => col,
        None => {
            let msg = Paragraph::new("No stations")
                .style(Style::default().fg(t.colors.fg_muted));
            frame.render_widget(msg, inner);
            return;
        }
    };

    if col.stations.is_empty() {
        let msg = Paragraph::new("(empty)")
            .style(Style::default().fg(t.colors.fg_muted));
        frame.render_widget(msg, inner);
        return;
    }

    let has_back_item = col.key.is_some(); // Non-root columns get a "← back" row
    let back_rows = if has_back_item { 1 } else { 0 };
    let station_visible_height = (inner.height as usize).saturating_sub(back_rows);
    let selected_idx = col.selected_index;
    let total_items = col.stations.len();
    let max_text_width = inner.width.saturating_sub(3) as usize;

    let scroll_offset = match state.scroll.station {
        Some(pinned) => pinned,
        None => NavigationService::calc_scroll_offset(selected_idx, station_visible_height, total_items),
    };

    // Render "◂ back" row for non-root columns
    if has_back_item {
        let back_style = if is_focused && state.scroll.station_back_highlighted {
            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
        } else if is_focused {
            Style::default().fg(t.colors.shortcut_key)
        } else {
            Style::default().fg(t.colors.fg_muted)
        };
        let back_item = Paragraph::new("◂ back").style(back_style);
        let back_area = Rect::new(inner.x, inner.y, inner.width, 1);
        frame.render_widget(back_item, back_area);
    }

    // Determine active station key and active DJ mode for visual indicators
    let active_station_key = state.radio.active_station.as_ref().map(|s| s.key.as_str());
    let active_dj_mode = state.dj.active_mode;
    // Ancestor key at current column depth (for ♪ on parent categories)
    let ancestor_key = state.radio.active_station.as_ref().and_then(|_|
        state.radio.playing_station_ancestors
            .get(state.station_nav.focused_column)
            .map(|k| k.as_str())
    );

    let visible_items: Vec<ListItem> = col.stations.iter()
        .enumerate()
        .skip(scroll_offset)
        .take(station_visible_height)
        .map(|(i, station)| {
            let is_selected = i == selected_idx;

            // Separator: render as dim horizontal line
            if station.is_separator() {
                let line = "\u{2500}".repeat(max_text_width.min(inner.width as usize));
                return ListItem::new(line).style(Style::default().fg(t.colors.fg_muted));
            }

            let is_category = station.is_category();
            let is_action = station.station_type == "action";
            let is_dj = station.is_dj_mode();
            let is_remix = station.is_remix();

            // Check active state
            let is_active_station = active_station_key == Some(station.key.as_str());
            let is_active_dj = is_dj && active_dj_mode.map(|m| m.key() == station.key.as_str()).unwrap_or(false);
            let is_friendganger = station.key == "dj:friendganger";
            let is_active_shuffle = station.key == "remix:shuffle" && state.queue.shuffle_undo_queue.is_some();
            let is_ancestor_of_playing = ancestor_key == Some(station.key.as_str());

            // Build display text with prefix
            // Active station/DJ/shuffle and ancestor categories get ♪ indicator
            let prefix = if is_active_station || is_active_dj || is_active_shuffle || is_ancestor_of_playing {
                "\u{266a} " // ♪
            } else {
                ""
            };
            let suffix = if is_category && !is_action && !is_dj && !is_remix { " \u{203a}" } else { "" }; // ›

            let available_width = max_text_width.saturating_sub(prefix.len() + suffix.len());
            let display_title = truncate_middle(&station.title, available_width);

            let back_active = state.scroll.station_back_highlighted;
            let style = if is_selected && is_focused && !back_active {
                Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
            } else if is_selected && !back_active {
                Style::default().fg(t.colors.fg_primary).bg(t.colors.bg_secondary)
            } else if is_friendganger {
                // Grayed out (unavailable)
                Style::default().fg(t.colors.fg_muted)
            } else if is_active_dj || is_active_shuffle {
                // Active DJ/shuffle: accent
                Style::default().fg(t.colors.fg_accent)
            } else if is_active_station || is_ancestor_of_playing {
                Style::default().fg(t.colors.fg_accent)
            } else if is_action || is_dj || is_remix {
                Style::default().fg(t.colors.fg_accent)
            } else {
                Style::default().fg(t.colors.fg_primary)
            };
            ListItem::new(format!("{}{}{}", prefix, display_title, suffix)).style(style)
        })
        .collect();

    let list = List::new(visible_items);
    let list_area = Rect::new(inner.x, inner.y + back_rows as u16, inner.width, station_visible_height as u16);
    frame.render_widget(list, list_area);

    // Scrollbar + position indicator for long lists
    if total_items > station_visible_height {
        crate::ui::widgets::render_scrollbar(frame, area, total_items, station_visible_height, scroll_offset, None);

        let footer = format!("{}/{}", selected_idx + 1, total_items);
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

/// Render the track list (queue or radio tracks with history).
fn render_track_list(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Title depends on playback mode
    let title = match state.playback_mode {
        PlaybackMode::Radio => {
            let suffix = if state.queue.sort_mode == QueueSortMode::Shuffle { " (shuffled)" } else { "" };
            if let Some(ref station) = state.radio.active_station {
                format!(" {}{} ", station.title, suffix)
            } else if let Some(ref seed) = state.radio.seed {
                format!(" {}{} ", seed.title, suffix)
            } else {
                format!(" radio{} ", suffix)
            }
        }
        PlaybackMode::Queue | PlaybackMode::None => {
            let mut parts = vec!["queue".to_string()];
            if let Some(dj) = state.dj.active_mode {
                parts.push(format!("({})", dj.name()));
            }
            if state.queue.sort_mode == QueueSortMode::Shuffle {
                parts.push("(shuffled)".to_string());
            }
            format!(" {} ", parts.join(" "))
        }
    };

    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.fg_accent))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Get tracks and current index based on mode
    let (tracks, current_idx) = match state.playback_mode {
        PlaybackMode::Radio => (&state.radio.tracks, state.radio.track_index),
        PlaybackMode::Queue | PlaybackMode::None => (&state.queue.tracks, state.queue.index),
    };

    if tracks.is_empty() {
        let msg = match state.playback_mode {
            PlaybackMode::Radio => "Station starting...",
            _ => "Queue is empty. Play a track to start.",
        };
        let empty = Paragraph::new(msg)
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let selected_idx = state.list_state.queue_index;
    let visible_height = inner.height as usize;

    // 2-row layout: each track takes 2 rows (title + artist-album subtitle)
    let visible_item_count = visible_height / 2;
    let max_text_width = inner.width.saturating_sub(4) as usize;
    let subtitle_width = (inner.width as usize).saturating_sub(7);

    let total_display = tracks.len();

    // Calculate scroll offset - center on selected item
    let scroll_offset = match state.scroll.queue {
        Some(pinned) => pinned,
        None => NavigationService::calc_scroll_offset(selected_idx, visible_item_count, total_display),
    };

    let mut items: Vec<ListItem> = Vec::new();

    // Add tracks
    for (i, track) in tracks.iter().enumerate() {
        if i < scroll_offset || i >= scroll_offset + visible_item_count {
            continue;
        }

        let is_current = current_idx == Some(i);
        let is_selected = i == selected_idx;
        let is_multi_selected = state.queue.selected.contains(&i);

        let prefix = if is_current && is_multi_selected { "♪●" } else if is_current { "♪ " } else if is_multi_selected { "● " } else { "  " };

        // Title with empty fallback
        let track_title = if track.title.is_empty() {
            track.file_name().unwrap_or("Unknown Track")
        } else {
            &track.title
        };

        // Title row: marquee if selected
        let title_display = if is_selected && state.view == crate::app::state::View::NowPlaying {
            let marquee_key = format!("np:{}", i);
            let mut marquee = state.marquee.borrow_mut();
            if marquee.selection_key != marquee_key {
                marquee.reset(marquee_key, track_title.to_string(), max_text_width);
            }
            if marquee.phase == crate::app::state::MarqueePhase::Inactive {
                crate::util::truncate_middle(track_title, max_text_width)
            } else {
                let text = marquee.display_text();
                drop(marquee);
                text
            }
        } else {
            crate::util::truncate_middle(track_title, max_text_width)
        };

        // Subtitle row: marquee if selected (independent)
        let subtitle_content = format_artist_album(track);
        let subtitle_display = if is_selected && state.view == crate::app::state::View::NowPlaying && !subtitle_content.is_empty() {
            let sub_key = format!("np:{}:sub", i);
            let mut sub_marquee = state.marquee_subtitle.borrow_mut();
            if sub_marquee.selection_key != sub_key {
                sub_marquee.reset(sub_key, subtitle_content.clone(), subtitle_width);
            }
            if sub_marquee.phase == crate::app::state::MarqueePhase::Inactive {
                crate::util::truncate_middle(&subtitle_content, subtitle_width)
            } else {
                let text = sub_marquee.display_text();
                drop(sub_marquee);
                text
            }
        } else {
            crate::util::truncate_middle(&subtitle_content, subtitle_width)
        };

        let tracks_focused = state.now_playing_focus == NowPlayingFocus::Tracks;
        let (line1_fg, line2_fg, item_bg) = if is_selected && tracks_focused {
            // Selection bar always wins when tracks focused (even on currently playing)
            (
                Style::default().fg(t.colors.selection_text).bold(),
                Style::default().fg(t.colors.selection_text),
                Style::default().bg(t.colors.selection_bar_bg),
            )
        } else if is_selected {
            // Dim selection when stations panel is focused
            (
                Style::default().fg(t.colors.fg_primary),
                Style::default().fg(t.colors.fg_muted),
                Style::default().bg(t.colors.bg_secondary),
            )
        } else if is_current && is_multi_selected {
            // Currently playing AND multi-selected
            (
                Style::default().fg(t.colors.fg_accent).bold(),
                Style::default().fg(t.colors.fg_accent),
                Style::default().bg(t.colors.bg_secondary),
            )
        } else if is_current {
            // Currently playing track (not selected)
            (
                Style::default().fg(t.colors.fg_accent).bold(),
                Style::default().fg(t.colors.fg_accent),
                Style::default(),
            )
        } else if is_multi_selected {
            (
                Style::default().fg(t.colors.fg_accent),
                Style::default().fg(t.colors.fg_accent),
                Style::default().bg(t.colors.bg_secondary),
            )
        } else {
            (
                Style::default().fg(t.colors.fg_primary),
                Style::default().fg(t.colors.fg_muted),
                Style::default(),
            )
        };

        let text = Text::from(vec![
            Line::from(Span::styled(format!("{}{}", prefix, title_display), line1_fg)),
            Line::from(Span::styled(format!("     {}", subtitle_display), line2_fg)),
        ]);
        items.push(ListItem::new(text).style(item_bg));
    }

    let list = List::new(items);
    frame.render_widget(list, inner);

    // Scrollbar for long lists
    if total_display > visible_item_count {
        crate::ui::widgets::render_scrollbar(frame, area, total_display, visible_item_count, scroll_offset, None);
    }

    // Footer: position and mode info
    let mode_indicator = match state.playback_mode {
        PlaybackMode::Radio => {
            if state.radio.fetching {
                "Radio (loading...)"
            } else {
                "Radio"
            }
        }
        PlaybackMode::Queue => "Queue",
        PlaybackMode::None => "",
    };

    let footer = if tracks.is_empty() {
        mode_indicator.to_string()
    } else {
        format!("{}/{} | {}", selected_idx + 1, tracks.len(), mode_indicator)
    };

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

/// Render the now playing view with artwork and waveform seekbar.
pub fn render_visualizer_mode(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Fill background
    frame.render_widget(
        Block::default().style(Style::default().bg(t.colors.bg_primary)),
        area,
    );

    // Check if we should show artwork (need enough width)
    let show_artwork = area.width > 50;

    if show_artwork {
        // Top panel: match queue artwork sizing (40% of height)
        let top_height = (area.height * 40 / 100).max(8);
        // Artwork width matches height for square image (2:1 char aspect ratio),
        // capped at 40% of total width
        let art_width = (top_height * 2).min(area.width * 40 / 100).max(25);

        // Layout with artwork: top row has art + info, bottom has waveform
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(top_height),
                Constraint::Min(8),
            ])
            .split(area);

        // Top row: artwork on left, track info on right
        let top_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(art_width),
                Constraint::Min(30),
            ])
            .split(chunks[0]);

        render_artwork_panel(frame, state, top_chunks[0]);
        render_track_info_panel(frame, state, top_chunks[1]);
        render_visualizer_panel(frame, state, chunks[1]);
    } else {
        // Narrow layout: track info on top, waveform below
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5), // Track info
                Constraint::Min(8),    // Waveform
            ])
            .split(area);

        render_track_info_panel(frame, state, chunks[0]);
        render_visualizer_panel(frame, state, chunks[1]);
    }
}

/// Render artwork panel for now playing mode.
fn render_artwork_panel(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.fg_accent))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let (Some(ref data), Some(ref thumb)) = (&state.artwork.current_data, &state.artwork.current_thumb) {
        ARTWORK_RENDERER.with(|renderer| {
            let mut renderer = renderer.borrow_mut();
            if renderer.load_image(data, thumb) {
                renderer.render(frame, inner);
            } else {
                render_artwork_placeholder(frame, inner, "Image load failed");
            }
        });
    } else if state.artwork.loading {
        render_artwork_placeholder(frame, inner, "Loading...");
    } else {
        render_artwork_placeholder(frame, inner, "No artwork");
    }
}

/// Render track information panel for now playing mode.
fn render_track_info_panel(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    let block = Block::default()
        .title(" now playing ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.fg_accent))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(track) = state.current_track() {
        let title = if track.title.is_empty() {
            track.file_name().unwrap_or("Unknown Track")
        } else {
            &track.title
        };
        let artist = track.track_artist();
        let album = track.album_name();

        let status_icon = match state.playback.status {
            PlayStatus::Playing => "▶",
            PlayStatus::Paused => "⏸",
            _ => "⏹",
        };

        let position = format_duration(state.playback.position_ms);
        let duration = format_duration(track.duration_ms());

        let text = vec![
            Line::from(vec![
                Span::styled(format!("{} ", status_icon), Style::default().fg(t.colors.fg_accent)),
                Span::styled(title.to_string(), Style::default().fg(t.colors.fg_primary).bold()),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("   ", Style::default()),
                Span::styled(artist.to_string(), Style::default().fg(t.colors.fg_secondary)),
            ]),
            Line::from(vec![
                Span::styled("   ", Style::default()),
                Span::styled(album.to_string(), Style::default().fg(t.colors.fg_muted)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("   ", Style::default()),
                Span::styled(format!("{} / {}", position, duration), Style::default().fg(t.colors.fg_muted)),
            ]),
        ];

        let paragraph = Paragraph::new(text);
        frame.render_widget(paragraph, inner);
    } else {
        let msg = Paragraph::new("No track playing")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(msg, inner);
    }
}

/// Render the visualizer panel with tab bar (Waveform / Spectrum / Spectrogram).
pub(crate) fn render_visualizer_panel(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 2 || inner.width == 0 {
        return;
    }

    // Tab bar (1 row)
    let tab_area = Rect::new(inner.x, inner.y, inner.width, 1);
    let content_area = Rect::new(inner.x, inner.y + 1, inner.width, inner.height - 1);

    // Register hit regions for mouse click handling
    {
        let mut hr = state.hit_regions.borrow_mut();
        hr.now_playing_content = Some(crate::ui::hit_regions::NowPlayingRegions {
            visualizer_tab_area: tab_area,
            visualizer_content_area: content_area,
        });
    }

    render_visualizer_tab_bar(frame, state, tab_area);

    let content_height = content_area.height as usize;
    let content_width = content_area.width as usize;
    if content_height == 0 || content_width == 0 {
        return;
    }

    match state.visualizer_tab {
        VisualizerTab::Waveform => {
            let mut lines: Vec<Line> = Vec::with_capacity(content_height);
            draw_waveform_seekbar(&mut lines, state, content_height, content_width);
            let paragraph = Paragraph::new(lines).alignment(Alignment::Center);
            frame.render_widget(paragraph, content_area);
        }
        VisualizerTab::Spectrum => {
            let mut lines: Vec<Line> = Vec::with_capacity(content_height);
            draw_spectrum_analyzer(&mut lines, state, content_height, content_width);
            let paragraph = Paragraph::new(lines).alignment(Alignment::Center);
            frame.render_widget(paragraph, content_area);
        }
        VisualizerTab::Spectrogram => {
            let mut lines: Vec<Line> = Vec::with_capacity(content_height);
            draw_spectrogram(&mut lines, state, content_height, content_width);
            let paragraph = Paragraph::new(lines).alignment(Alignment::Center);
            frame.render_widget(paragraph, content_area);
        }
        VisualizerTab::Vectorscope => {
            let lines = draw_vectorscope(state, content_height, content_width);
            let paragraph = Paragraph::new(lines).alignment(Alignment::Left);
            frame.render_widget(paragraph, content_area);
        }
    }
}

/// Render the visualizer tab bar.
fn render_visualizer_tab_bar(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();
    let tabs = [
        VisualizerTab::Waveform,
        VisualizerTab::Spectrum,
        VisualizerTab::Spectrogram,
        VisualizerTab::Vectorscope,
    ];
    let selected = state.visualizer_tab as usize;

    let titles: Vec<Line> = tabs.iter().enumerate().map(|(i, tab)| {
        if i == selected && state.visualizer_tab_focused {
            // Focused tab: selection bar style (same as list selection)
            Line::from(Span::styled(
                format!(" {} ", tab.name()),
                Style::default()
                    .fg(t.colors.selection_text)
                    .bg(t.colors.selection_bar_bg),
            ))
        } else if i == selected {
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

    let tab_widget = Tabs::new(titles)
        .select(selected)
        .highlight_style(Style::default())
        .style(Style::default().bg(t.colors.bg_primary).fg(t.colors.fg_muted))
        .divider(Span::styled(" │ ", Style::default().fg(t.colors.fg_muted)))
        .padding("", "");

    frame.render_widget(tab_widget, area);
}

/// Draw waveform seekbar visualization showing full song amplitude profile.
fn draw_waveform_seekbar(lines: &mut Vec<Line<'static>>, state: &AppState, height: usize, width: usize) {
    let vis_chars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

    // Calculate playback progress
    let progress = if state.playback.duration_ms > 0 {
        state.playback.position_ms as f32 / state.playback.duration_ms as f32
    } else {
        0.0
    };
    let position_col = (progress * width as f32).round() as usize;
    let position_col = position_col.min(width.saturating_sub(1));

    // Check if we have waveform data
    let waveform_available = state.waveform.data.is_some();
    let generating = state.waveform.generating;

    if waveform_available {
        // Braille rendering: each terminal cell hosts a 2 × 4 dot
        // grid, giving us **2× horizontal and 4× vertical**
        // sub-resolution. Bars become roughly quarter-cell wide
        // (vs the half-cell `▌`/`▐` blocks). Two amplitude bins
        // per cell — one per dot column — so the waveform's
        // horizontal density doubles too.
        let _ = vis_chars; // unused on the braille path
        let data = state.waveform.data.as_ref().unwrap();
        let sub_count = width.saturating_mul(2);
        let bins = data.resample(sub_count);

        let total_sub_rows = (height as i32) * 4;
        let center_dot = total_sub_rows / 2;

        for row in 0..height {
            let mut spans: Vec<Span> = Vec::with_capacity(width);
            let cell_top_dot = (row as i32) * 4;

            for col in 0..width {
                let l_amp = *bins.get(col * 2).unwrap_or(&0.0);
                let r_amp = *bins.get(col * 2 + 1).unwrap_or(&0.0);

                // Bar height in sub-rows. Half the total because the
                // bar mirrors above and below the centre line.
                let l_height = (l_amp * (total_sub_rows as f32 / 2.0)).round() as i32;
                let r_height = (r_amp * (total_sub_rows as f32 / 2.0)).round() as i32;

                // Build the cell's 8-bit braille bitmask. Dot
                // mapping (Unicode standard):
                //    (col 0, row 0) → 0x01    (col 1, row 0) → 0x08
                //    (col 0, row 1) → 0x02    (col 1, row 1) → 0x10
                //    (col 0, row 2) → 0x04    (col 1, row 2) → 0x20
                //    (col 0, row 3) → 0x40    (col 1, row 3) → 0x80
                let mut bits: u32 = 0;
                for dy in 0i32..4 {
                    let dot_y = cell_top_dot + dy;
                    let dist = (dot_y - center_dot).abs();
                    if dist < l_height {
                        bits |= match dy {
                            0 => 0x01,
                            1 => 0x02,
                            2 => 0x04,
                            3 => 0x40,
                            _ => 0,
                        };
                    }
                    if dist < r_height {
                        bits |= match dy {
                            0 => 0x08,
                            1 => 0x10,
                            2 => 0x20,
                            3 => 0x80,
                            _ => 0,
                        };
                    }
                }
                let ch = char::from_u32(0x2800 + bits).unwrap_or(' ');

                let is_position = col == position_col;
                let is_played = col < position_col;
                let style = if is_position {
                    Style::default().fg(Color::White).bg(Color::Blue)
                } else if is_played {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::DarkGray)
                };

                spans.push(Span::styled(ch.to_string(), style));
            }

            lines.push(Line::from(spans));
        }
    } else if generating {
        // Show loading state
        let padding = height.saturating_sub(3) / 2;
        for _ in 0..padding {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            "Generating waveform...",
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(""));

        // Show simple progress bar while loading
        let mut bar_spans: Vec<Span> = Vec::new();
        for i in 0..width {
            let ch = if i < position_col { '━' } else if i == position_col { '●' } else { '─' };
            let style = if i <= position_col {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            bar_spans.push(Span::styled(ch.to_string(), style));
        }
        lines.push(Line::from(bar_spans));
    } else {
        // No waveform, show error or loading message
        let padding = height.saturating_sub(3) / 2;
        for _ in 0..padding {
            lines.push(Line::from(""));
        }

        if let Some(ref error) = state.waveform.error {
            lines.push(Line::from(Span::styled(
                format!("Waveform error: {}", error),
                Style::default().fg(Color::Red),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "No waveform data available",
                Style::default().fg(Color::Gray),
            )));
        }
        lines.push(Line::from(""));

        // Show simple progress bar
        let mut bar_spans: Vec<Span> = Vec::new();
        for i in 0..width {
            let ch = if i < position_col { '━' } else if i == position_col { '●' } else { '─' };
            let style = if i <= position_col {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            bar_spans.push(Span::styled(ch.to_string(), style));
        }
        lines.push(Line::from(bar_spans));
    }
}

/// Draw spectrum analyzer visualization — vertical bars colored by frequency band.
fn draw_spectrum_analyzer(lines: &mut Vec<Line<'static>>, state: &AppState, height: usize, width: usize) {
    let bar_chars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

    // Reserve 1 row for seek position indicator at bottom
    let bar_height = height.saturating_sub(1);
    if bar_height == 0 {
        return;
    }

    // Calculate playback progress
    let progress = if state.playback.duration_ms > 0 {
        state.playback.position_ms as f32 / state.playback.duration_ms as f32
    } else {
        0.0
    };
    let position_col = (progress * width as f32).round() as usize;
    let position_col = position_col.min(width.saturating_sub(1));

    let spectrogram_available = state.spectrogram.data.is_some();
    let generating = state.spectrogram.generating;

    if spectrogram_available {
        let data = state.spectrogram.data.as_ref().unwrap();
        let frame_idx = data.frame_at_position(state.playback.position_ms);

        // Discrete bars with gaps: 1 char bar + 1 char gap = 2 chars per bar slot
        let num_bars = width / 2;
        let num_bars = num_bars.max(1);
        let spectrum = data.resample_spectrum(frame_idx, num_bars);

        // Color by frequency band position
        let frequency_band_color = |bar: usize, total: usize| -> Color {
            let frac = bar as f32 / total as f32;
            if frac < 0.20 {
                Color::Red       // bass
            } else if frac < 0.35 {
                Color::Yellow    // low-mid
            } else if frac < 0.55 {
                Color::Green     // mid
            } else if frac < 0.75 {
                Color::Cyan      // high-mid
            } else {
                Color::Blue      // treble
            }
        };

        // Pre-compute bar heights from dB-scaled values (0-255 → 0.0-1.0)
        let bar_values: Vec<f32> = spectrum.iter()
            .map(|&v| v as f32 / 255.0)
            .collect();

        // Auto-gain: scale so the tallest bar fills most of the display.
        // This makes quiet tracks show their frequency profile clearly.
        let max_val = bar_values.iter().cloned().fold(0.0f32, f32::max);
        let gain = if max_val > 0.01 {
            (0.85 / max_val).min(15.0)
        } else {
            1.0
        };
        let bar_values: Vec<f32> = bar_values.iter()
            .map(|&v| (v * gain).min(1.0))
            .collect();

        // Build rows from top to bottom
        for row in 0..bar_height {
            let mut spans: Vec<Span> = Vec::with_capacity(width);
            let row_from_bottom = bar_height - 1 - row;

            for bar_idx in 0..num_bars {
                let val = bar_values.get(bar_idx).copied().unwrap_or(0.0);
                let bar_fill = val * bar_height as f32;
                let full_rows = bar_fill.floor() as usize;
                let partial = bar_fill - full_rows as f32;

                let color = frequency_band_color(bar_idx, num_bars);

                // Bar character (1 char)
                if row_from_bottom < full_rows {
                    spans.push(Span::styled("█", Style::default().fg(color)));
                } else if row_from_bottom == full_rows {
                    let char_idx = (partial * 8.0).round() as usize;
                    if char_idx > 0 {
                        let ch = bar_chars[char_idx.min(7)];
                        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                    } else {
                        spans.push(Span::styled(" ", Style::default()));
                    }
                } else {
                    spans.push(Span::styled(" ", Style::default()));
                }

                // Gap character (1 char) — except after last bar
                if bar_idx < num_bars - 1 {
                    spans.push(Span::styled(" ", Style::default()));
                }
            }

            // Pad remaining columns if width is odd
            while spans.len() < width {
                spans.push(Span::styled(" ", Style::default()));
            }

            lines.push(Line::from(spans));
        }

        // Seek position indicator
        let mut bar_spans: Vec<Span> = Vec::with_capacity(width);
        for i in 0..width {
            let ch = if i < position_col { '━' } else if i == position_col { '●' } else { '─' };
            let style = if i <= position_col {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            bar_spans.push(Span::styled(ch.to_string(), style));
        }
        lines.push(Line::from(bar_spans));
    } else {
        // No spectrogram data yet
        let padding = height.saturating_sub(3) / 2;
        for _ in 0..padding {
            lines.push(Line::from(""));
        }

        if generating || state.spectrogram.error.is_some() {
            // Show "Generating" for both active generation and transient errors
            // (the tick safety net will auto-retry after errors)
            lines.push(Line::from(Span::styled(
                "Generating spectrum data...",
                Style::default().fg(Color::Yellow),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "No spectrum data — play a track to generate",
                Style::default().fg(Color::Gray),
            )));
        }
        lines.push(Line::from(""));

        // Simple progress bar even without spectrum data
        let mut bar_spans: Vec<Span> = Vec::with_capacity(width);
        for i in 0..width {
            let ch = if i < position_col { '━' } else if i == position_col { '●' } else { '─' };
            let style = if i <= position_col {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            bar_spans.push(Span::styled(ch.to_string(), style));
        }
        lines.push(Line::from(bar_spans));
    }
}

/// Draw spectrogram visualization — 2D time×frequency with half-block characters.
/// Frequency on x-axis (left=low, right=high), time scrolls vertically (current at bottom).
/// ANSI vectorscope: stereo Lissajous trace (right channel → X,
/// left channel → Y) rendered with Unicode braille glyphs so each
/// terminal cell carries a 2 × 4 sub-pixel grid. Mirrors the GUI's
/// `Vectorscope` widget, sourcing samples from
/// `state.vectorscope_buffer`.
///
/// Empty buffer / no audio → centered "No audio data" placeholder.
fn draw_vectorscope(state: &AppState, height: usize, width: usize) -> Vec<Line<'static>> {
    let t = theme();
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(height);

    if width == 0 || height == 0 {
        return lines;
    }

    let samples = &state.vectorscope_buffer;
    if samples.len() < 2 {
        // Center a "no audio" message on a blank pane.
        let msg = if state.audio_available {
            "No audio data — start playback to see the trace"
        } else {
            "No audio backend"
        };
        for row in 0..height {
            if row == height / 2 {
                let pad = width.saturating_sub(msg.chars().count()) / 2;
                let line_text = format!("{}{}", " ".repeat(pad), msg);
                lines.push(Line::from(Span::styled(
                    line_text,
                    Style::default().fg(t.colors.fg_muted),
                )));
            } else {
                lines.push(Line::from(" ".repeat(width)));
            }
        }
        return lines;
    }

    // 2 × 4 sub-pixel grid per terminal cell (braille). Build a
    // bitmap of lit dots, then convert each cell to its braille
    // character.
    let dots_w = width.saturating_mul(2);
    let dots_h = height.saturating_mul(4);
    if dots_w == 0 || dots_h == 0 {
        return lines;
    }
    let mut bits: Vec<u8> = vec![0u8; width * height];

    // Plot each (l, r) sample as a single dot. Right channel drives
    // X (0 → left edge, 1 → right edge); left channel drives Y
    // (Y is inverted so positive amplitude reads "up").
    let max_x = (dots_w - 1) as f32;
    let max_y = (dots_h - 1) as f32;
    for (l, r) in samples.iter() {
        let l = l.clamp(-1.0, 1.0);
        let r = r.clamp(-1.0, 1.0);
        let xf = ((r + 1.0) * 0.5) * max_x;
        let yf = (1.0 - (l + 1.0) * 0.5) * max_y;
        let x = xf.round() as usize;
        let y = yf.round() as usize;
        if x >= dots_w || y >= dots_h {
            continue;
        }
        // Braille dot bitmask:
        //   (col 0, row 0)→0x01    (col 1, row 0)→0x08
        //   (col 0, row 1)→0x02    (col 1, row 1)→0x10
        //   (col 0, row 2)→0x04    (col 1, row 2)→0x20
        //   (col 0, row 3)→0x40    (col 1, row 3)→0x80
        let cell_x = x / 2;
        let cell_y = y / 4;
        let dx = x % 2;
        let dy = y % 4;
        let bit: u8 = match (dx, dy) {
            (0, 0) => 0x01,
            (0, 1) => 0x02,
            (0, 2) => 0x04,
            (0, 3) => 0x40,
            (1, 0) => 0x08,
            (1, 1) => 0x10,
            (1, 2) => 0x20,
            (1, 3) => 0x80,
            _ => 0,
        };
        let cell_idx = cell_y * width + cell_x;
        if let Some(slot) = bits.get_mut(cell_idx) {
            *slot |= bit;
        }
    }

    // Cyan trace, matching the GUI canvas. Empty cells are left
    // blank so the dark background reads through.
    let trace_style = Style::default().fg(ratatui::style::Color::Cyan);
    let blank_style = Style::default();
    for row in 0..height {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut buf = String::with_capacity(width);
        let mut current_lit: Option<bool> = None;
        for col in 0..width {
            let mask = bits[row * width + col];
            let lit = mask != 0;
            // Group consecutive lit / unlit cells into single spans
            // so we don't emit `width` Span objects per row.
            if Some(lit) != current_lit && !buf.is_empty() {
                let style = if current_lit == Some(true) { trace_style } else { blank_style };
                spans.push(Span::styled(std::mem::take(&mut buf), style));
            }
            current_lit = Some(lit);
            if lit {
                let ch = char::from_u32(0x2800 + mask as u32).unwrap_or(' ');
                buf.push(ch);
            } else {
                buf.push(' ');
            }
        }
        if !buf.is_empty() {
            let style = if current_lit == Some(true) { trace_style } else { blank_style };
            spans.push(Span::styled(buf, style));
        }
        lines.push(Line::from(spans));
    }

    lines
}

fn draw_spectrogram(lines: &mut Vec<Line<'static>>, state: &AppState, height: usize, width: usize) {
    // Reserve 1 row for seek position indicator at bottom
    let vis_height = height.saturating_sub(1);
    if vis_height == 0 {
        return;
    }

    // Calculate playback progress
    let progress = if state.playback.duration_ms > 0 {
        state.playback.position_ms as f32 / state.playback.duration_ms as f32
    } else {
        0.0
    };
    let position_col = (progress * width as f32).round() as usize;
    let position_col = position_col.min(width.saturating_sub(1));

    let spectrogram_available = state.spectrogram.data.is_some();
    let generating = state.spectrogram.generating;

    if spectrogram_available {
        let data = state.spectrogram.data.as_ref().unwrap();
        let current_frame = data.frame_at_position(state.playback.position_ms);

        // Each terminal row uses half-block chars (▀) for 2x vertical resolution
        // So we need vis_height * 2 frames of data
        let pixel_rows = vis_height * 2;

        // Window of frames ending at current_frame (newest at bottom)
        // pixel_rows frames total, bottom pixel = current_frame
        for row in 0..vis_height {
            let mut spans: Vec<Span> = Vec::with_capacity(width);

            // Two pixel rows per terminal row: top and bottom
            let top_pixel = row * 2;
            let bottom_pixel = row * 2 + 1;

            // Map pixel rows to frames (top of display = oldest, bottom = newest)
            let top_frame_offset = pixel_rows.saturating_sub(1) - top_pixel;
            let bottom_frame_offset = pixel_rows.saturating_sub(1) - bottom_pixel;

            let top_frame = if current_frame >= top_frame_offset {
                current_frame - top_frame_offset
            } else {
                0
            };
            let bottom_frame = if current_frame >= bottom_frame_offset {
                current_frame - bottom_frame_offset
            } else {
                0
            };

            // Check if these frames are within valid range
            let top_valid = top_frame_offset <= current_frame && top_frame < data.frame_count;
            let bottom_valid = bottom_frame_offset <= current_frame && bottom_frame < data.frame_count;

            let top_spectrum = if top_valid {
                data.resample_spectrum(top_frame, width)
            } else {
                vec![0; width]
            };
            let bottom_spectrum = if bottom_valid {
                data.resample_spectrum(bottom_frame, width)
            } else {
                vec![0; width]
            };

            for col in 0..width {
                let top_val = top_spectrum.get(col).copied().unwrap_or(0);
                let bottom_val = bottom_spectrum.get(col).copied().unwrap_or(0);

                let top_color = intensity_color(top_val);
                let bottom_color = intensity_color(bottom_val);

                // Use ▀ (upper half block): fg = top pixel, bg = bottom pixel
                spans.push(Span::styled(
                    "▀",
                    Style::default().fg(top_color).bg(bottom_color),
                ));
            }
            lines.push(Line::from(spans));
        }

        // Seek position indicator
        let mut bar_spans: Vec<Span> = Vec::with_capacity(width);
        for i in 0..width {
            let ch = if i < position_col { '━' } else if i == position_col { '●' } else { '─' };
            let style = if i <= position_col {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            bar_spans.push(Span::styled(ch.to_string(), style));
        }
        lines.push(Line::from(bar_spans));
    } else {
        // No spectrogram data yet
        let padding = height.saturating_sub(3) / 2;
        for _ in 0..padding {
            lines.push(Line::from(""));
        }

        if generating {
            lines.push(Line::from(Span::styled(
                "Generating spectrogram...",
                Style::default().fg(Color::Yellow),
            )));
        } else if let Some(ref error) = state.spectrogram.error {
            lines.push(Line::from(Span::styled(
                format!("Spectrogram error: {}", error),
                Style::default().fg(Color::Red),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "No spectrogram data — play a track to generate",
                Style::default().fg(Color::Gray),
            )));
        }
        lines.push(Line::from(""));

        // Simple progress bar
        let mut bar_spans: Vec<Span> = Vec::with_capacity(width);
        for i in 0..width {
            let ch = if i < position_col { '━' } else if i == position_col { '●' } else { '─' };
            let style = if i <= position_col {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            bar_spans.push(Span::styled(ch.to_string(), style));
        }
        lines.push(Line::from(bar_spans));
    }
}

/// Map spectrogram intensity value (0-255) to a color.
/// Colormap: black → dark blue → blue → cyan → green → yellow → white
fn intensity_color(value: u8) -> Color {
    match value {
        0..=15 => Color::Black,
        16..=50 => Color::Rgb(0, 0, ((value - 16) as u16 * 150 / 34) as u8),
        51..=100 => {
            let t = (value - 51) as u16;
            Color::Rgb(0, (t * 255 / 49) as u8, 150 + (t * 105 / 49) as u8)
        }
        101..=150 => {
            let t = (value - 101) as u16;
            Color::Rgb(0, 255, 255 - (t * 255 / 49) as u8)
        }
        151..=200 => {
            let t = (value - 151) as u16;
            Color::Rgb((t * 255 / 49) as u8, 255, 0)
        }
        201..=255 => {
            let t = (value - 201) as u16;
            Color::Rgb(255, 255, (t * 255 / 54) as u8)
        }
    }
}
