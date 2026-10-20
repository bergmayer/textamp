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
    let task = tokio::spawn(std::future::pending::<()>());
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
    assert!(state.notifications.last_error.is_none());
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
    assert!(state.notifications.last_error.is_none());
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

async fn act(
    state: &mut AppState,
    config: &mut textamp::config::Config,
    action: impl Into<Action>,
) {
    let (tx, _rx) = mpsc::channel(16);
    textamp::app::dispatch::dispatch_action(
        action.into(),
        state,
        &mut textamp::audio::AudioPlayer::new_without_audio(),
        config,
        &tx,
    )
    .await
    .unwrap();
}
async fn press(
    state: &mut AppState,
    config: &mut textamp::config::Config,
    code: crossterm::event::KeyCode,
) {
    let actions = textamp::app::handlers::key_input::handle_key(
        crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE),
        state,
        config,
    );
    for action in actions {
        act(state, config, action).await;
    }
}
async fn results(
    state: &mut AppState,
    config: &mut textamp::config::Config,
    query: &str,
    ids: &[&str],
) {
    state.sources.nav_collection = Some(navidrome::commands::CollectionKind::AudioMuse(
        Feature::Describe,
    ));
    act(
        state,
        config,
        audiomuse::Command::Results {
            request_id: state.artist_nav_request_id,
            feature: Feature::Describe,
            title: query.into(),
            ids: ids.iter().map(|id| (*id).into()).collect(),
        },
    )
    .await;
}

#[tokio::test]
async fn description_history_survives_restart_is_library_scoped_and_arrows_never_prompt() {
    use crossterm::event::KeyCode;
    use textamp::app::state::{BrowseItem, CategoryRow};
    let mut state = state();
    add_catalog_track(&mut state);
    let mut config = textamp::config::Config::default();
    results(&mut state, &mut config, "Quiet piano", &["one"]).await;
    results(&mut state, &mut config, "Bright strings", &["one"]).await;
    results(&mut state, &mut config, "Quiet piano", &["one"]).await;
    let library_key = audiomuse::key(state.sources.active.navidrome().unwrap());
    assert_eq!(config.description_history[&library_key].len(), 2);
    assert_eq!(
        config.description_history[&library_key][0].query,
        "Quiet piano"
    );
    assert_eq!(state.artist_nav.focused_column, 1);
    let saved = toml::to_string(&config).unwrap();
    let config_loaded: textamp::config::Config = toml::from_str(&saved).unwrap();
    state.sources.audiomuse.description_history.clear();
    textamp::app::sources::manager::initialize(&mut state, &config_loaded);
    // This fixture's connection was in memory; startup normally loads it from config too.
    state.sources.audiomuse.connections.insert(
        library_key.clone(),
        Connection {
            url: "http://example.invalid".into(),
            username: "listener".into(),
            server: "Music".into(),
        },
    );
    state.sources.nav_collection = None;
    state.category_column_index = state
        .category_rows()
        .iter()
        .position(|row| {
            matches!(
                row,
                CategoryRow::NavidromeCollection(navidrome::commands::CollectionKind::AudioMuse(
                    Feature::Describe
                ))
            )
        })
        .unwrap();
    state.category_column_focused = true;
    press(&mut state, &mut config, KeyCode::Right).await;
    assert!(!state.category_column_focused);
    assert!(state.popups.input_dialog.is_none());
    assert_eq!(state.artist_nav.focused_column, 0);
    assert_eq!(state.artist_nav.columns[0].items[0].title(), "Quiet piano");
    assert_eq!(state.artist_nav.columns[1].tracks[0].title, "One");
    press(&mut state, &mut config, KeyCode::Down).await;
    assert_eq!(state.artist_nav.columns[1].title, "Bright strings");
    assert!(state.popups.input_dialog.is_none());
    press(&mut state, &mut config, KeyCode::Right).await;
    assert_eq!(state.artist_nav.focused_column, 1);
    assert!(matches!(
        state.artist_nav.selected_item(),
        Some(BrowseItem::Track { .. })
    ));
    press(&mut state, &mut config, KeyCode::Left).await;
    press(&mut state, &mut config, KeyCode::Left).await;
    assert!(state.category_column_focused);
    press(&mut state, &mut config, KeyCode::Enter).await;
    assert!(state.popups.input_dialog.is_some());
    press(&mut state, &mut config, KeyCode::Esc).await;
    assert_eq!(state.artist_nav.columns[0].items.len(), 2);
    assert!(state.queue.tracks.is_empty());
    // A different folder of the same account has independent history.
    if let textamp::app::sources::ActiveSource::Navidrome(session) = &mut state.sources.active {
        session.client.folder = Some("other".into());
    }
    let other_key = audiomuse::key(state.sources.active.navidrome().unwrap());
    state.sources.audiomuse.connections.insert(
        other_key,
        state.sources.audiomuse.connections[&library_key].clone(),
    );
    state.sources.nav_collection = None;
    act(
        &mut state,
        &mut config,
        audiomuse::Command::BrowseDescriptions { take_focus: true },
    )
    .await;
    assert!(state.artist_nav.columns[0].items.is_empty());
}

