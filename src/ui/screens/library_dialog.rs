//! Conventional library forms and account buttons, rendered from draft state.
use crate::app::sources::dialogs::{Dialog, WebdavForm};
use crate::ui::theme::theme;
use crate::ui::RenderState;
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

pub fn render(frame: &mut Frame, state: &RenderState) {
    let Some(dialog) = &state.popups.library_dialog else {
        return;
    };
    let width = frame.area().width.saturating_sub(4).min(78);
    let height = frame.area().height.saturating_sub(2).min(match dialog {
        Dialog::Library(options) => (options.items(state).len() as u16 + 12).min(22),
        Dialog::Webdav(_) | Dialog::AudioMuse(_) | Dialog::Navidrome(_) => 16,
        Dialog::Local(_) => 11,
        Dialog::Add { .. } => 9,
    });
    let area = Rect::new(
        (frame.area().width - width) / 2,
        (frame.area().height - height) / 2,
        width,
        height,
    );
    let title = if let Dialog::Library(options) = dialog {
        format!(" {} / Settings ", options.choice.label())
    } else {
        match dialog {
            Dialog::Library(_) => unreachable!(),
            Dialog::Local(_) => " Add local folder ",
            Dialog::Add { .. } => " Add library ",
            Dialog::Navidrome(_) => " Subsonic / Navidrome connection ",
            Dialog::AudioMuse(_) => " AudioMuse connection ",
            Dialog::Webdav(form) if form.editing => " Edit WebDAV library ",
            Dialog::Webdav(_) => " Add WebDAV library ",
        }
        .to_string()
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme().colors.fg_accent))
        .style(super::settings::paper());
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let mut controls = Vec::new();
    match dialog {
        Dialog::Library(options) => {
            use super::settings::{paper, selected};
            use crate::app::sources::options::{OptionAction, OptionsPage};
            let info_height = if inner.height >= 11 {
                4
            } else if inner.height >= 9 {
                2
            } else {
                0
            };
            let help_height = if inner.height >= 16 {
                4
            } else if inner.height >= 11 {
                2
            } else {
                0
            };
            let rows = Layout::vertical([
                Constraint::Length(1),
                Constraint::Length(info_height),
                Constraint::Min(1),
                Constraint::Length(help_height),
                Constraint::Length(1),
            ])
            .split(inner);
            let pages = options.pages(state);
            let slots = Layout::horizontal(
                pages
                    .iter()
                    .map(|_| Constraint::Ratio(1, pages.len() as u32)),
            )
            .split(rows[0]);
            for (i, page) in pages.iter().enumerate() {
                frame.render_widget(
                    Paragraph::new(page.name())
                        .alignment(Alignment::Center)
                        .style(if *page == options.page {
                            selected()
                        } else {
                            paper()
                        }),
                    slots[i],
                );
                state
                    .hit_regions
                    .borrow_mut()
                    .library_option_pages
                    .push((slots[i], *page));
            }
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
                    Ok(n) => crate::util::format_bytes(*n),
                    Err(_) => "Unavailable".into(),
                })
                .unwrap_or_else(|| "Measuring...".into());
            let artwork_size = options
                .choice
                .artwork_key()
                .and_then(|key| {
                    state
                        .settings_state
                        .artwork_cache_entries
                        .get(&key)
                        .copied()
                })
                .map_or_else(
                    || "Measuring...".to_string(),
                    |(count, bytes)| {
                        format!("{} ({count} files)", crate::util::format_bytes(bytes))
                    },
                );
            let heading = match options.page {
                OptionsPage::General => format!(
                    "{}\n{}\n{}",
                    options.choice.label(),
                    options.choice.provider(),
                    options.choice.location()
                ),
                OptionsPage::Analysis => format!(
                    "{}\nOptional analysis and music discovery",
                    options.choice.label()
                ),
                OptionsPage::Cache => format!(
                    "{}\nMetadata: {size}  Artwork: {artwork_size}\n{}",
                    options.choice.label(),
                    crate::app::sources::cache::status(&options.choice, state)
                ),
            };
            frame.render_widget(
                Paragraph::new(heading)
                    .style(paper())
                    .wrap(Wrap { trim: true }),
                rows[1],
            );
            let items = options.items(state);
            let visible = rows[2].height as usize;
            let offset = options
                .scroll_pin
                .unwrap_or_else(|| options.selected.saturating_sub(visible.saturating_sub(1)))
                .min(items.len().saturating_sub(visible));
            state.hit_regions.borrow_mut().library_option_scroll = offset;
            controls.resize(items.len(), Rect::default());
            for (index, (_, label)) in items.iter().enumerate().skip(offset).take(visible) {
                let rect = Rect::new(
                    rows[2].x,
                    rows[2].y + (index - offset) as u16,
                    rows[2].width,
                    1,
                );
                frame.render_widget(
                    Paragraph::new(format!(
                        " {}",
                        if label.starts_with('[') {
                            label.clone()
                        } else {
                            format!("[ {label} ]")
                        }
                    ))
                    .style(if index == options.selected {
                        selected()
                    } else {
                        paper()
                    }),
                    rect,
                );
                controls[index] = rect;
            }
            let help = match items.get(options.selected).map(|(action, _)| action) {
                Some(OptionAction::Activate) => "Open this library for browsing and playback. Highlighting or editing its settings does not activate it.",
                Some(OptionAction::Rename) if matches!(options.choice, crate::app::sources::LibraryChoice::Navidrome { .. }) => "Changes the account label in Textamp. Server library names are managed on the server.",
                Some(OptionAction::Rename) => "Changes the name shown in Textamp. The music folder stays in place.",
                Some(OptionAction::Connection) => "Edit the saved server address and sign-in. Changes are checked before saving.",
                Some(OptionAction::Sonic) => "Show or hide analysis and discovery features for this library. Existing analysis is kept.",
                Some(OptionAction::AudioMuse) => "Connect an AudioMuse analysis server for this library. Textamp reads existing analysis.",
                Some(OptionAction::Rescan) if crate::app::sources::cache::running(&options.choice, state) => "Cancel the background refresh. The last usable cached catalog is kept.",
                Some(OptionAction::Rescan) => "Refresh catalog and available analysis now. Normally runs weekly. Folder scans read directory names; audio files are not downloaded.",
                Some(OptionAction::Clear) => "Remove this library's cached catalog and analysis. Artwork, music and sign-ins are kept. Use Refresh catalog to rebuild metadata.",
                Some(OptionAction::ClearArtwork) => "Remove downloaded images for this library. Other libraries' artwork, music and display choices are kept.",
                Some(OptionAction::Remove) if matches!(options.choice, crate::app::sources::LibraryChoice::Navidrome { .. }) => "Remove the entire account and its libraries from Textamp. Server music and playlists are kept.",
                Some(OptionAction::Remove) => "Remove this library from Textamp. Music files are kept.",
                _ => "Return to the library list. Changes already applied are saved.",
            };
            frame.render_widget(
                Paragraph::new(help)
                    .style(paper().fg(theme().colors.fg_accent))
                    .wrap(Wrap { trim: true }),
                rows[3],
            );
            frame.render_widget(
                Paragraph::new("Left/Right Page  Enter Select  Esc Close").style(selected()),
                rows[4],
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
                    super::settings::selected()
                } else {
                    super::settings::paper()
                };
                frame.render_widget(Paragraph::new(format!("[ {label} ]")).style(style), rect);
                controls.push(rect);
            }
        }
        Dialog::Local(_) | Dialog::Webdav(_) | Dialog::AudioMuse(_) | Dialog::Navidrome(_) => {
            let local = matches!(dialog, Dialog::Local(_));
            let (fields, labels, focus, busy, error, editing, audiomuse): (
                &[_],
                &[_],
                _,
                _,
                _,
                _,
                _,
            ) = match dialog {
                Dialog::Local(form) => (
                    &form.fields,
                    &["Folder path", "Name (optional)"],
                    form.focus,
                    false,
                    form.error.as_deref(),
                    false,
                    false,
                ),
                Dialog::Navidrome(form) => (
                    &form.fields,
                    &crate::app::sources::dialogs::ServerForm::LABELS,
                    form.focus,
                    form.busy,
                    form.error.as_deref(),
                    form.editing,
                    false,
                ),
                Dialog::Webdav(form) => (
                    &form.fields,
                    &WebdavForm::LABELS,
                    form.focus,
                    form.task.is_some(),
                    form.error.as_deref(),
                    form.editing,
                    false,
                ),
                Dialog::AudioMuse(form) => (
                    &form.fields,
                    &crate::app::sources::audiomuse::Form::LABELS,
                    form.focus,
                    form.task.is_some(),
                    form.error.as_deref(),
                    true,
                    true,
                ),
                _ => unreachable!(),
            };
            let field_count = fields.len();
            let constraints: Vec<_> = std::iter::repeat_n(Constraint::Length(2), field_count)
                .chain([
                    Constraint::Min(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .collect();
            let rows = Layout::vertical(constraints).split(inner);
            for (index, field) in fields.iter().enumerate() {
                let row = rows[index];
                frame.render_widget(
                    Paragraph::new(labels[index])
                        .style(super::settings::paper().fg(theme().colors.fg_accent)),
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
                let chars: Vec<char> = if index == 2 && !local {
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
                    super::settings::selected()
                } else {
                    super::settings::paper().bg(theme().colors.bg_secondary)
                };
                let hint = if index == 0 && local {
                    "/path/to/music or ~/Music"
                } else if index == 0 {
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
            } else if local {
                "The folder is checked when you save. Adding a library does not switch playback."
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
                        theme().colors.error
                    } else {
                        theme().colors.fg_accent
                    })),
                rows[field_count],
            );
            if audiomuse {
                buttons(
                    frame,
                    rows[field_count + 1],
                    ["Save", "Disconnect", "Cancel"],
                    focus.checked_sub(field_count).unwrap_or(3),
                    busy,
                    &mut controls,
                );
            } else {
                buttons(
                    frame,
                    rows[field_count + 1],
                    [if editing { "Save" } else { "Add library" }, "Cancel"],
                    focus.checked_sub(field_count).unwrap_or(2),
                    busy,
                    &mut controls,
                );
            }
            frame.render_widget(
                Paragraph::new("Tab / ↑↓ fields · Enter submit · Esc cancel · Ctrl+A select all")
                    .style(super::settings::paper().fg(theme().colors.fg_accent)),
                rows[field_count + 2],
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
    let slots = Layout::horizontal(
        labels
            .iter()
            .map(|_| Constraint::Length(18))
            .collect::<Vec<_>>(),
    )
    .split(row);
    for (index, label) in labels.into_iter().enumerate() {
        let style = if selected == index && !(busy && index == 0) {
            super::settings::selected()
        } else {
            super::settings::paper().fg(theme().colors.fg_accent)
        };
        frame.render_widget(
            Paragraph::new(format!("[ {label} ]")).style(style),
            slots[index],
        );
        controls.push(slots[index]);
    }
}
