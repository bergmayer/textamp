//! Settings screen with Account and About sections.

use crate::app::state::{AppState, ConnectionState, CredentialField, SettingsFocus, SettingsSection};
use crate::ui::theme::{ThemeName, theme};

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
            let prefix = "  ";
            let style = if is_selected {
                Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
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
        SettingsSection::Account => render_account_content(frame, state, area, inner),
        SettingsSection::Textamp => render_textamp_content(frame, state, area, inner),
        SettingsSection::Sections => render_sections_content(frame, state, area, inner),
        SettingsSection::Cache => render_account_content(frame, state, area, inner),
        SettingsSection::About => render_about_content(frame, state, area, inner),
    }
}

/// Render the "Sections" tab — a checkbox per BrowseCategory letting
/// the user hide/show each section in the leftmost browse column.
fn render_sections_content(frame: &mut Frame, state: &AppState, _outer: Rect, area: Rect) {
    let t = theme();
    let is_focused = state.settings_state.focus == SettingsFocus::Content;
    let cats = crate::app::state::BrowseCategory::all();

    let mut lines: Vec<ratatui::text::Line> = vec![
        ratatui::text::Line::from(ratatui::text::Span::styled(
            "Show in left column:",
            Style::default().fg(t.colors.fg_accent),
        )),
        ratatui::text::Line::from(""),
    ];

    for (idx, cat) in cats.iter().enumerate() {
        let visible = !state.hidden_sections.contains(cat);
        let check = if visible { "\u{2611}" } else { "\u{2610}" };
        let is_selected = is_focused && idx == state.settings_state.item_index;
        let style = if is_selected {
            Style::default().fg(t.colors.selection_text).bg(t.colors.bg_selection)
        } else {
            Style::default().fg(t.colors.fg_primary)
        };
        let arrow = if is_selected { "\u{25b8} " } else { "  " };
        lines.push(ratatui::text::Line::from(vec![
            ratatui::text::Span::styled(arrow, style),
            ratatui::text::Span::styled(format!("{} {}", check, cat.display_label()), style),
        ]));
    }

    let para = ratatui::widgets::Paragraph::new(lines).style(Style::default().bg(t.colors.bg_primary));
    frame.render_widget(para, area);
}

