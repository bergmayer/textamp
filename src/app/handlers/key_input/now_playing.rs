//! Queue and Now Playing view key handling.

use crate::app::action::*;
use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::app::Action;
use crate::app::state::{NowPlayingFocus, PlaybackMode, View};
use crate::app::AppState;
use crate::plex::models::Track;

/// Handle Queue view keys (track list + stations panel).
pub(super) fn handle_queue_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    // Tab/Shift+Tab cycles through main views
    if key.code == KeyCode::Tab {
        return if key.modifiers.contains(KeyModifiers::SHIFT) {
            vec![NavigationAction::PrevView.into()]
        } else {
            vec![NavigationAction::NextView.into()]
        };
    }
    if key.code == KeyCode::BackTab {
        return vec![NavigationAction::PrevView.into()];
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
            // Esc does nothing (use Tab or Ctrl+key to navigate)
            vec![]
        }
        KeyCode::F(1) | KeyCode::Char('?') => vec![NavigationAction::SetView(View::Help).into()],

        // Tab/Shift+Tab cycles through main views
        KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => vec![NavigationAction::PrevView.into()],
        KeyCode::Tab => vec![NavigationAction::NextView.into()],

        // Up arrow at top: focus the tab bar
        KeyCode::Up => {
            state.visualizer_tab_focused = true;
            vec![]
        }

        // Left/Right arrow seeking (1 second increments)
        KeyCode::Left => vec![PlaybackAction::SeekRelative(-1000).into()],
        KeyCode::Right => vec![PlaybackAction::SeekRelative(1000).into()],

        _ => vec![],
    }
}

