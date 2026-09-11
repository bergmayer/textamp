use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::TestBackend, Terminal};
use textamp::app::action::SearchAction;
use textamp::app::handlers::{key_input, mouse_input};
use textamp::app::sources::{self, manager, ActiveSource, LibraryChoice, SourceAction};
use textamp::app::state::{SettingsFocus, SettingsSection, View};
use textamp::app::{Action, AppState};
use textamp::config::Config;
use textamp::library::{FolderLocation, FolderSource};

fn config() -> Config {
    Config {
        folder_sources: vec![FolderSource {
            id: "beatles".into(),
            name: "Beatles".into(),
            location: FolderLocation::Webdav {
                url: "https://example.test/".into(),
                username: Some("listener".into()),
                password_env: None,
            },
        }],
        navidrome_sources: vec![textamp::navidrome::Source {
            id: "nav".into(),
            name: "Navidrome account".into(),
            url: "https://nav.test/".into(),
            username: "listener".into(),
            libraries: vec![],
        }],
        default_navidrome: Some(textamp::navidrome::Selection {
            source_id: "nav".into(),
            folder: None,
        }),
        ..Default::default()
    }
}
fn press(state: &mut AppState, key: KeyCode) -> Vec<Action> {
    key_input::handle_key(KeyEvent::new(key, KeyModifiers::NONE), state, &config())
}

fn open_settings(state: &mut AppState) {
    state.set_view(View::Settings);
    state.settings_state.section = SettingsSection::Libraries;
    state.settings_state.focus = SettingsFocus::Content;
}

#[test]
fn tab_and_arrows_reach_all_three_sections() {
    let mut state = AppState::new();
    manager::initialize(&mut state, &config());
    open_settings(&mut state);
    press(&mut state, KeyCode::Left);
    assert_eq!(state.settings_state.focus, SettingsFocus::Sections);
    assert_eq!(SettingsSection::all().len(), 3);
    for section in SettingsSection::all() {
        assert_eq!(state.settings_state.section, *section);
        press(&mut state, KeyCode::Tab);
        assert_eq!(state.settings_state.focus, SettingsFocus::Content);
        press(&mut state, KeyCode::BackTab);
        assert_eq!(state.settings_state.focus, SettingsFocus::Sections);
        press(&mut state, KeyCode::Down);
    }
}

#[test]
fn active_marker_is_visible_before_a_long_name_and_not_the_highlighted_row() {
    for width in [40, 80, 140] {
        let mut state = AppState::new();
        manager::initialize(&mut state, &config());
        open_settings(&mut state);
        state.sources.folders[0].name = "A very long library name which will be clipped".into();
        state.sources.active = ActiveSource::Folder("beatles".into());
        state.popups.library_picker_index = 0;
        let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
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
        assert!(text.contains("Active: A"));
        assert!(text.contains("[Active] A"));
        assert!(
            !text.contains("> [Active]"),
            "highlight must not imply activation"
        );
    }
}

#[test]
fn sole_navidrome_folder_has_one_named_choice_in_settings_and_switcher() {
    let mut config = config();
    let source = &mut config.navidrome_sources[0];
    source.libraries = vec![textamp::navidrome::MusicFolder {
        id: "1".into(),
        name: "Music Library".into(),
    }];
    let session = sources::navidrome::Session {
        client: textamp::navidrome::Client::new(source, "fixture".into(), None).unwrap(),
        source: source.clone(),
        extensions: Default::default(),
    };
    assert_eq!(session.client.folder.as_deref(), Some("1"));
    let mut state = AppState::new();
    manager::initialize(&mut state, &config);
    state.sources.active = ActiveSource::Navidrome(Box::new(session));
    for popup in [false, true] {
        state.popups.library_picker_active = popup;
        let choices = sources::choices(&state);
        let nav: Vec<_> = choices
            .iter()
            .filter(|c| matches!(c, LibraryChoice::Navidrome { .. }))
            .collect();
        assert_eq!(nav.len(), 1);
        assert!(nav[0].label().starts_with("Music Library"));
        assert!(nav[0].active(&state));
    }
    state.sources.navidrome[0]
        .libraries
        .push(textamp::navidrome::MusicFolder {
            id: "2".into(),
            name: "Other".into(),
        });
    assert_eq!(
        sources::choices(&state)
            .iter()
            .filter(|c| matches!(c, LibraryChoice::Navidrome { .. }))
            .count(),
        3
    );
}

