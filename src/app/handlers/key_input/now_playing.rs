//! Queue and Now Playing view key handling.

use crate::app::action::*;
use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::app::Action;
use crate::app::AppState;
use crate::app::state::{NowPlayingFocus, PlaybackMode, View};
use crate::library::models::Track;

/// Handle Queue view keys (track list + sidebar buttons).
pub(super) fn handle_queue_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    if state.tui_queue_type_focused {
        match key.code {
            KeyCode::Enter | KeyCode::Char(' ') => {
                return crate::app::queue_controls::Menu::QueueType.open(state);
            }
            KeyCode::Right => {
                state.tui_queue_type_focused = false;
                state.now_playing_focus = NowPlayingFocus::Tools;
                return vec![];
            }
            KeyCode::Down | KeyCode::Esc => {
                state.tui_queue_type_focused = false;
                return vec![];
            }
            _ => {}
        }
    }
    // Header menus are reached with arrows or pane traversal.
    if state.now_playing_focus == NowPlayingFocus::Tracks && key.code == KeyCode::Left {
        return vec![NavigationAction::SetView(state.tui_queue_workspace.view()).into()];
    }
    if state.now_playing_focus == NowPlayingFocus::Tracks && key.code == KeyCode::Right {
        state.now_playing_focus = NowPlayingFocus::Tools;
        return vec![];
    }

    if state.now_playing_focus == NowPlayingFocus::Tools {
        match key.code {
            KeyCode::Enter | KeyCode::Char(' ') => {
                return crate::app::queue_controls::Menu::Tools.open(state);
            }
            KeyCode::Left => {
                state.now_playing_focus = NowPlayingFocus::Tracks;
                state.tui_queue_type_focused = true;
            }
            KeyCode::Down | KeyCode::Esc => state.now_playing_focus = NowPlayingFocus::Tracks,
            _ => {}
        }
        return vec![];
    }

    // Artwork focus exposes its artist destination.
    if state.now_playing_focus == NowPlayingFocus::Artwork {
        return handle_artwork_keys(key, state);
    }

    handle_queue_track_keys(key, state)
}

/// Handle keyboard nav while the right-side artwork panel has
/// focus. Arrow keys hop to neighbours; Enter is the easter-egg
/// shortcut to open the Artist Bio popup for the now-playing
/// (or focused) artist.
fn handle_artwork_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    match key.code {
        KeyCode::Left => {
            state.now_playing_focus = NowPlayingFocus::Tracks;
            vec![]
        }
        KeyCode::Down => {
            // Drop into the visualizer below — focus the tab bar so
            // the user can immediately pick a viz mode.
            state.now_playing_focus = NowPlayingFocus::Tracks;
            state.visualizer_tab_focused = true;
            vec![NavigationAction::SetView(View::NowPlaying).into()]
        }
        KeyCode::Up => {
            // No-op — artwork is the top-most slot in this column.
            vec![]
        }
        KeyCode::Esc => {
            state.now_playing_focus = NowPlayingFocus::Tracks;
            vec![]
        }
        KeyCode::Enter => state
            .current_track()
            .cloned()
            .map(|track| {
                vec![
                    BrowseAction::OpenDetailsInLibrary {
                        track: Box::new(track),
                        destination: crate::app::library_details::Destination::Artist,
                    }
                    .into(),
                ]
            })
            .unwrap_or_default(),
        _ => vec![],
    }
}

