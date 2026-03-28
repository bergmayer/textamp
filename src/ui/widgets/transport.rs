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
use unicode_width::UnicodeWidthStr;

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
    let right_content = build_right_content(state);
    let right_text = &right_content.text;

    // Calculate widths using display columns (emoji are 2 cells wide)
    let right_width = UnicodeWidthStr::width(right_text.as_str()) as u16;
    let available_width = area.width.saturating_sub(2); // Leave some padding

    // If the right side would overlap, truncate left content
    let left_max_width = available_width.saturating_sub(right_width + 1) as usize;
    let left_display = truncate_to_width(&left_text, left_max_width);

    // Create the full line with proper spacing
    let left_width = UnicodeWidthStr::width(left_display.as_str());
    let padding = available_width.saturating_sub(left_width as u16 + right_width);
    let full_line = format!("{}{:>pad$}{}", left_display, "", right_text, pad = padding as usize);

    // Register transport hit regions
    // Left content layout: " ⏸ 00:00 ━━●──────────── 04:32 ⏮  ⏭  │  Track info..."
    //                        ^3  ^5  1  ^20            1 ^5  1^1 2^1  ^5
    {
        let right_start_x = area.x + left_width as u16 + padding;
        // Find speaker icon position within right_text using display width
        let speaker_offset = right_text.find('🔊').or_else(|| right_text.find('🔇'))
            .map(|byte_pos| UnicodeWidthStr::width(&right_text[..byte_pos]) as u16)
            .unwrap_or(0);
        let speaker_x = right_start_x + speaker_offset;
        let search_x = right_start_x;

        // Volume slider hit region (inline, if present)
        let volume_slider = right_content.slider_bar.map(|(offset, width)| {
            Rect { x: right_start_x + offset as u16, y: area.y, width: width as u16, height: 1 }
        });

        // Compute left-side positions from known structure widths
        let status_w: u16 = 3;    // " ⏸ "
        let pos_str_w: u16 = 5;   // "00:00"
        let bar_w: u16 = 20;      // progress bar
        let dur_str_w: u16 = 5;   // "04:32"
        let seekbar_x = area.x + status_w + pos_str_w + 1; // +1 for space
        let prev_x = seekbar_x + bar_w + 1 + dur_str_w + 1; // bar + space + dur + space
        let next_x = prev_x + 1 + 2; // ⏮ + two spaces
        let separator_w: u16 = 5; // "  │  "
        let track_info_x = next_x + 1 + separator_w; // ⏭ + separator

        let mut hr = state.hit_regions.borrow_mut();
        hr.transport = Some(crate::ui::hit_regions::TransportRegions {
            play_pause: Rect { x: area.x, y: area.y, width: status_w + 2, height: 1 },
            seekbar: Rect { x: seekbar_x, y: area.y, width: bar_w, height: 1 },
            prev_track: Rect { x: prev_x.saturating_sub(1), y: area.y, width: 3, height: 1 },
            next_track: Rect { x: next_x.saturating_sub(1), y: area.y, width: 3, height: 1 },
            track_info: Some(Rect { x: track_info_x, y: area.y, width: search_x.saturating_sub(track_info_x), height: 1 }),
            search_icon: Some(Rect { x: search_x, y: area.y, width: speaker_offset, height: 1 }),
            speaker_icon: Some(Rect { x: speaker_x, y: area.y, width: right_width.saturating_sub(speaker_offset), height: 1 }),
            volume_slider,
        });
    }

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

    // Play/pause button at the start (padded for click target)
    let status_icon = match state.playback.status {
        PlayStatus::Playing => " ⏸ ",
        PlayStatus::Paused => " ▶ ",
        PlayStatus::Stopped => " ▶ ",
        PlayStatus::Buffering => " ◌ ",
    };
    line.push_str(status_icon);

    // Time display with progress bar
    let pos_str = format_time(state.playback.position_ms);
    let dur_str = format_time(state.playback.duration_ms);
    let progress = if state.playback.duration_ms > 0 {
        state.playback.position_ms as f32 / state.playback.duration_ms as f32
    } else {
        0.0
    };
    let time_bar = build_progress_bar(progress, 20);

    line.push_str(&format!("{} {} {} ⏮  ⏭", pos_str, time_bar, dur_str));

    line.push_str("  │  ");

    // Track info
    if let Some(track) = state.current_track() {
        let title = if track.title.is_empty() {
            track.file_name().unwrap_or("Unknown Track")
        } else {
            &track.title
        };
        line.push_str(title);
        line.push_str(" by ");
        line.push_str(track.track_artist());
        line.push_str(" from ");
        line.push_str(track.album_name());
    } else {
        line.push_str("No track playing");
    }

    line
}