#[tokio::test]
async fn description_results_use_playlist_rows_and_both_rows_click_the_same_track() {
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::{backend::TestBackend, Terminal};
    let mut state = state();
    state.library.all_tracks = serde_json::from_value(serde_json::json!([
        {"ratingKey":"navidrome:analysis-fixture:one", "title":"First song", "grandparentTitle":"First artist", "parentTitle":"First album"},
        {"ratingKey":"navidrome:analysis-fixture:two", "title":"Second song", "grandparentTitle":"Second artist", "parentTitle":"Second album"}
    ])).unwrap();
    let mut config = textamp::config::Config::default();
    results(
        &mut state,
        &mut config,
        "Quiet piano",
        &["two", "one", "two", "missing"],
    )
    .await;
    state.category_column_focused = false;
    state.terminal_width = 160;
    state.terminal_height = 30;
    let mut terminal = Terminal::new(TestBackend::new(160, 30)).unwrap();
    terminal
        .draw(|frame| state.apply_render_feedback(textamp::ui::render(frame, &state)))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("Second artist"));
    assert!(text.contains("Second album"));
    if let Ok(dir) = std::env::var("TEXTAMP_SETTINGS_PREVIEW_DIR") {
        let cells: Vec<_> = terminal.backend().buffer().content.iter().map(|c| serde_json::json!({"s":c.symbol(),"fg":format!("{:?}",c.fg),"bg":format!("{:?}",c.bg)})).collect();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            std::path::Path::new(&dir).join("description-history-160x30.json"),
            serde_json::to_vec(&serde_json::json!({"width":160,"height":30,"cells":cells}))
                .unwrap(),
        )
        .unwrap();
    }
    let region = state
        .hit_regions
        .miller_columns
        .as_ref()
        .unwrap()
        .columns
        .iter()
        .find(|r| r.col_idx == 1)
        .unwrap()
        .clone();
    assert_eq!(region.rows_per_item, 2);
    assert_eq!(state.artist_nav.columns[1].tracks.len(), 2);
    assert_eq!(state.artist_nav.columns[1].tracks[0].title, "Second song");
    for offset in [2, 3] {
        state.artist_nav.columns[1].selected_index = 0;
        state.scroll.browse_last_click = None;
        let actions = textamp::app::handlers::mouse_input::handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: region.inner.x + 2,
                row: region.inner.y + offset,
                modifiers: KeyModifiers::NONE,
            },
            &mut state,
        );
        for action in actions {
            act(&mut state, &mut config, action).await;
        }
        assert_eq!(state.artist_nav.columns[1].selected_index, 1);
        assert_eq!(state.scroll.browse, Some((1, 0)));
        assert!(state.popups.input_dialog.is_none());
    }
}

