use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};
use textamp::app::{
    Action, AppState,
    action::MillerAction,
    dispatch::dispatch_action,
    handlers::{key_input, mouse_input},
    state::{BrowseCategory, BrowseColumn, BrowseItem, ColumnHeaderSelection, PlayAllRow, View},
};
use textamp::{audio::AudioPlayer, config::Config, library::models::Track};

mod common;

#[tokio::test]
async fn album_details_requests_similar_music_once_for_its_own_seed() {
    use textamp::app::{Event, action::DataAction, state::BrowseDetails};

    let mut state = common::navidrome_state();
    state.view = View::Browse;
    state.browse_category = BrowseCategory::Library;
    let track = Track {
        rating_key: "album-seed".into(),
        parent_rating_key: Some("album".into()),
        duration: Some(61_000),
        ..Default::default()
    };
    let mut tracks = BrowseColumn::new_with_tracks("Album", vec![], vec![track]);
    tracks.play_all_row = Some(PlayAllRow::Album {
        rating_key: "album".into(),
        title: "Album".into(),
    });
    let mut details = BrowseColumn::new("Album Details", vec![]);
    details.play_all_row = Some(PlayAllRow::Album {
        rating_key: "album".into(),
        title: "Album".into(),
    });
    details.details = Some(BrowseDetails::Album {
        artwork_key: "album".into(),
        lines: vec![],
    });
    state.artist_nav.columns = vec![tracks, details];
    state.artist_nav.focused_column = 1;
    let details = textamp::app::details::project(&state).unwrap();
    assert!(details.rows.contains(&("Tracks".into(), "1".into())));
    assert!(details.rows.contains(&("Duration".into(), "1:01".into())));
    let (tx, _) = tokio::sync::mpsc::channel(8);
    let tick = |state: &mut AppState| {
        textamp::app::handlers::events::handle_app_event(Event::Tick, state, &tx)
    };
    assert!(matches!(tick(&mut state).as_slice(),
        [Action::Data(DataAction::LoadTrackPaneSimilar { rating_key })] if rating_key == "album-seed"));
    state.track_pane_similar_loading.insert("album-seed".into());
    assert!(tick(&mut state).is_empty());
    state.track_pane_similar_loading.clear();
    state
        .track_pane_similar
        .insert("album-seed".into(), Ok(vec![]));
    assert!(tick(&mut state).is_empty());
}

fn state(category: BrowseCategory) -> AppState {
    let mut state = AppState::new();
    state.view = View::Browse;
    state.browse_category = category;
    state.category_column_focused = false;
    let tracks: Vec<Track> = serde_json::from_value(serde_json::json!([
        {"ratingKey":"one", "title":"First", "parentTitle":"Album", "grandparentTitle":"Artist", "parentYear":2024, "duration":61000,"genres":["Rock","Pop"]},
        {"ratingKey":"two", "title":"Second", "parentTitle":"Album", "grandparentTitle":"Artist", "parentYear":2024,"duration":62000,"genres":["Pop"]}
    ])).unwrap();
    let mut col = BrowseColumn::new_with_tracks("Album", BrowseItem::from_tracks(&tracks), tracks);
    col.play_all_row = Some(PlayAllRow::Album {
        rating_key: "album".into(),
        title: "Album".into(),
    });
    col.header_selection = ColumnHeaderSelection::Play;
    let nav = state.browse_nav_mut().unwrap();
    if category == BrowseCategory::Playlists {
        nav.columns = vec![BrowseColumn::new("Playlists", vec![]), col];
        nav.focused_column = 1;
    } else {
        nav.columns = vec![col];
        nav.focused_column = 0;
    }
    state
}
fn key(state: &mut AppState, code: KeyCode) -> Vec<Action> {
    key_input::handle_key(
        KeyEvent::new(code, KeyModifiers::NONE),
        state,
        &Config::default(),
    )
}
fn draw(state: &mut AppState) -> String {
    state.terminal_width = 120;
    state.terminal_height = 40;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|frame| state.apply_render_feedback(textamp::ui::render(frame, state)))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}
