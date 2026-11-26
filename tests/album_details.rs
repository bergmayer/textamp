use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};
use textamp::app::{
    Action, AppState,
    action::MillerAction,
    handlers::{key_input, mouse_input},
    state::{BrowseCategory, BrowseColumn, BrowseItem, ColumnHeaderSelection, PlayAllRow, View},
};
use textamp::{audio::AudioPlayer, config::Config, library::models::Track};

mod common;

#[tokio::test]
async fn album_details_requests_similar_music_once_for_its_own_seed() {
    use textamp::app::{Event, action::DataAction};

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
    state.artist_nav.columns = vec![tracks];
    state.artist_nav.focused_column = 0;
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

fn album_with_recommendations() -> AppState {
    let mut state = common::navidrome_state();
    state.view = View::Browse;
    state.browse_category = BrowseCategory::Library;
    let tracks: Vec<_> = (0..48)
        .map(|i| Track {
            rating_key: format!("seed-{i}"),
            title: format!("Recording {i:02}"),
            ..Default::default()
        })
        .collect();
    let mut column =
        BrowseColumn::new_with_tracks("Album", BrowseItem::from_tracks(&tracks), tracks);
    column.play_all_row = Some(PlayAllRow::Album {
        rating_key: "source".into(),
        title: "Album".into(),
    });
    state.artist_nav.columns = vec![column];
    state.track_pane_similar.insert(
        "seed-0".into(),
        Ok((0..12)
            .map(|i| Track {
                rating_key: format!("track-{i}"),
                parent_rating_key: Some(format!("album-{i}")),
                parent_title: Some(format!("Album {i}")),
                grandparent_title: Some("Other Artist".into()),
                ..Default::default()
            })
            .collect()),
    );
    state.category_column_focused = false;
    state
}

#[test]
fn terminal_album_tracks_and_recommendations_scroll_in_sequence() {
    let mut state = album_with_recommendations();
    let mut boundary = false;
    let mut saw_last = false;
    for _ in 0..64 {
        let screen = draw(&mut state);
        let region = &state.hit_regions.miller_columns.as_ref().unwrap().columns[0];
        if let Some((tile, target, _)) = state.hit_regions.album_related.first() {
            if region.inner.height > 0 && screen.contains("Recording 47") {
                assert_eq!(target.parent_rating_key.as_deref(), Some("album-0"));
                assert_eq!(tile.y, region.inner.bottom() + 1);
                boundary = true;
            } else {
                assert!(
                    !screen.contains("Recording "),
                    "recommendations appeared before the final track"
                );
            }
        }
        saw_last |= state
            .hit_regions
            .album_related
            .iter()
            .any(|(_, target, _)| target.parent_rating_key.as_deref() == Some("album-11"));
        let (x, y) = (region.area.x + 1, region.area.y + 2);
        state.scroll.scroll_cooldown = None;
        mouse_input::handle_mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: x,
                row: y,
                modifiers: KeyModifiers::NONE,
            },
            &mut state,
        );
    }
    assert!(boundary && saw_last);
    assert_eq!(state.artist_nav.columns[0].selected_index, 0);
}

#[test]
fn dragging_the_album_scrollbar_reaches_the_last_recommendation_and_returns() {
    let mut state = album_with_recommendations();
    draw_at_height(&mut state, 24);
    let area = state.hit_regions.miller_columns.as_ref().unwrap().columns[0].area;
    let pointer = |state: &mut AppState, kind, row| {
        mouse_input::handle_mouse(
            MouseEvent {
                kind,
                column: area.right() - 1,
                row,
                modifiers: KeyModifiers::NONE,
            },
            state,
        );
    };
    let top = area.y + 1;
    let bottom = area.bottom() - 2;
    pointer(&mut state, MouseEventKind::Down(MouseButton::Left), top);
    pointer(&mut state, MouseEventKind::Drag(MouseButton::Left), bottom);
    assert!(draw_at_height(&mut state, 24).contains("Album 11"));
    assert_eq!(state.artist_nav.columns[0].selected_index, 0);
    pointer(&mut state, MouseEventKind::Drag(MouseButton::Left), top);
    pointer(&mut state, MouseEventKind::Up(MouseButton::Left), top);
    assert!(draw_at_height(&mut state, 24).contains("Recording 00"));
}