#[tokio::test]
async fn history_is_bounded_and_stale_results_do_not_write_it() {
    let mut state = state();
    add_catalog_track(&mut state);
    let mut config = textamp::config::Config::default();
    for i in 0..32 {
        results(
            &mut state,
            &mut config,
            &format!("Description {i}"),
            &["one"],
        )
        .await;
    }
    let library_key = audiomuse::key(state.sources.active.navidrome().unwrap());
    assert_eq!(config.description_history[&library_key].len(), 30);
    assert_eq!(
        config.description_history[&library_key][0].query,
        "Description 31"
    );
    let stale_id = state.artist_nav_request_id.wrapping_sub(1);
    act(
        &mut state,
        &mut config,
        audiomuse::Command::Results {
            request_id: stale_id,
            feature: Feature::Describe,
            title: "Stale description".into(),
            ids: vec!["one".into()],
        },
    )
    .await;
    assert_eq!(
        config.description_history[&library_key][0].query,
        "Description 31"
    );
    assert!(!state.category_rows().iter().any(|r| matches!(
        r,
        textamp::app::state::CategoryRow::NavidromeCollection(
            navidrome::commands::CollectionKind::AudioMuse(Feature::Analyzed)
        )
    )));
}

#[tokio::test]
async fn background_sync_failure_never_interrupts_biography_or_playback_even_without_cache() {
    use textamp::app::state::{ArtistBioPopup, PlayStatus};
    for preview in [false, true] {
        let mut state = state();
        add_catalog_track(&mut state);
        state.view = View::NowPlaying;
        state.playback.status = PlayStatus::Playing;
        state.playback.position_ms = 38000;
        state.queue.tracks = state.library.all_tracks.clone();
        state.queue.index = Some(0);
        state.popups.artist_bio = Some(ArtistBioPopup {
            artist_name: "Fixture artist".into(),
            document: Default::default(),
            scroll: 3,
            google_focused: true,
            loading: false,
            image_index: 0,
            task: None,
        });
        if preview {
            state.sources.audiomuse.snapshot = Some(snapshot());
        }
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
                result: Err("fixture request timed out".into()),
            },
        )
        .await;
        assert!(state.notifications.last_error.is_none());
        assert!(state
            .notifications
            .status_message
            .as_deref()
            .unwrap()
            .contains("analysis category (F5)"));
        assert!(state.sources.audiomuse.refresh_failed);
        assert_eq!(state.sources.audiomuse.snapshot.is_some(), preview);
        assert_eq!(state.view, View::NowPlaying);
        assert_eq!(state.playback.status, PlayStatus::Playing);
        assert_eq!(state.playback.position_ms, 38000);
        assert_eq!(state.queue.index, Some(0));
        let bio = state.popups.artist_bio.as_ref().unwrap();
        assert_eq!(bio.artist_name, "Fixture artist");
        assert_eq!(bio.scroll, 3);
        assert!(bio.google_focused);
        audiomuse::check_staleness(&mut state, &tx);
        assert!(!state.sources.nav_tasks.contains_key(audiomuse::SYNC_SLOT));
        assert!(task.await.unwrap_err().is_cancelled());
        // An explicit description search is still allowed to report a real error.
        let task = tokio::spawn(std::future::pending::<()>());
        state
            .sources
            .nav_tasks
            .insert("audiomuse", (8, textamp::app::tasks::TaskLease::new(&task)));
        dispatch(
            &mut state,
            &tx,
            navidrome::NavAction::Completed {
                generation,
                request_id: 8,
                slot: "audiomuse",
                result: Err("Search unavailable".into()),
            },
        )
        .await;
        assert_eq!(
            state.notifications.last_error.as_deref(),
            Some("Search unavailable")
        );
        assert!(task.await.unwrap_err().is_cancelled());
    }
}
