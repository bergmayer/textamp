use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
use textamp::app::{
    Action, AppState,
    action::{BrowseAction, NavigationAction, SettingsAction},
    handlers::{dispatch_navigation, key_input, mouse_input},
    state::*,
};
use textamp::config::Config;
use textamp::library::models::{Artist, Track};

fn fixture() -> AppState {
    let mut state = AppState::new();
    let source = textamp::navidrome::Source {
        id: "fixture".into(),
        name: "Fixture".into(),
        url: "http://127.0.0.1:9".into(),
        username: "fixture".into(),
        libraries: vec![],
    };
    state.sources.active = textamp::app::sources::ActiveSource::Navidrome(Box::new(
        textamp::app::sources::navidrome::Session {
            client: textamp::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
            source,
            extensions: Default::default(),
        },
    ));
    state.library.artists = std::sync::Arc::new(vec![Artist {
        rating_key: "artist".into(),
        title: "Artist A".into(),
        ..Default::default()
    }]);
    state
        .artist_nav
        .reset("Artists", state.build_artist_root_items());
    state.queue.tracks = vec![Track {
        rating_key: "track".into(),
        title: "Song A".into(),
        grandparent_rating_key: Some("artist".into()),
        grandparent_title: Some("Artist A".into()),
        parent_title: Some("Album A".into()),
        duration: Some(123_000),
        ..Default::default()
    }];
    state.queue.index = Some(0);
    state.playback_mode = PlaybackMode::Queue;
    state
}
fn draw(state: &mut AppState, width: u16, height: u16) -> Buffer {
    state.terminal_width = width;
    state.terminal_height = height;
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| state.apply_render_feedback(textamp::ui::render(frame, state)))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    if let Ok(directory) = std::env::var("TEXTAMP_TUI_REVIEW_DIR") {
        let path = std::path::Path::new(&directory);
        std::fs::create_dir_all(path).unwrap();
        let lines: Vec<String> = buffer
            .content
            .chunks(width as usize)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect();
        let cells: Vec<_> = buffer.content.iter().map(|cell| serde_json::json!({"text":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg)})).collect();
        std::fs::write(
            path.join(format!(
                "{}-{width}x{height}.json",
                if state.popups.adventure_launcher().is_some() {
                    "Journey".to_owned()
                } else {
                    format!("{:?}", state.view)
                }
            )),
            serde_json::to_vec(&serde_json::json!({"width":width,"height":height,"cells":cells}))
                .unwrap(),
        )
        .unwrap();
        std::fs::write(
            path.join(format!(
                "{}-{width}x{height}.txt",
                if state.popups.adventure_launcher().is_some() {
                    "Journey".to_owned()
                } else {
                    format!("{:?}", state.view)
                }
            )),
            lines.join("\n"),
        )
        .unwrap();
    }
    buffer
}
fn key(state: &mut AppState, code: KeyCode) -> Vec<Action> {
    key_input::handle_key(
        KeyEvent::new(code, KeyModifiers::NONE),
        state,
        &Config::default(),
    )
}
fn click(state: &mut AppState, rect: ratatui::layout::Rect) -> Vec<Action> {
    mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        },
        state,
    )
}
#[test]
fn sidebar_disclosures_share_the_desktop_hierarchy_and_keyboard_targets() {
    let mut state = fixture();
    let rows = state.category_rows();
    assert_eq!(state.category_column_index, 0);
    assert_eq!(rows[0], CategoryRow::Root(LibraryRoot::Artists));
    let albums = rows
        .iter()
        .position(|row| *row == CategoryRow::Header("Albums"))
        .unwrap();
    let compilations = rows
        .iter()
        .position(|row| *row == CategoryRow::Root(LibraryRoot::Compilations))
        .unwrap();
    let singles = rows
        .iter()
        .position(|row| *row == CategoryRow::Root(LibraryRoot::Singles))
        .unwrap();
    assert!(albums < compilations && compilations < singles);
    state.category_column_focused = true;
    state.category_column_index = albums;
    assert!(key(&mut state, KeyCode::Enter).is_empty());
    assert_eq!(
        state
            .tui_sidebar_group
            .as_ref()
            .map(|(title, _)| title.as_str()),
        Some("Albums")
    );
    assert!(!state.category_column_focused);
    key(&mut state, KeyCode::Esc);
    state.category_column_focused = true;
    state.desktop_collapsed_sections.push("Albums".into());
    assert!(
        state
            .category_rows()
            .contains(&CategoryRow::Category(BrowseCategory::Folders))
    );
    assert!(
        state
            .category_rows()
            .contains(&CategoryRow::Category(BrowseCategory::AlbumGenres))
    );
    assert!(
        !state
            .category_rows()
            .contains(&CategoryRow::Root(LibraryRoot::Compilations))
    );
    assert!(
        matches!(key(&mut state, KeyCode::Right).as_slice(), [Action::Settings(SettingsAction::ToggleSidebarGroup(title))] if title == "Albums")
    );
}
#[test]
fn queue_stays_on_the_right_and_tools_and_artist_labels_are_operable() {
    let mut state = fixture();
    for view in [View::Browse, View::NowPlaying] {
        state.view = view;
        for (width, height) in [(160, 48), (100, 30), (60, 16)] {
            let buffer = draw(&mut state, width, height);
            let queue = state.hit_regions.queue_content.as_ref().unwrap();
            assert_eq!(queue.track_list.right(), width);
            let queue_x = queue.track_list.x;
            let tools = &state.hit_regions.queue_menus;
            assert_eq!(tools.len(), 2);
            assert_eq!(
                tools
                    .iter()
                    .map(|(rect, _)| rect.y)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                1
            );
            assert!(tools.iter().all(|(rect, _)| rect.x >= queue_x
                && rect.right() <= width
                && rect.bottom() <= height));
            if view == View::NowPlaying {
                let visualizer = state.hit_regions.now_playing_content.as_ref().unwrap();
                assert!(visualizer.visualizer_content_area.right() <= queue_x);
                assert!(visualizer.visualizer_content_area.y > 0);
                let canvas = visualizer.visualizer_content_area;
                let text: String = (canvas.y..canvas.bottom())
                    .flat_map(|y| (canvas.x..canvas.right()).map(move |x| (x, y)))
                    .map(|(x, y)| buffer[(x, y)].symbol())
                    .collect();
                assert!(!text.contains("0:00"));
                let link = state
                    .hit_regions
                    .artist_links
                    .iter()
                    .find(|link| link.area.x < queue_x)
                    .unwrap()
                    .clone();
                assert!(matches!(
                    click(&mut state, link.area).as_slice(),
                    [Action::Browse(BrowseAction::OpenDetailsInLibrary {
                        destination: textamp::app::library_details::Destination::Artist,
                        ..
                    })]
                ));
            }
        }
    }
}
#[test]
fn pipe_toggles_queue_without_replacing_the_library_or_now_playing() {
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    for view in [View::Browse, View::NowPlaying] {
        let mut state = fixture();
        state.view = view;
        assert!(matches!(
            key(&mut state, KeyCode::Char('|')).as_slice(),
            [Action::Navigation(NavigationAction::ToggleQueuePanel)]
        ));
        dispatch_navigation::dispatch(&tx, NavigationAction::ToggleQueuePanel, &mut state).unwrap();
        assert!(!state.tui_queue_visible);
        assert_eq!(state.view, view);
        draw(&mut state, 100, 30);
        assert!(state.hit_regions.queue_content.is_none());
        dispatch_navigation::dispatch(&tx, NavigationAction::ToggleQueuePanel, &mut state).unwrap();
        draw(&mut state, 100, 30);
        assert!(state.hit_regions.queue_content.is_some());
    }
}
#[test]
fn queue_focus_returns_to_the_visible_library_when_clicking_its_sidebar() {
    let mut state = fixture();
    state.view = View::Browse;
    draw(&mut state, 120, 35);
    let queue = state
        .hit_regions
        .queue_content
        .as_ref()
        .unwrap()
        .track_list_inner;
    click(&mut state, queue);
    assert_eq!(state.view, View::Queue);
    draw(&mut state, 120, 35);
    let sidebar = state.hit_regions.category_column.as_ref().unwrap().inner;
    click(&mut state, sidebar);
    assert_eq!(state.view, View::Browse);
}
#[test]
fn about_restores_colored_ansi_art_and_settings_do_not_offer_output_routing() {
    let mut state = fixture();
    state.view = View::Settings;
    state.settings_state.section = SettingsSection::About;
    for (width, height) in [(100, 32), (65, 24), (40, 12)] {
        state.settings_state.focus = SettingsFocus::Content;
        let buffer = draw(&mut state, width, height);
        assert!(
            buffer
                .content
                .iter()
                .any(|cell| matches!(cell.symbol(), "▀" | "▄" | "█")
                    && matches!(cell.fg, ratatui::style::Color::Rgb(_, _, _)))
        );
    }
    assert!(
        textamp::app::command_palette::global_entries()
            .iter()
            .all(|entry| !entry.label.contains("Sound Output"))
    );
}

