mod common;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use textamp::app::action::SearchAction;
use textamp::app::event::UiEvent;
use textamp::app::handlers::{helpers, key_input};
use textamp::app::state::{ArtistBioPopup, View};
use textamp::app::{Action, AppState};
use textamp::config::Config;
use textamp::library::track::Track;
use textamp::services::biography::{BioImage, Biography};

fn popup() -> ArtistBioPopup {
    ArtistBioPopup {
        artist_key: "fixture-artist".into(),
        artist_name: "Tool".into(),
        document: Biography {
            text: "Current bio".into(),
            ..Default::default()
        },
        scroll: 0,
        google_focused: false,
        image_index: 0,
        task: None,
    }
}

#[test]
fn folder_biographies_require_embedded_artist_metadata_not_folder_names() {
    let mut state = AppState::new();
    state.view = View::NowPlaying;
    state.sources.active = textamp::app::sources::ActiveSource::Folder("local".into());
    state.queue.tracks = vec![Track::from_folder("local", "Tool/Album/01.flac")];
    state.queue.index = Some(0);
    assert!(helpers::get_artist_for_bio(&state).is_none());
    state.queue.tracks[0].original_title = Some("Tool".into());
    let actions = key_input::handle_key(
        KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE),
        &mut state,
        &Config::default(),
    );
    assert!(
        matches!(&actions[..], [Action::Search(SearchAction::ShowArtistBio { artist_key, artist_name })] if artist_key.is_empty() && artist_name == "Tool")
    );
}

#[test]
fn biography_photo_navigation_and_close_preserve_main_view() {
    let mut state = AppState::new();
    state.view = View::NowPlaying;
    let mut bio = popup();
    bio.document.images = (0..2)
        .map(|n| BioImage {
            key: n.to_string(),
            data: vec![],
            caption: n.to_string(),
        })
        .collect();
    state.popups.set_artist_bio(Some(bio));
    let config = Config::default();
    key_input::handle_key(
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        &mut state,
        &config,
    );
    assert_eq!(state.popups.artist_bio().unwrap().image_index, 1);
    key_input::handle_key(
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        &mut state,
        &config,
    );
    assert_eq!(state.popups.artist_bio().unwrap().image_index, 0);
    key_input::handle_key(
        KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE),
        &mut state,
        &config,
    );
    assert!(state.popups.artist_bio().is_none());
    assert_eq!(state.view, View::NowPlaying);
}

#[test]
fn biography_renders_readable_text_photos_and_shortcuts_at_small_sizes() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(20, 20)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    for (width, height) in [(120, 40), (60, 20), (12, 5)] {
        let mut state = AppState::new();
        state.artwork.mode = textamp::app::state::ArtworkMode::Braille;
        let mut bio = popup();
        bio.document.source_url = Some("https://en.wikipedia.org/wiki/Tool_(band)".into());
        bio.document.images.push(BioImage {
            key: "fixture".into(),
            data: png.get_ref().clone(),
            caption: "Band photo".into(),
        });
        state.popups.set_artist_bio(Some(bio));
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                textamp::ui::render(frame, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        if width == 120 {
            assert!(text.contains("Band photo"));
            assert!(text.contains("Current bio"));
            assert!(text.contains("B source"));
        }
    }
}

#[tokio::test]
async fn closing_bio_cancels_its_task_and_library_change_closes_popup() {
    let task = tokio::spawn(std::future::pending::<()>());
    let mut bio = popup();
    bio.task = Some(textamp::app::tasks::TaskLease::new(&task));
    let mut state = AppState::new();
    state.popups.set_artist_bio(Some(bio));
    state.advance_library_generation();
    assert!(state.popups.artist_bio().is_none());
    assert!(task.await.unwrap_err().is_cancelled());
}

#[test]
fn stale_biography_responses_cannot_replace_current_artist() {
    use textamp::app::dispatch::handle_core_event;
    let mut state = AppState::new();
    state.library_generation = 3;
    state.artist_bio_request_id = 4;
    state.popups.set_artist_bio(Some(popup()));

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    for (generation, request_id) in [(2, 4), (3, 3)] {
        handle_core_event(
            UiEvent::ArtistBioLoaded {
                generation,
                request_id,
                result: Err("Stale failure".into()),
            }
            .into(),
            &mut state,
            &tx,
        );
    }
    assert_eq!(
        state.popups.artist_bio().unwrap().document.text,
        "Current bio"
    );
}