#[test]
fn keyboard_moves_between_the_last_track_and_visible_recommended_albums() {
    use textamp::app::{action::BrowseAction, library_details::Destination};

    let mut state = album_with_recommendations();
    state.artist_nav.columns[0].selected_index = 47;
    state.artist_nav.columns[0].header_selection = ColumnHeaderSelection::Items;
    draw_at_height(&mut state, 24);
    key(&mut state, KeyCode::Down);
    let screen = draw_at_height(&mut state, 24);
    assert!(screen.contains("Album 0"));
    assert_eq!(
        state.artist_nav.columns[0].recommendation_focus,
        Some((0, 0))
    );

    let mut steps = 0;
    loop {
        let previous = state.artist_nav.columns[0].recommendation_focus;
        key(&mut state, KeyCode::Down);
        let screen = draw_at_height(&mut state, 24);
        let current = state.artist_nav.columns[0].recommendation_focus;
        let Some((0, item)) = current else {
            panic!("lost album focus")
        };
        let id = format!("album-{item}");
        assert!(screen.contains(&format!("Album {item}")));
        assert!(matches!(key(&mut state, KeyCode::Enter).as_slice(),
            [Action::Browse(BrowseAction::OpenDetailsInLibrary { track, destination: Destination::Album })]
                if track.parent_rating_key.as_ref() == Some(&id)));
        if current == previous {
            break;
        }
        steps += 1;
        assert!(steps < 12);
    }
    assert!(steps > 0);
    for _ in 0..=steps {
        key(&mut state, KeyCode::Up);
        draw_at_height(&mut state, 24);
    }
    assert_eq!(state.artist_nav.columns[0].recommendation_focus, None);
    assert!(draw_at_height(&mut state, 24).contains("Recording 47"));
}