#[test]
fn library_roots_share_source_actions_and_preserve_playback() {
    for (root, expected_offset) in [
        (LibraryRoot::Artists, 0),
        (LibraryRoot::Albums, 1),
        (LibraryRoot::Compilations, 1),
        (LibraryRoot::Singles, 0),
        (LibraryRoot::Tracks, 1),
    ] {
        let mut state = fixture();
        state.playback.position_ms = 12_345;
        let actions = textamp::app::sidebar::open_root(root, &mut state);
        assert_eq!(state.desktop_library_root, root);
        assert_eq!(state.browse_column_offset(), expected_offset);
        assert_eq!(state.playback.position_ms, 12_345);
        assert_eq!(state.queue.index, Some(0));
        assert_eq!(state.queue.tracks[0].rating_key, "track");
        assert!(
            actions
                .iter()
                .all(|action| !matches!(action, Action::Playback(_) | Action::Queue(_)))
        );
    }
}
#[test]
fn now_playing_multi_artist_fields_link_names_without_linking_the_track_title() {
    let mut state = fixture();
    state.library.track_artists = vec![Artist {
        rating_key: "b".into(),
        title: "Artist B".into(),
        ..Default::default()
    }];
    state.queue.tracks[0].original_title = Some("Artist A; Artist B".into());
    state.queue.tracks[0].title = "Artist A".into();
    state.view = View::NowPlaying;
    draw(&mut state, 160, 48);
    let queue_x = state
        .hit_regions
        .queue_content
        .as_ref()
        .unwrap()
        .track_list
        .x;
    for name in ["Artist A", "Artist B"] {
        assert!(
            state
                .hit_regions
                .artist_links
                .iter()
                .any(|link| link.name == name && link.area.x < queue_x)
        );
    }
}

#[test]
fn headers_filter_and_commands_have_keyboard_and_pointer_targets() {
    use textamp::app::{
        action::SearchAction,
        column_controls::{ColumnControl, ColumnHeaderFocus},
    };
    let mut state = fixture();
    state.tui_queue_visible = false;
    state.category_column_focused = false;
    state.artist_nav.focused_column = 0;
    draw(&mut state, 120, 32);
    let options = state.hit_regions.miller_columns.as_ref().unwrap().columns[0]
        .controls
        .clone();
    assert!(
        options
            .iter()
            .any(|(_, control)| *control == ColumnControl::Filter)
    );
    assert!(
        options
            .iter()
            .any(|(_, control)| *control == ColumnControl::Options)
    );
    assert!(
        options
            .iter()
            .any(|(_, control)| *control == ColumnControl::Close)
    );
    key_input::handle_key(
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
        &mut state,
        &Config::default(),
    );
    assert_eq!(
        state.tui_column_header,
        Some(ColumnHeaderFocus {
            category: BrowseCategory::Library,
            column: 0,
            control: ColumnControl::Filter
        })
    );
    let actions = key_input::handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut state,
        &Config::default(),
    );
    assert!(matches!(
        actions.as_slice(),
        [Action::Search(SearchAction::ActivateListFilter)]
    ));
    let title = options
        .iter()
        .find(|(_, control)| *control == ColumnControl::Filter)
        .unwrap()
        .0;
    let actions = mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: title.x + 1,
            row: title.y,
            modifiers: KeyModifiers::NONE,
        },
        &mut state,
    );
    assert!(matches!(
        actions.as_slice(),
        [Action::Search(SearchAction::ActivateListFilter)]
    ));
    state.list_filter.active = true;
    state.list_filter.category = state.browse_category;
    state.list_filter.column = 0;
    state.list_filter.query = "Artist".into();
    let buffer = draw(&mut state, 120, 32);
    let region = &state.hit_regions.miller_columns.as_ref().unwrap().columns[0];
    let header: String = (region.area.x..region.area.right())
        .map(|x| buffer[(x, region.area.y)].symbol())
        .collect();
    assert!(header.contains("Artist"));
    assert!(state.hit_regions.transport.as_ref().unwrap().play_pause.y > region.area.y);
    let close = region.controls[0].0;
    state.miller_scroll_manual = true;
    key_input::handle_key(
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        &mut state,
        &Config::default(),
    );
    assert!(
        state.miller_scroll_manual,
        "typing into a header filter must not pan the columns"
    );
    let actions = mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: close.x,
            row: close.y,
            modifiers: KeyModifiers::NONE,
        },
        &mut state,
    );
    assert!(matches!(
        actions.as_slice(),
        [Action::Search(SearchAction::DeactivateListFilter)]
    ));
}

