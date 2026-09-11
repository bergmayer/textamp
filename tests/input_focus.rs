use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::TestBackend, Terminal};
use textamp::app::action::{NavigationAction, SearchAction};
use textamp::app::handlers::{key_input, mouse_input};
use textamp::app::state::{BrowseCategory, InputDialog, InputDialogAction, NowPlayingFocus, View};
use textamp::app::{Action, AppState};
use textamp::config::Config;

fn key(state: &mut AppState, code: KeyCode) -> Vec<Action> {
    key_input::handle_key(
        KeyEvent::new(code, KeyModifiers::NONE),
        state,
        &Config::default(),
    )
}
fn render(state: &mut AppState) -> Terminal<TestBackend> {
    state.terminal_width = 120;
    state.terminal_height = 42;
    let mut terminal = Terminal::new(TestBackend::new(120, 42)).unwrap();
    terminal
        .draw(|f| {
            state.hit_regions = textamp::ui::render(f, state).hit_regions;
        })
        .unwrap();
    terminal
}
fn mouse(state: &mut AppState, kind: MouseEventKind, x: u16, y: u16) -> Vec<Action> {
    mouse_input::handle_mouse(
        MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        },
        state,
    )
}
fn queue_state(view: View, tall: bool) -> AppState {
    let mut state = AppState::new();
    state.view = view;
    state.tall_mode = tall;
    state.queue.tracks = (0..10)
        .map(|i| textamp::library::models::Track {
            title: format!("Track {i}"),
            rating_key: i.to_string(),
            ..Default::default()
        })
        .collect();
    state
}

#[test]
fn tab_uses_navigation_dispatch_in_both_directions_in_every_browse_category() {
    for tall in [false, true] {
        for category in BrowseCategory::all() {
            let mut state = AppState::new();
            state.view = View::Browse;
            state.browse_category = *category;
            state.tall_mode = tall;
            for code in [KeyCode::Tab, KeyCode::BackTab] {
                assert!(matches!(
                    key(&mut state, code).as_slice(),
                    [Action::Navigation(NavigationAction::SetView(
                        View::NowPlaying
                    ))]
                ));
            }
            for view in [View::Queue, View::NowPlaying] {
                state.view = view;
                assert!(matches!(
                    key(&mut state, KeyCode::Tab).as_slice(),
                    [Action::Navigation(NavigationAction::SetView(View::Browse))]
                ));
            }
        }
    }
}

#[test]
fn split_library_loses_its_focus_border_when_now_playing_is_active() {
    let mut state = queue_state(View::Browse, true);
    state.category_column_focused = true;
    let before = render(&mut state);
    let area = state.hit_regions.tall_mode_split.as_ref().unwrap().top;
    let focused = textamp::ui::theme::theme().colors.title_focused;
    assert!((area.y..area.bottom()).any(|y| before.backend().buffer()[(area.x, y)].fg == focused));
    state.set_view(View::NowPlaying);
    let after = render(&mut state);
    assert!(
        !(area.y..area.bottom()).any(|y| after.backend().buffer()[(area.x, y)].fg == focused),
        "inactive library still looks focused"
    );
}

#[test]
fn clicking_queue_in_now_playing_routes_following_arrow_keys_to_tracks() {
    for tall in [false, true] {
        let mut state = queue_state(View::NowPlaying, tall);
        render(&mut state);
        let rect = state
            .hit_regions
            .queue_content
            .as_ref()
            .unwrap()
            .track_list_inner;
        mouse(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            rect.x,
            rect.y,
        );
        assert_eq!(state.view, View::Queue);
        key(&mut state, KeyCode::Down);
        assert_eq!(state.list_state.queue_index, 1);
    }
}

#[test]
fn clicking_artwork_never_selects_or_plays_a_queue_row() {
    let mut state = queue_state(View::Queue, true);
    render(&mut state);
    let rect = state.hit_regions.queue_content.as_ref().unwrap().art_area;
    let actions = mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        rect.x + 1,
        rect.y + 3,
    );
    assert!(actions.is_empty(), "{actions:?}");
    assert_eq!(state.now_playing_focus, NowPlayingFocus::Artwork);
    assert_eq!(state.list_state.queue_index, 0);
}

