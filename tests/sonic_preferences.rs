use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::TestBackend, Terminal};
use textamp::app::action::{DataAction, QueueAction, RadioAction, SearchAction};
use textamp::app::handlers::{key_input, mouse_input};
use textamp::app::sources::{
    self, manager, navidrome::Session, sonic, ActiveSource, LibraryChoice, SourceAction,
};
use textamp::app::state::{DjMode, SettingsFocus, SettingsSection, View};
use textamp::app::{Action, AppState};
use textamp::config::Config;
use textamp::navidrome::{Client, Source};

fn source() -> Source {
    Source {
        id: "nav".into(),
        name: "Music".into(),
        url: "http://127.0.0.1:1".into(),
        username: "fixture".into(),
        libraries: vec![],
    }
}
fn nav_state(folder: Option<String>) -> (AppState, LibraryChoice) {
    let source = source();
    let choice = LibraryChoice::Navidrome {
        source: source.clone(),
        folder: folder.clone(),
        name: "Music".into(),
    };
    let mut state = AppState::new();
    state.sources.navidrome.push(source.clone());
    state.sources.active = ActiveSource::Navidrome(Box::new(Session {
        client: Client::new(&source, "fixture".into(), folder).unwrap(),
        source,
        extensions: std::collections::HashSet::from(["sonicSimilarity".into()]),
    }));
    (state, choice)
}
fn disable(state: &mut AppState, choice: &LibraryChoice) {
    let key = sonic::choice_key(choice, state).unwrap();
    state.sources.sonic_disabled_libraries.insert(key);
}

#[test]
fn single_folder_migration_preserves_disabled_sonic_and_default_selection() {
    let (mut state, legacy_choice) = nav_state(None);
    disable(&mut state, &legacy_choice);
    let mut source = source();
    source.libraries.push(textamp::navidrome::MusicFolder {
        id: "one".into(),
        name: "Music Library".into(),
    });
    let mut config = Config {
        navidrome_sources: vec![source.clone()],
        sonic_disabled_libraries: state.sources.sonic_disabled_libraries,
        default_navidrome: Some(textamp::navidrome::Selection {
            source_id: source.id.clone(),
            folder: None,
        }),
        ..Default::default()
    };
    assert!(textamp::config::canonicalize_navidrome(&mut config));
    assert!(!textamp::config::canonicalize_navidrome(&mut config));
    assert_eq!(
        config.default_navidrome.as_ref().unwrap().folder.as_deref(),
        Some("one")
    );
    let (mut restored, choice) = nav_state(Some("one".into()));
    manager::initialize(&mut restored, &config);
    assert!(!sonic::enabled(&restored));
    assert!(restored
        .sources
        .sonic_disabled_libraries
        .contains(&sonic::choice_key(&choice, &restored).unwrap()));
}

fn load_stations(state: &mut AppState) {
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    sources::navidrome::intercept(
        &textamp::app::action::BrowseAction::LoadStations.into(),
        state,
        &tx,
    );
}

#[test]
fn preferences_survive_config_round_trip_and_are_library_scoped() {
    let (mut state, choice) = nav_state(None);
    assert!(sonic::enabled(&state));
    disable(&mut state, &choice);
    let config = Config {
        sonic_disabled_libraries: state.sources.sonic_disabled_libraries.clone(),
        ..Default::default()
    };
    let restored: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
    let (mut restored_state, _) = nav_state(None);
    manager::initialize(&mut restored_state, &restored);
    assert!(!sonic::enabled(&restored_state));
    let (mut other, _) = nav_state(Some("one".into()));
    manager::initialize(&mut other, &restored);
    assert!(sonic::enabled(&other));
    let (mut other, _) = nav_state(None);
    other.sources.active = ActiveSource::None;
    manager::initialize(&mut other, &restored);
    assert!(!sonic::enabled(&other));
    assert!(toml::from_str::<Config>("")
        .unwrap()
        .sonic_disabled_libraries
        .is_empty());
}

#[tokio::test]
async fn disabled_commands_neither_start_requests_nor_change_the_queue() {
    let (mut state, choice) = nav_state(None);
    disable(&mut state, &choice);
    let mut audio = textamp::audio::AudioPlayer::new_without_audio();

    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    for action in [
        DataAction::LoadTrackPaneSimilar {
            rating_key: "track".into(),
        }
        .into(),
        DataAction::LoadSimilarTracks {
            rating_key: "track".into(),
            title: "Track".into(),
        }
        .into(),
        SearchAction::OpenAdventureLauncher.into(),
        RadioAction::ToggleDjMode(DjMode::Gemini).into(),
        RadioAction::PlayStation("nav-radio/sonic".into()).into(),
        RadioAction::StartSonicRadio(Box::default()).into(),
        QueueAction::RemixStretch.into(),
    ] {
        textamp::app::dispatch::dispatch_action(
            action,
            &mut state,
            &mut audio,
            &mut Config::default(),
            &tx,
        )
        .await
        .unwrap();
    }
    assert!(state.sources.nav_tasks.is_empty());
    assert!(state.track_pane_similar_loading.is_empty());
    assert!(state.popups.adventure_launcher.is_none());
    assert!(state.dj.active_mode.is_none());
    assert!(state.queue.tracks.is_empty());
    assert!(rx.try_recv().is_err());
    for action in [
        RadioAction::ToggleDjMode(DjMode::Twofer).into(),
        RadioAction::PlayStation("nav-radio/randomAlbum".into()).into(),
        QueueAction::RemixShuffle.into(),
    ] {
        assert!(!sonic::action_blocked(&state, &action));
    }
}