#[test]
fn columns_overflow_with_fixed_sidebar_and_scroll_back_to_parents() {
    let mut state = fixture();
    state.tui_queue_visible = false;
    state.category_column_focused = false;
    for index in 1..5 {
        state.artist_nav.columns.push(BrowseColumn::new(
            format!("Column {index}"),
            vec![BrowseItem::Artist {
                key: format!("artist-{index}"),
                title: format!("Artist {index}"),
                thumb: None,
                is_placeholder: false,
            }],
        ));
    }
    state.artist_nav.focused_column = 4;
    draw(&mut state, 100, 30);
    let sidebar = state.hit_regions.category_column.as_ref().unwrap().area;
    let regions = state.hit_regions.miller_columns.as_ref().unwrap();
    assert_eq!(regions.columns.last().unwrap().col_idx, 4);
    assert!(
        regions
            .columns
            .iter()
            .all(|column| column.area.x >= sidebar.right())
    );
    assert!(
        regions
            .columns
            .windows(2)
            .all(|pair| pair[0].area.width == pair[1].area.width)
    );
    assert!(state.hit_regions.miller_h_scrollbar.is_some());
    for _ in 0..5 {
        mouse_input::handle_mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollLeft,
                column: sidebar.right() + 5,
                row: 5,
                modifiers: KeyModifiers::NONE,
            },
            &mut state,
        );
        draw(&mut state, 100, 30);
    }
    assert_eq!(
        state.hit_regions.miller_columns.as_ref().unwrap().columns[0].col_idx,
        0
    );
    assert_eq!(
        state.hit_regions.category_column.as_ref().unwrap().area,
        sidebar
    );
    let artist = &state.hit_regions.miller_columns.as_ref().unwrap().columns[0];
    let alphabet = state.hit_regions.alphabet_strip.as_ref().unwrap();
    assert!(alphabet.area.x > artist.inner.x);
    assert!(alphabet.area.right() <= artist.area.right());
}

#[test]
fn queue_toggle_and_back_are_independent_of_workspace() {
    use textamp::app::action::NavigationAction;
    let mut state = fixture();
    for view in [View::Browse, View::NowPlaying] {
        state.view = view;
        let actions = key_input::handle_key(
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            &mut state,
            &Config::default(),
        );
        assert!(matches!(
            actions.as_slice(),
            [Action::Navigation(NavigationAction::ToggleQueuePanel)]
        ));
        assert_eq!(state.view, view);
    }
    let actions = key_input::handle_key(
        KeyEvent::new(KeyCode::Left, KeyModifiers::ALT),
        &mut state,
        &Config::default(),
    );
    assert!(matches!(
        actions.as_slice(),
        [Action::Navigation(NavigationAction::Back)]
    ));
    let buffer = draw(&mut state, 120, 32);
    let footer: String = buffer
        .content
        .chunks(120)
        .last()
        .unwrap()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(!footer.contains("full screen"));
    assert!(!footer.contains("queue"));
}

#[test]
fn notifications_share_the_bottom_right_footer_and_do_not_cover_content() {
    let mut state = fixture();
    state.notifications.toast_message = Some("Saved playlist".into());
    let buffer = draw(&mut state, 120, 32);
    let text: Vec<String> = buffer
        .content
        .chunks(120)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect();
    assert!(text[31].contains("Saved playlist"));
    assert!(text[31].ends_with("  "));
    assert!(text[..31].iter().all(|row| !row.contains("Saved playlist")));
}

#[tokio::test]
async fn journey_has_the_same_explicit_endpoint_preview_flow_in_both_frontends() {
    use textamp::app::{action::SearchAction, handlers::dispatch_search, journey::JourneyControl};
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    for desktop in [false, true] {
        let mut state = fixture();
        state.desktop_frontend = desktop;
        dispatch_search::dispatch(&tx, SearchAction::OpenAdventureLauncher, &mut state).unwrap();
        let queue = state.queue.tracks.clone();
        for (step, key) in [
            (AdventureStep::FindStartTrack, "start"),
            (AdventureStep::FindEndTrack, "end"),
        ] {
            let launcher = state.popups.adventure_launcher_mut().unwrap();
            launcher.step = step;
            launcher.search_tab = SearchTab::Tracks;
            launcher.results = Some(textamp::library::models::SearchResults {
                tracks: vec![Track {
                    rating_key: key.into(),
                    title: format!("{key} track"),
                    grandparent_title: Some("An artist".into()),
                    ..Default::default()
                }],
                ..Default::default()
            });
            launcher.item_index = 0;
            let actions = dispatch_search::dispatch(
                &tx,
                SearchAction::AdventureLauncherSelectTrack,
                &mut state,
            )
            .unwrap();
            assert!(
                actions.is_empty(),
                "selecting an endpoint must not start generation or playback"
            );
            assert!(matches!(
                state.popups.adventure_launcher().unwrap().stage,
                JourneyStage::Choosing
            ));
        }
        assert_eq!(
            state.popups.adventure_launcher().unwrap().terminal_focus,
            JourneyControl::Preview
        );
        assert_eq!(
            state
                .queue
                .tracks
                .iter()
                .map(|track| &track.rating_key)
                .collect::<Vec<_>>(),
            queue
                .iter()
                .map(|track| &track.rating_key)
                .collect::<Vec<_>>()
        );
        let _buffer = draw(&mut state, 100, 32);
        let regions = state
            .hit_regions
            .adventure_launcher
            .as_ref()
            .unwrap()
            .clone();
        for control in [
            JourneyControl::Start,
            JourneyControl::End,
            JourneyControl::Count,
            JourneyControl::Results,
            JourneyControl::Swap,
            JourneyControl::Cancel,
            JourneyControl::Preview,
        ] {
            assert!(regions.controls.iter().any(|(_, item)| *item == control));
        }
        let before = regions
            .controls
            .iter()
            .map(|(rect, control)| (*control, *rect))
            .collect::<Vec<_>>();
        state
            .popups
            .adventure_launcher_mut()
            .unwrap()
            .terminal_focus = JourneyControl::Count;
        draw(&mut state, 100, 32);
        assert_eq!(
            before,
            state
                .hit_regions
                .adventure_launcher
                .as_ref()
                .unwrap()
                .controls
                .iter()
                .map(|(rect, control)| (*control, *rect))
                .collect::<Vec<_>>()
        );
    }
}

#[tokio::test]
async fn journey_endpoint_fields_and_buttons_are_keyboard_and_mouse_operable() {
    use textamp::app::{action::SearchAction, handlers::dispatch_search, journey::JourneyControl};
    let mut state = fixture();
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    dispatch_search::dispatch(&tx, SearchAction::OpenAdventureLauncher, &mut state).unwrap();
    for (code, control) in [
        (KeyCode::Tab, JourneyControl::End),
        (KeyCode::Tab, JourneyControl::Count),
        (KeyCode::Tab, JourneyControl::Types),
        (KeyCode::Tab, JourneyControl::Results),
        (KeyCode::Tab, JourneyControl::Swap),
        (KeyCode::Tab, JourneyControl::Cancel),
        (KeyCode::Tab, JourneyControl::Preview),
    ] {
        let actions = key_input::handle_key(
            KeyEvent::new(code, KeyModifiers::NONE),
            &mut state,
            &Config::default(),
        );
        assert_eq!(
            state.popups.adventure_launcher().unwrap().terminal_focus,
            control
        );
        for action in actions {
            if let Action::Search(action) = action {
                dispatch_search::dispatch(&tx, action, &mut state).unwrap();
            }
        }
    }
    draw(&mut state, 100, 32);
    let end = state
        .hit_regions
        .adventure_launcher
        .as_ref()
        .unwrap()
        .controls
        .iter()
        .find(|(_, control)| *control == JourneyControl::End)
        .unwrap()
        .0;
    let actions = mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: end.x,
            row: end.y,
            modifiers: KeyModifiers::NONE,
        },
        &mut state,
    );
    assert!(matches!(
        actions.last(),
        Some(Action::Search(SearchAction::AdventureLauncherSetStep(
            AdventureStep::FindEndTrack
        )))
    ));
    for action in actions {
        if let Action::Search(action) = action {
            dispatch_search::dispatch(&tx, action, &mut state).unwrap();
        }
    }
    let actions = key_input::handle_key(
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        &mut state,
        &Config::default(),
    );
    assert!(
        matches!(actions.as_slice(),[Action::Search(SearchAction::AdventureLauncherSetEndpointQuery{step:AdventureStep::FindEndTrack,query})] if query=="a")
    );
    for (width, height) in [(100, 32), (60, 20), (40, 12)] {
        draw(&mut state, width, height);
        let regions = state.hit_regions.adventure_launcher.as_ref().unwrap();
        assert!(
            regions
                .controls
                .iter()
                .all(|(rect, _)| rect.right() <= width && rect.bottom() <= height)
        );
        assert!(
            regions
                .controls
                .iter()
                .any(|(rect, control)| *control == JourneyControl::Preview && rect.width > 0)
        );
    }
    let actions = key_input::handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL),
        &mut state,
        &Config::default(),
    );
    assert!(matches!(
        actions.as_slice(),
        [Action::Search(SearchAction::AdventureLauncherGenerate)]
    ));
}

