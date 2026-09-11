use std::time::Duration;
use textamp::app::action::{FolderAction, SettingsAction};
use textamp::app::dispatch::{dispatch_action, handle_core_event};
use textamp::app::sources::{self, ActiveSource};
use textamp::app::state::{BrowseCategory, View};
use textamp::app::{AppState, Event};
use textamp::audio::AudioPlayer;
use textamp::config::Config;
use textamp::library::{cache::Store, FolderEntry, FolderLocation, FolderSource};

use tokio::sync::mpsc;

#[test]
fn settings_have_one_library_account_home_and_cache_controls_for_each_provider() {
    use ratatui::{backend::TestBackend, Terminal};
    use textamp::app::state::{SettingsFocus, SettingsSection};
    assert_eq!(SettingsSection::default(), SettingsSection::Libraries);
    assert_eq!(SettingsSection::all().len(), 3);
    for (index, section) in SettingsSection::all().iter().enumerate() {
        assert_eq!(section.next(), SettingsSection::all()[(index + 1) % 3]);
        assert_eq!(section.next().prev(), *section);
        assert_ne!(section.name(), "account");
    }
    for active in [ActiveSource::None, ActiveSource::Folder("folder".into())] {
        let mut state = AppState::new();
        state.sources.active = active;
        state.view = View::Settings;
        state.settings_state.section = SettingsSection::Libraries;
        state.settings_state.focus = SettingsFocus::Content;
        textamp::app::sources::options::open(
            textamp::app::sources::LibraryChoice::Folder(FolderSource {
                id: "fixture".into(),
                name: "Fixture".into(),
                location: FolderLocation::Local {
                    path: "/fixture".into(),
                },
            }),
            &mut state,
        );
        let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();
        terminal
            .draw(|frame| {
                textamp::ui::render(frame, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("Clear shared artwork"));
        assert!(!text.contains("Cache all subfolders"));
    }
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
fn entry(name: &str) -> FolderEntry {
    FolderEntry {
        path: name.into(),
        name: name.into(),
        directory: false,
    }
}

#[tokio::test]
async fn all_folder_types_open_saved_listings_without_a_live_source() {
    let dir = tempfile::tempdir().unwrap();
    let locations = [
        FolderLocation::Local {
            path: dir.path().join("offline"),
        },
        FolderLocation::Webdav {
            url: "http://127.0.0.1:1/music/".into(),
            username: Some("cache-fixture".into()),
            password_env: None,
        },
    ];
    for location in locations {
        let source = FolderSource {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Cached music".into(),
            location,
        };
        let cache = Store::folder(&source).unwrap();
        cache
            .ticket("")
            .write(&vec![entry("01.flac"), entry("02.flac")])
            .unwrap();
        let mut state = AppState::new();
        state.view = View::Browse;
        state.browse_category = BrowseCategory::Folders;
        state.sources.active = ActiveSource::Folder(source.id.clone());
        state.sources.folders = vec![source.clone()];

        let mut audio = AudioPlayer::new_without_audio();
        let (tx, mut rx) = mpsc::channel(16);
        sources::folders(FolderAction::LoadFolderRoot, &mut state, &mut audio, &tx);
        while state.sources.listing.is_some() {
            receive(&mut state, &mut audio, &tx, &mut rx).await;
        }
        assert_eq!(
            state.folder_state.as_ref().unwrap().columns[0].items.len(),
            2
        );
        assert!(
            !state.library_loading,
            "fresh local and WebDAV caches use the same weekly policy"
        );
        assert!(state.notifications.last_error.is_none());
        sources::folders(
            FolderAction::RefreshSubfolder(String::new()),
            &mut state,
            &mut audio,
            &tx,
        );
        while state.sources.listing.is_some() {
            receive(&mut state, &mut audio, &tx, &mut rx).await;
        }
        assert!(state.notifications.last_error.is_some());
        assert_eq!(
            state.folder_state.as_ref().unwrap().columns[0].items.len(),
            2,
            "failed manual refresh keeps cached listings"
        );
        assert!(!state.library_loading);
        assert!(state.sources.listing.is_none());
        let mut renamed = source.clone();
        renamed.name = "Renamed".into();
        assert!(Store::folder(&renamed)
            .unwrap()
            .ticket("")
            .read::<Vec<FolderEntry>>(Duration::ZERO)
            .unwrap()
            .is_some());
        renamed.id = uuid::Uuid::new_v4().to_string();
        assert!(Store::folder(&renamed)
            .unwrap()
            .ticket("")
            .read::<Vec<FolderEntry>>(Duration::ZERO)
            .unwrap()
            .is_none());
    }
}

#[tokio::test]
async fn root_f5_forces_a_read_and_successful_empty_refresh_replaces_cache() {
    let dir = tempfile::tempdir().unwrap();
    let source = FolderSource {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Local".into(),
        location: FolderLocation::Local {
            path: dir.path().into(),
        },
    };
    let cache = Store::folder(&source).unwrap();
    cache
        .ticket("")
        .write(&vec![entry("removed.flac")])
        .unwrap();
    let mut state = AppState::new();
    state.view = View::Browse;
    state.browse_category = BrowseCategory::Folders;
    state.sources.active = ActiveSource::Folder(source.id.clone());
    state.sources.folders = vec![source];

    let mut audio = AudioPlayer::new_without_audio();
    let (tx, mut rx) = mpsc::channel(16);
    let actions = textamp::app::handlers::key_input::handle_key(
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::F(5),
            crossterm::event::KeyModifiers::NONE,
        ),
        &mut state,
        &Config::default(),
    );
    for action in actions {
        dispatch_action(action, &mut state, &mut audio, &mut Config::default(), &tx)
            .await
            .unwrap();
    }
    // Background tree/cache events may arrive before the requested listing.
    assert!(state.library_loading);
    while state.library_loading {
        receive(&mut state, &mut audio, &tx, &mut rx).await;
    }
    assert!(!state.library_loading);
    assert!(state.folder_state.as_ref().unwrap().columns[0]
        .items
        .is_empty());
    assert!(cache
        .ticket("")
        .read::<Vec<FolderEntry>>(Duration::ZERO)
        .unwrap()
        .unwrap()
        .value
        .is_empty());
    // Clear completion must reload this provider, not send its identity to Plex.
    dispatch_action(
        SettingsAction::LibraryCacheCleared(Ok(1)).into(),
        &mut state,
        &mut audio,
        &mut Config::default(),
        &tx,
    )
    .await
    .unwrap();
    while state.sources.listing.is_none() {
        receive(&mut state, &mut audio, &tx, &mut rx).await;
    }
    while state.sources.listing.is_some() {
        receive(&mut state, &mut audio, &tx, &mut rx).await;
    }
    assert!(!state.library_loading);
}

#[test]
fn cache_settings_are_available_for_folder_sources_without_plex_login() {
    use ratatui::{backend::TestBackend, Terminal};
    use textamp::app::state::{SettingsFocus, SettingsSection};
    let mut state = AppState::new();
    state.view = View::Settings;
    state.sources.active = ActiveSource::Folder("local".into());
    state.settings_state.section = SettingsSection::Libraries;
    state.settings_state.focus = SettingsFocus::Content;
    state.settings_state.cache_entries = vec![(
        textamp::app::sources::LibraryChoice::Folder(FolderSource {
            id: "local".into(),
            name: "My music".into(),
            location: FolderLocation::Local {
                path: "/fixture".into(),
            },
        }),
        Ok(2048),
    )];
    textamp::app::sources::options::open(
        state.settings_state.cache_entries[0].0.clone(),
        &mut state,
    );
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| {
            textamp::ui::render(frame, &state);
        })
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Clear library cache…"));
    assert!(text.contains("My music"));
    assert!(!text.contains("signed out"));
    let actions = textamp::app::handlers::key_input::handle_key(
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::F(5),
            crossterm::event::KeyModifiers::NONE,
        ),
        &mut state,
        &Config::default(),
    );
    assert!(matches!(
        actions.as_slice(),
        [textamp::app::Action::Settings(
            SettingsAction::RescanSourceCache(_)
        )]
    ));
}

