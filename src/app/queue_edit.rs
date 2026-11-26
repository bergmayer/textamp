//! Queue reordering and occurrence identity, independent of frontend gestures.
use crate::app::{
    AppState,
    state::{PlaybackMode, QueueContents, QueueSnapshot},
};
use std::collections::BTreeSet;

/// A temporary guard for a source-owned edit across one reducer turn.
pub(crate) struct PendingEdit {
    request_id: u64,
    tracks: Vec<String>,
}

pub(crate) fn pending(state: &AppState) -> Option<PendingEdit> {
    state
        .sources
        .nav_tasks
        .get("remix")
        .map(|(request_id, _)| PendingEdit {
            request_id: *request_id,
            tracks: state
                .playback_tracks()
                .iter()
                .map(|track| track.rating_key.clone())
                .collect(),
        })
}

pub(crate) fn cancel_changed(previous: Option<PendingEdit>, state: &mut AppState) {
    let Some(previous) = previous else {
        return;
    };
    if state
        .sources
        .nav_tasks
        .get("remix")
        .is_some_and(|(id, _)| *id == previous.request_id)
        && !state
            .playback_tracks()
            .iter()
            .map(|track| &track.rating_key)
            .eq(previous.tracks.iter())
    {
        state.sources.nav_tasks.remove("remix");
        state.set_status("Queue edit cancelled".into());
    }
}

/// Capture undo only when a successful edit is about to change the active buffer.
pub fn snapshot(state: &AppState, description: &str) -> QueueSnapshot {
    QueueSnapshot {
        recorded_at: std::time::Instant::now(),
        contents: if state.playback_mode == PlaybackMode::Radio {
            QueueContents::Radio(Box::new(state.radio.clone()))
        } else {
            QueueContents::Queue {
                tracks: state.queue.tracks.clone(),
                index: state.queue.index,
            }
        },
        description: description.into(),
    }
}

pub(crate) fn record(state: &mut AppState, description: &str) {
    state.queue.undo_snapshot = Some(snapshot(state, description));
}

pub fn play_at(state: &AppState, index: usize) -> Vec<crate::app::Action> {
    if index >= state.playback_tracks().len() {
        return vec![];
    }
    if state.playback_mode == PlaybackMode::Radio {
        vec![crate::app::action::RadioAction::JumpToRadioTrack(index).into()]
    } else {
        vec![crate::app::action::QueueAction::JumpToQueueIndex(index).into()]
    }
}

pub fn reorder(state: &mut AppState, from: usize, to: usize) {
    let len = state.playback_tracks().len();
    if from >= len || to > len || from == to {
        return;
    }
    let selected = if state.queue.selected.contains(&from) {
        state.queue.selected.range(..len).copied().collect()
    } else {
        BTreeSet::from([from])
    };
    if selected.contains(&to) {
        return;
    }
    record(state, "Reorder tracks");
    let moving = selected.iter().copied().collect::<Vec<_>>();
    let mut wanted = (0..len)
        .filter(|i| !selected.contains(i))
        .collect::<Vec<_>>();
    let destination = to.saturating_sub(selected.range(..to).count());
    wanted.splice(destination..destination, moving.iter().copied());
    let mut destinations = vec![0; len];
    for (destination, original) in wanted.iter().copied().enumerate() {
        destinations[original] = destination;
    }
    let (buffer, playing) = state.playback_buffer_mut();
    let mut tracks: Vec<_> = std::mem::take(buffer).into_iter().enumerate().collect();
    tracks.sort_unstable_by_key(|(original, _)| destinations[*original]);
    *buffer = tracks.into_iter().map(|(_, track)| track).collect();
    *playing = playing.and_then(|index| destinations.get(index).copied());
    state.queue.original.clear();
    state.queue.selected = wanted
        .iter()
        .enumerate()
        .filter_map(|(i, old)| selected.contains(old).then_some(i))
        .collect();
    state.list_state.queue_index = wanted
        .iter()
        .position(|i| *i == from)
        .unwrap_or(destination);
    state.queue.selection_anchor = Some(state.list_state.queue_index);
}
/// A deletion either preserves the audible occurrence or requires a playback transition.
#[derive(Debug, PartialEq, Eq)]
pub enum Removal {
    Unchanged,
    RetainedPlayback,
    RemovedPlayback,
}

