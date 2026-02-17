//! Queue and Now Playing view key handling.

use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::app::Action;
use crate::app::state::{NowPlayingFocus, PlaybackMode, View};
use crate::app::AppState;
use crate::api::models::Track;

/// Handle Queue view keys (track list + stations panel).
pub(super) fn handle_queue_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    // Tab/Shift+Tab cycles through main views
    if key.code == KeyCode::Tab {
        return if key.modifiers.contains(KeyModifiers::SHIFT) {
            vec![Action::PrevView]
        } else {
            vec![Action::NextView]
        };
    }
    if key.code == KeyCode::BackTab {
        return vec![Action::PrevView];
    }

    // With stations focused, dispatch to station handler
    if state.now_playing_focus == NowPlayingFocus::Stations {
        return handle_station_keys(key, state);
    }

    // Track list handling
    handle_queue_track_keys(key, state)
}

/// Handle Now Playing visualizer view keys (artwork + track info + waveform seekbar).
pub(super) fn handle_now_playing_visualizer_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    // When visualizer tab bar is focused, arrow keys navigate tabs
    if state.visualizer_tab_focused {
        match key.code {
            KeyCode::Left => {
                state.visualizer_tab = state.visualizer_tab.prev();
                return vec![];
            }
            KeyCode::Right => {
                state.visualizer_tab = state.visualizer_tab.next();
                return vec![];
            }
            KeyCode::Down | KeyCode::Enter => {
                state.visualizer_tab_focused = false;
                return vec![];
            }
            KeyCode::Esc => {
                state.visualizer_tab_focused = false;
                return vec![];
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Esc => {
            // Esc goes back to Queue view
            vec![Action::SetView(View::Queue)]
        }
        KeyCode::F(1) | KeyCode::Char('?') => vec![Action::SetView(View::Help)],

        // Tab/Shift+Tab cycles through main views
        KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => vec![Action::PrevView],
        KeyCode::Tab => vec![Action::NextView],

        // Up arrow at top: focus the tab bar
        KeyCode::Up => {
            state.visualizer_tab_focused = true;
            vec![]
        }

        // Left/Right arrow seeking (1 second increments)
        KeyCode::Left => vec![Action::SeekRelative(-1000)],
        KeyCode::Right => vec![Action::SeekRelative(1000)],

        _ => vec![],
    }
}

/// Handle station panel navigation keys (queue view, stations focused).
fn handle_station_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    match key.code {
        KeyCode::Esc => {
            state.now_playing_focus = NowPlayingFocus::Tracks;
            vec![]
        }
        KeyCode::F(1) | KeyCode::Char('?') => vec![Action::SetView(View::Help)],

        KeyCode::Up => {
            state.station_scroll_pin = None;
            state.station_nav.move_up();
            // Skip separators
            skip_station_separators(state, true);
            state.station_nav.truncate_right_columns();
            vec![]
        }
        KeyCode::Down => {
            state.station_scroll_pin = None;
            state.station_nav.move_down();
            // Skip separators
            skip_station_separators(state, false);
            state.station_nav.truncate_right_columns();
            vec![]
        }
        KeyCode::PageUp => {
            state.station_scroll_pin = None;
            if let Some(col) = state.station_nav.focused_mut() {
                col.selected_index = col.selected_index.saturating_sub(10);
            }
            state.station_nav.truncate_right_columns();
            vec![]
        }
        KeyCode::PageDown => {
            state.station_scroll_pin = None;
            if let Some(col) = state.station_nav.focused_mut() {
                let max = col.stations.len().saturating_sub(1);
                col.selected_index = (col.selected_index + 10).min(max);
            }
            state.station_nav.truncate_right_columns();
            vec![]
        }
        KeyCode::Home => {
            state.station_scroll_pin = None;
            if let Some(col) = state.station_nav.focused_mut() {
                col.selected_index = 0;
            }
            state.station_nav.truncate_right_columns();
            vec![]
        }
        KeyCode::End => {
            state.station_scroll_pin = None;
            if let Some(col) = state.station_nav.focused_mut() {
                col.selected_index = col.stations.len().saturating_sub(1);
            }
            state.station_nav.truncate_right_columns();
            vec![]
        }

        KeyCode::Right => {
            // Right arrow: drill into categories, otherwise move focus to tracks
            // First check if there's already a column to the right we can move to
            if state.station_nav.focus_right() {
                if let Some(col) = state.station_nav.focused() {
                    state.stations = col.stations.clone();
                }
                return vec![];
            }
            // Drill into categories only; non-categories → focus tracks
            if let Some(station) = state.station_nav.selected_station().cloned() {
                if station.is_category() && !station.key.starts_with("action:") {
                    return vec![Action::DrillIntoStation(station.key.clone(), station.title.clone())];
                }
            }
            state.now_playing_focus = NowPlayingFocus::Tracks;
            vec![]
        }

        KeyCode::Enter => {
            // Enter: drill into categories, play stations, toggle DJ modes, or trigger action popups
            if let Some(station) = state.station_nav.selected_station().cloned() {
                // Skip separators
                if station.is_separator() {
                    return vec![];
                }
                if station.key.starts_with("action:") {
                    return match station.key.as_str() {
                        "action:adventure" => vec![Action::OpenAdventureLauncher],
                        "action:artist_radio" => vec![Action::OpenArtistRadioPicker],
                        _ => vec![],
                    };
                }
                // Remix items
                if station.key.starts_with("remix:") {
                    return match station.key.as_str() {
                        "remix:gemini" => vec![Action::RemixGemini],
                        "remix:twofer" => vec![Action::RemixTwofer],
                        "remix:stretch" => vec![Action::RemixStretch],
                        "remix:doppelganger" => vec![Action::RemixDoppelganger],
                        "remix:shuffle" => {
                            if state.shuffle_undo_queue.is_some() {
                                vec![Action::RemixUndoShuffle]
                            } else {
                                vec![Action::RemixShuffle]
                            }
                        }
                        _ => vec![],
                    };
                }
                // DJ mode toggle
                if station.is_dj_mode() {
                    if let Some(mode) = crate::app::state::DjMode::from_key(&station.key) {
                        return vec![Action::ToggleDjMode(mode)];
                    }
                    // Friendganger is unavailable
                    return vec![];
                }
                if station.is_category() {
                    return vec![Action::DrillIntoStation(station.key.clone(), station.title.clone())];
                }
                return vec![Action::PlayStation(station.key.clone())];
            }
            vec![]
        }

        KeyCode::Left | KeyCode::Backspace => {
            if state.station_nav.can_go_left() {
                state.station_nav.focus_left();
                if let Some(col) = state.station_nav.focused() {
                    state.stations = col.stations.clone();
                }
            } else {
                // At root of station nav, move focus to tracks
                state.now_playing_focus = NowPlayingFocus::Tracks;
            }
            vec![]
        }

        // Alphabet jumping in station column
        KeyCode::Char(c) if c.is_ascii_alphabetic() && !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let letter_lower = c.to_ascii_lowercase();
            let use_second_char = key.modifiers.contains(KeyModifiers::SHIFT);
            if let Some(col) = state.station_nav.focused_mut() {
                if use_second_char {
                    let first_letter = col.stations.get(col.selected_index)
                        .and_then(|s| s.title.chars().next())
                        .map(|ch| ch.to_ascii_lowercase());
                    if let Some(first_letter) = first_letter {
                        if let Some(idx) = col.stations.iter().position(|s| {
                            let mut chars = s.title.chars();
                            let first = chars.next().map(|ch| ch.to_ascii_lowercase());
                            let second = chars.next().map(|ch| ch.to_ascii_lowercase());
                            first == Some(first_letter) && second == Some(letter_lower)
                        }) {
                            col.selected_index = idx;
                        }
                    }
                } else {
                    if let Some(idx) = col.stations.iter().position(|s| {
                        s.title.chars().next()
                            .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                            .unwrap_or(false)
                    }) {
                        col.selected_index = idx;
                    }
                }
            }
            state.station_nav.truncate_right_columns();
            vec![]
        }

        _ => vec![],
    }
}

