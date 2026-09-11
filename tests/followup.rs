use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::TestBackend, Terminal};
use std::time::Duration;
use textamp::app::action::QueueAction;
use textamp::app::dispatch::handle_core_event;
use textamp::app::event::RadioEvent;
use textamp::app::handlers::{key_input, mouse_input};
use textamp::app::state::{
    ActiveStation, BrowseCategory, BrowseColumn, BrowseItem, PlaybackMode, View,
};
use textamp::app::tasks::{TaskLease, TaskSession};
use textamp::app::{AppState, Event};

use tokio::sync::mpsc;

fn track(key: &str) -> textamp::library::models::Track {
    serde_json::from_value(serde_json::json!({"ratingKey": key, "title": key})).unwrap()
}

#[tokio::test]
async fn replacing_lease_cancels_work_without_waiting_for_network_timeout() {
    let task = tokio::spawn(std::future::pending::<()>());
    let lease = TaskLease::new(&task);
    let clone = lease.clone();
    drop(lease);
    assert!(!task.is_finished());
    drop(clone);
    assert!(tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap_err()
        .is_cancelled());
}

#[tokio::test]
async fn background_panics_reach_the_event_loop() {
    let (tx, mut rx) = mpsc::channel(8);
    let session = TaskSession::new(tx);
    session
        .run(async {
            let task = textamp::app::tasks::spawn(async { panic!("intentional worker failure") });
            assert!(task.await.unwrap_err().is_panic());
        })
        .await;
    assert!(matches!(rx.recv().await, Some(Event::WorkerFailed(_))));
}

#[test]
fn rendering_is_read_only_and_click_preserves_the_viewport() {
    let mut state = AppState::new();
    state.view = View::Browse;
    state.browse_category = BrowseCategory::Library;
    state.category_column_focused = false;
    state.terminal_width = 120;
    state.terminal_height = 30;
    let items = (0..80)
        .map(|i| BrowseItem::Artist {
            key: i.to_string(),
            title: format!("Artist {i:02}"),
            thumb: None,
            is_placeholder: false,
        })
        .collect();
    state
        .artist_nav
        .columns
        .push(BrowseColumn::new("artists", items));
    state.artist_nav.columns[0].selected_index = 35;
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    let mut feedback = None;
    terminal
        .draw(|frame| feedback = Some(textamp::ui::render(frame, &state)))
        .unwrap();
    assert!(state.hit_regions.miller_columns.is_none());
    assert!(state.marquee.selection_key.is_empty());
    state.apply_render_feedback(feedback.take().unwrap());
    let column = state.hit_regions.miller_columns.as_ref().unwrap().columns[0].clone();
    let before = terminal.backend().buffer().clone();
    mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: column.inner.x + 3,
            row: column.inner.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        &mut state,
    );
    assert!(
        state.scroll.browse.is_some(),
        "mouse selection must pin scroll"
    );
    let pinned = state.scroll.browse;
    terminal
        .draw(|frame| feedback = Some(textamp::ui::render(frame, &state)))
        .unwrap();
    state.apply_render_feedback(feedback.take().unwrap());
    let after = terminal.backend().buffer();
    // Selection changes styling, not which text occupies a viewport row.
    for y in column.inner.y..column.inner.bottom() {
        for x in column.inner.x..column.inner.right() {
            assert_eq!(before[(x, y)].symbol(), after[(x, y)].symbol());
        }
    }
    assert_eq!(state.scroll.browse, pinned);
    key_input::handle_key(
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        &mut state,
        &Default::default(),
    );
    assert!(
        state.scroll.browse.is_none(),
        "keyboard navigation releases click pin"
    );
}

