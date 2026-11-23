//! Literal input shared by terminal paste and native IME commits.
use super::{Action, AppState, action::SearchAction, state::SearchFocus};
use crate::config::Config;

/// Logical terminal text destination, independent of native or terminal widgets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Palette,
    Search,
    Filter {
        category: super::BrowseCategory,
        column: usize,
    },
    Description,
    Input(String),
    Library {
        kind: FormKind,
        owner: String,
        field: usize,
    },
    Journey(JourneyField),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormKind {
    Local,
    Server,
    Webdav,
    Analysis,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JourneyField {
    Start,
    End,
    Count,
}
/// Input-method projection. Secret values are replaced by one bullet per UTF-16 unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    pub target: Target,
    pub text: String,
    pub selection: std::ops::Range<usize>,
    pub secret: bool,
}
/// Native editing operations use the same logical destination as terminal paste.
#[derive(Clone, Debug)]
pub enum Edit {
    Select {
        target: Target,
        range: std::ops::Range<usize>,
    },
    Replace {
        target: Target,
        range: Option<std::ops::Range<usize>>,
        text: String,
    },
}
fn field(state: &AppState) -> Option<(&super::sources::dialogs::Field, Target, bool)> {
    use super::sources::dialogs::Dialog;
    let (fields, focus, kind, owner, password): (&[_], _, _, _, &[usize]) =
        match state.popups.library_dialog()? {
            Dialog::Local(form) if form.focus == 0 && form.existing_id.is_some() => return None,
            Dialog::Local(form) => (
                &form.fields[..],
                form.focus,
                FormKind::Local,
                form.instance.clone(),
                &[],
            ),
            Dialog::Navidrome(form) => (
                &form.fields[..],
                form.focus,
                FormKind::Server,
                form.source.id.clone(),
                &[2, 6],
            ),
            Dialog::Webdav(form) => (
                &form.fields[..],
                form.focus,
                FormKind::Webdav,
                form.instance.clone(),
                &[2],
            ),
            Dialog::AudioMuse(form) => (
                &form.fields[..],
                form.focus,
                FormKind::Analysis,
                form.instance.clone(),
                &[2],
            ),
            _ => return None,
        };
    Some((
        fields.get(focus)?,
        Target::Library {
            kind,
            owner,
            field: focus,
        },
        password.contains(&focus),
    ))
}
fn raw_document(state: &AppState) -> Option<Document> {
    let (target, text, cursor, selected, secret) = if state.palette.open {
        (
            Target::Palette,
            state.palette.query.clone(),
            state
                .palette
                .query
                .char_indices()
                .nth(state.palette.cursor)
                .map_or(state.palette.query.len(), |(byte, _)| byte),
            false,
            false,
        )
    } else if let Some((field, target, secret)) = field(state) {
        (
            target,
            field.value.to_string(),
            field.cursor,
            field.selected,
            secret,
        )
    } else if let Some(dialog) = state.popups.input_dialog() {
        (
            Target::Input(dialog.title.clone()),
            dialog.input.to_string(),
            dialog.input.len(),
            false,
            false,
        )
    } else if state.popups.search_active() && state.search.focus == SearchFocus::Input {
        (
            Target::Search,
            state.search.query.clone(),
            state.search.query.len(),
            false,
            false,
        )
    } else if super::tui_navigation::filter_focused(state) {
        (
            Target::Filter {
                category: state.browse_category,
                column: state.list_filter.column,
            },
            state.list_filter.query.clone(),
            state.list_filter.query.len(),
            false,
            false,
        )
    } else if let Some(launcher) = state.popups.adventure_launcher() {
        use super::journey::JourneyControl;
        let (target, value) = match launcher.terminal_focus {
            JourneyControl::Start => (JourneyField::Start, &launcher.query),
            JourneyControl::End => (JourneyField::End, &launcher.query),
            JourneyControl::Count => (JourneyField::Count, &launcher.track_count_input),
            _ => return None,
        };
        (
            Target::Journey(target),
            value.clone(),
            value.len(),
            false,
            false,
        )
    } else if state.tui_description_focused
        && state.view == super::View::Browse
        && matches!(state.popups.modal(), super::state::Modal::Closed)
    {
        (
            Target::Description,
            state.tui_description_query.clone(),
            state.tui_description_query.len(),
            false,
            false,
        )
    } else {
        return None;
    };
    let cursor = text[..cursor.min(text.len())].encode_utf16().count();
    let selection = state
        .terminal_text_selection
        .as_ref()
        .filter(|(current, _)| *current == target)
        .map(|(_, range)| range.clone())
        .unwrap_or_else(|| {
            if selected {
                0..text.encode_utf16().count()
            } else {
                cursor..cursor
            }
        });
    Some(Document {
        target,
        text,
        selection,
        secret,
    })
}
/// Describe the focused text input without exposing credential values.
pub fn document(state: &AppState) -> Option<Document> {
    let mut document = raw_document(state)?;
    if document.secret {
        document.text = "•".repeat(document.text.encode_utf16().count());
    }
    Some(document)
}
fn byte_offset(value: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (byte, character) in value.char_indices() {
        if units >= utf16 {
            return byte;
        }
        units += character.len_utf16();
        if units > utf16 {
            return byte;
        }
    }
    value.len()
}
fn library_fields_mut(state: &mut AppState) -> Option<&mut [super::sources::dialogs::Field]> {
    use super::sources::dialogs::Dialog;
    match state.popups.library_dialog_mut()? {
        Dialog::Local(form) => Some(&mut form.fields),
        Dialog::Navidrome(form) => Some(&mut form.fields[..]),
        Dialog::Webdav(form) => Some(&mut form.fields),
        Dialog::AudioMuse(form) => Some(&mut form.fields),
        _ => None,
    }
}

