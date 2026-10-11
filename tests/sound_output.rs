use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::TestBackend, Terminal};
use textamp::app::action::{PlaybackAction, SettingsAction};
use textamp::app::handlers::{dispatch_playback, dispatch_settings, key_input, mouse_input};
use textamp::app::state::{
    PaletteCommandKind, PlayStatus, SettingsFocus, SettingsPage, SettingsSection, TextampSetting,
    View,
};
use textamp::app::{Action, AppState};
use textamp::audio::airplay::{AirplayDestination, OutputDiscovery};
use textamp::audio::{AudioOutput, AudioPlayer};
use textamp::config::Config;

fn speaker() -> AirplayDestination {
    AirplayDestination {
        name: "Living room".into(),
        address: "192.168.1.5".parse().unwrap(),
        port: 7000,
        airplay2: true,
        txt: vec![],
    }
}
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn draw(state: &mut AppState, width: u16, height: u16) -> String {
    state.terminal_width = width;
    state.terminal_height = height;
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| state.apply_render_feedback(textamp::ui::render(frame, state)))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>()
}

#[tokio::test]
async fn shortcut_settings_and_palette_share_the_picker() {
    let mut state = AppState::new();
    let mut config = Config::default();
    let mut audio = AudioPlayer::new_without_audio();
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    assert!(matches!(
        key_input::handle_key(key(KeyCode::F(8)), &mut state, &config).as_slice(),
        [Action::Playback(PlaybackAction::OpenOutputPicker)]
    ));
    let entry = textamp::app::command_palette::materialize_entries(&state)
        .into_iter()
        .find(|e| e.label == "Sound Output")
        .unwrap();
    assert_eq!(entry.hint, "F8");
    assert!(matches!(
        textamp::app::command_palette::run(entry.command, &mut state).as_slice(),
        [Action::Playback(PlaybackAction::OpenOutputPicker)]
    ));
    state.view = View::Settings;
    state.settings_state.section = SettingsSection::Textamp;
    state.settings_state.page = SettingsPage::Display;
    state.settings_state.focus = SettingsFocus::Content;
    state.settings_state.item_index = state
        .textamp_settings()
        .iter()
        .position(|row| *row == TextampSetting::SoundOutput)
        .unwrap();
    let actions = dispatch_settings::dispatch(
        &tx,
        &mut config,
        SettingsAction::SettingsSelect,
        &mut state,
        &mut audio,
    )
    .await
    .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [Action::Playback(PlaybackAction::OpenOutputPicker)]
    ));
    let actions = dispatch_playback::dispatch(
        &tx,
        PlaybackAction::OpenOutputPicker,
        &mut state,
        &mut audio,
    )
    .await
    .unwrap();
    assert!(state.popups.sound_output_active);
    assert!(matches!(
        actions.as_slice(),
        [Action::Playback(PlaybackAction::DiscoverAirplay)]
    ));
    assert!(draw(&mut state, 80, 24).contains("Sound Output"));
}

#[tokio::test]
async fn cancelled_discovery_cannot_reopen_or_change_output() {
    let mut state = AppState::new();
    let config = Config::default();
    let mut audio = AudioPlayer::new_without_audio();
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    state.popups.sound_output_active = true;
    state.sound_output.loading = true;
    state.sound_output.request_id = 9;
    state.sound_output.current = AudioOutput::AirPlay(speaker());
    key_input::handle_key(key(KeyCode::Esc), &mut state, &config);
    dispatch_playback::dispatch(
        &tx,
        PlaybackAction::AirplayDiscovered {
            request_id: 9,
            result: Ok(OutputDiscovery {
                local: Ok(vec!["Device".into()]),
                airplay: Ok(vec![speaker()]),
            }),
        },
        &mut state,
        &mut audio,
    )
    .await
    .unwrap();
    assert!(!state.popups.sound_output_active && !state.sound_output.loading);
    assert!(state.sound_output.local_devices.is_empty());
    assert_eq!(state.sound_output.current, AudioOutput::AirPlay(speaker()));
}

