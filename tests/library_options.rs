use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::TestBackend, Terminal};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use textamp::app::action::SettingsAction;
use textamp::app::dispatch::{dispatch_action, handle_core_event};
use textamp::app::handlers::{key_input, mouse_input};
use textamp::app::sources::{
    cache,
    dialogs::Dialog,
    options::{self, OptionAction},
    ActiveSource, LibraryChoice, SourceAction,
};
use textamp::app::state::{PlayStatus, SettingsFocus, SettingsSection, View};
use textamp::app::{Action, AppState, Event};
use textamp::audio::AudioPlayer;
use textamp::config::Config;
use textamp::library::{
    cache::REFRESH_INTERVAL,
    tree::{Tree, KEY},
    FolderLocation, FolderSource,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

fn source(path: &std::path::Path) -> FolderSource {
    FolderSource {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Fixture music".into(),
        location: FolderLocation::Local { path: path.into() },
    }
}
fn press(state: &mut AppState, key: KeyCode) -> Vec<Action> {
    key_input::handle_key(
        KeyEvent::new(key, KeyModifiers::NONE),
        state,
        &Config::default(),
    )
}
fn select(state: &mut AppState, action: OptionAction) {
    let Some(Dialog::Library(options)) = &state.popups.library_dialog else {
        panic!("No library options")
    };
    let index = options
        .items(state)
        .iter()
        .position(|(a, _)| *a == action)
        .unwrap();
    if let Some(Dialog::Library(options)) = &mut state.popups.library_dialog {
        options.selected = index;
    }
}
fn draw(state: &mut AppState, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|f| state.apply_render_feedback(textamp::ui::render(f, state)))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}

#[test]
fn enter_manages_and_only_make_active_switches_mouse_and_keyboard_agree() {
    let mut state = AppState::new();
    let library = source(std::path::Path::new("/fixture"));
    let choice = LibraryChoice::Folder(library.clone());
    state.sources.folders = vec![library];
    state.sources.active = ActiveSource::Folder("playing-elsewhere".into());
    state.playback.position_ms = 42_000;
    state.playback.status = PlayStatus::Paused;
    state.view = View::Settings;
    state.settings_state.section = SettingsSection::Libraries;
    state.settings_state.focus = SettingsFocus::Content;
    assert!(matches!(
        press(&mut state, KeyCode::Enter).as_slice(),
        [Action::Settings(SettingsAction::RefreshCacheStats)]
    ));
    let text = draw(&mut state, 110, 35);
    assert!(text.contains("Make active"));
    assert!(text.contains("Clear library cache"));
    assert!(text.contains("Re-scan cache"));
    assert_eq!(
        state.sources.active.folder().map(String::as_str),
        Some("playing-elsewhere")
    );
    assert_eq!(state.playback.position_ms, 42_000);
    select(&mut state, OptionAction::Activate);
    let rect = state.hit_regions.library_dialog[0];
    let click = mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        },
        &mut state,
    );
    assert!(
        matches!(click.as_slice(), [Action::Source(SourceAction::Choose(LibraryChoice::Folder(s)))] if s.id == match &choice {LibraryChoice::Folder(s) => &s.id, _ => unreachable!()}.as_str())
    );
    options::open(choice, &mut state);
    select(&mut state, OptionAction::Activate);
    assert!(matches!(
        press(&mut state, KeyCode::Enter).as_slice(),
        [Action::Source(SourceAction::Choose(_))]
    ));
    assert_eq!(state.playback.status, PlayStatus::Paused);
}

#[test]
fn clear_confirmation_captures_input_and_returns_to_library_options() {
    let mut state = AppState::new();
    options::open(
        LibraryChoice::Folder(source(std::path::Path::new("/fixture"))),
        &mut state,
    );
    select(&mut state, OptionAction::Clear);
    press(&mut state, KeyCode::Enter);
    assert!(state.popups.confirm_dialog.is_some());
    assert!(matches!(
        state.popups.library_dialog,
        Some(Dialog::Library(_))
    ));
    press(&mut state, KeyCode::Esc);
    assert!(state.popups.confirm_dialog.is_none());
    assert!(matches!(
        state.popups.library_dialog,
        Some(Dialog::Library(_))
    ));
    for (width, height) in [(30, 10), (60, 16), (110, 35)] {
        select(&mut state, OptionAction::Close);
        draw(&mut state, width, height);
        let Some(Dialog::Library(options)) = &state.popups.library_dialog else {
            unreachable!()
        };
        let rect = state.hit_regions.library_dialog[options.selected];
        assert!(rect.height > 0 && rect.bottom() <= height);
    }
    press(&mut state, KeyCode::Esc);
    assert!(state.popups.library_dialog.is_none());
}

