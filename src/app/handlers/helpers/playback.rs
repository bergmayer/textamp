//! Playback helpers: track playing, server reporting, radio fetching.

use crate::app::state::{PlaybackMode, View};
use crate::app::{AppState, Event};
use crate::audio::{AudioEvent, AudioPlayer};
use crate::library::models::Track;

use tokio::sync::mpsc;

/// Clear presentation and retry state when the selected playback item changes.
pub(crate) fn reset_current_track(state: &mut AppState) {
    state.sources.nav_tasks.remove("artwork");
    state.artwork.fetch = Default::default();
    state.artwork.current_thumb = None;
    state.artwork.current_data = None;
    state.waveform = Default::default();
    state.spectrogram = Default::default();
    state.seek_drag = None;
    state.playback.position_ms = 0;
    state.playback.duration_ms = state.current_track().map_or(0, Track::duration_ms);
    state.playback.playback_started_at = None;
    state.consecutive_playback_errors = 0;
}

/// Play the downloaded copy when available, otherwise use the active library.
pub fn play_current_track(
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,
    audio: &mut AudioPlayer,
) {
    state.play_recorded = false;
    if crate::app::downloads::play(state, audio) {
        crate::app::queue_modes::prefetch(state, event_tx);
        return;
    }
    if state.downloads.is_offline() {
        audio.stop();
        state.playback.status = crate::app::state::PlayStatus::Stopped;
        state.set_status("Not downloaded · Unavailable offline".into());
        return;
    }
    if state.sources.active.navidrome().is_some() {
        crate::app::sources::navidrome::play(state, audio, event_tx);
    } else if let Some(track) = state.current_track().cloned() {
        crate::app::sources::play(state, audio, event_tx, track);
    }
    crate::app::queue_modes::prefetch(state, event_tx);
}

/// Compute the list of upcoming tracks to pre-fetch from current state.
pub fn get_upcoming_tracks(state: &AppState) -> Vec<Track> {
    let index = match state.playback_mode {
        PlaybackMode::Queue | PlaybackMode::None => state.queue.index,
        PlaybackMode::Radio => state.radio.track_index,
    };
    let tracks = state.playback_tracks();
    index
        .and_then(|index| tracks.get(index.saturating_add(1)..))
        .map(|upcoming| upcoming[..upcoming.len().min(10)].to_vec())
        .unwrap_or_default()
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
            if event_tx.send(ev.into()).await.is_err() {
                break;
            }
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
    crate::app::queue_edit::record(state, "Play track");
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

/// Replace the queue, open its view, and start playback at the selected index.
pub fn queue_and_play(
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,

    audio: &mut AudioPlayer,
    tracks: Vec<Track>,
    play_idx: usize,
) {
    let current_changed = state
        .current_track()
        .zip(tracks.get(play_idx))
        .is_none_or(|(old, new)| old.rating_key != new.rating_key || old.origin != new.origin);
    if state.playback_mode == PlaybackMode::Radio
        || !state
            .queue
            .tracks
            .iter()
            .map(|track| (&track.rating_key, &track.origin))
            .eq(tracks
                .iter()
                .map(|track| (&track.rating_key, &track.origin)))
    {
        crate::app::queue_edit::record(state, "Replace Queue");
    }
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
    if current_changed {
        reset_current_track(state);
    } else {
        state.consecutive_playback_errors = 0;
    }
    play_current_track(event_tx, state, audio);
}

/// Insert tracks into the queue immediately after the currently playing track.
/// If no track is playing, inserts at the beginning of the queue.
/// Does NOT start playback — just modifies the queue.
pub fn insert_tracks_next(state: &mut AppState, tracks: Vec<Track>) -> usize {
    if tracks.is_empty() {
        return 0;
    }
    crate::app::queue_edit::record(state, "Insert tracks next");
    state.queue.original.clear();
    state.queue.sort_mode = crate::app::state::QueueSortMode::QueueOrder;

    let (buffer, playing) = state.playback_buffer_mut();
    let insert_pos = playing.map(|idx| idx + 1).unwrap_or(0).min(buffer.len());
    let added = tracks.len();
    buffer.splice(insert_pos..insert_pos, tracks);
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
        crate::app::queue_edit::consume(state, index);
        play_current_track(event_tx, state, audio);
        if state.radio.tracks.len().saturating_sub(index + 1) <= 1 {
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
