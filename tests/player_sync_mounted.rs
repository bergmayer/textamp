#![cfg(target_os = "macos")]
use std::time::Duration;
use textamp::{
    app::{
        Action, AppState,
        dispatch::{dispatch_action, handle_core_event},
        player_sync::{Command, Phase},
        sources::{ActiveSource, navidrome::Session},
    },
    audio::AudioPlayer,
    config::Config,
    navidrome::{Client, Source},
    services::player_sync::volumes,
};
use tokio::sync::mpsc;

#[tokio::test]
#[ignore = "Requires an explicitly identified connected player and loopback fixture"]
async fn connected_player_transfer_roundtrip() {
    let mount = std::env::var("TEXTAMP_PLAYER_TEST_VOLUME").unwrap();
    let expected = std::env::var("TEXTAMP_PLAYER_TEST_UUID").unwrap();
    let mut volume = volumes::inspect(&mount).await.unwrap();
    assert_eq!(volume.uuid, expected);
    volumes::verify(&volume).await.unwrap();
    let folder = tempfile::Builder::new()
        .prefix(".TextampValidation-")
        .tempdir_in(&volume.mount)
        .unwrap();
    let url = std::env::var("TEXTAMP_PLAYER_TEST_SERVER").unwrap();
    assert_eq!(
        reqwest::Url::parse(&url).unwrap().host_str(),
        Some("127.0.0.1")
    );
    let source = Source {
        id: "fixture-player".into(),
        name: "Fixture".into(),
        url,
        username: "fixture".into(),
        libraries: vec![],
    };
    let client = Client::new(&source, "fixture-password".into(), None).unwrap();
    assert_eq!(
        client.call("ping", &[]).await.unwrap()["serverVersion"],
        "fixture"
    );
    let session = Session {
        source,
        client,
        extensions: Default::default(),
    };
    let response = session
        .client
        .call("getSong", &[("id", "song-0-0-0".into())])
        .await
        .unwrap();
    let song: textamp::navidrome::Song = serde_json::from_value(response["song"].clone()).unwrap();
    let songs = vec![textamp::services::player_sync::Song {
        id: song.id.clone(),
        track: session.track(song),
    }];
    let player = textamp::services::player_sync::Player {
        volume_uuid: expected,
        name: "Validation".into(),
        source_id: "fixture-player".into(),
        playlists: vec![String::new()],
        auto_sync: false,
    };
    volume.mount = folder.path().into();
    let target = volumes::Target::open(folder.path()).await.unwrap();
    let start = std::time::Instant::now();
    let first = textamp::services::player_sync::sync(
        &session.client,
        &target,
        &volume,
        &player,
        &songs,
        |_| {},
    )
    .await
    .unwrap();
    let seconds = start.elapsed().as_secs_f64();
    assert_eq!(first.copied, 1);
    let playlist = std::fs::read_to_string(&first.playlist).unwrap();
    let relative = playlist
        .lines()
        .find(|line| !line.starts_with('#'))
        .unwrap();
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(first.playlist.parent().unwrap().join(relative))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("duration=60.000000")
    );
    let second = textamp::services::player_sync::sync(
        &session.client,
        &target,
        &volume,
        &player,
        &songs,
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!((second.copied, second.reused), (0, 1));
    println!(
        "Connected player: {} bytes copied in {seconds:.2}s, 60-second WAV decoded, playlist resolved, second sync reused audio; temporary files removed",
        first.bytes
    );
    folder.close().unwrap();
}

