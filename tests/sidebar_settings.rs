use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::TestBackend, Terminal};
use textamp::app::action::SettingsAction;
use textamp::app::handlers::{dispatch_settings, key_input, mouse_input};
use textamp::app::sources::{
    audiomuse,
    navidrome::{commands::CollectionKind, Session},
    ActiveSource,
};
use textamp::app::state::{
    BrowseCategory, CategoryRow, InputDialog, InputDialogAction, SettingsFocus, SettingsSection,
    SidebarSection, View,
};
use textamp::app::{Action, AppState};
use textamp::audiomuse::Feature;
use textamp::config::Config;

fn nav_state() -> AppState {
    let source = textamp::navidrome::Source {
        id: "fixture".into(),
        name: "Music".into(),
        url: "http://example.invalid".into(),
        username: "listener".into(),
        libraries: vec![],
    };
    let session = Session {
        client: textamp::navidrome::Client::new(&source, "unused".into(), None).unwrap(),
        source,
        extensions: Default::default(),
    };
    let mut state = AppState::new();
    state.sources.audiomuse.connections.insert(
        audiomuse::key(&session),
        textamp::audiomuse::Connection {
            url: "http://example.invalid".into(),
            username: "listener".into(),
            server: "Navidrome".into(),
        },
    );
    state.sources.active = ActiveSource::Navidrome(Box::new(session));
    state.view = View::Settings;
    state.settings_state.section = SettingsSection::Textamp;
    state.settings_state.focus = SettingsFocus::Content;
    state
}

#[test]
fn sidebar_has_no_outer_title_and_renders_lowercase_group_headings() {
    for provider in ["navidrome", "none", "folder"] {
        let mut state = nav_state();
        match provider {
            "none" => state.sources.active = ActiveSource::None,
            "folder" => state.sources.active = ActiveSource::Folder("fixture".into()),
            _ => {}
        }
        state.library.playlists = ["Recently Added", "My Playlist"]
            .into_iter()
            .enumerate()
            .map(|(index, title)| {
                serde_json::from_value(serde_json::json!({
                    "ratingKey": index.to_string(), "key": format!("/playlists/{index}"),
                    "title": title, "type": "audio",
                }))
                .unwrap()
            })
            .collect();
        state.view = View::Browse;
        state.category_column_focused = true;
        state.category_column_index = 0;
        let mut terminal = Terminal::new(TestBackend::new(160, 70)).unwrap();
        terminal
            .draw(|frame| {
                state.apply_render_feedback(textamp::ui::render(frame, &state));
            })
            .unwrap();
        let region = state.hit_regions.category_column.as_ref().unwrap();
        let buffer = terminal.backend().buffer();
        let border: String = (region.inner.x..region.inner.right())
            .map(|x| buffer[(x, region.area.y)].symbol())
            .collect();
        assert!(
            border.chars().all(|c| c == '─'),
            "untitled sidebar border: {border}"
        );
        for (index, row) in state.category_rows().iter().enumerate() {
            if let CategoryRow::Header(label) = row {
                let y = region.inner.y + index as u16;
                let line: String = (region.inner.x..region.inner.right())
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                assert!(
                    line.contains(&format!("─ {} ─", label.to_lowercase())),
                    "{provider}: {line}"
                );
                assert!(!line.chars().any(char::is_uppercase));
            }
        }
    }
}

#[test]
fn saved_audiomuse_features_render_after_startup_without_a_loaded_analysis_index() {
    let configured = nav_state();
    let session = configured.sources.active.navidrome().unwrap().clone();
    let config = Config {
        audiomuse_connections: configured.sources.audiomuse.connections.clone(),
        navidrome_sources: vec![session.source.clone()],
        ..Default::default()
    };
    let config: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
    let mut state = AppState::new();
    textamp::app::sources::manager::initialize(&mut state, &config);
    state.sources.active = ActiveSource::Navidrome(Box::new(session));
    state.view = View::Browse;
    assert!(state.sources.audiomuse.snapshot.is_none());
    assert!(state
        .sources
        .active
        .navidrome()
        .unwrap()
        .extensions
        .is_empty());

    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
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
        .map(|c| c.symbol())
        .collect();
    let commands = textamp::app::sources::navidrome::commands::entries(&state);
    for feature in Feature::ALL {
        assert!(
            text.contains(feature.label()),
            "{} must render before network discovery",
            feature.label()
        );
        assert!(state
            .sidebar_sections()
            .contains(&SidebarSection::Collection(CollectionKind::AudioMuse(
                feature
            ))));
        assert_eq!(commands.iter().any(|entry| matches!(entry.command,
            textamp::app::state::PaletteCommandKind::Navidrome(
                textamp::app::sources::navidrome::commands::Command::Collection(CollectionKind::AudioMuse(f))
            ) if f == feature)), feature.is_search());
    }
}