#[tokio::test]
async fn radio_conversion_and_undo_preserve_current_track_and_station() {
    let mut state = AppState::new();
    state.set_playback_mode(PlaybackMode::Radio);
    state.radio.tracks = vec![track("first"), track("second")];
    state.radio.track_index = Some(1);
    state.radio.refill = textamp::app::state::RadioRefill::Prefetching;
    state.radio.active_station = Some(ActiveStation {
        source: textamp::library::models::RadioSource::Station("station".into()),
        title: "Station".into(),
    });
    state.queue.undo_snapshot = Some(state.convert_radio_to_queue("reorder"));
    assert_eq!(state.current_track().unwrap().rating_key, "second");
    assert!(state.radio.tracks.is_empty());
    state.queue.tracks.reverse();
    let (tx, _rx) = mpsc::channel(8);
    textamp::app::handlers::dispatch_queue::dispatch(
        &tx,
        QueueAction::UndoLastRemix,
        &mut state,
        &mut textamp::audio::AudioPlayer::new_without_audio(),
    )
    .await
    .unwrap();
    assert_eq!(state.playback_mode, PlaybackMode::Radio);
    assert_eq!(state.current_track().unwrap().rating_key, "second");
    assert_eq!(
        state
            .radio
            .active_station
            .as_ref()
            .unwrap()
            .source
            .station_key(),
        Some("station")
    );
    assert!(state.queue.tracks.is_empty());
    assert!(
        state.radio.refill == textamp::app::state::RadioRefill::Idle,
        "undo must not resurrect a cancelled in-flight refill"
    );
}

#[test]
fn old_station_results_cannot_interrupt_a_new_queue_and_empty_station_nav_is_safe() {
    let mut state = AppState::new();
    state.set_playback_mode(PlaybackMode::Radio);
    let old = state.radio_generation;
    state.set_playback_mode(PlaybackMode::Queue);
    state.queue.tracks.push(track("queue"));
    state.queue.index = Some(0);
    let event = || RadioEvent::StationTracksLoaded {
        station: ActiveStation {
            source: textamp::library::models::RadioSource::Station("station".into()),
            title: "Station".into(),
        },
        tracks: vec![track("radio")],
        time_travel_decades: vec![],
        time_travel_index: None,
    };
    let (tx, _rx) = mpsc::channel(8);
    handle_core_event(
        Event::RadioResult {
            generation: old,
            navigation_generation: None,
            event: Box::new(event().into()),
        },
        &mut state,
        &tx,
    );
    assert_eq!(state.current_track().unwrap().rating_key, "queue");
    // Current result with no station navigation: Alt+R need not visit Stations first.
    handle_core_event(event().into(), &mut state, &tx);
    assert_eq!(state.current_track().unwrap().rating_key, "radio");
}

#[tokio::test]
async fn queue_moves_preserve_block_order_playing_track_and_boundaries() {
    let mut state = AppState::new();
    state.set_playback_mode(PlaybackMode::Queue);
    state.queue.tracks = ["a", "b", "c", "d", "e"].into_iter().map(track).collect();
    state.queue.index = Some(2);
    state.queue.selected.extend([1, 2]);
    state.list_state.queue_index = 2;
    let (tx, _rx) = mpsc::channel(8);

    let mut audio = textamp::audio::AudioPlayer::new_without_audio();
    for action in [
        QueueAction::MoveSelectedTracksUp,
        QueueAction::MoveSelectedTracksUp,
    ] {
        textamp::app::handlers::dispatch_queue::dispatch(&tx, action, &mut state, &mut audio)
            .await
            .unwrap();
        let keys: Vec<_> = state
            .queue
            .tracks
            .iter()
            .map(|track| track.rating_key.as_str())
            .collect();
        assert_eq!(keys, ["b", "c", "a", "d", "e"]);
        assert_eq!(state.current_track().unwrap().rating_key, "c");
    }
    textamp::app::handlers::dispatch_queue::dispatch(
        &tx,
        QueueAction::MoveSelectedTracksDown,
        &mut state,
        &mut audio,
    )
    .await
    .unwrap();
    assert_eq!(state.queue.tracks[2].rating_key, "c");
    assert_eq!(state.current_track().unwrap().rating_key, "c");
    textamp::app::handlers::dispatch_queue::dispatch(
        &tx,
        QueueAction::MoveQueueTrack { from: 2, to: 4 },
        &mut state,
        &mut audio,
    )
    .await
    .unwrap();
    assert_eq!(state.queue.index, Some(4));
    assert_eq!(state.current_track().unwrap().rating_key, "c");
}

