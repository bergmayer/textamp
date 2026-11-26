use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};
use textamp::app::{
    Action, AppState,
    action::{BrowseAction, QueueAction, SearchAction},
    handlers::{key_input, mouse_input},
    state::{SearchFocus, SearchTab, SimilarMode, View},
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
    assert!(text.contains("Track Details"));
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
#[tokio::test]
async fn search_selection_opens_the_actual_library_detail_without_playing() {
    for tab in [SearchTab::Tracks, SearchTab::Global] {
        let mut state = AppState::new();
        state.popups.set_search_active(true);
        state.search.tab = tab;
        state.search.query = "Second".into();
        state.search.focus = SearchFocus::Results;
        state.search.results = Some(SearchResults {
            tracks: tracks(),
            ..Default::default()
        });
        state.list_state.search_item_index = 1;
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let actions = textamp::app::handlers::dispatch_search::dispatch(
            &tx,
            SearchAction::SelectSearchResult,
            &mut state,
        )
        .unwrap();
        assert!(
            matches!(actions.as_slice(), [Action::Settings(textamp::app::action::SettingsAction::RememberSearchResult(result)), Action::Browse(BrowseAction::OpenDetailsInLibrary{track,destination:textamp::app::library_details::Destination::Track})] if result.key == "second" && track.rating_key=="second")
        );
        let Action::Settings(textamp::app::action::SettingsAction::RememberSearchResult(result)) =
            &actions[0]
        else {
            panic!("expected a saved result");
        };
        let stored = serde_json::to_string(result).unwrap();
        state.search.recent = vec![serde_json::from_str(&stored).unwrap()];
        state.search.query.clear();
        state.search.results = None;
        state.list_state.search_item_index = 0;
        assert!(matches!(
            key(&mut state, KeyCode::Enter).as_slice(),
            [Action::Search(SearchAction::SelectSearchResult)]
        ));
        let actions = textamp::app::handlers::dispatch_search::dispatch(
            &tx,
            SearchAction::SelectSearchResult,
            &mut state,
        )
        .unwrap();
        assert!(
            matches!(actions.as_slice(), [Action::Browse(BrowseAction::OpenDetailsInLibrary{track,destination:textamp::app::library_details::Destination::Track})] if track.rating_key=="second")
        );
        assert!(state.search.query.is_empty());
        assert!(state.queue.tracks.is_empty());
    }
}
#[test]
fn similar_selection_opens_the_actual_track_detail_without_playing() {
    let mut state = AppState::new();
    state.view = View::Similar;
    state.similar.mode = SimilarMode::Tracks;
    state.similar.tracks = tracks();
    state.list_state.similar_index = 1;
    let actions = key(&mut state, KeyCode::Enter);
    assert!(
        matches!(actions.as_slice(),[Action::Browse(textamp::app::action::BrowseAction::OpenDetailsInLibrary{track,destination:textamp::app::library_details::Destination::Track})] if track.rating_key=="second")
    );
    assert!(state.queue.tracks.is_empty());
}
#[test]
fn queue_enter_opens_library_track_details_without_changing_playback() {
    let mut state = AppState::new();
    state.view = View::Queue;
    state.queue.tracks = tracks();
    state.list_state.queue_index = 1;
    assert!(matches!(
        key(&mut state, KeyCode::Enter).as_slice(),
        [Action::Browse(BrowseAction::OpenDetailsInLibrary {
            destination: textamp::app::library_details::Destination::Track,
            ..
        })]
    ));
}

fn browse_state() -> AppState {
    use textamp::app::sources::{ActiveSource, navidrome::Session};
    use textamp::app::state::{BrowseColumn, BrowseItem};
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

    let tracks = tracks();
    state.artist_nav.columns = vec![BrowseColumn::new_with_tracks(
        "Tracks",
        BrowseItem::from_tracks(&tracks),
        tracks.clone(),
    )];
    state.track_pane_open = true;
    state.track_pane_focused = true;
    state.track_pane_index = 3;
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
    for _ in [()] {
        for (w, h) in [(140, 40), (100, 32), (80, 24)] {
            draw(&mut state, "adjacent-details-automatic", w, h);
            let right = state.hit_regions.sonic_details.as_ref().unwrap();
            if let Some(left) = state.hit_regions.track_pane.as_ref() {
                assert_eq!(left.outer.right(), right.outer.x);
            } else {
                assert!(state.hit_regions.miller_h_scrollbar.is_some());
            }
            assert!(right.outer.right() <= w);
        }
    }

    assert_single(&key(&mut state, KeyCode::Enter));
    assert_following(&key_input::handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
        &mut state,
        &Config::default(),
    ));
    for expected in ["third", "second"] {
        key(&mut state, KeyCode::Down);
        assert_eq!(
            sonic_details::visible(&state).unwrap().details.action_index,
            3
        );
        assert!(key(&mut state, KeyCode::Enter).is_empty());
        assert_eq!(
            sonic_details::selected_track(&state).unwrap().rating_key,
            expected
        );
        assert_eq!(state.focused_track().unwrap().rating_key, "first");
        assert_eq!(state.artist_nav.columns.len(), 1);
    }
    key(&mut state, KeyCode::Left);
    assert!(!sonic_details::focused(&state));
    assert_eq!(state.track_pane_index, 3);
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
        state.popups.set_search_active(search);
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
        let cmd = command_palette::materialize_entries(&state)
            .into_iter()
            .find(|e| e.label == "Open in Library")
            .unwrap()
            .command;
        let actions = command_palette::run(cmd, &mut state);
        assert!(
            matches!(actions.as_slice(), [Action::Browse(BrowseAction::OpenDetailsInLibrary {
            track, destination: textamp::app::library_details::Destination::Track
        })] if track.rating_key == expected),
            "{view:?} {search}: {actions:?}"
        );
    }
    state.view = View::Browse;
    state.popups.set_search_active(false);
    state
        .track_pane_similar
        .insert("first".into(), Ok(tracks[1..].to_vec()));
    key(&mut state, KeyCode::Enter);
    let cmd = command_palette::materialize_entries(&state)
        .into_iter()
        .find(|e| e.label == "Open in Library")
        .unwrap()
        .command;
    assert!(
        matches!(command_palette::run(cmd,&mut state).as_slice(), [Action::Browse(BrowseAction::OpenDetailsInLibrary { track, destination: textamp::app::library_details::Destination::Track })] if track.rating_key == "second")
    );
    assert_eq!(state.queue.index, Some(0));
}

