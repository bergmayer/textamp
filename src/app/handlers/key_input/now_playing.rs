//! Now Playing view key handling.

use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::app::Action;
use crate::app::state::{PlaybackMode, View};
use crate::app::AppState;
use crate::api::models::Track;

/// Handle Now Playing view keys (unified queue/radio/playlist view).
pub(super) fn handle_now_playing_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    // Get the max index based on current mode
    let get_max_index = |state: &AppState| -> usize {
        match state.playback_mode {
            PlaybackMode::Queue | PlaybackMode::None => state.queue.len().saturating_sub(1),
            PlaybackMode::Radio => state.radio.tracks.len().saturating_sub(1),
        }
    };

    match key.code {
        KeyCode::Esc => vec![Action::SetView(View::Browse)],
        KeyCode::F(1) | KeyCode::Char('?') => vec![Action::SetView(View::Help)],

        // Tab/Shift+Tab cycles through nav bar views
        KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => vec![Action::PrevView],
        KeyCode::Tab => vec![Action::NextView],

        KeyCode::Up => {
            if state.list_state.queue_index > 0 {
                state.list_state.queue_index -= 1;
            }
            vec![]
        }
        KeyCode::Down => {
            let max = get_max_index(state);
            state.list_state.queue_index = (state.list_state.queue_index + 1).min(max);
            vec![]
        }
        KeyCode::PageUp => {
            state.list_state.queue_index = state.list_state.queue_index.saturating_sub(10);
            vec![]
        }
        KeyCode::PageDown => {
            let max = get_max_index(state);
            state.list_state.queue_index = (state.list_state.queue_index + 10).min(max);
            vec![]
        }
        KeyCode::Home => {
            state.list_state.queue_index = 0;
            vec![]
        }
        KeyCode::End => {
            let max = get_max_index(state);
            state.list_state.queue_index = max;
            vec![]
        }

        KeyCode::Enter => {
            // If the selected track is already playing, switch to NowPlaying view
            let is_current = match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => state.queue_index == Some(state.list_state.queue_index),
                PlaybackMode::Radio => state.radio.track_index == Some(state.list_state.queue_index),
            };
            if is_current {
                state.now_playing_mode = crate::app::state::NowPlayingMode::NowPlaying;
                return vec![Action::LoadWaveform];
            }

            // Play selected item from queue or radio
            match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => {
                    if let Some(track) = state.queue.get(state.list_state.queue_index).cloned() {
                        state.queue_index = Some(state.list_state.queue_index);
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
            // Only allow delete in queue mode
            if state.playback_mode == PlaybackMode::Queue {
                vec![Action::RemoveFromQueue(state.list_state.queue_index)]
            } else {
                vec![]
            }
        }

        // Left/Right arrow seeking in visualizer mode (1 second increments)
        KeyCode::Left if state.now_playing_mode == crate::app::state::NowPlayingMode::NowPlaying => {
            vec![Action::SeekRelative(-1000)]
        }
        KeyCode::Right if state.now_playing_mode == crate::app::state::NowPlayingMode::NowPlaying => {
            vec![Action::SeekRelative(1000)]
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
