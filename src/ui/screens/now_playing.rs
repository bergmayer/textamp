//! Unified Now Playing screen.
//!
//! Shows the current playback (queue, playlist, or station tracks)
//! with album artwork and play history support.
//! Cycles between Queue and Now Playing modes.

use crate::app::state::{PlaybackMode, NowPlayingMode, PlayStatus, QueueSortMode};
use crate::app::AppState;
use crate::services::NavigationService;
use crate::ui::theme::theme;
use crate::ui::artwork::ArtworkRenderer;
use crate::util::{format_duration, truncate_str};

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
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

/// Render the unified now playing screen.
pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Fill background
    frame.render_widget(
        Block::default().style(Style::default().bg(t.colors.bg_primary)),
        area,
    );

    // Render based on now playing mode
    match state.now_playing_mode {
        NowPlayingMode::Queue => render_queue_mode(frame, state, area),
        NowPlayingMode::NowPlaying => render_visualizer_mode(frame, state, area),
    }
}

/// Render the queue mode (original behavior).
fn render_queue_mode(frame: &mut Frame, state: &AppState, area: Rect) {
    // Check if we should show artwork
    let show_artwork = state.artwork_data.is_some() && area.width > 60;

    if show_artwork {
        // Split area into artwork (left) and track list (right)
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(25), // Artwork area (square-ish)
                Constraint::Min(40),    // Track list
            ])
            .split(area);

        render_artwork(frame, state, chunks[0]);
        render_track_list(frame, state, chunks[1]);
    } else {
        render_track_list(frame, state, area);
    }
}

