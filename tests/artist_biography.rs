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
        artist_name: "Tool".into(),
        document: Biography {
            text: "Current bio".into(),
            ..Default::default()
        },
        scroll: 0,
        google_focused: false,
        loading: false,
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
    state.popups.artist_bio = Some(bio);
    let config = Config::default();
    key_input::handle_key(
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        &mut state,
        &config,
    );
    assert_eq!(state.popups.artist_bio.as_ref().unwrap().image_index, 1);
    key_input::handle_key(
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        &mut state,
        &config,
    );
    assert_eq!(state.popups.artist_bio.as_ref().unwrap().image_index, 0);
    key_input::handle_key(
        KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE),
        &mut state,
        &config,
    );
    assert!(state.popups.artist_bio.is_none());
    assert_eq!(state.view, View::NowPlaying);
}

#[test]
fn biography_renders_readable_text_photos_and_shortcuts_at_small_sizes() {
    use ratatui::{backend::TestBackend, Terminal};
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
        state.popups.artist_bio = Some(bio);
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
    state.popups.artist_bio = Some(bio);
    state.advance_library_generation();
    assert!(state.popups.artist_bio.is_none());
    assert!(task.await.unwrap_err().is_cancelled());
}

#[test]
fn stale_biography_responses_cannot_replace_current_artist() {
    use textamp::app::dispatch::handle_core_event;
    let mut state = AppState::new();
    state.library_generation = 3;
    state.artist_bio_request_id = 4;
    state.popups.artist_bio = Some(popup());

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
        state.popups.artist_bio.unwrap().document.text,
        "Current bio"
    );
}

#[test]
fn google_button_and_keyboard_use_the_same_action_without_closing_biography() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use ratatui::{backend::TestBackend, Terminal};
    use textamp::app::handlers::mouse_input;
    for loading in [false, true] {
        for (width, height) in [(120, 40), (60, 20), (12, 5)] {
            let mut state = AppState::new();
            let mut bio = popup();
            bio.loading = loading;
            bio.document.text = "No biography available from Navidrome or Wikipedia.".into();
            state.popups.artist_bio = Some(bio);
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
                assert!(text.contains("[ Search Google ]"));
            }
            for key in [KeyCode::Char('g'), KeyCode::Char('G'), KeyCode::Enter] {
                if key == KeyCode::Enter {
                    assert!(key_input::handle_key(
                        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                        &mut state,
                        &Config::default(),
                    )
                    .is_empty());
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
                    [Action::Search(SearchAction::SearchBiographyOnGoogle)]
                ));
                assert!(state.popups.artist_bio.is_some());
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
                    [Action::Search(SearchAction::SearchBiographyOnGoogle)]
                ));
                assert!(state.popups.artist_bio.is_some());
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
    use ratatui::{backend::TestBackend, Terminal};
    use textamp::app::handlers::mouse_input;
    for long in [false, true] {
        let mut state = AppState::new();
        let mut bio = popup();
        if long {
            bio.document.text = "Line of biography\n".repeat(100);
        }
        state.popups.artist_bio = Some(bio);
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
            assert!(key_input::handle_key(
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                &mut state,
                &Config::default()
            )
            .is_empty());
            assert!(!state.popups.artist_bio.as_ref().unwrap().google_focused);
        }
        key_input::handle_key(
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            &mut state,
            &Config::default(),
        );
        assert!(state.popups.artist_bio.as_ref().unwrap().google_focused);
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
            [Action::Search(SearchAction::SearchBiographyOnGoogle)]
        ));
        key_input::handle_key(
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            &mut state,
            &Config::default(),
        );
        let bio = state.popups.artist_bio.as_ref().unwrap();
        assert!(!bio.google_focused);
        assert_eq!(bio.scroll, max_scroll);
        for key in [KeyCode::Tab, KeyCode::BackTab] {
            key_input::handle_key(
                KeyEvent::new(key, KeyModifiers::NONE),
                &mut state,
                &Config::default(),
            );
        }
        assert!(!state.popups.artist_bio.as_ref().unwrap().google_focused);
        key_input::handle_key(
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
            &mut state,
            &Config::default(),
        );
        let text = state.hit_regions.biography_text.as_ref().unwrap().area;
        assert!(mouse_input::handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: text.x,
                row: text.y,
                modifiers: KeyModifiers::NONE
            },
            &mut state
        )
        .is_empty());
        let bio = state.popups.artist_bio.as_ref().unwrap();
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
async fn closed_biography_cannot_launch_a_browser() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut state = AppState::new();
    textamp::app::handlers::dispatch_search::dispatch(
        &tx,
        SearchAction::SearchBiographyOnGoogle,
        &mut state,
    )
    .await
    .unwrap();
    assert!(rx.try_recv().is_err());
}
