//! Shared analysis loading, without credentials, a server, or audio hardware.
use std::sync::Arc;
use textamp::app::sources::{audiomuse, navidrome, sonic, ActiveSource};
use textamp::app::state::{BrowseCategory, View};
use textamp::app::{Action, AppState, Event};
use textamp::audiomuse::{Analysis, Connection, Feature, Snapshot, SyncProgress};
use tokio::sync::mpsc;

fn state() -> AppState {
    let source = textamp::navidrome::Source {
        id: "analysis-fixture".into(),
        name: "Music".into(),
        url: "http://example.invalid".into(),
        username: "listener".into(),
        libraries: vec![],
    };
    let session = navidrome::Session {
        client: textamp::navidrome::Client::new(&source, "unused".into(), None).unwrap(),
        source,
        extensions: Default::default(),
    };
    let mut state = AppState::new();
    state.sources.audiomuse.connections.insert(
        audiomuse::key(&session),
        Connection {
            url: "http://example.invalid".into(),
            username: "listener".into(),
            server: "Music".into(),
        },
    );
    state.sources.active = ActiveSource::Navidrome(Box::new(session));
    state.view = View::Browse;
    state
}

fn pending(state: &mut AppState) -> tokio::task::JoinHandle<()> {
    let task = tokio::spawn(std::future::pending());
    state.sources.nav_tasks.insert(
        audiomuse::SYNC_SLOT,
        (7, textamp::app::tasks::TaskLease::new(&task)),
    );
    task
}

async fn dispatch(state: &mut AppState, tx: &mpsc::Sender<Event>, action: impl Into<Action>) {
    textamp::app::dispatch::dispatch_action(
        action.into(),
        state,
        &mut textamp::audio::AudioPlayer::new_without_audio(),
        &mut textamp::config::Config::default(),
        tx,
    )
    .await
    .unwrap();
}

fn snapshot() -> Arc<Snapshot> {
    let track: Analysis = serde_json::from_value(serde_json::json!({
        "id":"one", "fp":"a", "tempo":120.0, "energy":0.7, "key":"C", "scale":"minor", "mood_vector":"rock:0.8"
    })).unwrap();
    Arc::new(Snapshot {
        tracks: [(track.id.clone(), track)].into_iter().collect(),
        partial: true,
        ..Default::default()
    })
}

fn add_catalog_track(state: &mut AppState) {
    state.library.all_tracks = vec![serde_json::from_value(serde_json::json!({
        "ratingKey":"navidrome:analysis-fixture:one", "title":"One"
    }))
    .unwrap()];
}

#[tokio::test]
async fn weekly_maintenance_keeps_navigation_and_fresh_data_and_manual_refresh_bypasses_age() {
    let mut state = state();
    add_catalog_track(&mut state);
    state.view = View::NowPlaying;
    let mut fresh = snapshot().as_ref().clone();
    fresh.updated = textamp::library::cache::now();
    state.sources.audiomuse.snapshot = Some(Arc::new(fresh));
    let (tx, _rx) = mpsc::channel(8);
    audiomuse::check_staleness(&mut state, &tx);
    assert!(state.sources.nav_tasks.is_empty());
    assert_eq!(state.view, View::NowPlaying);

    Arc::make_mut(state.sources.audiomuse.snapshot.as_mut().unwrap()).updated -=
        textamp::library::cache::REFRESH_INTERVAL.as_secs();
    audiomuse::check_staleness(&mut state, &tx);
    let request = state.sources.nav_tasks[audiomuse::SYNC_SLOT].0;
    assert_eq!(state.view, View::NowPlaying);
    assert!(state.sources.nav_collection.is_none());
    assert_eq!(
        state
            .sources
            .audiomuse
            .snapshot
            .as_ref()
            .unwrap()
            .tracks
            .len(),
        1
    );
    audiomuse::check_staleness(&mut state, &tx);
    assert_eq!(state.sources.nav_tasks[audiomuse::SYNC_SLOT].0, request);
    state.sources.nav_tasks.clear();

    Arc::make_mut(state.sources.audiomuse.snapshot.as_mut().unwrap()).updated =
        textamp::library::cache::now();
    audiomuse::open(Feature::Labels, true, &mut state, &tx);
    assert!(state.sources.nav_tasks.contains_key(audiomuse::SYNC_SLOT));
    assert!(
        !state.artist_nav.loading,
        "cached labels stay usable during refresh"
    );
    assert_eq!(state.artist_nav.columns[0].items.len(), 1);
}