#[test]
fn clicking_visualizer_tabs_moves_keyboard_focus_out_of_queue() {
    let mut state = queue_state(View::Queue, true);
    render(&mut state);
    let rect = state
        .hit_regions
        .now_playing_content
        .as_ref()
        .unwrap()
        .visualizer_tab_area;
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        rect.x,
        rect.y,
    );
    assert_eq!(state.view, View::NowPlaying);
    assert!(state.visualizer_tab_focused);
    let old = state.visualizer_tab;
    key(&mut state, KeyCode::Right);
    assert_ne!(state.visualizer_tab, old);
}

#[test]
fn wheel_over_queue_works_with_either_half_focused_without_stealing_focus() {
    for view in [View::Browse, View::Queue, View::NowPlaying] {
        let mut state = queue_state(view, true);
        render(&mut state);
        let rect = state
            .hit_regions
            .queue_content
            .as_ref()
            .unwrap()
            .track_list_inner;
        mouse(&mut state, MouseEventKind::ScrollDown, rect.x, rect.y);
        assert_eq!(state.list_state.queue_index, 1, "view {view:?}");
        assert_eq!(state.view, view);
    }
}

#[test]
fn search_owns_punctuation_and_function_keys_instead_of_global_commands() {
    let mut state = queue_state(View::Browse, true);
    state.popups.search_active = true;
    for c in [':', '/', '|', '\\', ',', '?', '<', '>'] {
        let actions = key(&mut state, KeyCode::Char(c));
        assert!(
            matches!(
                actions.as_slice(),
                [Action::Search(SearchAction::ExecuteLocalSearch)]
            ),
            "{c}: {actions:?}"
        );
        assert!(state.search.query.ends_with(c));
        assert!(!state.palette.open);
    }
    assert!(key(&mut state, KeyCode::F(3)).is_empty());
}

#[test]
fn text_dialog_blocks_clicks_drags_wheels_and_right_clicks_on_underlying_player() {
    let mut state = queue_state(View::Queue, true);
    state.popups.input_dialog = Some(InputDialog {
        title: "Rename".into(),
        input: "name".into(),
        action_type: InputDialogAction::FolderName("folder".into()),
    });
    render(&mut state);
    let rect = state
        .hit_regions
        .queue_content
        .as_ref()
        .unwrap()
        .track_list_inner;
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Down(MouseButton::Right),
        MouseEventKind::ScrollDown,
        MouseEventKind::Drag(MouseButton::Left),
    ] {
        let actions = mouse(&mut state, kind, rect.x, rect.y + 3);
        assert!(actions.is_empty(), "{actions:?}");
        assert_eq!(state.list_state.queue_index, 0);
        assert!(!state.palette.open);
    }
}

#[tokio::test]
async fn dispatched_tab_round_trips_all_providers_and_clears_old_filter_capture() {
    use textamp::app::sources::{navidrome::Session, ActiveSource};
    for provider in ["none", "navidrome", "local", "webdav"] {
        for tall in [false, true] {
            let mut state = queue_state(View::Browse, tall);
            state.queue.tracks.clear(); // Switching focus must not need any media or network.
            state.browse_category = BrowseCategory::Folders;
            state.category_column_focused = false;
            if provider == "navidrome" {
                let source = textamp::navidrome::Source {
                    id: "fixture".into(),
                    name: "Fixture".into(),
                    username: "fixture".into(),
                    url: "http://example.test/".into(),
                    libraries: vec![],
                };
                state.sources.active = ActiveSource::Navidrome(Box::new(Session {
                    client: textamp::navidrome::Client::new(&source, "fixture".into(), None)
                        .unwrap(),
                    source,
                    extensions: Default::default(),
                }));
            } else if provider != "none" {
                state.sources.active = ActiveSource::Folder(provider.into());
                state.sources.folders.push(textamp::library::FolderSource {
                    id: provider.into(),
                    name: provider.into(),
                    location: if provider == "local" {
                        textamp::library::FolderLocation::Local {
                            path: "/fixture".into(),
                        }
                    } else {
                        textamp::library::FolderLocation::Webdav {
                            url: "http://example.test/".into(),
                            username: None,
                            password_env: None,
                        }
                    },
                });
            }

            let mut audio = textamp::audio::AudioPlayer::new_without_audio();
            let (tx, _rx) = tokio::sync::mpsc::channel(32);
            let mut config = Config::default();
            for code in [KeyCode::Tab, KeyCode::BackTab] {
                state.list_filter.active = true;
                state.select_mode = true;
                for expected in [View::NowPlaying, View::Browse] {
                    for action in key(&mut state, code) {
                        textamp::app::dispatch::dispatch_action(
                            action,
                            &mut state,
                            &mut audio,
                            &mut config,
                            &tx,
                        )
                        .await
                        .unwrap();
                    }
                    assert_eq!(state.view, expected, "{provider}, split={tall}");
                    assert!(!state.list_filter.active && !state.select_mode);
                }
            }
        }
    }
}