#[tokio::test]
#[ignore = "Requires an explicitly supplied disposable FAT32 disk image and loopback fixture"]
async fn mounted_fat32_player_sync_roundtrip() {
    let mount = std::env::var("TEXTAMP_PLAYER_TEST_VOLUME").unwrap();
    assert!(mount.starts_with("/Volumes/TextampPlayerValidation-"));
    let url = std::env::var("TEXTAMP_PLAYER_TEST_SERVER").unwrap();
    let parsed = reqwest::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    let volume = volumes::inspect(&mount).await.unwrap();
    assert_eq!(volume.name, "TEXTAMPTEST");
    assert!(volume.available > 1024 * 1024);
    let source = Source {
        id: "fixture-player".into(),
        name: "Fixture Music".into(),
        url: url.clone(),
        username: "fixture".into(),
        libraries: vec![],
    };
    let client = Client::new(&source, "fixture-password".into(), None).unwrap();
    assert_eq!(
        client.call("ping", &[]).await.unwrap()["serverVersion"],
        "fixture"
    );
    let name = format!("Validation-{}", uuid::Uuid::new_v4().simple());
    let playlist_name = format!("Player validation {name}");
    let search = client
        .call(
            "search3",
            &[
                ("query", "A Track".into()),
                ("songCount", "2".into()),
                ("albumCount", "0".into()),
                ("artistCount", "0".into()),
            ],
        )
        .await
        .unwrap();
    let songs: Vec<textamp::navidrome::Song> =
        textamp::navidrome::array(&search["searchResult3"], "song").unwrap();
    assert_eq!(songs.len(), 2);
    let mut fields = vec![("name", playlist_name.clone())];
    fields.extend(songs.iter().map(|song| ("songId", song.id.clone())));
    let created = client.call("createPlaylist", &fields).await.unwrap();
    let playlist_id = created["playlist"]["id"].as_str().unwrap().to_owned();
    let mut state = AppState::new();
    state.desktop_frontend = true;
    state.sources.active = ActiveSource::Navidrome(Box::new(Session {
        source: source.clone(),
        client: client.clone(),
        extensions: Default::default(),
    }));
    let mut config = Config {
        navidrome_sources: vec![source],
        ..Default::default()
    };
    state
        .player_sync
        .players
        .push(textamp::services::player_sync::Player {
            volume_uuid: volume.uuid.clone(),
            name,
            source_id: "fixture-player".into(),
            playlists: vec![playlist_name.clone()],
            auto_sync: false,
        });
    let mut audio = AudioPlayer::new_without_audio();
    let (events, mut pending) = mpsc::channel(128);
    dispatch_action(
        Command::LoadPreview.into(),
        &mut state,
        &mut audio,
        &mut config,
        &events,
    )
    .await
    .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(10), pending.recv())
        .await
        .unwrap()
        .unwrap();
    for action in handle_core_event(event, &mut state, &events) {
        dispatch_action(action, &mut state, &mut audio, &mut config, &events)
            .await
            .unwrap();
    }
    let preview = textamp::app::player_sync::project(&state);
    assert_eq!(
        preview.configured_player.unwrap().playlists[0],
        playlist_name
    );
    assert_eq!(
        preview.size_label.unwrap(),
        format!(
            "Total file size: {}",
            textamp::util::format_bytes(2 * 960044)
        )
    );
    assert!(matches!(state.player_sync.phase, Phase::Idle));
    assert!(!volume.mount.join("Textamp fixture-player").exists());
    let calls: serde_json::Value = client
        .http()
        .get(format!("{url}/fixture-events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        !calls
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["endpoint"] == "download")
    );
    state.player_sync.volumes = vec![volume.clone()];
    state.player_sync.selected = Some(volume.uuid.clone());
    dispatch_action(
        Command::Start {
            create_playlist: false,
        }
        .into(),
        &mut state,
        &mut audio,
        &mut config,
        &events,
    )
    .await
    .unwrap();
    async fn finish(
        state: &mut AppState,
        audio: &mut AudioPlayer,
        config: &mut Config,
        events: &mpsc::Sender<textamp::app::Event>,
        pending: &mut mpsc::Receiver<textamp::app::Event>,
    ) {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let Phase::Failed(error) = &state.player_sync.phase {
                    panic!("{error}");
                }
                if let Phase::Review { operation, .. } = state.player_sync.phase {
                    dispatch_action(
                        Command::ApplyReview(operation).into(),
                        state,
                        audio,
                        config,
                        events,
                    )
                    .await
                    .unwrap();
                }
                if matches!(state.player_sync.phase, Phase::Complete(_)) {
                    break;
                }
                let event = pending.recv().await.unwrap();
                for action in handle_core_event(event, state, events) {
                    dispatch_action(action, state, audio, config, events)
                        .await
                        .unwrap();
                }
            }
        })
        .await
        .unwrap();
    }
    finish(&mut state, &mut audio, &mut config, &events, &mut pending).await;
    let Phase::Complete(first) = &state.player_sync.phase else {
        panic!("Missing completion")
    };
    assert_eq!(first.copied, 2);
    let playlist = tokio::fs::read_to_string(&first.playlist).await.unwrap();
    let root = first.playlist.parent().unwrap();
    for relative in playlist.lines().filter(|line| !line.starts_with('#')) {
        let path = root.join(relative);
        let output = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=noprint_wrappers=1",
            ])
            .arg(path)
            .output()
            .unwrap();
        assert!(output.status.success(), "Copied fixture audio must decode");
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .contains("duration=")
        );
    }
    let before: serde_json::Value = client
        .http()
        .get(format!("{url}/fixture-events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    dispatch_action(
        Action::PlayerSync(Command::Start {
            create_playlist: false,
        }),
        &mut state,
        &mut audio,
        &mut config,
        &events,
    )
    .await
    .unwrap();
    finish(&mut state, &mut audio, &mut config, &events, &mut pending).await;
    let Phase::Complete(second) = &state.player_sync.phase else {
        panic!("Missing completion")
    };
    assert_eq!((second.copied, second.reused), (0, 2));
    let after: serde_json::Value = client
        .http()
        .get(format!("{url}/fixture-events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let downloads = |value: &serde_json::Value| {
        value
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["endpoint"] == "download")
            .count()
    };
    assert_eq!(downloads(&before), downloads(&after));
    assert!(
        !textamp::config::XdgPaths::new("textamp")
            .data_dir
            .join("downloads")
            .exists()
    );
    client
        .call("deletePlaylist", &[("id", playlist_id)])
        .await
        .unwrap();
    assert!(textamp::app::tasks::finish_blocking(Duration::from_secs(3)).await);
    println!(
        "Mounted FAT32: two decodable files, M3U paths verified, repeat sync reused both files, Downloads untouched"
    );
}

#[tokio::test]
#[ignore = "Requires an attached MTP player named by TEXTAMP_TEST_MTP_PLAYER"]
async fn connected_mtp_player_is_visible_without_a_mount() {
    let name = std::env::var("TEXTAMP_TEST_MTP_PLAYER").unwrap();
    assert!(
        textamp::services::player_sync::mtp::discover()
            .await
            .contains(&name)
    );
}
