//! Settings location and staged source forms shared by windowed presentations.
use super::{AppState, View};

/// Validation progress contains no credentials and can be projected to either frontend.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Progress {
    #[default]
    Editing,
    Checking,
    Saved,
    Failed(String),
}
impl Progress {
    /// Read validation state from the source-owned form.
    pub fn from_dialog(dialog: Option<&super::sources::dialogs::Dialog>) -> Self {
        use super::sources::dialogs::Dialog;
        let (busy, error) = match dialog {
            Some(Dialog::Navidrome(form)) => (form.busy, form.error.as_ref()),
            Some(Dialog::Webdav(form)) => (form.task.is_some(), form.error.as_ref()),
            Some(Dialog::Local(form)) => (false, form.error.as_ref()),
            _ => (false, None),
        };
        if busy {
            Self::Checking
        } else if let Some(error) = error {
            Self::Failed(error.clone())
        } else {
            Self::Editing
        }
    }
}

/// A staged settings workflow; values are not saved or validated by a mode switch.
#[derive(Clone, Debug)]
pub struct Draft {
    pub progress: Progress,
    pub category: usize,
    pub item: usize,
    pub focus: super::state::SettingsFocus,
    pub dialog: Option<super::sources::dialogs::Dialog>,
}
impl Draft {
    /// Retain the current logical settings destination and unfinished source form.
    pub fn capture(state: &AppState) -> Option<Self> {
        (state.view == View::Settings || state.popups.library_dialog().is_some()).then(|| Self {
            progress: Progress::from_dialog(state.popups.library_dialog()),
            category: state.settings_state.category_index(),
            item: state.settings_state.item_index,
            focus: state.settings_state.focus,
            dialog: state.popups.library_dialog().cloned(),
        })
    }
    /// Keep the existing request identity and lease when a native form is validating.
    pub fn retain_pending_request(&mut self, state: &AppState) {
        if Progress::from_dialog(state.popups.library_dialog()) == Progress::Checking {
            self.dialog = state.popups.library_dialog().cloned();
            self.progress = Progress::Checking;
        }
    }
    /// Restore a settings workflow without opening a source, submitting or saving.
    pub fn restore(&self, state: &mut AppState) {
        if state.view != View::Settings {
            state.previous_view = Some(state.view);
        }
        state.view = View::Settings;
        state.settings_state.select_category(self.category);
        state.settings_state.item_index = self.item;
        state.settings_state.focus = self.focus;
        state.popups.set_library_dialog(self.dialog.clone());
    }
}

/// Update a parked native form from shared validation without retaining an invisible input popup.
pub fn refresh_parked(state: &mut AppState) {
    let progress = Progress::from_dialog(state.popups.library_dialog());
    let dialog = state.popups.library_dialog().cloned();
    let Some(draft) = state.staged_settings.as_mut() else {
        return;
    };
    if progress != Progress::Checking && draft.progress != Progress::Checking {
        return;
    }
    draft.progress = if dialog.is_none() {
        Progress::Saved
    } else {
        progress
    };
    if let Some(dialog) = dialog {
        draft.dialog = Some(dialog);
    }
    if draft.progress != Progress::Checking {
        use super::sources::dialogs::Dialog;
        match draft.dialog.as_mut() {
            Some(Dialog::Navidrome(form)) => form.busy = false,
            Some(Dialog::Webdav(form)) => form.task = None,
            _ => {}
        }
        state.popups.set_library_dialog(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn pending_validation_keeps_its_instance_and_reports_completion_to_the_parked_form() {
        use super::super::{
            sources::dialogs::{Dialog, WebdavForm},
            tasks::TaskLease,
        };
        let mut state = AppState::new();
        state.view = View::Settings;
        let task = tokio::spawn(std::future::pending::<()>());
        let mut form = WebdavForm::new(None);
        let instance = form.instance.clone();
        form.task = Some(TaskLease::new(&task));
        state.popups.set_library_dialog(Some(Dialog::Webdav(form)));
        let mut draft = Draft::capture(&state).unwrap();
        draft.dialog = Some(Dialog::Webdav(WebdavForm::new(None)));
        draft.retain_pending_request(&state);
        let Some(Dialog::Webdav(form)) = draft.dialog.as_ref() else {
            panic!("validation form")
        };
        assert_eq!(form.instance, instance);
        assert!(form.task.is_some());
        state.staged_settings = Some(draft);
        refresh_parked(&mut state);
        assert_eq!(
            state.staged_settings.as_ref().unwrap().progress,
            Progress::Checking
        );
        let Some(Dialog::Webdav(form)) = state.popups.library_dialog_mut() else {
            panic!("validation form")
        };
        form.task = None;
        form.error = Some("Server unavailable".into());
        refresh_parked(&mut state);
        assert_eq!(
            state.staged_settings.as_ref().unwrap().progress,
            Progress::Failed("Server unavailable".into())
        );
        assert!(state.popups.library_dialog().is_none());
        task.abort();
    }
    #[test]
    fn a_settings_draft_retains_fields_and_location_without_changing_the_library() {
        let mut state = AppState::new();
        state.active_library = Some("same-library".into());
        state.view = View::Settings;
        let mut form = super::super::sources::dialogs::ServerForm::new(None);
        form.fields[0].value = "unfinished address".into();
        form.fields[0].cursor = 4;
        state
            .popups
            .set_library_dialog(Some(super::super::sources::dialogs::Dialog::Navidrome(
                form,
            )));
        state.settings_state.select_category(2);
        state.settings_state.focus = super::super::state::SettingsFocus::Content;
        let draft = Draft::capture(&state).unwrap();
        state.popups.set_library_dialog(None);
        state.view = View::Browse;
        state.settings_state.select_category(0);
        draft.restore(&mut state);
        assert_eq!(state.active_library.as_deref(), Some("same-library"));
        assert_eq!(state.settings_state.category_index(), 2);
        assert_eq!(
            state.settings_state.focus,
            super::super::state::SettingsFocus::Content
        );
        let Some(super::super::sources::dialogs::Dialog::Navidrome(form)) =
            state.popups.library_dialog()
        else {
            panic!("staged source form");
        };
        assert_eq!(form.fields[0].value.as_str(), "unfinished address");
        assert_eq!(form.fields[0].cursor, 4);
        assert!(!form.busy);
    }
}