#[test]
fn manager_owns_navigation_and_refresh_keys_instead_of_the_underlying_view() {
    let mut state = AppState::new();
    manager::initialize(&mut state, &config());
    open_settings(&mut state);
    for key in ['a', 'p', 's', 'n'] {
        assert!(key_input::handle_key(
            KeyEvent::new(KeyCode::Char(key), KeyModifiers::CONTROL),
            &mut state,
            &config()
        )
        .is_empty());
    }
    assert!(press(&mut state, KeyCode::F(5)).is_empty());
    assert!(matches!(
        press(&mut state, KeyCode::Char('w')).as_slice(),
        [Action::Source(SourceAction::Choose(
            LibraryChoice::AddWebdav
        ))]
    ));
    assert!(press(&mut state, KeyCode::Char('s')).is_empty());
    press(&mut state, KeyCode::Left);
    assert_eq!(state.settings_state.focus, SettingsFocus::Sections);
}

#[test]
fn add_is_in_the_same_list_and_left_always_reaches_sidebar() {
    let mut state = AppState::new();
    manager::initialize(&mut state, &config());
    open_settings(&mut state);
    press(&mut state, KeyCode::End);
    assert!(matches!(
        sources::choices(&state)[state.popups.library_picker_index],
        LibraryChoice::Add
    ));
    assert!(matches!(
        press(&mut state, KeyCode::Enter).as_slice(),
        [Action::Source(SourceAction::Choose(LibraryChoice::Add))]
    ));
    press(&mut state, KeyCode::Left);
    assert_eq!(state.settings_state.focus, SettingsFocus::Sections);
}

#[test]
fn libraries_are_grouped_by_account_not_name() {
    let mut cfg = config();
    cfg.navidrome_sources[0].libraries = vec![
        textamp::navidrome::MusicFolder {
            id: "1".into(),
            name: "Z music".into(),
        },
        textamp::navidrome::MusicFolder {
            id: "2".into(),
            name: "A music".into(),
        },
    ];
    let mut state = AppState::new();
    manager::initialize(&mut state, &cfg);
    let choices = sources::library_choices(&state);
    assert!(choices[..3].iter().all(|c| c.group() == choices[0].group()));
    assert!(matches!(choices[3], LibraryChoice::Folder(_)));
}

#[test]
fn manager_rows_skip_headings_and_click_preserves_viewport() {
    for (width, height) in [(140, 42), (80, 24), (40, 12)] {
        let mut state = AppState::new();
        manager::initialize(&mut state, &config());
        open_settings(&mut state);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| state.apply_render_feedback(textamp::ui::render(f, &state)))
            .unwrap();
        let region = state.hit_regions.library_manager_rows.clone().unwrap();
        assert!(!region.rows.is_empty());
        let (rect, index) = *region.rows.last().unwrap();
        state.popups.library_picker_index = usize::MAX;
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        };
        assert!(mouse_input::handle_mouse(click, &mut state).is_empty());
        assert_eq!(state.popups.library_picker_index, index);
        terminal
            .draw(|f| state.apply_render_feedback(textamp::ui::render(f, &state)))
            .unwrap();
        assert_eq!(
            state
                .hit_regions
                .library_manager_rows
                .as_ref()
                .unwrap()
                .scroll_offset,
            region.scroll_offset
        );
        let choice = sources::choices(&state)[index].clone();
        let actions = mouse_input::handle_mouse(click, &mut state);
        if matches!(choice, LibraryChoice::Add) {
            assert!(matches!(
                actions.as_slice(),
                [Action::Source(SourceAction::Choose(LibraryChoice::Add))]
            ));
        } else {
            assert!(matches!(
                state.popups.library_dialog,
                Some(sources::dialogs::Dialog::Library(_))
            ));
            assert!(matches!(
                actions.as_slice(),
                [Action::Settings(
                    textamp::app::action::SettingsAction::RefreshCacheStats
                )]
            ));
        }
    }
}