async fn receive(
    state: &mut AppState,
    audio: &mut AudioPlayer,
    tx: &mpsc::Sender<Event>,
    rx: &mut mpsc::Receiver<Event>,
) {
    let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    for action in handle_core_event(event, state, tx) {
        dispatch_action(action, state, audio, &mut Config::default(), tx)
            .await
            .unwrap();
    }
}
async fn settle(
    choice: &LibraryChoice,
    state: &mut AppState,
    audio: &mut AudioPlayer,
    tx: &mpsc::Sender<Event>,
    rx: &mut mpsc::Receiver<Event>,
) {
    loop {
        let store = choice.cache_store().unwrap();
        if state
            .settings_state
            .cache_scans
            .get(store.scope_key())
            .is_some_and(|s| matches!(s.work, cache::Work::Idle | cache::Work::Cleared))
        {
            break;
        }
        receive(state, audio, tx, rx).await;
    }
}

#[tokio::test]
async fn inactive_scan_clear_and_failure_preserve_other_cache_and_playback() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("Artist/Album")).unwrap();
    std::fs::write(dir.path().join("Artist/Album/01.flac"), b"fixture").unwrap();
    let choice = LibraryChoice::Folder(source(dir.path()));
    let other = LibraryChoice::Folder(source(dir.path()));
    let other_store = other.cache_store().unwrap();
    other_store.ticket("sentinel").write(&42).unwrap();
    let mut state = AppState::new();
    state.sources.active = ActiveSource::Folder("unrelated-playing".into());
    state.playback.position_ms = 123_000;
    state.playback.status = PlayStatus::Paused;
    let (tx, mut rx) = mpsc::channel(64);
    let mut audio = AudioPlayer::new_without_audio();
    cache::start(choice.clone(), true, &mut state, &tx);
    let request = state.settings_state.scan_request;
    assert!(!cache::completed(
        &choice,
        request.wrapping_sub(1),
        &Ok(cache::ScanResult {
            updated: 123,
            warning: None
        }),
        &mut state
    ));
    settle(&choice, &mut state, &mut audio, &tx, &mut rx).await;
    let store = choice.cache_store().unwrap();
    let tree = store
        .ticket(KEY)
        .read::<Tree>(REFRESH_INTERVAL)
        .unwrap()
        .unwrap();
    assert_eq!(tree.value.listing("Artist").unwrap()[0].name, "Album");
    assert!(tree.value.listing("Artist/Album").is_none());
    assert!(store.bytes().unwrap() > 0);
    let previous = store.bytes().unwrap();
    // A failed re-scan must not replace the complete snapshot.
    std::fs::remove_file(dir.path().join("Artist/Album/01.flac")).unwrap();
    std::fs::remove_dir(dir.path().join("Artist/Album")).unwrap();
    std::fs::remove_dir(dir.path().join("Artist")).unwrap();
    std::fs::remove_dir(dir.path()).unwrap();
    cache::start(choice.clone(), true, &mut state, &tx);
    settle(&choice, &mut state, &mut audio, &tx, &mut rx).await;
    assert_eq!(store.bytes().unwrap(), previous);
    assert!(cache::status(&choice, &state).contains("failed"));
    cache::clear(choice.clone(), &mut state, &tx);
    settle(&choice, &mut state, &mut audio, &tx, &mut rx).await;
    assert_eq!(store.bytes().unwrap(), 0);
    assert_eq!(
        other_store
            .ticket("sentinel")
            .read::<u32>(REFRESH_INTERVAL)
            .unwrap()
            .unwrap()
            .value,
        42
    );
    assert_eq!(
        state.sources.active.folder().map(String::as_str),
        Some("unrelated-playing")
    );
    assert_eq!(state.playback.position_ms, 123_000);
    assert_eq!(state.playback.status, PlayStatus::Paused);
}