/// Delete a selected input range before interpreting Backspace or Delete as navigation.
pub(crate) fn delete_selection(
    key: crossterm::event::KeyEvent,
    state: &mut AppState,
) -> Option<Vec<Action>> {
    use crossterm::event::{KeyCode, KeyModifiers};
    if !matches!(key.code, KeyCode::Backspace | KeyCode::Delete)
        || key
            .modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL | KeyModifiers::SUPER)
    {
        return None;
    }
    let document = document(state)?;
    if document.selection.is_empty() {
        return None;
    }
    Some(edit(
        Edit::Replace {
            target: document.target,
            range: None,
            text: String::new(),
        },
        state,
    ))
}

/// Apply an edit only while its logical text destination is still focused.
pub fn edit(edit: Edit, state: &mut AppState) -> Vec<Action> {
    let Some(current) = raw_document(state) else {
        return vec![];
    };
    match edit {
        Edit::Select { target, range } if target == current.target => {
            let length = current.text.encode_utf16().count();
            let start = range.start.min(length);
            let end = range.end.min(length).max(start);
            let cursor = byte_offset(&current.text, end);
            match &target {
                Target::Palette => state.palette.cursor = current.text[..cursor].chars().count(),
                Target::Library { field, .. } => {
                    if let Some(field) =
                        library_fields_mut(state).and_then(|fields| fields.get_mut(*field))
                    {
                        field.cursor = cursor;
                        field.selected = false;
                    }
                }
                _ => {}
            }
            state.terminal_text_selection = Some((target, start..end));
            vec![]
        }
        Edit::Replace {
            target,
            range,
            text,
        } if target == current.target => {
            let range = range.unwrap_or(current.selection);
            let start = byte_offset(&current.text, range.start);
            let end = byte_offset(&current.text, range.end).max(start);
            let text = text
                .chars()
                .filter(|character| !character.is_control())
                .collect::<String>();
            let mut value = current.text;
            value.replace_range(start..end, &text);
            let cursor = start + text.len();
            let caret = value[..cursor].encode_utf16().count();
            state.terminal_text_selection = Some((target.clone(), caret..caret));
            match target {
                Target::Palette => {
                    state.palette.query = value;
                    state.palette.cursor = state.palette.query[..cursor].chars().count();
                    super::command_palette::refresh_matches(state);
                    vec![]
                }
                Target::Search => vec![SearchAction::SetSearchQuery(value).into()],
                Target::Filter { .. } => vec![SearchAction::SetListFilterQuery(value).into()],
                Target::Description => {
                    state.tui_description_query = value;
                    vec![]
                }
                Target::Input(_) => {
                    if let Some(dialog) = state.popups.input_dialog_mut() {
                        dialog.input = value.into();
                    }
                    vec![]
                }
                Target::Journey(field) => match field {
                    JourneyField::Count => {
                        vec![SearchAction::AdventureLauncherSetTrackCount(value).into()]
                    }
                    JourneyField::Start | JourneyField::End => {
                        vec![
                            SearchAction::AdventureLauncherSetEndpointQuery {
                                step: if field == JourneyField::Start {
                                    super::state::AdventureStep::FindStartTrack
                                } else {
                                    super::state::AdventureStep::FindEndTrack
                                },
                                query: value,
                            }
                            .into(),
                        ]
                    }
                },
                Target::Library { field, .. } => {
                    let Some(fields) = library_fields_mut(state) else {
                        return vec![];
                    };
                    if value.len() <= 4096
                        && let Some(field) = fields.get_mut(field)
                    {
                        field.value = value.into();
                        field.cursor = cursor;
                        field.selected = false;
                    }
                    vec![]
                }
            }
        }
        _ => vec![],
    }
}

