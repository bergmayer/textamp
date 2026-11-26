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
fn queue_state(view: View) -> AppState {
    let mut state = AppState::new();
    state.view = view;
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
fn opening_now_playing_preserves_the_library_sidebar_width() {
    for expanded in [true, false] {
        let mut state = queue_state(View::Browse);
        state.tui_sidebar_visible = expanded;
        render(&mut state);
        let sidebar = state
            .hit_regions
            .category_column
            .as_ref()
            .map(|column| column.area);
        state.set_view(View::NowPlaying);
        render(&mut state);
        assert_eq!(
            state
                .hit_regions
                .category_column
                .as_ref()
                .map(|column| column.area),
            sidebar
        );
    }
}

#[test]
fn now_playing_back_steps_up_without_dismissing_the_sheet() {
    use textamp::app::state::{BrowseColumn, BrowseItem};
    let mut state = queue_state(View::Browse);
    let item = BrowseItem::Artist {
        key: "a".into(),
        title: "Artist".into(),
        thumb: None,
        is_placeholder: false,
    };
    state.artist_nav.reset("Artists", vec![item.clone()]);
    state
        .artist_nav
        .push_column(BrowseColumn::new("Albums", vec![item]));
    state.category_column_focused = false;
    state.set_view(View::NowPlaying);
    render(&mut state);
    assert_eq!(
        state.hit_regions.miller_columns.as_ref().unwrap().columns[0].col_idx,
        1
    );
    let back = state.hit_regions.sidebar_back.unwrap();
    assert!(matches!(
        mouse(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            back.x + 1,
            back.y
        )
        .as_slice(),
        [Action::Navigation(
            textamp::app::action::NavigationAction::Back
        )]
    ));
    textamp::app::tui_navigation::sheet_back(&mut state);
    render(&mut state);
    assert!(state.tui_player_raised());
    assert_eq!(
        state.hit_regions.miller_columns.as_ref().unwrap().columns[0].col_idx,
        0
    );
    textamp::app::tui_navigation::sheet_back(&mut state);
    render(&mut state);
    assert!(
        state.hit_regions.category_column.is_some()
            && state.hit_regions.now_playing_content.is_some()
    );
}

#[test]
fn closing_the_single_library_column_shows_the_root_beside_the_player() {
    use textamp::app::state::{BrowseColumn, BrowseItem};
    let mut state = queue_state(View::NowPlaying);
    state.category_column_focused = false;
    let artist = BrowseItem::Artist {
        key: "a".into(),
        title: "Artist".into(),
        thumb: None,
        is_placeholder: false,
    };
    state.artist_nav.reset("Artists", vec![artist.clone()]);
    state
        .artist_nav
        .push_column(BrowseColumn::new("Albums", vec![artist]));
    render(&mut state);
    let close = state.hit_regions.miller_columns.as_ref().unwrap().columns[0]
        .close_x
        .unwrap();
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        close.x,
        close.y,
    );
    render(&mut state);
    assert!(state.hit_regions.category_column.is_some());
    assert!(state.hit_regions.now_playing_content.is_some());
    assert!(state.hit_regions.miller_columns.is_none());
}

#[test]
fn unavailable_transport_and_queue_buttons_are_dim_and_inert() {
    use textamp::app::transport_controls::TransportControl;
    let mut state = AppState::new();
    state.tui_queue_visible = true;
    let terminal = render(&mut state);
    let controls = state
        .hit_regions
        .transport
        .as_ref()
        .unwrap()
        .controls
        .clone();
    for control in [
        TransportControl::Lyrics,
        TransportControl::Menu,
        TransportControl::Clear,
    ] {
        let rect = if control == TransportControl::Clear {
            state.hit_regions.queue_clear.unwrap()
        } else {
            controls.iter().find(|(_, c)| *c == control).unwrap().0
        };
        assert!(
            terminal.backend().buffer()[(rect.x, rect.y)]
                .modifier
                .contains(ratatui::style::Modifier::DIM)
        );
        assert!(
            mouse(
                &mut state,
                MouseEventKind::Down(MouseButton::Left),
                rect.x,
                rect.y
            )
            .is_empty()
        );
        assert!(!state.palette.open);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn collapsed_sidebar_opens_library_destinations_from_now_playing() {
    use textamp::app::action::NavigationAction;
    use textamp::app::state::{CategoryRow, LibraryRoot};
    let mut state = queue_state(View::NowPlaying);
    state.tui_sidebar_visible = false;
    render(&mut state);
    let rail = state.hit_regions.category_column.as_ref().unwrap();
    let index = state
        .category_rows()
        .iter()
        .position(|row| matches!(row, CategoryRow::Root(LibraryRoot::Artists)))
        .unwrap();
    let x = rail.inner.x;
    let y = rail.inner.y + (index - rail.scroll_offset) as u16;
    let visualizer = state.hit_regions.now_playing_content.as_ref().unwrap();
    assert!(rail.area.right() <= visualizer.visualizer_content_area.x);
    let actions = mouse(&mut state, MouseEventKind::Down(MouseButton::Left), x, y);
    assert!(matches!(
        actions.as_slice(),
        [Action::Navigation(NavigationAction::OpenLibraryRoot(
            LibraryRoot::Artists
        ))]
    ));
    assert_eq!(state.view, View::Browse);
}

#[test]
fn clicking_queue_in_now_playing_routes_following_arrow_keys_to_tracks() {
    let mut state = queue_state(View::NowPlaying);
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

#[test]
fn clicking_artwork_never_selects_or_plays_a_queue_row() {
    let mut state = queue_state(View::Queue);
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
    let mut state = queue_state(View::Queue);
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
        let mut state = queue_state(view);
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

#[tokio::test]
async fn search_owns_punctuation_and_function_keys_instead_of_global_commands() {
    let mut state = queue_state(View::Browse);
    state.popups.set_search_active(true);
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    for c in [':', '/', '|', '\\', ',', '?', '<', '>'] {
        let actions = key(&mut state, KeyCode::Char(c));
        let [Action::Search(action @ SearchAction::SetSearchQuery(_))] = actions.as_slice() else {
            panic!("{c}: {actions:?}");
        };
        textamp::app::handlers::dispatch_search::dispatch(&tx, action.clone(), &mut state).unwrap();
        assert!(state.search.query.ends_with(c));
        assert!(!state.palette.open);
    }
    assert!(key(&mut state, KeyCode::F(3)).is_empty());
}

#[test]
fn text_dialog_blocks_clicks_drags_wheels_and_right_clicks_on_underlying_player() {
    let mut state = queue_state(View::Queue);
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
    let mut state = queue_state(View::Browse);
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
    for grouped in [false, true] {
        for category in [BrowseCategory::Library, BrowseCategory::AlbumGenres] {
            for row in 0..4 {
                let mut state = AppState::new();
                state.view = View::Browse;
                state.browse_category = category;
                state.album_grid = false;
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
                let region = state.hit_regions.miller_columns.as_ref().unwrap().columns[0].clone();
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
                    "grouped={grouped}, category={category:?}, row={row}"
                );
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
            1 => {
                textamp::app::command_palette::open(&mut state);
            }
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
fn help_end_reaches_wrapped_bottom_and_scrolling_is_bounded() {
    let mut state = AppState::new();
    state.view = View::Help;
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

#[test]
fn view_changes_clear_text_capture_and_selection_mode_but_redundant_focus_does_not() {
    let mut state = queue_state(View::Queue);
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
