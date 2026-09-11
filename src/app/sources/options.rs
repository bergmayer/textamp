//! One set of library options for mouse, keyboard and the settings popover.
use super::*;
use crate::app::action::SettingsAction;
use crate::app::state::{ConfirmAction, ConfirmDialog};
use crossterm::event::KeyCode;

#[derive(Debug, Clone)]
pub struct Options {
    pub choice: LibraryChoice,
    pub selected: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionAction {
    Activate,
    Rename,
    Connection,
    Sonic,
    AudioMuse,
    Rescan,
    Clear,
    SharedArtwork,
    Remove,
    Close,
}
impl Options {
    pub fn items(&self, state: &AppState) -> Vec<(OptionAction, String)> {
        use OptionAction::*;
        let mut items = vec![
            (
                Activate,
                if self.choice.active(state) {
                    "Active library"
                } else {
                    "Make active"
                }
                .into(),
            ),
            (Rename, "Rename…".into()),
        ];
        if matches!(
            &self.choice,
            LibraryChoice::Navidrome { .. }
                | LibraryChoice::Folder(FolderSource {
                    location: FolderLocation::Webdav { .. },
                    ..
                })
        ) {
            items.push((Connection, "Edit connection…".into()));
        }
        if let Some(key) = sonic::choice_key(&self.choice, state) {
            let enabled = !state.sources.sonic_disabled_libraries.contains(&key);
            items.push((
                Sonic,
                format!("[{}] Sonic / AI features", if enabled { "x" } else { " " }),
            ));
            items.push((AudioMuse, "AudioMuse connection…".into()));
        }
        items.extend([
            (
                Rescan,
                if super::cache::running(&self.choice, state) {
                    "Cancel re-scan"
                } else {
                    "Re-scan cache"
                }
                .into(),
            ),
            (Clear, "Clear library cache…".into()),
            (SharedArtwork, "Clear shared artwork cache…".into()),
            (
                Remove,
                if matches!(self.choice, LibraryChoice::Navidrome { .. }) {
                    "Remove account…"
                } else {
                    "Remove library…"
                }
                .into(),
            ),
            (Close, "Close".into()),
        ]);
        items
    }
}

pub fn open(choice: LibraryChoice, state: &mut AppState) -> Vec<Action> {
    if state.popups.library_picker_active
        || matches!(
            choice,
            LibraryChoice::Add
                | LibraryChoice::AddFolder
                | LibraryChoice::AddWebdav
                | LibraryChoice::AddNavidrome
        )
    {
        return vec![choice.action()];
    }
    state.popups.library_dialog = Some(dialogs::Dialog::Library(Options {
        choice,
        selected: 0,
    }));
    vec![SettingsAction::RefreshCacheStats.into()]
}

pub fn activate(state: &mut AppState) -> Vec<Action> {
    let Some(dialogs::Dialog::Library(options)) = &state.popups.library_dialog else {
        return vec![];
    };
    let choice = options.choice.clone();
    let Some((action, _)) = options.items(state).get(options.selected).cloned() else {
        return vec![];
    };
    use OptionAction::*;
    if action == Sonic {
        return vec![SourceAction::ToggleSonic(choice).into()];
    }
    if action == Rescan {
        return vec![SettingsAction::RescanSourceCache(choice).into()];
    }
    if !matches!(action, Clear | SharedArtwork) {
        state.popups.library_dialog = None;
    }
    match action {
        Activate => vec![choice.action()],
        Rename => shortcut(&choice, KeyCode::Char('r'), state),
        Connection => shortcut(&choice, KeyCode::Char('c'), state),
        AudioMuse => shortcut(&choice, KeyCode::Char('m'), state),
        Remove => shortcut(&choice, KeyCode::Delete, state),
        Clear | SharedArtwork => {
            state.popups.confirm_dialog = Some(ConfirmDialog {
                title: if action == Clear {
                    "Clear library cache?"
                } else {
                    "Clear shared artwork?"
                }
                .into(),
                message: if action == Clear {
                    format!("Clear metadata and analysis for {}? Music and sign-ins are kept. Re-scan to rebuild.", choice.label())
                } else {
                    "Artwork is shared across libraries and will download again when needed.".into()
                },
                selected_yes: false,
                on_confirm: if action == Clear {
                    ConfirmAction::ClearSourceCache(choice)
                } else {
                    ConfirmAction::ClearArtworkCache
                },
            });
            vec![]
        }
        _ => vec![],
    }
}

/// Kept as convenience keys, using the same operations as the visible buttons.
pub fn shortcut(choice: &LibraryChoice, key: KeyCode, state: &mut AppState) -> Vec<Action> {
    match key {
        KeyCode::Char('i') => {
            if let Some(choice) = Some(choice) {
                return vec![Action::Source(
                    crate::app::sources::SourceAction::ToggleSonic(choice.clone()),
                )];
            }
        }
        KeyCode::Char('m') => {
            if let Some(choice @ LibraryChoice::Navidrome { .. }) = Some(choice) {
                return vec![
                    crate::app::sources::audiomuse::Command::Configure(choice.clone()).into(),
                ];
            }
        }

        KeyCode::Char('c') => {
            if let Some(LibraryChoice::Folder(source)) = Some(choice) {
                if matches!(
                    source.location,
                    crate::library::FolderLocation::Webdav { .. }
                ) {
                    state.popups.library_dialog =
                        Some(crate::app::sources::dialogs::Dialog::Webdav(
                            crate::app::sources::dialogs::WebdavForm::new(Some(source)),
                        ));
                    return vec![];
                }
            }
            if let Some(LibraryChoice::Navidrome { source, .. }) = Some(choice) {
                state.popups.library_dialog =
                    Some(crate::app::sources::dialogs::Dialog::Navidrome(
                        crate::app::sources::dialogs::ServerForm::new(Some(source)),
                    ));

                return vec![];
            }
        }
        KeyCode::Char('r') => {
            if let Some(LibraryChoice::Navidrome { source, .. }) = Some(choice) {
                state.popups.input_dialog = Some(crate::app::state::InputDialog {
                    title: "Account name".into(),
                    input: source.name.clone().into(),
                    action_type: crate::app::state::InputDialogAction::NavidromeName(
                        source.id.clone(),
                    ),
                });
                return vec![];
            }
            if let Some(LibraryChoice::Folder(source)) = Some(choice) {
                state.popups.input_dialog = Some(crate::app::state::InputDialog {
                    title: "Library name".into(),
                    input: source.name.clone().into(),
                    action_type: crate::app::state::InputDialogAction::FolderName(
                        source.id.clone(),
                    ),
                });
            }
        }
        KeyCode::Delete | KeyCode::Backspace => {
            if let Some(LibraryChoice::Navidrome { source, .. }) = Some(choice) {
                state.popups.confirm_dialog = Some(crate::app::state::ConfirmDialog { title: "Remove Navidrome account?".into(), message: "Remove this account and its saved password from Textamp? Server files and playlists are not deleted.".into(), selected_yes: false, on_confirm: crate::app::state::ConfirmAction::RemoveNavidrome(source.id.clone()) });
                return vec![];
            }
            if let Some(LibraryChoice::Folder(source)) = Some(choice) {
                state.popups.confirm_dialog = Some(crate::app::state::ConfirmDialog {
                    title: "Remove library?".into(),
                    message: format!(
                        "Remove {} from Textamp? Music files are not deleted.",
                        source.name
                    ),
                    selected_yes: false,
                    on_confirm: crate::app::state::ConfirmAction::RemoveFolder(source.id.clone()),
                });
            }
        }
        _ => {}
    }
    vec![]
}
