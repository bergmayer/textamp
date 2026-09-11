use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::TestBackend, Terminal};
use std::time::Instant;
use textamp::app::handlers::{key_input, mouse_input};
use textamp::app::{
    state::{PlayStatus, View, VisualizerTab},
    AppState,
};
use textamp::media::SpectrogramData;

#[test]
fn all_six_visualizers_render_identically_for_each_library_provider() {
    use textamp::app::sources::{navidrome::Session, ActiveSource};
    use textamp::navidrome::{Client, Source};
    let source = Source {
        id: "render".into(),
        name: "render".into(),
        url: "http://127.0.0.1:1".into(),
        username: "fixture".into(),
        libraries: vec![],
    };
    let session = Session {
        client: Client::new(&source, "fixture".into(), None).unwrap(),
        source,
        extensions: Default::default(),
    };
    for tab in VisualizerTab::ALL {
        let mut state = fixture(tab);
        state.waveform.data = Some(textamp::media::generate_waveform_from_pcm(
            "fixture".into(),
            60000,
            &[0.1, 0.5, 0.3, 1.0],
        ));
        state
            .vectorscope_buffer
            .extend([(0.3, 0.7), (-0.2, 0.4), (0.8, -0.5)]);
        let output = render(&mut state, 120, 42);
        let expected_area = state
            .hit_regions
            .now_playing_content
            .as_ref()
            .unwrap()
            .visualizer_content_area;
        let visualizer_y = state
            .hit_regions
            .now_playing_content
            .as_ref()
            .unwrap()
            .visualizer_tab_area
            .y;
        let expected = output
            .lines()
            .skip(visualizer_y as usize)
            .collect::<Vec<_>>()
            .join("\n");
        for active in [
            ActiveSource::Folder("local".into()),
            ActiveSource::Folder("webdav".into()),
            ActiveSource::Navidrome(Box::new(session.clone())),
        ] {
            state.sources.active = active;
            let output = render(&mut state, 120, 42);
            assert_eq!(
                state
                    .hit_regions
                    .now_playing_content
                    .as_ref()
                    .unwrap()
                    .visualizer_content_area,
                expected_area
            );
            assert_eq!(
                output
                    .lines()
                    .skip(visualizer_y as usize)
                    .collect::<Vec<_>>()
                    .join("\n"),
                expected,
                "{tab:?}"
            );
            // Provider capability differences belong in the sidebar, not in
            // the visualizers. Validate its actual rendered action rows too.
            let rendered: Vec<_> = state
                .hit_regions
                .now_playing_sidebar
                .as_ref()
                .unwrap()
                .iter()
                .map(|(_, button)| *button)
                .collect();
            assert_eq!(rendered, state.now_playing_sidebar_buttons());
        }
    }
}

fn fixture(tab: VisualizerTab) -> AppState {
    let mut state = AppState::new();
    state.view = View::NowPlaying;
    state.visualizer_tab = tab;
    state.playback.status = PlayStatus::Playing;
    state.playback.position_ms = 18_000;
    state.playback.duration_ms = 60_000;
    state.spectrogram.data = Some(SpectrogramData {
        track_key: "fixture".into(),
        duration_ms: 60_000,
        bins_per_frame: 48,
        frame_count: 600,
        frames_per_second: 10.0,
        sample_rate: 44_100,
        version: 6,
        created_at: 0,
        frames: (0..600 * 48)
            .map(|i| {
                let band = (i % 48) as f32;
                let time = (i / 48) as f32;
                ((band * 0.3 + time * 0.1).sin().abs() * (230.0 - band * 3.0)) as u8
            })
            .collect(),
    });
    let now = Instant::now();
    for i in 0..120 {
        let level = 0.1 + (i as f32 * 0.12).sin().abs() * 0.5;
        state.studio_meters.update(
            &[(level, level * 0.7), (-level, -level * 0.7)],
            now + std::time::Duration::from_millis(i * 100),
            1,
        );
    }
    state
}

fn render(state: &mut AppState, width: u16, height: u16) -> String {
    state.terminal_width = width;
    state.terminal_height = height;
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut feedback = None;
    terminal
        .draw(|frame| feedback = Some(textamp::ui::render(frame, state)))
        .unwrap();
    state.hit_regions = feedback.unwrap().hit_regions;
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                + "\n"
        })
        .collect()
}

