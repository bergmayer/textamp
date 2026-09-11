use textamp::app::action::SearchAction;
use textamp::app::handlers::dispatch_search;
use textamp::app::state::{RefreshCategory, SearchTab, View};
use textamp::app::AppState;

use tokio::sync::mpsc;
mod common;

#[tokio::test]
async fn search_and_reopened_adventure_use_current_cached_tracks() {
    let mut state = common::navidrome_state();
    state.library.all_tracks = vec![textamp::library::models::Track {
        rating_key: "fixture".into(),
        title: "Music".into(),
        ..Default::default()
    }];
    let (tx, mut rx) = mpsc::channel(8);
    state.search.tab = SearchTab::Tracks;
    state.search.query = "music".into();
    dispatch_search::dispatch(&tx, SearchAction::ExecuteLocalSearch, &mut state)
        .await
        .unwrap();
    assert_eq!(state.search.results.as_ref().unwrap().tracks.len(), 1);
    state.search.query.clear();
    dispatch_search::dispatch(&tx, SearchAction::ExecuteLocalSearch, &mut state)
        .await
        .unwrap();
    assert!(state.search.results.is_none());

    for query in ["music", "no match"] {
        dispatch_search::dispatch(&tx, SearchAction::OpenAdventureLauncher, &mut state)
            .await
            .unwrap();
        state.popups.adventure_launcher.as_mut().unwrap().query = query.into();
        dispatch_search::dispatch(&tx, SearchAction::AdventureLauncherSearch, &mut state)
            .await
            .unwrap();
        let launcher = state.popups.adventure_launcher.as_ref().unwrap();
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
    use ratatui::{backend::TestBackend, Terminal};
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
    state.sources.sonic_tasks.insert("test", pending_lease());
    state.advance_library_generation();
    assert!(state.sources.sonic_tasks.is_empty());
    assert!(state.cache_mgmt.background_refresh.is_empty());
}

fn pending_lease() -> textamp::app::tasks::TaskLease {
    textamp::app::tasks::TaskLease::new(&tokio::spawn(std::future::pending::<()>()))
}