#[tokio::test]
async fn failed_analysis_refresh_keeps_cache_and_requires_manual_retry() {
    let mut state = state();
    add_catalog_track(&mut state);
    state.sources.audiomuse.snapshot = Some(snapshot());
    Arc::make_mut(state.sources.audiomuse.snapshot.as_mut().unwrap()).updated = 1;
    let task = pending(&mut state);
    let (tx, _rx) = mpsc::channel(8);
    let generation = state.library_generation;
    dispatch(
        &mut state,
        &tx,
        navidrome::NavAction::Completed {
            generation,
            request_id: 7,
            slot: audiomuse::SYNC_SLOT,
            result: Err("fixture outage".into()),
        },
    )
    .await;
    assert!(state.sources.audiomuse.refresh_failed);
    assert!(state.sources.audiomuse.snapshot.is_some());
    assert!(state.notifications.last_error.is_none());
    assert!(state
        .notifications
        .status_message
        .as_deref()
        .unwrap()
        .contains("cached data kept"));
    audiomuse::check_staleness(&mut state, &tx);
    assert!(!state.sources.nav_tasks.contains_key(audiomuse::SYNC_SLOT));
    audiomuse::open(Feature::Labels, true, &mut state, &tx);
    assert!(state.sources.nav_tasks.contains_key(audiomuse::SYNC_SLOT));
    assert!(!state.sources.audiomuse.refresh_failed);
    assert!(task.await.unwrap_err().is_cancelled());
}

fn loaded() -> Action {
    audiomuse::Command::Loaded {
        snapshot: snapshot(),
        warning: None,
    }
    .into()
}