#[test]
fn transport_keeps_metadata_centered_and_seeking_separate_from_navigation() {
    use textamp::app::{action::PlaybackAction, transport_controls::TransportControl};
    let mut state = fixture();
    state.playback.duration_ms = 123_000;
    for view in [View::Browse, View::NowPlaying] {
        state.view = view;
        draw(&mut state, 120, 35);
        let transport = state.hit_regions.transport.as_ref().unwrap().clone();
        assert_eq!(transport.area.height, 5);
        assert!(
            transport
                .controls
                .iter()
                .any(|(_, control)| *control == TransportControl::Lyrics)
        );
        let actions = mouse_input::handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: transport.seekbar.x + transport.seekbar.width / 2,
                row: transport.seekbar.y,
                modifiers: KeyModifiers::NONE,
            },
            &mut state,
        );
        assert!(matches!(
            actions.as_slice(),
            [Action::Playback(PlaybackAction::Seek(_))]
        ));
        assert_eq!(state.view, view);
        key_input::handle_key(
            KeyEvent::new(KeyCode::Char('t'), KeyModifiers::ALT),
            &mut state,
            &Config::default(),
        );
        assert_eq!(state.tui_transport_focus, Some(TransportControl::Play));
        for _ in 0..transport.controls.len() {
            if state.tui_transport_focus == Some(TransportControl::Seek) {
                break;
            }
            key(&mut state, KeyCode::Down);
        }
        assert_eq!(state.tui_transport_focus, Some(TransportControl::Seek));
        assert!(matches!(
            key(&mut state, KeyCode::Right).as_slice(),
            [Action::Playback(PlaybackAction::SeekRelative(5000))]
        ));
        key_input::handle_key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &mut state,
            &Config::default(),
        );
    }
}

#[test]
fn visible_artist_names_have_keyboard_access_through_the_shared_jump_action() {
    let mut state = fixture();
    state.view = View::NowPlaying;
    draw(&mut state, 120, 35);
    key_input::handle_key(
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT),
        &mut state,
        &Config::default(),
    );
    assert!(state.tui_artist_focus.is_some());
    let name = state.tui_artist_focus.as_ref().unwrap().name.clone();
    let actions = key_input::handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut state,
        &Config::default(),
    );
    assert!(
        matches!(actions.as_slice(),[Action::Browse(BrowseAction::OpenDetailsInLibrary{track,destination:textamp::app::library_details::Destination::Artist})] if track.grandparent_title.as_deref()==Some(name.as_str()))
    );
    assert!(state.tui_artist_focus.is_none());
}

#[test]
fn terminal_header_and_transport_focus_do_not_intercept_desktop_navigation() {
    let mut state = fixture();
    state.desktop_frontend = true;
    state.category_column_focused = false;
    state.artist_nav.focused_column = 0;
    key_input::handle_column_navigation(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), &mut state);
    assert!(state.tui_column_header.is_none());
    key_input::handle_key(
        KeyEvent::new(KeyCode::Char('t'), KeyModifiers::ALT),
        &mut state,
        &Config::default(),
    );
    assert!(state.tui_transport_focus.is_none());
}

#[test]
fn closing_a_filter_reveals_the_retained_selection_in_the_unfiltered_column() {
    use textamp::app::{action::SearchAction, handlers::dispatch_search};
    let mut state = fixture();
    state.tui_queue_visible = false;
    state.category_column_focused = false;
    state.artist_nav.columns[0].items = (0..100)
        .map(|index| BrowseItem::Artist {
            key: format!("a{index}"),
            title: format!("Artist {index:03}"),
            thumb: None,
            is_placeholder: false,
        })
        .collect();
    state.artist_nav.columns[0].selected_index = 60;
    state.scroll.browse = Some((0, 0));
    state.list_filter.active = true;
    state.list_filter.category = BrowseCategory::Library;
    state.list_filter.column = 0;
    state.list_filter.query = "060".into();
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    dispatch_search::dispatch(&tx, SearchAction::DeactivateListFilter, &mut state).unwrap();
    let buffer = draw(&mut state, 120, 32);
    let rows: Vec<String> = buffer
        .content
        .chunks(120)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect();
    let selected_row = rows
        .iter()
        .position(|row| row.contains("Artist 060"))
        .expect("retained artist must remain visible after removing the filter");
    assert!((10..=18).contains(&selected_row));
    assert_eq!(state.artist_nav.columns[0].selected_index, 60);
}

#[test]
fn tab_reaches_the_right_hand_alphabet_strip_without_changing_workspace() {
    let mut state = fixture();
    state.tui_queue_visible = false;
    state.category_column_focused = false;
    for _ in 0..8 {
        key(&mut state, KeyCode::Tab);
        if state.alphabet_strip_focused {
            break;
        }
    }
    assert!(state.alphabet_strip_focused);
    assert_eq!(state.view, View::Browse);
    key(&mut state, KeyCode::Left);
    assert!(!state.alphabet_strip_focused);
    assert!(!state.category_column_focused);
}

