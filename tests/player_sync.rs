use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use textamp::{
    navidrome::{Client, Source},
    services::{
        downloads,
        player_sync::{self, Player, Song, Volume, volumes::Target},
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Notify,
};
struct Server {
    url: String,
    calls: Arc<Mutex<Vec<String>>>,
    fail: Arc<Mutex<Option<String>>>,
    bodies: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    started: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn audio(id: &str) -> Vec<u8> {
    format!("RIFF fixture audio for {id}").into_bytes()
}
async fn server(stall: bool) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(Mutex::new(vec![]));
    let fail: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let bodies = Arc::new(Mutex::new(HashMap::new()));
    let payloads = Arc::clone(&bodies);
    let started = Arc::new(Notify::new());
    let (seen, errors, notice) = (calls.clone(), fail.clone(), started.clone());
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut connection, _)) = listener.accept().await else {
                break;
            };
            let mut request = vec![];
            loop {
                let mut bytes = [0; 4096];
                let count = connection.read(&mut bytes).await.unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&bytes[..count]);
                if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    break;
                }
            }
            let text = String::from_utf8(request).unwrap();
            let path = text.split_whitespace().nth(1).unwrap();
            let parsed = reqwest::Url::parse(&format!("http://localhost{path}")).unwrap();
            if parsed.path() == "/rest/scrobble" {
                let (headers, initial) = text.split_once("\r\n\r\n").unwrap();
                assert!(headers.starts_with("POST "));
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        let (k, v) = line.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse().unwrap())
                    })
                    .unwrap();
                let mut body = initial.as_bytes().to_vec();
                let received = body.len();
                body.resize(length, 0);
                connection.read_exact(&mut body[received..]).await.unwrap();
                let params: HashMap<String, String> = reqwest::Url::parse(&format!(
                    "http://localhost/?{}",
                    String::from_utf8(body).unwrap()
                ))
                .unwrap()
                .query_pairs()
                .into_owned()
                .collect();
                assert_eq!(params["submission"], "true");
                seen.lock()
                    .unwrap()
                    .push(format!("listen/{}/{}", params["id"], params["time"]));
                let ok = br#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;
                connection.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",ok.len()).as_bytes()).await.unwrap();
                connection.write_all(ok).await.unwrap();
                continue;
            }
            assert_eq!(parsed.path(), "/rest/download");
            let fields: HashMap<_, _> = parsed.query_pairs().into_owned().collect();
            let id = fields["id"].clone();
            seen.lock().unwrap().push(id.clone());
            let failure = errors.lock().unwrap().as_deref() == Some(&id);
            let body = match id.as_str() {
                "html" => b"<html>Sign in</html>".to_vec(),
                "empty" => vec![],
                _ => payloads
                    .lock()
                    .unwrap()
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| audio(&id)),
            };
            let mime = if id == "html" {
                "text/html"
            } else {
                "audio/mpeg"
            };
            let status = if failure { "503 Unavailable" } else { "200 OK" };
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            connection.write_all(header.as_bytes()).await.unwrap();
            if stall {
                notice.notify_one();
                std::future::pending::<()>().await;
            }
            let _ = connection.write_all(&body).await;
        }
    });
    Server {
        url,
        calls,
        fail,
        bodies,
        started,
        task,
    }
}
#[tokio::test]
async fn interrupted_playlist_publication_can_resume_without_losing_ownership() {
    let server = server(false).await;
    let client = client(&server);
    let folder = tempfile::tempdir().unwrap();
    let volume = mounted_volume(folder.path());
    let target = Target::open(folder.path()).await.unwrap();
    let first = player_sync::sync(&client, &target, &volume, &player(), &[song("a")], |_| {})
        .await
        .unwrap();
    let original = std::fs::read(&first.playlist).unwrap();
    let mut manifest = index(folder.path());
    manifest["playlists"] =
        serde_json::json!([first.playlist.file_name().unwrap().to_str().unwrap()]);
    std::fs::remove_file(&first.playlist).unwrap();
    std::fs::write(
        folder.path().join("Textamp fixture/.textamp-sync.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let next = player_sync::sync(&client, &target, &volume, &player(), &[song("a")], |_| {})
        .await
        .unwrap();
    assert_eq!((next.copied, next.reused), (0, 1));
    assert_eq!(
        index(folder.path())["playlists"],
        serde_json::json!(["NeoPod Synchronizer.m3u"])
    );
    assert_eq!(std::fs::read(next.playlist).unwrap(), original);
}

#[tokio::test]
async fn interrupted_track_publication_reopens_by_song_id_without_a_duplicate() {
    let server = server(false).await;
    let client = client(&server);
    let folder = tempfile::tempdir().unwrap();
    let volume = mounted_volume(folder.path());
    let target = Target::open(folder.path()).await.unwrap();
    player_sync::sync(&client, &target, &volume, &player(), &songs(), |_| {})
        .await
        .unwrap();
    let root = folder.path().join("Textamp fixture");
    let mut interrupted = index(folder.path());
    let published = interrupted["files"]
        .as_object_mut()
        .unwrap()
        .remove("a")
        .unwrap();
    let path = published["path"].as_str().unwrap().to_owned();
    interrupted["pending_file"] = serde_json::json!({"song_id":"a","entry":published,"digest":format!("{:x}",md5::compute(audio("a")))});
    std::fs::write(
        root.join(".textamp-sync.json"),
        serde_json::to_vec(&interrupted).unwrap(),
    )
    .unwrap();
    server.calls.lock().unwrap().clear();
    let reopened = Target::open(folder.path()).await.unwrap();
    let summary = player_sync::sync(&client, &reopened, &volume, &player(), &songs(), |_| {})
        .await
        .unwrap();
    assert_eq!((summary.copied, summary.reused), (0, 2));
    assert!(server.calls.lock().unwrap().is_empty());
    let current = index(folder.path());
    assert!(current.get("pending_file").is_none());
    assert_eq!(current["files"]["a"]["path"], path);
    assert_eq!(std::fs::read(root.join(&path)).unwrap(), audio("a"));
    assert_eq!(
        std::fs::read_dir(root.join(path).parent().unwrap())
            .unwrap()
            .count(),
        2
    );
}

#[tokio::test]
async fn login_documents_and_empty_audio_do_not_replace_a_working_playlist() {
    let server = server(false).await;
    let client = client(&server);
    let folder = tempfile::tempdir().unwrap();
    let volume = mounted_volume(folder.path());
    let target = Target::open(folder.path()).await.unwrap();
    let first = player_sync::sync(&client, &target, &volume, &player(), &[song("a")], |_| {})
        .await
        .unwrap();
    let playlist = std::fs::read(first.playlist).unwrap();
    for (id, message) in [
        ("html", "document instead of audio"),
        ("empty", "empty audio"),
    ] {
        let error = player_sync::sync(&client, &target, &volume, &player(), &[song(id)], |_| {})
            .await
            .unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
        assert_eq!(index(folder.path())["files"].as_object().unwrap().len(), 1);
        assert_eq!(
            std::fs::read_dir(folder.path().join("Textamp fixture/Artist/Unknown Album"))
                .unwrap()
                .count(),
            1
        );
    }
    let next = player_sync::sync(&client, &target, &volume, &player(), &[song("a")], |_| {})
        .await
        .unwrap();
    assert_eq!(std::fs::read(next.playlist).unwrap(), playlist);
}
fn client(server: &Server) -> Client {
    Client::new(
        &Source {
            id: "fixture".into(),
            name: "Fixture".into(),
            url: server.url.clone(),
            username: "fixture".into(),
            libraries: vec![],
        },
        "fixture-password".into(),
        None,
    )
    .unwrap()
}
fn mounted_volume(root: &std::path::Path) -> Volume {
    Volume {
        uuid: "fixture-volume".into(),
        name: "Fixture Player".into(),
        mount: root.into(),
        identifier: "fixture".into(),
        available: 1024 * 1024 * 128,
        capacity: 1024 * 1024 * 256,
    }
}
fn player() -> Player {
    Player {
        volume_uuid: "fixture-volume".into(),
        name: "Y2".into(),
        source_id: "fixture".into(),
        playlists: vec!["NeoPod Synchronizer".into()],
        auto_sync: true,
    }
}
fn song(id: &str) -> Song {
    Song{id:id.into(),track:serde_json::from_value(serde_json::json!({"ratingKey":id,"title":format!("Song {id}"),"grandparentTitle":"Artist","duration":60000,"Media":[{"container":"mp3","Part":[{"size":audio(id).len()}]}]})).unwrap()}
}
fn songs() -> Vec<Song> {
    vec![song("a"), song("b"), song("a")]
}
fn index(root: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root.join("Textamp fixture/.textamp-sync.json")).unwrap())
        .unwrap()
}
#[tokio::test]
async fn copies_directly_preserves_order_reuses_audio_and_removes_only_owned_files() {
    let server = server(false).await;
    let client = client(&server);
    let folder = tempfile::tempdir().unwrap();
    let volume = mounted_volume(folder.path());
    let target = Target::open(folder.path()).await.unwrap();
    std::fs::write(folder.path().join("unrelated.mp3"), b"user audio").unwrap();
    let summary = player_sync::sync(&client, &target, &volume, &player(), &songs(), |_| {})
        .await
        .unwrap();
    assert_eq!(summary.copied, 2);
    assert_eq!(
        summary.playlist.file_name().unwrap(),
        "NeoPod Synchronizer.m3u"
    );
    let manifest = index(folder.path());
    let a = manifest["files"]["a"]["path"].as_str().unwrap();
    let b = manifest["files"]["b"]["path"].as_str().unwrap();
    assert_eq!(
        std::fs::read(folder.path().join("Textamp fixture").join(a)).unwrap(),
        audio("a")
    );
    let text = std::fs::read_to_string(&summary.playlist).unwrap();
    let paths = text
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect::<Vec<_>>();
    assert_eq!(paths, [a, b, a]);
    let mut updated = songs();
    updated[0].track.view_count = Some(100);
    let summary = player_sync::sync(&client, &target, &volume, &player(), &updated, |_| {})
        .await
        .unwrap();
    assert_eq!((summary.copied, summary.reused), (0, 2));
    assert_eq!(server.calls.lock().unwrap().as_slice(), ["a", "b"]);
    std::fs::write(
        folder.path().join("Textamp fixture/unmanaged.mp3"),
        b"unmanaged",
    )
    .unwrap();
    let summary = player_sync::sync(&client, &target, &volume, &player(), &[song("b")], |_| {})
        .await
        .unwrap();
    assert_eq!(summary.removed, 1);
    assert!(folder.path().join("unrelated.mp3").exists());
    assert!(folder.path().join("Textamp fixture/unmanaged.mp3").exists());
    assert!(!folder.path().join("Textamp fixture").join(a).exists());
    assert!(
        !textamp::config::XdgPaths::new("textamp")
            .data_dir
            .join("downloads")
            .exists()
    );
}
#[tokio::test]
async fn two_same_size_catalog_edits_replace_only_those_tracks_and_then_transfer_nothing() {
    let server = server(false).await;
    let client = client(&server);
    let folder = tempfile::tempdir().unwrap();
    let volume = mounted_volume(folder.path());
    let target = Target::open(folder.path()).await.unwrap();
    let root = folder.path().join("Textamp fixture");
    let mut tracks: Vec<_> = ["a", "b", "c", "d"].map(song).into();
    player_sync::sync(&client, &target, &volume, &player(), &tracks, |_| {})
        .await
        .unwrap();
    let original = index(folder.path());
    server.calls.lock().unwrap().clear();
    for i in [0, 2] {
        tracks[i].track.year = Some(1994);
        let mut bytes = audio(&tracks[i].id);
        *bytes.last_mut().unwrap() = b'!';
        server
            .bodies
            .lock()
            .unwrap()
            .insert(tracks[i].id.clone(), bytes);
    }
    let fresh_target = Target::open(folder.path()).await.unwrap();
    let update = player_sync::sync(&client, &fresh_target, &volume, &player(), &tracks, |_| {})
        .await
        .unwrap();
    assert_eq!((update.copied, update.reused), (2, 2));
    assert_eq!(*server.calls.lock().unwrap(), ["a", "c"]);
    let manifest = index(folder.path());
    for t in &tracks {
        let id = t.id.as_str();
        let path = manifest["files"][id]["path"].as_str().unwrap();
        assert_eq!(original["files"][id]["path"], path);
        let expected = server
            .bodies
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .unwrap_or_else(|| audio(id));
        assert_eq!(std::fs::read(root.join(path)).unwrap(), expected);
    }
    let first_path = manifest["files"]["a"]["path"].as_str().unwrap();
    let album = root.join(first_path);
    assert_eq!(
        std::fs::read_dir(album.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "mp3"))
            .count(),
        4
    );
    server.calls.lock().unwrap().clear();
    let repeat = player_sync::sync(&client, &fresh_target, &volume, &player(), &tracks, |_| {})
        .await
        .unwrap();
    assert_eq!((repeat.copied, repeat.reused), (0, 4));
    assert!(server.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn multiple_playlists_share_files_and_remove_only_tracks_no_longer_selected() {
    use player_sync::Playlist;
    let server = server(false).await;
    let client = client(&server);
    let folder = tempfile::tempdir().unwrap();
    let volume = mounted_volume(folder.path());
    let target = Target::open(folder.path()).await.unwrap();
    let mut first = song("a");
    first.track.parent_title = Some("Album".into());
    first.track.index = Some(1);
    let mut second = song("b");
    second.track.title = first.track.title.clone();
    second.track.parent_title = first.track.parent_title.clone();
    second.track.index = Some(1);
    let playlists = [
        Playlist {
            title: "First".into(),
            songs: vec![first.clone(), second.clone(), first.clone()],
        },
        Playlist {
            title: "Second".into(),
            songs: vec![second],
        },
    ];
    player_sync::sync_playlists(
        &client,
        &target,
        &volume,
        &player(),
        &playlists,
        &downloads::Store::default(),
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(server.calls.lock().unwrap().as_slice(), ["a", "b"]);
    let root = folder.path().join("Textamp fixture");
    let manifest = index(folder.path());
    let path = manifest["files"]["a"]["path"].as_str().unwrap();
    let collision = manifest["files"]["b"]["path"].as_str().unwrap();
    assert_eq!(path, "Artist/Album/01 Song a.mp3");
    assert!(collision.starts_with("Artist/Album/01 Song a ["));
    let first_m3u = std::fs::read_to_string(root.join("First.m3u")).unwrap();
    assert_eq!(
        first_m3u
            .lines()
            .filter(|line| !line.starts_with('#'))
            .collect::<Vec<_>>(),
        [path, collision, path]
    );
    let summary = player_sync::sync_playlists(
        &client,
        &target,
        &volume,
        &player(),
        &playlists[1..],
        &downloads::Store::default(),
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!((summary.copied, summary.reused, summary.removed), (0, 1, 1));
    assert!(!root.join("First.m3u").exists());
    assert!(!root.join(path).exists());
    assert_eq!(std::fs::read(root.join(collision)).unwrap(), audio("b"));
    assert!(root.join("Second.m3u").exists());
    player_sync::sync_playlists(
        &client,
        &target,
        &volume,
        &player(),
        &[],
        &downloads::Store::default(),
        |_| {},
    )
    .await
    .unwrap();
    assert!(!root.join("Second.m3u").exists());
    assert!(!root.join(collision).exists());
}

#[tokio::test]
async fn numbers_discs_in_order_and_widens_every_filename_in_large_albums() {
    let server = server(false).await;
    let client = client(&server);
    let folder = tempfile::tempdir().unwrap();
    let volume = mounted_volume(folder.path());
    let target = Target::open(folder.path()).await.unwrap();
    let mut a = song("a");
    a.track.index = Some(1);
    let mut b = song("b");
    b.track.index = Some(99);
    let mut c = song("c");
    c.track.parent_index = Some(2);
    c.track.index = Some(1);
    player_sync::sync(&client, &target, &volume, &player(), &[c, a, b], |_| {})
        .await
        .unwrap();
    let index = index(folder.path());
    for (id, number) in [("a", "001"), ("b", "099"), ("c", "100")] {
        assert_eq!(
            index["files"][id]["path"],
            format!("Artist/Unknown Album/{number} Song {id}.mp3")
        );
    }
}

#[tokio::test]
async fn failed_sync_preserves_previous_playlist_and_successful_copies_resume() {
    let server = server(false).await;
    let client = client(&server);
    let folder = tempfile::tempdir().unwrap();
    let volume = mounted_volume(folder.path());
    let target = Target::open(folder.path()).await.unwrap();
    let first = player_sync::sync(&client, &target, &volume, &player(), &[song("a")], |_| {})
        .await
        .unwrap();
    let previous = std::fs::read(&first.playlist).unwrap();
    let old_audio = index(folder.path())["files"]["a"]["path"]
        .as_str()
        .unwrap()
        .to_owned();
    *server.fail.lock().unwrap() = Some("c".into());
    assert!(
        player_sync::sync(
            &client,
            &target,
            &volume,
            &player(),
            &[song("b"), song("c")],
            |_| {}
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read(&first.playlist).unwrap(), previous);
    assert!(
        folder
            .path()
            .join("Textamp fixture")
            .join(old_audio)
            .exists()
    );
    *server.fail.lock().unwrap() = None;
    let result = player_sync::sync(
        &client,
        &target,
        &volume,
        &player(),
        &[song("b"), song("c")],
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!((result.copied, result.reused, result.removed), (1, 1, 1));
    assert_eq!(
        server
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|id| *id == "b")
            .count(),
        1
    );
}
#[tokio::test]
async fn cancellation_cleans_the_incomplete_usb_file() {
    let server = server(true).await;
    let client = client(&server);
    let folder = tempfile::tempdir().unwrap();
    let volume = mounted_volume(folder.path());
    let task = tokio::spawn(async move {
        let target = Target::open(&volume.mount).await.unwrap();
        player_sync::sync(&client, &target, &volume, &player(), &[song("a")], |_| {}).await
    });
    server.started.notified().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        std::fs::read_dir(folder.path().join("Textamp fixture/Artist/Unknown Album"))
            .unwrap()
            .count(),
        0
    );
    assert!(
        index(folder.path())["files"]
            .as_object()
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn corrupt_paths_and_symbolic_links_do_not_write_outside_the_sync_folder() {
    let server = server(false).await;
    let client = client(&server);
    let folder = tempfile::tempdir().unwrap();
    let volume = mounted_volume(folder.path());
    let target = Target::open(folder.path()).await.unwrap();
    player_sync::sync(&client, &target, &volume, &player(), &[song("a")], |_| {})
        .await
        .unwrap();
    let mut manifest = index(folder.path());
    manifest["files"]["a"]["path"] = serde_json::json!("../unrelated.mp3");
    std::fs::write(
        folder.path().join("Textamp fixture/.textamp-sync.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(folder.path().join("unrelated.mp3"), b"keep").unwrap();
    assert!(
        player_sync::sync(&client, &target, &volume, &player(), &[], |_| {})
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(folder.path().join("unrelated.mp3")).unwrap(),
        b"keep"
    );
    #[cfg(unix)]
    {
        let other = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(other.path(), root.path().join("Textamp fixture")).unwrap();
        let volume = mounted_volume(root.path());
        let target = Target::open(root.path()).await.unwrap();
        assert!(
            player_sync::sync(&client, &target, &volume, &player(), &[song("a")], |_| {})
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_dir(other.path()).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn replacing_the_mount_contents_does_not_copy_to_the_replacement_player() {
    let server = server(false).await;
    let client = client(&server);
    let folder = tempfile::tempdir().unwrap();
    let volume = mounted_volume(folder.path());
    let target = Target::open(folder.path()).await.unwrap();
    let mut replaced = false;
    let result = player_sync::sync(
        &client,
        &target,
        &volume,
        &player(),
        &[song("a"), song("b")],
        |progress| {
            if progress.completed == 1 && !replaced {
                replaced = true;
                std::fs::rename(
                    folder.path().join("Textamp fixture"),
                    folder.path().join("removed-player"),
                )
                .unwrap();
                std::fs::create_dir(folder.path().join("Textamp fixture")).unwrap();
                std::fs::write(
                    folder.path().join("Textamp fixture/.textamp-volume"),
                    b"another-volume",
                )
                .unwrap();
            }
        },
    )
    .await;
    assert!(result.is_err());
    assert_eq!(server.calls.lock().unwrap().as_slice(), ["a"]);
    assert!(!folder.path().join("Textamp fixture/Artist").exists());
}
#[tokio::test]
async fn unsupported_audio_is_reported_before_any_audio_transfer() {
    let server = server(false).await;
    let client = client(&server);
    let folder = tempfile::tempdir().unwrap();
    let volume = mounted_volume(folder.path());
    let target = Target::open(folder.path()).await.unwrap();
    let mut bad = song("b");
    bad.track.media[0].container = Some("unsupported".into());
    assert!(
        player_sync::sync(
            &client,
            &target,
            &volume,
            &player(),
            &[song("a"), bad],
            |_| {}
        )
        .await
        .is_err()
    );
    assert!(server.calls.lock().unwrap().is_empty());
}

fn add_download(store: &mut downloads::Store, track: textamp::library::track::Track, bytes: &[u8]) {
    let id = downloads::Store::id("fixture", &track);
    let audio = store.root.join(&id).join("audio");
    std::fs::create_dir_all(audio.parent().unwrap()).unwrap();
    std::fs::write(audio, bytes).unwrap();
    store.entries.insert(
        id,
        downloads::Entry {
            library: "fixture".into(),
            library_name: "Fixture".into(),
            track,
            bytes: bytes.len() as u64,
            artwork: false,
        },
    );
}

#[tokio::test]
async fn player_sync_prefers_a_matching_download_and_fetches_other_audio() {
    use textamp::library::track::TrackOrigin;
    let server = server(false).await;
    let client = client(&server);
    let folder = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    let mut store = downloads::Store::load(local.path().into()).unwrap();
    let volume = mounted_volume(folder.path());
    let target = Target::open(folder.path()).await.unwrap();
    for (id, source) in [("a", "fixture"), ("b", "different-library")] {
        let mut track = song(id).track;
        track.origin = TrackOrigin::Navidrome {
            source_id: source.into(),
            song_id: id.into(),
        };
        add_download(&mut store, track, &vec![b'x'; audio(id).len()]);
    }
    let local_bytes = vec![b'x'; audio("a").len()];
    player_sync::sync_playlists(
        &client,
        &target,
        &volume,
        &player(),
        &[player_sync::Playlist {
            title: "Local".into(),
            songs: vec![song("a"), song("b")],
        }],
        &store,
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(server.calls.lock().unwrap().as_slice(), ["b"]);
    let manifest = index(folder.path());
    let read = |id: &str| {
        std::fs::read(
            folder
                .path()
                .join("Textamp fixture")
                .join(manifest["files"][id]["path"].as_str().unwrap()),
        )
        .unwrap()
    };
    assert_eq!(read("a"), local_bytes);
    assert_eq!(read("b"), audio("b"));
    let mut edited = song("a");
    edited.track.year = Some(1967);
    let updated = player_sync::sync_playlists(
        &client,
        &target,
        &volume,
        &player(),
        &[player_sync::Playlist {
            title: "Local".into(),
            songs: vec![edited, song("b")],
        }],
        &store,
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!((updated.copied, updated.reused), (1, 1));
    assert_eq!(server.calls.lock().unwrap().as_slice(), ["b", "a"]);
    let new_index = index(folder.path());
    let latest = new_index["files"]["a"]["path"].as_str().unwrap();
    assert_eq!(latest, manifest["files"]["a"]["path"].as_str().unwrap());
    let path = folder.path().join("Textamp fixture").join(latest);
    assert_eq!(std::fs::read(&path).unwrap(), audio("a"));
    let modified = path.metadata().unwrap().modified().unwrap();
    let mut same_audio = song("a");
    same_audio.track.year = Some(1968);
    for _ in 0..2 {
        let summary = player_sync::sync_playlists(
            &client,
            &target,
            &volume,
            &player(),
            &[player_sync::Playlist {
                title: "Local".into(),
                songs: vec![same_audio.clone(), song("b")],
            }],
            &store,
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!((summary.copied, summary.reused), (0, 2));
        assert_eq!(path.metadata().unwrap().modified().unwrap(), modified);
    }
    assert_eq!(server.calls.lock().unwrap().as_slice(), ["b", "a", "a"]);
}

#[tokio::test]
async fn webdav_folders_keep_original_names_and_own_only_their_media_marker() {
    use player_sync::{WebdavSelection, sync_webdav};
    use textamp::library::{FolderLocation, FolderSource};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let source = FolderSource {
        id: "beatles".into(),
        name: "Fixture WebDAV".into(),
        location: FolderLocation::Webdav {
            url: format!("http://{}/music/", listener.local_addr().unwrap()),
            username: None,
            password_env: None,
        },
    };
    let get_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let requests = get_count.clone();
    let heads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let fixture_heads = heads.clone();
    let edited = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let fixture_edited = edited.clone();
    let no_headers = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let omit_headers = no_headers.clone();
    let no_listing_validator = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let omit_listing = no_listing_validator.clone();
    let server = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut bytes = Vec::new();
            let (method, path) = loop {
                let mut buffer = [0; 4096];
                let size = stream.read(&mut buffer).await.unwrap();
                assert!(size > 0);
                bytes.extend_from_slice(&buffer[..size]);
                if bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    let request = String::from_utf8_lossy(&bytes);
                    let mut fields = request.split_whitespace();
                    break (
                        fields.next().unwrap().to_owned(),
                        fields.next().unwrap().to_owned(),
                    );
                }
            };
            let etag = if fixture_edited.load(std::sync::atomic::Ordering::Relaxed) {
                "edited"
            } else {
                "original"
            };
            let listing = format!(
                r#"<multistatus xmlns="DAV:"><response><href>/music/Concerts/Paris/Original%20name.FLAC</href><propstat><prop><getetag>"{etag}"</getetag></prop><status>HTTP/1.1 200 OK</status></propstat><propstat><prop><resourcetype/></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#
            );
            let listing = if omit_listing.load(std::sync::atomic::Ordering::Relaxed) {
                listing.replace(&format!("<getetag>\"{etag}\"</getetag>"), "")
            } else {
                listing
            };
            let (status, headers, body) = match (method.as_str(), path.as_str()) {
                ("PROPFIND", "/music/Concerts/") => (
                    "207 Multi-Status",
                    "",
                    r#"<multistatus xmlns="DAV:"><response><href>/music/Concerts/Paris/</href><propstat><prop><resourcetype><collection/></resourcetype></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#,
                ),
                ("PROPFIND", "/music/Concerts/Paris/") => {
                    ("207 Multi-Status", "", listing.as_str())
                }
                ("GET" | "HEAD", "/music/Concerts/Paris/Original%20name.FLAC") => {
                    if method == "GET" {
                        requests.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    } else {
                        fixture_heads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    if fixture_edited.load(std::sync::atomic::Ordering::Relaxed) {
                        (
                            "200 OK",
                            "Content-Type: audio/flac\r\nETag: \"edited\"\r\n",
                            "updated audio",
                        )
                    } else {
                        (
                            "200 OK",
                            "Content-Type: audio/flac\r\nETag: \"original\"\r\n",
                            "fixture audio",
                        )
                    }
                }
                other => panic!("Unexpected request: {other:?}"),
            };
            let headers = if omit_headers.load(std::sync::atomic::Ordering::Relaxed)
                && matches!(method.as_str(), "GET" | "HEAD")
            {
                "Content-Type: audio/flac\r\n"
            } else {
                headers
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n{}",
                body.len(),
                if method == "HEAD" { "" } else { body }
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
    let folder = tempfile::tempdir().unwrap();
    let volume = mounted_volume(folder.path());
    let target = Target::open(folder.path()).await.unwrap();
    let separate = folder.path().join("Textamp fixture");
    std::fs::create_dir(&separate).unwrap();
    let books = folder.path().join("Audiobooks");
    std::fs::create_dir(&books).unwrap();
    let mut selection = WebdavSelection::default();
    selection.folders.insert("Concerts".into());
    let root = folder.path().join("Textamp beatles");
    let rel = "Concerts/Paris/Original name.FLAC";
    let cancelled = player_sync::review_webdav(
        &source,
        &source.id,
        &target,
        &volume,
        &selection,
        &downloads::Store::default(),
        |_| {},
        |plan| async move {
            assert_eq!(plan[player_sync::ADD], [rel]);
            assert!(plan[player_sync::REMOVE].is_empty());
            anyhow::bail!("Review dismissed")
        },
    )
    .await;
    assert!(cancelled.is_err());
    assert!(!root.exists());
    let first = sync_webdav(
        &source,
        &target,
        &volume,
        &selection,
        &downloads::Store::default(),
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(first.copied, 1);
    let root = folder.path().join("Textamp beatles");
    let track = root.join("Concerts/Paris/Original name.FLAC");
    assert_eq!(std::fs::read(&track).unwrap(), b"fixture audio");
    assert_eq!(std::fs::metadata(root.join(".nomedia")).unwrap().len(), 0);
    assert!(!books.join(".nomedia").exists());
    assert!(!separate.join(".nomedia").exists());

    assert!(root.join("Concerts/.nomedia").is_file());
    assert!(root.join("Concerts/Paris/.nomedia").is_file());
    // The rename reached disk but the completed index write did not.
    let index_path = root.join(".textamp-sync.json");
    let mut interrupted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&index_path).unwrap()).unwrap();
    let published = interrupted["files"]
        .as_object_mut()
        .unwrap()
        .remove(rel)
        .unwrap();
    interrupted["pending_file"] = serde_json::json!({"entry":published,"digest":format!("{:x}",md5::compute(b"fixture audio"))});
    std::fs::write(&index_path, serde_json::to_vec(&interrupted).unwrap()).unwrap();
    let fresh_profile = tempfile::tempdir().unwrap();
    let fresh_downloads = downloads::Store::load(fresh_profile.path().into()).unwrap();
    let fresh_target = Target::open(folder.path()).await.unwrap();
    let mut other_computer = source.clone();
    other_computer.id = "second-computer-account".into();
    let pending_index_path = index_path.clone();
    let pending_index = interrupted.clone();
    let recovered = player_sync::review_webdav(
        &other_computer,
        &source.id,
        &fresh_target,
        &volume,
        &selection,
        &fresh_downloads,
        |_| {},
        |plan| async move {
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(
                    &std::fs::read(pending_index_path).unwrap()
                )
                .unwrap(),
                pending_index
            );
            assert_eq!(plan[player_sync::KEEP], [rel]);
            assert!(plan[player_sync::ADD].is_empty());
            Ok(())
        },
    )
    .await
    .unwrap();
    assert_eq!((recovered.copied, recovered.reused), (0, 1));
    assert_eq!(get_count.load(std::sync::atomic::Ordering::Relaxed), 1);
    // This same-sized destination is the old audio; the rename never happened.
    std::fs::write(&track, b"earlier audio").unwrap();
    let mut before_rename = interrupted;
    let mut old = before_rename["pending_file"]["entry"].clone();
    old["signature"] = "etag:old".into();
    before_rename["files"][rel] = old;
    std::fs::write(&index_path, serde_json::to_vec(&before_rename).unwrap()).unwrap();
    let restored = sync_webdav(
        &source,
        &target,
        &volume,
        &selection,
        &downloads::Store::default(),
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(restored.copied, 1);
    assert_eq!(std::fs::read(&track).unwrap(), b"fixture audio");
    let local = tempfile::tempdir().unwrap();
    let mut store = downloads::Store::load(local.path().into()).unwrap();
    add_download(
        &mut store,
        textamp::library::track::Track::from_folder("beatles", "Concerts/Paris/Original name.FLAC"),
        b"offline audio",
    );
    let before = get_count.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(before, 2);
    std::fs::remove_file(&track).unwrap();
    let downloaded = sync_webdav(&source, &target, &volume, &selection, &store, |_| {})
        .await
        .unwrap();
    assert_eq!(downloaded.copied, 1);
    assert_eq!(std::fs::read(&track).unwrap(), b"offline audio");
    assert_eq!(get_count.load(std::sync::atomic::Ordering::Relaxed), before);
    let heads_before = heads.load(std::sync::atomic::Ordering::Relaxed);
    let without_cache = player_sync::sync_webdav_device(
        &other_computer,
        &source.id,
        &fresh_target,
        &volume,
        &selection,
        &fresh_downloads,
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!((without_cache.copied, without_cache.reused), (0, 1));
    assert_eq!(
        heads.load(std::sync::atomic::Ordering::Relaxed),
        heads_before
    );
    assert_eq!(get_count.load(std::sync::atomic::Ordering::Relaxed), before);
    selection.nomedia = false;
    let second = sync_webdav(&source, &target, &volume, &selection, &store, |_| {})
        .await
        .unwrap();
    assert_eq!((second.copied, second.reused), (0, 1));
    assert!(!root.join(".nomedia").exists());
    assert!(!root.join("Concerts/Paris/.nomedia").exists());
    assert!(!root.join("Concerts/.nomedia").exists());
    // A manually placed marker is never claimed or removed by Textamp.
    std::fs::write(root.join("Concerts/.nomedia"), b"").unwrap();
    selection.nomedia = true;
    sync_webdav(&source, &target, &volume, &selection, &store, |_| {})
        .await
        .unwrap();
    selection.nomedia = false;
    sync_webdav(&source, &target, &volume, &selection, &store, |_| {})
        .await
        .unwrap();
    assert!(root.join("Concerts/.nomedia").is_file());
    assert!(!root.join("Concerts/Paris/.nomedia").exists());

    let nav = self::server(false).await;
    player_sync::sync(&client(&nav), &target, &volume, &player(), &songs(), |_| {})
        .await
        .unwrap();
    let catalog = player_sync::inventory(&volume, &Default::default())
        .await
        .unwrap();
    assert_eq!(catalog["Textamp beatles"], [rel]);
    assert!(
        catalog["Textamp fixture"]
            .iter()
            .any(|row| row.contains("NeoPod Synchronizer"))
    );

    no_headers.store(true, std::sync::atomic::Ordering::Relaxed);
    edited.store(true, std::sync::atomic::Ordering::Relaxed);
    let refreshed = sync_webdav(&source, &target, &volume, &selection, &store, |_| {})
        .await
        .unwrap();
    assert_eq!(refreshed.copied, 1);
    assert_eq!(std::fs::read(&track).unwrap(), b"updated audio");
    let repeat = sync_webdav(&source, &target, &volume, &selection, &store, |_| {})
        .await
        .unwrap();
    assert_eq!(repeat.reused, 1);
    assert_eq!(
        get_count.load(std::sync::atomic::Ordering::Relaxed),
        before + 1
    );
    no_listing_validator.store(true, std::sync::atomic::Ordering::Relaxed);
    let modified = track.metadata().unwrap().modified().unwrap();
    let verify = player_sync::review_webdav(
        &source,
        &source.id,
        &fresh_target,
        &volume,
        &selection,
        &fresh_downloads,
        |_| {},
        |report| async move {
            assert!(report[player_sync::UPDATE][0].contains("Version unavailable"));
            Ok(())
        },
    )
    .await
    .unwrap();
    assert_eq!((verify.copied, verify.reused, verify.bytes), (0, 1, 0));
    assert_eq!(track.metadata().unwrap().modified().unwrap(), modified);
    assert_eq!(
        get_count.load(std::sync::atomic::Ordering::Relaxed),
        before + 2
    );
    selection.folders.clear();
    assert_eq!(
        player_sync::synced_folders(&volume, "beatles", &selection.folders, false).unwrap(),
        ["Concerts".to_owned()].into()
    );
    let third = sync_webdav(
        &source,
        &target,
        &volume,
        &selection,
        &downloads::Store::default(),
        |_| {},
    )
    .await
    .unwrap();
    assert!(
        player_sync::synced_folders(&volume, "beatles", &selection.folders, false)
            .unwrap()
            .is_empty()
    );
    assert_eq!(third.removed, 1);
    assert!(!track.exists());
    assert!(books.is_dir() && separate.is_dir());
    let nav_index = index(folder.path());
    for id in ["a", "b"] {
        assert_eq!(
            std::fs::read(separate.join(nav_index["files"][id]["path"].as_str().unwrap())).unwrap(),
            audio(id)
        );
    }
    server.abort();
}

#[tokio::test]
async fn inventory_recognizes_optional_scrobbler_log_and_ignores_skips() {
    let temp = tempfile::tempdir().unwrap();
    let volume = Volume {
        uuid: "card".into(),
        name: "Player".into(),
        mount: temp.path().to_owned(),
        identifier: "card".into(),
        available: 0,
        capacity: 0,
    };
    let names = Default::default();
    assert!(
        player_sync::inventory(&volume, &names)
            .await
            .unwrap()
            .is_empty()
    );
    std::fs::write(temp.path().join(".scrobbler.log"),
        "#AUDIOSCROBBLER/1.1\n#TZ/UTC\nArtist\tAlbum\tPlayed\t1\t200\tL\t1700000000\t\nArtist\tAlbum\tSkipped\t2\t200\tS\t1700000123\t\n").unwrap();
    let report = player_sync::inventory(&volume, &names).await.unwrap();
    assert_eq!(
        report["Player listening history"],
        ["Artist · Played · Album · 2023-11-14 22:13 UTC"]
    );
}

#[tokio::test]
async fn imports_only_indexed_completed_listens_at_the_logged_time_once() {
    let server = server(false).await;
    let client = client(&server);
    let dir = tempfile::tempdir().unwrap();
    let volume = mounted_volume(dir.path());
    let target = Target::open(dir.path()).await.unwrap();
    player_sync::sync(&client, &target, &volume, &player(), &songs(), |_| {})
        .await
        .unwrap();
    std::fs::write(dir.path().join(".scrobbler.log"),"#AUDIOSCROBBLER/1.1\n#TZ/UTC\nArtist\t\tSong a\t1\t60\tL\t1710000000\t\nArtist\t\tSong b\t2\t60\tS\t1710000001\t\nOther\tBook\tChapter 1\t1\t100\tL\t1710000002\t\n").unwrap();
    for n in [1, 0] {
        let result = player_sync::sync(&client, &target, &volume, &player(), &songs(), |_| {})
            .await
            .unwrap();
        assert_eq!((result.listens, result.copied), (n, 0));
    }
    let calls = server.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|s| s.starts_with("listen/"))
            .collect::<Vec<_>>(),
        ["listen/a/1710000000000"]
    );
}
