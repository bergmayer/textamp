use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::TestBackend, layout::Rect, Terminal};
use textamp::app::action::PlaybackAction;
use textamp::app::handlers::mouse_input::handle_mouse;
use textamp::app::state::{PlayStatus, View, VisualizerTab};
use textamp::app::{Action, AppState};

fn render(state: &mut AppState) {
    state.terminal_width = 120;
    state.terminal_height = 42;
    let mut terminal = Terminal::new(TestBackend::new(120, 42)).unwrap();
    terminal
        .draw(|frame| {
            state.hit_regions = textamp::ui::render(frame, state).hit_regions;
        })
        .unwrap();
}

fn mouse(state: &mut AppState, kind: MouseEventKind, x: u16, y: u16) -> Vec<Action> {
    handle_mouse(
        MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        },
        state,
    )
}

fn assert_seek(actions: Vec<Action>, position: u64) {
    assert!(
        matches!(actions.as_slice(), [Action::Playback(PlaybackAction::Seek(ms))] if *ms == position),
        "{actions:?}"
    );
}

fn exercise_drag(state: &mut AppState, area: Rect) {
    // Start far from the old playhead. Every click, not just the thumb, grabs.
    let middle = area.width / 2;
    assert_seek(
        mouse(
            state,
            MouseEventKind::Down(MouseButton::Left),
            area.x + middle,
            area.y,
        ),
        60_000 * middle as u64 / (area.width - 1) as u64,
    );
    assert_eq!(state.seek_drag, Some(area));
    // Crossing rows must not switch between the waveform and transport scales.
    assert_seek(
        mouse(state, MouseEventKind::Drag(MouseButton::Left), area.x, 0),
        0,
    );
    assert_seek(
        mouse(
            state,
            MouseEventKind::Drag(MouseButton::Left),
            area.right() - 1,
            0,
        ),
        60_000,
    );
    // Release is authoritative, even if intermediate drag events were dropped.
    assert_seek(
        mouse(state, MouseEventKind::Up(MouseButton::Left), area.x, 41),
        0,
    );
    assert!(state.seek_drag.is_none());
    assert!(mouse(
        state,
        MouseEventKind::Drag(MouseButton::Left),
        area.x + 2,
        area.y
    )
    .is_empty());
}

#[test]
fn scrubber_click_and_drag_work_in_every_visualizer_and_split_layout() {
    for view in [View::Browse, View::Queue, View::NowPlaying] {
        for tall in [false, true] {
            for tab in VisualizerTab::ALL {
                let mut state = AppState::new();
                state.view = view;
                state.tall_mode = tall;
                state.visualizer_tab = tab;
                state.playback.duration_ms = 60_000;
                state.playback.status = PlayStatus::Playing;
                render(&mut state);
                let area = state.hit_regions.transport.as_ref().unwrap().seekbar;
                exercise_drag(&mut state, area);
            }
        }
    }
}

#[test]
fn visible_waveform_seeks_regardless_of_keyboard_focus() {
    for view in [View::Browse, View::Queue, View::NowPlaying] {
        let mut state = AppState::new();
        state.view = view;
        state.tall_mode = view == View::Browse;
        state.visualizer_tab = VisualizerTab::Waveform;
        state.playback.duration_ms = 60_000;
        state.playback.status = PlayStatus::Playing;
        render(&mut state);
        let area = state
            .hit_regions
            .now_playing_content
            .as_ref()
            .unwrap()
            .visualizer_content_area;
        exercise_drag(&mut state, area);
    }
}

#[test]
fn long_track_scrubber_hitbox_matches_the_rendered_bar() {
    for position in [0, 6_123_000, 61_234_000] {
        for filter in [false, true] {
            let mut state = AppState::new();
            state.view = View::Browse;
            state.terminal_width = 160;
            state.terminal_height = 42;
            state.list_filter.active = filter;
            state.playback.duration_ms = 65_000_000;
            state.playback.position_ms = position;
            let mut terminal = Terminal::new(TestBackend::new(160, 42)).unwrap();
            terminal
                .draw(|frame| state.hit_regions = textamp::ui::render(frame, &state).hit_regions)
                .unwrap();
            let area = state.hit_regions.transport.as_ref().unwrap().seekbar;
            let buffer = terminal.backend().buffer();
            assert!(matches!(buffer[(area.x, area.y)].symbol(), "●" | "━" | "─"));
            assert!(matches!(
                buffer[(area.right() - 1, area.y)].symbol(),
                "●" | "━" | "─"
            ));
            assert_eq!(buffer[(area.x - 1, area.y)].symbol(), " ");
            assert_eq!(buffer[(area.right(), area.y)].symbol(), " ");
        }
    }
}

