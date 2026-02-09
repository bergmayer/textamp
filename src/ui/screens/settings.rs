//! Settings screen with Account and About sections.

use crate::app::state::{AppState, ConnectionState, CredentialField, SettingsFocus, SettingsSection};
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
        SettingsSection::Account => render_account_content(frame, state, inner),
        SettingsSection::About => render_about_content(frame, state, inner),
    }
}

fn render_account_content(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();
    let is_focused = state.settings_state.focus == SettingsFocus::Content;

    if state.settings_state.signing_in {
        render_signin_form(frame, state, area);
        return;
    }

    let mut lines = vec![];

    match &state.connection {
        ConnectionState::Connected { username, has_plex_pass } => {
            // Account info
            lines.push(Line::from(Span::styled(
                format!("Signed in as {}", username),
                Style::default().fg(t.colors.fg_primary),
            )));
            let plex_pass_text = if *has_plex_pass { "Plex Pass: Active" } else { "Plex Pass: Inactive" };
            let plex_pass_color = if *has_plex_pass { t.colors.fg_accent } else { t.colors.fg_muted };
            lines.push(Line::from(Span::styled(plex_pass_text, Style::default().fg(plex_pass_color))));
            lines.push(Line::from(""));

            // Music libraries
            let lib_count = state.libraries.len();

            lines.push(Line::from(Span::styled(
                "Music libraries:",
                Style::default().fg(t.colors.fg_accent),
            )));

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
                    let active_marker = if is_active { " *" } else { "" };
                    let style = if is_selected { Theme::selected() } else { Style::default().fg(t.colors.fg_primary) };
                    lines.push(Line::from(Span::styled(
                        format!("{}{}{}", prefix, lib.title, active_marker),
                        style,
                    )));
                }
            }

            lines.push(Line::from(""));

            // Action buttons + Sign Out
            let crawl_label = if state.subfolder_preload_active {
                "Stop Subfolder Crawl"
            } else {
                "Start Subfolder Crawl"
            };
            let action_items = [
                "Clear Library Cache & Reload",
                "Clear Artwork Cache",
                "Clear Subfolder Cache",
                crawl_label,
                "Sign Out",
            ];
            for (i, label) in action_items.iter().enumerate() {
                let item_idx = lib_count + i;
                let is_selected = is_focused && item_idx == state.settings_state.item_index;
                let prefix = if is_selected { "> " } else { "  " };
                let style = if is_selected { Theme::selected() } else { Style::default().fg(t.colors.fg_primary) };
                lines.push(Line::from(Span::styled(
                    format!("{}{}", prefix, label),
                    style,
                )));
            }

            // Help text
            lines.push(Line::from(""));
            let selected_idx = state.settings_state.item_index;
            let help_text = if !is_focused {
                "Enter: select"
            } else if selected_idx < lib_count {
                "Enter: switch to library"
            } else {
                match selected_idx - lib_count {
                    0 => "Clears cached library data and reloads from server",
                    1 => "Clears artwork image cache from disk",
                    2 => "Clears cached subfolder contents",
                    3 => if state.subfolder_preload_active {
                        "Stops the active subfolder crawl"
                    } else {
                        "Crawls folder contents (2 levels deep) in background"
                    },
                    4 => "Signs out and clears all cached data",
                    _ => "Enter: select",
                }
            };
            lines.push(Line::from(Span::styled(help_text, Style::default().fg(t.colors.fg_muted))));

            // Server info
            if let Some(ref url) = state.connected_server_url {
                lines.push(Line::from(""));

                // Find server name
                let server_info = state.available_servers.iter()
                    .find(|s| s.connections.iter().any(|c| c.uri == *url));

                if let Some(server) = server_info {
                    lines.push(Line::from(Span::styled(
                        format!("Server: {}", server.name),
                        Style::default().fg(t.colors.fg_accent),
                    )));
                } else {
                    lines.push(Line::from(Span::styled(
                        "Server:",
                        Style::default().fg(t.colors.fg_accent),
                    )));
                }

                // Parse address details
                let secure = url.starts_with("https://");
                let without_scheme = url.trim_start_matches("https://").trim_start_matches("http://");

                // Build compact server detail line: address:port | connection_type | secure/insecure
                let mut detail_parts = vec![format!("  {}", without_scheme)];
                if let Some(server) = server_info {
                    if let Some(conn) = server.connections.iter().find(|c| c.uri == *url) {
                        let conn_type = if conn.relay { "relay" } else if conn.local { "local" } else { "remote" };
                        detail_parts.push(conn_type.to_string());
                    }
                }
                detail_parts.push(if secure { "secure".to_string() } else { "insecure".to_string() });
                lines.push(Line::from(Span::styled(
                    detail_parts.join(" | "),
                    Style::default().fg(t.colors.fg_muted),
                )));
            }

            // Cache status
            if state.active_library.is_some() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Cache:",
                    Style::default().fg(t.colors.fg_accent),
                )));

                // Item counts (compact single line)
                let counts = [
                    ("artists", state.artists.len()),
                    ("albums", state.albums.len()),
                    ("playlists", state.playlists.len()),
                    ("genres", state.genres.len()),
                    ("moods", state.moods.len()),
                    ("styles", state.styles.len()),
                    ("stations", state.stations.len()),
                ];
                let count_parts: Vec<String> = counts.iter()
                    .filter(|(_, n)| *n > 0)
                    .map(|(label, n)| format!("{} {}", n, label))
                    .collect();
                if !count_parts.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!("  {}", count_parts.join(", ")),
                        Style::default().fg(t.colors.fg_muted),
                    )));
                }

                // Folder info
                let root_folder_count = state.folder_state.as_ref()
                    .and_then(|fs| fs.columns.first())
                    .map(|col| col.items.iter()
                        .filter(|item| item.item_type == crate::services::FolderItemType::Folder)
                        .count())
                    .unwrap_or(0);
                let cached_listings = state.folder_contents_cache.len();

                if root_folder_count > 0 || cached_listings > 0 {
                    let folder_text = if state.subfolder_preload_active {
                        format!("  {} root folders, {} folder listings (crawling...)", root_folder_count, cached_listings)
                    } else if cached_listings > 0 {
                        format!("  {} root folders, {} folder listings", root_folder_count, cached_listings)
                    } else {
                        format!("  {} root folders", root_folder_count)
                    };
                    lines.push(Line::from(Span::styled(folder_text, Style::default().fg(t.colors.fg_muted))));
                }

                // Artwork cache
                if let Some((art_count, art_bytes)) = state.artwork_cache_stats {
                    let size_text = if art_bytes >= 1024 * 1024 {
                        format!("{:.1} MB", art_bytes as f64 / (1024.0 * 1024.0))
                    } else if art_bytes >= 1024 {
                        format!("{} KB", art_bytes / 1024)
                    } else {
                        format!("{} B", art_bytes)
                    };
                    lines.push(Line::from(Span::styled(
                        format!("  Artwork: {} images, {}", art_count, size_text),
                        Style::default().fg(t.colors.fg_muted),
                    )));
                }

                // Cache ages
                let now_ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let mut age_parts = vec![];
                if let Some(cache_ts) = state.cache_timestamp {
                    let age = std::time::Duration::from_secs(now_ts.saturating_sub(cache_ts));
                    age_parts.push(format!("Library data: {}", format_duration(age)));
                }
                if let Some(playlist_ts) = state.playlist_cache_timestamp {
                    let age = std::time::Duration::from_secs(now_ts.saturating_sub(playlist_ts));
                    age_parts.push(format!("Playlist data: {}", format_duration(age)));
                }
                if !age_parts.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!("  {}", age_parts.join(" | ")),
                        Style::default().fg(t.colors.fg_muted),
                    )));
                }

                // Last saved
                let elapsed = state.last_cache_save.elapsed();
                let dirty_marker = if state.cache_dirty { " (unsaved changes)" } else { "" };
                lines.push(Line::from(Span::styled(
                    format!("  Last saved: {} ago{}", format_duration(elapsed), dirty_marker),
                    Style::default().fg(t.colors.fg_muted),
                )));

                // Background refresh
                if !state.background_refresh_in_progress.is_empty() {
                    let categories: Vec<_> = state.background_refresh_in_progress
                        .iter()
                        .map(|c| c.display_name())
                        .collect();
                    lines.push(Line::from(Span::styled(
                        format!("  Refreshing: {}", categories.join(", ")),
                        Style::default().fg(t.colors.fg_accent),
                    )));
                }
            }
        }
        _ => {
            // Not signed in
            lines.push(Line::from(Span::styled(
                "Not signed in",
                Style::default().fg(t.colors.fg_muted),
            )));
            lines.push(Line::from(""));

            // Sign In button (item 0)
            let is_signin_selected = is_focused && state.settings_state.item_index == 0;
            let signin_prefix = if is_signin_selected { "> " } else { "  " };
            let signin_style = if is_signin_selected { Theme::selected() } else { Style::default().fg(t.colors.fg_primary) };
            lines.push(Line::from(Span::styled(
                format!("{}Sign In", signin_prefix),
                signin_style,
            )));

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Enter: start sign-in",
                Style::default().fg(t.colors.fg_muted),
            )));
        }
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

