use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::TestBackend, Terminal};
use textamp::app::{
    action::{QueueAction, SearchAction},
    handlers::{key_input, mouse_input},
    state::{SearchFocus, SearchTab, SimilarMode, View},
    Action, AppState,
};
use textamp::{
    config::Config,
    library::models::{SearchResults, Track},
};

fn tracks() -> Vec<Track> {
    serde_json::from_value(serde_json::json!([
        {"ratingKey":"first", "title":"First", "grandparentTitle":"First Artist", "parentTitle":"First Album"},
        {"ratingKey":"second", "title":"Second", "grandparentTitle":"Second Artist", "parentTitle":"Second Album"},
        {"ratingKey":"third", "title":"Third", "grandparentTitle":"Third Artist", "parentTitle":"Third Album"}
    ])).unwrap()
}
fn key(state: &mut AppState, code: KeyCode) -> Vec<Action> {
    key_input::handle_key(
        KeyEvent::new(code, KeyModifiers::NONE),
        state,
        &Config::default(),
    )
}
fn draw(state: &mut AppState, label: &str, width: u16, height: u16) {
    state.terminal_width = width;
    state.terminal_height = height;
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| state.apply_render_feedback(textamp::ui::render(frame, state)))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("Play Track++"));
    assert!(text.contains("Play Track"));
    if width >= 100 {
        assert!(text.contains("Second Artist"));
    }
    if let Ok(dir) = std::env::var("TEXTAMP_SETTINGS_PREVIEW_DIR") {
        let cells:Vec<_>=terminal.backend().buffer().content.iter().map(|c|serde_json::json!({"s":c.symbol(),"fg":format!("{:?}",c.fg),"bg":format!("{:?}",c.bg)})).collect();
        std::fs::write(
            std::path::Path::new(&dir).join(format!("{label}-{width}x{height}.json")),
            serde_json::to_vec(&serde_json::json!({"width":width,"height":height,"cells":cells}))
                .unwrap(),
        )
        .unwrap();
    }
}
fn assert_single(actions: &[Action]) {
    assert!(
        matches!(actions,[Action::Queue(QueueAction::PlayTrack(track))] if track.rating_key=="second")
    );
}
fn assert_following(actions: &[Action]) {
    assert!(
        matches!(actions,[Action::Queue(QueueAction::PlayTracksNow(tracks))] if tracks.iter().map(|t|t.rating_key.as_str()).collect::<Vec<_>>()==["second","third"])
    );
}
fn verify_actions(state: &mut AppState) {
    assert_single(&key(state, KeyCode::Enter));
    key(state, KeyCode::Down);
    assert_following(&key(state, KeyCode::Enter));
    let pane = state.hit_regions.track_pane.clone().unwrap();
    for (rect, following) in [(pane.play_button, false), (pane.following_button, true)] {
        let actions = mouse_input::handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: rect.x,
                row: rect.y,
                modifiers: KeyModifiers::NONE,
            },
            state,
        );
        if following {
            assert_following(&actions);
        } else {
            assert_single(&actions);
        }
    }
    assert!(
        state.queue.tracks.is_empty(),
        "Selecting details must not replace the queue"
    );
}
#[tokio::test]
async fn search_enter_keeps_results_and_opens_explicit_playback_actions_for_missing_metadata_too() {
    for tab in [SearchTab::Tracks, SearchTab::Global] {
        let mut state = AppState::new();
        state.view = View::Queue;
        state.popups.search_active = true;
        state.search.tab = tab;
        state.search.focus = SearchFocus::Results;
        state.search.query = "Search".into();
        state.search.results = Some(SearchResults {
            tracks: tracks(),
            ..Default::default()
        });
        state.list_state.search_item_index = 1;
        let actions = key(&mut state, KeyCode::Enter);
        assert!(matches!(
            actions.as_slice(),
            [Action::Search(SearchAction::SelectSearchResult)]
        ));
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let follow = textamp::app::handlers::dispatch_search::dispatch(
            &tx,
            SearchAction::SelectSearchResult,
            &mut state,
        )
        .await
        .unwrap();
        assert!(follow.is_empty());
        assert!(state.popups.search_active);
        assert_eq!(state.view, View::Queue);
        assert!(state.search.details.open && state.search.details.focused);
        for (w, h) in [(100, 32), (80, 24)] {
            draw(&mut state, "search-details", w, h);
        }
        verify_actions(&mut state);
        assert!(key(&mut state, KeyCode::Left).is_empty());
        assert!(!state.search.details.focused);
        key(&mut state, KeyCode::Down);
        assert_eq!(state.list_state.search_item_index, 2);
        key(&mut state, KeyCode::Right);
        assert!(state.search.details.focused);
        assert!(
            matches!(key(&mut state,KeyCode::Enter).as_slice(),[Action::Queue(QueueAction::PlayTrack(track))] if track.rating_key=="third")
        );
    }
}
#[test]
fn similar_results_open_details_in_place_and_preserve_rank_for_play_following() {
    let mut state = AppState::new();
    state.view = View::Queue;
    state.set_view(View::Similar);
    state.similar.mode = SimilarMode::Tracks;
    state.similar.tracks = tracks();
    state.list_state.similar_index = 1;
    assert!(key(&mut state, KeyCode::Enter).is_empty());
    assert!(state.similar.details.open && state.similar.details.focused);
    assert_eq!(state.previous_view, Some(View::Queue));
    for (w, h) in [(100, 32), (80, 24)] {
        draw(&mut state, "similar-details", w, h);
    }
    verify_actions(&mut state);
    assert!(key(&mut state, KeyCode::Esc).is_empty());
    assert!(!state.similar.details.focused);
    assert!(matches!(
        key(&mut state, KeyCode::Esc).as_slice(),
        [Action::Navigation(
            textamp::app::action::NavigationAction::SetView(View::Queue)
        )]
    ));
}
#[test]
fn queue_enter_still_plays_without_opening_result_details() {
    let mut state = AppState::new();
    state.view = View::Queue;
    state.queue.tracks = tracks();
    state.list_state.queue_index = 1;
    assert!(matches!(
        key(&mut state, KeyCode::Enter).as_slice(),
        [Action::Queue(QueueAction::JumpToQueueIndex(1))]
    ));
    assert!(!state.search.details.open && !state.similar.details.open);
}

