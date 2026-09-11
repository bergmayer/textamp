use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::TestBackend, Terminal};
use textamp::app::action::RadioAction;
use textamp::app::command_palette;
use textamp::app::handlers::{key_input, mouse_input};
use textamp::app::sources::ActiveSource;
use textamp::app::state::{NowPlayingFocus, StationColumn, View};
use textamp::app::{Action, AppState};
use textamp::config::Config;

fn station_state() -> AppState {
    let mut state = AppState::new();
    state.view = View::Queue;
    state.active_library = Some("5".into());
    state.stations = vec![serde_json::from_value(serde_json::json!({
        "key": "/library/sections/5/stations/randomAlbum",
        "title": "Random Album Radio",
        "type": "station",
        "identifier": "randomAlbum"
    }))
    .unwrap()];
    state
}

fn select_random_album_radio(state: &mut AppState) -> Vec<Action> {
    command_palette::open_with_query(state, "Random Album Radio");
    let command_palette::PaletteOutcome::Execute(command) =
        command_palette::handle_key(state, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("Expected a matching station");
    };
    state.palette.close();
    command_palette::run(command, state)
}

#[test]
fn palette_and_alt_r_use_the_same_station_action() {
    for key in [
        "/library/sections/5/stations/randomAlbum",
        "nav-radio/randomAlbum",
        "folder-radio/randomAlbum",
    ] {
        let mut state = station_state();
        state.stations[0].key = key.into();
        if key.starts_with("nav-radio/") {
            state.active_library = Some("navidrome:account:1".into());
        }
        if key.starts_with("folder-radio/") {
            state.sources.active = ActiveSource::Folder("fixture".into());
            state.active_library = Some("folder:fixture".into());
        }
        let config = Config::default();
        let palette = select_random_album_radio(&mut state);
        let shortcut = key_input::handle_key(
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT),
            &mut state,
            &config,
        );
        for actions in [&palette, &shortcut] {
            assert!(
                matches!(actions.as_slice(), [Action::Radio(RadioAction::PlayStation(key))]
            if key == &state.stations[0].key),
                "{actions:?}"
            );
        }

        // The Now Playing Radio button opens this same palette. Exercise its
        // keyboard entry and a real rendered row click, not just palette::run.
        state.now_playing_focus = NowPlayingFocus::Sidebar;
        state.now_playing_sidebar_index = 0;
        assert!(key_input::handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut state,
            &config
        )
        .is_empty());
        assert!(state.palette.open);
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| {
                state.apply_render_feedback(textamp::ui::render(frame, &state));
            })
            .unwrap();
        let row = state
            .hit_regions
            .command_palette
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .find(|(_, index)| {
                matches!(
                    &state.palette.entries[state.palette.matches[*index]].command,
                    textamp::app::state::PaletteCommandKind::PlayStation(k) if k == key
                )
            })
            .unwrap()
            .0;
        let clicked = mouse_input::handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: row.x,
                row: row.y,
                modifiers: KeyModifiers::NONE,
            },
            &mut state,
        );
        assert!(
            matches!(clicked.as_slice(), [Action::Radio(RadioAction::PlayStation(k))] if k == key)
        );

        // A category replaces the current list, but must not hide the root
        // station from the global shortcut.
        state.station_nav.columns.push(StationColumn::new(
            None,
            "stations".into(),
            state.stations.clone(),
        ));
        state.stations.clear();
        let nested = key_input::handle_key(
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT),
            &mut state,
            &config,
        );
        assert!(
            matches!(nested.as_slice(), [Action::Radio(RadioAction::PlayStation(k))] if k == key)
        );
    }
}

#[test]
fn random_album_shortcut_is_unavailable_without_a_station_for_every_source() {
    for source in [
        ActiveSource::None,
        ActiveSource::Folder("local-or-webdav".into()),
    ] {
        let mut state = station_state();
        state.sources.active = source;
        state.stations.clear();
        assert!(key_input::handle_key(
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT),
            &mut state,
            &Config::default()
        )
        .is_empty());
        assert!(
            !key_input::available_alt_commands(&state)
                .iter()
                .find(|c| c.key == 'r' && c.modifier == key_input::CommandModifier::Alt)
                .unwrap()
                .enabled
        );
    }
}

#[test]
fn folder_palette_keeps_shared_controls_but_hides_server_only_commands() {
    use textamp::app::state::PaletteCommandKind;
    let mut state = station_state();
    state.sources.active = ActiveSource::Folder("local-or-webdav".into());
    textamp::app::sources::radio::load_folder_stations(&mut state);
    state
        .queue
        .tracks
        .push(textamp::library::track::Track::from_folder(
            "local-or-webdav",
            "song.flac",
        ));
    let clear = key_input::handle_key(
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
        &mut state,
        &Config::default(),
    );
    assert!(matches!(
        clear.as_slice(),
        [Action::Queue(textamp::app::action::QueueAction::ClearQueue)]
    ));
    let entries = command_palette::materialize_entries(&state);
    assert!(entries
        .iter()
        .any(|e| matches!(e.command, PaletteCommandKind::OpenSearch)));
    assert!(entries
        .iter()
        .any(|e| matches!(e.command, PaletteCommandKind::GotoNowPlaying)));
    assert!(entries
        .iter()
        .any(|e| matches!(e.command, PaletteCommandKind::PlayPause)));
    assert!(!entries.iter().any(|e| matches!(
        e.command,
        PaletteCommandKind::SaveQueue
            | PaletteCommandKind::ArtistRadio
            | PaletteCommandKind::ToggleDj(_)
            | PaletteCommandKind::RemixTwofer
            | PaletteCommandKind::GotoGenres
            | PaletteCommandKind::GotoLibrary
    )));
    assert!(entries.iter().any(|e| matches!(&e.command, PaletteCommandKind::PlayStation(key) if key == "folder-radio/randomAlbum")));
    assert!(!entries.iter().any(|e| e.label == "DJ Modes"));
}
