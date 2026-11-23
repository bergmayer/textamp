use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};
use textamp::app::{
    Action, AppState, action::BrowseAction, handlers::mouse_input, presentation::ArtistLink,
    state::*,
};
use textamp::library::models::{Artist, SearchResults, Track};
use textamp::services::biography::Biography;

fn fixture() -> AppState {
    let mut state = AppState::new();
    state.view = View::NowPlaying;
    state.category_column_focused = false;
    state.library.artists = std::sync::Arc::new(
        [("a", "Artist A"), ("b", "Artist B")]
            .into_iter()
            .map(|(key, name)| Artist {
                rating_key: key.into(),
                title: name.into(),
                ..Default::default()
            })
            .collect(),
    );
    state.queue.tracks = vec![Track {
        rating_key: "track".into(),
        title: "Artist A".into(),
        grandparent_title: Some("Artist A".into()),
        grandparent_rating_key: Some("a".into()),
        parent_title: Some("Album A".into()),
        parent_rating_key: Some("album".into()),
        ..Default::default()
    }];
    state.queue.index = Some(0);
    state.playback_mode = PlaybackMode::Queue;
    state
}
fn draw(state: &mut AppState) -> ratatui::buffer::Buffer {
    state.terminal_width = 180;
    state.terminal_height = 48;
    let mut terminal = Terminal::new(TestBackend::new(180, 48)).unwrap();
    terminal
        .draw(|frame| state.apply_render_feedback(textamp::ui::render(frame, state)))
        .unwrap();
    terminal.backend().buffer().clone()
}
fn click(state: &mut AppState, link: &ArtistLink) -> Vec<Action> {
    mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: link.area.x,
            row: link.area.y,
            modifiers: KeyModifiers::NONE,
        },
        state,
    )
}
fn opens_artist(actions: &[Action], key: &str) -> bool {
    matches!(actions,[Action::Browse(BrowseAction::OpenDetailsInLibrary{track, destination: textamp::app::library_details::Destination::Artist})] if track.grandparent_rating_key.as_deref()==Some(key))
}
#[test]
fn queue_and_transport_artist_labels_open_library_without_playing_or_matching_track_titles() {
    let mut state = fixture();
    let buffer = draw(&mut state);
    let queue = state
        .hit_regions
        .queue_content
        .as_ref()
        .unwrap()
        .track_list_inner;
    let links = state.hit_regions.artist_links.clone();
    assert!(
        links
            .iter()
            .any(|link| link.area.y >= queue.y && link.area.y < queue.bottom())
    );
    assert!(links.iter().any(|link| link.area.x < queue.x));
    for link in &links {
        assert_eq!(
            buffer[(link.area.x, link.area.y)].symbol(),
            "A",
            "artist target must begin on the rendered name: {link:?}"
        );
        assert!(opens_artist(&click(&mut state, link), "a"));
    }
    assert_eq!(state.queue.index, Some(0));
    assert!(
        !links.iter().any(|link| link.area.y == queue.y),
        "track title must retain its selection/play behavior"
    );
}
#[test]
fn stale_artist_link_is_consumed_without_navigating_or_playing() {
    let mut state = fixture();
    draw(&mut state);
    let link = state.hit_regions.artist_links[0].clone();
    state.library_generation += 1;
    assert!(click(&mut state, &link).is_empty());
    assert_eq!(state.queue.index, Some(0));
}
#[test]
fn track_metadata_and_search_artist_spans_are_clickable() {
    let mut state = fixture();
    state.view = View::Browse;
    let track = state.queue.tracks[0].clone();
    state.artist_nav.columns = vec![BrowseColumn::new_with_tracks(
        "tracks",
        BrowseItem::from_tracks(std::slice::from_ref(&track)),
        vec![track.clone()],
    )];
    state.track_pane_open = true;
    draw(&mut state);
    let pane = state.hit_regions.track_pane.as_ref().unwrap().outer;
    let link = state
        .hit_regions
        .artist_links
        .iter()
        .find(|link| pane.contains((link.area.x, link.area.y).into()))
        .unwrap()
        .clone();
    assert!(opens_artist(&click(&mut state, &link), "a"));
    state.view = View::Search;
    state.search.query = "Artist".into();
    state.search.tab = SearchTab::Tracks;
    state.search.results = Some(SearchResults {
        tracks: vec![track],
        ..Default::default()
    });
    draw(&mut state);
    let link = state
        .hit_regions
        .artist_links
        .iter()
        .find(|link| link.key == "a")
        .unwrap()
        .clone();
    assert!(opens_artist(&click(&mut state, &link), "a"));
}
#[test]
fn artist_info_and_biography_reader_show_clickable_related_artists() {
    let mut state = fixture();
    state.view = View::Browse;
    let document = Biography {
        text: "No biography available".into(),
        related_artists: vec![state.library.artists[1].clone()],
        ..Default::default()
    };
    let mut info = BrowseColumn::new("Artist Info — Artist A", vec![]);
    info.details = Some(BrowseDetails::Artist(ArtistInfoDetails {
        artist_key: "a".into(),
        artist_name: "Artist A".into(),
        request_id: 1,
        document: document.clone(),
        image_index: 0,
        task: None,
    }));
    state.artist_nav.columns = vec![
        BrowseColumn::new(
            "Albums",
            vec![BrowseItem::ArtistInfo {
                artist_key: "a".into(),
                artist_name: "Artist A".into(),
                thumb: None,
            }],
        ),
        info,
    ];
    state.artist_nav.focused_column = 1;
    draw(&mut state);
    let link = state
        .hit_regions
        .artist_links
        .iter()
        .find(|link| link.key == "b")
        .expect("related artist in info column")
        .clone();
    assert!(opens_artist(&click(&mut state, &link), "b"));
    state.popups.set_artist_bio(Some(ArtistBioPopup {
        artist_key: "a".into(),
        artist_name: "Artist A".into(),
        document,
        scroll: 0,
        google_focused: false,
        image_index: 0,
        task: None,
    }));
    draw(&mut state);
    let link = state
        .hit_regions
        .artist_links
        .iter()
        .find(|link| link.key == "b")
        .expect("related artist in reader")
        .clone();
    assert!(opens_artist(&click(&mut state, &link), "b"));
}