#[tokio::test]
async fn backing_out_of_station_navigation_does_not_cancel_active_radio_refill() {
    let mut state = AppState::new();
    state.set_playback_mode(PlaybackMode::Radio);
    state.radio.tracks = vec![track("first")];
    state.radio.track_index = Some(0);
    state.radio.refill = textamp::app::state::RadioRefill::Prefetching;
    let playback_generation = state.radio_generation;
    let old_navigation = state.station_navigation_generation;
    let (tx, _rx) = mpsc::channel(8);

    textamp::app::handlers::dispatch_radio::dispatch(
        &tx,
        textamp::app::action::RadioAction::NavigateStationsBack,
        &mut state,
        &mut textamp::audio::AudioPlayer::new_without_audio(),
    )
    .await
    .unwrap();
    assert_eq!(state.radio_generation, playback_generation);
    let stale_navigation = RadioEvent::StationChildrenLoaded {
        station_key: "old".into(),
        station_title: "Old".into(),
        children: vec![],
    };
    handle_core_event(
        Event::RadioResult {
            generation: playback_generation,
            navigation_generation: Some(old_navigation),
            event: Box::new(stale_navigation.into()),
        },
        &mut state,
        &tx,
    );
    assert!(state.station_nav.columns.is_empty());
    let refill = RadioEvent::RadioTracksLoaded {
        result: Ok(vec![track("second")]),
        time_travel_index: None,
    };
    handle_core_event(
        Event::RadioResult {
            generation: playback_generation,
            navigation_generation: None,
            event: Box::new(refill.into()),
        },
        &mut state,
        &tx,
    );
    assert_eq!(state.radio.tracks.len(), 2);
    assert!(state.radio.refill == textamp::app::state::RadioRefill::Idle);
}

#[tokio::test]
async fn accepted_blocking_work_survives_session_cancel_and_has_a_bounded_drain() {
    let session = TaskSession::default();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_completed = completed.clone();
    session
        .run(async {
            textamp::app::tasks::spawn_blocking(move || {
                let _ = started_tx.send(());
                release_rx.recv().unwrap();
                worker_completed.store(true, std::sync::atomic::Ordering::Release);
            });
        })
        .await;
    started_rx.await.unwrap();
    drop(session);
    assert!(!textamp::app::tasks::finish_blocking(Duration::from_millis(20)).await);
    release_tx.send(()).unwrap();
    assert!(textamp::app::tasks::finish_blocking(Duration::from_secs(1)).await);
    assert!(completed.load(std::sync::atomic::Ordering::Acquire));
}

#[test]
fn queue_drag_rotation_matches_documented_order_for_every_small_move() {
    use textamp::app::state::QueueState;
    for from in 0..7 {
        for to in 0..7 {
            for playing in 0..5 {
                let original = ["a", "b", "c", "d", "e"];
                let mut expected = original.to_vec();
                if from < expected.len() && from != to {
                    let item = expected.remove(from);
                    expected.insert(to.min(expected.len()), item);
                }
                let mut queue = QueueState {
                    tracks: original.into_iter().map(track).collect(),
                    index: Some(playing),
                    ..Default::default()
                };
                queue.move_track(from, to);
                let keys: Vec<_> = queue
                    .tracks
                    .iter()
                    .map(|track| track.rating_key.as_str())
                    .collect();
                assert_eq!(keys, expected, "move {from} -> {to}");
                assert_eq!(
                    queue.tracks[queue.index.unwrap()].rating_key,
                    original[playing]
                );
            }
        }
    }
}
