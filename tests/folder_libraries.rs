use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::Duration;
use textamp::app::action::{FolderAction, PlaybackAction, SearchAction};
use textamp::app::handlers::key_input;
use textamp::app::sources::{self, FileAction, LibraryChoice};
use textamp::app::state::{BrowseCategory, InputDialog, InputDialogAction, PlayStatus};
use textamp::app::{Action, AppState};
use textamp::audio::AudioPlayer;
use textamp::config::Config;
use textamp::library::{filename_cmp, track::Track, FolderLocation, FolderSource};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

fn local(path: &std::path::Path) -> FolderSource {
    FolderSource {
        id: "local-test".into(),
        name: "Fixture".into(),
        location: FolderLocation::Local { path: path.into() },
    }
}

#[test]
fn file_libraries_reject_all_server_queue_loads_but_keep_local_queue_edits() {
    use textamp::app::action::QueueAction;
    let mut state = AppState::default();
    state.sources.active = sources::ActiveSource::Folder("fixture".into());
    for action in [
        QueueAction::PlayAlbumNow {
            rating_key: "album".into(),
            title: "Album".into(),
        },
        QueueAction::PlayPlaylistNow {
            playlist_key: "playlist".into(),
            title: "Playlist".into(),
        },
        QueueAction::EnqueueAlbumNext {
            rating_key: "album".into(),
            title: "Album".into(),
        },
        QueueAction::EnqueueArtistTracksNext {
            artist_key: "artist".into(),
            artist_name: "Artist".into(),
        },
    ] {
        assert!(sources::reject_unsupported_action(
            &action.into(),
            &mut state
        ));
    }
    for action in [
        QueueAction::ClearQueue,
        QueueAction::EnqueueSelection,
        QueueAction::EnqueueSelectionNext,
    ] {
        assert!(!sources::reject_unsupported_action(
            &action.into(),
            &mut state
        ));
    }
}

#[test]
fn filename_order_is_natural_deterministic_and_handles_large_numbers() {
    let mut names = [
        "10.flac",
        "2.flac",
        "01.flac",
        "1.flac",
        "999999999999999999999999.flac",
        "A.flac",
        "a.flac",
    ];
    names.sort_by(|a, b| filename_cmp(a, b));
    assert_eq!(
        names,
        [
            "01.flac",
            "1.flac",
            "2.flac",
            "10.flac",
            "999999999999999999999999.flac",
            "A.flac",
            "a.flac"
        ]
    );
}

#[tokio::test]
async fn local_listing_is_lazy_ordered_and_confined_to_root() {
    let root = tempfile::tempdir().unwrap();
    for file in ["10.flac", "2.mp3", "notes.txt", "é.wav"] {
        std::fs::write(root.path().join(file), b"fixture").unwrap();
    }
    std::fs::create_dir(root.path().join("Album")).unwrap();
    let source = local(root.path());
    let entries = source.list("").await.unwrap();
    assert_eq!(
        entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["Album", "2.mp3", "10.flac", "é.wav"]
    );
    assert!(entries[0].directory);
    assert!(source.list("Album").await.unwrap().is_empty());
    assert!(source.list("..").await.is_err());
    assert!(source.file("/etc/passwd").await.is_err());
    assert!(source.file("Album").await.is_err());
    assert!(source.file("missing.flac").await.is_err());
    assert!(!source.file("2.mp3").await.unwrap().is_temporary());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/passwd", root.path().join("escape.mp3")).unwrap();
        assert!(source.file("escape.mp3").await.is_err());
        assert!(!source
            .list("")
            .await
            .unwrap()
            .iter()
            .any(|e| e.name == "escape.mp3"));
    }
}

#[test]
fn source_identity_is_not_a_name_filename_or_plex_key() {
    let a = Track::from_folder("one", "Album/1.flac");
    let b = Track::from_folder("two", "Album/1.flac");
    assert_ne!(a.rating_key, b.rating_key);
    assert_eq!(a.file_name(), Some("1.flac"));
    assert!(matches!(
        a.origin,
        textamp::library::track::TrackOrigin::Folder { .. }
    ));
    let old: Track =
        serde_json::from_str(r#"{"ratingKey":"1","title":"Existing Plex track"}"#).unwrap();
    assert!(matches!(
        old.origin,
        textamp::library::track::TrackOrigin::Unavailable
    ));
    assert_eq!(
        serde_json::from_str::<Track>(&serde_json::to_string(&a).unwrap())
            .unwrap()
            .origin,
        a.origin
    );
}

#[test]
fn add_source_parsing_rejects_url_passwords_and_preserves_encoded_names() {
    let webdav = sources::source_from_location("https://alice@example.test/My%20Music/").unwrap();
    assert_eq!(webdav.name, "My Music");
    assert!(
        matches!(webdav.location, FolderLocation::Webdav { username: Some(ref user), ref url, .. } if user == "alice" && !url.contains("alice"))
    );
    for bad in [
        "sftp://alice@example.test:2222/Music",
        "relative/path",
        "https://a:password@example.test/music",
        "ftp://example.test/music",
        "sftp://example.test/music",
    ] {
        assert!(sources::source_from_location(bad).is_err(), "{bad}");
    }
}

#[test]
fn library_manager_is_available_without_plex_and_path_entry_keeps_slashes() {
    let mut state = AppState::new();
    let actions = key_input::handle_key(
        KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE),
        &mut state,
        &Config::default(),
    );
    assert!(matches!(
        actions.as_slice(),
        [Action::Search(SearchAction::OpenLibraryPicker)]
    ));
    state.view = textamp::app::state::View::Browse;
    state.popups.input_dialog = Some(InputDialog {
        title: "Location".into(),
        input: "https:".into(),
        action_type: InputDialogAction::FolderLocation,
    });
    for c in "/music.test/folder".chars() {
        assert!(key_input::handle_key(
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
            &mut state,
            &Config::default()
        )
        .is_empty());
    }
    assert_eq!(
        state.popups.input_dialog.as_ref().unwrap().input.as_str(),
        "https:/music.test/folder"
    );
    assert!(!state.list_filter.active);
}