fn render_account_content(frame: &mut Frame, state: &AppState, outer: Rect, area: Rect) {
    let t = theme();
    let is_focused = state.settings_state.focus == SettingsFocus::Content;

    if state.settings_state.signing_in {
        render_signin_form(frame, state, area);
        return;
    }

    let mut lines = vec![];
    let mut selected_line: Option<usize> = None;
    let connected = matches!(state.connection, ConnectionState::Connected { .. });

    // Account info header
    match &state.connection {
        ConnectionState::Connected { username, has_plex_pass } => {
            lines.push(Line::from(Span::styled(
                format!("signed in as {}", username),
                Style::default().fg(t.colors.fg_primary),
            )));
            let plex_pass_text = if *has_plex_pass { "plex pass: active" } else { "plex pass: inactive" };
            let plex_pass_color = if *has_plex_pass { t.colors.fg_accent } else { t.colors.fg_muted };
            lines.push(Line::from(Span::styled(plex_pass_text, Style::default().fg(plex_pass_color))));
        }
        _ => {
            lines.push(Line::from(Span::styled(
                "not signed in",
                Style::default().fg(t.colors.fg_muted),
            )));
        }
    }
    lines.push(Line::from(""));

    // Music libraries (always shown)
    let lib_count = state.libraries.len();

    lines.push(Line::from(Span::styled(
        "music libraries:",
        Style::default().fg(t.colors.fg_accent),
    )));

    if connected && !state.libraries.is_empty() {
        for (i, lib) in state.libraries.iter().enumerate() {
            let is_active = state.active_library.as_ref() == Some(&lib.key);
            let is_selected = is_focused && i == state.settings_state.item_index;
            if is_selected { selected_line = Some(lines.len()); }
            let prefix = if is_active { "  ♪ " } else { "  " };
            let style = if is_selected { Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg) } else { Style::default().fg(t.colors.fg_primary) };
            lines.push(Line::from(Span::styled(
                format!("{}{}", prefix, lib.title),
                style,
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "  (signed out)",
            Style::default().fg(t.colors.fg_muted),
        )));
    }

    lines.push(Line::from(""));

    // Action buttons / Sign In
    if connected {
        let crawl_label = if state.subfolder_preload_active {
            "stop subfolder crawl"
        } else {
            "start subfolder crawl"
        };
        let keep_cache_label = if state.keep_subfolder_cache {
            "keep subfolder cache: on"
        } else {
            "keep subfolder cache: off"
        };
        let action_items = [
            "clear library cache & reload",
            "clear artwork cache",
            "clear subfolder cache",
            crawl_label,
            keep_cache_label,
            "sign out",
        ];
        for (i, label) in action_items.iter().enumerate() {
            let item_idx = lib_count + i;
            let is_selected = is_focused && item_idx == state.settings_state.item_index;
            if is_selected { selected_line = Some(lines.len()); }
            let prefix = "  ";
            let style = if is_selected { Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg) } else { Style::default().fg(t.colors.fg_primary) };
            lines.push(Line::from(Span::styled(
                format!("{}{}", prefix, label),
                style,
            )));
        }

    } else {
        // Sign In button (item 0)
        let is_signin_selected = is_focused && state.settings_state.item_index == 0;
        if is_signin_selected { selected_line = Some(lines.len()); }
        let signin_prefix = "  ";
        let signin_style = if is_signin_selected { Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg) } else { Style::default().fg(t.colors.fg_primary) };
        lines.push(Line::from(Span::styled(
            format!("{}sign in", signin_prefix),
            signin_style,
        )));
    }

    // Server info (always shown)
    lines.push(Line::from(""));
    if let Some(ref url) = state.connected_server_url {
        let server_info = state.available_servers.iter()
            .find(|s| s.connections.iter().any(|c| c.uri == *url));

        if let Some(server) = server_info {
            lines.push(Line::from(Span::styled(
                format!("server: {}", server.name),
                Style::default().fg(t.colors.fg_accent),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "server:",
                Style::default().fg(t.colors.fg_accent),
            )));
        }

        let secure = url.starts_with("https://");
        let without_scheme = url.trim_start_matches("https://").trim_start_matches("http://");
        let mut detail_parts = vec![format!("  {}", without_scheme)];
        if let Some(server) = server_info {
            if let Some(conn) = server.connections.iter().find(|c| c.uri == *url) {
                let conn_type = if conn.relay { "relay" } else if conn.local { "local" } else { "remote" };
                detail_parts.push(conn_type.to_string());
            }
        }
        detail_parts.push(if secure { "secure".to_string() } else { "insecure".to_string() });
        if !connected {
            detail_parts.push("disconnected".to_string());
        }
        lines.push(Line::from(Span::styled(
            detail_parts.join(" | "),
            Style::default().fg(t.colors.fg_muted),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "server: (not connected)",
            Style::default().fg(t.colors.fg_accent),
        )));
    }

    // Cache status (always shown)
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "cache:",
        Style::default().fg(t.colors.fg_accent),
    )));

    if connected && state.active_library.is_some() {
        use crate::app::state::RefreshCategory;

        let now_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let alias_count: usize = state.library.artist_aliases.values().map(|s| s.len()).sum();

        // Mirror the GUI Cache table 1:1 — same columns (Category /
        // Count / Age / Size), same rows (Artists, Albums, Tracks,
        // Playlists, Genres, Moods, Styles, Stations, Aliases,
        // Folders, Artwork, Waveforms), same total. The fifth tuple
        // slot is the breakdown field name in `library_cache_stats`
        // for the Size column lookup; `None` means "no size shown".
        let library_size = |field: &str| -> Option<u64> {
            state
                .library_cache_stats
                .as_ref()
                .and_then(|(_, breakdown)| breakdown.iter().find(|(k, _)| k == field).map(|(_, v)| *v))
        };

        let row_specs: &[(&str, usize, bool, Option<RefreshCategory>, Option<&str>)] = &[
            ("Artists",   state.library.artists.len(),    false, Some(RefreshCategory::Artists),   Some("artists")),
            ("Albums",    state.library.albums.len(),     false, Some(RefreshCategory::Albums),    Some("albums")),
            ("Tracks",    state.library.all_tracks.len(), state.library.all_tracks.is_empty(), Some(RefreshCategory::AllTracks), Some("tracks")),
            ("Playlists", state.library.playlists.len(),  false, Some(RefreshCategory::Playlists), Some("playlist tracks")),
            ("Genres",    state.library.album_genres.len(),     false, Some(RefreshCategory::AlbumGenres),    Some("genres")),
            ("Moods",     state.library.moods.len(),      false, Some(RefreshCategory::Moods),     None),
            ("Styles",    state.library.styles.len(),     false, Some(RefreshCategory::Styles),    None),
            ("Stations",  state.stations.len(),           false, Some(RefreshCategory::Stations),  Some("stations")),
            ("Aliases",   alias_count,                    false, None,                              None),
        ];

        // Header row.
        lines.push(Line::from(Span::styled(
            format!("  {:11}{:>9}  {:8}{:>10}", "Category", "Count", "Age", "Size"),
            Style::default().fg(t.colors.fg_secondary),
        )));

        for (label, count, is_loading, cat, breakdown_field) in row_specs {
            if *count == 0 && !*is_loading {
                continue;
            }
            let count_str = if *is_loading { "loading".to_string() } else { format_count(*count) };
            let age_str = cat
                .and_then(|c| state.cache_mgmt.category_timestamps.get(&c))
                .map(|&ts| {
                    let age = std::time::Duration::from_secs(now_ts.saturating_sub(ts));
                    format_duration(age)
                })
                .unwrap_or_default();
            let refreshing = cat.map_or(false, |c| state.cache_mgmt.background_refresh.contains(&c));
            let age_display = if refreshing { format!("{age_str}*") } else { age_str };
            let size_str = breakdown_field
                .and_then(|f| library_size(f))
                .map(format_bytes)
                .unwrap_or_else(|| "-".to_string());
            lines.push(Line::from(Span::styled(
                format!("  {:11}{:>9}  {:8}{:>10}", label, count_str, age_display, size_str),
                Style::default().fg(t.colors.fg_muted),
            )));
        }

        // Folders row — count is "{cached}/{total_root}", size from breakdown.
        let root_folder_count = state.folder_state.as_ref()
            .and_then(|fs| fs.columns.first())
            .map(|col| col.items.iter()
                .filter(|item| item.item_type == crate::services::FolderItemType::Folder)
                .count())
            .unwrap_or(0);
        let cached_listings = state.folder_contents_cache.len();
        if root_folder_count > 0 || cached_listings > 0 {
            let count_str = if cached_listings > 0 {
                format!("{}/{}", cached_listings, root_folder_count)
            } else {
                format_count(root_folder_count)
            };
            let age_str = if state.subfolder_preload_active { "crawl*" } else { "" };
            let size_str = library_size("folders").map(format_bytes).unwrap_or_else(|| "-".to_string());
            lines.push(Line::from(Span::styled(
                format!("  {:11}{:>9}  {:8}{:>10}", "Folders", count_str, age_str, size_str),
                Style::default().fg(t.colors.fg_muted),
            )));
        }

        // Artwork + Waveforms rows.
        if let Some((art_count, art_bytes)) = state.artwork.cache_stats {
            lines.push(Line::from(Span::styled(
                format!("  {:11}{:>9}  {:8}{:>10}", "Artwork", format_count(art_count), "", format_bytes(art_bytes)),
                Style::default().fg(t.colors.fg_muted),
            )));
        }
        if let Some((wf_count, wf_bytes)) = state.waveform_cache_stats {
            lines.push(Line::from(Span::styled(
                format!("  {:11}{:>9}  {:8}{:>10}", "Waveforms", format_count(wf_count), "", format_bytes(wf_bytes)),
                Style::default().fg(t.colors.fg_muted),
            )));
        }

        // Total on disk: library + artwork + waveforms.
        let mut total_bytes: u64 = 0;
        if let Some((lib_bytes, _)) = state.library_cache_stats { total_bytes += lib_bytes; }
        if let Some((_, art_bytes)) = state.artwork.cache_stats { total_bytes += art_bytes; }
        if let Some((_, wf_bytes)) = state.waveform_cache_stats { total_bytes += wf_bytes; }
        if total_bytes > 0 {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("  Total on disk: {}", format_bytes(total_bytes)),
                Style::default().fg(t.colors.fg_secondary),
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "  (signed out)",
            Style::default().fg(t.colors.fg_muted),
        )));
    }

    // Auto-scroll to keep selected item visible
    let total = lines.len() as u16;
    let visible = area.height;
    let scroll = if let Some(sel) = selected_line {
        let sel = sel as u16;
        if total <= visible { 0 }
        else { sel.saturating_sub(visible / 3).min(total.saturating_sub(visible)) }
    } else {
        0
    };

    let paragraph = Paragraph::new(lines).scroll((scroll, 0));
    frame.render_widget(paragraph, area);

    // Scrollbar when content overflows
    if total > visible {
        crate::ui::widgets::render_scrollbar(frame, outer, total as usize, visible as usize, scroll as usize, None);
    }
}

