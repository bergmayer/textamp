//! Transport bar widget (musikcube-style).
//!
//! Layout:
//! Left:  ▶ 00:00 ━━●──────────── 04:32  │  Track by Artist from Album
//! Right: [notification OR volume]
//!
//! Notifications temporarily cover the volume widget when active.

use crate::app::state::{NotificationType, PlayStatus};
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

    // Special case: Inline list filter mode shows filter box
    if state.list_filter.active {
        render_with_filter(frame, state, area);
        return;
    }

    // Special case: Adventure mode messages take over the entire bar
    if state.adventure.active && !state.adventure.generating {
        let adventure_text = build_adventure_text(state);
        let paragraph = Paragraph::new(adventure_text)
            .style(Style::default().fg(t.colors.fg_accent).bg(t.colors.transport_bg));
        frame.render_widget(paragraph, area);
        return;
    }

    // Build left side (playback info)
    let left_text = build_left_content(state);

    // Build right side (notification or volume + indicators)
    let right_text = build_right_content(state);

    // Calculate widths - right side is fixed, left side fills remaining space
    let right_width = right_text.chars().count() as u16;
    let available_width = area.width.saturating_sub(2); // Leave some padding

    // If the right side would overlap, truncate left content
    let left_max_width = available_width.saturating_sub(right_width + 1) as usize;
    let left_display = if left_text.chars().count() > left_max_width {
        let mut truncated = String::new();
        for (i, c) in left_text.chars().enumerate() {
            if i >= left_max_width.saturating_sub(1) {
                truncated.push('…');
                break;
            }
            truncated.push(c);
        }
        truncated
    } else {
        left_text.clone()
    };

    // Create the full line with proper spacing
    let left_width = left_display.chars().count();
    let padding = available_width.saturating_sub(left_width as u16 + right_width);
    let full_line = format!("{}{:>pad$}{}", left_display, "", right_text, pad = padding as usize);

    let paragraph = Paragraph::new(full_line)
        .style(Style::default().fg(t.colors.fg_primary).bg(t.colors.transport_bg));

    frame.render_widget(paragraph, area);
}

/// Build adventure mode text for the transport bar.
fn build_adventure_text(state: &AppState) -> String {
    if state.adventure.start_track.is_some() && state.adventure.end_track.is_some() {
        // Both tracks selected - waiting for length input
        let start = state.adventure.start_track.as_ref()
            .map(|t| truncate_str(&t.title, 15))
            .unwrap_or_default();
        let end = state.adventure.end_track.as_ref()
            .map(|t| truncate_str(&t.title, 15))
            .unwrap_or_default();
        format!("🌟 ADVENTURE: {} → {} (enter length)", start, end)
    } else if state.adventure.start_track.is_some() {
        let start_title = state.adventure.start_track.as_ref()
            .map(|t| truncate_str(&t.title, 20))
            .unwrap_or_default();
        format!("🌟 ADVENTURE: {} → select END (Alt+A)", start_title)
    } else {
        "🌟 ADVENTURE: select START track (Alt+A)".to_string()
    }
}

/// Build the left side of the transport bar (playback controls and track info).
fn build_left_content(state: &AppState) -> String {
    let mut line = String::new();

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

    line
}

/// Build the right side of the transport bar (search + notification/volume + indicators).
fn build_right_content(state: &AppState) -> String {
    let mut right = String::new();

    // Search/filter emoji first (left of volume, clickable to activate filter)
    right.push_str("🔍 ");

    // Check for active notification
    if let Some(notification) = state.current_notification() {
        // Show notification instead of volume
        let icon = match notification.notification_type {
            NotificationType::Ongoing => "⟳",
            NotificationType::Toast => "ℹ",
        };
        right.push_str(&format!("{} {}", icon, notification.message));
    } else {
        // Show volume when no notification
        let vol_pct = (state.playback.volume * 100.0) as u8;
        if state.playback.muted {
            right.push_str("🔇 muted");
        } else {
            right.push_str(&format!("🔊 {}%", vol_pct));
        }
    }

    right
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

/// Render transport bar with inline filter box active.
fn render_with_filter(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Build left content (full playback info)
    let left_text = build_left_content(state);

    // Build filter box with cursor - minimum 20 chars wide for comfortable typing
    let min_filter_width: usize = 24;
    let query_display = if state.list_filter.loading {
        format!("{}...", state.list_filter.query)
    } else {
        format!("{}▋", state.list_filter.query)
    };

    // Show match count if we have results
    let match_suffix = if let Some(ref results) = state.list_filter.results {
        if results.has_more {
            format!(" ({}/{}+)", results.matched_indices.len(), results.total_matches)
        } else if results.matched_indices.is_empty() {
            " (no matches)".to_string()
        } else {
            format!(" ({})", results.total_matches)
        }
    } else {
        String::new()
    };

    // Pad query to minimum width for consistent text box appearance
    let query_padded = if query_display.chars().count() < min_filter_width {
        let padding = min_filter_width - query_display.chars().count();
        format!("{}{}", query_display, " ".repeat(padding))
    } else {
        query_display
    };

    let filter_text = format!("🔍 [{}]{}", query_padded, match_suffix);

    // Calculate widths
    let filter_width = filter_text.chars().count() as u16;
    let available_width = area.width.saturating_sub(2);

    // Truncate left content if needed to fit filter box
    let left_max_width = available_width.saturating_sub(filter_width + 1) as usize;
    let left_display = if left_text.chars().count() > left_max_width {
        let mut truncated = String::new();
        for (i, c) in left_text.chars().enumerate() {
            if i >= left_max_width.saturating_sub(1) {
                truncated.push('…');
                break;
            }
            truncated.push(c);
        }
        truncated
    } else {
        left_text
    };

    // Create the full line with proper spacing
    let left_width = left_display.chars().count() as u16;
    let padding = available_width.saturating_sub(left_width + filter_width);
    let full_line = format!("{}{:>pad$}{}", left_display, "", filter_text, pad = padding as usize);

    let paragraph = Paragraph::new(full_line)
        .style(Style::default().fg(t.colors.fg_primary).bg(t.colors.transport_bg));

    frame.render_widget(paragraph, area);
}