#[tokio::test]
async fn cancelling_password_entry_does_not_change_library_credentials() {
    let source = sources::source_from_location("https://original@example.test/Music").unwrap();
    let mut state = AppState::new();
    state.sources.folders = vec![source.clone()];
    let config = Config {
        folder_sources: vec![source.clone()],
        ..Config::default()
    };
    let mut form = sources::dialogs::WebdavForm::new(Some(&source));
    form.fields[1].value = "replacement".into();
    form.fields[1].cursor = form.fields[1].value.len();
    state.popups.library_dialog = Some(sources::dialogs::Dialog::Webdav(form));
    assert!(key_input::handle_key(
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        &mut state,
        &config
    )
    .is_empty());
    assert!(state.popups.library_dialog.is_none());
    assert_eq!(config.folder_sources[0].location, source.location);
    assert_eq!(state.sources.folders[0].location, source.location);
}

#[tokio::test]
async fn stale_folder_and_media_completions_do_not_change_state() {
    let mut state = AppState::new();
    state.sources.active = textamp::app::sources::ActiveSource::Folder("one".into());
    state.library_generation = 9;
    state.sources.list_id = 3;
    state.library_loading = true;
    state.playback.status = PlayStatus::Paused;
    let (tx, _rx) = mpsc::channel(8);
    let mut audio = AudioPlayer::new_without_audio();
    let mut config = Config::default();

    for action in [
        FileAction::Listed {
            generation: 8,
            id: 3,
            path: "".into(),
            column: 0,
            result: Err("old failure".into()),
            refreshing: false,
        },
        FileAction::Listed {
            generation: 9,
            id: 2,
            path: "".into(),
            column: 0,
            result: Ok(vec![]),
            refreshing: false,
        },
        FileAction::Prepared {
            generation: 8,
            id: 0,
            track: Box::new(Track::from_folder("one", "a.flac")),
            artwork: None,
            result: Err("old playback".into()),
        },
    ] {
        sources::dispatch(action.into(), &mut state, &mut audio, &mut config, &tx)
            .await
            .unwrap();
    }
    assert!(state.folder_state.is_none());
    assert!(state.library_loading);
    assert!(state.notifications.last_error.is_none());
    assert_eq!(state.playback.status, PlayStatus::Paused);
}

