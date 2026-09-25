//! DOS-style setup: menu bar, framed pages, radio controls and contextual help.
use crate::app::state::{SettingsFocus, SettingsPage, SettingsSection, TextampSetting};
use crate::ui::theme::theme;
use crate::ui::RenderState as AppState;
use ratatui::prelude::*;
use ratatui::widgets::{
    Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};

pub(crate) fn paper() -> Style {
    let colors = theme().colors;
    Style::default().fg(colors.fg_primary).bg(colors.bg_primary)
}
pub(crate) fn selected() -> Style {
    Style::default()
        .fg(theme().colors.selection_bar_fg)
        .bg(theme().colors.selection_bar_bg)
        .add_modifier(Modifier::BOLD)
}
pub(crate) fn panel(title: impl Into<Line<'static>>) -> Block<'static> {
    Block::default()
        .title(title)
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(theme().colors.fg_accent))
        .style(paper())
}

pub(crate) fn scroll_marker(frame: &mut Frame, column: Rect, total: usize, offset: usize) {
    if total <= column.height as usize || column.is_empty() {
        return;
    }
    let mut state = ScrollbarState::new(total)
        .position(offset)
        .viewport_content_length(column.height as usize);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"))
            .style(paper().fg(theme().colors.fg_accent)),
        column,
        &mut state,
    );
}

pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    if area.is_empty() {
        return;
    }
    frame.render_widget(Block::default().style(paper()), area);
    let block = panel(" TEXTAMP  /  SETUP ");
    let inside = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(inside);
    let sections = SettingsSection::all();
    let slots = Layout::horizontal(
        sections
            .iter()
            .map(|_| Constraint::Ratio(1, sections.len() as u32)),
    )
    .split(rows[0]);
    for (i, section) in sections.iter().enumerate() {
        let active = *section == state.settings_state.section;
        let name = match section {
            SettingsSection::Libraries => "Libraries",
            SettingsSection::Textamp => "Textamp",
            SettingsSection::About => "About",
        };
        let label = format!(" {} {name} ", i + 1);
        frame.render_widget(
            Paragraph::new(label)
                .alignment(Alignment::Center)
                .style(if active {
                    if state.settings_state.focus == SettingsFocus::Sections {
                        selected()
                    } else {
                        paper()
                            .bg(theme().colors.bg_highlight)
                            .add_modifier(Modifier::BOLD)
                    }
                } else {
                    paper()
                }),
            slots[i],
        );
        state
            .hit_regions
            .borrow_mut()
            .settings_sections
            .push((slots[i], *section));
    }
    match state.settings_state.section {
        SettingsSection::Libraries => {
            super::super::app::render_library_manager(frame, state, rows[1])
        }
        SettingsSection::Textamp => render_textamp(frame, state, rows[1]),
        SettingsSection::About => render_about(frame, state, rows[1]),
    }
    frame.render_widget(
        Paragraph::new(if inside.width >= 75 {
            " F10 Menu  ←→ Menu  ↓ Enter panel  Enter Select  : Commands  Esc Back "
        } else {
            " ↓/Tab Panel  Enter Select  : Cmd  Esc Back "
        })
        .style(selected()),
        rows[2],
    );
}