#[tokio::test]
async fn discovery_preserves_focus_and_errors_stay_in_popup() {
    let mut state = AppState::new();
    let mut audio = AudioPlayer::new_without_audio();
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    state.popups.sound_output_active = true;
    state.sound_output.request_id = 3;
    state.sound_output.destinations.push(speaker());
    state.sound_output.index = 1;
    dispatch_playback::dispatch(
        &tx,
        PlaybackAction::AirplayDiscovered {
            request_id: 3,
            result: Ok(OutputDiscovery {
                local: Ok(vec!["Device".into()]),
                airplay: Err("Avahi unavailable".into()),
            }),
        },
        &mut state,
        &mut audio,
    )
    .await
    .unwrap();
    assert_eq!(state.sound_output.index, 2);
    assert_eq!(
        state.sound_output.error.as_deref(),
        Some("Avahi unavailable")
    );
    assert!(draw(&mut state, 80, 24).contains("Avahi unavailable"));
    assert!(draw(&mut state, 40, 12).contains("Sound Output"));
}

#[tokio::test]
async fn selection_during_playback_does_not_stop_or_switch_audio() {
    let mut state = AppState::new();
    let mut audio = AudioPlayer::new_without_audio();
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    state.playback.status = PlayStatus::Playing;
    state.popups.sound_output_active = true;
    let generation = audio.playback_id();
    dispatch_playback::dispatch(
        &tx,
        PlaybackAction::SelectOutput(AudioOutput::AirPlay(speaker())),
        &mut state,
        &mut audio,
    )
    .await
    .unwrap();
    assert_eq!(state.playback.status, PlayStatus::Playing);
    assert_eq!(audio.playback_id(), generation);
    assert_eq!(state.sound_output.current, AudioOutput::Local);
    assert!(state
        .sound_output
        .error
        .as_deref()
        .unwrap()
        .contains("Stop playback"));
}

#[test]
fn mouse_highlights_only_and_select_and_cancel_match_keyboard() {
    let mut state = AppState::new();
    let config = Config::default();
    state.popups.sound_output_active = true;
    state.sound_output.destinations.push(speaker());
    draw(&mut state, 80, 24);
    let (area, _) = state.hit_regions.sound_output_rows[1];
    let click = |area: ratatui::layout::Rect| MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: area.x,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    };
    assert!(mouse_input::handle_mouse(click(area), &mut state).is_empty());
    assert_eq!(state.sound_output.index, 1);
    assert_eq!(state.sound_output.current, AudioOutput::Local);
    let actions = key_input::handle_key(key(KeyCode::Enter), &mut state, &config);
    assert!(matches!(
        actions.as_slice(),
        [Action::Playback(PlaybackAction::SelectOutput(
            AudioOutput::AirPlay(_)
        ))]
    ));
    let (area, _) = state.hit_regions.sound_output_buttons[2];
    mouse_input::handle_mouse(click(area), &mut state);
    assert!(!state.popups.sound_output_active);
    assert_eq!(state.sound_output.current, AudioOutput::Local);
}

#[test]
fn output_action_has_a_single_palette_entry() {
    let mut state = AppState::new();
    state.sound_output.destinations.push(speaker());
    let entries = textamp::app::command_palette::materialize_entries(&state);
    assert_eq!(
        entries
            .iter()
            .filter(|e| matches!(e.command, PaletteCommandKind::OpenOutputPicker))
            .count(),
        1
    );
}

#[test]
fn mouse_highlight_keeps_the_output_viewport() {
    let mut state = AppState::new();
    let config = Config::default();
    state.popups.sound_output_active = true;
    state.sound_output.local_devices = (0..30).map(|i| format!("Device {i}")).collect();
    state.sound_output.index = 25;
    draw(&mut state, 80, 24);
    let before = state.hit_regions.sound_output_rows.clone();
    let (area, index) = before[0];
    mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        },
        &mut state,
    );
    draw(&mut state, 80, 24);
    assert_eq!(state.sound_output.index, index);
    assert_eq!(state.hit_regions.sound_output_rows, before);
    key_input::handle_key(key(KeyCode::Up), &mut state, &config);
    assert_eq!(state.sound_output.scroll_pin, None);
}