/// Handle station panel navigation keys (queue view, stations focused).
fn handle_station_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    // When "◂ back" row is highlighted, handle it before normal dispatch
    if state.scroll.station_back_highlighted {
        match key.code {
            KeyCode::Enter => {
                state.scroll.station_back_highlighted = false;
                return vec![RadioAction::NavigateStationsBack.into()];
            }
            KeyCode::Down => {
                state.scroll.station_back_highlighted = false;
                return vec![];
            }
            KeyCode::Up => {
                // Already at back row, no-op
                return vec![];
            }
            _ => {
                state.scroll.station_back_highlighted = false;
            }
        }
    }

    match key.code {
        KeyCode::Esc => {
            // If drilled into a sub-column, go back a level first
            if state.station_nav.can_go_left() {
                state.station_nav.focus_left();
                if let Some(col) = state.station_nav.focused() {
                    state.stations = col.stations.clone();
                }
            } else {
                state.now_playing_focus = NowPlayingFocus::Tracks;
            }
            vec![]
        }
        KeyCode::F(1) | KeyCode::Char('?') => vec![NavigationAction::SetView(View::Help).into()],

        KeyCode::Up => {
            state.scroll.station = None;
            // At top of non-root column, highlight the "◂ back" row
            let at_top = state.station_nav.focused().map_or(false, |c| c.selected_index == 0);
            let is_drilled = state.station_nav.focused().map_or(false, |c| c.key.is_some());
            if at_top && is_drilled {
                state.scroll.station_back_highlighted = true;
                return vec![];
            }
            state.station_nav.move_up();
            // Skip separators
            skip_station_separators(state, true);
            state.station_nav.truncate_right_columns();
            vec![]
        }
        KeyCode::Down => {
            state.scroll.station = None;
            state.station_nav.move_down();
            // Skip separators
            skip_station_separators(state, false);
            state.station_nav.truncate_right_columns();
            vec![]
        }
        KeyCode::PageUp => {
            state.scroll.station = None;
            if let Some(col) = state.station_nav.focused_mut() {
                col.selected_index = col.selected_index.saturating_sub(10);
            }
            state.station_nav.truncate_right_columns();
            vec![]
        }
        KeyCode::PageDown => {
            state.scroll.station = None;
            if let Some(col) = state.station_nav.focused_mut() {
                let max = col.stations.len().saturating_sub(1);
                col.selected_index = (col.selected_index + 10).min(max);
            }
            state.station_nav.truncate_right_columns();
            vec![]
        }
        KeyCode::Home => {
            state.scroll.station = None;
            if let Some(col) = state.station_nav.focused_mut() {
                col.selected_index = 0;
            }
            state.station_nav.truncate_right_columns();
            vec![]
        }
        KeyCode::End => {
            state.scroll.station = None;
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
                    return vec![RadioAction::DrillIntoStation(station.key.clone(), station.title.clone()).into()];
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
                        "action:adventure" => vec![SearchAction::OpenAdventureLauncher.into()],
                        "action:artist_radio" => vec![SearchAction::OpenArtistRadioPicker.into()],
                        _ => vec![],
                    };
                }
                // Remix items
                if station.key.starts_with("remix:") {
                    return match station.key.as_str() {
                        "remix:gemini" => vec![QueueAction::RemixGemini.into()],
                        "remix:twofer" => vec![QueueAction::RemixTwofer.into()],
                        "remix:stretch" => vec![QueueAction::RemixStretch.into()],
                        "remix:doppelganger" => vec![QueueAction::RemixDoppelganger.into()],
                        "remix:shuffle" => {
                            if state.queue.shuffle_undo_queue.is_some() {
                                vec![QueueAction::RemixUndoShuffle.into()]
                            } else {
                                vec![QueueAction::RemixShuffle.into()]
                            }
                        }
                        _ => vec![],
                    };
                }
                // DJ mode toggle
                if station.is_dj_mode() {
                    if let Some(mode) = crate::app::state::DjMode::from_key(&station.key) {
                        return vec![RadioAction::ToggleDjMode(mode).into()];
                    }
                    // Friendganger is unavailable
                    return vec![];
                }
                if station.is_category() {
                    return vec![RadioAction::DrillIntoStation(station.key.clone(), station.title.clone()).into()];
                }
                return vec![RadioAction::PlayStation(station.key.clone()).into()];
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
            PlaybackMode::Queue | PlaybackMode::None => state.queue.tracks.len().saturating_sub(1),
            PlaybackMode::Radio => state.radio.tracks.len().saturating_sub(1),
        }
    };

    match key.code {
        KeyCode::Esc => {
            // If items are multi-selected, clear selection
            if !state.queue.selected.is_empty() {
                state.queue.selected.clear();
                return vec![];
            }
            // Otherwise ESC does nothing in queue (use Ctrl+shortcuts to navigate)
            vec![]
        }
        KeyCode::F(1) | KeyCode::Char('?') => vec![NavigationAction::SetView(View::Help).into()],

        // Shift+Up/Down: move queue track(s) up/down (batch if multi-selected)
        KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
            if state.queue.selected.is_empty() {
                vec![QueueAction::MoveQueueTrackUp.into()]
            } else {
                vec![QueueAction::MoveSelectedTracksUp.into()]
            }
        }
        KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
            if state.queue.selected.is_empty() {
                vec![QueueAction::MoveQueueTrackDown.into()]
            } else {
                vec![QueueAction::MoveSelectedTracksDown.into()]
            }
        }

        // Ctrl+Z: undo last remix
        KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            vec![QueueAction::UndoLastRemix.into()]
        }

        KeyCode::Up => {
            state.scroll.queue = None;
            if state.list_state.queue_index > 0 {
                state.list_state.queue_index -= 1;
            }
            vec![]
        }
        KeyCode::Down => {
            state.scroll.queue = None;
            let max = get_max_index(state);
            state.list_state.queue_index = (state.list_state.queue_index + 1).min(max);
            vec![]
        }
        KeyCode::PageUp => {
            state.scroll.queue = None;
            state.list_state.queue_index = state.list_state.queue_index.saturating_sub(10);
            vec![]
        }
        KeyCode::PageDown => {
            state.scroll.queue = None;
            let max = get_max_index(state);
            state.list_state.queue_index = (state.list_state.queue_index + 10).min(max);
            vec![]
        }
        KeyCode::Home => {
            state.scroll.queue = None;
            state.list_state.queue_index = 0;
            vec![]
        }
        KeyCode::End => {
            state.scroll.queue = None;
            let max = get_max_index(state);
            state.list_state.queue_index = max;
            vec![]
        }

        KeyCode::Enter => {
            // If the selected track is already playing, switch to NowPlaying view
            let is_current = match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => {
                    state.queue.index == Some(state.list_state.queue_index)
                }
                PlaybackMode::Radio => state.radio.track_index == Some(state.list_state.queue_index),
            };
            if is_current {
                return vec![NavigationAction::SetView(View::NowPlaying).into(), SystemAction::LoadWaveform.into()];
            }

            // Play selected item from queue or radio (without modifying queue order)
            match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => {
                    let queue_idx = state.list_state.queue_index;
                    if queue_idx < state.queue.tracks.len() {
                        vec![QueueAction::JumpToQueueIndex(queue_idx).into()]
                    } else {
                        vec![]
                    }
                }
                PlaybackMode::Radio => {
                    // Jump to selected radio track without clearing radio state
                    if state.list_state.queue_index < state.radio.tracks.len() {
                        vec![RadioAction::JumpToRadioTrack(state.list_state.queue_index).into()]
                    } else {
                        vec![]
                    }
                }
            }
        }

        KeyCode::Delete => {
            if !state.queue.selected.is_empty() {
                return vec![QueueAction::RemoveSelectedFromQueue.into()];
            }
            match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => {
                    vec![QueueAction::RemoveFromQueue(state.list_state.queue_index).into()]
                }
                PlaybackMode::Radio => {
                    let target_idx = state.list_state.queue_index;
                    let snapshot = state.convert_radio_to_queue("Delete track (from radio)");
                    state.queue.undo_snapshot = Some(snapshot);
                    vec![QueueAction::RemoveFromQueue(target_idx).into()]
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
                PlaybackMode::Queue | PlaybackMode::None => &state.queue.tracks,
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