/// Render the sign-in form (username/password/sign in button/servers).
fn render_signin_form(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();
    let mut lines = vec![];
    let is_focused = state.settings_state.focus == SettingsFocus::Content;

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
    let signin_style = if is_signin_selected { Theme::selected() } else { Style::default().fg(t.colors.fg_primary) };
    let signin_text = if state.settings_state.discovering_servers { "Signing in..." } else { "Sign In" };
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
        for (i, server) in state.available_servers.iter().enumerate() {
            let server_index = i + 3;
            let is_selected = is_focused && server_index == state.settings_state.item_index;
            let prefix = if is_selected { "> " } else { "  " };
            let style = if is_selected { Theme::selected() } else { Style::default().fg(t.colors.fg_primary) };
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
            "Enter: edit field | Esc: cancel sign-in",
            Style::default().fg(t.colors.fg_muted),
        )));
    } else if is_focused && state.settings_state.item_index == 2 {
        lines.push(Line::from(Span::styled(
            "Enter: sign in (password is not stored) | Esc: cancel",
            Style::default().fg(t.colors.fg_muted),
        )));
    } else if is_focused && state.settings_state.item_index >= 3 {
        lines.push(Line::from(Span::styled(
            "Enter: connect to server | Esc: cancel",
            Style::default().fg(t.colors.fg_muted),
        )));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

/// Format a Duration as a human-readable string (e.g. "5m", "2h", "3d").
fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

fn render_about_content(frame: &mut Frame, state: &AppState, area: Rect) {
    let is_focused = state.settings_state.focus == SettingsFocus::Content;
    let t = theme();

    let mut lines = parse_ansi_logo(t.colors.bg_primary);

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Version {}", env!("CARGO_PKG_VERSION")),
        Style::default().fg(t.colors.fg_primary),
    )));
    lines.push(Line::from(Span::styled(
        "A keyboard-driven TUI client for Plex Music",
        Style::default().fg(t.colors.fg_muted),
    )));
    lines.push(Line::from(Span::styled(
        "Author: John Bergmayer | License: MIT",
        Style::default().fg(t.colors.fg_primary),
    )));
    lines.push(Line::from(Span::styled(
        "https://github.com/bergmayer/textamp",
        Style::default().fg(t.colors.fg_accent),
    )));

    // Theme selection
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Theme:",
        Style::default().fg(t.colors.fg_accent),
    )));

    for (i, theme_name) in ThemeName::all().iter().enumerate() {
        let is_active = *theme_name == state.theme;
        let is_selected = is_focused && i == state.settings_state.item_index;
        let prefix = if is_selected { "> " } else { "  " };
        let active_marker = if is_active { " *" } else { "" };
        let style = if is_selected { Theme::selected() } else { Style::default().fg(t.colors.fg_primary) };
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

    // Graphics info
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Graphics:",
        Style::default().fg(t.colors.fg_accent),
    )));
    let protocol = crate::ui::screens::now_playing::artwork_protocol_name();
    lines.push(Line::from(Span::styled(
        format!("  Protocol: {} | Terminal: {}x{}", protocol, state.terminal_width, state.terminal_height),
        Style::default().fg(t.colors.fg_muted),
    )));

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

