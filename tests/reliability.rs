use textamp::app::AppState;
use textamp::app::action::SearchAction;
use textamp::app::handlers::dispatch_search;
use textamp::app::state::{RefreshCategory, SearchTab, View};

use tokio::sync::mpsc;
mod common;

#[tokio::test]
async fn search_and_reopened_adventure_use_current_cached_tracks() {
    let mut state = common::navidrome_state();
    state.library.all_tracks = std::sync::Arc::new(vec![textamp::library::models::Track {
        rating_key: "fixture".into(),
        title: "Music".into(),
        ..Default::default()
    }]);
    let (tx, mut rx) = mpsc::channel(8);
    state.search.tab = SearchTab::Tracks;
    state.search.query = "music".into();
    dispatch_search::dispatch(&tx, SearchAction::ExecuteLocalSearch, &mut state).unwrap();
    assert_eq!(state.search.results.as_ref().unwrap().tracks.len(), 1);
    state.search.query.clear();
    dispatch_search::dispatch(&tx, SearchAction::ExecuteLocalSearch, &mut state).unwrap();
    assert!(state.search.results.is_none());

    for query in ["music", "no match"] {
        dispatch_search::dispatch(&tx, SearchAction::OpenAdventureLauncher, &mut state).unwrap();
        state.popups.adventure_launcher_mut().unwrap().query = query.into();
        dispatch_search::dispatch(&tx, SearchAction::AdventureLauncherSearch, &mut state).unwrap();
        let launcher = state.popups.adventure_launcher().unwrap();
        assert!(!launcher.loading);
        assert_eq!(
            launcher.results.as_ref().unwrap().tracks.len(),
            usize::from(query == "music")
        );
    }
    assert!(
        rx.try_recv().is_err(),
        "search should not start server requests"
    );
}

#[test]
fn principal_views_render_at_small_and_normal_terminal_sizes() {
    use ratatui::{Terminal, backend::TestBackend};
    for (width, height) in [(40, 10), (80, 24), (120, 40)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut state = AppState::new();
        state.terminal_width = width;
        state.terminal_height = height;
        for view in [
            View::Browse,
            View::Queue,
            View::NowPlaying,
            View::Help,
            View::Settings,
        ] {
            state.view = view;
            terminal
                .draw(|frame| {
                    let _ = textamp::ui::render(frame, &state);
                })
                .unwrap();
        }
    }
}

#[tokio::test]
async fn absent_audio_device_returns_failure_without_starting_playback() {
    let mut audio = textamp::audio::AudioPlayer::new_without_audio();
    let (tx, _rx) = mpsc::channel(1);
    let error = audio
        .play_url("http://127.0.0.1:1/audio", tx, reqwest::Client::new())
        .unwrap_err();
    assert!(error.to_string().contains("No audio output device"));
    assert!(!audio.is_playing());
}

#[tokio::test]
async fn changing_library_cancels_owned_work_and_discards_old_progress() {
    let mut state = AppState::new();
    state
        .cache_mgmt
        .background_refresh
        .insert(RefreshCategory::Moods);
    let task = tokio::spawn(std::future::pending::<()>());
    state
        .sources
        .nav_tasks
        .insert("remix", (1, textamp::app::tasks::TaskLease::new(&task)));
    state.advance_library_generation();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(state.cache_mgmt.background_refresh.is_empty());
}

#[tokio::test]
async fn queue_edit_progress_lasts_for_the_task_lease() {
    let mut state = textamp::app::AppState::new();
    let task = tokio::spawn(std::future::pending::<()>());
    state
        .sources
        .nav_tasks
        .insert("remix", (1, textamp::app::tasks::TaskLease::new(&task)));
    state.set_status("A short-lived message".into());
    assert_eq!(state.operation_progress(), Some("Generating queue edit…"));
    let notification = state.current_notification().unwrap();
    assert_eq!(notification.message, "Generating queue edit…");
    assert!(matches!(
        notification.notification_type,
        textamp::app::state::NotificationType::Ongoing
    ));
    state.sources.nav_tasks.remove("remix");
    assert_eq!(state.operation_progress(), None);
    assert!(task.await.unwrap_err().is_cancelled());
}