#[test]
fn right_click_uses_two_row_queue_geometry_and_preserves_viewport() {
    let mut state = queue_state(View::Browse, true);
    state.scroll.queue = Some(2);
    render(&mut state);
    let area = state
        .hit_regions
        .queue_content
        .as_ref()
        .unwrap()
        .track_list_inner;
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Right),
        area.x,
        area.y + 1,
    );
    assert_eq!(state.view, View::Queue);
    assert_eq!(state.list_state.queue_index, 2);
    assert_eq!(state.scroll.queue, Some(2));
    assert!(state.palette.open);
}

#[test]
fn library_switcher_remains_usable_from_authentication_screen() {
    let mut state = AppState::new();
    assert!(matches!(
        key(&mut state, KeyCode::F(3)).as_slice(),
        [Action::Search(SearchAction::OpenLibraryPicker)]
    ));
    state.popups.library_picker_active = true;
    state
        .sources
        .folders
        .push(textamp::app::sources::source_from_location("/fixture").unwrap());
    assert!(matches!(
        key(&mut state, KeyCode::Enter).as_slice(),
        [Action::Source(_)]
    ));
}

#[test]
fn authentication_and_inline_filter_capture_layout_punctuation() {
    let mut state = AppState::new();
    state.view = View::Browse;
    state.list_filter.active = true;
    for c in "?:/|,<>\\".chars() {
        assert!(matches!(
            key(&mut state, KeyCode::Char(c)).as_slice(),
            [Action::Search(SearchAction::AppendListFilterChar(_))]
        ));
    }
}

#[test]
fn quit_is_available_while_any_input_owner_is_active() {
    for owner in 0..6 {
        let mut state = AppState::new();
        match owner {
            0 => state.popups.search_active = true,
            1 => textamp::app::command_palette::open(&mut state),
            2 => {
                state.popups.library_dialog = Some(textamp::app::sources::dialogs::Dialog::Webdav(
                    textamp::app::sources::dialogs::WebdavForm::new(None),
                ))
            }
            3 => {
                state.popups.input_dialog = Some(InputDialog {
                    title: "Input".into(),
                    input: "".into(),
                    action_type: InputDialogAction::SavePlaylist,
                })
            }
            4 => state.notifications.last_error = Some("fixture error".into()),
            _ => state.popups.library_picker_active = true,
        }
        let actions = key_input::handle_key(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
            &mut state,
            &Config::default(),
        );
        assert!(matches!(
            actions.as_slice(),
            [Action::System(textamp::app::action::SystemAction::Quit)]
        ));
    }
}

#[test]
fn help_end_reaches_wrapped_bottom_and_scrolling_is_bounded_in_split_mode() {
    for tall in [false, true] {
        let mut state = AppState::new();
        state.view = View::Help;
        state.tall_mode = tall;
        state.terminal_height = 30;
        state.terminal_width = 55;
        let mut terminal = Terminal::new(TestBackend::new(55, 30)).unwrap();
        terminal
            .draw(|f| {
                state.hit_regions = textamp::ui::render(f, &state).hit_regions;
            })
            .unwrap();
        let region = state.hit_regions.help.clone().unwrap();
        assert!(region.lines > textamp::util::help_text::total_lines());
        key(&mut state, KeyCode::End);
        assert_eq!(state.help_scroll, region.max_scroll());
        key(&mut state, KeyCode::Down);
        assert_eq!(state.help_scroll, region.max_scroll());
        key(&mut state, KeyCode::PageUp);
        assert!(state.help_scroll < region.max_scroll());
        key(&mut state, KeyCode::Home);
        assert_eq!(state.help_scroll, 0);
    }
}

#[test]
fn view_changes_clear_text_capture_and_selection_mode_but_redundant_focus_does_not() {
    let mut state = queue_state(View::Queue, true);
    state.list_filter.active = true;
    state.select_mode = true;
    state.queue.selected.insert(1);
    state.set_view(View::Queue);
    assert!(state.list_filter.active && state.select_mode);
    state.set_view(View::NowPlaying);
    assert!(!state.list_filter.active && !state.select_mode);
    assert!(state.queue.selected.contains(&1));
    state.set_view(View::Browse);
    assert!(state.queue.selected.is_empty());
}