/// Delete occurrence indices in one pass, keeping undo and selection identity intact.
pub fn remove(state: &mut AppState, indices: &BTreeSet<usize>) -> Removal {
    let len = state.playback_tracks().len();
    let selected: BTreeSet<_> = indices.range(..len).copied().collect();
    if selected.is_empty() {
        return Removal::Unchanged;
    }
    record(state, "Remove tracks");
    remove_selected(state, &selected)
}
pub(crate) fn consume(state: &mut AppState, index: usize) {
    remember_previous(state, index);
    remove_selected(state, &BTreeSet::from([index]));
    invalidate_playback_undo(state);
}
fn remember_previous(state: &mut AppState, index: usize) {
    if let Some(track) = state.playback_tracks().get(index).cloned() {
        state.queue.previous.push_back(track);
        if state.queue.previous.len() > 512 {
            state.queue.previous.pop_front();
        }
    }
}
fn invalidate_playback_undo(state: &mut AppState) {
    state.queue.shuffle_undo_queue = None;
    state.queue.shuffle_undo_index = None;
    state.queue.undo_snapshot = None;
}
pub(crate) fn before_jump(state: &mut AppState, destination: usize) -> usize {
    let current = if state.playback_mode == PlaybackMode::Radio {
        state.radio.track_index
    } else {
        state.queue.index
    };
    if destination > 0 && current.is_none_or(|index| index < destination) {
        if let Some(index) = current.filter(|_| state.play_recorded) {
            remember_previous(state, index);
        }
        let (buffer, playing) = state.playback_buffer_mut();
        buffer.drain(..destination);
        *playing = Some(0);
        state.queue.selected = state
            .queue
            .selected
            .iter()
            .filter_map(|index| index.checked_sub(destination))
            .collect();
        state.queue.selection_anchor = state
            .queue
            .selection_anchor
            .and_then(|index| index.checked_sub(destination));
        state.queue.original.clear();
        state.queue.sort_mode = crate::app::state::QueueSortMode::QueueOrder;
        invalidate_playback_undo(state);
        return 0;
    }
    if let Some(index) = current
        && index != destination
        && state.play_recorded
        && state.queue.repeat != crate::app::state::RepeatMode::Queue
    {
        consume(state, index);
    }
    destination
}
fn remove_selected(state: &mut AppState, selected: &BTreeSet<usize>) -> Removal {
    let (buffer, playing) = state.playback_buffer_mut();
    let removed_playback = playing.is_some_and(|index| selected.contains(&index));
    let current = playing.map(|index| index - selected.range(..index).count());
    let mut index = 0;
    buffer.retain(|_| {
        let keep = !selected.contains(&index);
        index += 1;
        keep
    });
    *playing = current
        .filter(|_| !buffer.is_empty())
        .map(|index| index.min(buffer.len() - 1));
    let remaining = buffer.len();
    state.queue.selected = state
        .queue
        .selected
        .iter()
        .filter(|index| !selected.contains(index))
        .map(|index| index - selected.range(..index).count())
        .collect();
    state.list_state.queue_index = state
        .list_state
        .queue_index
        .saturating_sub(selected.range(..state.list_state.queue_index).count())
        .min(remaining.saturating_sub(1));
    state.queue.original.clear();
    if removed_playback {
        Removal::RemovedPlayback
    } else {
        Removal::RetainedPlayback
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::models::Track;
    #[test]
    fn each_successful_remix_replaces_undo_and_failed_edits_leave_it_intact() {
        use crate::app::{
            action::{AsyncBatchOutcome, QueueAction},
            handlers::dispatch_queue,
        };
        let mut state = AppState::new();
        state.queue.tracks = ["a", "b", "c"]
            .map(|key| Track {
                rating_key: key.into(),
                ..Default::default()
            })
            .into();
        state.queue.index = Some(0);
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        let (events, _) = tokio::sync::mpsc::channel(8);
        dispatch_queue::dispatch(
            &events,
            QueueAction::RemixEnemiesReady(AsyncBatchOutcome::new(
                vec![(
                    0,
                    vec![Track {
                        rating_key: "enemy".into(),
                        ..Default::default()
                    }],
                )],
                1,
            )),
            &mut state,
            &mut audio,
        )
        .unwrap();
        assert_eq!(
            state
                .queue
                .tracks
                .iter()
                .map(|track| track.rating_key.as_str())
                .collect::<Vec<_>>(),
            ["a", "enemy", "b", "c"]
        );
        dispatch_queue::dispatch(
            &events,
            QueueAction::RemixOppositeDayReady(AsyncBatchOutcome::new(
                vec![(
                    3,
                    Track {
                        rating_key: "opposite".into(),
                        ..Default::default()
                    },
                )],
                1,
            )),
            &mut state,
            &mut audio,
        )
        .unwrap();
        let Some(QueueSnapshot {
            contents: QueueContents::Queue { tracks, .. },
            ..
        }) = &state.queue.undo_snapshot
        else {
            panic!("missing undo");
        };
        assert_eq!(
            tracks
                .iter()
                .map(|track| track.rating_key.as_str())
                .collect::<Vec<_>>(),
            ["a", "enemy", "b", "c"]
        );
        let recorded = state.queue.undo_snapshot.as_ref().unwrap().recorded_at;
        dispatch_queue::dispatch(
            &events,
            QueueAction::RemixOppositeDayReady(AsyncBatchOutcome::new(vec![], 1)),
            &mut state,
            &mut audio,
        )
        .unwrap();
        assert_eq!(
            state.queue.undo_snapshot.as_ref().unwrap().recorded_at,
            recorded
        );
        assert_eq!(state.queue.index, Some(0));
    }

    #[test]
    fn deleting_a_duplicate_preserves_the_audible_occurrence_and_undo() {
        let mut state = AppState::new();
        state.queue.tracks = ["same", "other", "same", "last"]
            .map(|key| Track {
                rating_key: key.into(),
                ..Default::default()
            })
            .into();
        state.queue.index = Some(2);
        assert_eq!(
            remove(&mut state, &BTreeSet::from([0])),
            Removal::RetainedPlayback
        );
        assert_eq!(state.queue.index, Some(1));
        assert_eq!(
            remove(&mut state, &BTreeSet::from([1])),
            Removal::RemovedPlayback
        );
        assert_eq!(state.queue.index, Some(1));
        assert_eq!(state.queue.tracks[1].rating_key, "last");
        assert!(state.queue.undo_snapshot.is_some());
        assert_eq!(
            remove(&mut state, &BTreeSet::from([0, 1])),
            Removal::RemovedPlayback
        );
        assert!(state.queue.tracks.is_empty());
        assert_eq!(state.queue.index, None);
    }
    #[test]
    fn forward_jump_drops_the_prefix_and_remembers_only_the_played_occurrence() {
        for mode in [PlaybackMode::Queue, PlaybackMode::Radio] {
            for recorded in [true, false] {
                let mut state = AppState::new();
                state.set_playback_mode(mode);
                state.queue.repeat = crate::app::state::RepeatMode::Queue;
                let (tracks, index) = state.playback_buffer_mut();
                *tracks = ["A", "B", "A", "D"]
                    .map(|title| Track {
                        title: title.into(),
                        ..Default::default()
                    })
                    .into();
                *index = Some(0);
                state.play_recorded = recorded;
                state.queue.selected.extend([1, 3]);
                state.queue.selection_anchor = Some(1);
                assert_eq!(before_jump(&mut state, 2), 0);
                let titles: Vec<_> = state
                    .playback_tracks()
                    .iter()
                    .map(|t| t.title.as_str())
                    .collect();
                assert_eq!(titles, ["A", "D"]);
                assert_eq!(state.queue.previous.len(), usize::from(recorded));
                assert_eq!(
                    state.queue.previous.front().map(|t| t.title.as_str()),
                    recorded.then_some("A")
                );
                assert_eq!(state.queue.selected, BTreeSet::from([1]));
                assert_eq!(state.queue.selection_anchor, None);
            }
        }
    }
    #[test]
    fn playing_a_duplicate_targets_its_existing_occurrence() {
        let mut state = AppState::new();
        state.queue.tracks = ["same", "other", "same"]
            .map(|key| Track {
                rating_key: key.into(),
                ..Default::default()
            })
            .into();
        state.queue.index = Some(0);
        let actions = play_at(&state, 2);
        assert!(matches!(
            actions.as_slice(),
            [crate::app::Action::Queue(
                crate::app::action::QueueAction::JumpToQueueIndex(2)
            )]
        ));
        assert_eq!(
            state
                .queue
                .tracks
                .iter()
                .map(|t| t.rating_key.as_str())
                .collect::<Vec<_>>(),
            ["same", "other", "same"]
        );
        assert!(play_at(&state, 3).is_empty());
    }
    #[test]
    fn group_drag_keeps_order_and_playing_duplicate_occurrence() {
        let mut state = AppState::new();
        state.queue.tracks = ["a", "same", "b", "same", "c", "d"]
            .map(|title| Track {
                title: title.into(),
                rating_key: title.into(),
                ..Default::default()
            })
            .into();
        state.queue.index = Some(3);
        state.queue.selected = BTreeSet::from([1, 3]);
        reorder(&mut state, 3, 5);
        assert_eq!(
            state
                .queue
                .tracks
                .iter()
                .map(|t| t.title.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c", "same", "same", "d"]
        );
        assert_eq!(state.queue.index, Some(4));
        assert_eq!(state.queue.selected, BTreeSet::from([3, 4]));
        assert!(state.queue.undo_snapshot.is_some());
        reorder(&mut state, 4, 6);
        assert_eq!(
            state
                .queue
                .tracks
                .iter()
                .map(|t| t.title.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c", "d", "same", "same"]
        );
        assert_eq!(state.queue.index, Some(5));
        assert_eq!(state.queue.selected, BTreeSet::from([4, 5]));
    }
}