#[tokio::test]
async fn settings_rows_and_keyboard_actions_follow_the_active_provider() {
    use textamp::app::state::TextampSetting;
    use textamp::services::external_search::SearchTarget;
    for provider in ["navidrome", "folder"] {
        let mut state = nav_state();
        state.settings_state.section = SettingsSection::Textamp;
        if provider == "folder" {
            state.sources.active = ActiveSource::Folder("fixture".into());
        }
        let rows = state.textamp_settings();
        assert_eq!(
            rows.contains(&TextampSetting::Transcode),
            provider != "folder"
        );
        let mut terminal = Terminal::new(TestBackend::new(120, 80)).unwrap();
        terminal
            .draw(|frame| {
                state.apply_render_feedback(textamp::ui::render(frame, &state));
            })
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(!text.contains("refresh players"));
        assert_eq!(text.contains("streaming quality"), provider != "folder");
        let mut config = Config::default();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let mut audio = textamp::audio::AudioPlayer::new_without_audio();
        for target in [
            SearchTarget::AppleMusic,
            SearchTarget::Spotify,
            SearchTarget::YouTube,
        ] {
            state.settings_state.item_index = rows
                .iter()
                .position(|row| *row == TextampSetting::ExternalSearch(target))
                .unwrap();
            let actions = dispatch_settings::dispatch(
                &tx,
                &mut config,
                SettingsAction::SettingsSelect,
                &mut state,
                &mut audio,
            )
            .await
            .unwrap();
            assert!(
                matches!(actions.as_slice(), [Action::Settings(SettingsAction::ToggleExternalSearchService(selected))] if *selected == target)
            );
        }
        for _ in 0..3 {
            key_input::handle_key(
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                &mut state,
                &config,
            );
        }
        let youtube = rows
            .iter()
            .position(|r| *r == TextampSetting::ExternalSearch(SearchTarget::YouTube))
            .unwrap();
        assert_eq!(
            state.settings_state.item_index,
            (youtube + 3).min(rows.len() - 1)
        );
        state.settings_state.item_index = youtube;
        terminal = Terminal::new(TestBackend::new(70, 14)).unwrap();
        terminal
            .draw(|frame| {
                state.apply_render_feedback(textamp::ui::render(frame, &state));
            })
            .unwrap();
        let region = state.hit_regions.settings_textamp.clone().unwrap();
        assert!(region.scroll_offset > 0);
        let (rect, index) = *region
            .rows
            .iter()
            .find(|(_, i)| rows[*i] == TextampSetting::ExternalSearch(SearchTarget::AppleMusic))
            .unwrap();
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        };
        assert!(mouse_input::handle_mouse(click, &mut state).is_empty());
        assert_eq!(state.settings_state.item_index, index);
        terminal
            .draw(|frame| {
                state.apply_render_feedback(textamp::ui::render(frame, &state));
            })
            .unwrap();
        assert_eq!(
            state
                .hit_regions
                .settings_textamp
                .as_ref()
                .unwrap()
                .scroll_offset,
            region.scroll_offset
        );
        assert!(matches!(
            mouse_input::handle_mouse(click, &mut state).as_slice(),
            [Action::Settings(SettingsAction::SettingsSelect)]
        ));
        let actions = dispatch_settings::dispatch(
            &tx,
            &mut config,
            SettingsAction::SettingsSelect,
            &mut state,
            &mut audio,
        )
        .await
        .unwrap();
        assert!(matches!(
            actions.as_slice(),
            [Action::Settings(
                SettingsAction::ToggleExternalSearchService(SearchTarget::AppleMusic)
            )]
        ));
        key_input::handle_key(
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            &mut state,
            &config,
        );
        assert!(state.scroll.settings_textamp.is_none());
        let selected = state.settings_state.item_index;
        mouse_input::handle_mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                ..click
            },
            &mut state,
        );
        assert_eq!(
            state.scroll.settings_textamp,
            Some(region.scroll_offset.saturating_sub(3))
        );
        assert_eq!(state.settings_state.item_index, selected);
        key_input::handle_key(
            KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
            &mut state,
            &config,
        );
        assert_eq!(state.settings_state.item_index, 0);
        key_input::handle_key(
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
            &mut state,
            &config,
        );
        assert_eq!(state.settings_state.item_index, rows.len() - 1);
    }
}