/// Render the album artwork.
fn render_artwork(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    let block = Block::default()
        .title(" artwork ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.fg_accent))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let (Some(ref data), Some(ref thumb)) = (&state.artwork_data, &state.artwork_thumb) {
        ARTWORK_RENDERER.with(|renderer| {
            let mut renderer = renderer.borrow_mut();
            if renderer.load_image(data, thumb) {
                renderer.render(frame, inner);
            } else {
                render_artwork_placeholder(frame, inner, "Image load failed");
            }
        });
    } else if state.artwork_loading {
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

/// Render the track list (queue or radio tracks with history).
fn render_track_list(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Title depends on playback mode
    let title = match state.playback_mode {
        PlaybackMode::Radio => {
            if let Some(ref station) = state.radio.active_station {
                format!(" {} ", station.title)
            } else if let Some(ref seed) = state.radio.seed {
                format!(" {} ", seed.title)
            } else {
                " radio ".to_string()
            }
        }
        PlaybackMode::Queue | PlaybackMode::None => {
            if state.queue_sort_mode == QueueSortMode::Shuffle {
                " queue (shuffled) ".to_string()
            } else {
                " queue ".to_string()
            }
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
        PlaybackMode::Queue | PlaybackMode::None => (&state.queue, state.queue_index),
    };

    if tracks.is_empty() && state.play_history.is_empty() {
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

    // Build combined list: history (dimmed) + current tracks
    let history_len = state.play_history.len();
    let total_display = history_len + tracks.len();

    // Calculate scroll offset - center on selected item in current tracks
    let display_selected = history_len + selected_idx;
    let scroll_offset = NavigationService::calc_scroll_offset(display_selected, visible_height, total_display);

    let mut items: Vec<ListItem> = Vec::new();

    // Add history items (shown dimmed, above current tracks)
    for (i, track) in state.play_history.iter().enumerate() {
        if i < scroll_offset || i >= scroll_offset + visible_height {
            continue;
        }

        let prefix = "  ";
        let title_str = truncate_str(&track.title, 30);
        let artist = truncate_str(&track.artist_name(), 20);
        let duration = format_duration(track.duration_ms());

        let line = format!(
            "{}  {:<32} {:<22} {:>6}",
            prefix, title_str, artist, duration
        );

        items.push(ListItem::new(line).style(Style::default().fg(t.colors.fg_muted)));
    }

    // Add current tracks
    for (i, track) in tracks.iter().enumerate() {
        let display_i = history_len + i;
        if display_i < scroll_offset || display_i >= scroll_offset + visible_height {
            continue;
        }

        let is_current = current_idx == Some(i);
        let is_selected = i == selected_idx;

        let prefix = if is_current { "♪ " } else { "  " };
        let title_str = truncate_str(&track.title, 30);
        let artist = truncate_str(&track.artist_name(), 20);
        let duration = format_duration(track.duration_ms());

        let line = format!(
            "{}  {:<32} {:<22} {:>6}",
            prefix, title_str, artist, duration
        );

        let style = if is_selected {
            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
        } else if is_current {
            Style::default().fg(t.colors.fg_accent).bold()
        } else {
            Style::default().fg(t.colors.fg_primary)
        };

        items.push(ListItem::new(line).style(style));
    }

    let list = List::new(items);
    frame.render_widget(list, inner);

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
        format!("History: {} | {}", history_len, mode_indicator)
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

/// Render the "now playing" mode with artwork and waveform seekbar.
fn render_visualizer_mode(frame: &mut Frame, state: &AppState, area: Rect) {
    // Check if we should show artwork (need enough width)
    let show_artwork = state.artwork_data.is_some() && area.width > 50;

    if show_artwork {
        // Layout with artwork: top row has art + info, bottom has waveform
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(12), // Artwork + track info row
                Constraint::Min(8),     // Waveform seekbar
            ])
            .split(area);

        // Top row: artwork on left, track info on right
        let top_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(24), // Artwork (square-ish)
                Constraint::Min(30),    // Track info
            ])
            .split(chunks[0]);

        render_artwork_panel(frame, state, top_chunks[0]);
        render_track_info_panel(frame, state, top_chunks[1]);
        render_waveform_panel(frame, state, chunks[1]);
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
        render_waveform_panel(frame, state, chunks[1]);
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

    if let (Some(ref data), Some(ref thumb)) = (&state.artwork_data, &state.artwork_thumb) {
        ARTWORK_RENDERER.with(|renderer| {
            let mut renderer = renderer.borrow_mut();
            if renderer.load_image(data, thumb) {
                renderer.render(frame, inner);
            } else {
                render_artwork_placeholder(frame, inner, "Image load failed");
            }
        });
    } else if state.artwork_loading {
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
        let title = &track.title;
        let artist = track.artist_name();
        let album = track.parent_title.as_deref().unwrap_or("");

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
                Span::styled(title.clone(), Style::default().fg(t.colors.fg_primary).bold()),
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

/// Render the waveform seekbar panel.
fn render_waveform_panel(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let inner_height = inner.height as usize;
    let inner_width = inner.width as usize;

    if inner_height == 0 || inner_width == 0 {
        return;
    }

    let mut lines: Vec<Line> = Vec::with_capacity(inner_height);
    draw_waveform_seekbar(&mut lines, state, inner_height, inner_width);

    let paragraph = Paragraph::new(lines).alignment(Alignment::Center);
    frame.render_widget(paragraph, inner);
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
        // Use actual waveform data
        let data = state.waveform.data.as_ref().unwrap();
        let bins = data.resample(width);

        // Calculate vertical center for the waveform
        let center_row = height / 2;

        // Build the seekbar grid
        for row in 0..height {
            let mut spans: Vec<Span> = Vec::with_capacity(width);

            for (col, &amplitude) in bins.iter().enumerate() {
                // Scale amplitude to row
                let bar_height = (amplitude * (height as f32 / 2.0)).round() as usize;
                let distance_from_center = if row < center_row {
                    center_row - row
                } else {
                    row - center_row
                };

                let is_filled = distance_from_center < bar_height;
                let is_position = col == position_col;
                let is_played = col < position_col;

                let ch = if is_filled {
                    // Choose character based on amplitude
                    let level = ((amplitude * 7.0) as usize).min(7);
                    vis_chars[level]
                } else {
                    ' '
                };

                let style = if is_position {
                    // Position marker - bright
                    Style::default().fg(Color::White).bg(Color::Blue)
                } else if is_played {
                    // Played portion - accent color
                    Style::default().fg(Color::Cyan)
                } else {
                    // Unplayed portion - muted
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
