//! Playback helpers: track playing, server reporting, radio fetching.

use crate::app::state::{PlaybackMode, View};
use crate::app::{AppState, Event};
use crate::audio::{AudioEvent, AudioPlayer};
use crate::library::models::Track;

use tokio::sync::mpsc;

/// Look up artist artwork as a fallback when a track has no thumb.
pub fn play_current_track(
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,
    audio: &mut AudioPlayer,
) {
    if state.sources.active.navidrome().is_some() {
        crate::app::sources::navidrome::play(state, audio, event_tx);
    } else if let Some(track) = state.current_track().cloned() {
        crate::app::sources::play(state, audio, event_tx, track);
    }
}

/// Compute the list of upcoming tracks to pre-fetch from current state.
pub fn get_upcoming_tracks(state: &AppState) -> Vec<Track> {
    match state.playback_mode {
        PlaybackMode::Queue | PlaybackMode::None => {
            if let Some(idx) = state.queue.index {
                let start = idx + 1;
                let end = (start + 10).min(state.queue.tracks.len());
                if start < state.queue.tracks.len() {
                    return state.queue.tracks[start..end].to_vec();
                }
            }
            vec![]
        }
        PlaybackMode::Radio => {
            if let Some(idx) = state.radio.track_index {
                let start = idx + 1;
                let end = (start + 10).min(state.radio.tracks.len());
                if start < state.radio.tracks.len() {
                    return state.radio.tracks[start..end].to_vec();
                }
            }
            vec![]
        }
    }
}

/// Create an adapter channel that converts `AudioEvent` to app `Event`.
///
/// Returns a sender that the audio player can use. The spawned task
/// forwards events to the app event loop.
pub(crate) fn audio_event_adapter(event_tx: &mpsc::Sender<Event>) -> mpsc::Sender<AudioEvent> {
    let (audio_tx, mut audio_rx) = mpsc::channel::<AudioEvent>(32);
    let event_tx = event_tx.clone();
    crate::app::tasks::spawn(async move {
        while let Some(ev) = audio_rx.recv().await {
            let _ = event_tx.send(ev.into()).await;
        }
    });
    audio_tx
}

/// Play a track, prepending it to the queue and preserving upcoming tracks.
pub fn play_track(
    event_tx: &mpsc::Sender<Event>,
    track: Track,
    state: &mut AppState,

    audio: &mut AudioPlayer,
) {
    // Report stop for currently playing track before switching

    // Generate new session ID for this playback context

    // Migrate radio tracks to queue before clearing radio mode
    if state.playback_mode == PlaybackMode::Radio {
        state.queue.tracks = state.radio.tracks.clone();
        state.queue.index = state.radio.track_index;
        state.radio.clear();
    }

    // Prepend new track at front of queue
    state.queue.tracks.insert(0, track);
    state.queue.index = Some(0);
    state.queue.selected.clear();
    state.queue.original.clear();
    state.queue.sort_mode = crate::app::state::QueueSortMode::QueueOrder;
    state.set_playback_mode(PlaybackMode::Queue);

    // Scroll queue view to top
    state.list_state.queue_index = 0;
    play_current_track(event_tx, state, audio);
}

/// Replace the active queue with `tracks`, start playback at `play_idx`,
/// and switch to the Queue view.
///
/// This consolidates the common queue-management sequence shared by all
/// "play tracks" handlers (Miller columns, folders, album groups, etc.):
///   1. Clear radio mode if active
///   2. Drain played tracks to history
///   3. Flush the audio pre-fetch cache
///   4. Splice new tracks into the queue
///   5. Set queue index, playback mode, list state
///   6. Switch to Now Playing
///   7. Start playback
pub fn queue_and_play(
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,

    audio: &mut AudioPlayer,
    tracks: Vec<Track>,
    play_idx: usize,
) {
    if state.playback_mode == PlaybackMode::Radio {
        state.radio.clear();
    }
    state.queue.tracks = tracks;
    state.queue.index = Some(play_idx);
    state.queue.selected.clear();
    state.queue.original.clear();
    state.queue.sort_mode = crate::app::state::QueueSortMode::QueueOrder;
    state.set_playback_mode(PlaybackMode::Queue);
    state.list_state.queue_index = play_idx;
    state.set_view(View::Queue);
    play_current_track(event_tx, state, audio);
}

/// Insert tracks into the queue immediately after the currently playing track.
/// If no track is playing, inserts at the beginning of the queue.
/// Does NOT start playback — just modifies the queue.
pub fn insert_tracks_next(state: &mut AppState, tracks: Vec<Track>) -> usize {
    // Convert radio to queue if needed
    if state.playback_mode == PlaybackMode::Radio {
        state.queue.tracks = state.radio.tracks.clone();
        state.queue.index = state.radio.track_index;
        state.set_playback_mode(PlaybackMode::Queue);
        state.radio.clear();
        if let Some(idx) = state.queue.index {
            state.list_state.queue_index = idx;
        }
    }

    state.queue.original.clear();
    state.queue.sort_mode = crate::app::state::QueueSortMode::QueueOrder;

    let insert_pos = state.queue.index.map(|idx| idx + 1).unwrap_or(0);
    let added = tracks.len();
    state.queue.tracks.splice(insert_pos..insert_pos, tracks);
    added
}

/// Advance a radio queue for either local or remote output. Prefetch completion
/// never advances playback unless Next has actually reached the buffer's end.
pub fn advance_radio(
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,

    audio: &mut AudioPlayer,
) -> bool {
    use crate::app::state::RadioRefill;
    let Some(index) = state.radio.track_index else {
        return false;
    };
    if index + 1 < state.radio.tracks.len() {
        state.radio.track_index = Some(index + 1);
        play_current_track(event_tx, state, audio);
        if state.radio.tracks.len().saturating_sub(index + 2) < 5 {
            crate::app::sources::radio::refill(event_tx, state);
        }
        true
    } else {
        crate::app::sources::radio::refill(event_tx, state);
        if state.radio.refill != RadioRefill::Idle {
            state.radio.refill = RadioRefill::Waiting;
        }
        false
    }
}