/// Right-side content with optional inline slider metadata.
struct RightContent {
    text: String,
    /// Display-column offset and width of the volume slider bar within `text` (if shown).
    slider_bar: Option<(usize, usize)>,
}

/// Build the right side of the transport bar (search + notification/volume + indicators).
fn build_right_content(state: &AppState) -> RightContent {
    let mut right = String::new();
    let mut slider_bar = None;

    // Remote output indicator
    if let crate::app::state::OutputTarget::Remote { ref player_name, .. } = state.remote.output_target {
        right.push_str(&format!("-> {} ", truncate_str(player_name, 15)));
    }

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
        // Inline volume slider when active
        let slider_active = state.volume_slider_until
            .map_or(false, |deadline| deadline > std::time::Instant::now());
        if slider_active {
            let vol = if state.playback.muted { 0.0 } else { state.playback.volume };
            let bar_width = 20usize;
            let bar_offset = UnicodeWidthStr::width(right.as_str());
            let bar = build_progress_bar(vol, bar_width);
            right.push_str(&bar);
            right.push(' ');
            slider_bar = Some((bar_offset, bar_width));
        }

        // Speaker icon + percentage (or no-audio indicator)
        if !state.audio_available {
            right.push_str("🔇 no audio");
        } else if state.playback.muted {
            right.push_str("🔇 muted");
        } else {
            let vol_pct = (state.playback.volume * 100.0) as u8;
            right.push_str(&format!("🔊 {}%", vol_pct));
        }
    }

    RightContent { text: right, slider_bar }
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

    // Build filter box with blinking cursor - minimum 20 chars wide for comfortable typing
    let min_filter_width: usize = 24;
    let cursor_visible = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| (d.as_millis() / 500) % 2 == 0)
        .unwrap_or(true);
    let query_display = if state.list_filter.loading {
        format!("{}...", state.list_filter.query)
    } else if cursor_visible {
        format!("{}▋", state.list_filter.query)
    } else {
        format!("{} ", state.list_filter.query)
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
    let query_width = UnicodeWidthStr::width(query_display.as_str());
    let query_padded = if query_width < min_filter_width {
        let padding = min_filter_width - query_width;
        format!("{}{}", query_display, " ".repeat(padding))
    } else {
        query_display
    };

    let filter_text = format!("🔍 [{}]{}", query_padded, match_suffix);

    // Calculate widths using display columns
    let filter_width = UnicodeWidthStr::width(filter_text.as_str()) as u16;
    let available_width = area.width.saturating_sub(2);

    // Truncate left content if needed to fit filter box
    let left_max_width = available_width.saturating_sub(filter_width + 1) as usize;
    let left_display = truncate_to_width(&left_text, left_max_width);

    // Create the full line with proper spacing
    let left_width = UnicodeWidthStr::width(left_display.as_str()) as u16;
    let padding = available_width.saturating_sub(left_width + filter_width);
    let full_line = format!("{}{:>pad$}{}", left_display, "", filter_text, pad = padding as usize);

    // Register transport hit regions for playback controls (still visible in filter mode)
    {
        let status_w: u16 = 3;
        let pos_str_w: u16 = 5;
        let bar_w: u16 = 20;
        let dur_str_w: u16 = 5;
        let seekbar_x = area.x + status_w + pos_str_w + 1;
        let prev_x = seekbar_x + bar_w + 1 + dur_str_w + 1;
        let next_x = prev_x + 1 + 2;

        let mut hr = state.hit_regions.borrow_mut();
        hr.transport = Some(crate::ui::hit_regions::TransportRegions {
            play_pause: Rect { x: area.x, y: area.y, width: status_w + 2, height: 1 },
            seekbar: Rect { x: seekbar_x, y: area.y, width: bar_w, height: 1 },
            prev_track: Rect { x: prev_x.saturating_sub(1), y: area.y, width: 3, height: 1 },
            next_track: Rect { x: next_x.saturating_sub(1), y: area.y, width: 3, height: 1 },
            track_info: None,
            search_icon: None,
            speaker_icon: None,
            volume_slider: None,
        });
    }

    let paragraph = Paragraph::new(full_line)
        .style(Style::default().fg(t.colors.fg_primary).bg(t.colors.transport_bg));

    frame.render_widget(paragraph, area);
}
