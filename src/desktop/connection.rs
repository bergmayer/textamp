//! Credential drafts stay on the command path, never in presentation snapshots.
use crate::app::settings_transfer::Progress;
use crate::app::{
    Action, AppState,
    sources::{
        SourceAction,
        dialogs::{Dialog, LocalForm, ServerForm, WebdavForm},
    },
};
use uuid::Uuid;

pub enum Draft {
    Server {
        form: Box<ServerForm>,
        library: Option<crate::app::sources::LibraryChoice>,
    },
    Webdav(WebdavForm),
    Local {
        path: String,
        name: String,
        existing_id: Option<String>,
    },
}
#[derive(Clone)]
pub struct Status {
    pub owner: Uuid,
    pub phase: Progress,
}

pub fn begin(
    owner: Uuid,
    draft: Draft,
    state: &mut AppState,
    status: &mut Option<Status>,
) -> Vec<Action> {
    if let Some(previous) = status.as_ref() {
        cancel(previous.owner, state, status);
    }
    state.clear_error();
    let action = match draft {
        Draft::Server { mut form, library } => {
            if let Some(library) = library {
                form.analysis_key = crate::app::sources::sonic::choice_key(&library, state);
            }
            form.submit().inspect(|_| {
                state
                    .popups
                    .set_library_dialog(Some(Dialog::Navidrome(*form)));
            })
        }
        Draft::Webdav(form) => form.draft().map(|draft| {
            state.popups.set_library_dialog(Some(Dialog::Webdav(form)));
            SourceAction::CheckWebdav(draft).into()
        }),
        Draft::Local {
            path,
            name,
            existing_id,
        } => {
            if let Some(id) = existing_id {
                Ok(SourceAction::Rename { id, name }.into())
            } else {
                let mut form = LocalForm::default();
                form.fields[0].value = path.clone().into();
                form.fields[1].value = name.clone().into();
                state.popups.set_library_dialog(Some(Dialog::Local(form)));
                Ok(SourceAction::SaveLocal { path, name }.into())
            }
        }
    };
    match action {
        Ok(action) => {
            *status = Some(Status {
                owner,
                phase: Progress::Checking,
            });
            vec![action]
        }
        Err(error) => {
            *status = Some(Status {
                owner,
                phase: Progress::Failed(error.to_string()),
            });
            vec![]
        }
    }
}
pub fn refresh(state: &AppState, status: &mut Option<Status>) {
    let Some(status) = status.as_mut().filter(|s| s.phase == Progress::Checking) else {
        return;
    };
    let progress = Progress::from_dialog(state.popups.library_dialog());
    status.phase = match (progress, state.notifications.last_error.as_ref()) {
        (Progress::Failed(error), _) => Progress::Failed(error),
        (_, Some(error)) => Progress::Failed(error.clone()),
        (Progress::Checking, None) => Progress::Checking,
        (Progress::Editing | Progress::Saved, None) => Progress::Saved,
    };
}
pub fn cancel(owner: Uuid, state: &mut AppState, status: &mut Option<Status>) {
    if status.as_ref().is_none_or(|s| s.owner != owner) {
        return;
    }
    if matches!(state.popups.library_dialog(), Some(Dialog::Navidrome(_))) {
        state.advance_connection_generation();
    }
    state.popups.set_library_dialog(None);
    *status = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::sources::navidrome::NavAction;
    fn server() -> Draft {
        let mut form = ServerForm::new(None);
        form.fields[0].value = "http://127.0.0.1:1".into();
        form.fields[1].value = "fixture".into();
        form.fields[2].value = "fixture-password".into();
        Draft::Server {
            form: Box::new(form),
            library: None,
        }
    }
    #[test]
    fn local_save_installs_validation_form_and_preserves_errors() {
        let mut state = AppState::new();
        let mut status = None;
        let actions = begin(
            Uuid::new_v4(),
            Draft::Local {
                path: "/missing-fixture".into(),
                name: "Fixture".into(),
                existing_id: None,
            },
            &mut state,
            &mut status,
        );
        assert!(matches!(
            actions.as_slice(),
            [Action::Source(SourceAction::SaveLocal { .. })]
        ));
        let Some(Dialog::Local(form)) = state.popups.library_dialog_mut() else {
            panic!("missing local validation form")
        };
        form.error = Some("Enter an existing folder path".into());
        refresh(&state, &mut status);
        assert!(matches!(status.unwrap().phase, Progress::Failed(_)));
        assert!(state.active_library.is_none());
    }

    #[test]
    fn draft_uses_existing_checked_save_route_without_activation() {
        let mut state = AppState::new();
        let mut status = None;
        let actions = begin(Uuid::new_v4(), server(), &mut state, &mut status);
        assert!(
            matches!(actions.as_slice(),[Action::Source(SourceAction::Navidrome(action))] if matches!(action.as_ref(),NavAction::Password {..}))
        );
        assert!(matches!(
            state.popups.library_dialog(),
            Some(Dialog::Navidrome(_))
        ));
        assert!(state.active_library.is_none());
        assert!(state.queue.tracks.is_empty());
    }
    #[test]
    fn invalid_form_stays_failed_without_an_effect() {
        let mut state = AppState::new();
        let mut status = None;
        let form = ServerForm::new(None);
        assert!(
            begin(
                Uuid::new_v4(),
                Draft::Server {
                    form: Box::new(form),
                    library: None
                },
                &mut state,
                &mut status
            )
            .is_empty()
        );
        assert!(matches!(status.unwrap().phase, Progress::Failed(_)));
    }
    #[test]
    fn cancelling_another_window_cannot_invalidate_the_editor() {
        let mut state = AppState::new();
        let mut status = None;
        let owner = Uuid::new_v4();
        begin(owner, server(), &mut state, &mut status);
        let generation = state.connection_generation;
        cancel(Uuid::new_v4(), &mut state, &mut status);
        assert_eq!(state.connection_generation, generation);
        assert!(matches!(
            state.popups.library_dialog(),
            Some(Dialog::Navidrome(_))
        ));
        assert_eq!(status.unwrap().owner, owner);
    }
    #[tokio::test]
    async fn cancellation_rejects_a_late_connection_completion() {
        let mut state = AppState::new();
        let mut status = None;
        let owner = Uuid::new_v4();
        begin(owner, server(), &mut state, &mut status);
        let generation = state.connection_generation;
        cancel(owner, &mut state, &mut status);
        assert!(state.popups.library_dialog().is_none());
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let mut config = crate::config::Config::default();
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        crate::app::dispatch::dispatch_action(
            NavAction::Connected {
                generation,
                result: Err("obsolete fixture".into()),
                password: None,
                analysis: None,
            }
            .into(),
            &mut state,
            &mut audio,
            &mut config,
            &tx,
        )
        .await
        .unwrap();
        assert!(config.navidrome_sources.is_empty());
        assert!(state.notifications.last_error.is_none());
        assert!(state.active_library.is_none());
    }
}