/// Count lines in the ANSI logo (used by mouse hit-testing).
pub(crate) fn ansi_logo_line_count() -> usize {
    let raw = include_str!("../../../textamp_clean.ansi");
    raw.lines().filter(|l| !l.is_empty()).count()
}

/// Parse the embedded ANSI art logo, replacing black with the theme background.
fn parse_ansi_logo(theme_bg: Color) -> Vec<Line<'static>> {
    let raw = include_str!("../../../textamp_clean.ansi");
    let mut result = Vec::new();

    for line in raw.lines() {
        let spans = parse_ansi_line(line, theme_bg);
        if !spans.is_empty() {
            result.push(Line::from(spans));
        }
    }

    result
}

/// Parse a single line of ANSI escape sequences into ratatui styled spans.
fn parse_ansi_line(line: &str, theme_bg: Color) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut current_text = String::new();
    let mut fg: Option<Color> = None;
    let mut bg: Option<Color> = None;

    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '\x1b' && i + 1 < chars.len() && chars[i + 1] == '[' {
            // Flush accumulated text as a span
            if !current_text.is_empty() {
                let mut style = Style::default();
                if let Some(c) = fg { style = style.fg(c); }
                if let Some(c) = bg { style = style.bg(c); }
                spans.push(Span::styled(std::mem::take(&mut current_text), style));
            }

            i += 2; // skip ESC[

            // Private mode sequences (e.g., ?25l cursor hide, ?25h cursor show)
            if i < chars.len() && chars[i] == '?' {
                while i < chars.len() && !chars[i].is_ascii_alphabetic() {
                    i += 1;
                }
                if i < chars.len() { i += 1; }
                continue;
            }

            // SGR sequence: collect params until 'm'
            let param_start = i;
            while i < chars.len() && chars[i] != 'm' {
                i += 1;
            }
            if i >= chars.len() { break; }

            let param_str: String = chars[param_start..i].iter().collect();
            i += 1; // skip 'm'

            let params: Vec<u8> = param_str
                .split(';')
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse().ok())
                .collect();

            if params.is_empty() {
                fg = None;
                bg = None;
            } else {
                let mut j = 0;
                while j < params.len() {
                    match params[j] {
                        0 => { fg = None; bg = None; j += 1; }
                        38 if j + 4 < params.len() && params[j + 1] == 2 => {
                            let (r, g, b) = (params[j + 2], params[j + 3], params[j + 4]);
                            fg = Some(if r == 0 && g == 0 && b == 0 { theme_bg } else { Color::Rgb(r, g, b) });
                            j += 5;
                        }
                        48 if j + 4 < params.len() && params[j + 1] == 2 => {
                            let (r, g, b) = (params[j + 2], params[j + 3], params[j + 4]);
                            bg = Some(if r == 0 && g == 0 && b == 0 { theme_bg } else { Color::Rgb(r, g, b) });
                            j += 5;
                        }
                        _ => { j += 1; }
                    }
                }
            }
        } else {
            current_text.push(chars[i]);
            i += 1;
        }
    }

    // Flush remaining text
    if !current_text.is_empty() {
        let mut style = Style::default();
        if let Some(c) = fg { style = style.fg(c); }
        if let Some(c) = bg { style = style.bg(c); }
        spans.push(Span::styled(current_text, style));
    }

    spans
}