/// Insert literal text as one edit and issue at most one search effect.
pub fn insert(text: &str, state: &mut AppState, _config: &Config) -> Vec<Action> {
    let Some(document) = document(state) else {
        return vec![];
    };
    edit(
        Edit::Replace {
            target: document.target,
            range: None,
            text: text.into(),
        },
        state,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf16_replacement_preserves_unicode_and_the_palette_character_cursor() {
        let mut state = AppState::new();
        state.palette.open = true;
        state.palette.query = "a😀e\u{301}b".into();
        state.palette.cursor = 5;
        let target = document(&state).unwrap().target;
        edit(
            Edit::Replace {
                target,
                range: Some(1..3),
                text: "界".into(),
            },
            &mut state,
        );
        assert_eq!(state.palette.query, "a界e\u{301}b");
        assert_eq!(state.palette.cursor, 2);
        assert_eq!(document(&state).unwrap().selection, 2..2);
        edit(
            Edit::Select {
                target: Target::Palette,
                range: 1..2,
            },
            &mut state,
        );
        crate::app::handlers::key_input::handle_key(
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Backspace,
                crossterm::event::KeyModifiers::NONE,
            ),
            &mut state,
            &Config::default(),
        );
        assert_eq!(state.palette.query, "ae\u{301}b");
        assert_eq!(state.palette.cursor, 1);
    }
    #[test]
    fn credential_projection_is_masked_and_stale_text_targets_are_rejected() {
        use super::super::sources::dialogs::{Dialog, ServerForm};
        let mut state = AppState::new();
        let mut form = ServerForm::new(None);
        form.focus = 2;
        form.fields[2].value = "private😀".into();
        form.fields[2].cursor = form.fields[2].value.len();
        state
            .popups
            .set_library_dialog(Some(Dialog::Navidrome(form)));
        let editor = document(&state).unwrap();
        assert!(editor.secret);
        assert!(!editor.text.contains("private"));
        state.popups.set_library_dialog(None);
        state.palette.open = true;
        edit(
            Edit::Replace {
                target: editor.target,
                range: None,
                text: "wrong field".into(),
            },
            &mut state,
        );
        assert!(state.palette.query.is_empty());
    }
    #[test]
    fn paste_cannot_execute_shortcuts_outside_a_text_field() {
        let mut state = AppState::new();
        assert!(insert(":q^", &mut state, &Config::default()).is_empty());
        assert!(!state.should_quit);
        assert!(!state.palette.open);
    }
    #[test]
    fn hidden_description_input_cannot_own_settings_text() {
        let mut state = AppState::new();
        state.tui_description_focused = true;
        state.view = super::super::View::Settings;
        assert!(document(&state).is_none());
        assert!(insert("hidden input", &mut state, &Config::default()).is_empty());
        assert!(state.tui_description_query.is_empty());
    }
    #[test]
    fn search_paste_keeps_unicode_and_starts_one_search() {
        let mut state = AppState::new();
        state.popups.set_search_active(true);
        state.search.focus = SearchFocus::Input;
        let actions = insert("e\u{301} 界", &mut state, &Config::default());
        assert!(matches!(
            actions.as_slice(),
            [Action::Search(SearchAction::SetSearchQuery(query))] if query == "e\u{301} 界"
        ));
    }
}
