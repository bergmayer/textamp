use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};
use textamp::app::action::SearchAction;
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
    state.desktop_split_view = tall;
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
fn terminal_tab_traverses_panels_without_changing_the_workspace() {
    for category in BrowseCategory::all() {
        let mut state = AppState::new();
        state.view = View::Browse;
        state.browse_category = *category;
        render(&mut state);
        for code in [KeyCode::Tab, KeyCode::BackTab] {
            assert!(key(&mut state, code).is_empty());
            let workspace = if state.view == View::Queue {
                state.tui_queue_workspace.view()
            } else {
                state.view
            };
            assert_eq!(workspace, View::Browse);
        }
    }
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
    state.tui_queue_workspace = textamp::app::state::TuiWorkspace::NowPlaying;
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
    state.tui_queue_workspace = textamp::app::state::TuiWorkspace::NowPlaying;
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
    state.popups.set_search_active(true);
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
    state.popups.set_input_dialog(Some(InputDialog {
        title: "Rename".into(),
        input: "name".into(),
        action_type: InputDialogAction::FolderName("folder".into()),
    }));
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
fn browse_album_row_boundaries_match_mouse_targets() {
    use textamp::app::state::{BrowseColumn, BrowseItem};
    for tall in [false, true] {
        for grouped in [false, true] {
            for category in [BrowseCategory::Library, BrowseCategory::AlbumGenres] {
                for row in 0..4 {
                    let mut state = AppState::new();
                    state.view = View::Browse;
                    state.desktop_split_view = tall;
                    state.browse_category = category;
                    let mut column = BrowseColumn::new(
                        "Albums",
                        (0..8)
                            .map(|i| BrowseItem::Album {
                                key: i.to_string(),
                                title: format!("Album {i}"),
                                artist: "Artist".into(),
                                year: None,
                                thumb: None,
                                is_placeholder: false,
                            })
                            .collect(),
                    );
                    column.grouped_by_album = grouped;
                    state.browse_nav_mut().unwrap().columns = vec![column];
                    render(&mut state);
                    let region =
                        state.hit_regions.miller_columns.as_ref().unwrap().columns[0].clone();
                    let height = if grouped || category == BrowseCategory::AlbumGenres {
                        2
                    } else {
                        1
                    };
                    assert_eq!(region.rows_per_item, height);
                    mouse(
                        &mut state,
                        MouseEventKind::Down(MouseButton::Left),
                        region.inner.x,
                        region.inner.y + row,
                    );
                    assert_eq!(
                        state.browse_nav().unwrap().columns[0].selected_index,
                        (row / height) as usize,
                        "tall={tall}, grouped={grouped}, category={category:?}, row={row}"
                    );
                }
            }
        }
    }
}

#[test]
fn library_switcher_remains_usable_from_authentication_screen() {
    let mut state = AppState::new();
    assert!(matches!(
        key(&mut state, KeyCode::F(3)).as_slice(),
        [Action::Search(SearchAction::OpenLibraryPicker)]
    ));
    state.popups.set_library_picker_active(true);
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
    let (tx, _) = tokio::sync::mpsc::channel(8);
    textamp::app::handlers::dispatch_search::dispatch(
        &tx,
        SearchAction::ActivateListFilter,
        &mut state,
    )
    .unwrap();
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
            0 => state.popups.set_search_active(true),
            1 => textamp::app::command_palette::open(&mut state),
            2 => state.popups.set_library_dialog(Some(
                textamp::app::sources::dialogs::Dialog::Webdav(
                    textamp::app::sources::dialogs::WebdavForm::new(None),
                ),
            )),
            3 => state.popups.set_input_dialog(Some(InputDialog {
                title: "Input".into(),
                input: "".into(),
                action_type: InputDialogAction::SavePlaylist,
            })),
            4 => state.notifications.last_error = Some("fixture error".into()),
            _ => state.popups.set_library_picker_active(true),
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
        state.desktop_split_view = tall;
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