#[test]
fn artist_biography_completion_preserves_the_album_cursor() {
    use textamp::app::{
        Event, dispatch::handle_core_event, event::UiEvent, state::ArtistInfoDetails,
    };
    let mut state = state(BrowseCategory::Library);
    let column = &mut state.artist_nav.columns[0];
    column.play_all_row = None;
    column.items = ["First release", "Second release"]
        .map(|title| BrowseItem::Album {
            title: title.into(),
            key: title.into(),
            artist: "Artist".into(),
            thumb: None,
            year: None,
            is_placeholder: false,
        })
        .to_vec();
    column.tracks.clear();
    column.selected_index = 1;
    column.artist_header = Some(ArtistInfoDetails {
        artist_key: "artist".into(),
        artist_name: "Artist".into(),
        request_id: 7,
        task: None,
        image_index: 0,
        document: Default::default(),
    });
    let (tx, _) = tokio::sync::mpsc::channel(8);
    handle_core_event(
        Event::Ui(UiEvent::ArtistInfoLoaded {
            generation: state.library_generation,
            request_id: 7,
            artist_key: "artist".into(),
            result: Ok(textamp::services::biography::Biography {
                text: "Inline biography".into(),
                related_artists: vec![textamp::library::models::Artist {
                    rating_key: "neighbor".into(),
                    title: "Neighbor".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
        }),
        &mut state,
        &tx,
    );
    assert_eq!(state.artist_nav.columns[0].selected_index, 1);
    let rendered = draw(&mut state);
    let bio = rendered.find("Inline biography").unwrap();
    let more = rendered.find("…more").unwrap();
    let album = rendered.find("Second release").unwrap();
    let related = rendered.find("Similar Artists").unwrap();
    assert!(bio < more && more < album && album < related);
    assert_eq!(
        state.hit_regions.album_related[0]
            .1
            .grandparent_rating_key
            .as_deref(),
        Some("neighbor")
    );
    let press = |state: &mut AppState, code| {
        key_input::handle_key(
            KeyEvent::new(code, KeyModifiers::NONE),
            state,
            &Config::default(),
        )
    };
    press(&mut state, KeyCode::Up);
    assert_eq!(
        state.tui_column_header.unwrap().control,
        textamp::app::column_controls::ColumnControl::ArtistBio
    );
    assert!(
        matches!(press(&mut state, KeyCode::Enter).as_slice(), [Action::Search(textamp::app::action::SearchAction::ShowArtistBio { artist_key, .. })] if artist_key == "artist")
    );
    press(&mut state, KeyCode::Down);
    assert!(press(&mut state, KeyCode::Right).is_empty());
    assert_eq!(state.artist_nav.columns.len(), 1);
    assert_eq!(state.artist_nav.focused_column, 0);
    press(&mut state, KeyCode::Down);
    assert_eq!(
        state.artist_nav.columns[0].recommendation_focus,
        Some((1, 0))
    );
    draw(&mut state);
    let actions = press(&mut state, KeyCode::Enter);
    assert!(
        matches!(actions.as_slice(), [Action::Browse(textamp::app::action::BrowseAction::OpenDetailsInLibrary{track, destination:textamp::app::library_details::Destination::Artist})] if track.grandparent_rating_key.as_deref()==Some("neighbor"))
    );
    press(&mut state, KeyCode::Up);
    assert!(state.artist_nav.columns[0].recommendation_focus.is_none());
}
#[test]
fn missing_biography_searches_for_the_displayed_artist_with_the_selected_provider() {
    use textamp::app::{
        action::SystemAction,
        column_controls::{self, ColumnControl},
        playback_controls::QueuePlacement,
        state::ArtistInfoDetails,
    };
    use textamp::services::web_search::Provider;
    let mut state = state(BrowseCategory::Library);
    state.web_search_provider = Provider::DuckDuckGo;
    let mut info = ArtistInfoDetails {
        artist_key: "abc".into(),
        artist_name: "A & B".into(),
        request_id: 1,
        document: Default::default(),
        task: None,
        image_index: 0,
    };
    assert!(info.needs_web_search());
    info.document.text = "No biography available".into();
    info.document.unavailable = true;
    assert!(info.excerpt().is_empty());
    state.artist_nav.columns[0].artist_header = Some(info);
    let actions =
        column_controls::activate(&mut state, 0, ColumnControl::ArtistBio, QueuePlacement::Now);
    assert!(
        matches!(&actions[..], [Action::System(SystemAction::OpenBrowser(url))]
        if url == "https://duckduckgo.com/?q=A%20%26%20B%20music%20artist%20biography")
    );
}

#[test]
fn an_empty_biography_keeps_its_reader_and_the_album_search_button() {
    use textamp::app::{
        details::album_bio, dispatch::handle_core_event, event::UiEvent, state::ArtistBioPopup,
    };
    let mut state = state(BrowseCategory::Library);
    state.artist_nav.columns[0].tracks[0].grandparent_rating_key = Some("artist".into());
    let label = |s: &AppState| album_bio(s, &s.artist_nav.columns[0]).unwrap().label;
    assert_eq!(label(&state), "Search Google");
    state.popups.set_artist_bio(Some(ArtistBioPopup {
        artist_key: "artist".into(),
        artist_name: "Artist".into(),
        document: Default::default(),
        scroll: 0,
        google_focused: false,
        image_index: 0,
        task: None,
    }));
    let (tx, _) = tokio::sync::mpsc::channel(8);
    let complete = |s: &mut AppState, request_id| {
        handle_core_event(
            UiEvent::ArtistBioLoaded {
                request_id,
                generation: s.library_generation,
                result: Err("No biography available".into()),
            }
            .into(),
            s,
            &tx,
        );
    };
    state.artist_bio_request_id = 7;
    complete(&mut state, 6);
    assert_eq!(label(&state), "Search Google");
    assert!(state.popups.artist_bio().unwrap().document.text.is_empty());
    complete(&mut state, 7);
    assert_eq!(
        state.popups.artist_bio().unwrap().document.text,
        "No biography available"
    );
    state.popups.set_artist_bio(None);
    assert_eq!(label(&state), "Search Google");
    state.web_search_provider = textamp::services::web_search::Provider::DuckDuckGo;
    assert_eq!(label(&state), "Search DuckDuckGo");
}

#[test]
fn open_album_switches_from_search_to_the_completed_artist_biography() {
    use textamp::app::{
        action::SearchAction, details::album_page, dispatch::handle_core_event, event::UiEvent,
        handlers::helpers::biography, state::ArtistInfoDetails,
    };
    let mut state = state(BrowseCategory::Library);
    state.web_search_provider = textamp::services::web_search::Provider::DuckDuckGo;
    state.artist_nav.columns[0].tracks[0].grandparent_rating_key = Some("artist".into());
    let header = |id| ArtistInfoDetails {
        artist_key: "artist".into(),
        artist_name: "Artist".into(),
        request_id: id,
        document: Default::default(),
        task: None,
        image_index: 0,
    };
    for id in [7, 8] {
        let mut parent = BrowseColumn::new("Artist", vec![]);
        parent.artist_header = Some(header(id));
        state.artist_nav.columns.insert(0, parent);
    }
    state.artist_nav.focused_column = 2;
    let button = |s: &AppState| {
        album_page(s, &s.artist_nav.columns[2])
            .unwrap()
            .artist_bio
            .unwrap()
    };
    assert_eq!(button(&state).label, "Search DuckDuckGo");
    let (tx, _) = tokio::sync::mpsc::channel(8);
    handle_core_event(
        UiEvent::ArtistInfoLoaded {
            generation: state.library_generation,
            request_id: 7,
            artist_key: "artist".into(),
            result: Ok(textamp::services::biography::Biography {
                text: "Biography from the server".into(),
                ..Default::default()
            }),
        }
        .into(),
        &mut state,
        &tx,
    );
    let action = button(&state);
    assert_eq!(action.label, "Artist Biography");
    let [
        Action::Search(SearchAction::ShowArtistBio {
            artist_key,
            artist_name,
        }),
    ] = action.actions.as_slice()
    else {
        panic!("expected biography reader")
    };
    biography::show(&mut state, &tx, artist_key.clone(), artist_name.clone());
    assert_eq!(
        state.popups.artist_bio().unwrap().document.text,
        "Biography from the server"
    );
    assert_eq!(state.artist_nav.focused_column, 2);
}

#[test]
fn terminal_album_exposes_biography_play_and_album_info_together() {
    use textamp::app::column_controls::ColumnControl;
    let mut state = state(BrowseCategory::Library);
    state.artist_nav.columns[0].tracks[0].grandparent_rating_key = Some("artist".into());
    let mut artist = BrowseColumn::new("Artist", vec![]);
    artist.artist_header = Some(textamp::app::state::ArtistInfoDetails {
        artist_key: "artist".into(),
        artist_name: "Artist".into(),
        request_id: 1,
        task: None,
        image_index: 0,
        document: textamp::services::biography::Biography {
            text: "Artist biography".into(),
            ..Default::default()
        },
    });
    state.artist_nav.columns.insert(0, artist);
    state.artist_nav.focused_column = 1;
    draw(&mut state);
    let controls = &state
        .hit_regions
        .miller_columns
        .as_ref()
        .unwrap()
        .columns
        .last()
        .unwrap()
        .controls;
    let bounds = |control| {
        controls
            .iter()
            .find(|(_, action)| *action == control)
            .unwrap()
            .0
    };
    let bio = bounds(ColumnControl::ArtistBio);
    let play = bounds(ColumnControl::Play);
    let info = bounds(ColumnControl::Details);
    assert_eq!((bio.y, info.y), (play.y, play.y));
    assert!(bio.x < play.x && play.x < info.x);
    let actions = mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: bio.x,
            row: bio.y,
            modifiers: KeyModifiers::NONE,
        },
        &mut state,
    );
    assert!(
        matches!(actions.as_slice(), [Action::Search(textamp::app::action::SearchAction::ShowArtistBio {artist_key,..})] if artist_key=="artist")
    );
}

#[test]
fn track_detail_uses_only_distinct_art_and_includes_embedded_lyrics() {
    let mut state = state(BrowseCategory::Library);
    let track = &mut state.artist_nav.columns[0].tracks[0];
    track.thumb = Some("album-cover".into());
    track.parent_thumb = track.thumb.clone();
    track.lyrics = Some("First line\nSecond line".into());
    state.track_pane_open = true;
    state.artist_nav.columns[0].header_selection = ColumnHeaderSelection::Items;
    let page = textamp::app::details::project(&state).unwrap();
    assert!(!page.track_artwork);
    assert_eq!(page.lyrics.as_deref(), Some("First line\nSecond line"));
    state.track_pane_focused = true;
    let screen = draw(&mut state);
    assert!(screen.contains("Second line"));
    state.artist_nav.columns[0].tracks[0].lyrics =
        Some(format!("{}\nFinal lyric", "Verse\n".repeat(25)));
    let screen = draw(&mut state);
    assert!(!screen.contains("Final lyric"));
    for _ in 0..5 {
        key(&mut state, KeyCode::PageDown);
        draw(&mut state);
    }
    assert!(draw(&mut state).contains("Final lyric"));
    state.artist_nav.columns[0].tracks[0].thumb = Some("single-cover".into());
    assert!(
        textamp::app::details::project(&state)
            .unwrap()
            .track_artwork
    );
    state.artist_nav.columns[0].tracks[0].parent_rating_key = Some("album".into());
    state.artwork.grid_cache.insert("one".into(), vec![1, 2]);
    state.artwork.grid_cache.insert("album".into(), vec![1, 2]);
    assert!(
        !textamp::app::details::project(&state)
            .unwrap()
            .track_artwork
    );
}

#[test]
fn album_tracks_keep_the_release_order_and_have_no_sort_control() {
    let state = state(BrowseCategory::Library);
    assert!(textamp::app::view_options::for_column(&state, 0).is_none());
    assert!(
        !textamp::app::column_controls::controls(&state, 0)
            .contains(&textamp::app::column_controls::ColumnControl::Options)
    );
}

fn key(state: &mut AppState, code: KeyCode) -> Vec<Action> {
    key_input::handle_key(
        KeyEvent::new(code, KeyModifiers::NONE),
        state,
        &Config::default(),
    )
}
fn draw(state: &mut AppState) -> String {
    draw_at_height(state, 40)
}

fn draw_at_height(state: &mut AppState, height: u16) -> String {
    state.terminal_width = 120;
    state.terminal_height = height;
    let mut terminal = Terminal::new(TestBackend::new(120, height)).unwrap();
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
        let depth = state.browse_nav().unwrap().columns.len();
        assert!(key(&mut state, KeyCode::Enter).is_empty());
        assert_eq!(state.browse_nav().unwrap().columns.len(), depth);
        assert!(state.popups.text().is_some());
        state.popups.set_text(None);
        let screen = draw(&mut state);
        for text in [
            "First",
            "Second",
            "Artist: Artist",
            "Year: 2024",
            "Genres: Rock, Pop",
            "Tracks: 2",
            "Duration: 2:03",
        ] {
            assert!(screen.contains(text), "missing {text}");
        }
        let regions = state.hit_regions.miller_columns.as_ref().unwrap();
        let region = regions
            .columns
            .iter()
            .find(|r| r.col_idx == depth - 1)
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
        key(&mut state, KeyCode::Enter);
        assert_eq!(state.browse_nav().unwrap().columns.len(), depth);
        assert!(state.queue.tracks.is_empty());
    }
}
#[test]
fn default_album_grid_draws_two_covers_on_one_row_in_a_narrow_column() {
    let mut state = state(BrowseCategory::Library);
    state.tui_queue_visible = false;
    state.terminal_width = 80;
    state.terminal_height = 24;
    let albums = ["The First Release", "The Second Release"];
    state.artist_nav.columns = vec![
        BrowseColumn::new(
            "Releases",
            albums
                .into_iter()
                .map(|title| BrowseItem::Album {
                    key: title.into(),
                    title: title.into(),
                    artist: String::new(),
                    thumb: None,
                    year: None,
                    is_placeholder: false,
                })
                .collect(),
        ),
        BrowseColumn::new(
            "Other",
            vec![BrowseItem::AllTracks {
                scope: textamp::app::state::AllTracksScope::Library,
                thumb: None,
            }],
        ),
    ];
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| state.apply_render_feedback(textamp::ui::render(frame, &state)))
        .unwrap();
    let tiles = state
        .hit_regions
        .miller_item_rows
        .iter()
        .filter(|(column, _, _)| *column == 0)
        .map(|(_, area, _)| *area)
        .collect::<Vec<_>>();
    assert_eq!(tiles.len(), 2);
    assert_eq!(tiles[0].y, tiles[1].y);
    assert_eq!(tiles[0].right(), tiles[1].x);
}