#[tokio::test]
async fn compact_browser_button_and_keyboard_share_an_action_without_closing_biography() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend};
    use textamp::app::handlers::mouse_input;
    for loading in [false, true] {
        for (width, height) in [(120, 40), (60, 20), (12, 5)] {
            let mut state = AppState::new();
            let mut bio = popup();
            let task = tokio::spawn(std::future::pending::<()>());
            bio.task = loading.then(|| textamp::app::tasks::TaskLease::new(&task));
            if !loading {
                task.abort();
            }
            bio.document.text = "No biography available".into();
            state.popups.set_artist_bio(Some(bio));
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    state.apply_render_feedback(textamp::ui::render(frame, &state));
                })
                .unwrap();
            if width > 12 {
                let text: String = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect();
                assert!(text.contains("[ G ]"));
            }
            for key in [KeyCode::Char('g'), KeyCode::Char('G'), KeyCode::Enter] {
                if key == KeyCode::Enter {
                    assert!(
                        key_input::handle_key(
                            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                            &mut state,
                            &Config::default(),
                        )
                        .is_empty()
                    );
                    key_input::handle_key(
                        KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
                        &mut state,
                        &Config::default(),
                    );
                }
                let actions = key_input::handle_key(
                    KeyEvent::new(key, KeyModifiers::NONE),
                    &mut state,
                    &Config::default(),
                );
                assert!(matches!(
                    actions.as_slice(),
                    [Action::Search(SearchAction::SearchBiographyOnWeb)]
                ));
                assert!(state.popups.artist_bio().is_some());
            }
            if let Some(button) = state.hit_regions.biography_google {
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
                    [Action::Search(SearchAction::SearchBiographyOnWeb)]
                ));
                assert!(state.popups.artist_bio().is_some());
            }
            key_input::handle_key(
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                &mut state,
                &Config::default(),
            );
            terminal
                .draw(|frame| {
                    state.apply_render_feedback(textamp::ui::render(frame, &state));
                })
                .unwrap();
            assert!(state.hit_regions.biography_google.is_none());
            assert!(state.hit_regions.biography_text.is_none());
        }
    }
}

#[test]
fn biography_arrows_focus_a_visibly_selected_button_and_return_to_text() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend};
    use textamp::app::handlers::mouse_input;
    for long in [false, true] {
        let mut state = AppState::new();
        let mut bio = popup();
        if long {
            bio.document.text = "Line of biography\n".repeat(100);
        }
        state.popups.set_artist_bio(Some(bio));
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|f| state.apply_render_feedback(textamp::ui::render(f, &state)))
            .unwrap();
        let button = state.hit_regions.biography_google.unwrap();
        let unfocused = terminal.backend().buffer()[(button.x, button.y)].style();
        let max_scroll = state
            .hit_regions
            .biography_text
            .as_ref()
            .unwrap()
            .max_scroll();
        assert_eq!(max_scroll > 0, long);
        for _ in 0..max_scroll {
            assert!(
                key_input::handle_key(
                    KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                    &mut state,
                    &Config::default()
                )
                .is_empty()
            );
            assert!(!state.popups.artist_bio().unwrap().google_focused);
        }
        key_input::handle_key(
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            &mut state,
            &Config::default(),
        );
        assert!(state.popups.artist_bio().unwrap().google_focused);
        terminal
            .draw(|f| state.apply_render_feedback(textamp::ui::render(f, &state)))
            .unwrap();
        assert_ne!(
            terminal.backend().buffer()[(button.x, button.y)].style(),
            unfocused
        );
        let actions = key_input::handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut state,
            &Config::default(),
        );
        assert!(matches!(
            actions.as_slice(),
            [Action::Search(SearchAction::SearchBiographyOnWeb)]
        ));
        key_input::handle_key(
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            &mut state,
            &Config::default(),
        );
        let bio = state.popups.artist_bio().unwrap();
        assert!(!bio.google_focused);
        assert_eq!(bio.scroll, max_scroll);
        for key in [KeyCode::Tab, KeyCode::BackTab] {
            key_input::handle_key(
                KeyEvent::new(key, KeyModifiers::NONE),
                &mut state,
                &Config::default(),
            );
        }
        assert!(!state.popups.artist_bio().unwrap().google_focused);
        key_input::handle_key(
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
            &mut state,
            &Config::default(),
        );
        let text = state.hit_regions.biography_text.as_ref().unwrap().area;
        assert!(
            mouse_input::handle_mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: text.x,
                    row: text.y,
                    modifiers: KeyModifiers::NONE
                },
                &mut state
            )
            .is_empty()
        );
        let bio = state.popups.artist_bio().unwrap();
        assert!(!bio.google_focused);
        assert_eq!(bio.scroll, max_scroll);
    }
}

