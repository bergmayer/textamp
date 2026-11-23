//! Pointer queue gestures share occurrence-aware queue commands with every frontend.
use super::{
    Action, AppState,
    action::QueueAction,
    state::{Modal, NowPlayingFocus, View},
};
use crate::library::track::TrackOrigin;
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

#[derive(Debug, Clone)]
pub struct Drag {
    from: usize,
    origin: (u16, u16),
    tracks: Vec<(TrackOrigin, String)>,
    phase: Phase,
}
#[derive(Debug, Clone)]
enum Phase {
    Pressed { play: bool },
    Moving { boundary: usize },
}
impl Drag {
    pub fn boundary(&self) -> Option<usize> {
        match self.phase {
            Phase::Moving { boundary } => Some(boundary),
            Phase::Pressed { .. } => None,
        }
    }
    fn valid(&self, state: &AppState) -> bool {
        self.tracks
            .iter()
            .map(|(origin, key)| (origin, key))
            .eq(state
                .playback_tracks()
                .iter()
                .map(|track| (&track.origin, &track.rating_key)))
    }
}
/// Capture a title-row gesture; metadata links and modified selection retain their handlers.
pub fn handle(event: MouseEvent, state: &mut AppState) -> Option<Vec<Action>> {
    if !matches!(state.popups.modal(), Modal::Closed)
        || state.palette.open
        || state.notifications.last_error.is_some()
        || state.view == View::Settings
        || state
            .hit_regions
            .motion_hidden
            .iter()
            .any(|region| region.contains((event.column, event.row).into()))
    {
        state.scroll.queue_drag = None;
        return None;
    }
    let Some(region) = state
        .hit_regions
        .queue_content
        .as_ref()
        .map(|regions| regions.track_list_inner)
    else {
        state.scroll.queue_drag = None;
        return None;
    };
    let len = state.playback_tracks().len();
    let visible = (usize::from(region.height) / 2).max(1);
    let offset = state.scroll.queue.unwrap_or_else(|| {
        super::handlers::helpers::calc_scroll_offset(state.list_state.queue_index, visible, len)
    });
    match event.kind {
        MouseEventKind::Down(MouseButton::Left)
            if event.modifiers == KeyModifiers::NONE
                && region.contains((event.column, event.row).into())
                && (event.row - region.y).is_multiple_of(2) =>
        {
            let from = offset + usize::from(event.row - region.y) / 2;
            if from >= len {
                return None;
            }
            let play = state.list_state.queue_index == from
                && state
                    .scroll
                    .queue_click_time
                    .is_some_and(|time| time.elapsed().as_millis() < 500);
            state.set_view(View::Queue);
            state.now_playing_focus = NowPlayingFocus::Tracks;
            state.scroll.queue = Some(offset);
            state.list_state.queue_index = from;
            if !state.queue.selected.contains(&from) {
                state.queue.selected.clear();
            }
            state.scroll.queue_drag = Some(Drag {
                from,
                origin: (event.column, event.row),
                tracks: state
                    .playback_tracks()
                    .iter()
                    .map(|track| (track.origin.clone(), track.rating_key.clone()))
                    .collect(),
                phase: Phase::Pressed { play },
            });
            Some(vec![])
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            let mut drag = state.scroll.queue_drag.take()?;
            if !drag.valid(state) {
                return Some(vec![]);
            }
            if drag.origin != (event.column, event.row) {
                let offset = if event.row <= region.y {
                    offset.saturating_sub(1)
                } else if event.row >= region.bottom().saturating_sub(1) {
                    (offset + 1).min(len.saturating_sub(visible))
                } else {
                    offset
                };
                state.scroll.queue = Some(offset);
                let boundary =
                    (offset + usize::from(event.row.saturating_sub(region.y)).div_ceil(2)).min(len);
                drag.phase = Phase::Moving { boundary };
            }
            state.scroll.queue_drag = Some(drag);
            Some(vec![])
        }
        MouseEventKind::Up(MouseButton::Left) => {
            let drag = state.scroll.queue_drag.take()?;
            if !drag.valid(state) {
                return Some(vec![]);
            }
            match drag.phase {
                Phase::Moving { boundary } => {
                    state.scroll.queue_click_time = None;
                    Some(vec![
                        QueueAction::ReorderSelection {
                            from: drag.from,
                            to: boundary,
                        }
                        .into(),
                    ])
                }
                Phase::Pressed { play } => {
                    state.scroll.queue_click_time = Some(std::time::Instant::now());
                    Some(if play {
                        super::queue_edit::play_at(state, drag.from)
                    } else {
                        vec![]
                    })
                }
            }
        }
        MouseEventKind::Down(_) => {
            state.scroll.queue_drag = None;
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{presentation::QueueRegions, state::PlaybackMode},
        library::models::Track,
    };
    fn event(kind: MouseEventKind, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            row,
            column: 6,
            modifiers: KeyModifiers::NONE,
        }
    }
    fn state() -> AppState {
        let mut state = AppState::new();
        state.playback_mode = PlaybackMode::Queue;
        state.queue.tracks = ["a", "b", "c"]
            .map(|key| Track {
                rating_key: key.into(),
                ..Default::default()
            })
            .into();
        state.hit_regions.queue_content = Some(QueueRegions {
            track_list: ratatui::layout::Rect::new(5, 5, 20, 10),
            track_list_inner: ratatui::layout::Rect::new(6, 6, 18, 8),
            art_area: Default::default(),
        });
        state
    }
    #[test]
    fn drag_defers_playback_and_rejects_a_changed_queue() {
        let mut state = state();
        assert!(
            handle(
                event(MouseEventKind::Down(MouseButton::Left), 6),
                &mut state
            )
            .unwrap()
            .is_empty()
        );
        handle(
            event(MouseEventKind::Drag(MouseButton::Left), 11),
            &mut state,
        );
        assert_eq!(
            state.scroll.queue_drag.as_ref().and_then(Drag::boundary),
            Some(3)
        );
        let actions = handle(event(MouseEventKind::Up(MouseButton::Left), 11), &mut state).unwrap();
        assert!(matches!(
            actions.as_slice(),
            [Action::Queue(QueueAction::ReorderSelection {
                from: 0,
                to: 3
            })]
        ));
        handle(
            event(MouseEventKind::Down(MouseButton::Left), 6),
            &mut state,
        );
        state.queue.tracks.swap(0, 1);
        assert!(
            handle(event(MouseEventKind::Up(MouseButton::Left), 10), &mut state)
                .unwrap()
                .is_empty()
        );
    }
}
