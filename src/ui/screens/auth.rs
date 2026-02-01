//! Authentication screen.

use crate::app::{AppState, ConnectionState};
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Fill background
    frame.render_widget(
        Block::default().style(Style::default().bg(t.colors.bg_primary)),
        area,
    );

    let block = Block::default()
        .title(" Authentication ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let (text, style) = match &state.connection {
        ConnectionState::Disconnected => {
            ("Not connected to Plex.\nChecking configuration...".to_string(),
             Style::default().fg(t.colors.fg_muted))
        }
        ConnectionState::Authenticating => {
            ("Authenticating with Plex...".to_string(),
             Style::default().fg(t.colors.fg_accent))
        }
        ConnectionState::AuthPending { pin_code, .. } => {
            (format!(
                "Please visit:\n\nhttps://plex.tv/link\n\nAnd enter code: {}\n\nWaiting for authorization...",
                pin_code
            ), Style::default().fg(t.colors.fg_accent))
        }
        ConnectionState::Connecting => {
            ("Connecting to Plex server...".to_string(),
             Style::default().fg(t.colors.fg_accent))
        }
        ConnectionState::Connected { username } => {
            (format!("Connected as: {}", username),
             Style::default().fg(t.colors.success))
        }
        ConnectionState::Error(err) => {
            (format!("Authentication failed:\n\n{}\n\nCheck your config.yaml", err),
             Style::default().fg(t.colors.error))
        }
    };

    let content = Paragraph::new(text)
        .style(style)
        .wrap(Wrap { trim: true })
        .alignment(Alignment::Center);

    // Center vertically
    let centered_area = Rect {
        x: inner.x,
        y: inner.y + inner.height / 3,
        width: inner.width,
        height: inner.height / 3,
    };

    frame.render_widget(content, centered_area);
}
