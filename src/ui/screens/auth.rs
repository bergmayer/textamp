//! Authentication screen with login form and server selection.

use crate::app::AppState;
use crate::app::state::AuthStep;
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Fill background
    frame.render_widget(
        Block::default().style(Style::default().bg(t.colors.bg_primary)),
        area,
    );

    match state.auth_state.step {
        AuthStep::Checking => render_checking(frame, state, area),
        AuthStep::Login => render_login_form(frame, state, area),
        AuthStep::Authenticating => render_authenticating(frame, state, area),
        AuthStep::ServerSelect => render_server_select(frame, state, area),
        AuthStep::Connecting => render_connecting(frame, state, area),
    }
}

fn render_checking(frame: &mut Frame, _state: &AppState, area: Rect) {
    let t = theme();

    let block = Block::default()
        .title(" Authentication ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let content = Paragraph::new("Checking for saved credentials...")
        .style(Style::default().fg(t.colors.fg_muted))
        .alignment(Alignment::Center);

    let centered_area = centered_rect(inner, 40, 3);
    frame.render_widget(content, centered_area);
}

fn render_login_form(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    let block = Block::default()
        .title(" Sign in to Plex ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border_focused))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Calculate form area - centered, fixed width
    let form_width = 50u16.min(inner.width.saturating_sub(4));
    let form_height = 12u16;
    let form_area = centered_rect(inner, form_width, form_height);

    // Layout: title, username field, password field, button, note, error
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Title
            Constraint::Length(3), // Username
            Constraint::Length(3), // Password
            Constraint::Length(2), // Button
            Constraint::Length(1), // Note
            Constraint::Min(1),    // Error/spacer
        ])
        .split(form_area);

    // Title
    let title = Paragraph::new("Sign in with your Plex account")
        .style(Style::default().fg(t.colors.fg_primary))
        .alignment(Alignment::Center);
    frame.render_widget(title, chunks[0]);

    // Username field
    let username_focused = state.auth_state.field_index == 0;
    let username_editing = username_focused && state.auth_state.editing;
    let username_style = if username_focused {
        Style::default().fg(t.colors.fg_accent)
    } else {
        Style::default().fg(t.colors.fg_muted)
    };
    let username_border = if username_focused {
        t.colors.border_focused
    } else {
        t.colors.border
    };

    let username_display = if state.auth_state.username_input.is_empty() && !username_editing {
        "username".to_string()
    } else if username_editing {
        format!("{}_", state.auth_state.username_input)
    } else {
        state.auth_state.username_input.clone()
    };

    let username_block = Block::default()
        .title(" Username ")
        .title_style(username_style)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(username_border));
    let username_inner = username_block.inner(chunks[1]);
    frame.render_widget(username_block, chunks[1]);

    let username_text = Paragraph::new(username_display)
        .style(if state.auth_state.username_input.is_empty() && !username_editing {
            Style::default().fg(t.colors.fg_muted)
        } else {
            Style::default().fg(t.colors.fg_primary)
        });
    frame.render_widget(username_text, username_inner);

    // Password field
    let password_focused = state.auth_state.field_index == 1;
    let password_editing = password_focused && state.auth_state.editing;
    let password_style = if password_focused {
        Style::default().fg(t.colors.fg_accent)
    } else {
        Style::default().fg(t.colors.fg_muted)
    };
    let password_border = if password_focused {
        t.colors.border_focused
    } else {
        t.colors.border
    };

    let password_display = if state.auth_state.password_input.is_empty() && !password_editing {
        "password".to_string()
    } else {
        let masked: String = "*".repeat(state.auth_state.password_input.len());
        if password_editing {
            format!("{}_", masked)
        } else {
            masked
        }
    };

    let password_block = Block::default()
        .title(" Password ")
        .title_style(password_style)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(password_border));
    let password_inner = password_block.inner(chunks[2]);
    frame.render_widget(password_block, chunks[2]);

    let password_text = Paragraph::new(password_display)
        .style(if state.auth_state.password_input.is_empty() && !password_editing {
            Style::default().fg(t.colors.fg_muted)
        } else {
            Style::default().fg(t.colors.fg_primary)
        });
    frame.render_widget(password_text, password_inner);

    // Sign In button
    let button_focused = state.auth_state.field_index == 2;
    let button_style = if button_focused {
        Style::default()
            .fg(t.colors.bg_primary)
            .bg(t.colors.fg_accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(t.colors.fg_primary)
            .bg(t.colors.bg_secondary)
    };

    let button = Paragraph::new("[ Sign In ]")
        .style(button_style)
        .alignment(Alignment::Center);
    frame.render_widget(button, chunks[3]);

    // Note
    let note = Paragraph::new("Your password is not stored.")
        .style(Style::default().fg(t.colors.fg_muted))
        .alignment(Alignment::Center);
    frame.render_widget(note, chunks[4]);

    // Error message (if any)
    if let Some(ref error) = state.auth_state.error_message {
        let error_text = Paragraph::new(error.as_str())
            .style(Style::default().fg(t.colors.error))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });
        frame.render_widget(error_text, chunks[5]);
    }
}

fn render_authenticating(frame: &mut Frame, _state: &AppState, area: Rect) {
    let t = theme();

    let block = Block::default()
        .title(" Authentication ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let content = Paragraph::new("Signing in...")
        .style(Style::default().fg(t.colors.fg_accent))
        .alignment(Alignment::Center);

    let centered_area = centered_rect(inner, 40, 3);
    frame.render_widget(content, centered_area);
}

fn render_server_select(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    let block = Block::default()
        .title(" Select a Server ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border_focused))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Build server list
    let items: Vec<ListItem> = state
        .available_servers
        .iter()
        .enumerate()
        .map(|(i, server)| {
            let is_selected = i == state.auth_state.server_index;
            let prefix = if is_selected { "> " } else { "  " };
            let shared = if server.owned { "" } else { " (shared)" };
            let text = format!("{}{}{}", prefix, server.name, shared);

            let style = if is_selected {
                Style::default().fg(t.colors.fg_accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.colors.fg_primary)
            };

            ListItem::new(text).style(style)
        })
        .collect();

    let list_height = (state.available_servers.len() as u16).min(10) + 4;
    let list_width = 50u16.min(inner.width.saturating_sub(4));
    let list_area = centered_rect(inner, list_width, list_height);

    // Layout: instruction, list, hint
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Instruction
            Constraint::Min(3),    // List
            Constraint::Length(1), // Hint
        ])
        .split(list_area);

    let instruction = Paragraph::new("Choose a server to connect to:")
        .style(Style::default().fg(t.colors.fg_primary))
        .alignment(Alignment::Center);
    frame.render_widget(instruction, chunks[0]);

    let list = List::new(items)
        .style(Style::default().bg(t.colors.bg_primary));
    frame.render_widget(list, chunks[1]);

    let hint = Paragraph::new("Enter: Connect")
        .style(Style::default().fg(t.colors.fg_muted))
        .alignment(Alignment::Center);
    frame.render_widget(hint, chunks[2]);
}

fn render_connecting(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    let block = Block::default()
        .title(" Authentication ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let server_name = state
        .available_servers
        .get(state.auth_state.server_index)
        .map(|s| s.name.as_str())
        .unwrap_or("server");

    let content = Paragraph::new(format!("Connecting to {}...", server_name))
        .style(Style::default().fg(t.colors.fg_accent))
        .alignment(Alignment::Center);

    let centered_area = centered_rect(inner, 50, 3);
    frame.render_widget(content, centered_area);
}

/// Helper to create a centered rect of a given size within an area.
fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
