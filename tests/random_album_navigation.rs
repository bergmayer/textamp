use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use textamp::app::{
    Action, AppState,
    action::{BrowseAction, MillerAction},
    command_palette,
    handlers::key_input,
    sources::{ActiveSource, navidrome::Session},
    state::{BrowseCategory, BrowseItem, PaletteCommandKind, View},
};
use textamp::{
    config::Config,
    library::models::{Album, Artist, Track},
    navidrome::{Client, Source},
};

fn fixture() -> AppState {
    let mut state = AppState::new();
    let source = Source {
        id: "fixture".into(),
        name: "Fixture".into(),
        url: "http://127.0.0.1:1".into(),
        username: "fixture".into(),
        libraries: vec![],
    };
    let client = Client::new(&source, "fixture-password".into(), None).unwrap();
    state.sources.active = ActiveSource::Navidrome(Box::new(Session {
        source,
        client,
        extensions: Default::default(),
    }));
    state.library.albums = std::sync::Arc::new(
        (0..4)
            .map(|n| Album {
                rating_key: format!("album-{n}"),
                title: format!("Album {n}"),
                parent_rating_key: Some(format!("artist-{n}")),
                parent_title: Some(format!("Artist {n}")),
                ..Default::default()
            })
            .collect(),
    );
    state.library.artists = std::sync::Arc::new(
        (0..4)
            .map(|n| Artist {
                rating_key: format!("artist-{n}"),
                title: format!("Artist {n}"),
                ..Default::default()
            })
            .collect(),
    );
    state.queue.tracks = vec![Track {
        rating_key: "playing".into(),
        title: "Playing".into(),
        ..Default::default()
    }];
    state.queue.index = Some(0);
    state.view = View::NowPlaying;
    state
}
#[test]
fn random_album_reveals_a_catalog_album_without_playing_and_clears_stale_navigation() {
    let mut state = fixture();
    state.search.pending_album_key = Some("old".into());
    state.category_column_focused = true;
    let old_request = state.artist_nav_request_id;
    let actions = command_palette::run(PaletteCommandKind::GoToRandomAlbum, &mut state);
    let [
        Action::Browse(BrowseAction::OpenDetailsInLibrary {
            track,
            destination: textamp::app::library_details::Destination::Album,
        }),
    ] = actions.as_slice()
    else {
        panic!("must use the shared library jump")
    };
    let artist_key = track.grandparent_rating_key.as_ref().unwrap();
    let album_key = track.parent_rating_key.as_ref().unwrap();
    let (tx, _) = tokio::sync::mpsc::channel(8);
    let Action::Browse(action) = actions[0].clone() else {
        unreachable!()
    };
    let follow =
        textamp::app::handlers::dispatch_browse::dispatch(&tx, action, &mut state).unwrap();
    assert!(matches!(
        follow.as_slice(),
        [Action::Miller(
            MillerAction::LoadArtistAlbumsForMiller { .. }
        )]
    ));
    assert_eq!(state.view, View::Browse);
    assert_eq!(state.browse_category, BrowseCategory::Library);
    assert!(!state.category_column_focused);
    assert_ne!(state.artist_nav_request_id, old_request);
    assert!(
        matches!(state.artist_nav.selected_item(),Some(BrowseItem::Artist {key,..}) if key==artist_key)
    );
    let mut audio = textamp::audio::AudioPlayer::new_without_audio();
    let albums = state
        .library
        .albums
        .iter()
        .filter(|album| album.parent_rating_key.as_ref() == Some(artist_key))
        .cloned()
        .collect();
    let current = state.artist_nav_request_id;
    let follow = textamp::app::handlers::dispatch_miller::dispatch(
        &tx,
        MillerAction::ArtistAlbumsForMillerLoaded {
            request_id: current,
            artist_key: artist_key.clone(),
            replace_child: false,
            is_catalog_artist: true,
            result: Ok(albums),
        },
        &mut state,
        &mut audio,
    )
    .unwrap();
    assert!(follow.iter().any(|action|matches!(action,Action::Miller(MillerAction::LoadAlbumTracksForMiller {album_key:key,..}) if key==album_key)));
    assert!(
        matches!(state.artist_nav.columns[0].selected_item(),Some(BrowseItem::Artist{key,..}) if key==artist_key)
    );
    assert!(
        matches!(state.artist_nav.columns[1].selected_item(),Some(BrowseItem::Album{key,..}) if key==album_key)
    );
    assert_eq!(state.queue.tracks[0].rating_key, "playing");
    assert_eq!(state.queue.index, Some(0));
    textamp::app::handlers::dispatch_miller::dispatch(
        &tx,
        MillerAction::ArtistAlbumsForMillerLoaded {
            request_id: old_request,
            artist_key: "old".into(),
            replace_child: false,
            is_catalog_artist: true,
            result: Ok(vec![]),
        },
        &mut state,
        &mut audio,
    )
    .unwrap();
    assert_eq!(state.artist_nav.columns.len(), 2);
}
#[test]
fn terminal_shift_control_r_handles_both_terminal_letter_encodings() {
    for (code, modifiers) in [
        (KeyCode::Char('R'), KeyModifiers::NONE),
        (KeyCode::Char('R'), KeyModifiers::SHIFT),
        (
            KeyCode::Char('r'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
        (
            KeyCode::Char('R'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
        (KeyCode::Char('R'), KeyModifiers::ALT | KeyModifiers::SHIFT),
        (KeyCode::Char('R'), KeyModifiers::ALT),
    ] {
        let mut state = fixture();
        let actions = key_input::handle_key(
            KeyEvent::new(code, modifiers),
            &mut state,
            &Config::default(),
        );
        assert!(matches!(
            actions.as_slice(),
            [Action::Browse(BrowseAction::OpenDetailsInLibrary {
                destination: textamp::app::library_details::Destination::Album,
                ..
            })]
        ));
        assert_eq!(state.view, View::NowPlaying);
    }
}
#[test]
fn empty_or_unsupported_catalog_preserves_the_current_view_and_queue() {
    for unsupported in [false, true] {
        let mut state = fixture();
        if unsupported {
            state.sources.active = ActiveSource::Folder("fixture".into());
        } else {
            std::sync::Arc::make_mut(&mut state.library.albums).clear();
        }
        assert!(command_palette::run(PaletteCommandKind::GoToRandomAlbum, &mut state).is_empty());
        assert_eq!(state.view, View::NowPlaying);
        assert_eq!(state.queue.tracks[0].rating_key, "playing");
        assert!(
            textamp::app::command_catalog::disabled_reason(
                &state,
                &PaletteCommandKind::GoToRandomAlbum
            )
            .is_some()
        );
    }
}

#[test]
fn omnibox_matches_artist_and_album_metadata_and_tabs_only_filter_existing_results() {
    use textamp::app::{
        action::SearchAction,
        handlers::dispatch_search,
        state::{SearchFocus, SearchTab},
    };
    let mut state = fixture();
    std::sync::Arc::make_mut(&mut state.library.artists)[0].title = "Thee Headcoats".into();
    std::sync::Arc::make_mut(&mut state.library.albums)[0].parent_title =
        Some("Thee Headcoats".into());
    state.library.all_tracks = std::sync::Arc::new(vec![Track {
        rating_key: "song".into(),
        title: "Instrumental".into(),
        grandparent_title: Some("Thee Headcoats".into()),
        parent_title: Some("Archive".into()),
        genres: vec!["Garage".into()],
        ..Default::default()
    }]);
    state.popups.set_search_active(true);
    state.search.tab = SearchTab::Tracks;
    let (tx, _) = tokio::sync::mpsc::channel(8);
    let actions = dispatch_search::dispatch(
        &tx,
        SearchAction::SetSearchQuery("headcoats".into()),
        &mut state,
    )
    .unwrap();
    assert_eq!(state.search.tab, SearchTab::Global);
    assert_eq!(state.search.focus, SearchFocus::Input);
    for action in actions {
        let Action::Search(action) = action else {
            panic!("search action")
        };
        dispatch_search::dispatch(&tx, action, &mut state).unwrap();
    }
    let result = state.search.results.as_ref().unwrap();
    assert_eq!(result.artists.len(), 1);
    assert_eq!(result.albums.len(), 1);
    assert_eq!(result.tracks.len(), 1);
    dispatch_search::dispatch(
        &tx,
        SearchAction::SetSearchTab(SearchTab::Tracks),
        &mut state,
    )
    .unwrap();
    assert_eq!(state.search.query, "headcoats");
    assert_eq!(state.search.results.as_ref().unwrap().artists.len(), 1);
    assert!(
        dispatch_search::dispatch(
            &tx,
            SearchAction::SetSearchQuery("headcoats".into()),
            &mut state
        )
        .unwrap()
        .is_empty()
    );
    assert_eq!(state.search.tab, SearchTab::Tracks);
}

#[test]
fn refreshing_artist_order_keeps_the_jump_destination_selected() {
    let mut state = fixture();
    state
        .artist_nav
        .reset("artists", state.build_artist_root_items());
    state.artist_nav.columns[0].selected_index = 3;
    let key = state.artist_nav.columns[0]
        .selected_item()
        .unwrap()
        .key()
        .to_owned();
    std::sync::Arc::make_mut(&mut state.library.artists).reverse();
    state
        .artist_nav
        .update_root_items("artists", state.build_artist_root_items());
    assert_eq!(
        state.artist_nav.columns[0].selected_item().unwrap().key(),
        key
    );
}