#[test]
fn opening_a_child_after_manual_pan_reveals_it_and_keeps_it_visible() {
    let mut state = fixture();
    state.tui_queue_visible = false;
    state.category_column_focused = false;
    for index in 1..4 {
        state.artist_nav.columns.push(BrowseColumn::new(
            format!("Column {index}"),
            vec![BrowseItem::Artist {
                key: format!("a{index}"),
                title: format!("Artist {index}"),
                thumb: None,
                is_placeholder: false,
            }],
        ));
    }
    draw(&mut state, 100, 30);
    state.miller_scroll_manual = true;
    state.miller_scroll_col = 0;
    draw(&mut state, 100, 30);
    state.artist_nav.drill_column(
        BrowseColumn::new(
            "Destination",
            vec![BrowseItem::Artist {
                key: "new".into(),
                title: "Destination Artist".into(),
                thumb: None,
                is_placeholder: false,
            }],
        ),
        false,
    );
    let last = state.artist_nav.columns.len() - 1;
    for _ in 0..3 {
        draw(&mut state, 100, 30);
        let region = state
            .hit_regions
            .miller_columns
            .as_ref()
            .unwrap()
            .columns
            .iter()
            .find(|region| region.col_idx == last)
            .expect("new destination must remain fully visible");
        assert!(region.area.right() <= 100);
        assert!(region.area.width >= 26);
    }
    state.artist_nav.focused_column = last - 1;
    state.miller_scroll_manual = true;
    state.miller_scroll_col = 0;
    draw(&mut state, 100, 30);
    let (tx, _) = tokio::sync::mpsc::channel(8);
    let request_id = state.artist_nav_request_id;
    let mut audio = textamp::audio::AudioPlayer::new_without_audio();
    let tracks = state.queue.tracks.clone();
    textamp::app::handlers::dispatch_miller::dispatch(
        &tx,
        textamp::app::action::MillerAction::AlbumTracksForMillerLoaded {
            request_id,
            album_key: "replacement".into(),
            album_title: "Replacement".into(),
            replace_child: true,
            result: Ok(tracks),
        },
        &mut state,
        &mut audio,
    )
    .unwrap();
    for _ in 0..2 {
        draw(&mut state, 100, 30);
        assert!(
            state
                .hit_regions
                .miller_columns
                .as_ref()
                .unwrap()
                .columns
                .iter()
                .any(|region| region.col_idx == last)
        );
    }
}

#[test]
fn description_field_submits_without_losing_history_or_opening_a_dialog() {
    let mut state = fixture();
    state.sources.nav_collection = Some(
        textamp::app::sources::navidrome::commands::CollectionKind::AudioMuse(
            textamp::audiomuse::Feature::Describe,
        ),
    );
    state.artist_nav.reset(
        "Description history",
        vec![BrowseItem::Description {
            query: "trumpet rock".into(),
        }],
    );
    draw(&mut state, 120, 32);
    let field = state.hit_regions.description_input.unwrap();
    click(&mut state, field);
    for c in "quiet jazz".chars() {
        key(&mut state, KeyCode::Char(c));
    }
    let actions = key(&mut state, KeyCode::Enter);
    let [Action::Source(textamp::app::sources::SourceAction::AudioMuse(command))] =
        actions.as_slice()
    else {
        panic!("description must use the shared source search command");
    };
    assert!(
        matches!(command.as_ref(), textamp::app::sources::audiomuse::Command::Search { query, .. } if query == "quiet jazz")
    );
    assert!(matches!(state.popups.modal(), Modal::Closed));
    assert_eq!(state.artist_nav.columns[0].items[0].title(), "trumpet rock");
}

#[test]
fn closing_the_decades_root_leaves_zero_columns() {
    use textamp::app::column_controls::ColumnControl;
    let mut state = fixture();
    state.tui_queue_visible = false;
    state.browse_category = BrowseCategory::Decades;
    state.tag_nav.reset(
        "Decades",
        vec![BrowseItem::Genre {
            key: "1990".into(),
            title: "1990s".into(),
        }],
    );
    draw(&mut state, 120, 32);
    let close = state.hit_regions.miller_columns.as_ref().unwrap().columns[0]
        .controls
        .iter()
        .find(|(_, control)| *control == ColumnControl::Close)
        .unwrap()
        .0;
    click(&mut state, close);
    state.active_library = Some("fixture-library".into());
    let (tx, _) = tokio::sync::mpsc::channel(8);
    let follow = textamp::app::handlers::dispatch_browse::dispatch(
        &tx,
        BrowseAction::TagListLoaded {
            library_key: "fixture-library".into(),
            section: BrowseCategory::Decades,
            result: Ok(vec![]),
        },
        &mut state,
    )
    .unwrap();
    assert!(
        follow.is_empty(),
        "a late tag refresh must not reopen a closed column"
    );
    for _ in 0..2 {
        draw(&mut state, 120, 32);
    }
    assert!(state.tag_nav.columns.is_empty());
    assert!(
        state
            .hit_regions
            .miller_columns
            .as_ref()
            .is_none_or(|regions| regions.columns.is_empty())
    );
}

#[test]
fn shift_click_next_in_track_details_queues_the_selected_track_and_following() {
    let mut state = fixture();
    state.tui_queue_visible = false;
    state.category_column_focused = false;
    let mut tracks = state.queue.tracks.clone();
    let mut second = tracks[0].clone();
    second.rating_key = "second".into();
    second.title = "Song B".into();
    tracks.push(second);
    let mut column = BrowseColumn::new("Tracks", BrowseItem::from_tracks(&tracks));
    column.tracks = tracks;
    state.artist_nav.columns.push(column);
    state.artist_nav.focused_column = 1;
    state.track_pane_open = true;
    draw(&mut state, 160, 40);
    let next = state.hit_regions.track_pane.as_ref().unwrap().next_button;
    let actions = mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: next.x,
            row: next.y,
            modifiers: KeyModifiers::SHIFT,
        },
        &mut state,
    );
    assert!(
        matches!(actions.as_slice(), [Action::Queue(textamp::app::action::QueueAction::EnqueueTracksNext(tracks))] if tracks.len() == 2 && tracks[1].rating_key == "second")
    );
    let buffer = draw(&mut state, 160, 40);
    let footer: String = (0..160).map(|x| buffer[(x, 39)].symbol()).collect();
    assert!(footer.contains("Play Next"));
}

#[test]
fn tab_traversal_preserves_workspace_and_operates_queue_tools() {
    let mut state = fixture();
    state.category_column_focused = true;
    draw(&mut state, 120, 32);
    for _ in 0..32 {
        key(&mut state, KeyCode::Tab);
        if state.view == View::Queue && state.now_playing_focus == NowPlayingFocus::Tools {
            break;
        }
    }
    assert_eq!(state.view, View::Queue);
    let actions = key(&mut state, KeyCode::Enter);
    assert!(matches!(
        actions.as_slice(),
        [Action::Queue(
            textamp::app::action::QueueAction::ToggleQueueShuffle
        )]
    ));
    key(&mut state, KeyCode::BackTab);
    assert_eq!(state.now_playing_focus, NowPlayingFocus::Tracks);
    key(&mut state, KeyCode::BackTab);
    assert_eq!(state.view, View::Browse);
    assert!(state.tui_artist_focus.is_some());
    assert_eq!(state.queue.tracks[0].rating_key, "track");
}

