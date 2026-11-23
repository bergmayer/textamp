use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};
use textamp::app::action::SettingsAction;
use textamp::app::handlers::{dispatch_settings, key_input, mouse_input};
use textamp::app::sources::{
    ActiveSource, audiomuse,
    navidrome::{Session, commands::CollectionKind},
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
fn sidebar_has_no_outer_title_and_renders_disclosure_headings() {
    for provider in ["navidrome", "none", "folder"] {
        let mut state = nav_state();
        match provider {
            "none" => state.sources.active = ActiveSource::None,
            "folder" => state.sources.active = ActiveSource::Folder("fixture".into()),
            _ => {}
        }
        state.library.playlists = std::sync::Arc::new(
            ["Recently Added", "My Playlist"]
                .into_iter()
                .enumerate()
                .map(|(index, title)| {
                    serde_json::from_value(serde_json::json!({
                        "ratingKey": index.to_string(), "key": format!("/playlists/{index}"),
                        "title": title, "type": "audio",
                    }))
                    .unwrap()
                })
                .collect(),
        );
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
            border.chars().all(|c| c == '━'),
            "untitled sidebar border: {border}"
        );
        for (index, row) in state.category_rows().iter().enumerate() {
            if let CategoryRow::Header(label) = row {
                let y = region.inner.y + index as u16;
                let line: String = (region.inner.x..region.inner.right())
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                assert!(line.contains(&format!("▾ {label}")), "{provider}: {line}");
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
    assert!(
        state
            .sources
            .active
            .navidrome()
            .unwrap()
            .extensions
            .is_empty()
    );

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
            text.contains(if feature == Feature::Describe {
                "Describe Music"
            } else {
                feature.label()
            }),
            "{} must render before network discovery",
            feature.label()
        );
        assert!(
            state
                .sidebar_sections()
                .contains(&SidebarSection::Collection(CollectionKind::AudioMuse(
                    feature
                )))
        );
        assert_eq!(commands.iter().any(|entry| matches!(entry.command,
            textamp::app::state::PaletteCommandKind::Navidrome(
                textamp::app::sources::navidrome::commands::Command::Collection(CollectionKind::AudioMuse(f))
            ) if f == feature)), true);
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
        state.settings_state.page = textamp::app::state::SettingsPage::Audio;
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
        assert_eq!(text.contains("Streaming quality"), provider != "folder");
        let mut config = Config::default();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let mut audio = textamp::audio::AudioPlayer::new_without_audio();
        state.settings_state.page = textamp::app::state::SettingsPage::Services;
        let rows = state.textamp_settings();
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
        assert!(!region.rows.is_empty());
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
        assert!(matches!(
            mouse_input::handle_mouse(click, &mut state).as_slice(),
            [Action::Settings(SettingsAction::SettingsSelect)]
        ));
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
fn palette_keeps_analysis_commands_and_omits_duplicate_catalog_destinations() {
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
                entries.iter().any(|e| e.label == destination.label())
                    == matches!(destination, CollectionKind::AudioMuse(_)),
                "{} analysis availability must be independent of sidebar navigation ({view:?})",
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
    assert!(
        !command_palette::materialize_entries(&state)
            .iter()
            .any(|e| e.label == "Favorite songs")
    );
}

#[tokio::test]
async fn native_views_share_browse_and_visibility_persists_without_palette_shortcuts() {
    let mut state = nav_state();
    let mut config = Config::default();
    let rows = state.category_rows();
    assert_eq!(
        rows[0],
        CategoryRow::Root(textamp::app::state::LibraryRoot::Artists)
    );
    assert_eq!(rows[1], CategoryRow::Header("Albums"));
    assert!(!rows.contains(&CategoryRow::Header("Navidrome")));
    let target = SidebarSection::Collection(CollectionKind::FavoriteArtists);
    state.settings_state.page = textamp::app::state::SettingsPage::Sidebar;
    state.settings_state.item_index = state
        .textamp_settings()
        .iter()
        .position(|s| *s == textamp::app::state::TextampSetting::Sidebar(target))
        .unwrap();
    assert!(
        !state
            .sidebar_sections()
            .contains(&SidebarSection::Category(BrowseCategory::Moods))
    );
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
    .unwrap();
    // Inspect the normal persistence command; don't write the user's settings in a test.
    assert!(matches!(
        actions.as_slice(),
        [Action::Settings(SettingsAction::SaveSettings)]
    ));
    assert!(
        !state
            .category_rows()
            .contains(&CategoryRow::NavidromeCollection(
                CollectionKind::FavoriteArtists
            ))
    );
    assert!(
        !textamp::app::sources::navidrome::commands::entries(&state)
            .iter()
            .any(|entry| entry.label == "Favorite artists")
    );
    config
        .ui
        .hidden_collections
        .push(CollectionKind::AudioMuse(Feature::Lyrics));
    let restored: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
    assert_eq!(restored.ui.hidden_collections, config.ui.hidden_collections);
    assert_eq!(restored.ui.hidden_sections, config.ui.hidden_sections);
    assert!(
        restored
            .ui
            .hidden_collections
            .contains(&CollectionKind::FavoriteArtists)
    );
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
    state.settings_state.item_index = state
        .textamp_settings()
        .iter()
        .rposition(|setting| matches!(setting, textamp::app::state::TextampSetting::Theme(_)))
        .unwrap();
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
    assert!(
        matches!(
            mouse_input::handle_mouse(mouse, &mut state).as_slice(),
            [Action::Settings(SettingsAction::SettingsSelect)]
        ),
        "clicking the already highlighted theme applies it"
    );
    assert!(matches!(
        key_input::handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut state,
            &config
        )
        .as_slice(),
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
        state.popups.set_input_dialog(Some(InputDialog {
            title: "Test".into(),
            input: "query".into(),
            action_type,
        }));
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
    assert!(
        !state
            .category_rows()
            .contains(&CategoryRow::Header("AudioMuse"))
    );
    state.sources.active = ActiveSource::Folder("local".into());
    assert_eq!(
        state.sidebar_sections(),
        [SidebarSection::Category(BrowseCategory::Folders)]
    );
    state.hidden_sections.push(BrowseCategory::Folders);
    state.browse_category = BrowseCategory::Folders;
    state.focus_category_column();
    use textamp::app::state::LibraryRoot;
    let mut expected = vec![CategoryRow::Root(LibraryRoot::Downloads)];
    if cfg!(target_os = "macos") {
        expected.push(CategoryRow::Root(LibraryRoot::Mp3Players));
    }
    assert_eq!(state.category_rows(), expected);
    assert!(matches!(
        key_input::handle_key(
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
            &mut state,
            &Config::default()
        )
        .as_slice(),
        [Action::Search(_)]
    ));
    state.sources.active = ActiveSource::None;
    assert!(
        state
            .sidebar_sections()
            .contains(&SidebarSection::Category(BrowseCategory::Moods))
    );
    assert!(
        !state
            .sidebar_sections()
            .iter()
            .any(|s| matches!(s, SidebarSection::Collection(_)))
    );
}

#[test]
fn right_reenters_audiomuse_search_results_while_enter_starts_a_new_search() {
    let config = Config::default();
    for feature in [Feature::Lyrics] {
        for loading in [false, true] {
            let mut state = nav_state();
            state.view = View::Browse;
            state.browse_category = BrowseCategory::Library;
            let kind = CollectionKind::AudioMuse(feature);
            state.sources.nav_collection = Some(kind);
            state.category_column_index = state
                .category_rows()
                .iter()
                .position(|row| *row == CategoryRow::NavidromeCollection(kind))
                .unwrap();
            state.category_column_focused = true;
            state.artist_nav =
                textamp::app::state::BrowseNavigationState::with_root("Existing search", vec![]);
            let tracks: Vec<textamp::library::models::Track> =
                serde_json::from_value(serde_json::json!([
                    {"ratingKey":"result-one", "title":"One"},
                    {"ratingKey":"result-two", "title":"Two"}
                ]))
                .unwrap();
            state.artist_nav.columns[0] = textamp::app::state::BrowseColumn::new_with_tracks(
                "Existing search",
                textamp::app::state::BrowseItem::from_tracks(&tracks),
                tracks,
            );
            state.artist_nav.columns[0].selected_index = 1;
            state.artist_nav.loading = loading;
            let actions = key_input::handle_key(
                KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
                &mut state,
                &config,
            );
            assert!(actions.is_empty(), "Right must not restart the search");
            assert!(!state.category_column_focused);
            assert_eq!(state.artist_nav.columns[0].title, "Existing search");
            assert_eq!(state.artist_nav.loading, loading);
            assert_eq!(state.artist_nav.columns[0].selected_index, 1);
            assert_eq!(
                state.artist_nav.columns[0].tracks[1].rating_key,
                "result-two"
            );
            assert_eq!(state.sources.nav_collection, Some(kind));
            assert!(state.popups.input_dialog().is_none());

            state.category_column_focused = true;
            let actions = key_input::handle_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &mut state,
                &config,
            );
            assert_eq!(actions.len(), 1, "Enter opens a new search");

            state.sources.nav_collection =
                Some(CollectionKind::AudioMuse(if feature == Feature::Describe {
                    Feature::Lyrics
                } else {
                    Feature::Describe
                }));
            let actions = key_input::handle_key(
                KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
                &mut state,
                &config,
            );
            assert!(actions.is_empty(), "Right must not open a prompt");
            state.sources.nav_collection = None;
            let actions = key_input::handle_key(
                KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
                &mut state,
                &config,
            );
            assert!(actions.is_empty(), "Enter is required for the first prompt");
        }
    }
}

#[test]
fn playlists_always_live_in_sidebar_and_legacy_grouping_is_ignored() {
    use textamp::app::state::{BrowseColumn, BrowseItem, LibraryRoot, SettingsPage};
    let mut state = nav_state();
    state.library.playlists = std::sync::Arc::new(vec![serde_json::from_value(
        serde_json::json!({"ratingKey":"playlist","key":"/playlists/playlist","title":"My Playlist","type":"audio"}),
    )
    .unwrap()]);
    state.browse_category = BrowseCategory::Playlists;
    state.playlist_nav.columns = vec![
        BrowseColumn::new(
            "playlists",
            BrowseItem::from_playlists(&state.library.playlists),
        ),
        BrowseColumn::new("tracks", vec![]),
    ];
    let rows = state.category_rows();
    assert!(
        rows.iter()
            .any(|row| matches!(row, CategoryRow::Playlist(0)))
    );
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, CategoryRow::Category(BrowseCategory::Playlists)))
    );
    if cfg!(target_os = "macos") {
        assert_eq!(
            rows.last(),
            Some(&CategoryRow::Root(LibraryRoot::Mp3Players))
        );
    }
    assert_eq!(state.browse_column_offset(), 1);
    assert!(
        !state
            .textamp_settings_for(SettingsPage::Sidebar)
            .iter()
            .any(|setting| format!("{setting:?}").contains("GroupPlaylists"))
    );
    let legacy: Config = toml::from_str("[ui]\ngroup_playlists=true\n").unwrap();
    assert!(
        !toml::to_string(&legacy)
            .unwrap()
            .contains("group_playlists")
    );
}