fn browse_state() -> AppState {
    use textamp::app::sources::{navidrome::Session, ActiveSource};
    use textamp::app::state::{BrowseColumn, BrowseItem, MillerLayoutMode};
    let mut state = AppState::new();
    let source = textamp::navidrome::Source {
        id: "details-fixture".into(),
        name: "Fixture".into(),
        url: "http://example.invalid".into(),
        username: "fixture".into(),
        libraries: vec![],
    };
    state.sources.active = ActiveSource::Navidrome(Box::new(Session {
        client: textamp::navidrome::Client::new(&source, "unused".into(), None).unwrap(),
        source,
        extensions: Default::default(),
    }));
    state.view = View::Browse;
    state.category_column_focused = false;
    state.miller_layout = MillerLayoutMode::Scrolling;
    let tracks = tracks();
    state.artist_nav.columns = vec![BrowseColumn::new_with_tracks(
        "Tracks",
        BrowseItem::from_tracks(&tracks),
        tracks.clone(),
    )];
    state.track_pane_open = true;
    state.track_pane_focused = true;
    state.track_pane_index = 2;
    state
        .track_pane_similar
        .insert("first".into(), Ok(tracks[1..].to_vec()));
    state.track_pane_similar.insert(
        "second".into(),
        Ok(vec![tracks[2].clone(), tracks[0].clone()]),
    );
    state
        .track_pane_similar
        .insert("third".into(), Ok(vec![tracks[1].clone()]));
    state
}

#[test]
fn related_details_reuse_one_column_and_keep_parent_and_ranked_playback() {
    use textamp::app::sonic_details;
    let mut state = browse_state();
    assert!(key(&mut state, KeyCode::Enter).is_empty());
    assert!(!state.palette.open);
    assert_eq!(state.focused_track().unwrap().rating_key, "first");
    assert_eq!(
        sonic_details::visible(&state).unwrap().tracks[0].rating_key,
        "second"
    );
    for layout in [
        textamp::app::state::MillerLayoutMode::Scrolling,
        textamp::app::state::MillerLayoutMode::Shrinking,
    ] {
        state.miller_layout = layout;
        for (w, h) in [(140, 40), (100, 32), (80, 24)] {
            draw(&mut state, &format!("adjacent-details-{layout:?}"), w, h);
            let left = state.hit_regions.track_pane.as_ref().unwrap();
            let right = state.hit_regions.sonic_details.as_ref().unwrap();
            assert_eq!(left.outer.right(), right.outer.x);
            assert!(right.outer.right() <= w);
        }
    }
    state.miller_layout = textamp::app::state::MillerLayoutMode::Scrolling;
    assert_single(&key(&mut state, KeyCode::Enter));
    key(&mut state, KeyCode::Down);
    assert_following(&key(&mut state, KeyCode::Enter));
    for _ in 0..12 {
        key(&mut state, KeyCode::Down);
        assert!(key(&mut state, KeyCode::Enter).is_empty());
        assert!(!state.palette.open);
        assert_eq!(state.artist_nav.columns.len(), 1);
        assert_eq!(state.focused_track().unwrap().rating_key, "first");
        assert_eq!(sonic_details::visible(&state).unwrap().parent_key, "first");
        key(&mut state, KeyCode::Down);
    }
    key(&mut state, KeyCode::Left);
    assert!(!sonic_details::focused(&state));
    assert_eq!(state.track_pane_index, 2);
    key(&mut state, KeyCode::Right);
    assert!(sonic_details::focused(&state));
    assert!(state.queue.tracks.is_empty());
    key_input::close_focused_browse_column(&mut state);
    assert!(state.sonic_details.is_none());
    assert!(state.track_pane_open && state.track_pane_focused);
    key_input::close_focused_browse_column(&mut state);
    assert!(!state.track_pane_open);
}

