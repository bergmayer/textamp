use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};
use textamp::app::handlers::key_input;
use textamp::app::sources::{
    self,
    dialogs::{Dialog, WebdavForm},
    ActiveSource, LibraryChoice, SourceAction,
};
use textamp::app::{Action, AppState, Event};
use textamp::config::Config;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn press(state: &mut AppState, code: KeyCode) -> Vec<Action> {
    key_input::handle_key(
        KeyEvent::new(code, KeyModifiers::NONE),
        state,
        &Config::default(),
    )
}
fn form(state: &mut AppState) -> &mut WebdavForm {
    let Some(Dialog::Webdav(form)) = &mut state.popups.library_dialog else {
        panic!("missing form")
    };
    form
}
fn fill(form: &mut WebdavForm, index: usize, value: &str) {
    form.fields[index].value = value.into();
    form.fields[index].cursor = value.len();
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
async fn add_library_chooses_type_without_touching_playback() {
    let mut state = AppState::new();
    state.sources.active = ActiveSource::Folder("playing".into());
    state.playback.position_ms = 12345;
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    dispatch(
        SourceAction::Choose(LibraryChoice::Add),
        &mut state,
        &mut Config::default(),
        &tx,
    )
    .await;
    assert!(matches!(
        state.popups.library_dialog,
        Some(Dialog::Add { .. })
    ));
    press(&mut state, KeyCode::Down);
    let actions = press(&mut state, KeyCode::Enter);
    assert!(matches!(
        actions.as_slice(),
        [Action::Source(SourceAction::Choose(
            LibraryChoice::AddWebdav
        ))]
    ));
    assert_eq!(state.playback.position_ms, 12345);
    assert_eq!(state.sources.active.folder().unwrap(), "playing");
}

#[test]
fn navidrome_form_shows_all_fields_and_cancels_inflight_sign_in() {
    use textamp::app::sources::dialogs::ServerForm;
    let mut state = AppState::new();
    state.popups.library_dialog = Some(Dialog::Navidrome(ServerForm::new(None)));
    for (i, value) in [
        "http://server:4533",
        "listener",
        "fixture-hidden-password",
        "Music",
    ]
    .iter()
    .enumerate()
    {
        for c in value.chars() {
            press(&mut state, KeyCode::Char(c));
        }
        if i != 3 {
            press(&mut state, KeyCode::Tab);
        }
    }
    let actions = press(&mut state, KeyCode::Enter);
    assert!(
        matches!(actions.as_slice(), [Action::Source(SourceAction::Navidrome(action))] if matches!(action.as_ref(), sources::navidrome::NavAction::Password { source, password } if source.username == "listener" && password.as_str() == "fixture-hidden-password"))
    );
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|f| {
            textamp::ui::render(f, &state);
        })
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    for label in [
        "URL",
        "Username",
        "Password",
        "Name (optional)",
        "Add library",
        "Cancel",
    ] {
        assert!(text.contains(label));
    }
    assert!(!text.contains("fixture-hidden-password"));
    let generation = state.connection_generation;
    if let Some(Dialog::Navidrome(form)) = &mut state.popups.library_dialog {
        form.busy = true;
    }
    press(&mut state, KeyCode::Esc);
    assert!(state.popups.library_dialog.is_none());
    assert_ne!(state.connection_generation, generation);
}

#[test]
fn form_edits_unicode_accepts_url_characters_and_keeps_underlying_navigation_closed() {
    let mut state = AppState::new();
    state.popups.library_dialog = Some(Dialog::Webdav(WebdavForm::new(None)));
    for c in "http://server:8081/音楽/".chars() {
        assert!(press(&mut state, KeyCode::Char(c)).is_empty());
    }
    assert!(!state.palette.open && !state.list_filter.active);
    press(&mut state, KeyCode::Backspace);
    press(&mut state, KeyCode::Backspace);
    assert_eq!(
        form(&mut state).fields[0].value.as_str(),
        "http://server:8081/音"
    );
    key_input::handle_key(
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
        &mut state,
        &Config::default(),
    );
    press(&mut state, KeyCode::Char('x'));
    assert_eq!(form(&mut state).fields[0].value.as_str(), "x");
    assert!(press(&mut state, KeyCode::Enter).is_empty());
    assert!(form(&mut state).error.is_some());
    assert_eq!(form(&mut state).fields[0].value.as_str(), "x");
    press(&mut state, KeyCode::BackTab);
    assert_eq!(form(&mut state).focus, 5);
    press(&mut state, KeyCode::Enter);
    assert!(state.popups.library_dialog.is_none());
}

