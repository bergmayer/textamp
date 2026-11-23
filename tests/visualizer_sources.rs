use std::time::Duration;
use textamp::app::action::SystemAction;
use textamp::app::dispatch::{dispatch_action, handle_core_event};
use textamp::app::sources::{ActiveSource, navidrome::Session};
use textamp::app::state::{PlaybackMode, View};
use textamp::app::{AppState, Event};
use textamp::audio::AudioPlayer;
use textamp::config::Config;
use textamp::library::track::Track;
use textamp::library::{FolderLocation, FolderSource, MediaFile};
use textamp::navidrome::{Client, Source};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
fn wav() -> Vec<u8> {
    let count = 8192u32;
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36 + count * 2).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(44100u32.to_le_bytes());
    bytes.extend(88200u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend((count * 2).to_le_bytes());
    for i in 0..count {
        bytes.extend((((i as f32 * 0.13).sin() * 16000.0) as i16).to_le_bytes());
    }
    bytes
}
async fn analyze(state: &mut AppState) {
    let (tx, mut rx) = mpsc::channel::<Event>(100);
    // Deliberately not connected.
    let mut audio = AudioPlayer::new_without_audio();
    dispatch_action(
        SystemAction::LoadWaveform.into(),
        state,
        &mut audio,
        &mut Config::default(),
        &tx,
    )
    .await
    .unwrap();
    assert!(state.waveform.task.is_some());
    tokio::time::timeout(Duration::from_secs(10), async {
        while state.waveform.generating() || state.spectrogram.generating() {
            let event = rx.recv().await.unwrap();
            for action in handle_core_event(event, state, &tx) {
                dispatch_action(action, state, &mut audio, &mut Config::default(), &tx)
                    .await
                    .unwrap();
            }
        }
    })
    .await
    .unwrap();
    assert!(
        state.waveform.error().is_none(),
        "{:?}",
        state.waveform.error()
    );
    assert!(
        state.spectrogram.error().is_none(),
        "{:?}",
        state.spectrogram.error()
    );
    assert!(state.waveform.data().unwrap().bins.iter().any(|v| *v > 0.5));
    assert!(
        state
            .spectrogram
            .data()
            .unwrap()
            .frames
            .iter()
            .any(|v| *v > 0)
    );
}
#[tokio::test]
async fn folder_visualizers_wait_for_preparation_then_analyze_all_folder_providers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("music.wav");
    std::fs::write(&path, wav()).unwrap();
    for location in [
        FolderLocation::Local {
            path: dir.path().into(),
        },
        FolderLocation::Webdav {
            url: "http://127.0.0.1:1/".into(),
            username: None,
            password_env: None,
        },
    ] {
        let id = uuid::Uuid::new_v4().to_string();
        let mut state = AppState::new();
        state.view = View::NowPlaying;
        state.sources.active = ActiveSource::Folder(id.clone());
        state.sources.folders.push(FolderSource {
            id: id.clone(),
            name: "fixture".into(),
            location,
        });
        state.set_playback_mode(PlaybackMode::Queue);
        let mut track = Track::from_folder(&id, "music.wav");
        track.duration = Some(186);
        state.queue.tracks = vec![track];
        state.queue.index = Some(0);
        let (tx, _) = mpsc::channel(10);

        dispatch_action(
            SystemAction::LoadWaveform.into(),
            &mut state,
            &mut AudioPlayer::new_without_audio(),
            &mut Config::default(),
            &tx,
        )
        .await
        .unwrap();
        assert!(!state.waveform.generating());
        assert!(state.waveform.error().is_none());
        state.sources.media =
            textamp::app::sources::MediaPreparation::Ready(MediaFile::local(path.clone()));
        analyze(&mut state).await;
    }
}
#[tokio::test]
async fn navidrome_visualizers_read_the_authenticated_raw_stream_without_plex() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 8192];
        let n = socket.read(&mut request).await.unwrap();
        let request = String::from_utf8_lossy(&request[..n]);
        assert!(request.contains("/rest/stream?"));
        assert!(request.contains("format=raw"));
        let bytes = wav();
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        socket.write_all(&bytes).await.unwrap();
    });
    let source = Source {
        id: uuid::Uuid::new_v4().to_string(),
        name: "fixture".into(),
        url,
        username: "fixture".into(),
        libraries: vec![],
    };
    let session = Session {
        client: Client::new(&source, "fixture".into(), None).unwrap(),
        source,
        extensions: Default::default(),
    };
    let track = session.track(
        serde_json::from_value(serde_json::json!({"id":"song","title":"Song","duration":1}))
            .unwrap(),
    );
    let mut state = AppState::new();
    state.view = View::NowPlaying;
    state.sources.active = ActiveSource::Navidrome(Box::new(session));
    state.set_playback_mode(PlaybackMode::Queue);
    state.queue.tracks = vec![track];
    state.queue.index = Some(0);
    analyze(&mut state).await;
    task.await.unwrap();
}

