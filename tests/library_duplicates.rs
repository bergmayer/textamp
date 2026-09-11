use textamp::app::sources::{
    self,
    dialogs::{Dialog, WebdavForm},
    navidrome::NavAction,
    SourceAction,
};
use textamp::app::{AppState, Event};
use textamp::config::Config;
use textamp::library::{FolderLocation, FolderSource};

fn webdav(id: &str, url: &str, username: Option<&str>) -> FolderSource {
    FolderSource {
        id: id.into(),
        name: id.into(),
        location: FolderLocation::Webdav {
            url: url.into(),
            username: username.map(str::to_owned),
            password_env: None,
        },
    }
}

#[test]
fn webdav_identity_ignores_names_secrets_and_url_spelling_but_preserves_accounts_and_paths() {
    let mut original = webdav("original", "http://EXAMPLE.test:80/music", Some("listener"));
    if let FolderLocation::Webdav { password_env, .. } = &mut original.location {
        *password_env = Some("FIXTURE_PASSWORD".into());
    }
    let duplicate = webdav("renamed", "http://example.test/music/", Some("listener"));
    assert!(original.same_library(&duplicate));
    for (url, user) in [
        ("http://example.test/music/", Some("another-account")),
        ("http://example.test/Music/", Some("listener")),
        ("http://example.test/other/", Some("listener")),
        ("https://example.test/music/", Some("listener")),
        ("http://example.test:8080/music/", Some("listener")),
    ] {
        assert!(!original.same_library(&webdav("other", url, user)));
    }
    assert!(
        webdav("a", "http://example.test", None).same_library(&webdav(
            "b",
            "http://example.test/",
            Some("")
        ))
    );
}

async fn dispatch(
    action: SourceAction,
    state: &mut AppState,
    config: &mut Config,
    tx: &tokio::sync::mpsc::Sender<Event>,
) {
    sources::dispatch(
        action,
        state,
        &mut textamp::audio::AudioPlayer::new_without_audio(),
        config,
        tx,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn duplicate_webdav_add_edit_and_late_result_do_not_start_work_or_change_config() {
    for editing in [false, true] {
        for completed in [false, true] {
            let saved = webdav("saved", "http://EXAMPLE.test:80/music", Some("listener"));
            let candidate = webdav("new", "http://example.test/music/", Some("listener"));
            let mut form = WebdavForm::new(Some(&candidate));
            form.editing = editing;
            let draft = form.draft().unwrap();
            let mut state = AppState::new();
            state.popups.library_dialog = Some(Dialog::Webdav(form));
            let mut config = Config::default();
            config.folder_sources.push(saved.clone());
            if editing {
                config.folder_sources.push(webdav(
                    "new",
                    "http://example.test/other/",
                    Some("listener"),
                ));
            }
            let before = config.folder_sources.clone();
            let (tx, mut rx) = tokio::sync::mpsc::channel(8);
            let action = if completed {
                SourceAction::WebdavChecked {
                    instance: draft.instance.clone(),
                    result: Ok(draft),
                }
            } else {
                SourceAction::CheckWebdav(draft)
            };
            dispatch(action, &mut state, &mut config, &tx).await;
            let Some(Dialog::Webdav(form)) = &state.popups.library_dialog else {
                panic!("lost form")
            };
            assert_eq!(
                form.error.as_deref(),
                Some("That WebDAV library is already added")
            );
            assert!(form.task.is_none());
            assert_eq!(config.folder_sources, before);
            assert!(rx.try_recv().is_err());
        }
    }
}

#[tokio::test]
async fn editing_the_existing_webdav_connection_is_allowed() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let saved = webdav(
        "saved",
        &format!("http://{}/", listener.local_addr().unwrap()),
        None,
    );
    let form = WebdavForm::new(Some(&saved));
    let draft = form.draft().unwrap();
    let mut state = AppState::new();
    state.popups.library_dialog = Some(Dialog::Webdav(form));
    let mut config = Config::default();
    config.folder_sources.push(saved);
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    dispatch(
        SourceAction::CheckWebdav(draft),
        &mut state,
        &mut config,
        &tx,
    )
    .await;
    let Some(Dialog::Webdav(form)) = &state.popups.library_dialog else {
        panic!("lost form")
    };
    assert!(form.error.is_none() && form.task.is_some());
    state.popups.library_dialog = None; // Cancel the read-only check.
}

#[tokio::test]
async fn local_add_rejects_resolved_duplicate_paths() {
    let root = tempfile::tempdir().unwrap();
    let music = root.path().join("music");
    std::fs::create_dir(&music).unwrap();
    std::fs::create_dir(music.join("subfolder")).unwrap();
    let mut config = Config::default();
    config
        .folder_sources
        .push(sources::source_from_location(music.to_str().unwrap()).unwrap());
    let mut variants = vec![music.join("."), music.join("subfolder/..")];
    #[cfg(unix)]
    {
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&music, &alias).unwrap();
        variants.push(alias);
    }
    for path in variants {
        let mut state = AppState::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        dispatch(
            SourceAction::AddLocation(path.to_str().unwrap().into()),
            &mut state,
            &mut config,
            &tx,
        )
        .await;
        assert_eq!(
            state.notifications.last_error.as_deref(),
            Some("That folder library is already added")
        );
        assert_eq!(config.folder_sources.len(), 1);
        assert!(rx.try_recv().is_err());
    }
}

#[tokio::test]
async fn navidrome_rejects_duplicate_account_before_connecting_and_on_completion() {
    use textamp::app::sources::navidrome::Session;
    let saved = textamp::navidrome::Source {
        id: "saved".into(),
        name: "Music".into(),
        url: "http://EXAMPLE.test:80/music/".into(),
        username: "listener".into(),
        libraries: vec![],
    };
    let candidate = textamp::navidrome::Source {
        id: "new".into(),
        name: "Renamed".into(),
        url: "http://example.test/music".into(),
        ..saved.clone()
    };
    assert!(saved.same_account(&candidate));
    let mut different = candidate.clone();
    different.username = "another-account".into();
    assert!(!saved.same_account(&different));
    different = candidate.clone();
    different.url = "http://example.test/other".into();
    assert!(!saved.same_account(&different));
    for completed in [false, true] {
        let mut config = Config::default();
        config.navidrome_sources.push(saved.clone());
        let before = toml::to_string(&config).unwrap();
        let mut state = AppState::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let action = if completed {
            NavAction::Connected {
                password: None,
                generation: state.connection_generation,
                result: Ok(Session {
                    client: textamp::navidrome::Client::new(&candidate, "fixture".into(), None)
                        .unwrap(),
                    source: candidate.clone(),
                    extensions: Default::default(),
                }),
            }
        } else {
            NavAction::Password {
                source: candidate.clone(),
                password: "fixture".into(),
            }
        };
        dispatch(
            SourceAction::Navidrome(Box::new(action)),
            &mut state,
            &mut config,
            &tx,
        )
        .await;
        assert_eq!(
            state.notifications.last_error.as_deref(),
            Some("That Navidrome account is already added")
        );
        assert!(state.sources.nav_connection_task.is_none());
        assert_eq!(toml::to_string(&config).unwrap(), before);
        assert!(rx.try_recv().is_err());
    }
}