#[test]
fn startup_uses_the_saved_source_without_requiring_a_picker() {
    let mut config = config();
    assert!(matches!(
        manager::startup_choice(&config),
        Some(LibraryChoice::Navidrome { .. })
    ));
    config.default_navidrome = None;
    config.default_folder_source = Some("beatles".into());
    assert!(matches!(
        manager::startup_choice(&config),
        Some(LibraryChoice::Folder(_))
    ));
    config.default_folder_source = Some("removed".into());
    assert!(matches!(
        manager::startup_choice(&config),
        Some(LibraryChoice::Navidrome { .. })
    ));
}

#[test]
fn quick_switcher_is_compact_and_does_not_offer_management_commands() {
    let mut state = AppState::new();
    manager::initialize(&mut state, &config());
    state.set_view(View::Browse);
    state.popups.library_picker_active = true;
    let mut terminal = Terminal::new(TestBackend::new(140, 42)).unwrap();
    terminal
        .draw(|frame| {
            let feedback = textamp::ui::render(frame, &state);
            state.apply_render_feedback(feedback);
        })
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Switch library"));
    assert!(text.contains("Beatles"));
    assert!(text.contains("F2 manage"));
    assert!(state.hit_regions.library_manager_rows.is_none());
    assert!(
        state
            .hit_regions
            .library_picker
            .as_ref()
            .unwrap()
            .outer
            .height
            < 15
    );
    for key in [
        KeyCode::Tab,
        KeyCode::Char('a'),
        KeyCode::Char('n'),
        KeyCode::Char('/'),
        KeyCode::Char(':'),
        KeyCode::Delete,
    ] {
        assert!(press(&mut state, key).is_empty());
    }
    assert!(matches!(
        press(&mut state, KeyCode::F(2)).as_slice(),
        [Action::Search(SearchAction::ManageLibraries)]
    ));
    assert!(matches!(
        press(&mut state, KeyCode::Esc).as_slice(),
        [Action::Search(SearchAction::CloseLibraryPicker)]
    ));
    assert_eq!(state.view, View::Browse);

    // The popup must not retain clickable tabs from underlying Settings.
    open_settings(&mut state);
    terminal
        .draw(|frame| {
            let feedback = textamp::ui::render(frame, &state);
            state.apply_render_feedback(feedback);
        })
        .unwrap();
    assert!(state.hit_regions.library_manager_rows.is_none());
}

#[tokio::test]
async fn switching_and_managing_are_separate_and_cancel_preserves_player_state() {
    use textamp::app::dispatch::dispatch_action;
    let mut state = AppState::new();
    let mut config = config();
    manager::initialize(&mut state, &config);
    state.sources.active = ActiveSource::Folder("beatles".into());
    state.playback.position_ms = 1234;
    state.set_view(View::Browse);

    let mut audio = textamp::audio::AudioPlayer::new_without_audio();
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    for action in [
        SearchAction::OpenLibraryPicker,
        SearchAction::ManageLibraries,
    ] {
        dispatch_action(action.into(), &mut state, &mut audio, &mut config, &tx)
            .await
            .unwrap();
    }
    assert!(manager::settings_active(&state));
    assert_eq!(state.settings_state.focus, SettingsFocus::Content);
    assert!(!state.popups.library_picker_active);
    dispatch_action(
        SearchAction::OpenLibraryPicker.into(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    assert!(sources::choices(&state)[state.popups.library_picker_index].active(&state));
    dispatch_action(
        SearchAction::CloseLibraryPicker.into(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    assert_eq!(state.view, View::Settings);
    assert_eq!(state.playback.position_ms, 1234);
    assert_eq!(
        state.sources.active.folder().map(String::as_str),
        Some("beatles")
    );
    let choice = sources::choices(&state)[state.popups.library_picker_index].clone();
    dispatch_action(choice.action(), &mut state, &mut audio, &mut config, &tx)
        .await
        .unwrap();
    assert_eq!(
        state.view,
        View::Browse,
        "opening the active library leaves settings without reloading it"
    );
    assert_eq!(state.playback.position_ms, 1234);
}