#[tokio::test]
async fn webdav_scan_uses_only_propfind_and_branch_cache_survives_offline() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let captured = captured.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0; 8192];
                let n = stream.read(&mut buffer).await.unwrap();
                let line = String::from_utf8_lossy(&buffer[..n])
                    .lines()
                    .next()
                    .unwrap()
                    .to_owned();
                let path = line.split_whitespace().nth(1).unwrap();
                let children = match path {
                    "/" => vec![("/Artist/", true)],
                    "/Artist/" => vec![("/Artist/Album/", true)],
                    "/Artist/Album/" => vec![("/Artist/Album/01.flac", false)],
                    _ => panic!("Unexpected path {path}"),
                };
                let mut body = String::from("<d:multistatus xmlns:d=\"DAV:\">");
                for (href, directory) in children {
                    let kind = if directory { "<d:collection/>" } else { "" };
                    body.push_str(&format!("<d:response><d:href>{href}</d:href><d:propstat><d:prop><d:resourcetype>{kind}</d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"));
                }
                body.push_str("</d:multistatus>");
                captured.lock().unwrap().push(line);
                let response = format!("HTTP/1.1 207 Multi-Status\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(response.as_bytes()).await.unwrap();
            });
        }
    });
    let choice = LibraryChoice::Folder(FolderSource {
        id: uuid::Uuid::new_v4().to_string(),
        name: "WebDAV fixture".into(),
        location: FolderLocation::Webdav {
            url,
            username: None,
            password_env: None,
        },
    });
    let mut state = AppState::new();
    let (tx, mut rx) = mpsc::channel(64);
    let mut audio = AudioPlayer::new_without_audio();
    cache::start(choice.clone(), true, &mut state, &tx);
    settle(&choice, &mut state, &mut audio, &tx, &mut rx).await;
    server.abort();
    assert_eq!(requests.lock().unwrap().len(), 3);
    assert!(requests
        .lock()
        .unwrap()
        .iter()
        .all(|r| r.starts_with("PROPFIND ")));
    let store = choice.cache_store().unwrap();
    assert_eq!(
        cache::read_folder(&store, "Artist").unwrap().unwrap().value[0].path,
        "Artist/Album"
    );
    assert!(cache::read_folder(&store, "Artist/Album")
        .unwrap()
        .is_none());
    // Fresh saved tree is reused, even in a new app session with no server.
    state.settings_state.cache_scans.clear();
    cache::start(choice.clone(), false, &mut state, &tx);
    settle(&choice, &mut state, &mut audio, &tx, &mut rx).await;
    assert_eq!(cache::status(&choice, &state), "Cache up to date");
}

#[tokio::test]
async fn clear_cancels_scan_and_late_completion_cannot_repopulate_or_switch() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let choice = LibraryChoice::Folder(FolderSource {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Slow fixture".into(),
        location: FolderLocation::Webdav {
            url: format!("http://{}", listener.local_addr().unwrap()),
            username: None,
            password_env: None,
        },
    });
    let store = choice.cache_store().unwrap();
    store.ticket(KEY).write(&Tree::default()).unwrap();
    let mut state = AppState::new();
    let LibraryChoice::Folder(source) = &choice else {
        unreachable!()
    };
    state.sources.folders = vec![source.clone()];
    state.sources.active = ActiveSource::Folder(source.id.clone());
    let mut folders =
        textamp::library::models::FolderNavigationState::for_library("fixture".into());
    folders.loading = true;
    state.folder_state = Some(folders);
    let (tx, mut rx) = mpsc::channel(64);
    let mut audio = AudioPlayer::new_without_audio();
    cache::start(choice.clone(), true, &mut state, &tx);
    let request = state.settings_state.scan_request;
    let (_connection, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
        .await
        .unwrap()
        .unwrap();
    cache::clear(choice.clone(), &mut state, &tx);
    assert!(!cache::completed(
        &choice,
        request,
        &Ok(cache::ScanResult {
            updated: 123,
            warning: None
        }),
        &mut state
    ));
    settle(&choice, &mut state, &mut audio, &tx, &mut rx).await;
    assert_eq!(store.bytes().unwrap(), 0);
    assert!(cache::maintenance_paused(&state));
    assert!(!state.folder_state.as_ref().unwrap().loading);
    textamp::app::sources::refresh_due_caches(&mut state, &tx);
    assert!(state.sources.listing.is_none());
    assert!(!cache::running(&choice, &state));
    cache::resume(&choice, &mut state);
    assert!(!cache::maintenance_paused(&state));
}
