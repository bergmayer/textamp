//! Queue and Now Playing screens.
//!
//! Queue view: shows the track list with queue-type and tool menus.
//! Now Playing view: shows artwork, track info, and waveform seekbar.

use crate::app::state::{NowPlayingFocus, PlaybackMode, QueueSortMode, View, VisualizerTab};
use crate::services::NavigationService;
use crate::ui::RenderState as AppState;
use crate::ui::artwork::ArtworkRenderer;
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Tabs};
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

/// Queue metadata uses the same artist and album labels as the desktop queue.
fn format_artist_album(track: &crate::library::models::Track) -> String {
    format!("{} · {}", track.track_artist(), track.album_name())
}

/// Queue menus share the list header, leaving the rest of the pane for tracks.
pub fn render_queue_mode(frame: &mut Frame, state: &AppState, area: Rect) {
    let block = Block::default().borders(Borders::TOP | Borders::LEFT | Borders::RIGHT);
    state.hit_regions.borrow_mut().queue_content = Some(crate::app::presentation::QueueRegions {
        track_list: area,
        track_list_inner: block.inner(area),
        art_area: Rect::default(),
    });
    render_track_list(frame, state, area);
}

pub(crate) fn queue_menu_symbol(menu: crate::app::queue_controls::Menu) -> &'static str {
    match menu {
        crate::app::queue_controls::Menu::QueueType => "◉",
        crate::app::queue_controls::Menu::Tools => "✎",
    }
}

/// Current artwork and metadata occupy the space above the transport and visualizer.
pub fn render_current_track(frame: &mut Frame, state: &AppState, area: Rect) {
    if area.is_empty() {
        return;
    }
    let colors = theme().colors;
    let block = Block::default()
        .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
        .title(" Now Playing ")
        .border_style(Style::new().fg(
            if state.view == View::NowPlaying
                && state.now_playing_focus == NowPlayingFocus::Artwork
                && state.tui_transport_focus.is_none()
                && state.tui_footer_focus.is_none()
                && state.tui_artist_focus.is_none()
            {
                colors.fg_accent
            } else {
                colors.border
            },
        ))
        .style(Style::new().bg(colors.bg_primary));
    let inner = block.inner(area);
    let inner = Rect::new(
        inner.x + u16::from(inner.width > 2),
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    );
    frame.render_widget(block, area);
    let art_width = ((f32::from(inner.height.saturating_sub(1)) * 20. / state.cell_width()).round()
        as u16)
        .min(inner.width.saturating_sub(24));
    let columns =
        Layout::horizontal([Constraint::Length(art_width), Constraint::Min(0)]).split(inner);
    if let Some(regions) = state.hit_regions.borrow_mut().queue_content.as_mut() {
        regions.art_area = columns[0];
    }
    render_current_artwork(frame, state, columns[0]);
    let inner = Rect::new(
        columns[1].x + 3.min(columns[1].width),
        columns[1].y,
        columns[1].width.saturating_sub(3),
        columns[1].height,
    );
    if let Some(track) = state.current_track() {
        let title = Paragraph::new(track.title.as_str())
            .wrap(ratatui::widgets::Wrap { trim: false })
            .style(
                Style::new()
                    .fg(colors.fg_primary)
                    .add_modifier(Modifier::BOLD),
            );
        let title_height = (title.line_count(inner.width) as u16)
            .min(2)
            .min(inner.height);
        frame.render_widget(
            title,
            Rect::new(inner.x, inner.y, inner.width, title_height),
        );
        crate::ui::detail_links::field(
            frame,
            state,
            Rect::new(inner.x, inner.y, inner.width, title_height),
            track,
            crate::app::library_details::Destination::Track,
        );
        let mut y = inner.y.saturating_add(title_height).saturating_add(1);
        let label_width = 9.min(inner.width / 3);
        for (label, value) in crate::app::details::now_playing_rows(track) {
            if y >= inner.bottom() || value.is_empty() {
                continue;
            }
            let field = Paragraph::new(value.as_str())
                .wrap(ratatui::widgets::Wrap { trim: false })
                .style(Style::new().fg(colors.fg_primary));
            let width = inner.width.saturating_sub(label_width);
            if width == 0 {
                break;
            }
            let height = (field.line_count(width) as u16)
                .min(2)
                .min(inner.bottom() - y);
            let value_area = Rect::new(inner.x + label_width, y, width, height);
            frame.render_widget(
                Paragraph::new(label.as_str()).style(Style::new().fg(colors.fg_muted)),
                Rect::new(inner.x, y, label_width, 1),
            );
            frame.render_widget(field, value_area);
            if label == "Album" {
                crate::ui::detail_links::field(
                    frame,
                    state,
                    value_area,
                    track,
                    crate::app::library_details::Destination::Album,
                );
            }
            if label == "Artist" {
                crate::ui::artist_links::visible(
                    frame,
                    state,
                    value_area,
                    track.track_artist(),
                    track.grandparent_rating_key.as_deref(),
                );
            }
            y += height;
        }
    } else {
        frame.render_widget(
            Paragraph::new("Nothing playing").style(Style::new().fg(colors.fg_primary).bold()),
            Rect::new(inner.x, inner.y, inner.width, inner.height.min(1)),
        );
        crate::ui::widgets::logo::render_themed(
            frame,
            Rect::new(
                inner.x,
                inner.y.saturating_add(2),
                inner.width,
                inner.height.saturating_sub(2),
            ),
            &colors,
        );
    }
}