#[test]
fn disabled_features_disappear_from_palette_context_and_station_navigation() {
    let (mut state, choice) = nav_state(None);
    state.view = View::Queue;
    load_stations(&mut state);
    assert!(state.stations.iter().any(|s| s.key == "nav-radio/sonic"));
    disable(&mut state, &choice);
    sonic::reconcile(&mut state);
    assert!(state
        .stations
        .iter()
        .any(|s| s.key == "nav-radio/randomAlbum"));
    assert!(state.stations.iter().any(|s| s.key == "dj:twofer"));
    assert!(!state
        .station_nav
        .columns
        .iter()
        .flat_map(|c| &c.stations)
        .any(sonic::station_requires_sonic));
    let entries = textamp::app::command_palette::materialize_entries(&state);
    assert!(!entries.iter().any(|e| e.label.contains("Sonic")
        || e.label.contains("Gemini")
        || e.label.contains("Stretch")
        || e.label.contains("Freeze")));
    let contexts =
        textamp::services::track_context::track_context_entries(&state, &Default::default(), false);
    assert!(!contexts
        .iter()
        .any(|e| e.label.contains("Similar") || e.label.contains("Sonic")));
    state.sources.sonic_disabled_libraries.clear();
    load_stations(&mut state);
    assert!(state.stations.iter().any(|s| s.key == "nav-radio/sonic"));
}

#[test]
fn settings_checkbox_uses_the_highlighted_library_and_is_not_on_accounts_or_folders() {
    let (mut state, _) = nav_state(None);
    state.view = View::Settings;
    state.settings_state.section = SettingsSection::Libraries;
    state.settings_state.focus = SettingsFocus::Content;
    let choice = textamp::app::sources::choices(&state)[state.popups.library_picker_index].clone();
    textamp::app::sources::options::open(choice, &mut state);
    let option_index = match &state.popups.library_dialog {
        Some(textamp::app::sources::dialogs::Dialog::Library(options)) => options
            .items(&state)
            .iter()
            .position(|(action, _)| *action == textamp::app::sources::options::OptionAction::Sonic)
            .unwrap(),
        _ => panic!("Expected library options"),
    };
    let mut terminal = Terminal::new(TestBackend::new(130, 35)).unwrap();
    let mut feedback = None;
    terminal
        .draw(|f| feedback = Some(textamp::ui::render(f, &state)))
        .unwrap();
    state.hit_regions = feedback.unwrap().hit_regions;
    let rect = state.hit_regions.library_dialog[option_index];
    let click = mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        },
        &mut state,
    );
    assert!(matches!(
        click.as_slice(),
        [Action::Source(SourceAction::ToggleSonic(
            LibraryChoice::Navidrome { folder: None, .. }
        ))]
    ));
    state.popups.library_dialog = None;
    let keys = key_input::handle_key(
        KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
        &mut state,
        &Config::default(),
    );
    assert!(matches!(
        keys.as_slice(),
        [Action::Source(SourceAction::ToggleSonic(_))]
    ));
    state.popups.library_picker_index = textamp::app::sources::choices(&state).len() - 1;
    terminal
        .draw(|f| {
            assert!(textamp::ui::render(f, &state)
                .hit_regions
                .library_dialog
                .is_empty())
        })
        .unwrap();
    let folder = LibraryChoice::Folder(textamp::library::FolderSource {
        id: "folder".into(),
        name: "Folder".into(),
        location: textamp::library::FolderLocation::Local {
            path: "/fixture".into(),
        },
    });
    assert!(sonic::choice_key(&folder, &state).is_none());
    state.sources.active = ActiveSource::Folder("folder".into());
    assert!(!sonic::enabled(&state));
}

#[tokio::test]
async fn disabling_cancels_work_without_stopping_current_music() {
    let (mut state, choice) = nav_state(None);
    let track = textamp::library::models::Track {
        rating_key: "current".into(),
        ..Default::default()
    };
    state.queue.tracks = vec![track];
    state.queue.index = Some(0);
    state.dj.active_mode = Some(DjMode::Gemini);
    let task = tokio::spawn(std::future::pending::<()>());
    state
        .sources
        .sonic_tasks
        .insert("remix", textamp::app::tasks::TaskLease::new(&task));
    let epoch = state.radio_generation;
    disable(&mut state, &choice);
    sonic::cancel_pending(&mut state);
    assert!(task.await.unwrap_err().is_cancelled());
    assert_ne!(state.radio_generation, epoch);
    assert!(state.dj.active_mode.is_none());
    assert_eq!(state.current_track().unwrap().rating_key, "current");
    state.dj.active_mode = Some(DjMode::Twofer);
    state.dj.inserting = true;
    sonic::cancel_pending(&mut state);
    assert_eq!(state.dj.active_mode, Some(DjMode::Twofer));
    assert!(!state.dj.inserting);
    // A queue undo may restore an old sonic station after the preference changed.
    state.radio.tracks = state.queue.tracks.clone();
    state.radio.track_index = Some(0);
    state.radio.active_station = Some(textamp::app::state::ActiveStation {
        source: textamp::library::models::RadioSource::Station("nav-radio/sonic".into()),
        title: "Sonic Radio".into(),
    });
    state.playback_mode = textamp::app::state::PlaybackMode::Radio;
    assert!(sonic::radio_blocked(&state));
    sonic::reconcile(&mut state);
    assert_eq!(
        state.playback_mode,
        textamp::app::state::PlaybackMode::Queue
    );
    assert_eq!(state.current_track().unwrap().rating_key, "current");
}
