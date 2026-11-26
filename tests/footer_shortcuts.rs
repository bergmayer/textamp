use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};
use textamp::app::{
    Action, AppState,
    action::SearchAction,
    handlers::{key_input, mouse_input},
    state::View,
};
use textamp::config::Config;

fn draw(app: &mut AppState, width: u16) {
    app.terminal_width = width;
    app.terminal_height = 32;
    Terminal::new(TestBackend::new(width, 32))
        .unwrap()
        .draw(|frame| app.apply_render_feedback(textamp::ui::render(frame, app)))
        .unwrap();
}

#[test]
fn footer_search_is_keyboard_and_mouse_accessible_and_modals_block_click_through() {
    let mut app = AppState::new();
    app.view = View::Browse;
    app.tui_footer_focus = textamp::app::tui_navigation::FOOTER
        .iter()
        .position(|entry| entry.0 == KeyCode::Char('f'));
    let actions = key_input::handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut app,
        &Config::default(),
    );
    assert!(matches!(
        actions.as_slice(),
        [Action::Search(SearchAction::OpenSearchPopup)]
    ));
    draw(&mut app, 100);
    let rect = app
        .hit_regions
        .footer_controls
        .iter()
        .find(|(_, index)| textamp::app::tui_navigation::FOOTER[*index].0 == KeyCode::Char('f'))
        .unwrap()
        .0;
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x,
        row: rect.y,
        modifiers: KeyModifiers::NONE,
    };
    assert!(matches!(
        mouse_input::handle_mouse(click, &mut app).as_slice(),
        [Action::Search(SearchAction::OpenSearchPopup)]
    ));
    textamp::app::command_palette::open(&mut app);
    draw(&mut app, 100);
    assert!(mouse_input::handle_mouse(click, &mut app).is_empty());
    assert!(!app.popups.search_active());
}

#[test]
fn narrow_footer_keeps_navigation_and_status_out_of_each_others_hit_targets() {
    for width in [28, 40, 80, 140] {
        let mut app = AppState::new();
        app.set_status("Loading album details…".into());
        draw(&mut app, width);
        let left = app.hit_regions.sidebar_toggle.unwrap();
        let right = app.hit_regions.queue_tabs[0].0;
        assert!(left.right() <= right.x && right.right() <= width);
        let controls = &app.hit_regions.footer_controls;
        for (i, (rect, _)) in controls.iter().enumerate() {
            assert!(
                controls[i + 1..]
                    .iter()
                    .all(|(other, _)| rect.intersection(*other).is_empty())
            );
        }
        assert!(controls.iter().all(|(rect, _)| rect.right() <= width));
    }
}

#[test]
fn queue_rows_are_centered_and_footer_has_its_own_separator() {
    for width in [50, 100, 140] {
        let mut app = AppState::new();
        app.tui_queue_visible = true;
        app.terminal_width = width;
        app.terminal_height = 32;
        let mut terminal = Terminal::new(TestBackend::new(width, 32)).unwrap();
        terminal
            .draw(|frame| app.apply_render_feedback(textamp::ui::render(frame, &app)))
            .unwrap();
        let transport = app.hit_regions.transport.as_ref().unwrap();
        let controls = &app.hit_regions.queue_menus;
        assert_eq!(controls.len(), 2);
        assert_eq!(controls[0].0.x, controls[1].0.x);
        assert!(controls[0].0.y < controls[1].0.y);
        assert_eq!(
            app.hit_regions
                .queue_content
                .as_ref()
                .unwrap()
                .track_list
                .bottom(),
            transport.area.y
        );
        let buffer = terminal.backend().buffer();
        assert!((0..width).all(|x| matches!(buffer[(x, 30)].symbol(), "─" | "┴" | "┘")));
        assert_eq!(transport.area.bottom(), 30);
        assert!(app.hit_regions.footer_controls.iter().all(
            |(rect, _)| rect.y == 31 || (rect.x == controls[0].0.x && rect.y < controls[0].0.y)
        ));
    }
}