#[test]
fn arrows_reach_transport_controls_and_tab_leaves_the_panel() {
    use textamp::app::{action::PlaybackAction, transport_controls::TransportControl as Control};
    let mut state = fixture();
    state.playback.duration_ms = 123_000;
    draw(&mut state, 140, 40);
    key_input::handle_key(
        KeyEvent::new(KeyCode::Char('t'), KeyModifiers::ALT),
        &mut state,
        &Config::default(),
    );
    key(&mut state, KeyCode::Down); // Next
    key(&mut state, KeyCode::Down); // Track
    key(&mut state, KeyCode::Down); // Artist
    key(&mut state, KeyCode::Down); // Album
    key(&mut state, KeyCode::Down); // Position
    assert_eq!(state.tui_transport_focus, Some(Control::Seek));
    assert!(matches!(
        key(&mut state, KeyCode::Left).as_slice(),
        [Action::Playback(PlaybackAction::SeekRelative(-5000))]
    ));
    key(&mut state, KeyCode::Down); // Commands
    assert_eq!(state.tui_transport_focus, Some(Control::Menu));
    key(&mut state, KeyCode::Enter);
    assert!(state.palette.open && state.palette.player_context);
    key(&mut state, KeyCode::Esc);
    key(&mut state, KeyCode::Down); // Lyrics
    assert!(matches!(
        key(&mut state, KeyCode::Enter).as_slice(),
        [Action::Playback(PlaybackAction::ShowLyrics)]
    ));
    key(&mut state, KeyCode::Down); // Mute
    assert!(matches!(
        key(&mut state, KeyCode::Enter).as_slice(),
        [Action::Playback(PlaybackAction::ToggleMute)]
    ));
    key(&mut state, KeyCode::Down); // Volume
    assert!(matches!(
        key(&mut state, KeyCode::Right).as_slice(),
        [Action::Playback(PlaybackAction::VolumeUp)]
    ));
    key(&mut state, KeyCode::Down); // Player chevron
    assert_eq!(state.tui_transport_focus, Some(Control::Player));
    key(&mut state, KeyCode::Tab); // Footer
    assert_eq!(state.tui_footer_focus, Some(0));
    key(&mut state, KeyCode::BackTab);
    assert_eq!(state.tui_transport_focus, Some(Control::Player));
}

#[test]
fn clicking_the_track_card_toggles_the_player_both_ways_without_seeking() {
    let mut state = fixture();
    let (tx, _) = tokio::sync::mpsc::channel(8);
    for expected in [View::NowPlaying, View::Browse] {
        draw(&mut state, 140, 40);
        let transport = state.hit_regions.transport.as_ref().unwrap();
        let background =
            ratatui::layout::Rect::new(transport.seekbar.x, transport.track_info.unwrap().y, 1, 1);
        let actions = click(&mut state, background);
        assert!(
            matches!(actions.first(), Some(Action::Navigation(NavigationAction::SetView(view))) if *view == expected)
        );
        for action in actions {
            if let Action::Navigation(action) = action {
                dispatch_navigation::dispatch(&tx, action, &mut state).unwrap();
            }
        }
        assert_eq!(state.view, expected);
        assert_eq!(state.queue.index, Some(0));
    }
}

#[test]
fn review_layouts_keep_the_transport_and_drawer_frames_inside_the_window() {
    use std::time::{Duration, Instant};
    for (view, sidebar, queue, name, width, height) in [
        (View::Browse, true, true, "Library", 160, 42),
        (View::Browse, false, true, "Library-hidden-sidebar", 120, 32),
        (View::Browse, true, false, "Library-hidden-queue", 120, 32),
        (View::NowPlaying, false, true, "NowPlaying", 140, 40),
        (
            View::NowPlaying,
            false,
            false,
            "NowPlaying-hidden-queue",
            120,
            32,
        ),
        (View::NowPlaying, true, true, "Compact", 70, 22),
    ] {
        let mut state = fixture();
        if let Ok(name) = std::env::var("TEXTAMP_TUI_REVIEW_THEME") {
            state.theme = *textamp::app::theme::ThemeName::all()
                .iter()
                .find(|theme| theme.display_name() == name)
                .expect("known review theme");
        }
        state.view = view;
        state.tui_sidebar_visible = sidebar;
        state.tui_queue_visible = queue;
        let mut track = state.queue.tracks[0].clone();
        track.parent_rating_key = Some("album".into());
        track.parent_title = Some("Night Signals".into());
        track.title = "The Long Way Home".into();
        track.year = Some(2024);
        state.queue.tracks = (0..12)
            .map(|index| {
                let mut t = track.clone();
                t.rating_key = format!("track-{index}");
                if index > 0 {
                    t.title = format!("Track {}", index + 1);
                }
                t
            })
            .collect();
        state.queue.index = Some(0);
        state.playback_mode = PlaybackMode::Queue;
        state.playback.duration_ms = 123_000;
        state.playback.position_ms = 30_000;
        state.playback.status = PlayStatus::Playing;
        let pixels = image::RgbImage::from_fn(48, 48, |x, y| {
            image::Rgb([(x * 4) as u8, (80 + y * 3) as u8, (180 - x * 2) as u8])
        });
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(pixels)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let png = png.into_inner();
        state.artwork.current_data = Some(png.clone());
        state.artwork.current_thumb = Some("review-artwork".into());
        state.artwork.grid_cache.insert("album".into(), png);
        state.artwork.mode = ArtworkMode::Braille;
        state.waveform.track_key = Some("track-0".into());
        state.waveform.phase = AnalysisPhase::Ready(std::sync::Arc::new(
            textamp::media::generate_waveform_from_pcm(
                "track-0".into(),
                123_000,
                &(0..1024)
                    .map(|index| ((index as f32 / 17.).sin() * (index as f32 / 89.).cos()).abs())
                    .collect::<Vec<_>>(),
            ),
        ));
        let mut tracks = BrowseColumn::new_with_tracks(
            "Night Signals",
            BrowseItem::from_tracks(&state.queue.tracks),
            state.queue.tracks.clone(),
        );
        tracks.play_all_row = Some(PlayAllRow::Album {
            rating_key: "album".into(),
            title: "Night Signals".into(),
        });
        state.artist_nav.columns.push(tracks);
        state.artist_nav.focused_column = 1;
        state.category_column_focused = false;
        draw(&mut state, width, height);
        let transport = state.hit_regions.transport.as_ref().unwrap();
        assert!(transport.area.right() <= width && transport.area.bottom() <= height - 1);
        if queue {
            let tools = &state.hit_regions.queue_menus;
            assert!(tools.iter().all(|(rect, _)| rect.bottom() <= height - 2));
            if view == View::Browse {
                assert!(tools[0].0.y < transport.area.y);
            }
        }
        if let Some(columns) = &state.hit_regions.miller_columns {
            assert!(
                columns
                    .columns
                    .iter()
                    .all(|region| region.area.right() <= transport.area.right())
            );
        }
        if let Ok(directory) = std::env::var("TEXTAMP_TUI_REVIEW_DIR") {
            for extension in ["json", "txt"] {
                let root = std::path::Path::new(&directory);
                std::fs::copy(
                    root.join(format!("{view:?}-{width}x{height}.{extension}")),
                    root.join(format!("Review-{name}.{extension}")),
                )
                .unwrap();
            }
        }
    }
    let mut state = fixture();
    let start = Instant::now() - Duration::from_millis(80);
    state.tui_motion.initialize(true, false, false, start);
    state.tui_motion.sync(false, true, true, start);
    state.tui_sidebar_visible = false;
    state.tui_queue_visible = true;
    state.view = View::NowPlaying;
    draw(&mut state, 120, 32);
    assert!(state.hit_regions.player_overlay.is_some());
    let now = Instant::now();
    let presented = state.tui_motion.queue(true, now);
    state.tui_motion.sync(true, false, false, now);
    state.view = View::Browse;
    assert!(
        (presented - state.tui_motion.queue(false, now)).abs() < 0.001,
        "reversing an animation must preserve its presented position"
    );
    draw(&mut state, 120, 32);
}