/// Render the sign-in form (username/password/sign in button/servers).
fn render_signin_form(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();
    let mut lines = vec![];
    let is_focused = state.settings_state.focus == SettingsFocus::Content;

    lines.push(Line::from(Span::styled(
        "sign in:",
        Style::default().fg(t.colors.fg_accent),
    )));

    // Username field (item index 0)
    let is_username_selected = is_focused && state.settings_state.item_index == 0;
    let is_username_editing = state.settings_state.editing_credential == Some(CredentialField::Username);
    let username_display = if is_username_editing {
        format!("{}▋", state.settings_state.username_input)
    } else if state.settings_state.username_input.is_empty() {
        "(enter username)".to_string()
    } else {
        state.settings_state.username_input.clone()
    };
    let username_prefix = "  ";
    let username_style = if is_username_editing {
        Style::default().fg(t.colors.fg_accent).add_modifier(ratatui::style::Modifier::BOLD)
    } else if is_username_selected {
        Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
    } else {
        Style::default().fg(t.colors.fg_primary)
    };
    lines.push(Line::from(Span::styled(
        format!("{}username: {}", username_prefix, username_display),
        username_style,
    )));

    // Password field (item index 1)
    let is_password_selected = is_focused && state.settings_state.item_index == 1;
    let is_password_editing = state.settings_state.editing_credential == Some(CredentialField::Password);
    let password_display = if is_password_editing {
        format!("{}▋", "•".repeat(state.settings_state.password_input.len()))
    } else if state.settings_state.password_input.is_empty() {
        "(enter password)".to_string()
    } else {
        "•".repeat(state.settings_state.password_input.len())
    };
    let password_prefix = "  ";
    let password_style = if is_password_editing {
        Style::default().fg(t.colors.fg_accent).add_modifier(ratatui::style::Modifier::BOLD)
    } else if is_password_selected {
        Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
    } else {
        Style::default().fg(t.colors.fg_primary)
    };
    lines.push(Line::from(Span::styled(
        format!("{}password: {}", password_prefix, password_display),
        password_style,
    )));

    // Sign In button (item index 2)
    let is_signin_selected = is_focused && state.settings_state.item_index == 2;
    let signin_prefix = "  ";
    let signin_style = if is_signin_selected { Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg) } else { Style::default().fg(t.colors.fg_primary) };
    let signin_text = if state.settings_state.discovering_servers { "signing in..." } else { "sign in" };
    lines.push(Line::from(Span::styled(
        format!("{}{}", signin_prefix, signin_text),
        signin_style,
    )));

    // Available servers
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "available servers:",
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
            let prefix = "  ";
            let style = if is_selected { Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg) } else { Style::default().fg(t.colors.fg_primary) };
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
            "type to enter | enter: done | esc: cancel",
            Style::default().fg(t.colors.fg_muted),
        )));
    } else if is_focused && state.settings_state.item_index <= 1 {
        lines.push(Line::from(Span::styled(
            "enter: edit field | esc: cancel sign-in",
            Style::default().fg(t.colors.fg_muted),
        )));
    } else if is_focused && state.settings_state.item_index == 2 {
        lines.push(Line::from(Span::styled(
            "enter: sign in (password is not stored) | esc: cancel",
            Style::default().fg(t.colors.fg_muted),
        )));
    } else if is_focused && state.settings_state.item_index >= 3 {
        lines.push(Line::from(Span::styled(
            "enter: connect to server | esc: cancel",
            Style::default().fg(t.colors.fg_muted),
        )));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

fn render_textamp_content(frame: &mut Frame, state: &AppState, outer: Rect, area: Rect) {
    use crate::app::state::OutputTarget;

    let t = theme();
    let is_focused = state.settings_state.focus == SettingsFocus::Content;
    let mut lines = vec![];
    let mut selected_line: Option<usize> = None;

    // Theme selection
    lines.push(Line::from(Span::styled(
        "theme:",
        Style::default().fg(t.colors.fg_accent),
    )));

    let theme_count = ThemeName::all().len();
    for (i, theme_name) in ThemeName::all().iter().enumerate() {
        let is_active = *theme_name == state.theme;
        let is_selected = is_focused && i == state.settings_state.item_index;
        if is_selected { selected_line = Some(lines.len()); }
        let prefix = if is_active { "  ♪ " } else { "  " };
        let style = if is_selected { Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg) } else { Style::default().fg(t.colors.fg_primary) };
        lines.push(Line::from(Span::styled(
            format!("{}{}", prefix, theme_name.display_name()),
            style,
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "enter: apply theme",
        Style::default().fg(t.colors.fg_muted),
    )));

    // Graphics info
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "graphics:",
        Style::default().fg(t.colors.fg_accent),
    )));
    let protocol = crate::ui::screens::now_playing::artwork_protocol_name();
    lines.push(Line::from(Span::styled(
        format!("  protocol: {} | terminal: {}x{}", protocol, state.terminal_width, state.terminal_height),
        Style::default().fg(t.colors.fg_muted),
    )));

    // Artwork mode selector
    lines.push(Line::from(Span::styled(
        "artwork:",
        Style::default().fg(t.colors.fg_accent),
    )));

    let artwork_modes = crate::app::state::ArtworkMode::all();
    for (i, mode) in artwork_modes.iter().enumerate() {
        let item_idx = theme_count + i;
        let is_active = *mode == state.artwork.mode;
        let is_selected = is_focused && state.settings_state.item_index == item_idx;
        if is_selected { selected_line = Some(lines.len()); }
        let prefix = if is_active { "  ♪ " } else { "  " };
        let style = if is_selected { Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg) } else { Style::default().fg(t.colors.fg_primary) };
        lines.push(Line::from(Span::styled(
            format!("{}{}", prefix, mode.name()),
            style,
        )));
    }

    // Playback output
    let output_offset = theme_count + artwork_modes.len();

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "playback output:",
        Style::default().fg(t.colors.fg_accent),
    )));

    // Local output
    let is_local = matches!(state.remote.output_target, OutputTarget::Local);
    let local_idx = output_offset;
    let is_selected = is_focused && state.settings_state.item_index == local_idx;
    if is_selected { selected_line = Some(lines.len()); }
    let prefix = if is_local { "  ♪ " } else { "  " };
    let style = if is_selected { Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg) } else { Style::default().fg(t.colors.fg_primary) };
    lines.push(Line::from(Span::styled(
        format!("{}local", prefix),
        style,
    )));

    // Remote players
    for (i, player) in state.remote.players.iter().enumerate() {
        let item_idx = output_offset + 1 + i;
        let is_active = match &state.remote.output_target {
            OutputTarget::Remote { player_id, .. } => *player_id == player.client_identifier,
            _ => false,
        };
        let is_selected = is_focused && item_idx == state.settings_state.item_index;
        if is_selected { selected_line = Some(lines.len()); }
        let prefix = if is_active { "  ♪ " } else { "  " };
        let style = if is_selected { Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg) } else { Style::default().fg(t.colors.fg_primary) };
        lines.push(Line::from(Span::styled(
            format!("{}{} ({})", prefix, player.name, player.product),
            style,
        )));
    }

    // Refresh players
    let refresh_idx = output_offset + 1 + state.remote.players.len();
    let is_selected = is_focused && refresh_idx == state.settings_state.item_index;
    if is_selected { selected_line = Some(lines.len() + 1); } // +1 for the blank line before
    let prefix = "  ";
    let style = if is_selected { Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg) } else { Style::default().fg(t.colors.fg_primary) };
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("{}refresh players", prefix),
        style,
    )));

    if state.remote.discovering {
        lines.push(Line::from(Span::styled(
            "  discovering...",
            Style::default().fg(t.colors.fg_muted),
        )));
    }

    // Transcode setting
    let transcode_offset = refresh_idx + 1;

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "streaming quality:",
        Style::default().fg(t.colors.fg_accent),
    )));

    let transcode_label = if state.transcode_kbps == 0 {
        "original (direct play)".to_string()
    } else {
        format!("transcode to {}kbps MP3", state.transcode_kbps)
    };
    let is_selected = is_focused && transcode_offset == state.settings_state.item_index;
    if is_selected { selected_line = Some(lines.len()); }
    let style = if is_selected { Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg) } else { Style::default().fg(t.colors.fg_primary) };
    lines.push(Line::from(Span::styled(
        format!("  {}", transcode_label),
        style,
    )));

    // External-services toggles. Three rows, indexed at
    // (transcode_offset + 1) + 0..=2. Each row toggles whether the
    // matching "Search ⟨service⟩" entry appears in the palette and
    // right-click context menus.
    let ext_base = transcode_offset + 1;
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "search in external services:",
        Style::default().fg(t.colors.fg_accent),
    )));
    let ext_entries: [(&str, bool); 3] = [
        ("Apple Music", state.external_search.apple_music),
        ("Spotify",     state.external_search.spotify),
        ("YouTube",     state.external_search.youtube),
    ];
    for (i, (label, on)) in ext_entries.iter().enumerate() {
        let item_idx = ext_base + i;
        let is_selected = is_focused && item_idx == state.settings_state.item_index;
        if is_selected { selected_line = Some(lines.len()); }
        let mark = if *on { "[x]" } else { "[ ]" };
        let style = if is_selected {
            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
        } else {
            Style::default().fg(t.colors.fg_primary)
        };
        lines.push(Line::from(Span::styled(
            format!("  {} {}", mark, label),
            style,
        )));
    }

    // Help text
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "enter: select | streaming quality saved to config",
        Style::default().fg(t.colors.fg_muted),
    )));

    // Auto-scroll to keep selected item visible
    let total = lines.len() as u16;
    let visible = area.height;
    let scroll = if let Some(sel) = selected_line {
        let sel = sel as u16;
        if total <= visible { 0 }
        else { sel.saturating_sub(visible / 3).min(total.saturating_sub(visible)) }
    } else {
        0
    };

    let paragraph = Paragraph::new(lines).scroll((scroll, 0));
    frame.render_widget(paragraph, area);

    // Scrollbar when content overflows
    if total > visible {
        crate::ui::widgets::render_scrollbar(frame, outer, total as usize, visible as usize, scroll as usize, None);
    }
}


