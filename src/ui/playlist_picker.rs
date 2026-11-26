//! Filesystem dialog shared by the terminal and embedded terminal hosts.
use crate::app::{Action, AppState, Event, action::QueueAction, state::InputDialogAction};
use crossterm::event::{Event as TerminalEvent, KeyCode};
use rat_widget::{
    button::ButtonStyle,
    event::FileOutcome,
    file_dialog::{FileDialog, FileDialogState, handle_events},
    list::ListStyle,
    text::TextStyle,
};
use ratatui::{
    Frame,
    style::Style,
    widgets::{Block, Clear, Paragraph},
};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub(crate) enum PlaylistPicker {
    #[default]
    Closed,
    Choosing(Box<FileDialogState>),
    ConfirmReplace(PathBuf),
}

impl PlaylistPicker {
    pub(crate) fn sync(&mut self, state: &mut AppState) {
        let Some(input) = state
            .popups
            .input_dialog()
            .filter(|i| i.action_type.is_playlist_file())
        else {
            *self = Self::default();
            return;
        };
        if !matches!(self, Self::Closed) {
            return;
        }
        let path = Path::new(input.input.as_str());
        let directory = path
            .parent()
            .filter(|p| p.is_dir())
            .map(Path::to_owned)
            .or_else(|| dirs::download_dir().filter(|p| p.is_dir()))
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| ".".into());
        let mut dialog = FileDialogState::new();
        let result = match input.action_type {
            InputDialogAction::ImportPlaylist { .. } => {
                dialog.set_filter(|p| {
                    p.extension().and_then(|s| s.to_str()).is_some_and(|s| {
                        ["m3u", "m3u8", "csv"]
                            .iter()
                            .any(|ext| s.eq_ignore_ascii_case(ext))
                    })
                });
                dialog.open_dialog(directory)
            }
            InputDialogAction::ExportPlaylist { format, .. } => dialog.save_dialog_ext(
                directory,
                path.file_name().unwrap_or_default().to_string_lossy(),
                format.extension(),
            ),
            _ => return,
        };
        match result {
            Ok(()) => *self = Self::Choosing(Box::new(dialog)),
            Err(error) => {
                state.popups.set_input_dialog(None);
                state.set_error(format!("Could not open playlist picker: {error}"));
            }
        }
    }

    pub(crate) fn draw(&mut self, frame: &mut Frame, state: &AppState) {
        let Some(input) = state.popups.input_dialog() else {
            return;
        };
        let colors = super::theme::theme().colors;
        let style = Style::default().bg(colors.bg_primary).fg(colors.fg_primary);
        let block = Block::bordered().title(input.title.as_str()).style(style);
        let area = super::layout::centered_rect(85, 80, frame.area());
        match self {
            Self::Closed => {}
            Self::ConfirmReplace(path) => {
                frame.render_widget(Clear, area);
                frame.render_widget(
                    Paragraph::new(format!(
                        "Replace {}?\nEnter to replace · Esc to cancel",
                        path.display()
                    ))
                    .block(block),
                    area,
                );
            }
            Self::Choosing(dialog) => {
                frame.render_widget(Clear, area);
                let focus = Some(colors.focused_row());
                let list = ListStyle {
                    style,
                    select: Some(colors.inactive_row()),
                    focus,
                    ..Default::default()
                };
                let widget = FileDialog::new()
                    .style(style)
                    .block(block)
                    .list_style(list.clone())
                    .roots_style(list)
                    .text_style(TextStyle {
                        style,
                        focus,
                        select: focus,
                        ..Default::default()
                    })
                    .button_style(ButtonStyle {
                        style,
                        focus,
                        armed: focus,
                        ..Default::default()
                    })
                    .ok_text(input.action_type.submit_label());
                frame.render_stateful_widget(widget, area, dialog.as_mut());
            }
        }
    }

    pub(crate) fn input(&mut self, event: &Event, state: &mut AppState) -> Option<Vec<Action>> {
        let input = match event {
            Event::Key(key) => TerminalEvent::Key(*key),
            Event::Mouse(mouse) => TerminalEvent::Mouse(*mouse),
            Event::Paste(text) => TerminalEvent::Paste(text.clone()),
            _ => return None,
        };
        let outcome = match self {
            Self::Closed => return None,
            Self::ConfirmReplace(path) => match input {
                TerminalEvent::Key(key) if key.code == KeyCode::Enter => {
                    FileOutcome::Ok(std::mem::take(path))
                }
                TerminalEvent::Key(key) if key.code == KeyCode::Esc => FileOutcome::Cancel,
                _ => return Some(vec![]),
            },
            Self::Choosing(dialog) => match handle_events(dialog, true, &input) {
                Ok(FileOutcome::Ok(path))
                    if path.exists()
                        && state.popups.input_dialog().is_some_and(|i| {
                            matches!(i.action_type, InputDialogAction::ExportPlaylist { .. })
                        }) =>
                {
                    *self = Self::ConfirmReplace(path);
                    return Some(vec![]);
                }
                Ok(outcome) => outcome,
                Err(error) => {
                    state.set_error(error.to_string());
                    FileOutcome::Cancel
                }
            },
        };
        let action = match outcome {
            FileOutcome::Ok(path) => {
                state
                    .popups
                    .input_dialog()
                    .and_then(|i| match &i.action_type {
                        InputDialogAction::ImportPlaylist { generation } => {
                            Some(QueueAction::ImportPlaylist {
                                generation: *generation,
                                path,
                            })
                        }
                        InputDialogAction::ExportPlaylist {
                            generation,
                            contents,
                            ..
                        } => Some(QueueAction::ExportPlaylist {
                            generation: *generation,
                            contents: contents.clone(),
                            path,
                        }),
                        _ => None,
                    })
            }
            FileOutcome::Cancel => None,
            _ => return Some(vec![]),
        };
        state.popups.set_input_dialog(None);
        *self = Self::default();
        Some(action.into_iter().map(Action::from).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::InputDialog;
    use crate::services::playlist_export::Format;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn saving_to_an_existing_file_requires_a_second_confirmation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Mix.csv");
        std::fs::write(&path, b"original").unwrap();
        let mut state = AppState::new();
        let mut picker = PlaylistPicker::default();
        let mut terminal = Terminal::new(TestBackend::new(90, 30)).unwrap();
        let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
        for confirm in [false, true] {
            state.popups.set_input_dialog(Some(InputDialog {
                title: "Export Playlist".into(),
                input: path.to_string_lossy().as_ref().into(),
                action_type: InputDialogAction::ExportPlaylist {
                    generation: 7,
                    format: Format::Csv,
                    contents: std::sync::Arc::new(b"updated".to_vec()),
                },
            }));
            picker.sync(&mut state);
            terminal.draw(|frame| picker.draw(frame, &state)).unwrap();
            assert!(
                picker
                    .input(&key(KeyCode::Enter), &mut state)
                    .unwrap()
                    .is_empty()
            );
            let actions = picker
                .input(
                    &key(if confirm {
                        KeyCode::Enter
                    } else {
                        KeyCode::Esc
                    }),
                    &mut state,
                )
                .unwrap();
            if confirm {
                assert!(
                    matches!(actions.as_slice(), [Action::Queue(QueueAction::ExportPlaylist {generation:7, path: destination, ..})] if destination == &path)
                );
            } else {
                assert!(actions.is_empty());
            }
            assert!(state.popups.input_dialog().is_none());
            assert_eq!(std::fs::read(&path).unwrap(), b"original");
        }
    }
}
