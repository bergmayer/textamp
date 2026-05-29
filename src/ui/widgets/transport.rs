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

    // Fill background across both rows.
    frame.render_widget(Paragraph::new("").style(bg_style), area);

    // Top row: playback controls + track info + volume.
    // Bottom row: Library / Now Playing tab strip + ":" hint.
    let top = Rect { x: area.x, y: area.y, width: area.width, height: 1 };
    let bottom = Rect { x: area.x, y: area.y + 1, width: area.width, height: area.height.saturating_sub(1) };

    // Special case: Inline list filter mode shows filter box across the
    // top row; the tab strip still renders on the bottom row.
    if state.list_filter.active {
        render_with_filter(frame, state, top);
        render_tab_strip(frame, state, bottom);
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

    // The transport content renders only on the top row; the bottom
    // row is the tab strip (rendered separately at the end).
    let area = top;

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
        // The right side is now just the `/ filter` affordance — the
        // volume widget and its slider hit region are gone, so the
        // search-icon area covers the entire right_text width.
        let search_x = right_start_x;
        let search_w = right_width;

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
            search_icon: Some(Rect { x: search_x, y: area.y, width: search_w, height: 1 }),
            speaker_icon: None,
            volume_slider: None,
        });
    }

    let paragraph = Paragraph::new(full_line)
        .style(Style::default().fg(t.colors.fg_primary).bg(t.colors.transport_bg));

    frame.render_widget(paragraph, area);

    render_tab_strip(frame, state, bottom);
}

/// Bottom-row tab strip — Library / Now Playing — mirroring the
/// GUI's transport-bar tab strip. Active tab uses the selection
/// background. Hit regions are registered so a future click handler
/// can dispatch SetView; the `:` palette hint sits on the right.
fn render_tab_strip(frame: &mut Frame, state: &AppState, area: Rect) {
    use crate::app::state::View;
    let t = theme();

    if area.height == 0 {
        return;
    }

    // Library = any Browse view; Now Playing = Queue / NowPlaying.
    let library_active = state.view == View::Browse;
    let now_active = matches!(state.view, View::Queue | View::NowPlaying);

    // Lower-case labels matching the visualizer tab strip
    // (`waveform` / `spectrum` / `spectrogram`). Active tab uses
    // accent fg + bold; inactive is muted. A `│` divider sits
    // between the two so the strip visually mirrors the
    // visualizer's tab style.
    let lib_label = " library ";
    let divider = "│";
    let np_label = " now playing ";
    // Symbol cheat sheet: `:` = command palette, `/` = inline filter
    // overlay, `?` = search popup, `⇥` = Tab toggles Library ↔ Now
    // Playing, `,` = open Settings (mirrors macOS Cmd+,), `\` =
    // toggle scrolling Miller layout, `|` = toggle tall split.
    // Mirrors classic vim chrome — same row as the Library / Now
    // Playing tabs, on the right edge.
    let palette_hint = " :  /  ?  \u{21e5}  ,  \\  | ";

    // Active tab uses an inverted (selection-style) background so the
    // selected/unselected distinction is obvious in every theme — in
    // B&W in particular, the previous "bold accent on transport bg"
    // versus "muted on transport bg" lookup was just bold-vs-non-bold
    // black-on-white, which the user couldn't read at a glance.
    let active_style = Style::default()
        .fg(t.colors.selection_bar_fg)
        .bg(t.colors.selection_bar_bg)
        .add_modifier(Modifier::BOLD);
    let inactive_style = Style::default()
        .fg(t.colors.fg_muted)
        .bg(t.colors.transport_bg);
    let divider_style = Style::default()
        .fg(t.colors.fg_muted)
        .bg(t.colors.transport_bg);

    let lib_w = UnicodeWidthStr::width(lib_label) as u16;
    let div_w = UnicodeWidthStr::width(divider) as u16;
    let np_w = UnicodeWidthStr::width(np_label) as u16;
    let hint_w = UnicodeWidthStr::width(palette_hint) as u16;

    let lib_x = area.x;
    let div_x = lib_x + lib_w;
    let np_x = div_x + div_w;
    let hint_x = area.x + area.width.saturating_sub(hint_w);

    frame.render_widget(
        Paragraph::new(lib_label).style(if library_active { active_style } else { inactive_style }),
        Rect { x: lib_x, y: area.y, width: lib_w, height: 1 },
    );
    frame.render_widget(
        Paragraph::new(divider).style(divider_style),
        Rect { x: div_x, y: area.y, width: div_w, height: 1 },
    );
    frame.render_widget(
        Paragraph::new(np_label).style(if now_active { active_style } else { inactive_style }),
        Rect { x: np_x, y: area.y, width: np_w, height: 1 },
    );
    frame.render_widget(
        Paragraph::new(palette_hint).style(Style::default().fg(t.colors.fg_muted).bg(t.colors.transport_bg)),
        Rect { x: hint_x, y: area.y, width: hint_w, height: 1 },
    );

    // Register click hit regions. The shared tab_bar_action handler
    // expects index 0 = Library (Browse) and index 2 = Now Playing.
    let lib_rect = Rect { x: lib_x, y: area.y, width: lib_w, height: 1 };
    let np_rect = Rect { x: np_x, y: area.y, width: np_w, height: 1 };
    let mut hr = state.hit_regions.borrow_mut();
    hr.tab_bar = Some(crate::ui::hit_regions::TabBarRegions {
        library_label: None,
        quit_button: None,
        tabs: vec![(lib_rect, 0), (np_rect, 2)],
    });
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

struct RightContent {
    text: String,
}

/// Build the right side of the transport bar: any active notification
/// plus, when a remote player is selected, a `-> Name` indicator.
fn build_right_content(state: &AppState) -> RightContent {
    let mut right = String::new();

    if let crate::app::state::OutputTarget::Remote { ref player_name, .. } = state.remote.output_target {
        right.push_str(&format!("-> {} ", truncate_str(player_name, 15)));
    }

    if let Some(notification) = state.current_notification() {
        let icon = match notification.notification_type {
            NotificationType::Ongoing => "⟳",
            NotificationType::Toast => "ℹ",
        };
        right.push_str(&format!("{} {} ", icon, notification.message));
    }

    RightContent { text: right }
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