/// Handle keyboard nav over the now-playing left sidebar buttons:
/// Capability-filtered Radio / Queue Edits / Clear Queue.
/// Handle Now Playing visualizer view keys (artwork + track info + waveform seekbar).
///
/// Standard arrow-key navigation: arrows always traverse the
/// on-screen UI elements. Seeking uses the global Shift+arrow shortcuts so the
/// arrows are free to walk between the visualizer, the visualizer
/// tab bar, and the queue / sidebar / tracks at the top of the
/// combined Now-Playing screen.
pub(super) fn handle_now_playing_visualizer_keys(
    key: event::KeyEvent,
    state: &mut AppState,
) -> Vec<Action> {
    if state.now_playing_focus == NowPlayingFocus::Artwork {
        return handle_artwork_keys(key, state);
    }
    // When visualizer tab bar is focused, arrow keys navigate tabs.
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
            KeyCode::Up => {
                // Tab bar → queue tracks at the top of the screen.
                state.visualizer_tab_focused = false;
                state.now_playing_focus = NowPlayingFocus::Tracks;
                return vec![NavigationAction::SetView(View::Queue).into()];
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
        KeyCode::Esc => vec![],
        KeyCode::F(1) => vec![NavigationAction::SetView(View::Help).into()],

        // Up: focus the visualizer tab bar (waveform / spectrum /
        // spectrogram). A second Up walks onto the queue tracks at
        // the top of the screen.
        KeyCode::Up => {
            state.visualizer_tab_focused = true;
            vec![]
        }
        // Down on the visualizer: nothing — it's the bottom-most
        // region of the screen.
        KeyCode::Down => vec![],

        // Left jumps up to the queue area and lands on the left-side
        // sidebar (Radio / DJ Modes / Remix Tools / Clear Queue) —
        // single keypress from the visualizer to the sidebar.
        KeyCode::Left => {
            state.now_playing_focus = NowPlayingFocus::Tools;
            vec![NavigationAction::SetView(View::Queue).into()]
        }
        // Right jumps up to the queue track list. Symmetric with
        // Left so the queue area is one keypress away in either
        // direction.
        KeyCode::Right => {
            state.now_playing_focus = NowPlayingFocus::Tracks;
            vec![NavigationAction::SetView(View::Queue).into()]
        }

        // Preserve the one-second forward shortcut; comma opens Settings globally.
        KeyCode::Char('.') => vec![PlaybackAction::SeekRelative(1000).into()],

        _ => vec![],
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
            // Exit multi-select mode, clearing any selection.
            if state.select_mode || !state.queue.selected.is_empty() {
                state.select_mode = false;
                state.queue.selected.clear();
                return vec![];
            }
            vec![]
        }
        KeyCode::F(1) => vec![NavigationAction::SetView(View::Help).into()],

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
            } else {
                state.tui_queue_type_focused = true;
            }
            vec![]
        }
        KeyCode::Down => {
            state.scroll.queue = None;
            let max = get_max_index(state);
            // Down at the bottom of the list (or in an empty queue)
            // hops the focus over to the visualizer area below — its
            // tab bar gets focused so the user can immediately
            // arrow-Left/Right between waveform / spectrum /
            // spectrogram / vectorscope.
            if state.queue.tracks.is_empty() || state.list_state.queue_index >= max {
                state.visualizer_tab_focused = true;
                return vec![NavigationAction::SetView(View::NowPlaying).into()];
            }
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
            let tracks = if state.playback_mode == PlaybackMode::Radio {
                &state.radio.tracks
            } else {
                &state.queue.tracks
            };
            tracks
                .get(state.list_state.queue_index)
                .cloned()
                .map(|track| {
                    vec![
                        BrowseAction::OpenDetailsInLibrary {
                            track: Box::new(track),
                            destination: crate::app::library_details::Destination::Track,
                        }
                        .into(),
                    ]
                })
                .unwrap_or_default()
        }

        KeyCode::Delete => {
            if !state.queue.selected.is_empty() {
                return vec![QueueAction::RemoveSelectedFromQueue.into()];
            }
            vec![QueueAction::RemoveFromQueue(state.list_state.queue_index).into()]
        }

        // Alphabet jumping
        KeyCode::Char(c) if c.is_ascii_alphabetic() && key.modifiers.is_empty() => {
            let letter_lower = c.to_ascii_lowercase();
            let tracks: &[Track] = match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => &state.queue.tracks,
                PlaybackMode::Radio => &state.radio.tracks,
            };
            if let Some(idx) = tracks.iter().position(|t| {
                t.title
                    .chars()
                    .next()
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
