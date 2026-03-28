//! Now playing bar widget.

use crate::app::{AppState, PlayStatus};
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Gauge, Paragraph};

/// Render the now playing bar at the bottom of the screen.
pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(t.colors.border))
        .style(Style::default().bg(t.colors.transport_bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(track) = state.current_track() {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(3),   // Play status
                Constraint::Length(35),  // Track info
                Constraint::Min(20),     // Progress bar
                Constraint::Length(12),  // Time
                Constraint::Length(10),  // Volume
            ])
            .split(inner);

        // Play/pause indicator
        let status_symbol = match state.playback.status {
            PlayStatus::Playing => "▶",
            PlayStatus::Paused => "⏸",
            PlayStatus::Buffering => "⏳",
            PlayStatus::Stopped => "⏹",
        };
        let status = Paragraph::new(status_symbol)
            .style(Style::default().fg(t.colors.fg_accent));
        frame.render_widget(status, chunks[0]);

        // Track info
        let info = format!(
            "{} - {}",
            track.title,
            track.track_artist()
        );
        let info_text = Paragraph::new(info)
            .style(Style::default().fg(t.colors.fg_primary));
        frame.render_widget(info_text, chunks[1]);

        // Progress bar
        let progress = if state.playback.duration_ms > 0 {
            ((state.playback.position_ms as f64 / state.playback.duration_ms as f64) * 100.0) as u16
        } else {
            0
        };

        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(t.colors.fg_accent).bg(t.colors.border))
            .percent(progress)
            .label("");
        frame.render_widget(gauge, chunks[2]);

        // Time display
        let time = format!(
            "{} / {}",
            format_duration(state.playback.position_ms),
            format_duration(state.playback.duration_ms)
        );
        let time_text = Paragraph::new(time)
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(time_text, chunks[3]);

        // Volume (or no-audio indicator)
        let vol_text = if !state.audio_available {
            "🔇 no audio".to_string()
        } else if state.playback.muted {
            "🔇".to_string()
        } else {
            format!("🔊 {}%", (state.playback.volume * 100.0) as u8)
        };
        let volume = Paragraph::new(vol_text)
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Right);
        frame.render_widget(volume, chunks[4]);
    } else {
        // No track playing - show debug info
        let screen_name = match &state.current_screen {
            crate::app::Screen::Auth => "Auth",
            crate::app::Screen::Home => "Home",
            crate::app::Screen::Library { section } => match section {
                crate::app::LibrarySection::Artists => "Artists",
                crate::app::LibrarySection::Albums => "Albums",
                crate::app::LibrarySection::Tracks => "Tracks",
                crate::app::LibrarySection::Playlists => "Playlists",
                _ => "Library",
            },
            crate::app::Screen::Search => "Search",
            crate::app::Screen::NowPlaying => "Now Playing",
            crate::app::Screen::Help => "Help",
            _ => "Other",
        };
        let key_info = state.last_key.as_deref().unwrap_or("none");
        let text = format!("No track playing  |  Screen: {}  |  Last key: {}  |  Press ? for help", screen_name, key_info);
        let text = Paragraph::new(text)
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(text, inner);
    }
}

/// Format milliseconds as MM:SS.
fn format_duration(ms: u64) -> String {
    let secs = ms / 1000;
    let mins = secs / 60;
    let secs = secs % 60;
    format!("{:02}:{:02}", mins, secs)
}