#[tokio::test]
async fn backing_out_discards_a_pending_folder_load() {
    let mut state = AppState::new();
    state.view = textamp::app::state::View::Browse;
    state.browse_category = BrowseCategory::Folders;
    state.category_column_focused = false;
    state.sources.active = textamp::app::sources::ActiveSource::Folder("fixture".into());
    state.library_loading = true;
    let old_id = state.sources.list_id;
    let mut config = Config::default();
    key_input::handle_key(
        KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
        &mut state,
        &config,
    );
    assert!(state.category_column_focused);
    assert!(!state.library_loading);
    assert_ne!(state.sources.list_id, old_id);
    let (tx, _rx) = mpsc::channel(8);
    sources::dispatch(
        FileAction::Listed {
            generation: state.library_generation,
            id: old_id,
            path: "".into(),
            column: 0,
            result: Err("cancelled operation".into()),
            refreshing: false,
        }
        .into(),
        &mut state,
        &mut AudioPlayer::new_without_audio(),
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    assert!(state.notifications.last_error.is_none());
    assert!(state.folder_state.is_none());
}

#[tokio::test]
async fn folder_refresh_preserves_selection_and_mixed_directory_play_index() {
    use textamp::library::FolderEntry;
    let root = tempfile::tempdir().unwrap();
    let source = local(root.path());
    let mut state = AppState::new();
    state.sources.folders = vec![source.clone()];
    state.sources.active = textamp::app::sources::ActiveSource::Folder(source.id.clone());
    state.browse_category = BrowseCategory::Folders;
    let (tx, _rx) = mpsc::channel(16);
    let mut audio = AudioPlayer::new_without_audio();
    let mut config = Config::default();

    let entries: Vec<_> = [("Album", true), ("1.flac", false), ("2.flac", false)]
        .into_iter()
        .map(|(name, directory)| FolderEntry {
            path: name.into(),
            name: name.into(),
            directory,
        })
        .collect();
    let loaded = FileAction::Listed {
        generation: 0,
        id: 0,
        path: "".into(),
        column: 0,
        result: Ok(entries),
        refreshing: false,
    };
    sources::dispatch(
        loaded.clone().into(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    state.folder_state.as_mut().unwrap().columns[0].selected_index = 2;
    sources::dispatch(loaded.into(), &mut state, &mut audio, &mut config, &tx)
        .await
        .unwrap();
    assert_eq!(
        state.folder_state.as_ref().unwrap().columns[0].selected_index,
        2
    );
    sources::folders(FolderAction::PlayFolderTracks, &mut state, &mut audio, &tx);
    assert_eq!(state.queue.tracks.len(), 2);
    assert_eq!(state.queue.index, Some(1));
    assert_eq!(state.current_track().unwrap().file_name(), Some("2.flac"));
    textamp::app::dispatch::dispatch_action(
        PlaybackAction::Stop.into(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    assert!(state.sources.preparing.is_none());
    assert_eq!(state.playback.status, PlayStatus::Stopped);
}

async fn server(
    status: &str,
    body: &'static [u8],
    expected_request: &'static str,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/music/", listener.local_addr().unwrap());
    let status = status.to_owned();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut buffer = [0; 4096];
            let count = stream.read(&mut buffer).await.unwrap();
            assert!(count > 0);
            request.extend_from_slice(&buffer[..count]);
            if request.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        assert!(String::from_utf8_lossy(&request).starts_with(expected_request));
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        stream.write_all(body).await.unwrap();
    });
    (url, task)
}

#[tokio::test]
async fn webdav_lists_encoded_filenames_and_spools_media_with_automatic_cleanup() {
    let body = br#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/music/</d:href></d:response><d:response><d:href>/music/2%20song.flac</d:href><d:propstat><d:prop><d:resourcetype/></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
    let (url, task) = server("207 Multi-Status", body, "PROPFIND /music/").await;
    let mut source = FolderSource {
        id: "dav-test".into(),
        name: "DAV".into(),
        location: FolderLocation::Webdav {
            url,
            username: None,
            password_env: None,
        },
    };
    let entries = tokio::time::timeout(Duration::from_secs(3), source.list(""))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(entries[0].path, "2 song.flac");
    task.await.unwrap();
    let (url, task) = server("200 OK", b"media fixture", "GET /music/2%20song.flac ").await;
    if let FolderLocation::Webdav {
        url: ref mut value, ..
    } = source.location
    {
        *value = url;
    }
    let file = source.file(&entries[0].path).await.unwrap();
    let path = file.path.clone();
    assert!(file.is_temporary());
    assert_eq!(std::fs::read(&path).unwrap(), b"media fixture");
    let owner = file.clone();
    drop(file);
    assert!(path.exists());
    drop(owner);
    assert!(!path.exists());
    task.await.unwrap();
}

#[tokio::test]
async fn webdav_http_failure_is_not_an_empty_library() {
    let (url, task) = server("401 Unauthorized", b"private server error", "PROPFIND ").await;
    let source = FolderSource {
        id: "dav-error".into(),
        name: "DAV".into(),
        location: FolderLocation::Webdav {
            url,
            username: None,
            password_env: None,
        },
    };
    let error = source.list("").await.unwrap_err().to_string();
    assert!(error.contains("401"));
    assert!(!error.contains("private server error"));
    task.await.unwrap();
}

#[test]
fn all_sources_share_one_manager_and_only_folder_categories_are_visible() {
    let mut state = AppState::new();
    state.sources.folders = (0..30)
        .map(|n| FolderSource {
            id: n.to_string(),
            name: format!("Library {n:02}"),
            location: FolderLocation::Local {
                path: "/music".into(),
            },
        })
        .collect();
    state.sources.active = textamp::app::sources::ActiveSource::Folder("20".into());
    let choices = sources::choices(&state);
    assert_eq!(choices.len(), 31); // Saved libraries plus Add library.
    assert_eq!(
        choices.iter().filter(|entry| entry.active(&state)).count(),
        1
    );
    assert!(matches!(choices[20], LibraryChoice::Folder(_)));
    assert_eq!(
        state.category_rows(),
        vec![
            textamp::app::state::CategoryRow::Search,
            textamp::app::state::CategoryRow::Header("Browse"),
            textamp::app::state::CategoryRow::Category(
                textamp::app::state::BrowseCategory::Folders
            ),
        ]
    );
    state.popups.library_picker_index = 20;
    state.sources.picker_scroll_pin = Some(12);
    assert_eq!(sources::picker_offset(&state, 10), 12);
}