#[test]
fn analysis_progress_is_scoped_to_the_current_track() {
    use textamp::app::event::VisualizerEvent;
    use textamp::app::state::AnalysisProgress;
    let mut state = AppState::new();
    state.waveform.track_key = Some("current".into());
    state.waveform.start();
    let (tx, _) = mpsc::channel(8);
    for key in ["old", "current"] {
        handle_core_event(
            Event::Visualizer(VisualizerEvent::AnalysisProgress {
                track_key: key.into(),
                progress: AnalysisProgress::Downloading {
                    received: 25,
                    total: Some(100),
                },
            }),
            &mut state,
            &tx,
        );
        assert_eq!(state.waveform.progress().is_some(), key == "current");
    }
    assert_eq!(
        state.waveform.progress().unwrap().message(),
        "Downloading audio… 25%"
    );
}

#[tokio::test]
async fn analysis_retry_and_cache_miss_cannot_replace_completed_data() {
    use textamp::app::{event::VisualizerEvent, state::AnalysisPhase};
    let mut state = AppState::new();
    state.waveform.track_key = Some("current".into());
    state.waveform.start();
    let (tx, _rx) = mpsc::channel(16);
    handle_core_event(
        Event::Visualizer(VisualizerEvent::WaveformFailed {
            track_key: "current".into(),
            error: "Temporary".into(),
        }),
        &mut state,
        &tx,
    );
    assert!(matches!(
        state.waveform.phase,
        AnalysisPhase::WaitingToRetry
    ));
    state.waveform.succeed(textamp::media::WaveformData {
        track_key: "current".into(),
        duration_ms: 1000,
        bins: vec![0.5],
        version: 1,
        created_at: 0,
    });
    let shared = state.waveform.shared_data().unwrap();
    let cloned = state.waveform.clone().shared_data().unwrap();
    assert!(std::sync::Arc::ptr_eq(&shared, &cloned));
    assert!(
        handle_core_event(
            Event::Visualizer(VisualizerEvent::WaveformRetry("current".into())),
            &mut state,
            &tx
        )
        .is_empty()
    );
    handle_core_event(
        Event::Visualizer(VisualizerEvent::WaveformFailed {
            track_key: "current".into(),
            error: "Late failure".into(),
        }),
        &mut state,
        &tx,
    );
    assert!(state.waveform.error().is_none());
    assert!(state.waveform.data().is_some());
    state.spectrogram.track_key = Some("current".into());
    state.spectrogram.start();
    handle_core_event(
        Event::Visualizer(VisualizerEvent::SpectrogramCacheMiss {
            track_key: "current".into(),
        }),
        &mut state,
        &tx,
    );
    assert!(matches!(state.spectrogram.phase, AnalysisPhase::Idle));
    assert!(state.spectrogram.error().is_none());
}
