mod common;
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};
use textamp::app::{
    AppState,
    handlers::mouse_input,
    state::{BrowseColumn, BrowseItem, Focus},
};

fn fixture() -> AppState {
    let mut state = AppState::new();
    state.focus = Focus::Right;
    state.category_column_focused = false;
    let albums = BrowseColumn::new(
        "albums — An artist with a very long name that must not overlap the controls",
        vec![BrowseItem::Album {
            key: "album".into(),
            title: "Album".into(),
            artist: "Artist".into(),
            year: None,
            thumb: None,
            is_placeholder: false,
        }],
    );
    state.artist_nav.columns = vec![
        BrowseColumn::new(
            "artists",
            vec![BrowseItem::Artist {
                key: "artist".into(),
                title: "Artist".into(),
                thumb: None,
                is_placeholder: false,
            }],
        ),
        albums,
    ];
    state.artist_nav.focused_column = 1;
    state
}
#[test]
fn view_options_header_targets_clicked_column_without_sorting_or_playing() {
    let mut state = fixture();

    state.terminal_width = 120;
    state.terminal_height = 40;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|frame| state.apply_render_feedback(textamp::ui::render(frame, &state)))
        .unwrap();
    let regions = state.hit_regions.miller_columns.clone().unwrap();
    for region in &regions.columns {
        let button = region
            .view_options
            .expect("artist and album options available");
        assert_eq!(
            terminal.backend().buffer()[(button.x, button.y)].symbol(),
            "≡"
        );
        assert!(region.close_x.is_none_or(|r| r.x > button.x));
        let actions = mouse_input::handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: button.x,
                row: button.y,
                modifiers: KeyModifiers::NONE,
            },
            &mut state,
        );
        assert!(actions.is_empty());
        assert!(state.palette.open);
        assert_eq!(state.palette.path, ["View", "View Options"]);
        assert_eq!(state.artist_nav.focused_column, region.col_idx);
        assert_eq!(state.artist_nav.columns.len(), 2);
        assert!(state.queue.tracks.is_empty());
        let grid = textamp::app::state::PaletteCommandKind::SetAlbumLayout(true);
        assert!(state.palette.matches.iter().any(|&i| matches!(
            state.palette.entries[i].command,
            textamp::app::state::PaletteCommandKind::SetAlbumLayout(true)
        )));
        assert_eq!(
            textamp::app::command_catalog::disabled_reason(&state, &grid).is_none(),
            region.col_idx == 1
        );
        assert!(state.popups.sort().is_none());
        state.palette.open = false;
    }
}

#[test]
fn view_options_shortcuts_open_the_album_layout_choices() {
    use crossterm::event::{KeyCode, KeyEvent};
    use textamp::app::{command_palette, handlers::key_input, state::PaletteCommandKind};
    let mut state = fixture();
    for (code, modifiers) in [
        (KeyCode::F(6), KeyModifiers::NONE),
        (KeyCode::Char('v'), KeyModifiers::CONTROL),
    ] {
        key_input::handle_key(
            KeyEvent::new(code, modifiers),
            &mut state,
            &Default::default(),
        );
        assert_eq!(state.palette.path, ["View", "View Options"]);
        let choices = &state.palette.matches;
        assert!(choices.iter().any(|&i| matches!(
            state.palette.entries[i].command,
            PaletteCommandKind::SetAlbumLayout(true)
        ) && state.palette.entries[i].hint == "✓"));
        state.palette.selected = choices
            .iter()
            .position(|&i| {
                matches!(
                    state.palette.entries[i].command,
                    PaletteCommandKind::SetAlbumLayout(false)
                )
            })
            .unwrap();
        assert!(matches!(
            command_palette::handle_key(
                &mut state,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
            ),
            command_palette::PaletteOutcome::Execute(PaletteCommandKind::SetAlbumLayout(false))
        ));
        state.palette.open = false;
    }
}

#[test]
fn options_eligibility_matches_tracks_and_non_sortable_columns() {
    use textamp::app::state::SortColumnType;
    let mut state = fixture();
    state.artist_nav.columns = vec![BrowseColumn::new(
        "genres",
        vec![BrowseItem::Genre {
            key: "g".into(),
            title: "Genre".into(),
        }],
    )];
    assert!(state.column_options_type(0).is_none());
    state.artist_nav.columns[0] = BrowseColumn::new("empty", vec![]);
    assert!(state.column_options_type(0).is_none());
    state.artist_nav.columns[0] = BrowseColumn::new_with_tracks(
        "tracks",
        vec![BrowseItem::Track {
            download_id: None,
            key: "track".into(),
            title: "Track".into(),
            artist_name: Some("Artist".into()),
            album_name: Some("Album".into()),
            year: None,
            duration_ms: 60_000,
            track_number: Some(1),
        }],
        vec![textamp::library::models::Track {
            rating_key: "track".into(),
            title: "Track".into(),
            ..Default::default()
        }],
    );
    assert!(matches!(
        state.column_options_type(0),
        Some(SortColumnType::Track | SortColumnType::AllTracks)
    ));
}

#[test]
fn album_track_header_omits_sort_control() {
    let mut state = fixture();
    let tracks: Vec<_> = ["First", "Second"]
        .into_iter()
        .map(|title| textamp::library::models::Track {
            rating_key: title.into(),
            title: title.into(),
            ..Default::default()
        })
        .collect();
    let mut album =
        BrowseColumn::new_with_tracks("Album", BrowseItem::from_tracks(&tracks), tracks);
    album.play_all_row = Some(textamp::app::state::PlayAllRow::Album {
        rating_key: "album".into(),
        title: "Album".into(),
    });
    state.artist_nav.columns = vec![album];
    state.artist_nav.focused_column = 0;
    state.terminal_width = 100;
    state.terminal_height = 30;
    Terminal::new(TestBackend::new(100, 30))
        .unwrap()
        .draw(|frame| state.apply_render_feedback(textamp::ui::render(frame, &state)))
        .unwrap();
    let regions = state.hit_regions.miller_columns.unwrap();
    assert_eq!(regions.columns.len(), 1);
    assert!(regions.columns[0].view_options.is_none());
}

#[test]
fn artist_mix_header_targets_the_artist_shown_in_that_column() {
    use textamp::app::{
        Action, action::RadioAction, column_controls::ColumnControl, state::ArtistInfoDetails,
    };
    let mut state = fixture();
    state.sources.active = common::navidrome_state().sources.active;
    state.artist_nav.columns[1].artist_header = Some(ArtistInfoDetails {
        artist_key: "artist".into(),
        artist_name: "Artist".into(),
        request_id: 1,
        document: Default::default(),
        task: None,
        image_index: 0,
    });
    state.terminal_width = 120;
    state.terminal_height = 40;
    Terminal::new(TestBackend::new(120, 40))
        .unwrap()
        .draw(|frame| state.apply_render_feedback(textamp::ui::render(frame, &state)))
        .unwrap();
    let column = &state.hit_regions.miller_columns.as_ref().unwrap().columns[1];
    let (button, _) = column
        .controls
        .iter()
        .find(|(_, control)| *control == ColumnControl::ArtistMix)
        .unwrap();
    assert_eq!(button.y, column.area.y);
    let actions = mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: button.x,
            row: button.y,
            modifiers: KeyModifiers::NONE,
        },
        &mut state,
    );
    assert!(
        matches!(actions.as_slice(), [Action::Radio(RadioAction::StartArtistRadio { key, title })] if key == "artist" && title == "Artist")
    );
}
