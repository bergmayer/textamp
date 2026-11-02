use std::time::{Duration, Instant};
use textamp::app::action::PlaybackAction;
use textamp::app::dispatch::{dispatch_action, handle_core_event};
use textamp::app::event::ArtworkEvent;
use textamp::app::handlers::{key_input, lazy_art};
use textamp::app::sources::{dialogs::Dialog, FileAction, MediaPreparation};
use textamp::app::state::{
    ArtworkFetch, ArtworkNavigation, ConfirmAction, ConfirmDialog, Modal, PlayStatus, PlaybackMode,
    ART_LOAD_PAUSE_MS,
};
use textamp::app::{AppState, Event};
use textamp::config::Config;
use textamp::library::models::Track;

#[test]
fn modal_replacement_and_confirmation_dismissal_preserve_only_library_parent() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut state = AppState::new();
    let confirm = || ConfirmDialog {
        title: "Clear?".into(),
        message: "Fixture".into(),
        selected_yes: false,
        on_confirm: ConfirmAction::ClearLibraryCache,
    };
    state.popups.set_search_active(true);
    state
        .popups
        .set_library_dialog(Some(Dialog::Add { selected: 2 }));
    assert!(!state.popups.search_active());
    state.popups.set_confirm_dialog(Some(confirm()));
    assert!(matches!(state.popups.modal(), Modal::Confirmation { .. }));
    key_input::handle_key(
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        &mut state,
        &Config::default(),
    );
    assert!(matches!(state.popups.modal(), Modal::Library(_)));
    assert!(matches!(
        state.popups.library_dialog(),
        Some(Dialog::Add { selected: 2 })
    ));
    state.popups.set_sound_output_active(true);
    state.popups.set_confirm_dialog(Some(confirm()));
    assert!(!state.popups.sound_output_active());
    key_input::handle_key(
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        &mut state,
        &Config::default(),
    );
    assert!(matches!(state.popups.modal(), Modal::Closed));
}

#[test]
fn deferred_artwork_settles_once_and_stale_results_do_not_complete_current_fetch() {
    let mut state = AppState::new();
    state.artwork.navigation = ArtworkNavigation::Moving {
        last_motion: Instant::now(),
    };
    assert!(lazy_art::settle(&mut state).is_none());
    state.artwork.navigation = ArtworkNavigation::Moving {
        last_motion: Instant::now() - Duration::from_millis(ART_LOAD_PAUSE_MS + 1),
    };
    assert!(lazy_art::settle(&mut state).is_some());
    assert!(lazy_art::settle(&mut state).is_none());
    state.artwork.fetch = ArtworkFetch::Loading {
        thumb: "new".into(),
    };
    state.artwork.grid_generation = 4;
    let (tx, _) = tokio::sync::mpsc::channel(4);
    for (generation, thumb) in [(3, "new"), (4, "old")] {
        handle_core_event(
            Event::Artwork(ArtworkEvent::ArtworkLoaded {
                generation,
                thumb_path: thumb.into(),
                data: vec![1],
            }),
            &mut state,
            &tx,
        );
        assert!(state.artwork.fetch.is_loading());
        assert!(state.artwork.current_data.is_none());
    }
    handle_core_event(
        Event::Artwork(ArtworkEvent::ArtworkLoaded {
            generation: 4,
            thumb_path: "new".into(),
            data: vec![2],
        }),
        &mut state,
        &tx,
    );
    assert!(!state.artwork.fetch.is_loading());
    assert_eq!(state.artwork.current_data, Some(vec![2]));
}

#[tokio::test]
async fn stop_cancels_preparation_and_rejects_its_late_completion() {
    let mut state = AppState::new();
    let track = Track::from_folder("fixture", "track.flac");
    state.queue.tracks = vec![track.clone()];
    state.queue.index = Some(0);
    state.set_playback_mode(PlaybackMode::Queue);
    state.playback.status = PlayStatus::Buffering;
    let task = tokio::spawn(std::future::pending::<()>());
    state.sources.media = MediaPreparation::Preparing(textamp::app::tasks::TaskLease::new(&task));
    let preparation = state.playback.preparation_id;
    let mut audio = textamp::audio::AudioPlayer::new_without_audio();
    let mut config = Config::default();
    let (tx, _) = tokio::sync::mpsc::channel(4);
    dispatch_action(
        PlaybackAction::Stop.into(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    assert!(task.await.unwrap_err().is_cancelled());
    dispatch_action(
        FileAction::Prepared {
            generation: state.library_generation,
            id: preparation,
            track: Box::new(track),
            artwork: None,
            result: Err("late failure".into()),
        }
        .into(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    assert!(matches!(state.sources.media, MediaPreparation::Idle));
    assert_eq!(state.playback.status, PlayStatus::Stopped);
    assert!(state.notifications.last_error.is_none());
}
