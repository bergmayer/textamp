//! Transport bar widget (musikcube-style).
//!
//! Shows: playing [title] by [artist] from [album] | vol --■-- 80% | 2:30 --■-- 4:32 | shuffle | repeat

use crate::app::state::PlayStatus;
use crate::app::AppState;
use crate::ui::theme::theme;
use crate::util::truncate_str;

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Render the transport bar.
pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();
    let bg_style = Style::default().bg(t.colors.transport_bg);

    // Fill background
    let bg = Paragraph::new("").style(bg_style);
    frame.render_widget(bg, area);

    // Build transport text
    let transport_text = build_transport_line(state);

    let paragraph = Paragraph::new(transport_text)
        .style(Style::default().fg(t.colors.fg_primary).bg(t.colors.transport_bg));

    frame.render_widget(paragraph, area);
}

fn build_transport_line(state: &AppState) -> String {
    let mut line = String::new();

    // Show adventure mode status
    if state.adventure.active {
        if state.adventure.generating {
            line.push_str("🌟 ADVENTURE: generating sonic bridge...");
            return line;
        } else if state.adventure.start_track.is_some() && state.adventure.end_track.is_some() {
            // Both tracks selected - waiting for length input
            let start = state.adventure.start_track.as_ref()
                .map(|t| truncate_str(&t.title, 15))
                .unwrap_or_default();
            let end = state.adventure.end_track.as_ref()
                .map(|t| truncate_str(&t.title, 15))
                .unwrap_or_default();
            line.push_str(&format!("🌟 ADVENTURE: {} → {} (enter length)", start, end));
            return line;
        } else if state.adventure.start_track.is_some() {
            let start_title = state.adventure.start_track.as_ref()
                .map(|t| truncate_str(&t.title, 20))
                .unwrap_or_default();
            line.push_str(&format!("🌟 ADVENTURE: {} → select END (Alt+V)", start_title));
            return line;
        } else {
            line.push_str("🌟 ADVENTURE: select START track (Alt+V)");
            return line;
        }
    }

    // Play/pause button at the start
    let status_icon = match state.playback.status {
        PlayStatus::Playing => "⏸",
        PlayStatus::Paused => "▶",
        PlayStatus::Stopped => "▶",
        PlayStatus::Buffering => "◌",
    };
    line.push_str(status_icon);
    line.push(' ');

    // Time display with progress bar
    let pos_str = format_time(state.playback.position_ms);
    let dur_str = format_time(state.playback.duration_ms);
    let progress = if state.playback.duration_ms > 0 {
        state.playback.position_ms as f32 / state.playback.duration_ms as f32
    } else {
        0.0
    };
    let time_bar = build_progress_bar(progress, 20);

    line.push_str(&format!("{} {} {}", pos_str, time_bar, dur_str));

    line.push_str("  │  ");

    // Track info
    if let Some(track) = state.current_track() {
        line.push_str(&track.title);
        line.push_str(" by ");
        line.push_str(&track.artist_name());
        line.push_str(" from ");
        line.push_str(&track.album_name());
    } else {
        line.push_str("No track playing");
    }

    line.push_str("  │  ");

    // Volume indicator (more compact)
    let vol_pct = (state.playback.volume * 100.0) as u8;
    if state.playback.muted {
        line.push_str("🔇");
    } else {
        line.push_str(&format!("🔊{}%", vol_pct));
    }

    // Shuffle indicator
    if state.playback.shuffle {
        line.push_str(" 🔀");
    }

    // Repeat indicator
    match state.playback.repeat_mode {
        crate::app::state::RepeatMode::All => line.push_str(" 🔁"),
        crate::app::state::RepeatMode::One => line.push_str(" 🔂"),
        _ => {}
    }

    // Status message at end (doesn't replace playback info)
    if let Some(ref msg) = state.status_message {
        line.push_str("  │  ");
        line.push_str(msg);
    }

    line
}

/// Build a progress bar with filled/empty segments and position indicator.
fn build_progress_bar(progress: f32, width: usize) -> String {
    let filled = (progress * width as f32).round() as usize;
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

/// Format milliseconds as MM:SS.
fn format_time(ms: u64) -> String {
    let total_secs = ms / 1000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{:02}:{:02}", mins, secs)
}