#[test]
fn new_modes_render_with_data_without_mutating_meter_history() {
    for tab in [VisualizerTab::Landscape, VisualizerTab::Meters] {
        let mut state = fixture(tab);
        let history = state.studio_meters.history.clone();
        for (w, h) in [(40, 10), (60, 24), (120, 42), (200, 60)] {
            let output = render(&mut state, w, h);
            assert_eq!(state.studio_meters.history, history);
            assert!(!output.contains("spectral trail"));
            assert!(!output.contains("Recent sampled"));
            if w == 120 {
                if tab == VisualizerTab::Landscape {
                    assert!(output
                        .chars()
                        .any(|c| ('\u{2801}'..='\u{28ff}').contains(&c)));
                } else {
                    for caption in [
                        "S T U D I O",
                        "sampled PCM",
                        "Correlation",
                        "opposite",
                        "mono",
                        "RMS",
                        "peak",
                        "hold",
                    ] {
                        assert!(!output.contains(caption), "{caption}");
                    }
                    assert!(output.contains('█'));
                    assert!(output.contains('┼'));
                }
                if std::env::var_os("TEXTAMP_VISUALIZER_PREVIEW").is_some() {
                    println!("{tab:?}\n{output}");
                }
            }
        }
    }
}

#[test]
fn keyboard_cycles_all_six_modes_and_mouse_hits_rendered_tabs() {
    let mut state = fixture(VisualizerTab::Waveform);
    state.visualizer_tab_focused = true;
    for expected in VisualizerTab::ALL.into_iter().cycle().skip(1).take(6) {
        key_input::handle_key(
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            &mut state,
            &textamp::config::Config::default(),
        );
        assert_eq!(state.visualizer_tab, expected);
    }
    for view in [View::Queue, View::NowPlaying] {
        for selected in VisualizerTab::ALL {
            for width in [40, 60, 120] {
                state.view = view;
                state.visualizer_tab = selected;
                render(&mut state, width, 42);
                let area = state
                    .hit_regions
                    .now_playing_content
                    .as_ref()
                    .unwrap()
                    .visualizer_tab_area;
                let tabs = selected.visible_tabs(area.width);
                let mut x = area.x;
                for (tab, label) in tabs {
                    // Restore the selected tab whose layout produced these coordinates.
                    state.visualizer_tab = selected;
                    mouse_input::handle_mouse(
                        MouseEvent {
                            kind: MouseEventKind::Down(MouseButton::Left),
                            column: x + 1,
                            row: area.y,
                            modifiers: KeyModifiers::NONE,
                        },
                        &mut state,
                    );
                    assert_eq!(state.visualizer_tab, tab, "{view:?} {width} {label}");
                    x += label.len() as u16 + 5;
                }
            }
        }
    }
}

#[test]
fn new_canvases_do_not_seek_and_missing_data_is_explicit() {
    for tab in [VisualizerTab::Landscape, VisualizerTab::Meters] {
        let mut state = fixture(tab);
        for view in [View::Queue, View::NowPlaying] {
            state.view = view;
            render(&mut state, 120, 42);
            let area = state
                .hit_regions
                .now_playing_content
                .as_ref()
                .unwrap()
                .visualizer_content_area;
            let actions = mouse_input::handle_mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: area.x + 3,
                    row: area.y + 1,
                    modifiers: KeyModifiers::NONE,
                },
                &mut state,
            );
            assert!(actions.is_empty(), "{view:?} {tab:?}: {actions:?}");
            assert!(state.seek_drag.is_none());
        }
        state.spectrogram.data = None;
        state.studio_meters = Default::default();
        let output = render(&mut state, 120, 42);
        assert!(output.contains(if tab == VisualizerTab::Landscape {
            "No spectral data"
        } else {
            "No live PCM"
        }));
    }
}

#[test]
fn individual_new_renderers_handle_tiny_canvases() {
    for tab in [VisualizerTab::Landscape, VisualizerTab::Meters] {
        let state = fixture(tab);
        for (width, height) in [(0, 0), (1, 1), (10, 3), (30, 7)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    if tab == VisualizerTab::Landscape {
                        textamp::ui::screens::visualizers::landscape(frame, &state, frame.area());
                    } else {
                        textamp::ui::screens::visualizers::meters(frame, &state, frame.area());
                    }
                })
                .unwrap();
        }
    }
}
