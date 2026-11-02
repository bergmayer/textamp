use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::TestBackend, Terminal};
use textamp::app::{
    action::MillerAction,
    dispatch::dispatch_action,
    handlers::{key_input, mouse_input},
    state::{BrowseCategory, BrowseColumn, BrowseItem, ColumnHeaderSelection, PlayAllRow, View},
    Action, AppState,
};
use textamp::{audio::AudioPlayer, config::Config, library::models::Track};

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
    nav.columns = vec![col];
    nav.focused_column = 0;
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
        assert!(key(&mut state, KeyCode::Down).is_empty());
        assert_eq!(
            state
                .browse_nav()
                .unwrap()
                .focused()
                .unwrap()
                .header_selection,
            ColumnHeaderSelection::AlbumDetails
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
        assert!(key(&mut state, KeyCode::Enter).is_empty());
        key(&mut state, KeyCode::Left);
        assert_eq!(state.browse_nav().unwrap().focused_column, 0);
        assert!(draw(&mut state).contains("Duration: 2:03"));
        let actions = key(&mut state, KeyCode::Right);
        for action in actions {
            dispatch_action(action, &mut state, &mut audio, &mut config, &tx)
                .await
                .unwrap();
        }
        assert_eq!(state.browse_nav().unwrap().columns.len(), 2);
        key(&mut state, KeyCode::Left);
        key(&mut state, KeyCode::Down);
        assert_eq!(
            state
                .browse_nav()
                .unwrap()
                .focused()
                .unwrap()
                .header_selection,
            ColumnHeaderSelection::Items
        );
        key(&mut state, KeyCode::Up);
        assert_eq!(
            state
                .browse_nav()
                .unwrap()
                .focused()
                .unwrap()
                .header_selection,
            ColumnHeaderSelection::AlbumDetails
        );
        assert!(state.queue.tracks.is_empty());
    }
}
#[test]
fn mouse_album_details_and_first_track_have_distinct_hit_regions() {
    let mut state = state(BrowseCategory::Library);
    assert!(draw(&mut state).contains("Album Details"));
    let region = state.hit_regions.miller_columns.as_ref().unwrap().columns[0].clone();
    let click = |row| MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: region.inner.x + 3,
        row,
        modifiers: KeyModifiers::NONE,
    };
    let actions = mouse_input::handle_mouse(click(region.inner.y + 1), &mut state);
    assert!(matches!(
        actions.as_slice(),
        [Action::Miller(MillerAction::OpenAlbumDetails)]
    ));
    let actions = mouse_input::handle_mouse(click(region.inner.y + 2), &mut state);
    assert!(!actions.iter().any(|a| matches!(a, Action::Queue(_))));
    let col = state.browse_nav().unwrap().focused().unwrap();
    assert_eq!(col.selected_index, 0);
    assert_eq!(col.header_selection, ColumnHeaderSelection::Items);
}

#[test]
fn wrapped_album_metadata_scrolls_to_the_end() {
    let mut state = state(BrowseCategory::Library);
    let mut page = BrowseColumn::new("Album Details", vec![]);
    page.album_details = Some(vec![format!("{} LastGenre", "LongGenre ".repeat(300))]);
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
    .await
    .unwrap();
    assert!(
        matches!(actions.as_slice(), [Action::System(SystemAction::LoadAlbumArt(items))] if items == &vec![("album".into(), "fixture-cover".into())])
    );
    assert!(matches!(
        state.artwork.navigation,
        textamp::app::state::ArtworkNavigation::Settled
    ));
    assert_eq!(
        state
            .artist_nav
            .focused()
            .unwrap()
            .album_details_art_key
            .as_deref(),
        Some("album")
    );
    assert!(draw(&mut state).contains("(no cover)"));
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
    assert!(!screen.contains("(no cover)"));
    assert!(screen.contains("Artist: Artist"));
    key(&mut state, KeyCode::Left);
    let actions = textamp::app::handlers::dispatch_miller::dispatch(
        &tx,
        MillerAction::OpenAlbumDetails,
        &mut state,
        &mut audio,
    )
    .await
    .unwrap();
    assert!(
        actions.is_empty(),
        "cached artwork should not be fetched again"
    );
    assert!(state.queue.tracks.is_empty());
}
