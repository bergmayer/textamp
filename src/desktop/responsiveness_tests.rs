//! Opt-in development measurements using generated catalogs and silent playback.
use super::{browser, engine::Snapshot, player_model};
use crate::app::{
    AppState,
    state::{BrowseColumn, BrowseItem},
};
use std::{hint::black_box, time::Instant};

fn measure(name: &str, samples: usize, mut run: impl FnMut()) {
    run();
    let mut times: Vec<_> = (0..samples)
        .map(|_| {
            let start = Instant::now();
            run();
            start.elapsed().as_secs_f64() * 1000.
        })
        .collect();
    times.sort_by(f64::total_cmp);
    eprintln!(
        "{name}: median={:.3} ms p95={:.3} ms samples={samples}",
        times[samples / 2],
        times[(samples * 95 / 100).min(samples - 1)]
    );
}

#[test]
#[ignore = "generated large-catalog responsiveness measurements"]
fn large_catalog_responsiveness() {
    let mut state = AppState::new();
    state.view = crate::app::View::Browse;
    state.artist_nav.columns = vec![BrowseColumn::new(
        "Artists",
        (0..25_000)
            .map(|i| BrowseItem::Artist {
                key: format!("artist-{i}"),
                title: format!("Artist {i:05}"),
                thumb: None,
                is_placeholder: false,
            })
            .collect(),
    )];
    state.artwork.current_data = Some(vec![127; 2 * 1024 * 1024]);
    let snapshot = Snapshot {
        columns: browser::columns(&state),
        artwork: Some(vec![127; 2 * 1024 * 1024].into()),
        player: player_model::project(&state, &mut Default::default()),
        ..Default::default()
    };
    measure("browse_25000_artists", 15, || {
        black_box(browser::columns(&state));
    });
    measure("snapshot_clone_with_artwork", 100, || {
        black_box(snapshot.clone());
    });
    let mut player = player_model::Projection::default();
    measure("player_projection_with_artwork", 100, || {
        black_box(player_model::project(&state, &mut player));
    });
    let temporary = tempfile::tempdir().unwrap();
    let store = crate::library::cache::Store::at(
        temporary.path().join("metadata"),
        "responsiveness-fixture",
    )
    .unwrap();
    let ticket = store.ticket("artists");
    let artists: Vec<_> = (0..25_000).map(|i| format!("Artist {i:05}")).collect();
    ticket.write(&artists).unwrap();
    measure("cached_metadata_read_25000_artists", 15, || {
        black_box(
            ticket
                .read::<Vec<String>>(crate::library::cache::REFRESH_INTERVAL)
                .unwrap()
                .unwrap(),
        );
    });
    let artwork = crate::media::ArtworkCache::new(temporary.path().join("artwork"));
    assert!(artwork.save("cover", &vec![127; 2 * 1024 * 1024]));
    measure("cached_artwork_read_2mib", 30, || {
        black_box(artwork.load("cover", u64::MAX).unwrap());
    });
    let records: Vec<_> = (0..250_000)
        .map(|i| format!("Artist {i} Album {i} Track {i}"))
        .collect();
    measure("filter_250000_records", 7, || {
        black_box(crate::services::filter_with_priority(
            &records,
            "artist 123",
            String::as_str,
            100,
        ));
    });
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "loopback startup, browse and presentation measurements"]
fn live_catalog_responsiveness() {
    use super::engine::{Command, Engine};
    use crate::app::{
        launch::{ApplicationMode, Mode, WindowedMode},
        state::PaletteCommandKind,
    };
    use std::{
        path::PathBuf,
        process::{Child, Command as Process, Stdio},
        time::Duration,
    };
    struct Fixture(Child);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let profile = PathBuf::from(
        std::env::var_os("TEXTAMP_CARGO_TEST_PROFILE")
            .expect("run through Cargo's isolated test runner"),
    );
    let paths = crate::config::XdgPaths::new("textamp");
    assert!(
        paths.config_dir.starts_with(&profile)
            && paths.data_dir.starts_with(&profile)
            && paths.cache_dir.starts_with(&profile)
            && paths.state_dir.starts_with(&profile)
    );
    let ready = profile.join("fixture-ready");
    let mut fixture = Fixture(
        Process::new("python3")
            .args([
                "tests/fixtures/desktop_ui_server.py",
                "--catalog",
                "--artist-count",
                "5000",
                "--ready-file",
            ])
            .arg(&ready)
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let start = Instant::now();
    while !ready.exists() {
        assert!(
            fixture.0.try_wait().unwrap().is_none() && start.elapsed() < Duration::from_secs(10)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let url = std::fs::read_to_string(ready).unwrap();
    std::fs::create_dir_all(&paths.config_dir).unwrap();
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    std::fs::write(paths.config_dir.join("config.toml"), format!("[default_navidrome]\nsource_id = \"fixture\"\n[[navidrome_sources]]\nid = \"fixture\"\nname = \"Fixture\"\nurl = \"{url}\"\nusername = \"fixture\"\n[ui]\ndesktop_separate_player = false\n")).unwrap();
    std::fs::write(
        paths.data_dir.join("navidrome-credentials.toml"),
        "fixture = \"fixture-password\"\n",
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    async fn wait(
        updates: &mut tokio::sync::watch::Receiver<Snapshot>,
        predicate: impl Fn(&Snapshot) -> bool,
    ) -> Snapshot {
        tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                let snapshot = updates.borrow_and_update().clone();
                if predicate(&snapshot) {
                    return snapshot;
                }
                updates.changed().await.unwrap();
            }
        })
        .await
        .expect("fixture response")
    }
    for label in ["cold", "cached"] {
        let started = Instant::now();
        let engine = Engine::start_mode(true, Mode::Windowed(WindowedMode::Gui)).unwrap();
        eprintln!(
            "{label}_session_ready: {:.3} ms",
            started.elapsed().as_secs_f64() * 1000.
        );
        let mut updates = engine.snapshot.clone();
        runtime.block_on(async {
            let mut frame = wait(&mut updates, |s| {
                s.columns.first().is_some_and(|c| c.rows.len() >= 5000)
            })
            .await;
            eprintln!(
                "{label}_browse_ready_5000_artists: {:.3} ms",
                started.elapsed().as_secs_f64() * 1000.
            );
            for visit in 0..2 {
                let serial = frame.navigation_serial;
                let target = frame.columns[0].target(frame.generation, 0, 1).unwrap();
                let started = Instant::now();
                engine
                    .commands
                    .send(Command::Select {
                        target,
                        mode: browser::Selection::Single,
                        origin: browser::Origin::Keyboard,
                        intent: browser::Intent::Inspect,
                    })
                    .await
                    .unwrap();
                frame = wait(&mut updates, |s| {
                    s.navigation_serial > serial
                        && s.columns.get(1).is_some_and(|c| c.rows.len() >= 2)
                })
                .await;
                eprintln!(
                    "{label}_artist_column_visit_{visit}: {:.3} ms",
                    started.elapsed().as_secs_f64() * 1000.
                );
            }
            let started = Instant::now();
            engine
                .commands
                .send(Command::Palette(PaletteCommandKind::SetApplicationMode(
                    ApplicationMode::Text,
                )))
                .await
                .unwrap();
            let preparing = wait(&mut updates, |s| s.presentation.pending().is_some()).await;
            let request = preparing.presentation.pending().unwrap();
            engine
                .commands
                .send(Command::TextViewport {
                    identity: request.identity,
                    viewport: crate::text_mode::backend::Viewport {
                        metrics: crate::text_mode::backend::Metrics {
                            columns: 130,
                            rows: 38,
                            cell_width: 18,
                            cell_height: 40,
                        },
                        scale_factor: 2.,
                        revision: 1,
                        geometry: Default::default(),
                        appearance: crate::text_mode::backend::Appearance {
                            system: None,
                            foreground: [255; 3],
                            background: [0; 3],
                        },
                    },
                })
                .await
                .unwrap();
            let text = wait(&mut updates, |s| {
                s.presentation.active() == Some(WindowedMode::Text) && s.text_frame.is_some()
            })
            .await;
            eprintln!(
                "{label}_gui_to_text_projection: {:.3} ms",
                started.elapsed().as_secs_f64() * 1000.
            );
            engine
                .commands
                .observe(text.presentation.identity().unwrap());
            let started = Instant::now();
            engine
                .commands
                .send(Command::Palette(PaletteCommandKind::SetApplicationMode(
                    ApplicationMode::GuiSingle,
                )))
                .await
                .unwrap();
            let preparing = wait(&mut updates, |s| s.presentation.pending().is_some()).await;
            engine
                .commands
                .send(Command::PresentationReady(
                    preparing.presentation.pending().unwrap(),
                ))
                .await
                .unwrap();
            let gui = wait(&mut updates, |s| {
                s.presentation.active() == Some(WindowedMode::Gui) && s.columns.len() > 1
            })
            .await;
            eprintln!(
                "{label}_text_to_gui_projection: {:.3} ms",
                started.elapsed().as_secs_f64() * 1000.
            );
            assert_eq!(gui.playback_id, frame.playback_id);
            assert_eq!(gui.columns[0].selected, frame.columns[0].selected);
        });
        drop(engine);
    }
}
