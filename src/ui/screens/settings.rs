//! Settings screen with account, library, playback, and interface options.

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
        SettingsSection::Libraries => render_libraries_content(frame, state, inner),
        SettingsSection::Playback => render_playback_content(frame, state, inner),
        SettingsSection::Interface => render_interface_content(frame, state, inner),
        SettingsSection::About => render_about_content(frame, inner),
    }
}

fn render_account_content(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();
    let is_focused = state.settings_state.focus == SettingsFocus::Content;

    if state.settings_state.signing_in {
        // Sign-in mode: show login form (same as old Server section)
        render_signin_form(frame, state, area);
        return;
    }

    let mut lines = vec![];

    match &state.connection {
        ConnectionState::Connected { username, has_plex_pass } => {
            // Signed in state
            lines.push(Line::from(Span::styled(
                format!("Signed in as {}", username),
                Style::default().fg(t.colors.fg_primary),
            )));

            let plex_pass_text = if *has_plex_pass { "Plex Pass: Active" } else { "Plex Pass: Inactive" };
            let plex_pass_color = if *has_plex_pass { t.colors.fg_accent } else { t.colors.fg_muted };
            lines.push(Line::from(Span::styled(plex_pass_text, Style::default().fg(plex_pass_color))));
            lines.push(Line::from(""));

            // Active library info
            if let Some(ref lib_key) = state.active_library {
                let lib_name = state.libraries.iter()
                    .find(|l| &l.key == lib_key)
                    .map(|l| l.title.as_str())
                    .unwrap_or("Unknown");
                lines.push(Line::from(Span::styled(
                    format!("Library: {}", lib_name),
                    Style::default().fg(t.colors.fg_muted),
                )));
                lines.push(Line::from(""));
            }

            // Sign Out (item 0)
            let is_signout_selected = is_focused && state.settings_state.item_index == 0;
            let signout_prefix = if is_signout_selected { "> " } else { "  " };
            let signout_style = if is_signout_selected { Theme::selected() } else { Style::default().fg(t.colors.fg_primary) };
            lines.push(Line::from(Span::styled(
                format!("{}Sign Out", signout_prefix),
                signout_style,
            )));

            // Help text
            lines.push(Line::from(""));
            let help_text = if is_signout_selected {
                "Signs out and clears all cached data"
            } else {
                "Enter: select action"
            };
            lines.push(Line::from(Span::styled(help_text, Style::default().fg(t.colors.fg_muted))));
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

fn render_libraries_content(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();
    let mut lines = vec![];
    let is_focused = state.settings_state.focus == SettingsFocus::Content;

    // Server connection info
    if let Some(ref url) = state.connected_server_url {
        lines.push(Line::from(Span::styled(
            "Server:",
            Style::default().fg(t.colors.fg_accent),
        )));
        lines.push(Line::from(""));

        // Find server name from available_servers
        let server_name = state.available_servers.iter()
            .find(|s| s.connections.iter().any(|c| c.uri == *url))
            .map(|s| s.name.as_str());
        if let Some(name) = server_name {
            lines.push(Line::from(Span::styled(
                format!("  Name: {}", name),
                Style::default().fg(t.colors.fg_muted),
            )));
        }

        // Parse and display address info (scheme://host:port)
        let secure = url.starts_with("https://");
        let without_scheme = url.trim_start_matches("https://").trim_start_matches("http://");
        let (host, port) = if let Some(colon) = without_scheme.rfind(':') {
            (&without_scheme[..colon], Some(&without_scheme[colon + 1..]))
        } else {
            (without_scheme, None)
        };
        lines.push(Line::from(Span::styled(
            format!("  Address: {}", host),
            Style::default().fg(t.colors.fg_muted),
        )));
        if let Some(port) = port {
            lines.push(Line::from(Span::styled(
                format!("  Port: {}", port),
                Style::default().fg(t.colors.fg_muted),
            )));
        }
        lines.push(Line::from(Span::styled(
            format!("  Secure: {}", if secure { "yes" } else { "no" }),
            Style::default().fg(t.colors.fg_muted),
        )));

        // Check if this is a local connection
        if let Some(server) = state.available_servers.iter()
            .find(|s| s.connections.iter().any(|c| c.uri == *url))
        {
            if let Some(conn) = server.connections.iter().find(|c| c.uri == *url) {
                let conn_type = if conn.relay {
                    "relay"
                } else if conn.local {
                    "local"
                } else {
                    "remote"
                };
                lines.push(Line::from(Span::styled(
                    format!("  Connection: {}", conn_type),
                    Style::default().fg(t.colors.fg_muted),
                )));
            }
        }

        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        "Music libraries:",
        Style::default().fg(t.colors.fg_accent),
    )));
    lines.push(Line::from(""));

    let lib_count = state.libraries.len();

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

    // Action buttons (after libraries)
    lines.push(Line::from(""));

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
    let help_text = if is_focused && selected_idx < lib_count {
        "Enter: switch to library"
    } else if is_focused && selected_idx == lib_count {
        "Clears cached library data and reloads from server"
    } else if is_focused && selected_idx == lib_count + 1 {
        "Clears artwork image cache from disk"
    } else if is_focused && selected_idx == lib_count + 2 {
        "Clears cached subfolder contents"
    } else if is_focused && selected_idx == lib_count + 3 {
        if state.subfolder_preload_active {
            "Stops the active subfolder crawl"
        } else {
            "Crawls root-level subfolders in background"
        }
    } else {
        "Enter: select"
    };
    lines.push(Line::from(Span::styled(help_text, Style::default().fg(t.colors.fg_muted))));

    // Cache status section (for active library)
    if state.active_library.is_some() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Cache status:",
            Style::default().fg(t.colors.fg_accent),
        )));
        lines.push(Line::from(""));

        // Item counts
        let counts = [
            ("Artists", state.artists.len()),
            ("Albums", state.albums.len()),
            ("Playlists", state.playlists.len()),
            ("Genres", state.genres.len()),
            ("Moods", state.moods.len()),
            ("Styles", state.styles.len()),
            ("Stations", state.stations.len()),
        ];
        let count_parts: Vec<String> = counts.iter()
            .filter(|(_, n)| *n > 0)
            .map(|(label, n)| format!("{} {}", n, label.to_lowercase()))
            .collect();
        if !count_parts.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("  Items: {}", count_parts.join(", ")),
                Style::default().fg(t.colors.fg_muted),
            )));
        }

        // Root folders count
        let root_folder_count = state.folder_state.as_ref()
            .and_then(|fs| fs.columns.first())
            .map(|col| col.items.iter()
                .filter(|item| item.item_type == crate::services::FolderItemType::Folder)
                .count())
            .unwrap_or(0);

        if root_folder_count > 0 {
            lines.push(Line::from(Span::styled(
                format!("  Root folders: {}", root_folder_count),
                Style::default().fg(t.colors.fg_muted),
            )));
        }

        // Subfolder cache status
        let cached_subfolders = state.folder_contents_cache.len();
        if root_folder_count > 0 || cached_subfolders > 0 {
            // Count stale entries (> 32 days old)
            let now_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let stale_threshold = crate::plex::constants::CACHE_VERY_STALE_THRESHOLD_SECS;
            let stale_count = state.folder_contents_cache.values()
                .filter(|c| now_ts.saturating_sub(c.timestamp) >= stale_threshold)
                .count();
            let fresh_count = cached_subfolders - stale_count;

            let status_text = if state.subfolder_preload_active {
                if stale_count > 0 {
                    format!("  Subfolder cache: refreshing... {} fresh, {} stale", fresh_count, stale_count)
                } else {
                    format!("  Subfolder cache: crawling... {} cached", cached_subfolders)
                }
            } else if cached_subfolders > 0 {
                if stale_count > 0 {
                    format!("  Subfolder cache: {} fresh, {} stale", fresh_count, stale_count)
                } else {
                    format!("  Subfolder cache: {} subfolders cached", cached_subfolders)
                }
            } else {
                "  Subfolder cache: empty".to_string()
            };
            lines.push(Line::from(Span::styled(
                status_text,
                Style::default().fg(t.colors.fg_muted),
            )));
        }

        // Artwork cache stats
        if let Some((art_count, art_bytes)) = state.artwork_cache_stats {
            let size_text = if art_bytes >= 1024 * 1024 {
                format!("{:.1} MB", art_bytes as f64 / (1024.0 * 1024.0))
            } else if art_bytes >= 1024 {
                format!("{} KB", art_bytes / 1024)
            } else {
                format!("{} B", art_bytes)
            };
            lines.push(Line::from(Span::styled(
                format!("  Artwork cache: {} images, {}", art_count, size_text),
                Style::default().fg(t.colors.fg_muted),
            )));
        }

        // Playlist tracks cached
        let cached_playlist_count = state.playlist_tracks_cache.len();
        if cached_playlist_count > 0 {
            lines.push(Line::from(Span::styled(
                format!("  Playlist tracks: {} playlists cached", cached_playlist_count),
                Style::default().fg(t.colors.fg_muted),
            )));
        }

        // Cache ages (separate timestamps for library vs playlist/dynamic data)
        let now_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if let Some(cache_ts) = state.cache_timestamp {
            let cache_age = std::time::Duration::from_secs(now_ts.saturating_sub(cache_ts));
            lines.push(Line::from(Span::styled(
                format!("  Library data age: {}", format_duration(cache_age)),
                Style::default().fg(t.colors.fg_muted),
            )));
        }
        if let Some(playlist_ts) = state.playlist_cache_timestamp {
            let playlist_age = std::time::Duration::from_secs(now_ts.saturating_sub(playlist_ts));
            lines.push(Line::from(Span::styled(
                format!("  Playlist data age: {}", format_duration(playlist_age)),
                Style::default().fg(t.colors.fg_muted),
            )));
        }

        // Cache save status
        let elapsed = state.last_cache_save.elapsed();
        let age_text = format_duration(elapsed);
        let dirty_marker = if state.cache_dirty { " (unsaved changes)" } else { "" };
        lines.push(Line::from(Span::styled(
            format!("  Last saved: {} ago{}", age_text, dirty_marker),
            Style::default().fg(t.colors.fg_muted),
        )));

        // Background refresh status
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

    // Graphics protocol info
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Graphics:",
        Style::default().fg(t.colors.fg_accent),
    )));
    lines.push(Line::from(""));
    let protocol = crate::ui::screens::now_playing::artwork_protocol_name();
    lines.push(Line::from(Span::styled(
        format!("  Protocol: {}", protocol),
        Style::default().fg(t.colors.fg_muted),
    )));
    lines.push(Line::from(Span::styled(
        format!("  Terminal: {}x{}", state.terminal_width, state.terminal_height),
        Style::default().fg(t.colors.fg_muted),
    )));

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

fn render_about_content(frame: &mut Frame, area: Rect) {
    let t = theme();

    let mut lines = parse_ansi_logo(t.colors.bg_primary);

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
        "License: MIT",
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
