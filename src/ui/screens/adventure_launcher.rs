//! One Journey form: endpoint search, length, preview and playback.
use crate::app::{
    journey::{self, JourneyControl as Control},
    state::*,
};
use crate::ui::{RenderState, theme::theme};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

/// Render a stable, responsive form for Journey and seeded radio stations.
pub fn render(frame: &mut Frame, state: &RenderState<'_>, area: Rect) {
    render_in(frame, state, area, false);
}
pub fn render_inline(frame: &mut Frame, state: &RenderState<'_>, area: Rect) {
    render_in(frame, state, area, true);
}
fn render_in(frame: &mut Frame, state: &RenderState<'_>, area: Rect, inline: bool) {
    let Some(launcher) = state.popups.adventure_launcher() else {
        return;
    };
    let radio = launcher.purpose != TrackPickerPurpose::Journey;
    let colors = &theme().colors;
    let width = area.width.saturating_sub(4).clamp(1, 88).min(area.width);
    let height = area.height.saturating_sub(2).clamp(1, 26).min(area.height);
    let popup = if inline {
        area
    } else {
        Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        )
    };
    if !inline {
        frame.render_widget(Clear, popup);
        crate::ui::widgets::register_popup_close(state, popup);
    }
    let title = if radio {
        " Choose starting track "
    } else {
        " Journey "
    };
    let block = Block::default()
        .title(if inline { "" } else { title })
        .borders(if inline { Borders::NONE } else { Borders::ALL })
        .border_style(Style::default().fg(colors.fg_accent))
        .style(Style::default().fg(colors.fg_primary).bg(colors.bg_primary));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let field_height = if inner.height >= 18 { 3 } else { 1 };
    let mut y = inner.y;
    let mut controls = Vec::new();
    for (control, label, step, track) in [
        (
            Control::Start,
            journey::START_TRACK,
            AdventureStep::FindStartTrack,
            &launcher.start_track,
        ),
        (
            Control::End,
            journey::END_TRACK,
            AdventureStep::FindEndTrack,
            &launcher.end_track,
        ),
    ]
    .into_iter()
    .filter(|(control, _, _, _)| !radio || *control == Control::Start)
    {
        if y >= inner.bottom().saturating_sub(4) {
            break;
        }
        let rect = Rect::new(
            inner.x + 1,
            y,
            inner.width.saturating_sub(2),
            field_height.min(inner.bottom() - y),
        );
        let focused = launcher.terminal_focus == control;
        let editing = launcher.step == step && !launcher.query.is_empty();
        let value = if editing {
            launcher.query.as_str()
        } else {
            track
                .as_ref()
                .map_or("Search tracks, artists or albums…", |track| {
                    track.title.as_str()
                })
        };
        let summary = track
            .as_ref()
            .filter(|_| !editing)
            .map(|track| format!("{} · {}", track.title, track.track_artist()));
        let value = summary.as_deref().unwrap_or(value);
        let value = crate::util::truncate_middle(
            value,
            rect.width
                .saturating_sub(if field_height == 1 { 15 } else { 4 }) as usize,
        );
        let border = Style::default().fg(if focused {
            colors.fg_accent
        } else {
            colors.border
        });
        let text = value;
        let input_area;
        if field_height > 1 {
            let block = Block::default()
                .title(format!(" {label} "))
                .borders(Borders::ALL)
                .border_style(border);
            let content = block.inner(rect);
            input_area = content;
            frame.render_widget(block, rect);
            frame.render_widget(Paragraph::new(text), content);
        } else {
            let prefix = unicode_width::UnicodeWidthStr::width(label).saturating_add(2) as u16;
            input_area = Rect::new(
                rect.x + prefix.min(rect.width),
                rect.y,
                rect.width.saturating_sub(prefix),
                1,
            );
            frame.render_widget(
                Paragraph::new(format!("{label}: {text}")).style(if focused {
                    Style::default().fg(colors.fg_accent)
                } else {
                    Style::default().fg(colors.fg_primary)
                }),
                rect,
            );
        }
        crate::ui::widgets::text_input::render(
            frame,
            state,
            input_area,
            crate::app::terminal_text::Target::Journey(if control == Control::Start {
                crate::app::terminal_text::JourneyField::Start
            } else {
                crate::app::terminal_text::JourneyField::End
            }),
            Style::default().fg(colors.fg_primary).bg(colors.bg_primary),
        );
        controls.push((rect, control));
        y += field_height;
    }
    if !radio && y < inner.bottom().saturating_sub(3) {
        let rect = Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), 1);
        frame.render_widget(
            Paragraph::new(format!(
                "{}: {}",
                journey::MAXIMUM_TRACKS,
                launcher.track_count_input
            ))
            .style(Style::default().fg(
                if launcher.terminal_focus == Control::Count {
                    colors.fg_accent
                } else {
                    colors.fg_primary
                },
            )),
            rect,
        );
        let prefix =
            unicode_width::UnicodeWidthStr::width(journey::MAXIMUM_TRACKS).saturating_add(2) as u16;
        let input = Rect::new(
            rect.x + prefix.min(rect.width),
            rect.y,
            rect.width.saturating_sub(prefix).min(4),
            1,
        );
        crate::ui::widgets::text_input::render(
            frame,
            state,
            input,
            crate::app::terminal_text::Target::Journey(
                crate::app::terminal_text::JourneyField::Count,
            ),
            Style::default().fg(colors.fg_primary).bg(colors.bg_primary),
        );
        controls.push((rect, Control::Count));
        y += 1;
    }
    let tabs_area = Rect::new(
        inner.x + 1,
        y,
        inner.width.saturating_sub(2),
        u16::from(y < inner.bottom()),
    );
    let mut tabs = Vec::new();
    let mut x = tabs_area.x;
    for (label, tab) in [
        ("All", SearchTab::Global),
        ("Artists", SearchTab::Artists),
        ("Albums", SearchTab::Albums),
        ("Tracks", SearchTab::Tracks),
    ] {
        let rect = Rect::new(
            x,
            y,
            (label.len() as u16 + 2).min(tabs_area.right().saturating_sub(x)),
            tabs_area.height,
        );
        let selected = launcher.search_tab == tab;
        frame.render_widget(
            Paragraph::new(format!(" {label} ")).style(if selected {
                Style::default()
                    .fg(colors.selection_text)
                    .bg(colors.bg_selection)
            } else {
                Style::default().fg(colors.fg_muted)
            }),
            rect,
        );
        tabs.push((rect, tab));
        x += rect.width;
    }
    controls.push((tabs_area, Control::Types));
    y += 1;
    let result_area = Rect::new(
        inner.x + 1,
        y,
        inner.width.saturating_sub(2),
        inner.bottom().saturating_sub(y + 2),
    );
    controls.push((result_area, Control::Results));
    let mut rows = Vec::new();
    match &launcher.stage {
        JourneyStage::Generating => frame.render_widget(
            Paragraph::new("Building your journey…").style(Style::default().fg(colors.fg_accent)),
            result_area,
        ),
        JourneyStage::Failed(error) => frame.render_widget(
            Paragraph::new(error.as_str()).wrap(Wrap { trim: true }),
            result_area,
        ),
        stage => {
            let preview = if let JourneyStage::Ready(tracks) = stage {
                Some(tracks)
            } else {
                None
            };
            let choices = journey::choices(launcher);
            let count = preview.map_or(choices.len(), Vec::len);
            let two_rows = result_area.height >= 8;
            let row_height = if two_rows { 2 } else { 1 };
            let visible = (result_area.height / row_height) as usize;
            let offset = launcher
                .scroll_pin
                .unwrap_or_else(|| {
                    crate::services::NavigationService::calc_scroll_offset(
                        launcher.item_index,
                        visible,
                        count,
                    )
                })
                .min(count.saturating_sub(visible));
            let items: Vec<_> = (offset..count.min(offset + visible))
                .map(|index| {
                    let (title, subtitle) = if let Some(tracks) = preview {
                        (&*tracks[index].title, tracks[index].track_artist())
                    } else {
                        (choices[index].title(), choices[index].subtitle())
                    };
                    let selected =
                        launcher.terminal_focus == Control::Results && launcher.item_index == index;
                    let style = if selected {
                        Style::default()
                            .fg(colors.selection_text)
                            .bg(colors.bg_selection)
                    } else {
                        Style::default().fg(colors.fg_primary)
                    };
                    let title = crate::util::truncate_middle(
                        title,
                        result_area.width.saturating_sub(2) as usize,
                    );
                    let lines = if two_rows {
                        vec![
                            Line::from(format!(" {title}")),
                            Line::from(format!(
                                " {}",
                                crate::util::truncate_middle(
                                    subtitle,
                                    result_area.width.saturating_sub(2) as usize
                                )
                            )),
                        ]
                    } else {
                        vec![Line::from(format!(" {title}"))]
                    };
                    rows.push((
                        Rect::new(
                            result_area.x,
                            result_area.y + (index - offset) as u16 * row_height,
                            result_area.width,
                            row_height,
                        ),
                        index,
                    ));
                    ListItem::new(lines).style(style)
                })
                .collect();
            frame.render_widget(List::new(items), result_area);
            if count == 0 {
                let message = if launcher.loading {
                    "Searching…"
                } else if launcher.start_track.is_some() && launcher.end_track.is_some() {
                    "Ctrl+Enter previews the route."
                } else if launcher.query.is_empty() {
                    "Search either endpoint to choose a track."
                } else {
                    "No matches"
                };
                frame.render_widget(
                    Paragraph::new(message).style(Style::default().fg(colors.fg_muted)),
                    result_area,
                );
            } else if count > visible {
                crate::ui::widgets::render_scrollbar_borderless(
                    frame,
                    result_area,
                    count,
                    visible,
                    offset,
                );
            }
        }
    }
    let button_y = inner.bottom().saturating_sub(1);
    let primary = match launcher.stage {
        JourneyStage::Ready(_) => journey::PLAY,
        JourneyStage::Failed(_) => "Retry",
        _ if radio => {
            if inner.width < 60 {
                "Start"
            } else {
                "Start Radio"
            }
        }
        _ => {
            if inner.width < 60 {
                "Preview"
            } else {
                journey::PREVIEW
            }
        }
    };
    let buttons = if radio {
        vec![(Control::Cancel, "Cancel"), (Control::Preview, primary)]
    } else {
        vec![
            (
                Control::Swap,
                if inner.width < 60 {
                    "Swap"
                } else {
                    journey::SWAP
                },
            ),
            (Control::Cancel, "Cancel"),
            (Control::Preview, primary),
        ]
    };
    let total: u16 = buttons
        .iter()
        .map(|(_, label)| label.len() as u16 + 4)
        .sum();
    let mut x = inner.right().saturating_sub(total.min(inner.width));
    for (control, label) in buttons {
        let rect = Rect::new(
            x,
            button_y,
            (label.len() as u16 + 4).min(inner.right().saturating_sub(x)),
            u16::from(inner.height > 0),
        );
        frame.render_widget(
            Paragraph::new(format!("[ {label} ]")).style(if launcher.terminal_focus == control {
                Style::default()
                    .fg(colors.selection_text)
                    .bg(colors.bg_selection)
            } else {
                Style::default().fg(if control == Control::Preview {
                    colors.fg_accent
                } else {
                    colors.fg_primary
                })
            }),
            rect,
        );
        controls.push((rect, control));
        x += rect.width;
    }
    state.hit_regions.borrow_mut().adventure_launcher =
        Some(crate::app::presentation::AdventureLauncherRegions {
            outer: popup,
            results: result_area,
            controls,
            tabs,
            rows,
        });
}