#[test]
fn inline_filter_leaves_adjacent_columns_and_their_keyboard_navigation_intact() {
    let mut state = fixture();
    state.tui_queue_visible = false;
    state.category_column_focused = false;
    state.artist_nav.columns.push(BrowseColumn::new(
        "Albums",
        vec![
            BrowseItem::Album {
                key: "a".into(),
                title: "Unrelated release".into(),
                artist: "Artist A".into(),
                year: None,
                thumb: None,
                is_placeholder: false,
            },
            BrowseItem::Album {
                key: "b".into(),
                title: "Another release".into(),
                artist: "Artist A".into(),
                year: None,
                thumb: None,
                is_placeholder: false,
            },
        ],
    ));
    state.list_filter.active = true;
    state.list_filter.category = BrowseCategory::Library;
    state.list_filter.column = 0;
    state.list_filter.query = "Artist A".into();
    state.list_filter.results = Some(ListFilterResults {
        matched_indices: vec![0],
        total_matches: 1,
        has_more: false,
    });
    state.artist_nav.focused_column = 1;
    let buffer = draw(&mut state, 160, 32);
    let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(text.contains("Unrelated release"));
    assert!(text.contains("Another release"));
    let actions = key_input::handle_key(
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        &mut state,
        &Config::default(),
    );
    assert!(!actions.iter().any(|action| matches!(
        action,
        textamp::app::Action::Search(textamp::app::action::SearchAction::FilteredListDown)
    )));
    assert_eq!(state.artist_nav.columns.len(), 2);
    assert_eq!(state.artist_nav.focused_column, 1);
}

#[tokio::test]
async fn opening_artist_details_focuses_and_reveals_the_existing_destination() {
    use textamp::app::handlers::dispatch_search;
    let mut state = fixture();
    state.category_column_focused = false;
    state.artist_nav.columns.push(BrowseColumn::new(
        "Albums",
        vec![BrowseItem::ArtistInfo {
            artist_key: "artist".into(),
            artist_name: "Artist A".into(),
            thumb: None,
        }],
    ));
    let mut details = BrowseColumn::new("Artist Details", vec![]);
    details.details = Some(BrowseDetails::Artist(ArtistInfoDetails {
        artist_key: "artist".into(),
        artist_name: "Artist A".into(),
        request_id: 1,
        document: Default::default(),
        image_index: 0,
        task: None,
    }));
    state.artist_nav.columns.push(details);
    state.artist_nav.focused_column = 1;
    state.miller_scroll_manual = true;
    state.miller_scroll_col = 0;
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    dispatch_search::dispatch(
        &tx,
        textamp::app::action::SearchAction::ShowArtistInfo {
            artist_key: "artist".into(),
            artist_name: "Artist A".into(),
        },
        &mut state,
    )
    .unwrap();
    assert_eq!(state.artist_nav.focused_column, 2);
    draw(&mut state, 100, 32);
    assert!(
        state
            .hit_regions
            .miller_columns
            .as_ref()
            .unwrap()
            .columns
            .iter()
            .any(|region| region.col_idx == 2)
    );
    assert!(state.miller_scroll_col > 0);
    assert_eq!(state.artist_nav.columns.len(), 3);
}

#[test]
fn enter_on_a_library_track_opens_details_without_queue_actions() {
    let mut state = fixture();
    state.category_column_focused = false;
    state.artist_nav.columns = vec![BrowseColumn::new_with_tracks(
        "Tracks",
        BrowseItem::from_tracks(&state.queue.tracks),
        state.queue.tracks.clone(),
    )];
    let actions = key_input::handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut state,
        &Config::default(),
    );
    assert!(matches!(
        actions.as_slice(),
        [Action::Browse(BrowseAction::OpenTrackDetails)]
    ));
}

#[test]
fn backspace_restores_a_jump_but_keeps_its_text_editing_role_in_a_filter() {
    use textamp::app::{action::SearchAction, navigation_history};
    let mut state = fixture();
    navigation_history::record(&mut state);
    state.artist_nav = BrowseNavigationState::with_root("Jump", vec![]);
    let actions = key_input::handle_key(
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        &mut state,
        &Config::default(),
    );
    assert!(matches!(
        actions.as_slice(),
        [Action::Navigation(NavigationAction::Back)]
    ));
    state.category_column_focused = false;
    state.list_filter.active = true;
    state.list_filter.category = state.browse_category;
    state.list_filter.column = 0;
    state.list_filter.query = "Artist".into();
    let actions = key_input::handle_key(
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        &mut state,
        &Config::default(),
    );
    assert!(matches!(
        actions.as_slice(),
        [Action::Search(SearchAction::DeleteListFilterChar)]
    ));
}

#[test]
fn settings_is_a_bounded_popover_and_close_exits_from_the_content_pane() {
    let mut state = fixture();
    state.previous_view = Some(View::Browse);
    state.view = View::Settings;
    state.settings_state.section = SettingsSection::About;
    state.settings_state.focus = SettingsFocus::Content;
    let before = state.artist_nav.columns[0].title.clone();
    draw(&mut state, 120, 40);
    let popup = state.hit_regions.popup_outer.unwrap();
    assert!(popup.x > 0 && popup.y > 0 && popup.right() < 120 && popup.bottom() < 40);
    assert!(state.hit_regions.miller_columns.is_none());
    let close = state.hit_regions.popup_close.unwrap();
    assert!(matches!(
        click(&mut state, close).as_slice(),
        [Action::Navigation(NavigationAction::SetView(View::Browse))]
    ));
    state.previous_view = Some(View::NowPlaying);
    assert!(matches!(
        click(&mut state, close).as_slice(),
        [Action::Navigation(NavigationAction::SetView(
            View::NowPlaying
        ))]
    ));
    assert_eq!(state.artist_nav.columns[0].title, before);
}

#[test]
fn shift_status_changes_only_on_modifier_events_and_releases_do_not_activate() {
    use crossterm::event::{KeyEventKind, ModifierKeyCode};
    let mut state = fixture();
    state.track_pane_focused = true;
    state.track_pane_index = 1;
    assert_eq!(
        textamp::app::tui_navigation::status(&state).as_deref(),
        Some("Play Next")
    );
    for (kind, expected) in [
        (KeyEventKind::Press, "Play Track and Following Next"),
        (KeyEventKind::Release, "Play Next"),
    ] {
        assert!(
            key_input::handle_key(
                KeyEvent::new_with_kind(
                    KeyCode::Modifier(ModifierKeyCode::LeftShift),
                    KeyModifiers::SHIFT,
                    kind
                ),
                &mut state,
                &Config::default()
            )
            .is_empty()
        );
        assert_eq!(
            textamp::app::tui_navigation::status(&state).as_deref(),
            Some(expected)
        );
    }
    assert!(
        key_input::handle_key(
            KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Release),
            &mut state,
            &Config::default()
        )
        .is_empty()
    );
    assert!(
        key_input::handle_key(
            KeyEvent::new_with_kind(
                KeyCode::Char('^'),
                KeyModifiers::SHIFT,
                KeyEventKind::Repeat
            ),
            &mut state,
            &Config::default()
        )
        .is_empty()
    );
}

