use textamp::app::sources::{manager, ActiveSource, LibraryChoice};
use textamp::app::{Action, AppState};
use textamp::config::Config;
use textamp::library::{FolderLocation, FolderSource};

fn server() -> textamp::navidrome::Source {
    textamp::navidrome::Source {
        id: "server".into(),
        name: "Music".into(),
        url: "http://127.0.0.1:1".into(),
        username: "fixture".into(),
        libraries: vec![textamp::navidrome::MusicFolder {
            id: "one".into(),
            name: "Music".into(),
        }],
    }
}

#[test]
fn old_settings_load_without_creating_a_retired_backend() {
    let mut config: Config = toml::from_str(
        r#"
        [plex]
        server_url = "http://unused.invalid:32400"
        [libraries]
        default_library = "old"
        selected_server = "old-server"
    "#,
    )
    .unwrap();
    assert!(manager::startup_choice(&config).is_none());
    config.navidrome_sources.push(server());
    assert!(
        matches!(manager::startup_choice(&config), Some(LibraryChoice::Navidrome { folder: Some(id), .. }) if id == "one")
    );
    let saved = toml::to_string(&config).unwrap();
    assert!(!saved.contains("[plex]"));
    assert!(!saved.contains("unused.invalid"));
    let restored: Config = toml::from_str(&saved).unwrap();
    assert_eq!(restored.navidrome_sources[0].id, "server");
}

#[test]
fn last_folder_selection_wins_over_server_fallback() {
    let mut config = Config::default();
    config.navidrome_sources.push(server());
    config.folder_sources.push(FolderSource {
        id: "folder".into(),
        name: "Files".into(),
        location: FolderLocation::Local {
            path: "/fixture".into(),
        },
    });
    config.default_folder_source = Some("folder".into());
    assert!(
        matches!(manager::startup_choice(&config), Some(LibraryChoice::Folder(s)) if s.id == "folder")
    );
}

#[tokio::test]
async fn no_library_requests_are_explicit_errors_without_starting_tasks() {
    let mut state = AppState::new();
    assert!(matches!(state.sources.active, ActiveSource::None));
    assert_eq!(state.view, textamp::app::state::View::Browse);
    assert!(textamp::app::sources::library_choices(&state).is_empty());
    let mut audio = textamp::audio::AudioPlayer::new_without_audio();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    textamp::app::dispatch::dispatch_action(
        Action::Data(textamp::app::action::DataAction::LoadArtists),
        &mut state,
        &mut audio,
        &mut Config::default(),
        &tx,
    )
    .await
    .unwrap();
    assert!(state.notifications.last_error.is_some());
    assert!(rx.try_recv().is_err());
    assert!(!state.library_loading);
    assert!(!audio.is_playing());
}

#[tokio::test]
async fn switching_accounts_cancels_connection_work() {
    let mut state = AppState::new();
    let task = tokio::spawn(std::future::pending::<()>());
    state.sources.nav_connection_task = Some(textamp::app::tasks::TaskLease::new(&task));
    state.advance_connection_generation();
    assert!(state.sources.nav_connection_task.is_none());
    assert!(task.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn clearing_cache_without_a_library_does_not_request_a_reload() {
    let mut state = AppState::new();
    let mut audio = textamp::audio::AudioPlayer::new_without_audio();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    textamp::app::dispatch::dispatch_action(
        textamp::app::action::SettingsAction::LibraryCacheCleared(Ok(0)).into(),
        &mut state,
        &mut audio,
        &mut Config::default(),
        &tx,
    )
    .await
    .unwrap();
    tokio::task::yield_now().await;
    while let Ok(event) = rx.try_recv() {
        assert!(
            matches!(
                event,
                textamp::app::Event::Effect(Action::Settings(
                    textamp::app::action::SettingsAction::CacheSizes { .. }
                )) | textamp::app::Event::Artwork(_)
                    | textamp::app::Event::Cache(_)
            ),
            "Only disk statistics may be refreshed: {event:?}"
        );
    }
    assert!(state.notifications.last_error.is_none());
    assert!(!state.library_loading);
}
