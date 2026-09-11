use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use textamp::library::{
    cache::Store,
    radio::{select, Recipe},
    FolderLocation, FolderSource,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn source(path: &std::path::Path) -> FolderSource {
    FolderSource {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Fixture".into(),
        location: FolderLocation::Local { path: path.into() },
    }
}
fn album(root: &std::path::Path, path: &str) {
    let dir = root.join(path);
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["10.flac", "2.flac", "01.flac"] {
        std::fs::write(dir.join(name), b"listing fixture").unwrap();
    }
}

#[tokio::test]
async fn random_albums_are_complete_leaf_folders_in_filename_order() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    album(root.path(), "collection/Artist/Album");
    album(root.path(), "Box set/Disc 1");
    album(root.path(), "Box set/Disc 2");
    std::fs::create_dir(root.path().join("Empty")).unwrap();
    std::fs::write(root.path().join("Empty/cover.jpg"), b"cover").unwrap();
    std::fs::write(root.path().join("not-a-leaf.flac"), b"audio").unwrap();
    let source = source(root.path());
    let store = Store::at(cache.path().into(), "fixture").unwrap();
    let mut excluded = HashSet::new();
    let mut albums = HashSet::new();
    for _ in 0..3 {
        let tracks = select(&source, &store, Recipe::RandomAlbum, &excluded)
            .await
            .unwrap();
        assert_eq!(
            tracks
                .iter()
                .map(|t| t.file_name().unwrap())
                .collect::<Vec<_>>(),
            ["01.flac", "2.flac", "10.flac"]
        );
        let parent = tracks[0].parent_title.clone();
        assert!(tracks.iter().all(|t| t.parent_title == parent));
        assert!(albums.insert(parent));
        excluded.extend(tracks.into_iter().map(|t| t.rating_key));
    }
    assert!(select(&source, &store, Recipe::RandomAlbum, &excluded)
        .await
        .unwrap_err()
        .to_string()
        .contains("No unplayed albums"));
    let shuffled = select(&source, &store, Recipe::Library, &HashSet::new())
        .await
        .unwrap();
    assert_eq!(shuffled.len(), 10);
    assert_eq!(
        shuffled
            .iter()
            .map(|t| &t.rating_key)
            .collect::<HashSet<_>>()
            .len(),
        10
    );
    assert!(store.bytes().unwrap() > 0);
}