#[test]
fn numbered_navigation_and_queue_shortcuts_work_from_every_workspace() {
    use textamp::app::action::NavigationAction;
    let config = Config::default();
    for desktop in [false, true] {
        for view in [View::Browse, View::NowPlaying, View::Settings] {
            let mut app = AppState::new();
            app.desktop_frontend = desktop;
            app.view = view;
            for key in ['2', 'u'] {
                for _ in 0..2 {
                    let before = app.tui_queue_visible;
                    let actions = key_input::handle_key(
                        KeyEvent::new(KeyCode::Char(key), KeyModifiers::CONTROL),
                        &mut app,
                        &config,
                    );
                    assert!(matches!(
                        actions.as_slice(),
                        [Action::Navigation(NavigationAction::ToggleQueuePanel)]
                    ));
                    textamp::app::handlers::dispatch_navigation::dispatch(
                        &tokio::sync::mpsc::channel(1).0,
                        NavigationAction::ToggleQueuePanel,
                        &mut app,
                    )
                    .unwrap();
                    assert_eq!(app.tui_queue_visible, !before);
                    assert_eq!(app.view, view);
                }
            }
            let actions = key_input::handle_key(
                KeyEvent::new(KeyCode::Char('3'), KeyModifiers::CONTROL),
                &mut app,
                &config,
            );
            assert!(matches!(
                actions.as_slice(),
                [Action::Navigation(NavigationAction::SetView(
                    View::NowPlaying
                ))]
            ));
        }
    }
}

#[test]
fn sidebar_trash_targets_the_visible_queue_tab() {
    use textamp::app::action::QueueAction;
    let mut app = AppState::new();
    app.tui_queue_visible = true;
    app.set_view(View::Queue);
    app.queue.tracks = vec![Default::default()].into();
    app.listening_history = vec![textamp::app::listening_history::Play::from_track(
        &app.queue.tracks[0],
    )]
    .into();
    for history in [false, true] {
        app.showing_queue_history = history;
        draw(&mut app, 120);
        let rect = app.hit_regions.queue_clear.unwrap();
        let actions = mouse_input::handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: rect.x + 1,
                row: rect.y,
                modifiers: KeyModifiers::NONE,
            },
            &mut app,
        );
        assert!(matches!(
            (history, actions.as_slice()),
            (false, [Action::Queue(QueueAction::ClearQueue)])
                | (true, [Action::Queue(QueueAction::ClearHistory)])
        ));
    }
}

#[test]
fn history_track_click_selects_the_occurrence_for_keyboard_commands() {
    use textamp::app::{action::QueueAction, listening_history::Play, track_context::ContextKind};
    let mut app = AppState::new();
    app.tui_queue_visible = true;
    app.showing_queue_history = true;
    let plays = vec![
        Play::from_track(&Default::default()),
        Play::from_track(&Default::default()),
    ];
    let id = plays[1].id.clone();
    app.listening_history = plays.into();
    draw(&mut app, 120);
    let rect = app.hit_regions.history_rows[1].0;
    mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        },
        &mut app,
    );
    for key in [KeyCode::Enter, KeyCode::Right, KeyCode::Char('+')] {
        let actions = key_input::handle_key(
            KeyEvent::new(key, KeyModifiers::NONE),
            &mut app,
            &Config::default(),
        );
        assert!(
            matches!(actions.as_slice(),[Action::Queue(QueueAction::HistoryTrack(target,command))] if target==&id && matches!((key,command),(KeyCode::Enter,ContextKind::PlayTrack)|(KeyCode::Right,ContextKind::ShowDetails(textamp::app::library_details::Destination::Track))|(KeyCode::Char('+'),ContextKind::PlayNextInQueue)))
        );
    }
}

#[test]
fn queue_type_header_is_clickable_and_keyboard_reachable() {
    let mut app = AppState::new();
    app.tui_queue_visible = true;
    app.set_view(View::Queue);
    app.tui_queue_type_focused = true;
    draw(&mut app, 140);
    let rect = app
        .hit_regions
        .queue_menus
        .iter()
        .find(|(_, menu)| *menu == textamp::app::queue_controls::Menu::QueueType)
        .unwrap()
        .0;
    assert_eq!(rect.height, 1);
    key_input::handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut app,
        &Config::default(),
    );
    assert!(app.palette.open);
    assert_eq!(app.palette.path, ["Queue", "Queue Type"]);
    app.palette.close();
    mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x + 2,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        },
        &mut app,
    );
    assert!(app.palette.open);
    assert_eq!(app.palette.path, ["Queue", "Queue Type"]);
}

