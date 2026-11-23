use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};
use textamp::app::{
    Action, AppState,
    handlers::mouse_input,
    state::{BrowseColumn, BrowseItem, Focus},
};
use textamp::config::Config;

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
#[tokio::test]
async fn view_options_header_targets_clicked_column_without_sorting_or_playing() {
    use textamp::app::{action::SearchAction, state::SortColumnType};
    for _ in [()] {
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
            assert!(matches!(
                actions.as_slice(),
                [Action::Search(SearchAction::OpenSortPopup)]
            ));
            assert_eq!(state.artist_nav.focused_column, region.col_idx);
            assert_eq!(state.artist_nav.columns.len(), 2);
            assert!(state.queue.tracks.is_empty());
            let mut config = Config::default();
            let mut audio = textamp::audio::AudioPlayer::new_without_audio();
            let (tx, _rx) = tokio::sync::mpsc::channel(8);
            for action in actions {
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
            let popup = state.popups.sort().unwrap();
            assert_eq!(popup.column_idx, region.col_idx);
            assert_eq!(
                popup.column_type,
                if region.col_idx == 0 {
                    SortColumnType::Artist
                } else {
                    SortColumnType::Album
                }
            );
            state.popups.set_sort(None);
        }
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