#[tokio::test]
async fn cache_sizes_are_request_scoped_and_clear_confirmation_names_the_library() {
    use textamp::app::{
        action::SettingsAction,
        state::{ConfirmAction, SettingsFocus, SettingsSection},
    };
    let mut state = AppState::new();
    state.view = View::Settings;
    state.settings_state.section = SettingsSection::Libraries;
    state.settings_state.focus = SettingsFocus::Content;
    state.settings_state.cache_request = 2;
    let choice = textamp::app::sources::LibraryChoice::Folder(FolderSource {
        id: "fixture-target".into(),
        name: "Target library".into(),
        location: FolderLocation::Local {
            path: "/fixture-only".into(),
        },
    });
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let mut audio = textamp::audio::AudioPlayer::new_without_audio();
    let mut config = Config::default();
    for request in [1, 2] {
        textamp::app::dispatch::dispatch_action(
            SettingsAction::CacheSizes {
                request,
                entries: vec![(choice.clone(), Ok(1234))],
            }
            .into(),
            &mut state,
            &mut audio,
            &mut config,
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(
            state.settings_state.cache_entries.len(),
            if request == 2 { 1 } else { 0 }
        );
    }
    use textamp::app::sources::{
        dialogs::Dialog,
        options::{OptionAction, Options},
    };
    let mut options = Options {
        choice,
        selected: 0,
    };
    options.selected = options
        .items(&state)
        .iter()
        .position(|(action, _)| *action == OptionAction::Clear)
        .unwrap();
    state.popups.library_dialog = Some(Dialog::Library(options));
    textamp::app::sources::options::activate(&mut state);
    let dialog = state.popups.confirm_dialog.unwrap();
    assert!(!dialog.selected_yes);
    assert!(
        matches!(dialog.on_confirm, ConfirmAction::ClearSourceCache(textamp::app::sources::LibraryChoice::Folder(s)) if s.id == "fixture-target")
    );
}