#[tokio::test]
async fn album_details_keyboard_opens_metadata_and_returns_without_playing() {
    for category in [
        BrowseCategory::Library,
        BrowseCategory::AlbumGenres,
        BrowseCategory::Playlists,
    ] {
        let mut state = state(category);
        key(&mut state, KeyCode::Up);
        assert_eq!(
            state.tui_column_header.unwrap().control,
            textamp::app::column_controls::ColumnControl::Details
        );
        let actions = key(&mut state, KeyCode::Enter);
        assert!(matches!(
            actions.as_slice(),
            [Action::Miller(MillerAction::OpenAlbumDetails)]
        ));
        let mut audio = AudioPlayer::new_without_audio();
        let mut config = Config::default();
        let (tx, _) = tokio::sync::mpsc::channel(8);
        for action in actions {
            dispatch_action(action, &mut state, &mut audio, &mut config, &tx)
                .await
                .unwrap();
        }
        let screen = draw(&mut state);
        for text in [
            "Album Details",
            "Artist: Artist",
            "Year: 2024",
            "Genres: Rock, Pop",
            "Tracks: 2",
            "Duration: 2:03",
        ] {
            assert!(screen.contains(text), "missing {text}");
        }
        assert!(matches!(key(&mut state, KeyCode::Enter).as_slice(),
            [Action::Queue(textamp::app::action::QueueAction::PlayAlbumNow { rating_key, title })]
                if rating_key == "album" && title == "Album"));
        assert!(
            !state
                .browse_nav()
                .unwrap()
                .focused()
                .unwrap()
                .has_album_details()
        );
        let regions = state.hit_regions.miller_columns.as_ref().unwrap();
        let region = regions
            .columns
            .iter()
            .find(|r| {
                r.col_idx
                    == if category == BrowseCategory::Playlists {
                        2
                    } else {
                        1
                    }
            })
            .unwrap();
        let play = region
            .controls
            .iter()
            .find(|(_, control)| *control == textamp::app::column_controls::ColumnControl::Play)
            .unwrap()
            .0;
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: play.x,
            row: play.y,
            modifiers: KeyModifiers::NONE,
        };
        assert!(
            matches!(mouse_input::handle_mouse(click, &mut state).as_slice(), [Action::Queue(textamp::app::action::QueueAction::PlayAlbumNow { rating_key,.. })] if rating_key == "album")
        );
        key_input::close_focused_browse_column(&mut state);
        assert_eq!(
            state.browse_nav().unwrap().focused_column,
            if category == BrowseCategory::Playlists {
                1
            } else {
                0
            }
        );
        assert!(state.queue.tracks.is_empty());
    }
}
#[test]
fn mouse_album_details_and_first_track_have_distinct_hit_regions() {
    let mut state = state(BrowseCategory::Library);
    draw(&mut state);
    let region = state.hit_regions.miller_columns.as_ref().unwrap().columns[0].clone();
    let details = region
        .controls
        .iter()
        .find(|(_, control)| *control == textamp::app::column_controls::ColumnControl::Details)
        .unwrap()
        .0;
    let event = |x, y| MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    };
    assert!(matches!(
        mouse_input::handle_mouse(event(details.x, details.y), &mut state).as_slice(),
        [Action::Miller(MillerAction::OpenAlbumDetails)]
    ));
    let actions = mouse_input::handle_mouse(event(region.inner.x + 3, region.inner.y), &mut state);
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, Action::Queue(_)))
    );
    assert_eq!(
        state
            .browse_nav()
            .unwrap()
            .focused()
            .unwrap()
            .selected_index,
        0
    );
}

#[test]
fn wrapped_album_metadata_scrolls_to_the_end() {
    let mut state = state(BrowseCategory::Library);
    let mut page = BrowseColumn::new("Album Details", vec![]);
    page.details = Some(textamp::app::state::BrowseDetails::Album {
        lines: vec![format!("{} LastGenre", "LongGenre ".repeat(300))],
        artwork_key: String::new(),
    });
    state.artist_nav.push_column(page);
    draw(&mut state);
    key(&mut state, KeyCode::End);
    assert!(draw(&mut state).contains("LastGenre"));
    key(&mut state, KeyCode::Home);
    assert_eq!(state.artist_nav.focused().unwrap().selected_index, 0);
}

#[tokio::test]
async fn album_details_fetches_missing_cover_and_renders_cached_artwork() {
    use textamp::app::action::SystemAction;
    let mut state = state(BrowseCategory::Library);
    state.artist_nav.columns[0].tracks[0].parent_thumb = Some("fixture-cover".into());
    state.artwork.navigation = textamp::app::state::ArtworkNavigation::Moving {
        last_motion: std::time::Instant::now(),
    };
    let mut audio = AudioPlayer::new_without_audio();
    let (tx, _) = tokio::sync::mpsc::channel(8);
    let actions = textamp::app::handlers::dispatch_miller::dispatch(
        &tx,
        MillerAction::OpenAlbumDetails,
        &mut state,
        &mut audio,
    )
    .unwrap();
    assert!(
        matches!(actions.as_slice(), [Action::System(SystemAction::LoadAlbumArt(items))] if items == &vec![("album".into(), "fixture-cover".into())])
    );
    assert!(matches!(
        state.artwork.navigation,
        textamp::app::state::ArtworkNavigation::Settled
    ));
    assert_eq!(
        state.artist_nav.focused().unwrap().detail_art_key(),
        Some("album")
    );
    let placeholder = draw(&mut state);
    assert!(placeholder.contains("Artist: Artist"));
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        32,
        32,
        image::Rgb([220, 40, 30]),
    ))
    .write_to(&mut png, image::ImageFormat::Png)
    .unwrap();
    state
        .artwork
        .insert_grid_art("album".into(), png.into_inner());
    let picker = ratatui_image::picker::Picker::halfblocks();
    textamp::ui::artwork::init_grid_renderer(picker);
    let screen = draw(&mut state);
    assert_ne!(
        screen, placeholder,
        "the cached cover must replace its placeholder"
    );
    assert!(screen.contains("Artist: Artist"));
    key(&mut state, KeyCode::Left);
    let actions = textamp::app::handlers::dispatch_miller::dispatch(
        &tx,
        MillerAction::OpenAlbumDetails,
        &mut state,
        &mut audio,
    )
    .unwrap();
    assert!(
        actions.is_empty(),
        "cached artwork should not be fetched again"
    );
    assert!(state.queue.tracks.is_empty());
}