#[tokio::test]
async fn related_track_jump_rebuilds_ancestors_and_back_restores_the_previous_view() {
    use std::sync::Arc;
    use textamp::{
        app::{
            action::{BrowseAction, MillerAction},
            handlers::{dispatch_browse, dispatch_miller},
            library_details, navigation_history,
            state::BrowseItem,
        },
        audio::AudioPlayer,
        library::models::{Album, Artist},
    };
    let mut state = browse_state();
    let album = Album {
        rating_key: "related-album".into(),
        title: "Related Album".into(),
        parent_rating_key: Some("related-artist".into()),
        parent_title: Some("Related Artist".into()),
        ..Default::default()
    };
    state.library.artists = Arc::new(vec![Artist {
        rating_key: "related-artist".into(),
        title: "Related Artist".into(),
        ..Default::default()
    }]);
    state.library.albums = Arc::new(vec![album.clone()]);
    let tracks: Vec<_> = tracks()
        .into_iter()
        .map(|mut track| {
            track.parent_rating_key = Some(album.rating_key.clone());
            track.parent_title = Some(album.title.clone());
            track.grandparent_rating_key = album.parent_rating_key.clone();
            track.grandparent_title = album.parent_title.clone();
            track
        })
        .collect();
    state
        .track_pane_similar
        .insert("first".into(), Ok(tracks[1..].to_vec()));
    state
        .track_pane_similar
        .insert("second".into(), Ok(vec![tracks[2].clone()]));
    state.list_filter.query = "First".into();
    state.scroll.browse = Some((0, 2));
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let mut audio = AudioPlayer::new_without_audio();
    for (seed, target) in [("first", "second"), ("second", "third")] {
        let actions = dispatch_browse::dispatch(
            &tx,
            BrowseAction::OpenSimilarTrack {
                seed_key: seed.into(),
                track_key: target.into(),
            },
            &mut state,
        )
        .unwrap();
        assert!(
            matches!(actions.as_slice(), [Action::Miller(MillerAction::LoadArtistAlbumsForMiller {artist_key,..})] if artist_key == "related-artist")
        );
        let request_id = state.artist_nav_request_id;
        dispatch_miller::dispatch(
            &tx,
            MillerAction::ArtistAlbumsForMillerLoaded {
                request_id,
                artist_key: "related-artist".into(),
                replace_child: false,
                is_catalog_artist: true,
                result: Ok(vec![album.clone()]),
            },
            &mut state,
            &mut audio,
        )
        .unwrap();
        dispatch_miller::dispatch(
            &tx,
            MillerAction::AlbumTracksForMillerLoaded {
                request_id,
                album_key: album.rating_key.clone(),
                album_title: album.title.clone(),
                replace_child: false,
                result: Ok(tracks.clone()),
            },
            &mut state,
            &mut audio,
        )
        .unwrap();
        let actions = library_details::finish(&mut state);
        assert!(matches!(
            actions.as_slice(),
            [Action::Browse(BrowseAction::OpenTrackDetails)]
        ));
        dispatch_browse::dispatch(&tx, BrowseAction::OpenTrackDetails, &mut state).unwrap();
        assert_eq!(state.artist_nav.columns.len(), 3);
        assert!(
            matches!(state.artist_nav.columns[0].selected_item(), Some(BrowseItem::Artist {key,..}) if key == "related-artist")
        );
        assert!(
            matches!(state.artist_nav.columns[1].selected_item(), Some(BrowseItem::Album {key,..}) if key == "related-album")
        );
        assert_eq!(state.focused_track().unwrap().rating_key, target);
        assert!(state.track_pane_open);
        let history = state.jump_history.head();
        assert!(
            dispatch_browse::dispatch(
                &tx,
                BrowseAction::OpenSimilarTrack {
                    seed_key: seed.into(),
                    track_key: target.into()
                },
                &mut state
            )
            .unwrap()
            .is_empty()
        );
        assert_eq!(state.jump_history.head(), history);
    }
    navigation_history::back(&mut state);
    assert_eq!(state.focused_track().unwrap().rating_key, "second");
    assert_eq!(state.artist_nav.columns.len(), 3);
    assert!(state.track_pane_open);
    navigation_history::back(&mut state);
    assert_eq!(state.focused_track().unwrap().rating_key, "first");
    assert_eq!(state.artist_nav.columns.len(), 1);
    assert_eq!(state.list_filter.query, "First");
    assert_eq!(state.scroll.browse, Some((0, 2)));
    assert!(state.track_pane_open);
    assert!(state.jump_history.head().is_none());
    assert!(state.queue.tracks.is_empty());
    assert_eq!(audio.playback_id(), 0);
}