#[test]
fn google_search_encodes_the_artist_as_one_query_parameter() {
    let artist = "AC/DC & Björk #1";
    let url =
        reqwest::Url::parse(&textamp::services::biography::google_search_url(artist)).unwrap();
    assert_eq!(url.host_str(), Some("www.google.com"));
    assert_eq!(url.path(), "/search");
    let params: Vec<_> = url.query_pairs().collect();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].0, "q");
    assert_eq!(params[0].1, format!("{artist} music artist biography"));
    assert!(url.fragment().is_none());
}

#[tokio::test]
async fn biography_search_reaches_the_host_with_the_artist_query() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut state = AppState::new();
    let bio = popup();
    let expected = state.web_search_provider.artist_url(&bio.artist_name);
    state.popups.set_artist_bio(Some(bio));
    let actions = textamp::app::handlers::dispatch_search::dispatch(
        &tx,
        SearchAction::SearchBiographyOnWeb,
        &mut state,
    )
    .unwrap();
    assert!(actions.is_empty());
    let effect = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("browser handoff");
    assert!(matches!(effect, Some(textamp::app::Event::Effect(
        Action::System(textamp::app::action::SystemAction::OpenBrowser(url)))) if url == expected));
}

#[tokio::test]
async fn closed_biography_cannot_launch_a_browser() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut state = AppState::new();
    textamp::app::handlers::dispatch_search::dispatch(
        &tx,
        SearchAction::SearchBiographyOnWeb,
        &mut state,
    )
    .unwrap();
    assert!(rx.try_recv().is_err());
}

#[test]
fn artist_info_radio_button_and_key_target_the_biography_artist() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend};
    use textamp::app::handlers::mouse_input;
    let mut state = common::navidrome_state();
    state.popups.set_artist_bio(Some(popup()));
    state.terminal_width = 120;
    state.terminal_height = 40;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|frame| state.apply_render_feedback(textamp::ui::render(frame, &state)))
        .unwrap();
    let region = state.hit_regions.biography_radio.unwrap();
    let actions = mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: region.x,
            row: region.y,
            modifiers: KeyModifiers::NONE,
        },
        &mut state,
    );
    assert!(
        matches!(actions.as_slice(),[Action::Radio(textamp::app::action::RadioAction::StartArtistRadio{key,title})] if key=="fixture-artist" && title=="Tool")
    );
    state.popups.set_artist_bio(Some(popup()));
    let actions = key_input::handle_key(
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
        &mut state,
        &Config::default(),
    );
    assert!(
        matches!(actions.as_slice(),[Action::Radio(textamp::app::action::RadioAction::StartArtistRadio{key,..})] if key=="fixture-artist")
    );
    assert!(state.popups.artist_bio().is_none());
    assert!(state.queue.tracks.is_empty());
}

#[tokio::test]
async fn artist_info_column_owns_cancellation_and_rejects_stale_results() {
    use textamp::app::{
        dispatch::handle_core_event,
        state::{ArtistInfoDetails, BrowseColumn, BrowseDetails, BrowseItem},
    };
    let mut state = common::navidrome_state();
    state.library_generation = 7;
    let task = tokio::spawn(std::future::pending::<()>());
    let mut details = BrowseColumn::new("Artist Info", vec![]);
    details.details = Some(BrowseDetails::Artist(ArtistInfoDetails {
        artist_key: "artist".into(),
        artist_name: "Artist".into(),
        request_id: 42,
        document: Biography {
            text: "Current info".into(),
            ..Default::default()
        },
        image_index: 0,
        task: Some(textamp::app::tasks::TaskLease::new(&task)),
    }));
    state.artist_nav.columns = vec![
        BrowseColumn::new(
            "albums",
            vec![BrowseItem::ArtistInfo {
                artist_key: "artist".into(),
                artist_name: "Artist".into(),
                thumb: None,
            }],
        ),
        details,
    ];
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    for (generation, request_id, key) in [(6, 42, "artist"), (7, 41, "artist"), (7, 42, "other")] {
        handle_core_event(
            UiEvent::ArtistInfoLoaded {
                generation,
                request_id,
                artist_key: key.into(),
                result: Err("Stale failure".into()),
            }
            .into(),
            &mut state,
            &tx,
        );
    }
    assert!(
        matches!(&state.artist_nav.columns[1].details,Some(BrowseDetails::Artist(info)) if info.document.text=="Current info" && info.task.is_some())
    );
    assert!(state.popups.artist_bio().is_none());
    state.advance_library_generation();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(state.artist_nav.columns.len(), 1);
    handle_core_event(
        UiEvent::ArtistInfoLoaded {
            generation: 7,
            request_id: 42,
            artist_key: "artist".into(),
            result: Err("Late completion".into()),
        }
        .into(),
        &mut state,
        &tx,
    );
    assert_eq!(state.artist_nav.columns.len(), 1);
}