/// Render the album artwork.
/// Shared artwork canvas for normal and full-screen views.
pub(crate) fn render_current_artwork(frame: &mut Frame, state: &AppState, inner: Rect) {
    if inner.is_empty() {
        return;
    }
    if let (Some(data), Some(thumb)) = (&state.artwork.current_data, &state.artwork.current_thumb) {
        ARTWORK_RENDERER.with(|renderer| {
            let mut renderer = renderer.borrow_mut();
            if renderer.load_image(data, thumb) {
                let canvas = if state.fullscreen.is_open() {
                    // Scale preserves aspect ratio but anchors at the top left.
                    // Center its canvas using image headers, without decoding again.
                    image::ImageReader::new(std::io::Cursor::new(data))
                        .with_guessed_format()
                        .ok()
                        .and_then(|reader| reader.into_dimensions().ok())
                        .filter(|(width, height)| *width > 0 && *height > 0)
                        .map(|(width, height)| {
                            let ratio = width as f64 / height as f64;
                            let columns = (inner.height as f64 * 2.0 * ratio)
                                .round()
                                .clamp(1.0, inner.width as f64)
                                as u16;
                            let rows = (columns as f64 / (2.0 * ratio))
                                .round()
                                .clamp(1.0, inner.height as f64)
                                as u16;
                            Rect::new(
                                inner.x + (inner.width - columns) / 2,
                                inner.y + (inner.height - rows) / 2,
                                columns,
                                rows,
                            )
                        })
                        .unwrap_or(inner)
                } else {
                    inner
                };
                renderer.render(frame, canvas);
            } else {
                render_artwork_placeholder(frame, inner, "Image load failed");
            }
        });
    } else if let Some(data) = state.current_artwork_bytes() {
        crate::ui::artwork::render_grid_image(frame, inner, "now-playing-artwork", data);
    } else if state.artwork.fetch.is_loading() {
        render_artwork_placeholder(frame, inner, "Loading...");
    } else {
        crate::ui::artwork::render_placeholder(
            frame,
            inner,
            crate::ui::artwork::Placeholder::Track,
        );
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
    // A candidate station is not the active queue until its request succeeds.
    let title = if let Some(start) = &state.station_starting {
        format!(" Loading {}… (current queue) ", start.title)
    } else {
        match state.playback_mode {
            PlaybackMode::Radio => {
                let suffix = if state.queue.sort_mode == QueueSortMode::Shuffle {
                    " (shuffled)"
                } else {
                    ""
                };
                if let Some(ref station) = state.radio.active_station {
                    format!(" {}{} ", station.title, suffix)
                } else if let Some(ref seed) = state.radio.seed {
                    format!(" {}{} ", seed.title, suffix)
                } else {
                    format!(" radio{} ", suffix)
                }
            }
            PlaybackMode::Queue | PlaybackMode::None => {
                let mut parts = vec!["Play Queue".to_string()];

                if state.queue.sort_mode == QueueSortMode::Shuffle {
                    parts.push("(shuffled)".to_string());
                }
                format!(" {} ", parts.join(" "))
            }
        }
    };

    let color = if state.view == View::Queue && state.now_playing_focus == NowPlayingFocus::Tracks {
        t.colors.border_focused
    } else {
        t.colors.fg_muted
    };
    let block = Block::default()
        .title(crate::util::truncate_middle(
            &title,
            area.width.saturating_sub(10) as usize,
        ))
        .title_style(Style::default().fg(if state.view == View::Queue {
            t.colors.fg_accent
        } else {
            t.colors.fg_secondary
        }))
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
        .border_style(Style::default().fg(color))
        .style(Style::default().bg(t.colors.bg_secondary));

    let inner = block.inner(area);
    frame.render_widget(block, area);
    if area.width >= 10 {
        use crate::app::queue_controls::Menu;
        for (offset, menu, focused) in [
            (8, Menu::QueueType, state.tui_queue_type_focused),
            (
                4,
                Menu::Tools,
                state.view == View::Queue && state.now_playing_focus == NowPlayingFocus::Tools,
            ),
        ] {
            let rect = Rect::new(area.right() - offset, area.y, 3, 1);
            let active = menu == Menu::QueueType
                && crate::app::queue_modes::current(state) != crate::app::queue_modes::Mode::Manual;
            let style = if focused {
                t.colors.focused_row()
            } else {
                Style::default().bg(t.colors.bg_secondary).fg(if active {
                    t.colors.fg_accent
                } else {
                    t.colors.fg_secondary
                })
            };
            frame.render_widget(
                Paragraph::new(format!(
                    "{}{}▾",
                    queue_menu_symbol(menu),
                    if active { "•" } else { " " }
                ))
                .style(style),
                rect,
            );
            state
                .hit_regions
                .borrow_mut()
                .queue_menus
                .push((rect, menu));
        }
    }

    // Get tracks and current index based on mode
    let (tracks, current_idx) = match state.playback_mode {
        PlaybackMode::Radio => (&state.radio.tracks, state.radio.track_index),
        PlaybackMode::Queue | PlaybackMode::None => (&state.queue.tracks, state.queue.index),
    };

    if tracks.is_empty() {
        let msg = if state.station_starting.is_some() {
            "Loading station…"
        } else if state.playback_mode == PlaybackMode::Radio {
            "No radio tracks. Choose a station."
        } else {
            "Queue is empty."
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
        None => {
            NavigationService::calc_scroll_offset(selected_idx, visible_item_count, total_display)
        }
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

        let prefix = if is_current && is_multi_selected {
            "♪●"
        } else if is_current {
            "♪ "
        } else if is_multi_selected {
            "● "
        } else {
            "  "
        };

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
        let subtitle_display = if is_selected
            && state.view == crate::app::state::View::NowPlaying
            && !subtitle_content.is_empty()
        {
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

        let tracks_focused =
            state.view == View::Queue && state.now_playing_focus == NowPlayingFocus::Tracks;
        let (line1_fg, line2_fg, item_bg) = if is_selected && tracks_focused {
            // Selection bar always wins when tracks focused (even on currently playing)
            (
                t.colors.focused_row().bold(),
                t.colors.focused_row(),
                t.colors.focused_row(),
            )
        } else if is_selected && is_current {
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
            Line::from(Span::styled(
                format!("{}{}", prefix, title_display),
                line1_fg,
            )),
            Line::from(Span::styled(format!("     {}", subtitle_display), line2_fg)),
        ]);
        crate::ui::artist_links::text(
            state,
            Rect::new(
                inner.x + 5,
                inner.y + ((i - scroll_offset) * 2 + 1) as u16,
                inner.width.saturating_sub(5),
                1,
            ),
            &subtitle_display,
            track.track_artist(),
            track.grandparent_rating_key.as_deref(),
        );
        items.push(ListItem::new(text).style(item_bg));
    }

    let list = List::new(items);
    frame.render_widget(list, inner);
    if let Some(boundary) = state
        .scroll
        .queue_drag
        .as_ref()
        .and_then(crate::app::queue_pointer::Drag::boundary)
        && boundary >= scroll_offset
        && boundary <= scroll_offset + visible_item_count
    {
        let y = (inner.y + ((boundary - scroll_offset) * 2) as u16)
            .min(inner.bottom().saturating_sub(1));
        frame.render_widget(
            Paragraph::new("─".repeat(inner.width as usize))
                .style(Style::new().fg(t.colors.fg_accent)),
            Rect::new(inner.x, y, inner.width, 1),
        );
    }

    // Scrollbar for long lists
    if total_display > visible_item_count {
        crate::ui::widgets::render_scrollbar(
            frame,
            area,
            total_display,
            visible_item_count,
            scroll_offset,
            None,
        );
    }

    // Footer: position and mode info
    let mode_indicator = match state.playback_mode {
        PlaybackMode::Radio => {
            if state.radio.refill != crate::app::state::RadioRefill::Idle {
                "Radio (loading...)"
            } else {
                "Radio"
            }
        }
        // Queue mode is the default — no footer label, just the row
        // counter so "1/6 | Queue" becomes "1/6". Radio mode keeps
        // its label because it's a meaningful distinction.
        PlaybackMode::Queue | PlaybackMode::None => "",
    };

    let footer = if tracks.is_empty() {
        mode_indicator.to_string()
    } else if mode_indicator.is_empty() {
        format!("{}/{}", selected_idx + 1, tracks.len())
    } else {
        format!("{}/{} | {}", selected_idx + 1, tracks.len(), mode_indicator)
    };

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

/// Render the six-mode visualizer panel. Tabs occupy one row above the canvas.
pub(crate) fn render_visualizer_panel(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if state.view == View::NowPlaying {
            t.colors.border_focused
        } else {
            t.colors.border
        }))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 2 || inner.width == 0 {
        return;
    }

    // Tab bar (1 row)
    let tab_area = Rect::new(inner.x, inner.y, inner.width, 1);
    // Separate dense spectrogram blocks from the labels above them. Some
    // terminal fonts let half-block glyphs touch the preceding text row.
    let gap = if state.graphical_visualizer() {
        2.min(inner.height.saturating_sub(2))
    } else {
        u16::from(state.visualizer_tab == VisualizerTab::Spectrogram && inner.height > 2)
    };
    let content_area = Rect::new(
        inner.x,
        inner.y + 1 + gap,
        inner.width,
        inner.height - 1 - gap,
    );

    // Register hit regions for mouse click handling
    {
        let mut hr = state.hit_regions.borrow_mut();
        hr.now_playing_content = Some(crate::app::presentation::NowPlayingRegions {
            visualizer_tab_area: tab_area,
            visualizer_content_area: content_area,
        });
    }

    render_visualizer_tab_bar(frame, state, tab_area);

    render_visualizer_canvas(frame, state, content_area);
}

/// Shared canvas; full-screen input does not enable waveform seeking.
pub(crate) fn render_visualizer_canvas(frame: &mut Frame, state: &AppState, content_area: Rect) {
    if state.graphical_visualizer() {
        state.hit_regions.borrow_mut().graphical_visualizer = Some(content_area);
        return;
    }
    let content_height = content_area.height as usize;
    let content_width = content_area.width as usize;
    if content_height == 0 || content_width == 0 {
        return;
    }

    match state.visualizer_tab {
        VisualizerTab::Waveform => {
            let mut lines: Vec<Line> = Vec::with_capacity(content_height);
            let ruler = content_height >= 5
                && content_width >= 24
                && state
                    .waveform
                    .data()
                    .is_some_and(|data| !data.bins.is_empty());
            draw_waveform_seekbar(
                &mut lines,
                state,
                content_height - usize::from(ruler) * 2,
                content_width,
            );
            if ruler {
                let mut ticks = vec!['─'; content_width];
                let mut labels = vec![' '; content_width];
                for (fraction, text) in crate::media::time_axis::time_ticks(
                    state.playback.duration_ms,
                    (content_width / 12).clamp(2, 8),
                ) {
                    let x = (fraction * (content_width - 1) as f32).round() as usize;
                    ticks[x] = '┬';
                    let x = x.min(content_width.saturating_sub(text.chars().count()));
                    for (i, c) in text.chars().enumerate() {
                        if x + i < content_width {
                            labels[x + i] = c;
                        }
                    }
                }
                lines.push(Line::from(ticks.into_iter().collect::<String>()));
                lines.push(Line::from(labels.into_iter().collect::<String>()));
            }
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
            frame.render_widget(
                Paragraph::new(lines).alignment(Alignment::Center),
                content_area,
            );
        }
        VisualizerTab::Landscape => super::visualizers::landscape(frame, state, content_area),
        VisualizerTab::Meters => super::visualizers::meters(frame, state, content_area),
    }
}

/// Render the visualizer tab bar.
fn render_visualizer_tab_bar(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();
    let tabs = state.visualizer_tab.visible_tabs(area.width);
    let selected = tabs
        .iter()
        .position(|(tab, _)| *tab == state.visualizer_tab)
        .unwrap_or(0);

    let titles: Vec<Line> = tabs
        .iter()
        .enumerate()
        .map(|(i, (_, label))| {
            if i == selected && state.view == View::NowPlaying && state.visualizer_tab_focused {
                // Focused tab: selection bar style (same as list selection)
                Line::from(Span::styled(
                    format!(" {} ", label),
                    Style::default()
                        .fg(t.colors.selection_text)
                        .bg(t.colors.selection_bar_bg),
                ))
            } else if i == selected {
                Line::from(Span::styled(
                    format!(" {} ", label),
                    Style::default()
                        .fg(t.colors.fg_accent)
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(Span::styled(
                    format!(" {} ", label),
                    Style::default().fg(t.colors.fg_muted),
                ))
            }
        })
        .collect();

    let tab_widget = Tabs::new(titles)
        .select(selected)
        .highlight_style(Style::default())
        .style(
            Style::default()
                .bg(t.colors.bg_primary)
                .fg(t.colors.fg_muted),
        )
        .divider(Span::styled(" │ ", Style::default().fg(t.colors.fg_muted)))
        .padding("", "");

    frame.render_widget(tab_widget, area);
}

/// Draw waveform seekbar visualization showing full song amplitude profile.
fn draw_waveform_seekbar(
    lines: &mut Vec<Line<'static>>,
    state: &AppState,
    height: usize,
    width: usize,
) {
    if state.waveform.data().is_none()
        && !state.waveform.generating()
        && state.waveform.error().is_none()
    {
        lines.extend((0..height).map(|_| Line::from("")));
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

    let generating = state.waveform.generating();

    if let Some(data) = state.waveform.data() {
        // Braille rendering: each terminal cell hosts a 2 × 4 dot
        // grid, giving us **2× horizontal and 4× vertical**
        // sub-resolution. Bars become roughly quarter-cell wide
        // (vs the half-cell `▌`/`▐` blocks). Two amplitude bins
        // per cell — one per dot column — so the waveform's
        // horizontal density doubles too.
        let sub_count = width.saturating_mul(2);
        let total_sub_rows = (height as i32) * 4;
        let center_dot = total_sub_rows / 2;

        // A waveform has only three style runs per row (played, cursor,
        // unplayed). Build those strings directly instead of allocating one
        // String and one Span for every terminal cell on every frame.
        let mut row_segments: Vec<[String; 3]> = (0..height)
            .map(|_| {
                [
                    String::with_capacity(position_col.saturating_mul(3)),
                    String::with_capacity(3),
                    String::with_capacity(width.saturating_sub(position_col + 1).saturating_mul(3)),
                ]
            })
            .collect();

        for col in 0..width {
            let l_amp = data.resampled_peak_at(sub_count, col * 2);
            let r_amp = data.resampled_peak_at(sub_count, col * 2 + 1);

            // Bar height in sub-rows. Half the total because the
            // bar mirrors above and below the centre line.
            let l_height = (l_amp * (total_sub_rows as f32 / 2.0)).round() as i32;
            let r_height = (r_amp * (total_sub_rows as f32 / 2.0)).round() as i32;
            let segment = if col < position_col {
                0
            } else if col == position_col {
                1
            } else {
                2
            };

            for (row, segments) in row_segments.iter_mut().enumerate() {
                let cell_top_dot = (row as i32) * 4;

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
                segments[segment].push(ch);
            }
        }

        let styles = [
            Style::default().fg(Color::Cyan),
            Style::default().fg(Color::White).bg(Color::Blue),
            Style::default().fg(Color::DarkGray),
        ];
        for segments in row_segments {
            let spans = segments
                .into_iter()
                .zip(styles)
                .filter_map(|(text, style)| (!text.is_empty()).then(|| Span::styled(text, style)))
                .collect::<Vec<_>>();
            lines.push(Line::from(spans));
        }
    } else if generating {
        // Show loading state
        let padding = height.saturating_sub(3) / 2;
        for _ in 0..padding {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            state
                .waveform
                .progress()
                .map(|p| p.message())
                .unwrap_or_else(|| "Preparing waveform…".into()),
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(""));

        // Show simple progress bar while loading
        let mut bar_spans: Vec<Span> = Vec::new();
        for i in 0..width {
            let ch = if i < position_col {
                '━'
            } else if i == position_col {
                '●'
            } else {
                '─'
            };
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

        if let Some(error) = state.waveform.error() {
            lines.push(Line::from(Span::styled(
                format!("Waveform error: {}", error),
                Style::default().fg(Color::Red),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "",
                Style::default().fg(Color::Gray),
            )));
        }
        lines.push(Line::from(""));

        // Show simple progress bar
        let mut bar_spans: Vec<Span> = Vec::new();
        for i in 0..width {
            let ch = if i < position_col {
                '━'
            } else if i == position_col {
                '●'
            } else {
                '─'
            };
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
fn draw_spectrum_analyzer(
    lines: &mut Vec<Line<'static>>,
    state: &AppState,
    height: usize,
    width: usize,
) {
    if state.spectrogram.data().is_none()
        && !state.spectrogram.generating()
        && state.spectrogram.error().is_none()
    {
        lines.extend((0..height).map(|_| Line::from("")));
        return;
    }

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

    let spectrogram_available = state.spectrogram.data().is_some();
    let generating = state.spectrogram.generating();

    if spectrogram_available {
        let data = state.spectrogram.data().unwrap();
        let frame_idx = data.frame_at_position(state.playback.position_ms);

        // Discrete bars with gaps: 1 char bar + 1 char gap = 2 chars per bar slot
        let num_bars = width / 2;
        let num_bars = num_bars.max(1);
        let spectrum = data.resample_spectrum(frame_idx, num_bars);

        // Color by frequency band position
        let frequency_band_color = |bar: usize, total: usize| -> Color {
            let frac = bar as f32 / total as f32;
            if frac < 0.20 {
                Color::Red // bass
            } else if frac < 0.35 {
                Color::Yellow // low-mid
            } else if frac < 0.55 {
                Color::Green // mid
            } else if frac < 0.75 {
                Color::Cyan // high-mid
            } else {
                Color::Blue // treble
            }
        };

        // Auto-gain: scale so the tallest bar fills most of the display.
        // This makes quiet tracks show their frequency profile clearly.
        let max_val = spectrum.iter().copied().max().unwrap_or(0) as f32 / 255.0;
        let gain = if max_val > 0.01 {
            (0.85 / max_val).min(15.0)
        } else {
            1.0
        };

        // Build rows from top to bottom
        for row in 0..bar_height {
            let mut spans: Vec<Span> = Vec::with_capacity(width);
            let row_from_bottom = bar_height - 1 - row;

            for bar_idx in 0..num_bars {
                let val = spectrum
                    .get(bar_idx)
                    .map_or(0.0, |value| ((*value as f32 / 255.0) * gain).min(1.0));
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
            let ch = if i < position_col {
                '━'
            } else if i == position_col {
                '●'
            } else {
                '─'
            };
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
                state
                    .spectrogram
                    .progress()
                    .map(|p| p.message())
                    .unwrap_or_else(|| "Preparing spectral data…".into()),
                Style::default().fg(Color::Yellow),
            )));
        } else if let Some(error) = state.spectrogram.error() {
            lines.push(Line::from(Span::styled(
                format!("Spectrogram error: {error}"),
                Style::default().fg(Color::Red),
            )));
        }
        lines.push(Line::from(""));

        // Simple progress bar even without spectrum data
        let mut bar_spans: Vec<Span> = Vec::with_capacity(width);
        for i in 0..width {
            let ch = if i < position_col {
                '━'
            } else if i == position_col {
                '●'
            } else {
                '─'
            };
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

/// ANSI vectorscope: stereo Lissajous trace (right channel → X,
/// left channel → Y) rendered with Unicode braille glyphs so each
/// terminal cell carries a 2 × 4 sub-pixel grid. Mirrors the GUI's
/// `Vectorscope` widget, sourcing samples from
/// `state.vectorscope_buffer`.
///
/// An empty sample buffer leaves the trace area blank.
fn draw_vectorscope(state: &AppState, height: usize, width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(height);

    if width == 0 || height == 0 {
        return lines;
    }

    let samples = &state.vectorscope_buffer;
    if samples.len() < 2 {
        return vec![Line::from(" ".repeat(width)); height];
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
                let style = if current_lit == Some(true) {
                    trace_style
                } else {
                    blank_style
                };
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
            let style = if current_lit == Some(true) {
                trace_style
            } else {
                blank_style
            };
            spans.push(Span::styled(buf, style));
        }
        lines.push(Line::from(spans));
    }

    lines
}

fn draw_spectrogram(lines: &mut Vec<Line<'static>>, state: &AppState, height: usize, width: usize) {
    if state.spectrogram.data().is_none()
        && !state.spectrogram.generating()
        && state.spectrogram.error().is_none()
    {
        lines.extend((0..height).map(|_| Line::from("")));
        return;
    }

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

    let spectrogram_available = state.spectrogram.data().is_some();
    let generating = state.spectrogram.generating();

    if spectrogram_available {
        let data = state.spectrogram.data().unwrap();
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

            let top_frame = current_frame.saturating_sub(top_frame_offset);
            let bottom_frame = current_frame.saturating_sub(bottom_frame_offset);

            // Check if these frames are within valid range
            let top_valid = top_frame_offset <= current_frame && top_frame < data.frame_count;
            let bottom_valid =
                bottom_frame_offset <= current_frame && bottom_frame < data.frame_count;

            for col in 0..width {
                let top_val = if top_valid {
                    data.resampled_spectrum_peak(top_frame, width, col)
                } else {
                    0
                };
                let bottom_val = if bottom_valid {
                    data.resampled_spectrum_peak(bottom_frame, width, col)
                } else {
                    0
                };

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
            let ch = if i < position_col {
                '━'
            } else if i == position_col {
                '●'
            } else {
                '─'
            };
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
                state
                    .spectrogram
                    .progress()
                    .map(|p| p.message())
                    .unwrap_or_else(|| "Preparing spectral data…".into()),
                Style::default().fg(Color::Yellow),
            )));
        } else if let Some(error) = state.spectrogram.error() {
            lines.push(Line::from(Span::styled(
                format!("Spectrogram error: {}", error),
                Style::default().fg(Color::Red),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "",
                Style::default().fg(Color::Gray),
            )));
        }
        lines.push(Line::from(""));

        // Simple progress bar
        let mut bar_spans: Vec<Span> = Vec::with_capacity(width);
        for i in 0..width {
            let ch = if i < position_col {
                '━'
            } else if i == position_col {
                '●'
            } else {
                '─'
            };
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
