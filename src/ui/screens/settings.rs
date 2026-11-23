//! Two-pane settings: flat categories, focused controls, and contextual actions.
use crate::app::state::{SettingsFocus, SettingsPage, SettingsSection, TextampSetting};
use crate::ui::RenderState as AppState;
use crate::ui::theme::theme;
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
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
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
    let block = panel(" Settings ");
    let inside = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(inside);
    // Wide terminals show both panes. Narrow terminals show the category list
    // or the focused page at full width; Tab and Esc use the same focus model.
    let navigation = state.settings_state.focus == SettingsFocus::Sections;
    let content = if area.width >= 65 {
        let panes = Layout::horizontal([Constraint::Length(17), Constraint::Min(0)]).split(rows[0]);
        render_categories(frame, state, panes[0]);
        Some(panes[1])
    } else if navigation {
        render_categories(frame, state, rows[0]);
        None
    } else {
        Some(rows[0])
    };
    if let Some(content) = content {
        match state.settings_state.section {
            SettingsSection::Libraries => {
                super::super::app::render_library_manager(frame, state, content)
            }
            SettingsSection::Textamp => render_textamp(frame, state, content),
            SettingsSection::About => render_about(frame, state, content),
        }
    }
    let navigation = state.settings_state.focus == SettingsFocus::Sections;
    let help = if inside.width >= 72 {
        if navigation {
            "↑↓ Category  Enter Open  Tab Controls  : Commands"
        } else {
            "↑↓ Move  Enter Select  Tab Categories  : Commands"
        }
    } else if inside.width >= 48 {
        if navigation {
            "↑↓ Category  Enter Open  Tab Pane"
        } else {
            "↑↓ Move  Enter Select  Tab Pane"
        }
    } else if navigation {
        "↑↓ Move  → Open"
    } else {
        "↑↓ Move  Enter Select"
    };
    let footer = Layout::horizontal([Constraint::Min(0), Constraint::Length(10)]).split(rows[1]);
    frame.render_widget(
        Paragraph::new(help).style(paper().fg(theme().colors.fg_muted)),
        footer[0],
    );
    frame.render_widget(
        Paragraph::new(if navigation { "Esc Close" } else { "Esc Back" })
            .alignment(Alignment::Right)
            .style(paper().fg(theme().colors.fg_accent)),
        footer[1],
    );
    state.hit_regions.borrow_mut().settings_back = Some(footer[1]);
}

fn render_categories(frame: &mut Frame, state: &AppState, area: Rect) {
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(paper().fg(theme().colors.border));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    state.hit_regions.borrow_mut().settings_categories = Some(inner);
    let index = state.settings_state.category_index();
    let offset = index.saturating_sub(inner.height.saturating_sub(1) as usize);
    let labels: Vec<_> = std::iter::once("Libraries")
        .chain(SettingsPage::ALL.into_iter().map(SettingsPage::name))
        .chain(std::iter::once("About"))
        .collect();
    for (i, label) in labels
        .iter()
        .enumerate()
        .skip(offset)
        .take(inner.height as usize)
    {
        let active = index == i;
        let focus = state.settings_state.focus == SettingsFocus::Sections;
        let style = if active && focus {
            selected()
        } else if active {
            theme().colors.inactive_row().add_modifier(Modifier::BOLD)
        } else {
            paper()
        };
        let rect = Rect::new(inner.x, inner.y + (i - offset) as u16, inner.width, 1);
        frame.render_widget(
            Paragraph::new(format!("{}{label}", if active { "› " } else { "  " })).style(style),
            rect,
        );
        let mut hits = state.hit_regions.borrow_mut();
        match i {
            0 => hits
                .settings_sections
                .push((rect, SettingsSection::Libraries)),
            i if i == labels.len() - 1 => {
                hits.settings_sections.push((rect, SettingsSection::About))
            }
            _ => hits.settings_pages.push((rect, SettingsPage::ALL[i - 1])),
        }
    }
    scroll_marker(
        frame,
        Rect::new(area.right().saturating_sub(1), inner.y, 1, inner.height),
        labels.len(),
        offset,
    );
}