#[test]
fn form_is_one_screen_and_never_renders_or_debug_prints_password() {
    for (width, height) in [(40, 12), (80, 24), (140, 40)] {
        let mut state = AppState::new();
        state.popups.library_dialog = Some(Dialog::Webdav(WebdavForm::new(None)));
        fill(form(&mut state), 2, "fixture-password-not-visible");
        assert!(!format!("{:?}", state.popups.library_dialog).contains("fixture-password"));
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| {
                state.hit_regions = textamp::ui::render(f, &state).hit_regions;
            })
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(!text.contains("fixture-password"));
        if height >= 24 {
            for label in [
                "URL",
                "Username",
                "Password",
                "Name (optional)",
                "[ Add library ]",
                "[ Cancel ]",
            ] {
                assert!(text.contains(label), "missing {label}");
            }
            assert_eq!(state.hit_regions.library_dialog.len(), 6);
        }
    }
}

#[tokio::test]
async fn cancelling_or_reopening_form_ignores_old_success_without_saving() {
    let mut state = AppState::new();
    let mut old = WebdavForm::new(None);
    fill(&mut old, 0, "http://example.test/");
    let draft = old.draft().unwrap();
    state.popups.library_dialog = Some(Dialog::Webdav(old));
    press(&mut state, KeyCode::Esc);
    let mut config = Config::default();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    dispatch(
        SourceAction::WebdavChecked {
            instance: draft.instance.clone(),
            result: Ok(draft.clone()),
        },
        &mut state,
        &mut config,
        &tx,
    )
    .await;
    state.popups.library_dialog = Some(Dialog::Webdav(WebdavForm::new(None)));
    dispatch(
        SourceAction::WebdavChecked {
            instance: draft.instance.clone(),
            result: Ok(draft),
        },
        &mut state,
        &mut config,
        &tx,
    )
    .await;
    assert!(config.folder_sources.is_empty());
    assert!(form(&mut state).fields[0].value.is_empty());
    assert!(rx.try_recv().is_err());
}

async fn server(status: &str, body: &str) -> (String, tokio::task::JoinHandle<Vec<u8>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 8192];
        let size = stream.read(&mut request).await.unwrap();
        request.truncate(size);
        stream.write_all(response.as_bytes()).await.unwrap();
        request
    });
    (url, task)
}

#[tokio::test]
async fn webdav_checks_real_listing_and_reports_auth_and_protocol_errors() {
    let (url, task) = server("207 Multi-Status", r#"<d:multistatus xmlns:d="DAV:"/>"#).await;
    let source = sources::source_from_location(&url).unwrap();
    textamp::library::check_webdav(&source, "").await.unwrap();
    assert!(task.await.unwrap().starts_with(b"PROPFIND / HTTP/1.1"));
    let (url, task) = server("401 Unauthorized", "").await;
    let source = sources::source_from_location(&url).unwrap();
    assert!(textamp::library::check_webdav(&source, "")
        .await
        .unwrap_err()
        .to_string()
        .contains("username and password"));
    task.await.unwrap();
    let (url, task) = server("400 Bad Request", "plain HTTP endpoint").await;
    let source = sources::source_from_location(&url.replacen("http:", "https:", 1)).unwrap();
    let error = textamp::library::check_webdav(&source, "fixture-secret")
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("Check the scheme and port"),
        "{error:#}"
    );
    let request = task.await.unwrap();
    assert!(!request.starts_with(b"PROPFIND"), "no plaintext downgrade");
    assert!(!String::from_utf8_lossy(&request).contains("fixture-secret"));
}

#[tokio::test]
async fn failed_connection_stays_in_form_and_does_not_create_library() {
    let (url, server) = server("401 Unauthorized", "").await;
    let mut state = AppState::new();
    state.popups.library_dialog = Some(Dialog::Webdav(WebdavForm::new(None)));
    fill(form(&mut state), 0, &url);
    fill(form(&mut state), 1, "listener");
    fill(form(&mut state), 2, "wrong-fixture-password");
    let draft = form(&mut state).draft().unwrap();
    let mut config = Config::default();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    dispatch(
        SourceAction::CheckWebdav(draft),
        &mut state,
        &mut config,
        &tx,
    )
    .await;
    assert!(form(&mut state).task.is_some());
    let Event::Effect(Action::Source(action)) =
        tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("expected checked result")
    };
    dispatch(action, &mut state, &mut config, &tx).await;
    assert!(form(&mut state).error.as_ref().unwrap().contains("401"));
    assert_eq!(
        form(&mut state).fields[2].value.as_str(),
        "wrong-fixture-password"
    );
    assert!(form(&mut state).task.is_none());
    assert!(config.folder_sources.is_empty() && state.sources.folders.is_empty());
    server.await.unwrap();
}

