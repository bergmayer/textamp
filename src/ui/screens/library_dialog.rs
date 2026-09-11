//! Conventional library forms and account buttons, rendered from draft state.
use crate::app::sources::dialogs::{Dialog, WebdavForm};
use crate::ui::{theme::theme, RenderState};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

pub fn render(frame: &mut Frame, state: &RenderState) {
    let Some(dialog) = &state.popups.library_dialog else {
        return;
    };
    let t = theme();
    let width = frame.area().width.saturating_sub(4).min(78);
    let height = frame.area().height.saturating_sub(2).min(match dialog {
        Dialog::Library(_) => 24,
        Dialog::Webdav(_) | Dialog::AudioMuse(_) | Dialog::Navidrome(_) => 19,
        _ => 13,
    });
    let area = Rect::new(
        (frame.area().width - width) / 2,
        (frame.area().height - height) / 2,
        width,
        height,
    );
    let title = match dialog {
        Dialog::Library(_) => " Library options ",
        Dialog::Add { .. } => " Add library ",
        Dialog::Navidrome(_) => " Subsonic / Navidrome connection ",
        Dialog::AudioMuse(_) => " AudioMuse connection ",
        Dialog::Webdav(form) if form.editing => " Edit WebDAV library ",
        Dialog::Webdav(_) => " Add WebDAV library ",
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border_focused))
        .style(
            Style::default()
                .bg(t.colors.bg_primary)
                .fg(t.colors.fg_primary),
        );
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let mut controls = Vec::new();
    match dialog {
        Dialog::Library(options) => {
            let size = state
                .settings_state
                .cache_entries
                .iter()
                .find(|(c, _)| {
                    c.cache_store()
                        .ok()
                        .zip(options.choice.cache_store().ok())
                        .is_some_and(|(a, b)| a.same_scope(&b))
                })
                .map(|(_, size)| match size {
                    Ok(bytes) => crate::util::format_bytes(*bytes),
                    Err(e) => format!("Unavailable: {e}"),
                })
                .unwrap_or_else(|| "Measuring…".into());
            let scan = crate::app::sources::cache::status(&options.choice, state);
            let heading = format!(
                "{}\n{}\nCache: {size} · weekly + manual\n{scan}",
                options.choice.label(),
                options.choice.location()
            );
            let rows = Layout::vertical([
                Constraint::Length(5),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(inner);
            frame.render_widget(
                Paragraph::new(heading).style(Style::default().fg(t.colors.fg_primary)),
                rows[0],
            );
            let items = options.items(state);
            let visible = rows[1].height as usize;
            let offset = options.selected.saturating_sub(visible.saturating_sub(1));
            controls.resize(items.len(), Rect::default());
            for (index, (_, label)) in items.iter().enumerate().skip(offset).take(visible) {
                let rect = Rect::new(
                    rows[1].x,
                    rows[1].y + (index - offset) as u16,
                    rows[1].width,
                    1,
                );
                let style = if index == options.selected {
                    Style::default()
                        .fg(t.colors.selection_text)
                        .bg(t.colors.selection_bar_bg)
                } else {
                    Style::default().fg(t.colors.fg_primary)
                };
                frame.render_widget(Paragraph::new(format!("[ {label} ]")).style(style), rect);
                controls[index] = rect;
            }
            frame.render_widget(
                Paragraph::new("↑↓ options · Enter select · Esc close")
                    .style(Style::default().fg(t.colors.fg_muted)),
                rows[2],
            );
        }
        Dialog::Add { selected } => {
            for (index, label) in ["Local folder", "WebDAV share", "Subsonic / Navidrome"]
                .iter()
                .enumerate()
            {
                if index as u16 >= inner.height {
                    break;
                }
                let rect = Rect::new(inner.x, inner.y + index as u16, inner.width, 1);
                let style = if index == *selected {
                    Style::default()
                        .fg(t.colors.selection_text)
                        .bg(t.colors.selection_bar_bg)
                } else {
                    Style::default().fg(t.colors.fg_primary)
                };
                frame.render_widget(Paragraph::new(format!("[ {label} ]")).style(style), rect);
                controls.push(rect);
            }
        }
        Dialog::Webdav(_) | Dialog::AudioMuse(_) | Dialog::Navidrome(_) => {
            let (fields, labels, focus, busy, error, editing, audiomuse) = match dialog {
                Dialog::Navidrome(form) => (
                    &form.fields,
                    WebdavForm::LABELS,
                    form.focus,
                    form.busy,
                    form.error.as_deref(),
                    form.editing,
                    false,
                ),
                Dialog::Webdav(form) => (
                    &form.fields,
                    WebdavForm::LABELS,
                    form.focus,
                    form.task.is_some(),
                    form.error.as_deref(),
                    form.editing,
                    false,
                ),
                Dialog::AudioMuse(form) => (
                    &form.fields,
                    crate::app::sources::audiomuse::Form::LABELS,
                    form.focus,
                    form.task.is_some(),
                    form.error.as_deref(),
                    true,
                    true,
                ),
                _ => unreachable!(),
            };
            // URL / username / masked password / name, inline status, buttons.
            let rows = Layout::vertical([
                Constraint::Length(2),
                Constraint::Length(2),
                Constraint::Length(2),
                Constraint::Length(2),
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);
            for (index, field) in fields.iter().enumerate() {
                let row = rows[index];
                frame.render_widget(
                    Paragraph::new(labels[index]).style(Style::default().fg(t.colors.fg_accent)),
                    Rect {
                        height: row.height.min(1),
                        ..row
                    },
                );
                let input = Rect::new(
                    row.x,
                    row.y.saturating_add(1),
                    row.width,
                    row.height.saturating_sub(1),
                );
                controls.push(input);
                if input.height == 0 || input.width == 0 {
                    continue;
                }
                let focused = focus == index && !busy;
                let chars: Vec<char> = if index == 2 {
                    vec!['•'; field.value.chars().count()]
                } else {
                    field.value.chars().collect()
                };
                let caret = field.value[..field.cursor].chars().count();
                let mut start = 0;
                while start < caret
                    && UnicodeWidthStr::width(
                        chars[start..caret].iter().collect::<String>().as_str(),
                    ) >= input.width as usize
                {
                    start += 1;
                }
                let text: String = chars[start..].iter().collect();
                let style = if focused {
                    Style::default()
                        .fg(t.colors.selection_text)
                        .bg(t.colors.selection_bar_bg)
                } else {
                    Style::default().fg(t.colors.fg_primary)
                };
                let hint = if index == 0 {
                    "http://server:port/path/ or https://…"
                } else if index == 2 && editing {
                    "Leave blank to keep saved password"
                } else {
                    ""
                };
                frame.render_widget(
                    Paragraph::new(if text.is_empty() && !focused {
                        hint
                    } else {
                        &text
                    })
                    .style(style),
                    input,
                );
                if focused {
                    let offset = UnicodeWidthStr::width(
                        chars[start..caret].iter().collect::<String>().as_str(),
                    ) as u16;
                    frame.set_cursor_position((input.x + offset.min(input.width - 1), input.y));
                }
            }
            let message = if busy {
                "Checking connection… Esc cancels."
            } else if let Some(error) = &error {
                error
            } else if audiomuse {
                "Analysis only; music and scanning stay on the server. Blank password keeps saved credentials."
            } else if editing {
                "Check the URL scheme and port. Blank password keeps the saved password."
            } else {
                "Use the server's exact URL, including http:// or https://. Credentials are stored privately."
            };
            frame.render_widget(
                Paragraph::new(message)
                    .wrap(Wrap { trim: false })
                    .style(Style::default().fg(if error.is_some() {
                        t.colors.error
                    } else {
                        t.colors.fg_muted
                    })),
                rows[4],
            );
            if audiomuse {
                buttons(
                    frame,
                    rows[5],
                    ["Save", "Disconnect", "Cancel"],
                    focus.checked_sub(4).unwrap_or(3),
                    busy,
                    &mut controls,
                );
            } else {
                buttons(
                    frame,
                    rows[5],
                    [if editing { "Save" } else { "Add library" }, "Cancel"],
                    focus.checked_sub(4).unwrap_or(2),
                    busy,
                    &mut controls,
                );
            }
            frame.render_widget(
                Paragraph::new("Tab / ↑↓ fields · Enter submit · Esc cancel · Ctrl+A select all")
                    .style(Style::default().fg(t.colors.fg_muted)),
                rows[6],
            );
        }
    }
    state.hit_regions.borrow_mut().library_dialog = controls;
}

fn buttons<const N: usize>(
    frame: &mut Frame,
    row: Rect,
    labels: [&str; N],
    selected: usize,
    busy: bool,
    controls: &mut Vec<Rect>,
) {
    let t = theme();
    let slots = Layout::horizontal(
        labels
            .iter()
            .map(|_| Constraint::Length(18))
            .collect::<Vec<_>>(),
    )
    .split(row);
    for (index, label) in labels.into_iter().enumerate() {
        let style = if selected == index && !(busy && index == 0) {
            Style::default()
                .fg(t.colors.selection_text)
                .bg(t.colors.selection_bar_bg)
        } else {
            Style::default().fg(t.colors.fg_muted)
        };
        frame.render_widget(
            Paragraph::new(format!("[ {label} ]")).style(style),
            slots[index],
        );
        controls.push(slots[index]);
    }
}