fn render_textamp(frame: &mut Frame, state: &AppState, area: Rect) {
    let page = state.settings_state.page;
    let focused = state.settings_state.focus == SettingsFocus::Content;
    let block = panel(format!(" {} ", page.name()))
        .borders(Borders::TOP)
        .border_style(paper().fg(if focused {
            theme().colors.border_focused
        } else {
            theme().colors.border
        }));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let settings = state.textamp_settings();
    let subtitle = match page {
        SettingsPage::Themes => crate::app::sources::library_choices(state)
            .into_iter()
            .find(|c| c.active(state))
            .map_or_else(
                || "Theme and artwork rendering".into(),
                |c| format!("Library: {}", c.label()),
            ),
        SettingsPage::Audio => "Playback and streaming quality".into(),
        SettingsPage::Sidebar => "Visible in your library sidebar".into(),
        SettingsPage::Services => "Search links in the command palette".into(),
    };
    let help_height = if inner.height >= 12 {
        3
    } else if inner.height >= 7 {
        2
    } else {
        0
    };
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(help_height),
        Constraint::Length(u16::from(inner.height >= 5)),
    ])
    .split(inner);
    frame.render_widget(
        Paragraph::new(subtitle).style(paper().fg(theme().colors.fg_muted)),
        rows[0],
    );
    let (controls, preview) =
        if page == SettingsPage::Themes && rows[1].width >= 54 && rows[1].height >= 8 {
            let panes =
                Layout::horizontal([Constraint::Length(25), Constraint::Min(0)]).split(rows[1]);
            (panes[0], Some(panes[1]))
        } else {
            (rows[1], None)
        };
    let definitions = crate::app::preferences::page(state, page);
    let mut lines = Vec::new();
    let mut item_lines = Vec::new();
    let mut previous = "";
    for (i, definition) in definitions.iter().enumerate() {
        let heading = definition.group;
        let label = match &definition.value {
            crate::app::preferences::Value::Toggle { current, .. } => format!(
                "{} {}",
                if *current { "[x]" } else { "[ ]" },
                definition.label
            ),
            crate::app::preferences::Value::Choice { selected, .. } => format!(
                "{} {}",
                if *selected { "(•)" } else { "( )" },
                definition.label
            ),
            _ => format!("{}: {}", definition.label, definition.value.display()),
        };
        if heading != previous {
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            lines.push(Line::styled(heading, paper().fg(theme().colors.fg_accent)));
            previous = heading;
        }
        item_lines.push((lines.len(), i));
        let active = i == state.settings_state.item_index;
        lines.push(Line::styled(
            format!("{}{label}", if active && focused { "› " } else { "  " }),
            if active && focused {
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
        .unwrap_or_else(|| selected_line.saturating_sub(controls.height.saturating_sub(1) as usize))
        .min(total.saturating_sub(controls.height as usize));
    let hits = item_lines
        .into_iter()
        .filter_map(|(line, i)| {
            let y = line.checked_sub(offset)?;
            (y < controls.height as usize).then_some((
                Rect::new(controls.x, controls.y + y as u16, controls.width, 1),
                i,
            ))
        })
        .collect();
    state.hit_regions.borrow_mut().settings_textamp =
        Some(crate::app::presentation::SettingsContentRegion {
            inner: controls,
            rows: hits,
            scroll_offset: offset,
            total_lines: total,
        });
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((offset as u16, 0))
            .style(paper()),
        controls,
    );
    scroll_marker(
        frame,
        Rect::new(
            controls.right().saturating_sub(1),
            controls.y,
            1,
            controls.height,
        ),
        total,
        offset,
    );
    let help = definitions.get(state.settings_state.item_index).map_or(
        "Open a library to configure its preferences.",
        |definition| definition.description,
    );

    frame.render_widget(
        Paragraph::new(help)
            .wrap(Wrap { trim: true })
            .style(paper().fg(theme().colors.fg_muted)),
        rows[2],
    );
    if page == SettingsPage::Themes {
        frame.render_widget(
            Paragraph::new("[ Apply theme ]").style(paper().fg(theme().colors.fg_accent)),
            rows[3],
        );
        if !rows[3].is_empty() {
            state.hit_regions.borrow_mut().settings_apply = Some(rows[3]);
        }
    } else if !settings.is_empty() {
        frame.render_widget(
            Paragraph::new("Changes save automatically").style(paper().fg(theme().colors.fg_muted)),
            rows[3],
        );
    }
    if let Some(preview) = preview {
        render_preview(frame, state, preview, focused, &settings);
    }
}

fn render_preview(
    frame: &mut Frame,
    state: &AppState,
    area: Rect,
    focused: bool,
    settings: &[TextampSetting],
) {
    let name = match settings.get(state.settings_state.item_index) {
        Some(TextampSetting::Theme(name)) if focused => *name,
        _ => state.theme,
    };
    let colors = crate::ui::theme::Theme::new(name).colors;
    let base = Style::default().fg(colors.fg_primary).bg(colors.bg_primary);
    let block = Block::default()
        .title(" Preview ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(base.fg(colors.fg_accent))
        .style(base);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    state.hit_regions.borrow_mut().settings_preview = Some(area);
    let lines = vec![
        Line::styled(name.display_name(), base.add_modifier(Modifier::BOLD)),
        Line::from(""),
        Line::styled(" Library", base.fg(colors.fg_accent)),
        Line::styled(
            format!(" › {}", crate::app::preferences::THEME_PREVIEW_ALBUM),
            Style::default()
                .fg(colors.selection_bar_fg)
                .bg(colors.selection_bar_bg),
        ),
        Line::styled(
            format!(
                "   {:<17} {}",
                crate::app::preferences::THEME_PREVIEW_TRACKS[0].0,
                crate::app::preferences::THEME_PREVIEW_TRACKS[0].1
            ),
            base,
        ),
        Line::styled(
            format!(
                "   {:<17} {}",
                crate::app::preferences::THEME_PREVIEW_TRACKS[1].0,
                crate::app::preferences::THEME_PREVIEW_TRACKS[1].1
            ),
            base,
        ),
        Line::from(""),
        Line::styled(" Playing ━━━━━───  2:10", base.fg(colors.fg_accent)),
        Line::styled(" Enter Play   Esc Back", base.fg(colors.fg_muted)),
    ];
    frame.render_widget(Paragraph::new(lines).style(base), inner);
}

fn render_about(frame: &mut Frame, state: &AppState, area: Rect) {
    let block = panel(" About ").borders(Borders::TOP);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length((inner.height / 2).clamp(3, 14).min(inner.height)),
        Constraint::Min(0),
    ])
    .split(inner);
    if let Some(text) = &state.settings_state.activity_history {
        let paragraph = Paragraph::new(text.as_ref()).wrap(Wrap { trim: false });
        let lines = paragraph.line_count(inner.width);
        state.hit_regions.borrow_mut().settings_about =
            Some(crate::app::presentation::ScrollableTextRegion {
                area: inner,
                lines,
                visible: inner.height as usize,
            });
        frame.render_widget(paragraph.scroll((state.settings_state.scroll, 0)), inner);
        return;
    }
    state.hit_regions.borrow_mut().settings_apply = Some(rows[0]);
    crate::ui::widgets::logo::render_original(frame, rows[0], &theme().colors);
    let inner = rows[1];
    let lines = vec![
        Line::styled(
            format!("Textamp {}", env!("CARGO_PKG_VERSION")),
            paper().add_modifier(Modifier::BOLD),
        ),
        Line::from(""),
        Line::from("John Bergmayer · Textamp source: Unlicense"),
        Line::from("github.com/bergmayer/textamp"),
        Line::from(""),
        Line::from(format!(
            "Graphics: {}",
            crate::ui::screens::now_playing::artwork_protocol_name()
        )),
        Line::from(format!(
            "Terminal: {} × {}",
            state.terminal_width, state.terminal_height
        )),
    ];
    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: true })
        .style(paper());
    let total = paragraph.line_count(inner.width);
    let offset =
        (state.settings_state.scroll as usize).min(total.saturating_sub(inner.height as usize));
    state.hit_regions.borrow_mut().settings_about =
        Some(crate::app::presentation::ScrollableTextRegion {
            area: inner,
            lines: total,
            visible: inner.height as usize,
        });
    frame.render_widget(paragraph.scroll((offset as u16, 0)), inner);
    scroll_marker(
        frame,
        Rect::new(area.right().saturating_sub(1), inner.y, 1, inner.height),
        total,
        offset,
    );
}
