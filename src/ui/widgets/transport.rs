//! Playback controls flank a centered track card and scrubber.
//! The fixed footer carries navigation controls and contextual status.

use crate::app::state::PlayStatus;
use crate::app::state::View;
use crate::ui::RenderState as AppState;
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

/// Render a framed transport with centered metadata and a separate scrubber.
pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    use crate::app::transport_controls::TransportControl as Control;
    if area.is_empty() {
        return;
    }
    let colors = &theme().colors;
    let outer = area;
    let border = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::TOP)
        .border_style(Style::default().fg(colors.border))
        .style(Style::default().bg(colors.transport_bg));
    let border = if state.playback_mode == crate::app::state::PlaybackMode::Radio
        && !state.tui_queue_visible
    {
        border.title(Line::from(crate::app::queue_modes::label(state)).alignment(Alignment::Right))
    } else {
        border
    };
    let area = border.inner(outer);
    frame.render_widget(border, outer);
    if state.tui_sidebar_visible
        && (state.view == View::Browse
            || (state.view == View::Queue
                && state.tui_queue_workspace == crate::app::state::TuiWorkspace::Library))
    {
        let sidebar = state.sidebar_width().min(outer.width / 3);
        if sidebar > 0 {
            for (x, symbol) in [(outer.x, "└"), (outer.x + sidebar - 1, "┴")] {
                frame.render_widget(
                    Paragraph::new(symbol).style(Style::default().fg(colors.border)),
                    Rect::new(x, outer.y, 1, 1),
                );
            }
        }
    }
    if area.is_empty() {
        return;
    }

    frame.render_widget(
        ratatui::widgets::Block::default().style(Style::default().bg(colors.transport_bg)),
        area,
    );
    let (_, info_width) = crate::ui::layout::transport_widths(
        f32::from(state.terminal_width) * state.cell_width(),
        f32::from(state.queue_width()) * state.cell_width(),
    );
    let center_width = ((info_width / state.cell_width()).round() as u16)
        .min(
            area.width
                .saturating_sub(if area.width >= 70 { 36 } else { 24 }),
        )
        .max(1)
        .min(area.width);
    let center = Rect::new(
        area.x + (area.width - center_width) / 2,
        area.y,
        center_width,
        area.height,
    );
    frame.render_widget(
        ratatui::widgets::Block::default()
            .style(Style::default().bg(colors.track_card_background())),
        center,
    );
    let row = area.y + u16::from(area.height > 1);
    let button_style = |control| {
        if state.tui_transport_focus == Some(control) {
            colors.focused_row()
        } else {
            Style::default()
                .fg(colors.fg_primary)
                .bg(colors.transport_bg)
        }
    };
    let button_width = (36. / state.cell_width()).round().max(3.) as u16;
    let control = |index: u16| {
        let x = if f32::from(center.x.saturating_sub(area.x)) * state.cell_width() >= 132. {
            ((16. + f32::from(index) * 48.) / state.cell_width()).round() as u16
        } else {
            index * 3
        };
        Rect::new(area.x + x, row, button_width, 1).intersection(area)
    };
    let previous = control(0);
    let play = control(1);
    let next = control(2);
    for (rect, label, control) in [
        (previous, "⏮︎", Control::Previous),
        (
            play,
            if state.playback.status == PlayStatus::Playing {
                "Ⅱ"
            } else {
                "▶"
            },
            Control::Play,
        ),
        (next, "⏭︎", Control::Next),
    ] {
        frame.render_widget(
            Paragraph::new(label.trim())
                .alignment(Alignment::Center)
                .style(button_style(control)),
            rect,
        );
    }
    let metadata_y = area.y + u16::from(area.height > 2);
    let has_art = center.width >= 24;
    let art_width = (32. / state.cell_width()).round().max(1.) as u16;
    let art_gap = (12. / state.cell_width()).round().max(1.) as u16;
    let text_limit = center.width.saturating_sub(if has_art {
        2 * (art_width + art_gap + 1)
    } else {
        2
    });
    let text_width = state
        .current_track()
        .map_or_else(
            || "Nothing playing".width(),
            |track| {
                track
                    .title
                    .width()
                    .max(track.track_artist().width() + 3 + track.album_name().width())
            },
        )
        .min(text_limit as usize) as u16;
    let title_area = Rect::new(
        center.x + (center.width - text_width) / 2,
        area.y,
        text_width,
        1,
    );
    let artist_area = Rect::new(
        title_area.x,
        metadata_y,
        text_width,
        u16::from(area.height > 1),
    );
    if has_art {
        let art = Rect::new(
            title_area.x - art_gap - art_width,
            area.y,
            art_width,
            2.min(area.height),
        );
        if let Some(data) = state.current_artwork_bytes() {
            crate::ui::artwork::render_grid_image(frame, art, "transport-artwork", data);
        } else {
            crate::ui::artwork::render_placeholder(
                frame,
                art,
                crate::ui::artwork::Placeholder::Track,
            );
        }
    }
    if let Some(track) = state.current_track() {
        frame.render_widget(
            Paragraph::new(truncate_to_width(&track.title, title_area.width as usize))
                .alignment(Alignment::Center)
                .style(if state.tui_transport_focus == Some(Control::Track) {
                    colors.focused_row().add_modifier(Modifier::BOLD)
                } else {
                    colors.track_card_style().add_modifier(Modifier::BOLD)
                }),
            title_area,
        );
        crate::ui::detail_links::field(
            frame,
            state,
            title_area,
            track,
            crate::app::library_details::Destination::Track,
        );
        let artist = track.track_artist();
        let metadata = truncate_to_width(
            &format!("{artist} · {}", track.album_name()),
            artist_area.width as usize,
        );
        frame.render_widget(
            Paragraph::new(metadata.clone())
                .alignment(Alignment::Center)
                .style(
                    if matches!(
                        state.tui_transport_focus,
                        Some(Control::Artist | Control::Album)
                    ) {
                        colors.focused_row()
                    } else {
                        colors.track_card_secondary_style()
                    },
                ),
            artist_area,
        );
        let metadata_width = UnicodeWidthStr::width(metadata.as_str()) as u16;
        let text_area = Rect::new(
            artist_area.x + artist_area.width.saturating_sub(metadata_width) / 2,
            artist_area.y,
            metadata_width.min(artist_area.width),
            artist_area.height,
        );
        let album_start = UnicodeWidthStr::width(artist) as u16 + 3;
        if text_area.width > album_start {
            crate::ui::detail_links::field(
                frame,
                state,
                Rect::new(
                    text_area.x + album_start,
                    text_area.y,
                    text_area.width - album_start,
                    text_area.height,
                ),
                track,
                crate::app::library_details::Destination::Album,
            );
        }
        crate::ui::artist_links::visible(
            frame,
            state,
            artist_area,
            artist,
            track.grandparent_rating_key.as_deref(),
        );
    } else {
        frame.render_widget(
            Paragraph::new("Nothing playing")
                .alignment(Alignment::Center)
                .style(colors.track_card_secondary_style()),
            title_area,
        );
    }
    let seekbar = Rect::new(
        center.x + 1,
        area.bottom().saturating_sub(2),
        center.width.saturating_sub(2),
        u16::from(area.height > 1),
    );
    let progress = if state.playback.duration_ms > 0 {
        state.playback.position_ms as f32 / state.playback.duration_ms as f32
    } else {
        0.
    };
    if seekbar.height > 0 {
        frame.render_widget(
            Paragraph::new(build_progress_bar(progress, seekbar.width as usize)).style(
                if state.tui_transport_focus == Some(Control::Seek) {
                    colors.focused_row()
                } else {
                    colors.track_card_style()
                },
            ),
            seekbar,
        );
    }
    let (elapsed, remaining) = crate::media::time_axis::transport_times(
        state.playback.position_ms,
        state.playback.duration_ms,
    );
    if area.height >= 4 {
        let time_area = Rect::new(
            center.x + 1,
            area.bottom() - 1,
            center.width.saturating_sub(2),
            1,
        );
        let style = colors.track_card_secondary_style();
        frame.render_widget(Paragraph::new(elapsed).style(style), time_area);
        frame.render_widget(
            Paragraph::new(remaining)
                .alignment(Alignment::Right)
                .style(style),
            time_area,
        );
    }
    let right = Rect::new(
        center.right() + 1,
        area.y,
        area.right().saturating_sub(center.right() + 1),
        area.height,
    );
    let mut controls = vec![
        (previous, Control::Previous),
        (play, Control::Play),
        (next, Control::Next),
        (title_area, Control::Track),
    ];
    for (area, control) in state
        .hit_regions
        .borrow()
        .artist_links
        .iter()
        .filter(|link| artist_area.intersection(link.area) == link.area)
        .map(|link| (link.area, Control::Artist))
        .chain(
            state
                .hit_regions
                .borrow()
                .detail_links
                .iter()
                .filter(|link| {
                    link.destination == crate::app::library_details::Destination::Album
                        && artist_area.intersection(link.area) == link.area
                })
                .map(|link| (link.area, Control::Album)),
        )
    {
        if !controls.iter().any(|(_, visible)| *visible == control) {
            controls.push((area, control));
        }
    }

    controls.push((seekbar, Control::Seek));
    let mut speaker = None;
    let mut volume = None;
    if right.width >= 7 {
        let menu = Rect::new(right.x, row, 2, 1);
        let lyrics = Rect::new(right.x + 3, row, 2, 1);
        let mute = Rect::new(right.x + 6, row, 2.min(right.width - 6), 1);
        for (rect, label, control) in [
            (menu, "… ", Control::Menu),
            (lyrics, "¶ ", Control::Lyrics),
            (
                mute,
                if state.playback.muted { "× " } else { "◖ " },
                Control::Mute,
            ),
        ] {
            frame.render_widget(Paragraph::new(label).style(button_style(control)), rect);
            controls.push((rect, control));
        }
        speaker = Some(mute);
        if right.width >= 12 {
            let slider = Rect::new(right.x + 9, row, right.width.saturating_sub(9), 1);
            frame.render_widget(
                Paragraph::new(build_progress_bar(
                    if state.playback.muted {
                        0.
                    } else {
                        state.playback.volume
                    },
                    slider.width as usize,
                ))
                .style(button_style(Control::Volume)),
                slider,
            );
            controls.push((slider, Control::Volume));
            volume = Some(slider);
        }
    }
    let player =
        Rect::new(area.right().saturating_sub(3), area.y, 2.min(area.width), 1).intersection(area);
    let workspace = if matches!(state.view, View::Settings | View::Similar | View::Related) {
        state.previous_view.unwrap_or(state.view)
    } else {
        state.view
    };
    let active = workspace == View::NowPlaying
        || (workspace == View::Queue
            && state.tui_queue_workspace == crate::app::state::TuiWorkspace::NowPlaying);
    if !active {
        frame.render_widget(
            Paragraph::new("⌃ ").style(button_style(Control::Player)),
            player,
        );
        controls.push((player, Control::Player));
    }
    controls.retain(|(rect, _)| !rect.is_empty());
    state.hit_regions.borrow_mut().transport = Some(crate::app::presentation::TransportRegions {
        area: outer,
        controls,
        play_pause: play,
        seekbar,
        prev_track: previous,
        next_track: next,
        track_info: Some(title_area),
        search_icon: None,
        speaker_icon: speaker,
        volume_slider: volume,
    });
}

