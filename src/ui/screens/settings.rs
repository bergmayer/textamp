//! Settings: Libraries, Textamp and About.

use crate::app::state::{SettingsFocus, SettingsSection};
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Fill background
    frame.render_widget(
        Block::default().style(Style::default().bg(t.colors.bg_primary)),
        area,
    );

    // Split into left (sections) and right (content) panels
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(16), Constraint::Min(0)])
        .split(area);

    render_sections(frame, state, chunks[0]);
    render_content(frame, state, chunks[1]);
}

fn render_sections(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();
    let is_focused = state.settings_state.focus == SettingsFocus::Sections;
    let border_color = if is_focused {
        t.colors.border_focused
    } else {
        t.colors.border
    };

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
            let prefix = if is_selected && is_focused {
                "> "
            } else {
                "  "
            };
            let style = if is_selected && is_focused {
                Style::default()
                    .fg(t.colors.selection_text)
                    .bg(t.colors.selection_bar_bg)
            } else if is_selected {
                Style::default().fg(t.colors.fg_accent)
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
    let border_color = if is_focused {
        t.colors.border_focused
    } else {
        t.colors.border
    };

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
        SettingsSection::Libraries => {
            super::super::app::render_library_manager(frame, state, inner);
        }
        SettingsSection::Textamp => render_textamp_content(frame, state, area, inner),
        SettingsSection::About => render_about_content(frame, state, area, inner),
    }
}

/// Per-library disk usage (catalog and AudioMuse together), plus shared media.
fn render_textamp_content(frame: &mut Frame, state: &AppState, outer: Rect, area: Rect) {
    use crate::app::state::TextampSetting;
    use crate::services::external_search::SearchTarget;

    let t = theme();
    let is_focused = state.settings_state.focus == SettingsFocus::Content;
    let settings = state.textamp_settings();
    // Active-theme art remains visible. Only a focused theme row previews a change.
    let area = if area.width >= 60 {
        let panes = Layout::horizontal([Constraint::Length(38), Constraint::Min(0)]).split(area);
        let name = match settings.get(state.settings_state.item_index) {
            Some(TextampSetting::Theme(name)) if is_focused => *name,
            _ => state.theme,
        };
        let preview = crate::ui::theme::Theme::new(name);
        crate::ui::widgets::logo::render_themed(frame, panes[1], &preview.colors);
        panes[0]
    } else {
        area
    };
    let mut lines = vec![];
    let mut selected_line = None;
    let mut previous_heading = "";
    let mut item_lines = Vec::new();
    for (index, item) in settings.into_iter().enumerate() {
        let (heading, label, active) = match item {
            TextampSetting::Theme(name) => (
                "theme:",
                name.display_name().to_string(),
                name == state.theme,
            ),
            TextampSetting::Artwork(mode) => (
                "artwork:",
                mode.name().to_string(),
                mode == state.artwork.mode,
            ),

            TextampSetting::Transcode => (
                "streaming quality:",
                if state.transcode_kbps == 0 {
                    "original (direct play)".into()
                } else {
                    format!("transcode to {}kbps", state.transcode_kbps)
                },
                false,
            ),
            TextampSetting::Sidebar(section) => (
                "show in left column:",
                format!(
                    "{} {}",
                    if section.hidden(state) { "[ ]" } else { "[x]" },
                    section.label()
                ),
                false,
            ),
            TextampSetting::ExternalSearch(target) => {
                let (label, enabled) = match target {
                    SearchTarget::AppleMusic => ("Apple Music", state.external_search.apple_music),
                    SearchTarget::Spotify => ("Spotify", state.external_search.spotify),
                    SearchTarget::YouTube => ("YouTube", state.external_search.youtube),
                };
                (
                    "search in external services:",
                    format!("{} {label}", if enabled { "[x]" } else { "[ ]" }),
                    false,
                )
            }
        };
        if heading != previous_heading {
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(
                heading,
                Style::default().fg(t.colors.fg_accent),
            )));
            previous_heading = heading;
        }
        let selected = is_focused && index == state.settings_state.item_index;
        item_lines.push((lines.len(), index));
        if selected {
            selected_line = Some(lines.len());
        }
        let style = if selected {
            Style::default()
                .fg(t.colors.selection_text)
                .bg(t.colors.selection_bar_bg)
        } else {
            Style::default().fg(t.colors.fg_primary)
        };
        lines.push(Line::from(Span::styled(
            format!("  {}{label}", if active { "♪ " } else { "" }),
            style,
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "graphics: {} | terminal: {}x{}",
            crate::ui::screens::now_playing::artwork_protocol_name(),
            state.terminal_width,
            state.terminal_height
        ),
        Style::default().fg(t.colors.fg_muted),
    )));
    lines.push(Line::from(Span::styled(
        "enter: select",
        Style::default().fg(t.colors.fg_muted),
    )));

    // Auto-scroll to keep selected item visible
    let total = lines.len() as u16;
    let visible = area.height;
    let scroll = if let Some(sel) = selected_line {
        let sel = sel as u16;
        if total <= visible {
            0
        } else {
            sel.saturating_sub(visible / 3)
                .min(total.saturating_sub(visible))
        }
    } else {
        0
    };

    let scroll = state
        .scroll
        .settings_textamp
        .unwrap_or(scroll as usize)
        .min(total.saturating_sub(visible) as usize);
    let rows = item_lines
        .into_iter()
        .filter_map(|(line, index)| {
            let row = line.checked_sub(scroll)?;
            (row < area.height as usize)
                .then_some((Rect::new(area.x, area.y + row as u16, area.width, 1), index))
        })
        .collect();
    state.hit_regions.borrow_mut().settings_textamp =
        Some(crate::app::presentation::SettingsContentRegion {
            inner: area,
            rows,
            scroll_offset: scroll,
            total_lines: total as usize,
        });

    let paragraph = Paragraph::new(lines).scroll((scroll as u16, 0));
    frame.render_widget(paragraph, area);

    // Scrollbar when content overflows
    if total > visible {
        crate::ui::widgets::render_scrollbar(
            frame,
            outer,
            total as usize,
            visible as usize,
            scroll,
            None,
        );
    }
}

fn render_about_content(frame: &mut Frame, state: &AppState, outer: Rect, area: Rect) {
    let t = theme();

    let mut lines = crate::ui::widgets::logo::original(t.colors.bg_primary);

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("version {}", env!("CARGO_PKG_VERSION")),
        Style::default().fg(t.colors.fg_primary),
    )));
    lines.push(Line::from(Span::styled(
        "a terminal player for Subsonic, Navidrome and music folders",
        Style::default().fg(t.colors.fg_muted),
    )));
    lines.push(Line::from(Span::styled(
        "author: John Bergmayer | license: Unlicense",
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
        format!(
            "  protocol: {} | terminal: {}x{}",
            protocol, state.terminal_width, state.terminal_height
        ),
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
        crate::ui::widgets::render_scrollbar(
            frame,
            outer,
            total as usize,
            visible as usize,
            scroll as usize,
            None,
        );
    }
}

use crate::ui::RenderState as AppState;