#[test]
fn reversed_player_transitions_do_not_include_library_cells_inside_the_player() {
    use std::time::{Duration, Instant};
    let mut state = fixture();
    state.tui_sidebar_visible = false;
    state.tui_queue_visible = false;
    state.artist_nav.columns = vec![BrowseColumn::new(
        "Library Marker",
        (0..80)
            .map(|index| BrowseItem::Artist {
                key: index.to_string(),
                title: "LIBRARYLEAK".into(),
                thumb: None,
                is_placeholder: false,
            })
            .collect(),
    )];
    state.queue.tracks.clear();
    state.queue.index = None;
    state
        .tui_motion
        .initialize(false, false, false, Instant::now() - Duration::from_secs(1));
    draw(&mut state, 100, 35);
    for view in [
        View::NowPlaying,
        View::Browse,
        View::NowPlaying,
        View::Browse,
        View::NowPlaying,
    ] {
        state.view = view;
        state.tui_motion.sync(
            false,
            false,
            view == View::NowPlaying,
            Instant::now() - Duration::from_millis(75),
        );
        let buffer = draw(&mut state, 100, 35);
        if let Some(overlay) = state.hit_regions.player_overlay {
            let text: String = (overlay.y..overlay.bottom())
                .flat_map(|y| (overlay.x..overlay.right()).map(move |x| (x, y)))
                .map(|(x, y)| buffer[(x, y)].symbol())
                .collect();
            assert!(!text.contains("LIBRARYLEAK"));
        }
    }
}

#[test]
fn artist_artwork_uses_the_library_artwork_preference_and_reclaims_hidden_space() {
    let mut state = fixture();
    state.tui_queue_visible = false;
    state.tui_sidebar_visible = false;
    let mut detail = BrowseColumn::new("Artist Details", vec![]);
    detail.details = Some(BrowseDetails::Artist(ArtistInfoDetails {
        artist_key: "artist".into(),
        artist_name: "Artist A".into(),
        request_id: 1,
        document: textamp::services::biography::Biography {
            text: "Biography marker".into(),
            ..Default::default()
        },
        image_index: 0,
        task: None,
    }));
    state.artist_nav.columns = vec![detail];
    state.artist_nav.columns[0].artwork_visible = false;
    let hidden = draw(&mut state, 80, 30);
    let hidden_y = hidden
        .content
        .chunks(80)
        .position(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .contains("Biography marker")
        })
        .unwrap();
    state.artist_nav.columns[0].artwork_visible = true;
    let shown = draw(&mut state, 80, 30);
    let shown_y = shown
        .content
        .chunks(80)
        .position(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .contains("Biography marker")
        })
        .unwrap();
    assert!(shown_y > hidden_y);
}

#[test]
fn album_artwork_captions_move_below_large_covers_and_keep_the_year() {
    let mut state = fixture();
    state.tui_sidebar_visible = false;
    state.tui_queue_visible = false;
    state.queue.tracks.clear();
    state.queue.index = None;
    let mut column = BrowseColumn::new(
        "Albums",
        vec![BrowseItem::Album {
            key: "album".into(),
            title: "Night Signals".into(),
            artist: "Hidden Artist Credit".into(),
            year: Some(2024),
            thumb: None,
            is_placeholder: false,
        }],
    );
    column.artwork_visible = true;
    state.artist_nav.columns = vec![column];
    for large in [false, true] {
        state.large_artwork = large;
        let buffer = draw(&mut state, 80, 32);
        let rows = buffer
            .content
            .chunks(80)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>();
        let (y, x) = rows
            .iter()
            .enumerate()
            .find_map(|(y, row)| {
                row.find("Night Signals (2024)")
                    .map(|x| (y, unicode_width::UnicodeWidthStr::width(&row[..x])))
            })
            .expect("album caption includes its year");
        let inner = state.hit_regions.miller_columns.as_ref().unwrap().columns[0].inner;
        if large {
            assert_eq!(x, usize::from(inner.x));
            assert!(y > usize::from(inner.y + 1));
        } else {
            assert!(x > usize::from(inner.x));
            assert!(y <= usize::from(inner.y + 1));
        }
        assert!(!rows.iter().any(|row| row.contains("Hidden Artist Credit")));
        let (_, selection, _) = state.hit_regions.miller_item_rows[0];
        let caption_background = buffer[(x as u16, y as u16)].bg;
        assert_eq!(
            buffer[(selection.right() - 1, selection.y)].bg,
            caption_background
        );
        assert_eq!(
            buffer[(selection.right() - 1, selection.bottom() - 1)].bg,
            caption_background
        );
    }
}

#[test]
fn filtered_artwork_clicks_select_the_visible_original_item() {
    let mut state = fixture();
    state.tui_sidebar_visible = false;
    state.tui_queue_visible = false;
    state.large_artwork = true;
    let mut column = BrowseColumn::new(
        "Albums",
        (0..20)
            .map(|index| BrowseItem::Album {
                key: format!("album-{index}"),
                title: format!("Album {index}"),
                artist: "Artist A".into(),
                year: Some(2000),
                thumb: None,
                is_placeholder: false,
            })
            .collect(),
    );
    column.artwork_visible = true;
    column.selected_index = 17;
    state.artist_nav.columns = vec![column];
    state.list_filter.active = true;
    state.list_filter.category = state.browse_category;
    state.list_filter.column = 0;
    state.list_filter.query = "Album".into();
    state.list_filter.results = Some(ListFilterResults {
        matched_indices: vec![3, 17],
        total_matches: 2,
        ..Default::default()
    });
    draw(&mut state, 100, 32);
    let (_, area, item) = state.hit_regions.miller_item_rows[0];
    assert_eq!(item, 3);
    let actions = mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: area.x + 1,
            row: area.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        &mut state,
    );
    assert_eq!(state.artist_nav.columns[0].selected_index, 3);
    assert!(
        state.scroll.browse.is_some(),
        "pointer selection must pin the viewport"
    );
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, Action::Playback(_)))
    );
}

#[test]
fn tab_moves_between_columns_without_changing_their_selected_items() {
    let mut state = fixture();
    state.view = View::Browse;
    state.category_column_focused = false;
    state.artist_nav.columns = (0..3)
        .map(|column| {
            BrowseColumn::new(
                format!("Column {column}"),
                (0..3)
                    .map(|row| BrowseItem::Artist {
                        key: format!("{column}-{row}"),
                        title: format!("Artist {row}"),
                        thumb: None,
                        is_placeholder: false,
                    })
                    .collect(),
            )
        })
        .collect();
    state.artist_nav.focused_column = 0;
    state.artist_nav.columns[0].selected_index = 1;
    state.artist_nav.columns[1].selected_index = 2;
    draw(&mut state, 160, 40);
    key(&mut state, KeyCode::Tab);
    assert_eq!(state.artist_nav.focused_column, 1);
    assert_eq!(state.artist_nav.columns[0].selected_index, 1);
    assert_eq!(state.artist_nav.columns[1].selected_index, 2);
    key(&mut state, KeyCode::Tab);
    assert_eq!(state.artist_nav.focused_column, 2);
    key(&mut state, KeyCode::BackTab);
    assert_eq!(state.artist_nav.focused_column, 1);
    assert_eq!(state.artist_nav.columns[1].selected_index, 2);
}