fn render_textamp(frame: &mut Frame, state: &AppState, area: Rect) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    // A compact page selector works at 80 columns and keeps the controls in one list.
    let page = state.settings_state.page;
    let index = SettingsPage::ALL
        .iter()
        .position(|p| *p == page)
        .unwrap_or(0);
    let slots = Layout::horizontal(SettingsPage::ALL.iter().map(|_| Constraint::Ratio(1, 4)))
        .split(Rect {
            height: rows[0].height.min(1),
            ..rows[0]
        });
    for (i, p) in SettingsPage::ALL.iter().enumerate() {
        let short = if area.width < 60 {
            ["Theme", "Display", "Sidebar", "Search"][i]
        } else {
            ["Themes", "Display", "Sidebar", "Services"][i]
        };
        frame.render_widget(
            Paragraph::new(short)
                .alignment(Alignment::Center)
                .style(if i == index {
                    if state.settings_state.focus == SettingsFocus::Pages {
                        selected()
                    } else {
                        paper()
                            .bg(theme().colors.bg_highlight)
                            .add_modifier(Modifier::BOLD)
                    }
                } else {
                    paper()
                }),
            slots[i],
        );
        state
            .hit_regions
            .borrow_mut()
            .settings_pages
            .push((slots[i], *p));
    }
    let content = rows[1];
    let (controls, preview) = if content.width >= 74 {
        let panes = Layout::horizontal([Constraint::Length(38), Constraint::Min(0)]).split(content);
        (panes[0], Some(panes[1]))
    } else {
        (content, None)
    };
    let page_title = if page == SettingsPage::Display {
        let library = crate::app::sources::library_choices(state)
            .into_iter()
            .find(|choice| choice.active(state))
            .map_or_else(|| "No library".to_string(), |choice| choice.label());
        format!(" {} · {} ", page.name(), library)
    } else {
        format!(" {} ", page.name())
    };
    let block = panel(page_title);
    let inner = block.inner(controls);
    frame.render_widget(block, controls);
    let rows = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(if inner.height >= 9 { 4 } else { 0 }),
    ])
    .split(inner);
    let settings = state.textamp_settings();
    let focused = state.settings_state.focus == SettingsFocus::Content;
    let mut lines = Vec::new();
    let mut item_lines = Vec::new();
    let mut previous_heading = "";
    for (i, setting) in settings.iter().enumerate() {
        let (heading, label) = setting_label(state, *setting);
        if heading != previous_heading {
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            lines.push(Line::styled(heading, paper().add_modifier(Modifier::BOLD)));
            previous_heading = heading;
        }
        item_lines.push((lines.len(), i));
        lines.push(Line::styled(
            format!(" {label}"),
            if focused && i == state.settings_state.item_index {
                selected()
            } else {
                paper()
            },
        ));
    }
    let total = lines.len();
    let selected_line = item_lines
        .iter()
        .find(|(_, i)| *i == state.settings_state.item_index)
        .map_or(0, |(line, _)| *line);
    let offset = state
        .scroll
        .settings_textamp
        .unwrap_or_else(|| selected_line.saturating_sub(rows[0].height.saturating_sub(1) as usize))
        .min(total.saturating_sub(rows[0].height as usize));
    let hits = item_lines
        .into_iter()
        .filter_map(|(line, i)| {
            let y = line.checked_sub(offset)?;
            (y < rows[0].height as usize).then_some((
                Rect::new(rows[0].x, rows[0].y + y as u16, rows[0].width, 1),
                i,
            ))
        })
        .collect();
    state.hit_regions.borrow_mut().settings_textamp =
        Some(crate::app::presentation::SettingsContentRegion {
            inner: rows[0],
            rows: hits,
            scroll_offset: offset,
            total_lines: total,
        });
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((offset as u16, 0))
            .style(paper()),
        rows[0],
    );
    scroll_marker(
        frame,
        Rect::new(
            controls.right().saturating_sub(1),
            rows[0].y,
            1,
            rows[0].height,
        ),
        total,
        offset,
    );
    let help = match page {
        SettingsPage::Themes => "Highlight to preview. Enter applies and saves the theme.",
        SettingsPage::Display if state.active_library.is_none() => "Open a library to set its artwork rendering and default cover visibility.",
        SettingsPage::Display if settings.contains(&TextampSetting::Transcode) => "Artwork choices apply to this library. ( ) sets rendering; [X] shows covers in new columns. Streaming quality cycles bit rates.",
        SettingsPage::Display => "Artwork choices apply to this library. ( ) sets rendering; [X] shows covers in new album columns.",
        SettingsPage::Sidebar => "[X] appears in the browse sidebar. Space toggles the selected entry. Changes save immediately.",
        SettingsPage::Services => "Enable external search commands. Enter or Space toggles a service. Changes save immediately.",
    };
    frame.render_widget(
        Paragraph::new(help)
            .style(paper().fg(theme().colors.fg_accent))
            .wrap(Wrap { trim: true }),
        rows[1],
    );
    if let Some(preview) = preview {
        state.hit_regions.borrow_mut().settings_preview = Some(preview);
        let name = match settings.get(state.settings_state.item_index) {
            Some(TextampSetting::Theme(name)) if focused => *name,
            _ => state.theme,
        };
        let colors = crate::ui::theme::Theme::new(name).colors;
        let block = Block::default()
            .title(format!(" {} — preview ", name.display_name()))
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(colors.fg_accent))
            .style(Style::default().bg(colors.bg_primary));
        let inner = block.inner(preview);
        frame.render_widget(block, preview);
        crate::ui::widgets::logo::render_themed(frame, inner, &colors);
    }
}

fn setting_label(state: &AppState, setting: TextampSetting) -> (&'static str, String) {
    use crate::services::external_search::SearchTarget;
    let radio = |yes: bool| if yes { "(*)" } else { "( )" };
    let check = |yes: bool| if yes { "[X]" } else { "[ ]" };
    match setting {
        TextampSetting::Theme(name) => (
            "Color scheme",
            format!("{} {}", radio(name == state.theme), name.display_name()),
        ),
        TextampSetting::Artwork(mode) => (
            "This library's artwork",
            format!("{} {}", radio(mode == state.artwork.mode), mode.name()),
        ),
        TextampSetting::ArtworkVisible => (
            "This library's artwork",
            format!(
                "{} Show covers in new columns",
                check(state.artwork.default_visible)
            ),
        ),
        TextampSetting::Transcode => (
            "streaming quality",
            format!(
                "[ {} ]",
                if state.transcode_kbps == 0 {
                    "Original audio".into()
                } else {
                    format!("{} kbps", state.transcode_kbps)
                }
            ),
        ),
        TextampSetting::Sidebar(section) => (
            "Show in sidebar",
            format!("{} {}", check(!section.hidden(state)), section.label()),
        ),
        TextampSetting::ExternalSearch(target) => {
            let (label, enabled) = match target {
                SearchTarget::AppleMusic => ("Apple Music", state.external_search.apple_music),
                SearchTarget::Spotify => ("Spotify", state.external_search.spotify),
                SearchTarget::YouTube => ("YouTube", state.external_search.youtube),
            };
            ("Search commands", format!("{} {label}", check(enabled)))
        }
    }
}

fn render_about(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = crate::ui::theme::theme();
    let block = panel(" About Textamp ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines = crate::ui::widgets::logo::original(t.colors.bg_primary);
    lines.extend([
        Line::from(""),
        Line::from(format!("Textamp {}", env!("CARGO_PKG_VERSION"))),
        Line::from("A terminal player for Subsonic, Navidrome and music folders."),
        Line::from("John Bergmayer  |  Unlicense"),
        Line::from("https://github.com/bergmayer/textamp"),
        Line::from(""),
        Line::from(format!(
            "Graphics: {}  |  Terminal: {} x {}",
            crate::ui::screens::now_playing::artwork_protocol_name(),
            state.terminal_width,
            state.terminal_height
        )),
    ]);
    let scroll = state
        .settings_state
        .scroll
        .min(lines.len().saturating_sub(inner.height as usize) as u16);
    frame.render_widget(
        Paragraph::new(lines)
            .style(
                Style::default()
                    .fg(t.colors.fg_primary)
                    .bg(t.colors.bg_primary),
            )
            .scroll((scroll, 0)),
        inner,
    );
}