#[test]
fn related_details_mouse_opens_replaces_and_closes_only_child() {
    let mut state = browse_state();
    key(&mut state, KeyCode::Enter);
    key(&mut state, KeyCode::Down);
    key(&mut state, KeyCode::Down);
    draw(&mut state, "adjacent-selected", 100, 40);
    let row = state
        .hit_regions
        .sonic_details
        .as_ref()
        .unwrap()
        .similar_rows[0]
        .0;
    let event = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: row.x,
        row: row.y,
        modifiers: KeyModifiers::NONE,
    };
    assert!(mouse_input::handle_mouse(event, &mut state).is_empty());
    assert_eq!(
        textamp::app::sonic_details::visible(&state).unwrap().tracks[0].rating_key,
        "third"
    );
    assert!(!state.palette.open);
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    terminal
        .draw(|f| state.apply_render_feedback(textamp::ui::render(f, &state)))
        .unwrap();
    let close = state
        .hit_regions
        .sonic_details
        .as_ref()
        .unwrap()
        .close_x
        .unwrap();
    mouse_input::handle_mouse(
        MouseEvent {
            column: close.x,
            row: close.y,
            ..event
        },
        &mut state,
    );
    assert!(state.sonic_details.is_none());
    assert!(state.track_pane_open && state.track_pane_focused);
}

#[test]
fn jump_commands_target_queue_search_related_and_playback_fallback() {
    use textamp::app::{action::BrowseAction, command_palette};
    let mut tracks = tracks();
    for t in &mut tracks {
        t.grandparent_rating_key = Some(format!("{}-artist", t.rating_key));
        t.parent_rating_key = Some(format!("{}-album", t.rating_key));
    }
    let mut state = browse_state();
    state.queue.tracks = tracks.clone();
    state.queue.index = Some(0);
    state.set_playback_mode(textamp::app::state::PlaybackMode::Queue);
    for (view, search, expected) in [
        (View::Queue, false, "second"),
        (View::Queue, true, "third"),
        (View::Similar, false, "second"),
        (View::Help, false, "first"),
    ] {
        state.view = view;
        state.popups.search_active = search;
        state.list_state.queue_index = 1;
        state.search.tab = SearchTab::Tracks;
        state.search.results = Some(SearchResults {
            tracks: tracks.clone(),
            ..Default::default()
        });
        state.list_state.search_item_index = 2;
        state.similar.tracks = tracks.clone();
        state.similar.mode = SimilarMode::Tracks;
        state.list_state.similar_index = 1;
        for (label, album) in [("Jump to artist", false), ("Jump to album", true)] {
            let cmd = command_palette::materialize_entries(&state)
                .into_iter()
                .find(|e| e.label == label)
                .unwrap()
                .command;
            let actions = command_palette::run(cmd, &mut state);
            assert!(
                matches!(actions.as_slice(),[Action::Browse(BrowseAction::OpenInLibrary{artist_key,album_key,..})] if artist_key==&format!("{expected}-artist") && album_key==&album.then(||format!("{expected}-album")))
            );
        }
    }
    state.view = View::Browse;
    state.popups.search_active = false;
    state
        .track_pane_similar
        .insert("first".into(), Ok(tracks[1..].to_vec()));
    key(&mut state, KeyCode::Enter);
    let cmd = command_palette::materialize_entries(&state)
        .into_iter()
        .find(|e| e.label == "Jump to album")
        .unwrap()
        .command;
    assert!(
        matches!(command_palette::run(cmd,&mut state).as_slice(),[Action::Browse(BrowseAction::OpenInLibrary{album_key:Some(key),..})] if key=="second-album")
    );
    assert_eq!(state.queue.index, Some(0));
}