#[test]
fn overlays_and_unknown_duration_do_not_start_a_seek() {
    let mut state = AppState::new();
    state.view = View::NowPlaying;
    render(&mut state);
    let area = state
        .hit_regions
        .now_playing_content
        .as_ref()
        .unwrap()
        .visualizer_content_area;
    assert!(mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        area.x,
        area.y
    )
    .is_empty());
    assert!(state.seek_drag.is_none());
    state.playback.duration_ms = 60_000;
    state.palette.open = true;
    render(&mut state);
    assert!(mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        area.x,
        area.y
    )
    .is_empty());
    assert!(state.seek_drag.is_none());
}

#[test]
fn spectrogram_leaves_text_and_a_blank_row_above_its_canvas_untouched() {
    for (width, height) in [(40, 12), (80, 24), (120, 42)] {
        let mut state = AppState::new();
        state.view = View::NowPlaying;
        state.visualizer_tab = VisualizerTab::Spectrogram;
        state.playback.duration_ms = 60_000;
        state.playback.position_ms = 30_000;
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| state.hit_regions = textamp::ui::render(frame, &state).hit_regions)
            .unwrap();
        let before = terminal.backend().buffer().clone();
        let regions = state.hit_regions.now_playing_content.as_ref().unwrap();
        let canvas = regions.visualizer_content_area;
        let tabs = regions.visualizer_tab_area;
        assert_eq!(canvas.y, tabs.bottom() + 1);
        state.spectrogram.data = Some(textamp::media::SpectrogramData {
            track_key: "fixture".into(),
            duration_ms: 60_000,
            bins_per_frame: 48,
            frame_count: 600,
            frames_per_second: 10.0,
            sample_rate: 44_100,
            version: 6,
            created_at: 0,
            frames: vec![220; 600 * 48],
        });
        terminal
            .draw(|frame| {
                textamp::ui::render(frame, &state);
            })
            .unwrap();
        let after = terminal.backend().buffer();
        for y in 0..canvas.y {
            for x in 0..width {
                assert_eq!(before[(x, y)], after[(x, y)], "{width}x{height} at {x},{y}");
            }
        }
        for x in canvas.x..canvas.right() {
            assert_eq!(after[(x, canvas.y - 1)].symbol(), " ");
        }
        if canvas.height > 1 {
            assert_eq!(after[(canvas.x, canvas.y)].symbol(), "▀");
        }
    }
}

#[tokio::test]
async fn rejected_seek_is_visible_without_moving_position_or_stopping_playback() {
    use textamp::app::dispatch::{dispatch_action, handle_core_event};
    use textamp::app::event::PlaybackEvent;
    let mut state = AppState::new();
    state.playback.status = PlayStatus::Playing;
    state.playback.duration_ms = 60_000;
    state.playback.position_ms = 10_000;
    let mut audio = textamp::audio::AudioPlayer::new_without_audio();
    let mut config = textamp::config::Config::default();

    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    dispatch_action(
        PlaybackAction::Seek(30_000).into(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    assert_eq!(state.playback.position_ms, 10_000);
    assert_eq!(state.playback.status, PlayStatus::Playing);
    assert_eq!(
        state.notifications.status_message.as_deref(),
        Some("Cannot seek")
    );
    let actions = handle_core_event(
        PlaybackEvent::SeekFailed {
            playback_id: state.playback.request_id,
            message: "Cannot seek in this track".into(),
        }
        .into(),
        &mut state,
        &tx,
    );
    assert!(actions.is_empty());
    assert_eq!(state.playback.status, PlayStatus::Playing);
    assert_eq!(state.consecutive_playback_errors, 0);
}