/// Skip over separator items when navigating stations.
/// `going_up` indicates whether the user pressed Up (true) or Down (false).
fn skip_station_separators(state: &mut AppState, going_up: bool) {
    if let Some(col) = state.station_nav.focused_mut() {
        let max = col.stations.len();
        if max == 0 { return; }
        let mut attempts = 0;
        while attempts < max {
            if let Some(station) = col.stations.get(col.selected_index) {
                if !station.is_separator() {
                    break;
                }
            } else {
                break;
            }
            if going_up {
                if col.selected_index == 0 {
                    // Wrapped to top and still on separator, move down
                    col.selected_index = 1.min(max.saturating_sub(1));
                } else {
                    col.selected_index -= 1;
                }
            } else {
                if col.selected_index >= max.saturating_sub(1) {
                    col.selected_index = max.saturating_sub(2);
                } else {
                    col.selected_index += 1;
                }
            }
            attempts += 1;
        }
    }
}

/// Handle track list keys in the Queue view.
fn handle_queue_track_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    // Get the max index based on current mode
    let get_max_index = |state: &AppState| -> usize {
        match state.playback_mode {
            PlaybackMode::Queue | PlaybackMode::None => state.queue.len().saturating_sub(1),
            PlaybackMode::Radio => state.radio.tracks.len().saturating_sub(1),
        }
    };

    match key.code {
        KeyCode::Esc => {
            // If items are multi-selected, clear selection
            if !state.queue_selected.is_empty() {
                state.queue_selected.clear();
                return vec![];
            }
            // Otherwise ESC does nothing in queue (use Ctrl+shortcuts to navigate)
            vec![]
        }
        KeyCode::F(1) | KeyCode::Char('?') => vec![Action::SetView(View::Help)],

        // Shift+Up/Down: move queue track(s) up/down (batch if multi-selected)
        KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
            if state.queue_selected.is_empty() {
                vec![Action::MoveQueueTrackUp]
            } else {
                vec![Action::MoveSelectedTracksUp]
            }
        }
        KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
            if state.queue_selected.is_empty() {
                vec![Action::MoveQueueTrackDown]
            } else {
                vec![Action::MoveSelectedTracksDown]
            }
        }

        // Ctrl+Z: undo last remix
        KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            vec![Action::UndoLastRemix]
        }

        KeyCode::Up => {
            state.queue_scroll_pin = None;
            if state.list_state.queue_index > 0 {
                state.list_state.queue_index -= 1;
            }
            vec![]
        }
        KeyCode::Down => {
            state.queue_scroll_pin = None;
            let max = get_max_index(state);
            state.list_state.queue_index = (state.list_state.queue_index + 1).min(max);
            vec![]
        }
        KeyCode::PageUp => {
            state.queue_scroll_pin = None;
            state.list_state.queue_index = state.list_state.queue_index.saturating_sub(10);
            vec![]
        }
        KeyCode::PageDown => {
            state.queue_scroll_pin = None;
            let max = get_max_index(state);
            state.list_state.queue_index = (state.list_state.queue_index + 10).min(max);
            vec![]
        }
        KeyCode::Home => {
            state.queue_scroll_pin = None;
            state.list_state.queue_index = 0;
            vec![]
        }
        KeyCode::End => {
            state.queue_scroll_pin = None;
            let max = get_max_index(state);
            state.list_state.queue_index = max;
            vec![]
        }

        KeyCode::Enter => {
            // If the selected track is already playing, switch to NowPlaying view
            let is_current = match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => {
                    state.queue_index == Some(state.list_state.queue_index)
                }
                PlaybackMode::Radio => state.radio.track_index == Some(state.list_state.queue_index),
            };
            if is_current {
                return vec![Action::SetView(View::NowPlaying), Action::LoadWaveform];
            }

            // Play selected item from queue or radio
            match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => {
                    let queue_idx = state.list_state.queue_index;
                    if let Some(track) = state.queue.get(queue_idx).cloned() {
                        state.queue_index = Some(queue_idx);
                        vec![Action::PlayTrack(track)]
                    } else {
                        vec![]
                    }
                }
                PlaybackMode::Radio => {
                    // Jump to selected radio track without clearing radio state
                    if state.list_state.queue_index < state.radio.tracks.len() {
                        vec![Action::JumpToRadioTrack(state.list_state.queue_index)]
                    } else {
                        vec![]
                    }
                }
            }
        }

        KeyCode::Delete => {
            if !state.queue_selected.is_empty() {
                return vec![Action::RemoveSelectedFromQueue];
            }
            match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => {
                    vec![Action::RemoveFromQueue(state.list_state.queue_index)]
                }
                PlaybackMode::Radio => {
                    let target_idx = state.list_state.queue_index;
                    let snapshot = state.convert_radio_to_queue("Delete track (from radio)");
                    state.queue_undo_snapshot = Some(snapshot);
                    vec![Action::RemoveFromQueue(target_idx)]
                }
            }
        }

        // Left: switch focus to stations panel
        KeyCode::Left => {
            state.now_playing_focus = NowPlayingFocus::Stations;
            vec![]
        }

        // Alphabet jumping
        KeyCode::Char(c) if c.is_ascii_alphabetic() && key.modifiers.is_empty() => {
            let letter_lower = c.to_ascii_lowercase();
            let tracks: &[Track] = match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => &state.queue,
                PlaybackMode::Radio => &state.radio.tracks,
            };
            if let Some(idx) = tracks.iter().position(|t| {
                t.title.chars().next()
                    .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                    .unwrap_or(false)
            }) {
                state.list_state.queue_index = idx;
            }
            vec![]
        }

        _ => vec![],
    }
}
