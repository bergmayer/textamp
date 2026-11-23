use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend, style::Color};
use textamp::app::{
    Action, AppState,
    action::NavigationAction,
    handlers::{dispatch_navigation, key_input, mouse_input},
    state::{NowPlayingFocus, PaletteCommandKind, PlayStatus, View, VisualizerTab},
};
use textamp::config::Config;

fn key(code: KeyCode, state: &mut AppState) -> Vec<Action> {
    key_input::handle_key(
        KeyEvent::new(code, KeyModifiers::NONE),
        state,
        &Config::default(),
    )
}
fn draw(state: &mut AppState, width: u16, height: u16) -> ratatui::buffer::Buffer {
    state.terminal_width = width;
    state.terminal_height = height;
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| state.apply_render_feedback(textamp::ui::render(frame, state)))
        .unwrap();
    terminal.backend().buffer().clone()
}
fn text(buffer: &ratatui::buffer::Buffer) -> String {
    buffer.content.iter().map(|cell| cell.symbol()).collect()
}
#[tokio::test]
async fn shortcut_palette_and_escape_preserve_navigation_and_playback() {
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    for view in [
        View::Browse,
        View::Queue,
        View::NowPlaying,
        View::Settings,
        View::Help,
    ] {
        let mut state = AppState::new();
        state.view = view;
        assert!(
            textamp::app::command_palette::materialize_entries(&state)
                .iter()
                .any(|entry| matches!(entry.command, PaletteCommandKind::ToggleFullscreen))
        );
        state.now_playing_focus = NowPlayingFocus::Tools;
        state.playback.status = PlayStatus::Playing;
        state.playback.position_ms = 1234;
        state.visualizer_tab_focused = true;
        state.select_mode = true;
        state.track_pane_focused = true;
        state.scroll.queue = Some(8);
        for _ in 0..3 {
            assert!(matches!(
                key(KeyCode::F(7), &mut state).as_slice(),
                [Action::Navigation(NavigationAction::ToggleFullscreen)]
            ));
            dispatch_navigation::dispatch(&tx, NavigationAction::ToggleFullscreen, &mut state)
                .unwrap();
            assert!(state.fullscreen.is_open());
            assert_eq!(state.view, view);
            assert!(matches!(
                key(KeyCode::Esc, &mut state).as_slice(),
                [Action::Navigation(NavigationAction::ToggleFullscreen)]
            ));
            dispatch_navigation::dispatch(&tx, NavigationAction::ToggleFullscreen, &mut state)
                .unwrap();
            assert!(!state.fullscreen.is_open());
            assert_eq!(state.view, view);
            assert_eq!(state.now_playing_focus, NowPlayingFocus::Tools);
            assert_eq!(state.playback.status, PlayStatus::Playing);
            assert_eq!(state.playback.position_ms, 1234);
            assert!(state.visualizer_tab_focused);
            assert!(state.select_mode);
            assert!(state.track_pane_focused);
            assert_eq!(state.scroll.queue, Some(8));
        }
        state.view = View::Browse;
        state.fullscreen = textamp::app::state::FullscreenView::Visualizer;
        dispatch_navigation::dispatch(
            &tx,
            NavigationAction::set_category(state.browse_category),
            &mut state,
        )
        .unwrap();
        assert!(!state.fullscreen.is_open());
        dispatch_navigation::dispatch(&tx, NavigationAction::FullscreenArtwork, &mut state)
            .unwrap();
        assert_eq!(textamp::app::fullscreen::mode(&state), 0);
        assert!(state.fullscreen.is_artwork());
        dispatch_navigation::dispatch(&tx, NavigationAction::ToggleFullscreen, &mut state).unwrap();
        dispatch_navigation::dispatch(&tx, NavigationAction::ToggleFullscreen, &mut state).unwrap();
        assert!(state.fullscreen.is_open());
        assert!(
            !state.fullscreen.is_artwork(),
            "the visualizer command must open a visualizer after artwork"
        );
        let entries = textamp::app::command_palette::materialize_entries(&state);
        assert_eq!(
            entries
                .iter()
                .filter(|e| matches!(e.command, PaletteCommandKind::ToggleFullscreen))
                .count(),
            1
        );
        assert!(matches!(
            textamp::app::command_palette::run(PaletteCommandKind::FullscreenArtwork, &mut state)
                .as_slice(),
            [Action::Navigation(NavigationAction::FullscreenArtwork)]
        ));
    }
}
#[test]
fn modes_resize_missing_art_and_mouse_never_seek() {
    let mut state = AppState::new();
    state.fullscreen = textamp::app::state::FullscreenView::Visualizer;
    for size in [(0, 0), (1, 1), (20, 3), (80, 24), (160, 48), (40, 10)] {
        for i in 0..=VisualizerTab::ALL.len() as u8 {
            key(KeyCode::Char((b'1' + i) as char), &mut state);
            assert_eq!(textamp::app::fullscreen::mode(&state), i as usize);
            assert_eq!(state.fullscreen.is_artwork(), i == 0);
            if i > 0 {
                assert_eq!(state.visualizer_tab, VisualizerTab::ALL[i as usize - 1]);
            }
            let buffer = draw(&mut state, size.0, size.1);
            if i == 0 && size.0 >= 40 && size.1 >= 10 {
                assert!(text(&buffer).contains("No artwork"));
            }
            assert!(
                mouse_input::handle_mouse(
                    MouseEvent {
                        kind: MouseEventKind::Down(MouseButton::Left),
                        column: 10,
                        row: 5,
                        modifiers: KeyModifiers::NONE
                    },
                    &mut state
                )
                .is_empty()
            );
            assert!(state.seek_drag.is_none());
        }
    }
    key(KeyCode::Right, &mut state);
    assert_eq!(textamp::app::fullscreen::mode(&state), 0);
    key(KeyCode::Left, &mut state);
    assert_eq!(
        textamp::app::fullscreen::mode(&state),
        VisualizerTab::ALL.len()
    );
    draw(&mut state, 120, 30);
    let (area, index) = state.hit_regions.fullscreen_modes[1];
    mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        },
        &mut state,
    );
    assert_eq!(textamp::app::fullscreen::mode(&state), index);
}
#[test]
fn palette_cancel_returns_to_viewer() {
    let mut state = AppState::new();
    state.fullscreen = textamp::app::state::FullscreenView::Visualizer;
    key(KeyCode::Char(':'), &mut state);
    assert!(state.palette.open);
    key(KeyCode::Esc, &mut state);
    assert!(!state.palette.open);
    assert!(state.fullscreen.is_open());
}
#[test]
fn nord_focus_borders_match_including_sidebar_and_errors_keep_their_color() {
    let mut state = AppState::new();
    state.theme = textamp::ui::ThemeName::Nord;
    state.view = View::Queue;
    let pink = Color::Rgb(196, 145, 162);
    assert_ne!(
        textamp::ui::theme::ThemeColors::for_theme(textamp::app::theme::ThemeName::Nord).error,
        pink
    );
    for focus in [
        NowPlayingFocus::Tools,
        NowPlayingFocus::Tracks,
        NowPlayingFocus::Artwork,
    ] {
        state.now_playing_focus = focus;
        let buffer = draw(&mut state, 120, 40);
        evidence(&buffer, &format!("nord-{focus:?}"));
        assert!(
            buffer
                .content
                .iter()
                .any(|c| c.fg == pink && c.symbol() == "┌"),
            "{focus:?}"
        );
    }
    state.view = View::NowPlaying;
    state.visualizer_tab_focused = true;
    let buffer = draw(&mut state, 120, 40);
    assert!(
        buffer
            .content
            .iter()
            .any(|c| c.fg == pink && ["┌", "╭", "╔"].contains(&c.symbol()))
    );
}
#[test]
fn artwork_canvas_reuses_decoding_and_analysis_survives_resize() {
    textamp::ui::screens::now_playing::init_artwork_renderer(
        ratatui_image::picker::Picker::halfblocks(),
    );
    let mut state = AppState::new();
    state.fullscreen = textamp::app::state::FullscreenView::Visualizer;
    state.fullscreen = textamp::app::state::FullscreenView::Artwork;
    state.artwork.mode = textamp::app::state::ArtworkMode::Halfblocks;
    let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(96, 96, |x, y| {
        image::Rgb([(x * 2) as u8, (y * 2) as u8, 130])
    }));
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    state.artwork.current_data = Some(png.into_inner());
    state.artwork.current_thumb = Some("fullscreen-fixture".into());
    for size in [(80, 24), (120, 40), (40, 12)] {
        let buffer = draw(&mut state, size.0, size.1);
        evidence(&buffer, &format!("artwork-{}x{}", size.0, size.1));
        assert!(!text(&buffer).contains("No artwork"));
        assert!(!text(&buffer).contains("Image load failed"));
        let middle = size.1 / 2;
        assert_eq!(
            buffer[(0, middle)].bg,
            textamp::ui::theme().colors.bg_primary
        );
        assert_ne!(
            buffer[(size.0 / 2, middle)].bg,
            textamp::ui::theme().colors.bg_primary
        );
        assert!(
            buffer
                .content
                .iter()
                .any(|cell| cell.symbol() == "▀" || cell.symbol() == "▄")
        );
    }
    state.fullscreen = textamp::app::state::FullscreenView::Visualizer;
    state
        .waveform
        .succeed(textamp::media::generate_waveform_from_pcm(
            "fixture".into(),
            60000,
            &[0.1, 0.5, 0.3, 1.0],
        ));
    for size in [(80, 24), (120, 40), (40, 12)] {
        draw(&mut state, size.0, size.1);
    }
    assert!(state.waveform.data().is_some());
}

// Optional render evidence uses actual Ratatui cells; tests require no screenshots.
fn evidence(buffer: &ratatui::buffer::Buffer, name: &str) {
    if let Ok(dir) = std::env::var("TEXTAMP_RENDER_EVIDENCE") {
        let dir = std::path::Path::new(&dir);
        std::fs::create_dir_all(dir).unwrap();
        let cells = buffer.content.iter().map(|cell| serde_json::json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg)})).collect::<Vec<_>>();
        std::fs::write(dir.join(format!("{name}.json")),serde_json::to_vec(&serde_json::json!({"width":buffer.area.width,"height":buffer.area.height,"cells":cells})).unwrap()).unwrap();
    }
}