#[test]
fn palette_omits_browse_collections_but_keeps_search_and_selection_actions() {
    use textamp::app::command_palette;
    let mut state = nav_state();
    state.queue.tracks = vec![textamp::library::track::Track {
        rating_key: "navidrome:fixture:song".into(),
        title: "Song".into(),
        ..Default::default()
    }];
    state.queue.index = Some(0);
    let destinations: Vec<_> = CollectionKind::SIDEBAR
        .into_iter()
        .chain(
            Feature::ALL
                .into_iter()
                .filter(|f| !f.is_search())
                .map(CollectionKind::AudioMuse),
        )
        .collect();
    for destination in &destinations {
        assert!(
            state
                .sidebar_sections()
                .contains(&SidebarSection::Collection(*destination)),
            "{} remains available for the sidebar",
            destination.label()
        );
    }
    for view in [View::Browse, View::Queue, View::NowPlaying, View::Settings] {
        state.view = view;
        let entries = command_palette::materialize_entries(&state);
        for destination in &destinations {
            assert!(
                !entries.iter().any(|e| e.label == destination.label()),
                "{} is navigation, not a command ({view:?})",
                destination.label()
            );
        }
        for label in [
            "Describe music…",
            "Search lyrics…",
            "Random song mix",
            "Save playback queue to Navidrome",
            "Restore playback queue from Navidrome",
            "Quit",
        ] {
            assert!(
                entries.iter().any(|e| e.label == label),
                "action {label} remains available"
            );
        }
    }
    state.view = View::Queue;
    let entries = command_palette::materialize_entries(&state);
    for label in [
        "Favorite selection",
        "Unfavorite selection",
        "Lyrics",
        "Rate song 5 stars",
        "AudioMuse analysis",
    ] {
        assert!(
            entries.iter().any(|e| e.label == label),
            "selection action {label} remains available"
        );
    }
    // Hiding a sidebar collection must not resurrect it as a palette shortcut.
    state.hidden_collections = destinations;
    assert!(!command_palette::materialize_entries(&state)
        .iter()
        .any(|e| e.label == "Favorite songs"));
}

#[tokio::test]
async fn native_views_share_browse_and_visibility_persists_without_palette_shortcuts() {
    let mut state = nav_state();
    let mut config = Config::default();
    let rows = state.category_rows();
    assert_eq!(rows[0], CategoryRow::Search);
    assert_eq!(rows[1], CategoryRow::Header("Browse"));
    assert!(!rows.contains(&CategoryRow::Header("Navidrome")));
    let target = SidebarSection::Collection(CollectionKind::FavoriteArtists);
    state.settings_state.item_index = state
        .textamp_settings()
        .iter()
        .position(|s| *s == textamp::app::state::TextampSetting::Sidebar(target))
        .unwrap();
    assert!(!state
        .sidebar_sections()
        .contains(&SidebarSection::Category(BrowseCategory::Moods)));
    let actions = key_input::handle_key(
        KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
        &mut state,
        &config,
    );
    assert!(matches!(
        actions.as_slice(),
        [Action::Settings(SettingsAction::SettingsSelect)]
    ));
    let (tx, _rx) = tokio::sync::mpsc::channel(10);

    let mut audio = textamp::audio::AudioPlayer::new_without_audio();
    let actions = dispatch_settings::dispatch(
        &tx,
        &mut config,
        SettingsAction::SettingsSelect,
        &mut state,
        &mut audio,
    )
    .await
    .unwrap();
    assert!(
        matches!(actions.as_slice(), [Action::Settings(SettingsAction::ToggleSectionVisibility(s))] if *s == target)
    );
    let actions = dispatch_settings::dispatch(
        &tx,
        &mut config,
        SettingsAction::ToggleSectionVisibility(target),
        &mut state,
        &mut audio,
    )
    .await
    .unwrap();
    // Inspect the normal persistence command; don't write the user's settings in a test.
    assert!(matches!(
        actions.as_slice(),
        [Action::Settings(SettingsAction::SaveSettings)]
    ));
    assert!(!state
        .category_rows()
        .contains(&CategoryRow::NavidromeCollection(
            CollectionKind::FavoriteArtists
        )));
    assert!(!textamp::app::sources::navidrome::commands::entries(&state)
        .iter()
        .any(|entry| entry.label == "Favorite artists"));
    config
        .ui
        .hidden_collections
        .push(CollectionKind::AudioMuse(Feature::Lyrics));
    let restored: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
    assert_eq!(restored.ui.hidden_collections, config.ui.hidden_collections);
    assert_eq!(restored.ui.hidden_sections, config.ui.hidden_sections);
    assert!(restored
        .ui
        .hidden_collections
        .contains(&CollectionKind::FavoriteArtists));
}