#[test]
fn queue_tools_menu_supports_arrows_mouse_and_live_capabilities() {
    use textamp::app::{
        queue_controls::Menu,
        state::{NowPlayingFocus, PaletteCommandKind},
    };
    let mut app = AppState::new();
    app.tui_queue_visible = true;
    app.set_view(View::Queue);
    app.tui_queue_type_focused = true;
    draw(&mut app, 100);
    key_input::handle_key(
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        &mut app,
        &Config::default(),
    );
    assert_eq!(app.now_playing_focus, NowPlayingFocus::Tools);
    assert_eq!(
        textamp::app::tui_navigation::status(&app).as_deref(),
        Some("Queue Tools")
    );
    key_input::handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut app,
        &Config::default(),
    );
    assert!(app.palette.open);
    assert_eq!(app.palette.path, ["Queue", "Queue Tools"]);
    let tools: Vec<_> = app
        .palette
        .entries
        .iter()
        .filter(|entry| matches!(entry.command, PaletteCommandKind::QueueTool(_)))
        .collect();
    assert_eq!(tools.first().unwrap().label, "Shuffle");
    assert_eq!(tools.last().unwrap().label, "Clear Queue");
    assert!(
        textamp::app::command_catalog::disabled_reason(&app, &tools.last().unwrap().command)
            .is_some()
    );
    app.palette.close();
    draw(&mut app, 100);
    let rect = app
        .hit_regions
        .queue_menus
        .iter()
        .find(|(_, menu)| *menu == Menu::Tools)
        .unwrap()
        .0;
    mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        },
        &mut app,
    );
    assert!(app.palette.open);
    assert_eq!(app.palette.path, ["Queue", "Queue Tools"]);
}

#[test]
fn downloaded_size_does_not_hide_focused_transport_help() {
    use textamp::app::{downloads, transport_controls::TransportControl};
    use textamp::services::downloads::{Entry, Store};
    for width in [100, 140] {
        let mut app = AppState::new();
        let track = textamp::library::models::Track::from_folder("fixture", "song.wav");
        app.downloads.store.entries.insert(
            Store::id("fixture", &track),
            Entry {
                library: "fixture".into(),
                library_name: "Fixture".into(),
                track,
                bytes: 5 * 1024 * 1024,
                artwork: false,
            },
        );
        downloads::open(&mut app);
        app.tui_transport_focus = Some(TransportControl::Play);
        app.set_status("Caching playlists and artwork…".into());
        app.terminal_width = width;
        app.terminal_height = 32;
        let mut terminal = Terminal::new(TestBackend::new(width, 32)).unwrap();
        terminal
            .draw(|frame| app.apply_render_feedback(textamp::ui::render(frame, &app)))
            .unwrap();
        let footer = (0..width)
            .map(|x| terminal.backend().buffer()[(x, 31)].symbol())
            .collect::<String>();
        assert!(footer.contains("1 item · 5.0 MB"));
        assert!(footer.contains("Play / Pause"));
        assert!(!footer.contains("Caching"));
    }
}

#[test]
fn pending_download_rows_and_notification_have_distinct_click_targets() {
    use textamp::app::{
        downloads,
        state::{BrowseColumn, BrowseItem, NotificationTarget},
    };
    for width in [80, 140] {
        let mut app = AppState::new();
        app.view = View::Browse;
        app.library_navigation = textamp::app::navigation_history::LibraryNavigation::Columns(None);
        let track = textamp::library::models::Track::from_folder("fixture", "Pending Song.wav");
        let mut items = BrowseItem::from_tracks(std::slice::from_ref(&track));
        if let BrowseItem::Track { download_id, .. } = &mut items[0] {
            *download_id = Some("transfer".into());
        }
        app.artist_nav.columns = vec![BrowseColumn::new_with_tracks(
            "Downloads",
            items,
            vec![track],
        )];
        app.set_status("Downloaded another song".into());
        app.notifications.status_target = Some(NotificationTarget::Downloads);
        draw(&mut app, width);
        let cancel = app.hit_regions.download_cancel[0].0;
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: cancel.x,
            row: cancel.y,
            modifiers: KeyModifiers::NONE,
        };
        assert!(
            matches!(mouse_input::handle_mouse(click, &mut app).as_slice(),
            [Action::System(textamp::app::action::SystemAction::Downloads(downloads::Command::Cancel(id)))] if id == "transfer")
        );
        assert!(matches!(
            key_input::handle_key(
                KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE),
                &mut app,
                &Config::default()
            )
            .as_slice(),
            [Action::System(
                textamp::app::action::SystemAction::Downloads(downloads::Command::Cancel(_))
            )]
        ));
        app.tui_footer_focus = textamp::app::tui_navigation::FOOTER
            .iter()
            .position(|entry| entry.0 == KeyCode::Char('f'));
        draw(&mut app, width);
        assert!(app.hit_regions.download_status.is_none());
    }
}