#[tokio::test]
async fn root_can_be_an_album_and_local_changes_are_not_hidden_by_cache() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    album(root.path(), "");
    let source = source(root.path());
    let store = Store::at(cache.path().into(), "fixture").unwrap();
    assert_eq!(
        select(&source, &store, Recipe::RandomAlbum, &HashSet::new())
            .await
            .unwrap()
            .len(),
        3
    );
    std::fs::remove_file(root.path().join("2.flac")).unwrap();
    assert_eq!(
        select(&source, &store, Recipe::RandomAlbum, &HashSet::new())
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn webdav_radio_uses_only_listings_and_reuses_the_persistent_cache() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/music/", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let server = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                bytes.push(socket.read_u8().await.unwrap());
            }
            let headers = String::from_utf8(bytes).unwrap();
            assert!(
                headers.starts_with("PROPFIND "),
                "Radio must not download or modify music"
            );
            let len: usize = headers
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|n| n.trim().parse().ok())
                })
                .unwrap_or(0);
            let mut body = vec![0; len];
            socket.read_exact(&mut body).await.unwrap();
            let path = headers.split_whitespace().nth(1).unwrap();
            let entries = match path {
                "/music/" => vec![("/music/", true), ("/music/Album/", true)],
                "/music/Album/" => vec![
                    ("/music/Album/", true),
                    ("/music/Album/10.flac", false),
                    ("/music/Album/2.flac", false),
                ],
                _ => panic!("Unexpected path {path}"),
            };
            count.fetch_add(1, Ordering::SeqCst);
            let responses: String = entries.into_iter().map(|(path, dir)| format!("<d:response><d:href>{path}</d:href><d:propstat><d:prop><d:resourcetype>{}</d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>", if dir { "<d:collection/>" } else { "" })).collect();
            let body = format!("<d:multistatus xmlns:d=\"DAV:\">{responses}</d:multistatus>");
            let response = format!("HTTP/1.1 207 Multi-Status\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    let source = FolderSource {
        id: "webdav-fixture".into(),
        name: "DAV".into(),
        location: FolderLocation::Webdav {
            url,
            username: None,
            password_env: None,
        },
    };
    let cache = tempfile::tempdir().unwrap();
    let store = Store::at(cache.path().into(), "webdav-fixture").unwrap();
    let tracks = select(&source, &store, Recipe::RandomAlbum, &HashSet::new())
        .await
        .unwrap();
    assert_eq!(
        tracks
            .iter()
            .map(|t| t.file_name().unwrap())
            .collect::<Vec<_>>(),
        ["2.flac", "10.flac"]
    );
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    server.abort();
    // Reopen the store, with the server gone: discovery comes from disk.
    let reopened = Store::at(cache.path().into(), "webdav-fixture").unwrap();
    assert_eq!(
        select(&source, &reopened, Recipe::RandomAlbum, &HashSet::new())
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn shared_station_lifecycle_commits_folder_tracks_and_cancels_obsolete_work() {
    use textamp::app::action::{PlaybackAction, RadioAction};
    use textamp::app::dispatch::{dispatch_action, handle_core_event};
    use textamp::app::state::{PlaybackMode, RadioRefill};
    use textamp::app::{Action, AppState, Event};
    let root = tempfile::tempdir().unwrap();
    album(root.path(), "A");
    album(root.path(), "B");
    let source = source(root.path());
    let mut state = AppState::new();
    state.active_library = Some(format!("folder:{}", source.id));
    state.sources.active = textamp::app::sources::ActiveSource::Folder(source.id.clone());
    state.sources.folders.push(source);
    textamp::app::sources::radio::load_folder_stations(&mut state);

    let mut audio = textamp::audio::AudioPlayer::new_without_audio();
    let mut config = textamp::config::Config::default();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Event>(16);
    let action = RadioAction::PlayStation("folder-radio/randomAlbum".into());
    dispatch_action(
        action.clone().into(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    assert!(state.radio_task.is_some());
    let result = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let follow = handle_core_event(result, &mut state, &tx);
    assert!(matches!(
        follow.as_slice(),
        [Action::Radio(RadioAction::PlayCurrentRadioTrack)]
    ));
    assert_eq!(state.playback_mode, PlaybackMode::Radio);
    assert_eq!(state.radio.tracks.len(), 3);
    assert!(state.radio_task.is_none());
    textamp::app::sources::radio::refill(&tx, &mut state);
    assert_eq!(state.radio.refill, RadioRefill::Prefetching);
    let result = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(handle_core_event(result, &mut state, &tx).is_empty());
    assert_eq!(state.radio.tracks.len(), 6);
    assert_eq!(state.radio.track_index, Some(0));
    dispatch_action(
        action.clone().into(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    let stale = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    dispatch_action(
        PlaybackAction::Stop.into(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    assert!(state.radio_task.is_none());
    assert!(handle_core_event(stale, &mut state, &tx).is_empty());
    assert!(state.station_starting.is_none());
    dispatch_action(action.into(), &mut state, &mut audio, &mut config, &tx)
        .await
        .unwrap();
    state.advance_library_generation();
    assert!(state.radio_task.is_none());
}

#[tokio::test]
async fn prepared_metadata_updates_radio_even_when_audio_output_is_unavailable() {
    use textamp::app::sources::{self, FileAction};
    use textamp::app::state::PlaybackMode;
    use textamp::library::{track::Track, MediaFile};
    let mut state = textamp::app::AppState::new();
    state.queue.tracks = vec![Track::from_folder("fixture", "inactive.flac")];
    state.queue.index = Some(0);
    state.radio.tracks = vec![Track::from_folder("fixture", "Album/song.flac")];
    state.radio.track_index = Some(0);
    state.set_playback_mode(PlaybackMode::Radio);
    let mut track = state.current_track().unwrap().clone();
    track.title = "Embedded title".into();
    track.duration = Some(123_000);
    let action = FileAction::Prepared {
        generation: state.library_generation,
        id: state.playback.preparation_id,
        track: Box::new(track),
        artwork: Some(vec![1]),
        result: Ok(MediaFile::local("/unused-without-audio".into())),
    };
    let (tx, _) = tokio::sync::mpsc::channel(8);
    sources::dispatch(
        action.into(),
        &mut state,
        &mut textamp::audio::AudioPlayer::new_without_audio(),
        &mut textamp::config::Config::default(),
        &tx,
    )
    .await
    .unwrap();
    assert_eq!(state.current_track().unwrap().title, "Embedded title");
    assert_eq!(state.queue.tracks[0].title, "inactive.flac");
    assert_eq!(state.playback.duration_ms, 123_000);
    assert_eq!(state.artwork.current_data, Some(vec![1]));
}