#[test]
fn verified_save_uses_private_isolated_storage() {
    let root = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "verified_save_worker", "--nocapture"])
        .env("TEXTAMP_DIALOG_TEST", "1")
        .env("XDG_DATA_HOME", root.path())
        .env("XDG_CONFIG_HOME", root.path())
        .env("XDG_CACHE_HOME", root.path())
        .env("XDG_STATE_HOME", root.path())
        .status()
        .unwrap();
    assert!(status.success());
}

#[tokio::test]
async fn verified_save_worker() {
    if std::env::var("TEXTAMP_DIALOG_TEST").as_deref() != Ok("1") {
        return;
    }
    let (url, server) = server("207 Multi-Status", r#"<d:multistatus xmlns:d="DAV:"/>"#).await;
    let mut state = AppState::new();
    state.popups.library_dialog = Some(Dialog::Webdav(WebdavForm::new(None)));
    fill(form(&mut state), 0, &url);
    fill(form(&mut state), 1, "listener");
    fill(form(&mut state), 2, "fixture-secret");
    let draft = form(&mut state).draft().unwrap();
    let id = draft.source.id.clone();
    let mut config = Config::default();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    dispatch(
        SourceAction::CheckWebdav(draft),
        &mut state,
        &mut config,
        &tx,
    )
    .await;
    assert!(config.folder_sources.is_empty());
    let Event::Effect(Action::Source(action)) =
        tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("expected checked result")
    };
    dispatch(action, &mut state, &mut config, &tx).await;
    assert!(state.popups.library_dialog.is_none());
    assert_eq!(config.folder_sources.len(), 1);
    assert_eq!(config.folder_sources[0].id, id);
    assert_eq!(
        textamp::library::credentials::load(&id)
            .unwrap()
            .unwrap()
            .as_str(),
        "fixture-secret"
    );
    assert!(!toml::to_string(&config).unwrap().contains("fixture-secret"));
    assert!(textamp::app::tasks::finish_blocking(std::time::Duration::from_secs(5)).await);
    server.await.unwrap();

    // A private-store failure must leave the form open and configuration intact.
    let credential_path = textamp::config::XdgPaths::new("textamp")
        .data_dir
        .join("folder-credentials.toml");
    std::fs::write(&credential_path, "[invalid fixture").unwrap();
    let mut failed = WebdavForm::new(None);
    fill(&mut failed, 0, "http://example.test/");
    fill(&mut failed, 1, "listener");
    fill(&mut failed, 2, "fixture-secret");
    let draft = failed.draft().unwrap();
    state.popups.library_dialog = Some(Dialog::Webdav(failed));
    dispatch(
        SourceAction::WebdavChecked {
            instance: draft.instance.clone(),
            result: Ok(draft),
        },
        &mut state,
        &mut config,
        &tx,
    )
    .await;
    assert!(form(&mut state)
        .error
        .as_ref()
        .unwrap()
        .contains("Save credentials"));
    assert_eq!(config.folder_sources.len(), 1);
    assert_eq!(
        std::fs::read_to_string(credential_path).unwrap(),
        "[invalid fixture"
    );
}

#[tokio::test]
async fn cancel_aborts_an_in_flight_connection_check() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let mut state = AppState::new();
    state.popups.library_dialog = Some(Dialog::Webdav(WebdavForm::new(None)));
    fill(form(&mut state), 0, &url);
    let draft = form(&mut state).draft().unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let mut config = Config::default();
    dispatch(
        SourceAction::CheckWebdav(draft),
        &mut state,
        &mut config,
        &tx,
    )
    .await;
    let (mut socket, _) =
        tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
            .await
            .unwrap()
            .unwrap();
    let mut bytes = [0; 4096];
    assert!(socket.read(&mut bytes).await.unwrap() > 0);
    press(&mut state, KeyCode::Esc);
    // Cancellation closes the HTTP request without waiting for the 30-second deadline.
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if socket.read(&mut bytes).await.unwrap_or(0) == 0 {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(rx.try_recv().is_err());
    assert!(config.folder_sources.is_empty());
}

#[test]
fn dialog_mouse_buttons_work_and_clicks_do_not_reach_the_player() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use textamp::app::handlers::mouse_input;
    let mut state = AppState::new();
    state.popups.library_dialog = Some(Dialog::Webdav(WebdavForm::new(None)));
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|f| {
            state.hit_regions = textamp::ui::render(f, &state).hit_regions;
        })
        .unwrap();
    let click = |x, y| MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    };
    assert!(mouse_input::handle_mouse(click(0, 23), &mut state).is_empty());
    assert!(state.popups.library_dialog.is_some());
    let cancel = state.hit_regions.library_dialog[5];
    assert!(mouse_input::handle_mouse(click(cancel.x, cancel.y), &mut state).is_empty());
    assert!(state.popups.library_dialog.is_none());
}
