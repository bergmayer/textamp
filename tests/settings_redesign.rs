use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::TestBackend, Terminal};
use textamp::app::handlers::{key_input, mouse_input};
use textamp::app::sources::{
    self,
    dialogs::{Dialog, ServerForm},
    options::{self, OptionAction, OptionsPage},
    ActiveSource, LibraryChoice, SourceAction,
};
use textamp::app::state::{SettingsFocus, SettingsPage, SettingsSection, View};
use textamp::app::AppState;
use textamp::config::Config;

fn fixture() -> AppState {
    let mut state = AppState::new();
    state.view = View::Settings;
    state.settings_state.focus = SettingsFocus::Content;
    state.sources.folders.push(textamp::library::FolderSource {
        id: "music".into(),
        name: "Music folder".into(),
        location: textamp::library::FolderLocation::Local {
            path: "/fixture/music".into(),
        },
    });
    state.sources.navidrome.push(textamp::navidrome::Source {
        id: "server".into(),
        name: "Home music".into(),
        url: "http://music.example:4533".into(),
        username: "listener".into(),
        libraries: vec![],
    });
    state.sources.active = ActiveSource::Folder("music".into());
    state.active_library = Some("folder:music".into());
    state
}
fn press(state: &mut AppState, code: KeyCode) -> Vec<textamp::app::Action> {
    key_input::handle_key(
        KeyEvent::new(code, KeyModifiers::NONE),
        state,
        &Config::default(),
    )
}
fn draw(state: &mut AppState, width: u16, height: u16, name: &str) {
    state.terminal_width = width;
    state.terminal_height = height;
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| state.apply_render_feedback(textamp::ui::render(frame, state)))
        .unwrap();
    if let Ok(dir) = std::env::var("TEXTAMP_SETTINGS_PREVIEW_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        let cells: Vec<_> = terminal.backend().buffer().content.iter().map(|c| serde_json::json!({"s":c.symbol(),"fg":format!("{:?}",c.fg),"bg":format!("{:?}",c.bg)})).collect();
        std::fs::write(
            std::path::Path::new(&dir).join(format!("{name}-{width}x{height}.json")),
            serde_json::to_vec(&serde_json::json!({"width":width,"height":height,"cells":cells}))
                .unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn setup_pages_have_matching_mouse_targets_and_keep_playback_unchanged() {
    let mut state = fixture();
    state.playback.position_ms = 12345;
    for (width, height) in [(100, 32), (80, 24), (40, 12)] {
        draw(&mut state, width, height, "libraries");
        state.settings_state.focus = SettingsFocus::Sections;
        draw(&mut state, width, height, "categories");
        let target = state
            .hit_regions
            .settings_pages
            .iter()
            .find(|(_, p)| *p == SettingsPage::Themes)
            .unwrap()
            .0;
        mouse_input::handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: target.x,
                row: target.y,
                modifiers: KeyModifiers::NONE,
            },
            &mut state,
        );
        assert_eq!(state.settings_state.section, SettingsSection::Textamp);
        assert_eq!(state.settings_state.focus, SettingsFocus::Content);
        for page in SettingsPage::ALL {
            state.settings_state.page = page;
            state.settings_state.item_index = 0;
            draw(&mut state, width, height, page.name());
            assert!(!state
                .hit_regions
                .settings_textamp
                .as_ref()
                .unwrap()
                .rows
                .is_empty());
            assert_eq!(state.sources.active.folder().unwrap(), "music");
            assert_eq!(state.playback.position_ms, 12345);
        }
        state.settings_state.section = SettingsSection::Libraries;
        state.settings_state.focus = SettingsFocus::Content;
    }
}
#[test]
fn library_connection_cancel_returns_to_setup_and_cache_is_separate() {
    let mut state = fixture();
    let choice = sources::library_choices(&state)
        .into_iter()
        .find(|c| matches!(c, LibraryChoice::Navidrome { .. }))
        .unwrap();
    options::open(choice, &mut state);
    let index = match &state.popups.library_dialog {
        Some(Dialog::Library(o)) => o
            .items(&state)
            .iter()
            .position(|(a, _)| *a == OptionAction::Connection)
            .unwrap(),
        _ => unreachable!(),
    };
    if let Some(Dialog::Library(o)) = &mut state.popups.library_dialog {
        o.selected = index;
    }
    press(&mut state, KeyCode::Enter);
    assert!(matches!(
        state.popups.library_dialog,
        Some(Dialog::Navidrome(_))
    ));
    draw(&mut state, 100, 32, "connection");
    press(&mut state, KeyCode::Esc);
    assert!(matches!(
        state.popups.library_dialog,
        Some(Dialog::Library(_))
    ));
    press(&mut state, KeyCode::Right);
    press(&mut state, KeyCode::Right);
    if let Some(Dialog::Library(o)) = &state.popups.library_dialog {
        assert_eq!(o.page, OptionsPage::Cache);
        assert!(!o
            .items(&state)
            .iter()
            .any(|(a, _)| *a == OptionAction::Activate));
    } else {
        panic!("missing setup")
    }
    for (w, h) in [(100, 32), (80, 24), (40, 12)] {
        draw(&mut state, w, h, "library-cache");
    }
    let index = match &state.popups.library_dialog {
        Some(Dialog::Library(options)) => options
            .items(&state)
            .iter()
            .position(|(action, _)| *action == OptionAction::ClearArtwork)
            .unwrap(),
        _ => unreachable!(),
    };
    if let Some(Dialog::Library(options)) = &mut state.popups.library_dialog {
        options.selected = index;
    }
    press(&mut state, KeyCode::Enter);
    assert!(
        matches!(state.popups.confirm_dialog.as_ref().map(|dialog| &dialog.on_confirm),
        Some(textamp::app::state::ConfirmAction::ClearArtworkCache(LibraryChoice::Navidrome { source, .. })) if source.id == "server")
    );
}
#[tokio::test]
async fn saving_a_server_connection_does_not_activate_it() {
    let mut state = fixture();
    let source = state.sources.navidrome[0].clone();
    state.popups.library_dialog = Some(Dialog::Navidrome(ServerForm::new(Some(&source))));
    state.playback.position_ms = 9876;
    let session = sources::navidrome::Session {
        client: textamp::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
        source,
        extensions: Default::default(),
    };
    let mut config = Config {
        default_folder_source: Some("music".into()),
        ..Default::default()
    };
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let generation = state.connection_generation;
    textamp::app::dispatch::dispatch_action(
        sources::navidrome::NavAction::Connected {
            generation,
            result: Ok(session),
            password: None,
        }
        .into(),
        &mut state,
        &mut textamp::audio::AudioPlayer::new_without_audio(),
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    assert_eq!(state.view, View::Settings);
    assert_eq!(state.sources.active.folder().unwrap(), "music");
    assert_eq!(state.playback.position_ms, 9876);
    assert_eq!(config.default_folder_source.as_deref(), Some("music"));
    assert_eq!(config.navidrome_sources.len(), 1);
    assert!(state.sources.reconnect_sources.contains("server"));
}
#[tokio::test]
async fn local_form_saves_path_and_name_together_and_cancel_discards_draft() {
    let mut state = fixture();
    let mut config = Config::default();
    let mut audio = textamp::audio::AudioPlayer::new_without_audio();
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    textamp::app::dispatch::dispatch_action(
        LibraryChoice::AddFolder.action(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    assert!(matches!(
        state.popups.library_dialog,
        Some(Dialog::Local(_))
    ));
    press(&mut state, KeyCode::Esc);
    assert!(config.folder_sources.is_empty());
    textamp::app::dispatch::dispatch_action(
        LibraryChoice::AddFolder.action(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    textamp::app::dispatch::dispatch_action(
        SourceAction::SaveLocal {
            path: dir.path().display().to_string(),
            name: "New folder".into(),
        }
        .into(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    assert_eq!(config.folder_sources[0].name, "New folder");
    assert_eq!(state.sources.active.folder().unwrap(), "music");
    assert!(state.popups.library_dialog.is_none() && state.popups.input_dialog.is_none());
}

#[tokio::test]
async fn editing_active_webdav_returns_to_setup_without_interrupting_playback() {
    let mut state = fixture();
    let source = textamp::library::FolderSource {
        id: "music".into(),
        name: "Music share".into(),
        location: textamp::library::FolderLocation::Webdav {
            url: "http://old.example/music/".into(),
            username: None,
            password_env: None,
        },
    };
    state.sources.folders = vec![source.clone()];
    state.playback.position_ms = 9876;
    let mut config = Config {
        folder_sources: vec![source.clone()],
        ..Default::default()
    };
    options::open(LibraryChoice::Folder(source), &mut state);
    let index = match &state.popups.library_dialog {
        Some(Dialog::Library(o)) => o
            .items(&state)
            .iter()
            .position(|(a, _)| *a == OptionAction::Connection)
            .unwrap(),
        _ => unreachable!(),
    };
    if let Some(Dialog::Library(o)) = &mut state.popups.library_dialog {
        o.selected = index;
    }
    press(&mut state, KeyCode::Enter);
    let Some(Dialog::Webdav(form)) = &mut state.popups.library_dialog else {
        panic!("connection form")
    };
    form.fields[0].value = "http://new.example/music/".into();
    let draft = form.draft().unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    textamp::app::dispatch::dispatch_action(
        SourceAction::WebdavChecked {
            instance: draft.instance.clone(),
            result: Ok(draft),
        }
        .into(),
        &mut state,
        &mut textamp::audio::AudioPlayer::new_without_audio(),
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    assert_eq!(state.view, View::Settings);
    assert_eq!(state.playback.position_ms, 9876);
    assert!(state.sources.reconnect_sources.contains("music"));
    assert!(
        matches!(&state.popups.library_dialog,Some(Dialog::Library(o)) if o.choice.location().contains("new.example"))
    );
}

#[test]
fn categories_and_controls_are_reachable_with_two_focus_targets() {
    let mut state = fixture();
    state.settings_state.focus = SettingsFocus::Sections;
    for index in 0..textamp::app::state::SettingsState::CATEGORY_COUNT {
        assert_eq!(state.settings_state.category_index(), index);
        for (width, height) in [(100, 32), (80, 24), (40, 12)] {
            draw(&mut state, width, height, "navigation");
            assert!(
                state
                    .hit_regions
                    .settings_sections
                    .iter()
                    .any(|(_, s)| *s == state.settings_state.section)
                    || state
                        .hit_regions
                        .settings_pages
                        .iter()
                        .any(|(_, p)| *p == state.settings_state.page)
            );
        }
        press(&mut state, KeyCode::Right);
        assert_eq!(state.settings_state.focus, SettingsFocus::Content);
        assert!(state.popups.library_dialog.is_none());
        press(&mut state, KeyCode::BackTab);
        assert_eq!(state.settings_state.focus, SettingsFocus::Sections);
        press(&mut state, KeyCode::Tab);
        assert_eq!(state.settings_state.focus, SettingsFocus::Content);
        press(&mut state, KeyCode::Esc);
        assert_eq!(state.settings_state.focus, SettingsFocus::Sections);
        press(&mut state, KeyCode::Down);
    }
    press(&mut state, KeyCode::Home);
    assert_eq!(state.settings_state.section, SettingsSection::Libraries);
    assert_eq!(state.sources.active.folder().unwrap(), "music");
    assert!(matches!(
        press(&mut state, KeyCode::Esc).as_slice(),
        [textamp::app::Action::Navigation(
            textamp::app::action::NavigationAction::SetView(View::Browse)
        )]
    ));
}

#[test]
fn command_palette_opens_cancels_and_executes_from_settings() {
    let mut state = fixture();
    for section in SettingsSection::all() {
        state.settings_state.section = *section;
        for focus in [SettingsFocus::Sections, SettingsFocus::Content] {
            state.settings_state.focus = focus;
            for key in [
                KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Char(':'), KeyModifiers::SHIFT),
                KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
            ] {
                key_input::handle_key(key, &mut state, &Config::default());
                assert!(state.palette.open);
                draw(&mut state, 80, 24, "settings-palette");
                press(&mut state, KeyCode::Esc);
                assert!(!state.palette.open);
                assert_eq!(state.view, View::Settings);
                assert_eq!(state.settings_state.focus, focus);
                assert_eq!(state.settings_state.section, *section);
            }
        }
    }
    press(&mut state, KeyCode::Char(':'));
    press(&mut state, KeyCode::Char('q'));
    let actions = press(&mut state, KeyCode::Enter);
    assert!(actions.iter().any(|a| matches!(
        a,
        textamp::app::Action::System(textamp::app::action::SystemAction::Quit)
    )));
}

#[test]
fn settings_panels_and_library_dialogs_use_active_theme() {
    use textamp::ui::theme::{Theme, ThemeName};
    let mut state = fixture();
    for name in ThemeName::all() {
        state.theme = *name;
        let colors = Theme::new(*name).colors;
        for section in SettingsSection::all() {
            state.settings_state.section = *section;
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal
                .draw(|frame| state.apply_render_feedback(textamp::ui::render(frame, &state)))
                .unwrap();
            let border = &terminal.backend().buffer()[(0, 0)];
            assert_eq!(border.bg, colors.bg_primary, "{name:?} {section:?}");
            assert_eq!(border.fg, colors.fg_accent, "{name:?} {section:?}");
        }
        state.settings_state.section = SettingsSection::Libraries;
        options::open(
            LibraryChoice::Folder(state.sources.folders[0].clone()),
            &mut state,
        );
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| state.apply_render_feedback(textamp::ui::render(frame, &state)))
            .unwrap();
        let rect = state
            .hit_regions
            .library_dialog
            .iter()
            .find(|r| !r.is_empty())
            .unwrap();
        let selected = &terminal.backend().buffer()[(rect.x, rect.y)];
        assert_eq!(selected.bg, colors.selection_bar_bg, "{name:?}");
        assert_eq!(selected.fg, colors.selection_bar_fg, "{name:?}");
        press(&mut state, KeyCode::Esc);
    }
}

#[tokio::test]
async fn artwork_choices_follow_each_library_and_settings_show_the_active_one() {
    use textamp::app::action::SettingsAction;
    use textamp::app::state::{ArtworkMode, SettingsPage};
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let folders = [first.path(), second.path()]
        .into_iter()
        .enumerate()
        .map(|(i, path)| textamp::library::FolderSource {
            id: format!("folder-{i}"),
            name: format!("Library {i}"),
            location: textamp::library::FolderLocation::Local { path: path.into() },
        })
        .collect::<Vec<_>>();
    let mut config = Config {
        folder_sources: folders.clone(),
        ..Default::default()
    };
    let mut state = AppState::new();
    state.sources.folders = folders.clone();
    let mut audio = textamp::audio::AudioPlayer::new_without_audio();
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    sources::dispatch(
        SourceAction::Choose(LibraryChoice::Folder(folders[0].clone())),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    state.view = View::Settings;
    state.settings_state.section = SettingsSection::Textamp;
    state.settings_state.page = SettingsPage::Display;
    state.settings_state.focus = SettingsFocus::Content;
    state.settings_state.item_index = 1;
    textamp::app::dispatch::dispatch_action(
        SettingsAction::SettingsSelect.into(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    assert_eq!(state.artwork.mode, ArtworkMode::Halfblocks);
    state.settings_state.item_index = 3;
    textamp::app::dispatch::dispatch_action(
        SettingsAction::SettingsSelect.into(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    assert!(!state.artwork.default_visible);
    assert_eq!(
        config.artwork_preferences("folder:folder-0"),
        (ArtworkMode::Halfblocks, false)
    );
    assert_eq!(
        config.artwork_preferences("folder:folder-1"),
        (ArtworkMode::Auto, true)
    );
    assert_eq!(config.ui.artwork_mode, "auto");
    assert!(config.ui.cover_art_view);

    sources::dispatch(
        SourceAction::Choose(LibraryChoice::Folder(folders[1].clone())),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    assert_eq!(state.artwork.mode, ArtworkMode::Auto);
    assert!(state.artwork.default_visible);
    state.view = View::Settings;
    state.settings_state.section = SettingsSection::Textamp;
    state.settings_state.page = SettingsPage::Display;
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| state.apply_render_feedback(textamp::ui::render(frame, &state)))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Library 1"));
    assert!(text.contains("Show covers in new columns"));

    sources::dispatch(
        SourceAction::Choose(LibraryChoice::Folder(folders[0].clone())),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    assert_eq!(state.artwork.mode, ArtworkMode::Halfblocks);
    assert!(!state.artwork.default_visible);
    config.set_column_artwork("folder:folder-0", "albums:artist", true);
    assert_eq!(
        config.artwork_preferences("folder:folder-0"),
        (ArtworkMode::Halfblocks, false)
    );
    assert_eq!(
        config.artwork_preferences("folder:folder-1"),
        (ArtworkMode::Auto, true)
    );
}

#[test]
fn downloaded_artwork_cache_is_scoped_to_each_library() {
    let first_key = format!("folder:art-first-{}", uuid::Uuid::new_v4());
    let second_key = format!("folder:art-second-{}", uuid::Uuid::new_v4());
    let first = textamp::media::ArtworkCache::for_library(&first_key);
    let second = textamp::media::ArtworkCache::for_library(&second_key);
    assert!(first.save("same-cover-id", b"first library"));
    assert!(second.save("same-cover-id", b"second library"));
    assert_eq!(
        first.load("same-cover-id", 0).as_deref(),
        Some(b"first library".as_slice())
    );
    assert_eq!(
        second.load("same-cover-id", 0).as_deref(),
        Some(b"second library".as_slice())
    );
    assert_eq!(first.stats().0, 1);
    assert_eq!(second.stats().0, 1);
    assert_eq!(first.clear_all(), 1);
    assert_eq!(
        second.load("same-cover-id", 0).as_deref(),
        Some(b"second library".as_slice())
    );
    assert_eq!(second.clear_all(), 1);
}

#[test]
fn mouse_opens_categories_previews_themes_and_applies_only_on_request() {
    use textamp::app::{action::SettingsAction, Action};
    for (width, height) in [(100, 32), (80, 24), (40, 12)] {
        let mut state = fixture();
        state.settings_state.focus = SettingsFocus::Sections;
        draw(&mut state, width, height, "mouse-categories");
        let target = state
            .hit_regions
            .settings_pages
            .iter()
            .find(|(_, p)| *p == SettingsPage::Themes)
            .unwrap()
            .0;
        let click = |rect: ratatui::layout::Rect| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        };
        assert!(mouse_input::handle_mouse(click(target), &mut state).is_empty());
        assert_eq!(state.settings_state.focus, SettingsFocus::Content);
        draw(&mut state, width, height, "mouse-appearance");
        let region = state.hit_regions.settings_textamp.clone().unwrap();
        let (row, index) = *region.rows.last().unwrap();
        let original = state.theme;
        assert!(mouse_input::handle_mouse(click(row), &mut state).is_empty());
        assert_eq!(state.settings_state.item_index, index);
        assert_eq!(state.theme, original);
        draw(&mut state, width, height, "mouse-preview");
        assert_eq!(
            state
                .hit_regions
                .settings_textamp
                .as_ref()
                .unwrap()
                .scroll_offset,
            region.scroll_offset
        );
        if let Some(apply) = state.hit_regions.settings_apply {
            assert!(matches!(
                mouse_input::handle_mouse(click(apply), &mut state).as_slice(),
                [Action::Settings(SettingsAction::SettingsSelect)]
            ));
        }
        assert!(matches!(
            press(&mut state, KeyCode::Enter).as_slice(),
            [Action::Settings(SettingsAction::SettingsSelect)]
        ));
        let back = state.hit_regions.settings_back.unwrap();
        assert!(mouse_input::handle_mouse(click(back), &mut state).is_empty());
        assert_eq!(state.settings_state.focus, SettingsFocus::Sections);
        assert_eq!(state.sources.active.folder().unwrap(), "music");
    }
}

#[test]
fn resizing_keeps_selection_and_keyboard_navigation_releases_mouse_scroll() {
    let mut state = fixture();
    state.settings_state.section = SettingsSection::Textamp;
    state.settings_state.page = SettingsPage::Themes;
    press(&mut state, KeyCode::End);
    let selected = state.settings_state.item_index;
    for (width, height) in [(100, 32), (40, 12), (80, 24), (28, 10), (100, 32)] {
        draw(&mut state, width, height, "resize");
        assert_eq!(state.settings_state.item_index, selected);
        assert!(state
            .hit_regions
            .settings_textamp
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .any(|(_, i)| *i == selected));
    }
    state.scroll.settings_textamp = Some(0);
    press(&mut state, KeyCode::Up);
    assert!(state.scroll.settings_textamp.is_none());
    assert_eq!(state.settings_state.item_index, selected - 1);
}

#[test]
fn empty_artwork_page_and_audio_page_are_distinct_and_navigate_back() {
    let mut state = AppState::new();
    state.view = View::Settings;
    state.settings_state.section = SettingsSection::Textamp;
    state.settings_state.page = SettingsPage::Display;
    state.settings_state.focus = SettingsFocus::Content;
    assert!(state.textamp_settings().is_empty());
    draw(&mut state, 80, 24, "empty-artwork");
    assert!(state
        .hit_regions
        .settings_textamp
        .as_ref()
        .unwrap()
        .rows
        .is_empty());
    press(&mut state, KeyCode::Left);
    press(&mut state, KeyCode::Down);
    assert_eq!(state.settings_state.page, SettingsPage::Audio);
    press(&mut state, KeyCode::Right);
    assert_eq!(
        state.textamp_settings(),
        vec![textamp::app::state::TextampSetting::SoundOutput]
    );
    draw(&mut state, 80, 24, "empty-audio");
}