#[test]
fn small_settings_scroll_and_mouse_selection_preserve_the_viewport() {
    let mut state = nav_state();
    let config = Config::default();
    key_input::handle_key(
        KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
        &mut state,
        &config,
    );
    assert_eq!(
        state.settings_state.item_index,
        state.textamp_settings().len() - 1
    );
    let mut terminal = Terminal::new(TestBackend::new(90, 14)).unwrap();
    terminal
        .draw(|frame| {
            state.apply_render_feedback(textamp::ui::render(frame, &state));
        })
        .unwrap();
    let region = state.hit_regions.settings_textamp.clone().unwrap();
    assert!(region.scroll_offset > 0);
    let (rect, index) = region.rows[0];
    let mouse = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x,
        row: rect.y,
        modifiers: KeyModifiers::NONE,
    };
    assert!(mouse_input::handle_mouse(mouse, &mut state).is_empty());
    assert_eq!(state.settings_state.item_index, index);
    terminal
        .draw(|frame| {
            state.apply_render_feedback(textamp::ui::render(frame, &state));
        })
        .unwrap();
    assert_eq!(
        state
            .hit_regions
            .settings_textamp
            .as_ref()
            .unwrap()
            .scroll_offset,
        region.scroll_offset
    );
    assert!(matches!(
        mouse_input::handle_mouse(mouse, &mut state).as_slice(),
        [Action::Settings(SettingsAction::SettingsSelect)]
    ));
    key_input::handle_key(
        KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
        &mut state,
        &config,
    );
    assert_eq!(state.settings_state.item_index, 0);
    assert!(state.scroll.settings_textamp.is_none());
    terminal
        .draw(|frame| {
            state.apply_render_feedback(textamp::ui::render(frame, &state));
        })
        .unwrap();
    let region = state.hit_regions.settings_textamp.as_ref().unwrap();
    let mouse = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: region.inner.x,
        row: region.inner.y,
        modifiers: KeyModifiers::NONE,
    };
    mouse_input::handle_mouse(mouse, &mut state);
    assert_eq!(state.scroll.settings_textamp, Some(3));
    assert_eq!(
        state.settings_state.item_index, 0,
        "wheel does not move keyboard focus"
    );
}

#[test]
fn search_dialogs_offer_search_and_storage_dialogs_still_offer_save() {
    for (action_type, expected) in [
        (
            InputDialogAction::AudioMuseSearch(Feature::Describe),
            "Enter: Search",
        ),
        (
            InputDialogAction::AudioMuseSearch(Feature::Lyrics),
            "Enter: Search",
        ),
        (InputDialogAction::SavePlaylist, "Enter: Save"),
    ] {
        let mut state = nav_state();
        state.popups.input_dialog = Some(InputDialog {
            title: "Test".into(),
            input: "query".into(),
            action_type,
        });
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
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
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains(expected));
        if expected == "Enter: Search" {
            assert!(!text.contains("Enter: Save"));
        }
    }
}

#[test]
fn source_specific_choices_and_empty_groups_do_not_leave_unselectable_focus() {
    let mut state = nav_state();
    state
        .hidden_collections
        .extend(Feature::ALL.map(CollectionKind::AudioMuse));
    assert!(!state
        .category_rows()
        .contains(&CategoryRow::Header("AudioMuse")));
    state.sources.active = ActiveSource::Folder("local".into());
    assert_eq!(
        state.sidebar_sections(),
        [SidebarSection::Category(BrowseCategory::Folders)]
    );
    state.hidden_sections.push(BrowseCategory::Folders);
    state.browse_category = BrowseCategory::Folders;
    state.focus_category_column();
    assert_eq!(
        state.category_rows()[state.category_column_index],
        CategoryRow::Search
    );
    state.sources.active = ActiveSource::None;
    assert!(state
        .sidebar_sections()
        .contains(&SidebarSection::Category(BrowseCategory::Moods)));
    assert!(!state
        .sidebar_sections()
        .iter()
        .any(|s| matches!(s, SidebarSection::Collection(_))));
}