#[test]
fn narrow_terminal_keeps_the_focused_album_wide_enough_to_browse() {
    let mut state = album_with_recommendations();
    let album = state.artist_nav.columns.pop().unwrap();
    state.artist_nav.columns = vec![
        BrowseColumn::new("Artists", vec![]),
        BrowseColumn::new("Releases", vec![]),
        album,
    ];
    state.artist_nav.focused_column = 2;
    state.tui_queue_visible = false;
    state.terminal_width = 80;
    state.terminal_height = 24;
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| state.apply_render_feedback(textamp::ui::render(frame, &state)))
        .unwrap();
    let regions = state.hit_regions.miller_columns.as_ref().unwrap();
    let album = regions
        .columns
        .iter()
        .find(|region| region.col_idx == 2)
        .unwrap();
    assert!(album.area.width >= 36);
    assert!(album.inner.height >= 8);
}

#[test]
fn wheel_scrolls_the_album_track_list_without_moving_its_selection() {
    let mut state = state(BrowseCategory::Library);
    let mut tracks = Vec::new();
    for n in 1..=60 {
        tracks.push(Track {
            rating_key: format!("track-{n}"),
            title: format!("Track {n:02}"),
            ..Default::default()
        });
    }
    let column = &mut state.artist_nav.columns[0];
    column.items = BrowseItem::from_tracks(&tracks);
    column.tracks = tracks;
    draw(&mut state);
    let region = state.hit_regions.miller_columns.as_ref().unwrap().columns[0].inner;
    assert!(region.height >= 10);
    mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: region.x + 3,
            row: region.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        &mut state,
    );
    let screen = draw(&mut state);
    assert!(!screen.contains("Track 01"));
    assert!(screen.contains("Track 02"));
    assert_eq!(state.artist_nav.columns[0].selected_index, 0);
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
    mouse_input::handle_mouse(event(details.x, details.y), &mut state);
    assert!(state.popups.text().is_some());
    state.popups.set_text(None);
    draw(&mut state);
    let region = state.hit_regions.miller_columns.as_ref().unwrap().columns[0].clone();
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
    assert!(state.artist_nav.focused().unwrap().has_album_details());
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