/// Render the fixed application footer independently of the transport.
pub fn render_footer(frame: &mut Frame, state: &AppState, area: Rect) {
    if area.is_empty() {
        return;
    }
    let colors = &theme().colors;
    let border = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::TOP)
        .border_style(Style::default().fg(colors.border))
        .style(Style::default().bg(colors.transport_bg));
    let inner = border.inner(area);
    frame.render_widget(border, area);
    if let Some(queue) = &state.hit_regions.borrow().queue_content {
        for (x, symbol) in [
            (queue.track_list.x, "┴"),
            (queue.track_list.right().saturating_sub(1), "┘"),
        ] {
            if x >= area.x && x < area.right() {
                frame.render_widget(
                    Paragraph::new(symbol).style(Style::default().fg(colors.border)),
                    Rect::new(x, area.y, 1, 1),
                );
            }
        }
    }
    let area = inner;
    if area.is_empty() {
        return;
    }
    let mut x = area.x + u16::from(area.width > 2);
    for (index, (key, icon, _)) in crate::app::tui_navigation::FOOTER.iter().enumerate() {
        let rect = if *key == crossterm::event::KeyCode::Char(']') {
            Rect::new(area.right().saturating_sub(4), area.y, 3.min(area.width), 1)
        } else {
            let rect = Rect::new(x, area.y, 3, 1).intersection(area);
            x = x.saturating_add(4);
            rect
        };
        frame.render_widget(
            Paragraph::new(*icon).style(if state.tui_footer_focus == Some(index) {
                colors.focused_row()
            } else {
                Style::default().fg(colors.fg_muted).bg(colors.transport_bg)
            }),
            rect,
        );
        state
            .hit_regions
            .borrow_mut()
            .footer_controls
            .push((rect, index));
    }
    let message = crate::app::tui_navigation::status(state)
        .or_else(|| {
            state
                .notifications
                .toast_message
                .as_deref()
                .map(str::to_owned)
                .or_else(|| {
                    state
                        .current_notification()
                        .map(|notification| notification.message.clone())
                })
        })
        .unwrap_or_default();
    let download_link = crate::app::tui_navigation::status(state).is_none()
        && state.notifications.toast_message.is_none()
        && state
            .current_notification()
            .is_some_and(|n| n.target.is_some());
    let count = state.browse_item_summary().unwrap_or_default();
    let count_width = UnicodeWidthStr::width(count.as_str()) as u16;
    let right = area.right().saturating_sub(7);
    let count_width = if right.saturating_sub(x) >= count_width + 3 {
        count_width
    } else {
        0
    };
    let message_right = right.saturating_sub(count_width + u16::from(count_width > 0) * 2);
    let message = truncate_to_width(&message, message_right.saturating_sub(x + 1) as usize);
    let width = UnicodeWidthStr::width(message.as_str()) as u16;
    let style = Style::default().fg(colors.fg_muted).bg(colors.transport_bg);
    frame.render_widget(
        Paragraph::new(message).style(style),
        Rect::new(message_right.saturating_sub(width), area.y, width, 1).intersection(area),
    );
    if download_link && width > 0 {
        state.hit_regions.borrow_mut().download_status = Some(
            Rect::new(message_right.saturating_sub(width), area.y, width, 1).intersection(area),
        );
    }
    if count_width > 0 {
        frame.render_widget(
            Paragraph::new(count).style(style),
            Rect::new(right - count_width, area.y, count_width, 1),
        );
    }
}

/// Build a progress bar with filled/empty segments and position indicator.
fn build_progress_bar(progress: f32, width: usize) -> String {
    let filled = (progress.clamp(0.0, 1.0) * width.saturating_sub(1) as f32).round() as usize;
    let mut bar = String::with_capacity(width);

    for i in 0..width {
        if i == filled && filled < width {
            bar.push('●'); // Position indicator
        } else if i < filled {
            bar.push('━'); // Filled (thick)
        } else {
            bar.push('─'); // Empty (thin)
        }
    }

    bar
}

/// Truncate a string to fit within `max_width` display columns, adding '…' if truncated.
fn truncate_to_width(s: &str, max_width: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    let total_width = UnicodeWidthStr::width(s);
    if total_width <= max_width {
        return s.to_string();
    }
    let mut truncated = String::new();
    let mut width = 0;
    for ch in s.chars() {
        let ch_w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_w >= max_width {
            truncated.push('…');
            break;
        }
        truncated.push(ch);
        width += ch_w;
    }
    truncated
}