#[test]
fn artist_labels_handle_unicode_and_clipped_names_and_modal_capture() {
    for name in [
        "宇多田ヒカル",
        "A very long artist name that is longer than the available queue subtitle width and must be clipped",
    ] {
        let mut state = fixture();
        std::sync::Arc::make_mut(&mut state.library.artists)[0].title = name.into();
        state.queue.tracks[0].grandparent_title = Some(name.into());
        let buffer = draw(&mut state);
        let link = state
            .hit_regions
            .artist_links
            .iter()
            .find(|link| link.key == "a")
            .expect("visible artist label")
            .clone();
        assert_eq!(
            buffer[(link.area.x, link.area.y)].symbol(),
            name.chars().next().unwrap().to_string()
        );
        assert!(opens_artist(&click(&mut state, &link), "a"));
        textamp::app::command_palette::open_with_query(&mut state, "");
        draw(&mut state);
        assert!(
            !opens_artist(&click(&mut state, &link), "a"),
            "palette must capture the underlying name click"
        );
    }
}
#[test]
fn an_explicit_artist_identity_disambiguates_equal_names() {
    let mut state = fixture();
    std::sync::Arc::make_mut(&mut state.library.artists).insert(
        0,
        Artist {
            rating_key: "other-a".into(),
            title: "Artist A".into(),
            ..Default::default()
        },
    );
    draw(&mut state);
    let queue = state
        .hit_regions
        .queue_content
        .as_ref()
        .unwrap()
        .track_list_inner;
    let link = state
        .hit_regions
        .artist_links
        .iter()
        .find(|link| queue.contains((link.area.x, link.area.y).into()))
        .unwrap()
        .clone();
    assert_eq!(link.key, "a");
    assert!(opens_artist(&click(&mut state, &link), "a"));
}

#[test]
fn an_artist_name_containing_a_comma_is_one_album_metadata_destination() {
    let mut state = fixture();
    state.view = View::Browse;
    std::sync::Arc::make_mut(&mut state.library.artists)[0].title = "Earth, Wind & Fire".into();
    std::sync::Arc::make_mut(&mut state.library.artists).push(Artist {
        rating_key: "earth".into(),
        title: "Earth".into(),
        ..Default::default()
    });
    let mut column = BrowseColumn::new("Album Info", vec![]);
    column.details = Some(BrowseDetails::Album {
        lines: vec!["Album".into(), "Artist: Earth, Wind & Fire".into()],
        artwork_key: "album".into(),
    });
    state.artist_nav.columns = vec![column];
    draw(&mut state);
    let link = state
        .hit_regions
        .artist_links
        .iter()
        .find(|link| link.key == "a")
        .expect("one complete artist label")
        .clone();
    assert!(
        !state
            .hit_regions
            .artist_links
            .iter()
            .any(|link| link.key == "earth")
    );
    assert!(opens_artist(&click(&mut state, &link), "a"));
}

#[test]
fn now_playing_track_and_album_names_open_details_without_flipping_or_playing() {
    use textamp::app::library_details::Destination;
    let mut state = fixture();
    draw(&mut state);
    let links = state.hit_regions.detail_links.clone();
    for destination in [Destination::Track, Destination::Album] {
        assert!(
            links
                .iter()
                .filter(|link| link.destination == destination)
                .count()
                >= 2,
            "both the main metadata and transport should expose {destination:?}"
        );
    }
    for link in links {
        let actions = mouse_input::handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: link.area.x,
                row: link.area.y,
                modifiers: KeyModifiers::NONE,
            },
            &mut state,
        );
        assert!(
            matches!(actions.as_slice(), [Action::Browse(BrowseAction::OpenDetailsInLibrary { track, destination })]
            if *destination == link.destination && track.rating_key == "track")
        );
        assert_eq!(state.view, View::NowPlaying);
        assert_eq!(state.queue.index, Some(0));
    }
    let link = state.hit_regions.detail_links[0].clone();
    state.playback.request_id += 1;
    assert!(link.action(&state).is_none());
}

#[test]
fn multi_artist_transport_has_one_keyboard_stop_for_each_metadata_control() {
    use textamp::app::transport_controls::{self, TransportControl};
    let mut state = fixture();
    state.queue.tracks[0].original_title = Some("Artist A; Artist B".into());
    draw(&mut state);
    let controls = transport_controls::visible(&state);
    assert_eq!(
        controls
            .iter()
            .filter(|control| **control == TransportControl::Artist)
            .count(),
        1
    );
    assert_eq!(
        controls
            .iter()
            .filter(|control| **control == TransportControl::Album)
            .count(),
        1
    );
}