#[test]
fn artist_album_grid_scrolls_into_related_artists_after_the_last_album() {
    use textamp::app::state::ArtistInfoDetails;
    use textamp::library::models::Artist;

    let mut state = state(BrowseCategory::Library);
    let items = (0..25)
        .map(|i| BrowseItem::Album {
            key: format!("album-{i}"),
            title: format!("Release {i}"),
            artist: "Artist".into(),
            thumb: None,
            year: None,
            is_placeholder: false,
        })
        .collect();
    let mut column = BrowseColumn::new("Artist", items);
    column.artist_header = Some(ArtistInfoDetails {
        artist_key: "artist".into(),
        artist_name: "Artist".into(),
        request_id: 1,
        task: None,
        image_index: 0,
        document: textamp::services::biography::Biography {
            text: "Inline biography".into(),
            related_artists: (0..9)
                .map(|i| Artist {
                    rating_key: format!("related-{i}"),
                    title: format!("Related {i}"),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        },
    });
    state.artist_nav.columns[0] = column;
    draw(&mut state);
    let viewport = state.hit_regions.album_grids[0].2;
    let mut saw_boundary = false;
    for _ in 0..25 {
        draw(&mut state);
        let rows = &state.hit_regions.miller_item_rows;
        if let Some((tile, _, _)) = state.hit_regions.album_related.first()
            && let Some((_, last, index)) = rows.last()
        {
            assert_eq!(*index, 24, "recommendations precede the last album");
            assert_eq!(tile.y, last.bottom() + 1, "sections must share a viewport");
            saw_boundary = true;
        }
        state.scroll.scroll_cooldown = None;
        mouse_input::handle_mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: viewport.x,
                row: viewport.y,
                modifiers: KeyModifiers::NONE,
            },
            &mut state,
        );
    }
    assert!(saw_boundary);
    assert!(
        state
            .hit_regions
            .album_related
            .iter()
            .any(|(_, track, _)| { track.grandparent_rating_key.as_deref() == Some("related-8") })
    );
    let (_, columns, viewport) = state.hit_regions.album_grids[0];
    state.scroll.browse = Some((0, 25_usize.div_ceil(columns) * columns));
    // Two full bands, a heading, and a final band with portrait and one caption line.
    let compact_height = 40 - viewport.height + 20;
    draw_at_height(&mut state, compact_height);
    let tiles = &state.hit_regions.album_related;
    assert_eq!(tiles.len(), columns * 3);
    assert_eq!(tiles.last().unwrap().0.height, 5);
}