#[test]
fn playlist_names_preserve_order_and_ignore_obsolete_disclosure_state() {
    use textamp::app::sidebar::{self, SidebarGroup};
    let mut state = nav_state();
    state.view = View::Browse;
    state.desktop_collapsed_sections = vec!["Mu Playlists".into()];
    state.library.playlists = std::sync::Arc::new(
        ["Weekend", "Mu Radio", "mu discoveries", "Favourites"]
            .into_iter()
            .enumerate()
            .map(|(index, title)| {
                serde_json::from_value(serde_json::json!({
                    "ratingKey": index.to_string(), "key": format!("/playlists/{index}"),
                    "title": title, "type": "audio"
                }))
                .unwrap()
            })
            .collect(),
    );
    state.station_nav.columns = vec![textamp::app::state::StationColumn::new(
        None,
        "Radio".into(),
        textamp::library::station::standard_stations(
            "nav-radio",
            textamp::library::capabilities::NAVIDROME,
        ),
    )];
    let rows = sidebar::visible_rows(&state);
    let headings: Vec<_> = rows
        .iter()
        .filter(|row| row.parent.is_none())
        .map(|row| row.label.as_str())
        .collect();
    assert!(
        headings
            .windows(2)
            .any(|pair| pair == ["Albums", "Playlists"])
    );
    assert!(sidebar::allows_disclosure(&rows, "Radio"));
    assert!(
        sidebar::group_rows(&state, "Radio")
            .iter()
            .any(|(_, row)| row.label == "Play Random Albums")
    );
    assert!(
        sidebar::visible_indices(&rows, &["Radio".into()])
            .iter()
            .all(|&i| rows[i].parent != Some(SidebarGroup::Radio))
    );
    let playlists: Vec<_> = rows
        .iter()
        .filter(|row| row.playlist_key.is_some())
        .collect();
    assert_eq!(playlists.len(), 4);
    for (index, row) in playlists.iter().enumerate() {
        assert_eq!(row.parent, Some(SidebarGroup::Playlists));
        assert_eq!(
            row.playlist_key.as_deref(),
            Some(index.to_string().as_str())
        );
        assert_eq!(row.label, state.library.playlists[index].title);
    }
    assert_eq!(
        rows.iter()
            .filter(|row| row.header && row.label == "Playlists")
            .count(),
        1
    );
    let categories = state.category_rows();
    assert_eq!(
        categories
            .iter()
            .filter_map(|row| {
                if let CategoryRow::Playlist(index) = row {
                    Some(*index)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    state.desktop_collapsed_sections = vec!["Playlists".into()];
    assert!(
        sidebar::visible_rows(&state)
            .iter()
            .all(|row| row.playlist_key.is_none())
    );
    assert_eq!(sidebar::group_rows(&state, "Playlists").len(), 4);
}

#[test]
fn terminal_playlist_filter_opens_the_matching_original_row() {
    let mut state = nav_state();
    state.view = View::Browse;
    state.category_column_focused = false;
    state.tui_sidebar_group = Some(("Playlists".into(), 0));
    state.replace_playlists(
        ["First", "Second"]
            .into_iter()
            .map(|title| {
                serde_json::from_value(serde_json::json!({
                    "ratingKey":title,"key":title,"title":title,"type":"audio"
                }))
                .unwrap()
            })
            .collect(),
    );
    let config = Config::default();
    for ch in "/Second".chars() {
        key_input::handle_key(
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
            &mut state,
            &config,
        );
    }
    assert_eq!(state.playlist_group_filter.as_deref(), Some("Second"));
    let actions = key_input::handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut state,
        &config,
    );
    assert!(actions.iter().any(|action| matches!(action,
        Action::Miller(textamp::app::action::MillerAction::LoadPlaylistTracksForMiller { playlist_key, .. }) if playlist_key == "Second")));
}

#[test]
fn select_all_shortcuts_use_the_filtered_rows_in_the_focused_column() {
    use textamp::app::state::{BrowseColumn, BrowseItem};
    let config = Config::default();
    for modifier in [KeyModifiers::SUPER, KeyModifiers::CONTROL] {
        let mut state = nav_state();
        state.view = View::Browse;
        state.browse_category = BrowseCategory::Library;
        state.category_column_focused = false;
        state.artist_nav.columns = vec![BrowseColumn::new(
            "Albums",
            ["first", "second", "third"]
                .map(|title| BrowseItem::Album {
                    key: title.into(),
                    title: title.into(),
                    artist: "Artist".into(),
                    thumb: None,
                    year: None,
                    is_placeholder: false,
                })
                .to_vec(),
        )];
        state.list_filter.active = true;
        state.list_filter.query = "ir".into();
        state.list_filter.results = Some(textamp::services::ListFilterResults {
            matched_indices: vec![2, 0],
            total_matches: 2,
            has_more: false,
        });
        key_input::handle_key(
            KeyEvent::new(KeyCode::Char('a'), modifier),
            &mut state,
            &config,
        );
        let marked = &state.artist_nav.columns[0].selected_set;
        assert_eq!(marked.len(), 2);
        assert!(marked.contains(&0) && marked.contains(&2));
    }
}
