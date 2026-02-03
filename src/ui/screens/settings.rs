//! Settings screen with server, library, playback, and interface options.

use crate::app::state::{AppState, CredentialField, SettingsFocus, SettingsSection};
use crate::ui::theme::{Theme, ThemeName, theme};

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Fill background
    frame.render_widget(
        Block::default().style(Style::default().bg(t.colors.bg_primary)),
        area
    );

    // Split into left (sections) and right (content) panels
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(16),
            Constraint::Min(0),
        ])
        .split(area);

    render_sections(frame, state, chunks[0]);
    render_content(frame, state, chunks[1]);
}

fn render_sections(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();
    let is_focused = state.settings_state.focus == SettingsFocus::Sections;
    let border_color = if is_focused { t.colors.border_focused } else { t.colors.border };

    let block = Block::default()
        .title(" settings ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections: Vec<ListItem> = SettingsSection::all()
        .iter()
        .map(|section| {
            let is_selected = *section == state.settings_state.section;
            let prefix = if is_selected { "> " } else { "  " };
            let style = if is_selected {
                Theme::selected()
            } else {
                Style::default().fg(t.colors.fg_primary)
            };
            ListItem::new(format!("{}{}", prefix, section.name())).style(style)
        })
        .collect();

    let list = List::new(sections);
    frame.render_widget(list, inner);
}

fn render_content(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();
    let is_focused = state.settings_state.focus == SettingsFocus::Content;
    let border_color = if is_focused { t.colors.border_focused } else { t.colors.border };

    let title = format!(" {} ", state.settings_state.section.name());
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    match state.settings_state.section {
        SettingsSection::Server => render_server_content(frame, state, inner),
        SettingsSection::Libraries => render_libraries_content(frame, state, inner),
        SettingsSection::Playback => render_playback_content(frame, state, inner),
        SettingsSection::Interface => render_interface_content(frame, state, inner),
        SettingsSection::Data => render_data_content(frame, state, inner),
        SettingsSection::About => render_about_content(frame, inner),
    }
}

fn render_server_content(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();
    let mut lines = vec![];
    let is_focused = state.settings_state.focus == SettingsFocus::Content;

    // Current server/library info
    if let Some(ref lib_key) = state.active_library {
        let lib_name = state.libraries.iter()
            .find(|l| &l.key == lib_key)
            .map(|l| l.title.as_str())
            .unwrap_or("Unknown");
        lines.push(Line::from(Span::styled(
            format!("Active library: {}", lib_name),
            Style::default().fg(t.colors.fg_primary),
        )));
        lines.push(Line::from(""));
    }

    // Sign In section
    lines.push(Line::from(Span::styled(
        "Sign In:",
        Style::default().fg(t.colors.fg_accent),
    )));

    // Username field (item index 0)
    let is_username_selected = is_focused && state.settings_state.item_index == 0;
    let is_username_editing = state.settings_state.editing_credential == Some(CredentialField::Username);
    let username_display = if is_username_editing {
        format!("{}█", state.settings_state.username_input)
    } else if state.settings_state.username_input.is_empty() {
        "(enter username)".to_string()
    } else {
        state.settings_state.username_input.clone()
    };
    let username_prefix = if is_username_selected { "> " } else { "  " };
    let username_style = if is_username_editing {
        Style::default().fg(t.colors.fg_accent).add_modifier(ratatui::style::Modifier::BOLD)
    } else if is_username_selected {
        Theme::selected()
    } else {
        Style::default().fg(t.colors.fg_primary)
    };
    lines.push(Line::from(Span::styled(
        format!("{}Username: {}", username_prefix, username_display),
        username_style,
    )));

    // Password field (item index 1)
    let is_password_selected = is_focused && state.settings_state.item_index == 1;
    let is_password_editing = state.settings_state.editing_credential == Some(CredentialField::Password);
    let password_display = if is_password_editing {
        format!("{}█", "•".repeat(state.settings_state.password_input.len()))
    } else if state.settings_state.password_input.is_empty() {
        "(enter password)".to_string()
    } else {
        "•".repeat(state.settings_state.password_input.len())
    };
    let password_prefix = if is_password_selected { "> " } else { "  " };
    let password_style = if is_password_editing {
        Style::default().fg(t.colors.fg_accent).add_modifier(ratatui::style::Modifier::BOLD)
    } else if is_password_selected {
        Theme::selected()
    } else {
        Style::default().fg(t.colors.fg_primary)
    };
    lines.push(Line::from(Span::styled(
        format!("{}Password: {}", password_prefix, password_display),
        password_style,
    )));

    // Sign In button (item index 2)
    let is_signin_selected = is_focused && state.settings_state.item_index == 2;
    let signin_prefix = if is_signin_selected { "> " } else { "  " };
    let signin_style = if is_signin_selected {
        Theme::selected()
    } else {
        Style::default().fg(t.colors.fg_primary)
    };
    let signin_text = if state.settings_state.discovering_servers {
        "Signing in..."
    } else {
        "Sign In"
    };
    lines.push(Line::from(Span::styled(
        format!("{}{}", signin_prefix, signin_text),
        signin_style,
    )));

    // Available servers
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Available servers:",
        Style::default().fg(t.colors.fg_accent),
    )));

    if state.available_servers.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (sign in to discover servers)",
            Style::default().fg(t.colors.fg_muted),
        )));
    } else {
        // Server items start at index 3 (after username, password, sign in)
        for (i, server) in state.available_servers.iter().enumerate() {
            let server_index = i + 3; // Offset by credential fields + sign in button
            let is_selected = is_focused && server_index == state.settings_state.item_index;
            let prefix = if is_selected { "> " } else { "  " };
            let style = if is_selected {
                Theme::selected()
            } else {
                Style::default().fg(t.colors.fg_primary)
            };
            lines.push(Line::from(Span::styled(
                format!("{}{}", prefix, server.name),
                style,
            )));
        }
    }

    // Help text
    lines.push(Line::from(""));
    if state.settings_state.editing_credential.is_some() {
        lines.push(Line::from(Span::styled(
            "Type to enter | Enter: done | Esc: cancel",
            Style::default().fg(t.colors.fg_muted),
        )));
    } else if is_focused && state.settings_state.item_index <= 1 {
        lines.push(Line::from(Span::styled(
            "Enter: edit field",
            Style::default().fg(t.colors.fg_muted),
        )));
    } else if is_focused && state.settings_state.item_index == 2 {
        lines.push(Line::from(Span::styled(
            "Enter: sign in (password is not stored)",
            Style::default().fg(t.colors.fg_muted),
        )));
    } else if is_focused && state.settings_state.item_index >= 3 {
        lines.push(Line::from(Span::styled(
            "Enter: connect to server",
            Style::default().fg(t.colors.fg_muted),
        )));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

fn render_libraries_content(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();
    let mut lines = vec![];
    let is_focused = state.settings_state.focus == SettingsFocus::Content;

    lines.push(Line::from(Span::styled(
        "Music libraries:",
        Style::default().fg(t.colors.fg_accent),
    )));
    lines.push(Line::from(""));

    if state.libraries.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No libraries available",
            Style::default().fg(t.colors.fg_muted),
        )));
    } else {
        for (i, lib) in state.libraries.iter().enumerate() {
            let is_active = state.active_library.as_ref() == Some(&lib.key);
            let is_selected = is_focused && i == state.settings_state.item_index;
            let prefix = if is_selected { "> " } else { "  " };
            let active_marker = if is_active { " *" } else { "  " };

            let style = if is_selected {
                Theme::selected()
            } else {
                Style::default().fg(t.colors.fg_primary)
            };

            lines.push(Line::from(Span::styled(
                format!("{}{}{}", prefix, lib.title, active_marker),
                style,
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Enter: switch to library",
        Style::default().fg(t.colors.fg_muted),
    )));

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

fn render_playback_content(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();
    let lines = vec![
        Line::from(Span::styled(
            "Playback settings:",
            Style::default().fg(t.colors.fg_accent),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("  Volume: {:.0}%", state.playback.volume * 100.0),
            Style::default().fg(t.colors.fg_primary),
        )),
        Line::from(Span::styled(
            format!("  Shuffle: {}", if state.playback.shuffle { "on" } else { "off" }),
            Style::default().fg(t.colors.fg_primary),
        )),
        Line::from(Span::styled(
            format!("  Repeat: {}", state.playback.repeat_mode.label().trim()),
            Style::default().fg(t.colors.fg_primary),
        )),
    ];

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

fn render_interface_content(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();
    let is_focused = state.settings_state.focus == SettingsFocus::Content;

    let mut lines = vec![
        Line::from(Span::styled(
            "Theme:",
            Style::default().fg(t.colors.fg_accent),
        )),
        Line::from(""),
    ];

    // List available themes
    for (i, theme_name) in ThemeName::all().iter().enumerate() {
        let is_active = *theme_name == state.theme;
        let is_selected = is_focused && i == state.settings_state.item_index;
        let prefix = if is_selected { "> " } else { "  " };
        let active_marker = if is_active { " *" } else { "" };

        let style = if is_selected {
            Theme::selected()
        } else {
            Style::default().fg(t.colors.fg_primary)
        };

        lines.push(Line::from(Span::styled(
            format!("{}{}{}", prefix, theme_name.display_name(), active_marker),
            style,
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Enter: apply theme",
        Style::default().fg(t.colors.fg_muted),
    )));

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

fn render_data_content(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();
    let is_focused = state.settings_state.focus == SettingsFocus::Content;

    let mut lines = vec![
        Line::from(Span::styled(
            "Data Management:",
            Style::default().fg(t.colors.fg_accent),
        )),
        Line::from(""),
    ];

    // Clear Cache option (index 0)
    let is_clear_selected = is_focused && state.settings_state.item_index == 0;
    let clear_prefix = if is_clear_selected { "> " } else { "  " };
    let clear_style = if is_clear_selected {
        Theme::selected()
    } else {
        Style::default().fg(t.colors.fg_primary)
    };
    lines.push(Line::from(Span::styled(
        format!("{}Clear Cache & Reload", clear_prefix),
        clear_style,
    )));

    // Sign Out option (index 1)
    let is_signout_selected = is_focused && state.settings_state.item_index == 1;
    let signout_prefix = if is_signout_selected { "> " } else { "  " };
    let signout_style = if is_signout_selected {
        Theme::selected()
    } else {
        Style::default().fg(t.colors.fg_primary)
    };
    lines.push(Line::from(Span::styled(
        format!("{}Sign Out", signout_prefix),
        signout_style,
    )));

    lines.push(Line::from(""));

    // Help text based on selection
    let help_text = if is_clear_selected {
        "Clears cached library data and reloads from server"
    } else if is_signout_selected {
        "Signs out and clears all cached data"
    } else {
        "Enter: select action"
    };

    lines.push(Line::from(Span::styled(
        help_text,
        Style::default().fg(t.colors.fg_muted),
    )));

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

fn render_about_content(frame: &mut Frame, area: Rect) {
    let t = theme();

    // ASCII art logo
    let logo = vec![
        " ██                ██                       ",
        "▀██▀▀ ▄█▀█▄ ██ ██ ▀██▀▀ ▀▀█▄ ███▄███▄ ████▄ ",
        " ██   ██▄█▀  ███   ██  ▄█▀██ ██ ██ ██ ██ ██ ",
        " ██   ▀█▄▄▄ ██ ██  ██  ▀█▄██ ██ ██ ██ ████▀ ",
        "                                      ██    ",
        "                                      ▀▀    ",
    ];

    let mut lines: Vec<Line> = logo
        .iter()
        .map(|line| Line::from(Span::styled(*line, Style::default().fg(t.colors.fg_accent))))
        .collect();

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Version {}", env!("CARGO_PKG_VERSION")),
        Style::default().fg(t.colors.fg_primary),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "A keyboard-driven TUI client for Plex Music",
        Style::default().fg(t.colors.fg_muted),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Author: John Bergmayer",
        Style::default().fg(t.colors.fg_primary),
    )));
    lines.push(Line::from(Span::styled(
        "License: GPL-3.0",
        Style::default().fg(t.colors.fg_primary),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "https://github.com/bergmayer/textamp",
        Style::default().fg(t.colors.fg_accent),
    )));

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}