/// Format a byte count as a human-readable string (matches the GUI's
/// settings table). KB/MB/GB use 1024 multiples like macOS Finder.
fn format_bytes(b: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let bf = b as f64;
    if bf >= GB {
        format!("{:.1} GB", bf / GB)
    } else if bf >= MB {
        format!("{:.1} MB", bf / MB)
    } else if bf >= KB {
        format!("{:.1} KB", bf / KB)
    } else {
        format!("{} B", b)
    }
}

/// Format a count with comma separators (e.g. 12345 → "12,345").
fn format_count(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
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

fn render_about_content(frame: &mut Frame, state: &AppState, outer: Rect, area: Rect) {
    let t = theme();

    let mut lines = parse_ansi_logo(t.colors.bg_primary);

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("version {}", env!("CARGO_PKG_VERSION")),
        Style::default().fg(t.colors.fg_primary),
    )));
    lines.push(Line::from(Span::styled(
        "a keyboard-driven TUI for Plex music",
        Style::default().fg(t.colors.fg_muted),
    )));
    lines.push(Line::from(Span::styled(
        "author: John Bergmayer | license: MIT",
        Style::default().fg(t.colors.fg_primary),
    )));
    lines.push(Line::from(Span::styled(
        "https://github.com/bergmayer/textamp",
        Style::default().fg(t.colors.fg_accent),
    )));

    // Graphics info
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "graphics:",
        Style::default().fg(t.colors.fg_accent),
    )));
    let protocol = crate::ui::screens::now_playing::artwork_protocol_name();
    lines.push(Line::from(Span::styled(
        format!("  protocol: {} | terminal: {}x{}", protocol, state.terminal_width, state.terminal_height),
        Style::default().fg(t.colors.fg_muted),
    )));

    // Manual scroll (no selectable items)
    let total = lines.len() as u16;
    let visible = area.height;
    let max_scroll = total.saturating_sub(visible);
    let scroll = state.settings_state.scroll.min(max_scroll);

    let paragraph = Paragraph::new(lines).scroll((scroll, 0));
    frame.render_widget(paragraph, area);

    // Scrollbar when content overflows
    if total > visible {
        crate::ui::widgets::render_scrollbar(frame, outer, total as usize, visible as usize, scroll as usize, None);
    }
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