#[tokio::test]
async fn preview_populates_all_analysis_views_and_survives_a_later_failure() {
    let mut state = state();
    let task = pending(&mut state);
    let (tx, _rx) = mpsc::channel(8);
    audiomuse::open(Feature::Labels, false, &mut state, &tx);
    let generation = state.library_generation;
    dispatch(
        &mut state,
        &tx,
        audiomuse::Command::Progress {
            generation,
            request_id: 7,
            progress: SyncProgress::Preview(snapshot()),
        },
    )
    .await;
    for feature in [
        Feature::Labels,
        Feature::Tempo,
        Feature::Energy,
        Feature::Key,
    ] {
        audiomuse::open(feature, false, &mut state, &tx);
        assert!(!state.artist_nav.columns[0].items.is_empty());
        assert_eq!(state.sources.nav_tasks[audiomuse::SYNC_SLOT].0, 7);
    }
    dispatch(
        &mut state,
        &tx,
        navidrome::NavAction::Completed {
            generation,
            request_id: 7,
            slot: audiomuse::SYNC_SLOT,
            result: Err("fixture outage".into()),
        },
    )
    .await;
    assert!(!state.artist_nav.loading);
    assert!(!state.artist_nav.columns[0].items.is_empty());
    assert!(state.sources.audiomuse.snapshot.as_ref().unwrap().partial);
    assert!(state.notifications.last_error.is_some());
    assert!(task.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn analysis_views_share_loading_and_completion_follows_the_current_view() {
    let mut state = state();
    let task = pending(&mut state);
    let (tx, _rx) = mpsc::channel(8);
    for feature in [
        Feature::Labels,
        Feature::Tempo,
        Feature::Energy,
        Feature::Key,
        Feature::Labels,
    ] {
        audiomuse::open(feature, false, &mut state, &tx);
        assert_eq!(state.sources.nav_tasks[audiomuse::SYNC_SLOT].0, 7);
        assert!(state.artist_nav.loading);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 40)).unwrap();
        terminal
            .draw(|frame| {
                textamp::ui::render(frame, &state);
            })
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("Loading…"));
        assert!(!text.contains("(empty)"));
        state.set_browse_category(BrowseCategory::Library, false);
        assert!(state.sources.nav_tasks.contains_key(audiomuse::SYNC_SLOT));
    }
    audiomuse::open(Feature::Labels, false, &mut state, &tx);
    let generation = state.library_generation;
    dispatch(
        &mut state,
        &tx,
        navidrome::NavAction::Completed {
            generation,
            request_id: 7,
            slot: audiomuse::SYNC_SLOT,
            result: Ok(vec![loaded()]),
        },
    )
    .await;
    assert!(!state.artist_nav.loading);
    assert!(!state.artist_nav.columns[0].items.is_empty());
    assert!(state
        .sources
        .audiomuse
        .snapshot
        .as_ref()
        .unwrap()
        .tracks
        .contains_key("one"));
    assert!(task.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn sync_completion_is_cached_after_browsing_away_without_replacing_the_view() {
    let mut state = state();
    let task = pending(&mut state);
    let (tx, _rx) = mpsc::channel(8);
    audiomuse::open(Feature::Labels, false, &mut state, &tx);
    state.set_browse_category(BrowseCategory::Library, false);
    let generation = state.library_generation;
    dispatch(
        &mut state,
        &tx,
        navidrome::NavAction::Completed {
            generation,
            request_id: 7,
            slot: audiomuse::SYNC_SLOT,
            result: Ok(vec![loaded()]),
        },
    )
    .await;
    assert!(state.sources.nav_collection.is_none());
    assert!(state.sources.audiomuse.snapshot.is_some());
    audiomuse::open(Feature::Labels, false, &mut state, &tx);
    assert!(!state.artist_nav.loading);
    assert!(!state.artist_nav.columns[0].items.is_empty());
    assert!(task.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn stale_progress_and_completion_are_rejected_and_failure_clears_loading() {
    let mut state = state();
    let task = pending(&mut state);
    let (tx, _rx) = mpsc::channel(8);
    audiomuse::open(Feature::Labels, false, &mut state, &tx);
    let generation = state.library_generation;
    for (epoch, request_id) in [(generation + 1, 7), (generation, 6)] {
        dispatch(
            &mut state,
            &tx,
            audiomuse::Command::Progress {
                generation: epoch,
                request_id,
                progress: SyncProgress::Tracks {
                    loaded: 999,
                    total: 1000,
                },
            },
        )
        .await;
        assert!(!state
            .notifications
            .status_message
            .as_ref()
            .unwrap()
            .contains("999"));
        dispatch(
            &mut state,
            &tx,
            navidrome::NavAction::Completed {
                generation: epoch,
                request_id,
                slot: audiomuse::SYNC_SLOT,
                result: Ok(vec![loaded()]),
            },
        )
        .await;
        assert!(state.sources.audiomuse.snapshot.is_none());
        assert!(state.artist_nav.loading);
    }
    dispatch(
        &mut state,
        &tx,
        navidrome::NavAction::Completed {
            generation,
            request_id: 7,
            slot: audiomuse::SYNC_SLOT,
            result: Err("fixture outage".into()),
        },
    )
    .await;
    assert!(!state.artist_nav.loading);
    assert!(state.notifications.last_error.is_some());
    assert!(task.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn disabling_sonic_cancels_the_shared_sync() {
    let mut state = state();
    let task = pending(&mut state);
    state
        .sources
        .sonic_disabled_libraries
        .insert(audiomuse::key(state.sources.active.navidrome().unwrap()));
    sonic::reconcile(&mut state);
    assert!(!state.sources.nav_tasks.contains_key(audiomuse::SYNC_SLOT));
    assert!(task.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn completed_import_preserves_the_preview_group_and_open_child() {
    let mut state = state();
    add_catalog_track(&mut state);
    state.library.all_tracks.push(
        serde_json::from_value(serde_json::json!({
            "ratingKey":"navidrome:analysis-fixture:two", "title":"Two"
        }))
        .unwrap(),
    );
    let task = pending(&mut state);
    let (tx, _rx) = mpsc::channel(8);
    audiomuse::open(Feature::Labels, false, &mut state, &tx);
    let generation = state.library_generation;
    dispatch(
        &mut state,
        &tx,
        audiomuse::Command::Progress {
            generation,
            request_id: 7,
            progress: SyncProgress::Preview(snapshot()),
        },
    )
    .await;
    let selected = state.artist_nav.columns[0]
        .selected_item()
        .unwrap()
        .key()
        .to_owned();
    state
        .artist_nav
        .columns
        .push(textamp::app::state::BrowseColumn::new("rock songs", vec![]));
    state.artist_nav.focused_column = 1;
    let mut complete = snapshot().as_ref().clone();
    complete.tracks.get_mut("one").unwrap().mood_vector = Some("ambient:0.7,rock:0.8".into());
    let mut second = complete.tracks["one"].clone();
    second.id = "two".into();
    complete.tracks.insert(second.id.clone(), second);
    dispatch(
        &mut state,
        &tx,
        navidrome::NavAction::Completed {
            generation,
            request_id: 7,
            slot: audiomuse::SYNC_SLOT,
            result: Ok(vec![audiomuse::Command::Loaded {
                snapshot: Arc::new(complete),
                warning: None,
            }
            .into()]),
        },
    )
    .await;
    assert_eq!(state.artist_nav.columns[0].items.len(), 2);
    assert_eq!(
        state.artist_nav.columns[0].selected_item().unwrap().key(),
        selected
    );
    assert_eq!(state.artist_nav.columns[1].title, "rock songs");
    assert_eq!(
        state.artist_nav.columns[1].tracks.len(),
        2,
        "new analysis reaches the already-open group"
    );
    assert_eq!(state.artist_nav.focused_column, 1);
    assert!(task.await.unwrap_err().is_cancelled());
    let mut changed = state
        .sources
        .audiomuse
        .snapshot
        .as_ref()
        .unwrap()
        .as_ref()
        .clone();
    for track in changed.tracks.values_mut() {
        track.mood_vector = Some("ambient:0.9".into());
    }
    dispatch(
        &mut state,
        &tx,
        audiomuse::Command::Loaded {
            snapshot: Arc::new(changed),
            warning: None,
        },
    )
    .await;
    assert_eq!(state.artist_nav.columns.len(), 1);
    assert_eq!(state.artist_nav.focused_column, 0);
}

#[tokio::test]
async fn analysis_opened_before_a_cold_catalog_resumes_when_tracks_arrive() {
    for success in [true, false] {
        let mut state = state();
        state.library_loading = true;
        let (tx, _rx) = mpsc::channel(8);
        audiomuse::open(Feature::Labels, false, &mut state, &tx);
        assert!(state.artist_nav.loading);
        assert!(!state.sources.nav_tasks.contains_key(audiomuse::SYNC_SLOT));
        let catalog: navidrome::Catalog = serde_json::from_value(serde_json::json!({
            "artists": [], "albums": [], "playlists": [], "libraries": [], "extensions": [],
            "tracks": [{"ratingKey":"navidrome:analysis-fixture:one", "title":"One"}]
        }))
        .unwrap();
        let action = navidrome::NavAction::Catalog {
            generation: state.library_generation,
            request_id: state.sources.list_id,
            refreshing: false,
            timestamp: 1,
            result: if success {
                Ok(catalog.prepare())
            } else {
                Err("fixture outage".into())
            },
        };
        let actions = navidrome::dispatch(
            action,
            &mut state,
            &mut textamp::audio::AudioPlayer::new_without_audio(),
            &mut Default::default(),
            &tx,
        )
        .await
        .unwrap();
        if success {
            assert!(
                matches!(actions.as_slice(), [Action::Source(textamp::app::sources::SourceAction::AudioMuse(command))]
                if matches!(command.as_ref(), audiomuse::Command::Open { feature: Feature::Labels, refresh: false }))
            );
            assert_eq!(state.library.all_tracks.len(), 1);
        } else {
            assert!(actions.is_empty());
            assert!(!state.artist_nav.loading);
            assert!(state.notifications.last_error.is_some());
        }
    }
}
