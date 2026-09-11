use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc, time::Duration};
use textamp::app::action::*;
use textamp::app::dispatch::{dispatch_action, handle_core_event};
use textamp::app::sources::navidrome::{NavAction, Session};
use textamp::app::sources::{ActiveSource, LibraryChoice};
use textamp::app::state::{BrowseCategory, BrowseItem, PlayStatus, View};
use textamp::app::{AppState, Event};
use textamp::audio::AudioPlayer;
use textamp::config::Config;
use textamp::navidrome::{Client, Source};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, Mutex};

#[tokio::test]
async fn compilations_browse_from_server_and_offline_cache_and_reset_on_refresh() {
    let (url, requests, task) = server(|method, params| {
        let albums = json!([
            {"id":"solo","name":"Solo","artist":"A","artistId":"a","songCount":1},
            {"id":"comp","name":"Compilation","artist":"A","artistId":"a","songCount":3,"isCompilation":true},
            {"id":"hits","name":"Greatest Hits","artist":"Various Artists","artistId":"va","songCount":2}
        ]);
        let songs = json!([
            {"id":"solo1","title":"Solo","albumId":"solo","artist":"A","artistId":"a","albumArtist":"A","track":1},
            {"id":"comp1","title":"A song","albumId":"comp","artist":"A","artistId":"a","albumArtist":"A","track":1},
            {"id":"comp2","title":"B song","albumId":"comp","artist":"B","artistId":"b","albumArtist":"A","track":2},
            {"id":"comp3","title":"D song","albumId":"comp","artist":"D","artistId":"d","albumArtist":"A","track":3},
            {"id":"hits1","title":"C one","albumId":"hits","artist":"C","artistId":"va","albumArtist":"Various Artists","track":1},
            {"id":"hits2","title":"C two","albumId":"hits","artist":"C","artistId":"va","albumArtist":"Various Artists","track":2}
        ]);
        let payload = match method {
            "getArtists" => json!({"artists":{"index":[{"name":"A","artist":[
                {"id":"a","name":"A"},{"id":"b","name":"B"},{"id":"va","name":"Various Artists"}
            ]}]}}),
            "getAlbumList2" => json!({"albumList2":{"album":albums}}),
            "search3" => json!({"searchResult3":{"song":songs}}),
            _ => fixture(method, params),
        };
        (200, envelope(payload))
    }).await;
    let session = session(url);
    let mut app = App::new(session.clone());
    app.catalog().await;
    task.abort(); // Everything below must work from the saved catalog.
    let calls = requests.lock().await.len();
    let mut app = App::new(session.clone());
    textamp::app::sources::navidrome::open(&mut app.state, &app.tx);
    while app.state.library_loading {
        app.receive().await;
    }
    let root = &app.state.artist_nav.columns[0].items;
    assert!(root
        .iter()
        .any(|item| matches!(item, BrowseItem::Compilations)));
    assert!(root.iter().any(|item| item.key() == session.key("a")));
    assert!(root.iter().any(|item| item.key() == "track_artist:c"));
    assert!(!root
        .iter()
        .any(|item| item.key() == session.key("b") || item.key() == session.key("va")));

    app.send(MillerAction::LoadArtistAlbumsForMiller {
        artist_key: session.key("a"),
        replace_child: true,
    })
    .await;
    let items = &app.state.artist_nav.columns.last().unwrap().items;
    assert!(items.iter().any(|item| item.key() == session.key("solo")));
    assert!(!items.iter().any(|item| item.key() == session.key("comp")));
    assert!(items
        .iter()
        .any(|item| matches!(item, BrowseItem::CompilationTracks { .. })));
    // A compilation-only artist, reached by search, needs no getArtist fallback.
    app.send(MillerAction::LoadArtistAlbumsForMiller {
        artist_key: session.key("b"),
        replace_child: true,
    })
    .await;
    assert!(app
        .state
        .artist_nav
        .columns
        .last()
        .unwrap()
        .items
        .iter()
        .any(|item| matches!(item, BrowseItem::CompilationTracks { .. })));
    assert!(!app.state.sources.nav_tasks.contains_key("artist-browse"));
    app.send(MillerAction::LoadCompilationAlbumsForMiller {
        artist_key: session.key("b"),
        artist_name: "B".into(),
        replace_child: true,
    })
    .await;
    assert!(app
        .state
        .artist_nav
        .columns
        .last()
        .unwrap()
        .items
        .iter()
        .any(|item| item.key() == session.key("comp")));
    app.send(MillerAction::LoadCompilationAllTracksForMiller {
        artist_key: session.key("b"),
        artist_name: "B".into(),
        replace_child: true,
    })
    .await;
    assert_eq!(
        app.state
            .artist_nav
            .columns
            .last()
            .unwrap()
            .tracks
            .iter()
            .map(|t| t.title.as_str())
            .collect::<Vec<_>>(),
        ["A song", "B song", "D song"]
    );
    app.send(MillerAction::LoadArtistAlbumsForMiller {
        artist_key: "track_artist:c".into(),
        replace_child: true,
    })
    .await;
    assert_eq!(
        app.state
            .artist_nav
            .columns
            .last()
            .unwrap()
            .items
            .iter()
            .filter(|item| item.key() == session.key("hits"))
            .count(),
        1
    );
    assert!(!app.state.sources.nav_tasks.contains_key("artist-browse"));
    assert_eq!(requests.lock().await.len(), calls);
    app.state.search.query = "D".into();
    app.send(SearchAction::ExecuteLocalSearch).await;
    assert!(app
        .state
        .search
        .results
        .as_ref()
        .unwrap()
        .artists
        .iter()
        .any(|a| a.rating_key == "track_artist:d"));
    assert!(!app.state.artist_nav.columns[0]
        .items
        .iter()
        .any(|item| item.key() == "track_artist:d"));
    app.send(MillerAction::LoadArtistAlbumsForMiller {
        artist_key: "track_artist:d".into(),
        replace_child: true,
    })
    .await;
    assert!(app
        .state
        .artist_nav
        .columns
        .last()
        .unwrap()
        .items
        .iter()
        .any(|item| matches!(item, BrowseItem::CompilationTracks { .. })));
    assert!(!app.state.sources.nav_tasks.contains_key("artist-browse"));
    assert!(
        app.state.notifications.last_error.is_none(),
        "{:?}",
        app.state.notifications.last_error
    );

    let empty = || {
        serde_json::from_value::<textamp::app::sources::navidrome::Catalog>(json!({
            "artists":[],"albums":[],"tracks":[],"playlists":[],"libraries":[],"extensions":[]
        }))
        .unwrap()
        .prepare()
    };
    app.send(NavAction::Catalog {
        generation: app.state.library_generation.wrapping_add(1),
        request_id: app.state.sources.list_id,
        refreshing: false,
        timestamp: 0,
        result: Ok(empty()),
    })
    .await;
    assert_eq!(app.state.library.compilations.albums.len(), 1);
    app.send(NavAction::Catalog {
        generation: app.state.library_generation,
        request_id: app.state.sources.list_id,
        refreshing: false,
        timestamp: 0,
        result: Ok(empty()),
    })
    .await;
    assert!(app.state.library.compilations.albums.is_empty());
    assert!(app.state.library.compilations.artist_map.is_empty());
    assert!(app.state.library.compilations.single_artist.is_empty());
    assert!(app.state.library.track_artists.is_empty());
    assert!(!app.state.artist_nav.columns[0]
        .items
        .iter()
        .any(|item| matches!(item, BrowseItem::Compilations)));
}

#[test]
fn opensubsonic_compilation_flags_and_release_types_are_optional_and_preserved() {
    let session = session("http://example.test".into());
    for extra in [
        json!({"isCompilation":true}),
        json!({"releaseTypes":["Album","Compilation"]}),
        json!({}),
    ] {
        let mut value = json!({"id":"album","name":"Album","artist":"DJ"});
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let album = session.album(serde_json::from_value(value).unwrap());
        assert_eq!(
            album.is_compilation_candidate(),
            !extra.as_object().unwrap().is_empty()
        );
    }
}

#[tokio::test]
async fn selected_album_uses_the_same_index_as_the_shared_album_list() {
    let (url, _, task) = server(|m, p| (200, envelope(fixture(m, p)))).await;
    let session = session(url);
    let mut app = App::new(session.clone());
    app.catalog().await;
    app.state.library.selected_artist_albums = app.state.library.albums.clone();
    app.state.list_state.right_albums_index = 1; // Row zero is All Tracks.
    app.send(DataAction::LoadSelectedAlbumTracks).await;
    assert_eq!(app.state.artist_nav.columns.last().unwrap().tracks.len(), 3);
    assert!(app
        .state
        .artist_nav
        .columns
        .last()
        .unwrap()
        .tracks
        .iter()
        .all(|t| t.parent_rating_key.as_deref() == Some(session.key("album").as_str())));
    task.abort();
}

#[tokio::test]
async fn radio_playlist_save_uses_active_tracks_and_rejects_blank_names() {
    use textamp::app::state::PlaybackMode;
    let (url, requests, task) = server(|m, p| (200, envelope(fixture(m, p)))).await;
    let session = session(url);
    let mut app = App::new(session.clone());
    app.state.queue.tracks =
        vec![session.track(serde_json::from_value(song("inactive", 1, 1)).unwrap())];
    app.state.radio.tracks =
        vec![session.track(serde_json::from_value(song("radio-song", 1, 1)).unwrap())];
    app.state.set_playback_mode(PlaybackMode::Radio);
    app.send(QueueAction::PromptSavePlaylist).await;
    assert_eq!(
        app.state.popups.input_dialog.as_ref().unwrap().title,
        "Save Station as Playlist"
    );
    app.send(QueueAction::SaveQueueAsPlaylist("Station fixture".into()))
        .await;
    while app.state.sources.nav_tasks.contains_key("playlist-write") {
        app.receive().await;
    }
    let calls = requests.lock().await;
    let (_, params) = calls.iter().find(|(m, _)| m == "createPlaylist").unwrap();
    assert_eq!(params["songId"], "radio-song");
    assert_eq!(params["name"], "Station fixture");
    drop(calls);
    app.send(QueueAction::SaveQueueAsPlaylist("   ".into()))
        .await;
    while app.state.sources.nav_tasks.contains_key("playlist-write") {
        app.receive().await;
    }
    assert_eq!(
        app.state.notifications.last_error.as_deref(),
        Some("Playlist name cannot be empty")
    );
    assert_eq!(
        requests
            .lock()
            .await
            .iter()
            .filter(|(m, _)| m == "createPlaylist")
            .count(),
        1
    );
    task.abort();
}

#[tokio::test]
async fn shared_navidrome_actions_use_the_selected_catalog() {
    use textamp::app::state::{BrowseCategory, PlaybackMode};
    let (url, _, server_task) = server(|m, p| (200, envelope(fixture(m, p)))).await;
    let session = session(url);
    let mut app = App::new(session.clone());
    app.catalog().await;
    let actions: Vec<Action> = vec![
        MillerAction::LoadArtistAlbumsForMiller {
            artist_key: session.key("artist"),
            replace_child: false,
        }
        .into(),
        MillerAction::LoadAlbumTracksForMiller {
            album_key: session.key("album"),
            replace_child: false,
        }
        .into(),
        MillerAction::RefreshAlbumTracks {
            album_key: session.key("album"),
        }
        .into(),
        MillerAction::LoadGenreAlbumsForMiller {
            genre_key: "Rock".into(),
            replace_child: false,
        }
        .into(),
        MillerAction::LoadGenreTracksForMiller {
            album_key: session.key("album"),
            replace_child: false,
        }
        .into(),
        MillerAction::LoadPlaylistTracksForMiller {
            playlist_key: session.key("playlist"),
            replace_child: false,
        }
        .into(),
        SearchAction::ShowArtistBio {
            artist_key: session.key("artist"),
            artist_name: "Artist".into(),
        }
        .into(),
        DataAction::LoadSimilarAlbums {
            rating_key: session.key("album"),
            title: "Album".into(),
        }
        .into(),
        DataAction::LoadSimilarTracks {
            rating_key: session.key("first"),
            title: "Song".into(),
        }
        .into(),
        DataAction::LoadRelated {
            artist_key: session.key("artist"),
            title: "Artist".into(),
        }
        .into(),
        QueueAction::EnqueueAlbum {
            rating_key: session.key("album"),
            title: "Album".into(),
        }
        .into(),
        QueueAction::EnqueueArtistTracks {
            artist_key: session.key("artist"),
            artist_name: "Artist".into(),
        }
        .into(),
        RadioAction::PlayStation("nav-radio/randomAlbum".into()).into(),
        RadioAction::PlayStation("nav-radio/randomArtist".into()).into(),
        RadioAction::StartArtistRadio {
            key: session.key("artist"),
            title: "Artist".into(),
        }
        .into(),
    ];
    for action in actions {
        app.state.clear_error();
        app.send(action).await;
        while !app.state.sources.nav_tasks.is_empty() || app.state.radio_task.is_some() {
            app.receive().await;
        }
        assert!(app
            .state
            .notifications
            .last_error
            .as_deref()
            .is_none_or(|error| !error.contains("has no Navidrome handler")));
    }
    app.state.clear_error();
    app.state.set_view(View::Browse);
    app.state.browse_category = BrowseCategory::Folders;
    app.send(FolderAction::LoadFolderRoot).await;
    app.send(FolderAction::NavigateIntoFolder {
        folder_key: session.key("artist"),
        replace_child: false,
    })
    .await;
    app.send(FolderAction::NavigateIntoFolder {
        folder_key: session.key("album"),
        replace_child: false,
    })
    .await;
    app.state
        .folder_state
        .as_mut()
        .unwrap()
        .focused_mut()
        .unwrap()
        .selected_index = 1;
    app.send(FolderAction::PlayFolderTracks).await;
    assert_eq!(app.state.queue.tracks.len(), 2);
    assert_eq!(app.state.queue.tracks[0].rating_key, session.key("second"));

    // A refresh can remove catalog entries while a folder is still open.
    // Enqueue must report that absence, not try a Plex folder endpoint.
    app.state.set_view(View::Browse);
    app.state.set_playback_mode(PlaybackMode::Queue);
    app.state.library.all_tracks.clear();
    app.state.category_column_focused = false;
    for action in [
        QueueAction::EnqueueSelection,
        QueueAction::EnqueueSelectionNext,
    ] {
        app.state.clear_error();
        app.send(action).await;
        assert!(app
            .state
            .notifications
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("refresh with F5")));
        assert_eq!(app.state.queue.tracks.len(), 2);
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    server_task.abort();
}

#[tokio::test]
async fn remix_completion_survives_playback_advancing_but_keeps_the_current_song() {
    use textamp::app::state::PlaybackMode;
    let session = session("http://127.0.0.1:1".into());
    let mut app = App::new(session.clone());
    app.state.set_playback_mode(PlaybackMode::Queue);
    app.state.queue.tracks = ["first", "second", "third"]
        .map(|id| session.track(serde_json::from_value(song(id, 1, 1)).unwrap()))
        .to_vec();
    app.state.queue.index = Some(1);
    app.state.playback.status = PlayStatus::Playing;
    let replacement = session.track(serde_json::from_value(song("replacement", 1, 1)).unwrap());
    app.send(NavAction::QueueEdit {
        expected: app
            .state
            .queue
            .tracks
            .iter()
            .map(|t| t.rating_key.clone())
            .collect(),
        index: Some(0),
        action: Box::new(
            QueueAction::RemixDoppelgangerReady(AsyncBatchOutcome::new(
                vec![
                    (0, replacement.clone()),
                    (1, replacement.clone()),
                    (2, replacement),
                ],
                3,
            ))
            .into(),
        ),
    })
    .await;
    assert_eq!(app.state.queue.tracks[0].rating_key, session.key("first"));
    assert_eq!(app.state.queue.tracks[1].rating_key, session.key("second"));
    assert_eq!(
        app.state.queue.tracks[2].rating_key,
        session.key("replacement")
    );
    assert_eq!(app.state.playback.status, PlayStatus::Playing);
}

#[tokio::test]
async fn twofer_handles_single_tracks_and_does_not_sample_already_queued_tracks() {
    use textamp::app::state::PlaybackMode;
    let session = session("http://127.0.0.1:1".into()); // Twofer must not need a server.
    for queued in [1, 30] {
        let mut app = App::new(session.clone());
        app.state.set_playback_mode(PlaybackMode::Queue);
        app.state.library.all_tracks = (0..queued + 1)
            .map(|i| session.track(serde_json::from_value(song(&i.to_string(), 1, 1)).unwrap()))
            .collect();
        app.state.queue.tracks = app.state.library.all_tracks[..queued].to_vec();
        app.state.queue.index = Some(0);
        app.send(QueueAction::RemixTwofer).await;
        while app.state.sources.nav_tasks.contains_key("remix") {
            app.receive().await;
        }
        assert_eq!(app.state.queue.tracks.len(), queued + 1);
        assert_eq!(
            app.state.queue.tracks[1].rating_key,
            session.key(&queued.to_string()),
            "insert after the first seed even when its neighbor has the same artist"
        );
        assert!(app.state.notifications.last_error.is_none());
    }
}

#[tokio::test]
async fn doppelganger_replacing_only_later_tracks_does_not_stop_playback() {
    use textamp::app::state::PlaybackMode;
    let session = session("http://127.0.0.1:1".into());
    let mut app = App::new(session.clone());
    app.state.set_playback_mode(PlaybackMode::Queue);
    app.state.queue.tracks = ["first", "second"]
        .map(|id| session.track(serde_json::from_value(song(id, 1, 1)).unwrap()))
        .to_vec();
    app.state.queue.index = Some(0);
    app.state.playback.status = PlayStatus::Playing;
    app.state.playback.position_ms = 12345;
    let replacement = session.track(serde_json::from_value(song("replacement", 1, 1)).unwrap());
    app.send(QueueAction::RemixDoppelgangerReady(AsyncBatchOutcome::new(
        vec![(1, replacement)],
        2,
    )))
    .await;
    assert_eq!(app.state.playback.status, PlayStatus::Playing);
    assert_eq!(app.state.playback.position_ms, 12345);
    assert_eq!(
        app.state.queue.tracks[1].rating_key,
        session.key("replacement")
    );
}

#[tokio::test]
async fn all_navidrome_remixes_use_scoped_candidates_and_keep_undo() {
    use textamp::app::state::PlaybackMode;
    for remix in [
        QueueAction::RemixGemini,
        QueueAction::RemixTwofer,
        QueueAction::RemixStretch,
        QueueAction::RemixDoppelganger,
    ] {
        let (url, _, server) = server(|method, params| {
            let mut candidate = song("second", 2, 1);
            candidate["artistId"] = json!("other-artist");
            let result = match method {
                "getOpenSubsonicExtensions" => {
                    json!({"openSubsonicExtensions":[{"name":"sonicSimilarity","versions":[1]}]})
                }
                "getSonicSimilarTracks" => {
                    json!({"sonicMatch":[{"entry":candidate,"similarity":0.95}]})
                }
                "findSonicPath" => json!({"sonicMatch":[
                    {"entry":song(&params["startSongId"],1,1)},
                    {"entry":candidate},
                    {"entry":song(&params["endSongId"],1,2)}
                ]}),
                _ => fixture(method, params),
            };
            (200, envelope(result))
        })
        .await;
        let session = session(url);
        let mut app = App::new(session.clone());
        app.catalog().await;
        app.state.set_playback_mode(PlaybackMode::Queue);
        app.state.queue.tracks = vec![
            session.track(serde_json::from_value(song("first", 1, 1)).unwrap()),
            session.track(serde_json::from_value(song("disc2", 1, 2)).unwrap()),
        ];
        app.state.queue.index = Some(0);
        let replacing = matches!(remix, QueueAction::RemixDoppelganger);
        app.send(remix).await;
        while app.state.sources.nav_tasks.contains_key("remix") {
            app.receive().await;
        }
        assert!(app.state.queue.undo_snapshot.is_some());
        assert!(app
            .state
            .queue
            .tracks
            .iter()
            .any(|t| t.rating_key == session.key("second")));
        assert_eq!(app.state.queue.tracks.len(), if replacing { 2 } else { 3 });
        server.abort();
    }
}

#[tokio::test]
async fn navidrome_station_list_and_random_album_work_without_sonic_or_plex() {
    let (url, seen, server) =
        server(|method, params| (200, envelope(fixture(method, params)))).await;
    let mut app = App::new(session(url));
    app.catalog().await;
    assert!(app
        .state
        .stations
        .iter()
        .any(|s| s.title == "Random Artist Radio"));
    assert!(app
        .state
        .stations
        .iter()
        .any(|s| s.title == "Random Album Radio"));
    seen.lock().await.clear();
    // A drilled category replaces the current list; root stations remain in
    // station_nav and must still supply the shortcut's key and title.
    app.state.stations.clear();
    let actions = textamp::app::handlers::key_input::handle_key(
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::ALT,
        ),
        &mut app.state,
        &app.config,
    );
    assert!(
        matches!(actions.as_slice(), [Action::Radio(RadioAction::PlayStation(key))] if key == "nav-radio/randomAlbum")
    );
    for action in actions {
        app.send(action).await;
    }
    while app.state.station_starting.is_some() {
        app.receive().await;
    }
    // Station creation succeeds; the silent fixture deliberately has no output device.
    assert_eq!(
        app.state.radio.active_station.as_ref().unwrap().title,
        "Random Album Radio"
    );
    assert_eq!(
        app.state.notifications.last_error.as_deref(),
        Some("Playback: No audio output device is available")
    );
    assert_eq!(
        app.state
            .radio
            .tracks
            .iter()
            .map(|t| t.index.unwrap())
            .collect::<Vec<_>>(),
        [1, 2, 1]
    );
    assert_eq!(
        app.state
            .radio
            .tracks
            .iter()
            .map(|t| t.parent_index.unwrap())
            .collect::<Vec<_>>(),
        [1, 1, 2]
    );
    assert!(seen
        .lock()
        .await
        .iter()
        .all(|(method, _)| method == "scrobble" || method == "getCoverArt" || method == "stream"));
    server.abort();
}
#[tokio::test]
async fn navidrome_dj_uses_sonic_api_and_rejects_stale_results() {
    use textamp::app::state::{DjMode, PlaybackMode};
    let (url, seen, server) = server(|method, params| {
        let result = match method {
            "getOpenSubsonicExtensions" => json!({"openSubsonicExtensions":[{"name":"sonicSimilarity","versions":[1]}]}),
            "getSonicSimilarTracks" => json!({"sonicMatch":[{"entry":song("second",2,1),"similarity":0.9},{"entry":song("outside-library",1,1),"similarity":0.8}]}),
            _ => fixture(method, params),
        };
        (200, envelope(result))
    }).await;
    let session = session(url);
    let mut app = App::new(session.clone());
    app.catalog().await;
    app.state.set_playback_mode(PlaybackMode::Queue);
    let first = session.track(serde_json::from_value(song("first", 1, 1)).unwrap());
    let last = session.track(serde_json::from_value(song("disc2", 1, 2)).unwrap());
    app.state.queue.tracks = vec![first, last];
    app.state.queue.index = Some(0);
    app.send(RadioAction::ToggleDjMode(DjMode::Gemini)).await;
    while app.state.dj.inserting {
        app.receive().await;
    }
    assert_eq!(app.state.queue.tracks[1].rating_key, session.key("second"));
    assert_eq!(app.state.queue.tracks.len(), 3);
    assert!(seen
        .lock()
        .await
        .iter()
        .any(|(m, _)| m == "getSonicSimilarTracks"));
    let before = app.state.queue.tracks.len();
    app.send(NavAction::DjReady {
        playback_id: app.state.playback.request_id.wrapping_add(1),
        track: session.key("first"),
        mode: DjMode::Gemini,
        result: Ok(vec![
            session.track(serde_json::from_value(song("stale", 1, 1)).unwrap())
        ]),
    })
    .await;
    assert_eq!(app.state.queue.tracks.len(), before);
    app.send(NavAction::QueueEdit {
        expected: vec!["old-queue".into()],
        index: Some(0),
        action: Box::new(QueueAction::ClearQueue.into()),
    })
    .await;
    assert_eq!(app.state.queue.tracks.len(), before);
    server.abort();
}

#[tokio::test]
async fn navidrome_sonic_station_and_track_action_share_continuation_and_scoped_refills() {
    use textamp::app::state::{PlayStatus, PlaybackMode, RadioRefill};
    let (url, seen, server) = server(|method, params| {
        let result = match method {
            "getSonicSimilarTracks" => {
                let next = if params.get("id").is_some_and(|id| id == "second") { "disc2" } else { "second" };
                json!({"sonicMatch":[{"entry":song(next,2,1),"similarity":0.9},{"entry":song("outside-library",1,1),"similarity":0.8}]})
            }
            _ => fixture(method, params),
        };
        (200, envelope(result))
    }).await;
    for from_track in [false, true] {
        let mut session = session(url.clone());
        session.extensions.insert("sonicSimilarity".into());
        let mut app = App::new(session.clone());
        // No catalog fetch or real audio: isolate the station's API requests.
        app.state.library.all_tracks = ["first", "second", "disc2"]
            .iter()
            .map(|id| session.track(serde_json::from_value(song(id, 1, 1)).unwrap()))
            .collect();
        app.state.set_playback_mode(PlaybackMode::Queue);
        let seed = app.state.library.all_tracks[0].clone();
        app.state.queue.tracks = vec![seed.clone()];
        app.state.queue.index = Some(0);
        app.state.playback.status = PlayStatus::Paused;
        app.state.playback.position_ms = 31_000;
        app.state.playback.request_id = 5;
        seen.lock().await.clear();
        app.send(if from_track {
            RadioAction::StartSonicRadio(Box::new(seed))
        } else {
            RadioAction::PlayStation("nav-radio/sonic".into())
        })
        .await;
        while app.state.station_starting.is_some() {
            app.receive().await;
        }
        assert_eq!(app.state.playback.status, PlayStatus::Paused);
        assert_eq!(app.state.playback.position_ms, 31_000);
        assert_eq!(app.state.playback.request_id, 5);
        assert_eq!(
            app.state
                .radio
                .tracks
                .iter()
                .map(|t| t.rating_key.clone())
                .collect::<Vec<_>>(),
            [session.key("first"), session.key("second")]
        );
        app.state.radio.track_index = Some(1);
        textamp::app::sources::radio::refill(&app.tx, &mut app.state);
        while app.state.radio.refill != RadioRefill::Idle {
            app.receive().await;
        }
        assert_eq!(
            app.state.current_track().unwrap().rating_key,
            session.key("second")
        );
        assert_eq!(app.state.radio.tracks[2].rating_key, session.key("disc2"));
        let requests = seen.lock().await;
        assert_eq!(
            requests.len(),
            2,
            "no playback, Plex, or catalog requests: {requests:?}"
        );
        assert!(requests
            .iter()
            .all(|(method, _)| method == "getSonicSimilarTracks"));
        assert_eq!(requests[0].1["id"], "first");
        assert_eq!(requests[1].1["id"], "second");
    }
    server.abort();
}

#[tokio::test]
async fn random_artist_radio_uses_twenty_five_track_catalog_minimum() {
    let mut app = App::new(session("http://127.0.0.1:1".into()));
    for (artist, count) in [("small", 24), ("large", 25)] {
        for i in 0..count {
            app.state
                .library
                .all_tracks
                .push(textamp::library::models::Track {
                    rating_key: format!("navidrome:account-one:{artist}-{i}"),
                    grandparent_rating_key: Some(artist.into()),
                    ..Default::default()
                });
        }
    }
    {
        let station = "nav-radio/randomArtist";
        app.send(RadioAction::PlayStation(station.into())).await;
        while app.state.station_starting.is_some() {
            app.receive().await;
        }
        assert_eq!(app.state.radio.tracks.len(), 25);
        assert!(app
            .state
            .radio
            .tracks
            .iter()
            .all(|t| t.grandparent_rating_key.as_deref() == Some("large")));
        let catalog = app.state.library.all_tracks.clone();
        app.state
            .library
            .all_tracks
            .retain(|t| t.grandparent_rating_key.as_deref() == Some("small"));
        app.send(RadioAction::PlayStation(station.into())).await;
        while app.state.station_starting.is_some() {
            app.receive().await;
        }
        assert!(app
            .state
            .notifications
            .last_error
            .as_ref()
            .unwrap()
            .contains("at least 25 tracks"));
        assert_eq!(
            app.state.radio.tracks.len(),
            25,
            "failure preserves previous station"
        );
        app.state.library.all_tracks = catalog;
    }
}

fn source(url: String) -> Source {
    Source {
        id: "account-one".into(),
        name: "Test account".into(),
        url,
        username: "fixture".into(),
        libraries: vec![],
    }
}

#[tokio::test]
async fn every_dj_mode_uses_the_shared_queue_reducer() {
    use textamp::app::state::DjMode;
    let (url, _, task) = server(|m, p| {
        let body = match m {
            "getOpenSubsonicExtensions" => json!({"openSubsonicExtensions":[{"name":"sonicSimilarity"}]}),
            "getSonicSimilarTracks" => json!({"sonicMatch":[{"entry":song("disc2",1,2)}]}),
            "findSonicPath" => json!({"sonicMatch":[{"entry":song("first",1,1)},{"entry":song("disc2",1,2)},{"entry":song("second",2,1)}]}),
            "getSimilarSongs2" => json!({"similarSongs2":{"song":[song("disc2",1,2)]}}),
            _ => fixture(m, p),
        };
        (200, envelope(body))
    }).await;
    let mut session = session(url);
    session.extensions.insert("sonicSimilarity".into());
    for mode in [
        DjMode::Twofer,
        DjMode::Contempo,
        DjMode::Groupie,
        DjMode::Gemini,
        DjMode::Freeze,
        DjMode::Stretch,
    ] {
        let mut app = App::new(session.clone());
        app.catalog().await;
        for track in &mut app.state.library.all_tracks {
            track.year = Some(1975);
        }
        let mut first = session.track(serde_json::from_value(song("first", 1, 1)).unwrap());
        first.year = Some(1971);
        let mut next = session.track(serde_json::from_value(song("second", 2, 1)).unwrap());
        next.grandparent_rating_key = Some(session.key("another-artist"));
        app.state.queue.tracks = vec![first, next];
        app.state.queue.index = Some(0);
        app.send(RadioAction::ToggleDjMode(mode)).await;
        while app.state.sources.nav_tasks.contains_key("dj") {
            app.receive().await;
        }
        assert_eq!(app.state.dj.active_mode, Some(mode));
        assert_eq!(
            app.state.queue.tracks[1].rating_key,
            session.key("disc2"),
            "{mode:?}: {:?}",
            app.state.notifications.last_error
        );
        assert_eq!(app.state.queue.tracks.len(), 3, "{mode:?}");
        assert_eq!(
            app.state.queue.index,
            Some(0),
            "DJ insertion must not skip the playing song"
        );
        assert!(!app.state.dj.inserting);
    }
    task.abort();
}

#[tokio::test]
async fn groupie_keeps_same_artist_radio_when_related_service_is_unavailable() {
    use textamp::app::state::DjMode;
    let (url, requests, task) = server(|m, p| {
        if m == "getSimilarSongs2" {
            return (200, json!({"subsonic-response":{"status":"failed","error":{"code":70,"message":"Related service unavailable"}}}).to_string());
        }
        (200, envelope(fixture(m, p)))
    }).await;
    let session = session(url);
    let mut app = App::new(session);
    app.catalog().await;
    app.state.queue.tracks = vec![app.state.library.all_tracks[0].clone()];
    app.state.queue.index = Some(0);
    app.send(RadioAction::ToggleDjMode(DjMode::Groupie)).await;
    while app.state.sources.nav_tasks.contains_key("dj") {
        app.receive().await;
    }
    assert!(app.state.queue.tracks.len() > 1);
    let artist = &app.state.queue.tracks[0].grandparent_rating_key;
    assert!(app
        .state
        .queue
        .tracks
        .iter()
        .all(|t| &t.grandparent_rating_key == artist));
    let artist = artist.clone().unwrap();
    app.send(RadioAction::StartArtistRadio {
        key: artist.clone(),
        title: "Artist Radio".into(),
    })
    .await;
    while app.state.radio_task.is_some() {
        app.receive().await;
    }
    assert!(!app.state.radio.tracks.is_empty());
    assert!(app
        .state
        .radio
        .tracks
        .iter()
        .all(|t| t.grandparent_rating_key.as_ref() == Some(&artist)));
    assert!(!requests
        .lock()
        .await
        .iter()
        .any(|(m, _)| m == "getSonicSimilarTracks"));
    task.abort();
}
fn session(url: String) -> Session {
    let source = source(url);
    Session {
        client: Client::new(&source, "fixture-password".into(), Some("1".into())).unwrap(),
        source,
        extensions: Default::default(),
    }
}
fn song(id: &str, track: u32, disc: u32) -> Value {
    json!({"id":id,"title":format!("Song {id}"),"artist":"Artist","artistId":"artist","albumArtist":"Artist","album":"Album","albumId":"album","track":track,"discNumber":disc,"duration":60,"suffix":"flac"})
}
fn album() -> Value {
    json!({"id":"album","name":"Album","artist":"Artist","artistId":"artist","genre":"Ambient","songCount":3})
}
fn fixture(method: &str, _params: &HashMap<String, String>) -> Value {
    match method {
        "getMusicFolders" => json!({"musicFolders":{"musicFolder":[{"id":"1","name":"Music"}]}}),
        "getArtists" => {
            json!({"artists":{"index":[{"name":"A","artist":[{"id":"artist","name":"Artist"}]}]}})
        }
        "getAlbumList2" => json!({"albumList2":{"album":[album()]}}),
        "getAlbum" => {
            let mut album = album();
            album["song"] = json!([
                song("disc2", 1, 2),
                song("second", 2, 1),
                song("first", 1, 1)
            ]);
            json!({"album":album})
        }
        "getArtist" => json!({"artist":{"id":"artist","name":"Artist","album":[album()]}}),
        "search3" => {
            json!({"searchResult3":{"song":[song("disc2",1,2),song("second",2,1),song("first",1,1)]}})
        }
        "getPlaylists" => {
            json!({"playlists":{"playlist":[{"id":"playlist","name":"Playlist","songCount":2}]}})
        }
        "getPlaylist" => {
            json!({"playlist":{"id":"playlist","name":"Playlist","entry":[song("second",2,1),song("first",1,1)]}})
        }
        "getArtistInfo2" => {
            json!({"artistInfo2":{"biography":"An <b>artist</b> biography.","similarArtist":[]}})
        }
        "getSimilarSongs2" => {
            json!({"similarSongs2":{"song":[song("first",1,1),song("outside-library",1,1)]}})
        }
        _ => json!({}),
    }
}
type Requests = Arc<Mutex<Vec<(String, HashMap<String, String>)>>>;
async fn server(
    respond: impl Fn(&str, &HashMap<String, String>) -> (u16, String) + Send + Sync + 'static,
) -> (String, Requests, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/prefix", listener.local_addr().unwrap());
    let requests: Requests = Default::default();
    let seen = requests.clone();
    let respond = Arc::new(respond);
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let seen = seen.clone();
            let respond = respond.clone();
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                let end = loop {
                    let n = stream.read(&mut buffer).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let header = String::from_utf8(bytes[..end].to_vec()).unwrap();
                let len = header
                    .lines()
                    .find_map(|l| {
                        l.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while bytes.len() < end + len {
                    let n = stream.read(&mut buffer).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&buffer[..n]);
                }
                let path = header.split_whitespace().nth(1).unwrap();
                let url = reqwest::Url::parse(&format!("http://localhost{path}")).unwrap();
                assert!(url.path().starts_with("/prefix/rest/"));
                let method = url.path().rsplit('/').next().unwrap().to_owned();
                let mut params: HashMap<_, _> = url
                    .query_pairs()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                if len > 0 {
                    let form = reqwest::Url::parse(&format!(
                        "http://localhost/?{}",
                        String::from_utf8_lossy(&bytes[end..end + len])
                    ))
                    .unwrap();
                    params.extend(
                        form.query_pairs()
                            .map(|(k, v)| (k.into_owned(), v.into_owned())),
                    );
                    params.insert("_verb".into(), "POST".into());
                }
                seen.lock().await.push((method.clone(), params.clone()));
                let (status, body) = respond(&method, &params);
                if status == 0 {
                    return;
                } // interrupted connection, before any response
                let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    (url, requests, task)
}
fn envelope(mut value: Value) -> String {
    value["status"] = json!("ok");
    value["version"] = json!("1.16.1");
    json!({"subsonic-response":value}).to_string()
}

#[test]
fn system_collections_are_separate_from_saved_playlists_without_name_guessing() {
    use textamp::app::sources::navidrome::commands::CollectionKind;
    use textamp::app::state::CategoryRow;
    let session = session("http://example.test".into());
    let mut app = App::new(session.clone());
    app.state.library.playlists = ["Recently Played", "All Music", "My ❤️ mix"]
        .iter()
        .enumerate()
        .map(|(i, name)| {
            session
                .playlist(serde_json::from_value(json!({"id":i.to_string(),"name":name})).unwrap())
        })
        .collect();
    let rows = app.state.category_rows();
    let system: Vec<_> = rows
        .iter()
        .filter_map(|row| {
            if let CategoryRow::NavidromeCollection(k) = row {
                Some(*k)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(system, CollectionKind::SIDEBAR);
    let first_playlist = rows
        .iter()
        .position(|r| matches!(r, CategoryRow::Playlist(_)))
        .unwrap();
    assert_eq!(rows[first_playlist - 1], CategoryRow::Header("Playlists"));
    assert!(rows[first_playlist..]
        .iter()
        .all(|r| matches!(r, CategoryRow::Playlist(_))));
    assert_eq!(rows[first_playlist..].len(), 3);
    app.state.sources.active = ActiveSource::None;
    assert!(!app
        .state
        .category_rows()
        .iter()
        .any(|r| matches!(r, CategoryRow::NavidromeCollection(_))));
}

#[test]
fn system_sidebar_keyboard_mouse_and_overflow_use_the_rendered_rows() {
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use ratatui::{backend::TestBackend, Terminal};
    use textamp::app::handlers::{key_input, mouse_input};
    use textamp::app::sources::navidrome::commands::CollectionKind;
    use textamp::app::state::{BrowseNavigationState, CategoryRow};
    let session = session("http://example.test".into());
    let mut app = App::new(session.clone());
    app.state.tall_mode = true;
    app.state.category_column_focused = true;
    app.state.library.playlists = (0..40)
        .map(|i| {
            session.playlist(
                serde_json::from_value(json!({"id":i.to_string(),"name":format!("Saved {i:02}")}))
                    .unwrap(),
            )
        })
        .collect();
    app.state.playlist_nav = BrowseNavigationState::with_root(
        "playlists",
        BrowseItem::from_playlists(&app.state.library.playlists),
    );
    let index = app
        .state
        .category_rows()
        .iter()
        .position(|r| {
            matches!(
                r,
                CategoryRow::NavidromeCollection(CollectionKind::RecentlyPlayed)
            )
        })
        .unwrap();
    app.state.category_column_index = index;
    let actions = key_input::handle_key(
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        &mut app.state,
        &app.config,
    );
    assert_eq!(
        format!("{actions:?}"),
        format!("{:?}", vec![CollectionKind::RecentlyPlayed.action()])
    );
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    let render = |terminal: &mut Terminal<TestBackend>, state: &mut AppState| {
        state.terminal_width = 100;
        state.terminal_height = 40;
        terminal
            .draw(|f| {
                state.hit_regions = textamp::ui::render(f, state).hit_regions;
            })
            .unwrap();
    };
    app.state.category_column_index = 0;
    render(&mut terminal, &mut app.state);
    let region = app.state.hit_regions.category_column.clone().unwrap();
    let actions = mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: region.inner.x,
            row: region.inner.y + index as u16,
            modifiers: KeyModifiers::NONE,
        },
        &mut app.state,
    );
    assert_eq!(
        format!("{actions:?}"),
        format!("{:?}", vec![CollectionKind::RecentlyPlayed.action()])
    );
    assert_eq!(app.state.scroll.category, Some(0));
    app.state.category_column_focused = true;
    key_input::handle_key(
        KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
        &mut app.state,
        &app.config,
    );
    assert!(app.state.scroll.category.is_none());
    render(&mut terminal, &mut app.state);
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("Saved 39"));
    assert!(
        app.state
            .hit_regions
            .category_column
            .as_ref()
            .unwrap()
            .scroll_offset
            > 0
    );
    let before = app.state.category_column_index;
    let region = app.state.hit_regions.category_column.clone().unwrap();
    mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: region.inner.x,
            row: region.inner.y,
            modifiers: KeyModifiers::NONE,
        },
        &mut app.state,
    );
    assert_eq!(app.state.scroll.category, Some(region.scroll_offset - 1));
    assert_eq!(app.state.category_column_index, before);
    assert!(app.state.category_column_focused);
}

#[tokio::test]
async fn system_collections_use_scoped_native_endpoints_and_preserve_server_order() {
    use textamp::app::sources::navidrome::commands::{load_collection, Collection, CollectionKind};
    let (url, seen, task) = server(|method, _| {
        let response = match method {
            "getStarred2" => json!({"starred2":{"song":[song("second",2,1),song("first",1,1)],"album":[album()],"artist":[{"id":"artist","name":"Artist"}]}}),
            _ => json!({"albumList2":{"album":[{"id":"z","name":"Z"},{"id":"a","name":"A"}]}}),
        };
        (200, envelope(response))
    }).await;
    let session = session(url);
    for kind in CollectionKind::SIDEBAR {
        let result = load_collection(&session, kind).await.unwrap();
        let seen = seen.lock().await;
        let (method, params) = seen.last().unwrap();
        assert_eq!(params["musicFolderId"], "1");
        match kind {
            CollectionKind::RecentlyPlayed
            | CollectionKind::RecentlyAdded
            | CollectionKind::MostPlayed => {
                assert_eq!(method, "getAlbumList2");
                let expected = match kind {
                    CollectionKind::RecentlyPlayed => "recent",
                    CollectionKind::MostPlayed => "frequent",
                    _ => "newest",
                };
                assert_eq!(params["type"], expected);
                let Collection::Albums(albums) = result else {
                    panic!("not albums")
                };
                assert_eq!(
                    albums.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(),
                    ["Z", "A"]
                );
            }
            _ => {
                assert_eq!(method, "getStarred2");
            }
        }
    }
    task.abort();
}

#[tokio::test]
async fn system_navigation_preserves_sidebar_focus_and_leaving_rejects_stale_results() {
    use textamp::app::sources::navidrome::commands::{Collection, CollectionKind};
    let (url, _, task) = server(|method, params| (200, envelope(fixture(method, params)))).await;
    let mut app = App::new(session(url));
    app.state.category_column_focused = true;
    app.send(CollectionKind::RecentlyPlayed.action()).await;
    assert!(app.state.artist_nav.loading);
    assert!(!app.state.alphabet_strip_visible());
    let old_request = app.state.artist_nav_request_id;
    while app.state.sources.nav_tasks.contains_key("collection") {
        app.receive().await;
    }
    assert!(!app.state.artist_nav.loading);
    assert!(app.state.category_column_focused);
    assert_eq!(app.state.artist_nav.columns.len(), 1);
    assert_eq!(
        app.state.artist_nav.columns[0].title,
        CollectionKind::RecentlyPlayed.label()
    );
    // Native history albums must drill through the normal keyboard path.
    app.state.category_column_focused = false;
    for action in textamp::app::handlers::key_input::handle_key(
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Right,
            crossterm::event::KeyModifiers::NONE,
        ),
        &mut app.state,
        &app.config,
    ) {
        app.send(action).await;
    }
    while app.state.sources.nav_tasks.contains_key("album-browse") {
        app.receive().await;
    }
    assert_eq!(app.state.artist_nav.columns.last().unwrap().tracks.len(), 3);
    assert_eq!(app.state.playback.status, PlayStatus::Stopped);
    assert!(
        !textamp::app::sources::navidrome::commands::entries(&app.state)
            .iter()
            .any(|e| e.label == "Delete playlist")
    );
    app.send(NavigationAction::set_category(BrowseCategory::Library))
        .await;
    assert!(app.state.sources.nav_collection.is_none());
    assert_eq!(app.state.artist_nav.columns[0].title, "artists");
    app.send(NavAction::List {
        request_id: old_request,
        kind: CollectionKind::RecentlyPlayed,
        items: Collection::Albums(vec![]),
    })
    .await;
    assert_eq!(app.state.artist_nav.columns[0].title, "artists");
    task.abort();
}

#[tokio::test]
async fn failed_system_collection_clears_loading_and_can_retry_with_f5() {
    use textamp::app::sources::navidrome::commands::CollectionKind;
    let (url, _, task) = server(|_, _| (500, "temporary failure".into())).await;
    let mut app = App::new(session(url));
    app.send(CollectionKind::RecentlyPlayed.action()).await;
    while app.state.sources.nav_tasks.contains_key("collection") {
        app.receive().await;
    }
    assert!(!app.state.artist_nav.loading);
    assert!(app.state.notifications.last_error.is_some());
    let actions = textamp::app::handlers::helpers::refresh_current_view(&mut app.state);
    assert_eq!(
        format!("{actions:?}"),
        format!("{:?}", vec![CollectionKind::RecentlyPlayed.action()])
    );
    task.abort();
}

#[tokio::test]
async fn refreshed_catalog_drops_deleted_playlist_tracks_without_overwriting_system_view() {
    use textamp::app::sources::navidrome::commands::CollectionKind;
    let mut app = App::new(session("http://example.test".into()));
    app.state.sources.nav_collection = Some(CollectionKind::RecentlyPlayed);
    app.state.artist_nav = textamp::app::state::BrowseNavigationState::with_root("history", vec![]);
    app.state.playlist_nav = textamp::app::state::BrowseNavigationState::with_root(
        "playlists",
        vec![BrowseItem::Playlist {
            key: "deleted".into(),
            title: "Deleted".into(),
            track_count: Some(0),
        }],
    );
    app.state
        .playlist_nav
        .push_column(textamp::app::state::BrowseColumn::new(
            "stale tracks",
            vec![],
        ));
    let catalog: textamp::app::sources::navidrome::Catalog = serde_json::from_value(
        json!({"artists":[],"albums":[],"tracks":[],"playlists":[],"libraries":[],"extensions":[]}),
    )
    .unwrap();
    app.send(NavAction::Catalog {
        generation: app.state.library_generation,
        request_id: app.state.sources.list_id,
        refreshing: false,
        timestamp: 0,
        result: Ok(catalog.prepare()),
    })
    .await;
    assert!(app.state.library.playlists.is_empty());
    assert_eq!(app.state.playlist_nav.columns.len(), 1);
    assert_eq!(app.state.playlist_nav.focused_column, 0);
    assert_eq!(app.state.artist_nav.columns[0].title, "history");
}

#[tokio::test]
async fn audiomuse_browse_uses_existing_tracks_and_normal_navigation_without_playlists() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use textamp::app::handlers::key_input;
    use textamp::app::sources::audiomuse::{self, Command};
    use textamp::app::sources::navidrome::commands::CollectionKind;
    use textamp::audiomuse::{Feature, Snapshot};
    let session = session("http://example.test".into());
    let mut app = App::new(session.clone());
    app.state.library.all_tracks = ["one", "two"]
        .iter()
        .map(|id| session.track(serde_json::from_value(song(id, 1, 1)).unwrap()))
        .collect();
    let library_key = audiomuse::key(&session);
    app.state.sources.audiomuse.connections.insert(
        library_key.clone(),
        textamp::audiomuse::Connection {
            url: "http://unreachable.invalid".into(),
            username: "user".into(),
            server: "Navidrome".into(),
        },
    );
    app.state.sources.audiomuse.snapshot = Some(Arc::new(Snapshot {
        tracks: ["one","two","outside"].into_iter().map(|id| (id.into(),serde_json::from_value(json!({
            "id":id,"fp":"a","tempo":120,"energy":0.8,"key":"C","scale":"minor","mood_vector":"rock:0.7","other_features":"happy:0"
        })).unwrap())).collect(), ..Default::default()
    }));
    app.send(CollectionKind::AudioMuse(Feature::Labels).action())
        .await;
    assert_eq!(app.state.artist_nav.columns[0].items.len(), 1);
    assert_eq!(app.state.artist_nav.columns[0].items[0].title(), "rock");
    app.state.category_column_focused = false;
    for action in key_input::handle_key(
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        &mut app.state,
        &app.config,
    ) {
        app.send(action).await;
    }
    assert_eq!(app.state.artist_nav.focused_column, 1);
    assert_eq!(app.state.artist_nav.columns[1].tracks.len(), 2);
    assert_eq!(app.state.playback.status, PlayStatus::Stopped);
    assert!(app.state.library.playlists.is_empty());
    app.send(QueueAction::EnqueueSelection).await;
    // Existing Ctrl+E semantics: selected track and all following tracks.
    assert_eq!(app.state.queue.tracks.len(), 2);
    assert_eq!(app.state.playback.status, PlayStatus::Stopped);
    for action in key_input::handle_key(
        KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
        &mut app.state,
        &app.config,
    ) {
        app.send(action).await;
    }
    assert_eq!(app.state.artist_nav.focused_column, 0);
    let request_id = app.state.artist_nav_request_id;
    app.send(NavigationAction::set_category(BrowseCategory::Library))
        .await;
    app.send(Command::Results {
        request_id,
        feature: Feature::Labels,
        title: "stale".into(),
        ids: vec!["one".into()],
    })
    .await;
    assert_ne!(app.state.artist_nav.columns[0].title, "stale");
    app.state
        .sources
        .sonic_disabled_libraries
        .insert(library_key);
    textamp::app::sources::sonic::reconcile(&mut app.state);
    assert!(!app.state.category_rows().iter().any(|r| matches!(
        r,
        textamp::app::state::CategoryRow::NavidromeCollection(CollectionKind::AudioMuse(_))
    )));
    assert!(app.state.sources.audiomuse.snapshot.is_none());
    assert_eq!(app.state.queue.tracks.len(), 2);
}

#[tokio::test]
async fn sidebar_headings_are_skipped_and_search_requires_explicit_activation_for_every_source() {
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use ratatui::{backend::TestBackend, Terminal};
    use textamp::app::handlers::{key_input, mouse_input};
    use textamp::app::state::CategoryRow;
    for source in [
        ActiveSource::None,
        ActiveSource::Folder("local".into()),
        ActiveSource::Navidrome(Box::new(session("http://example.test".into()))),
    ] {
        let mut app = App::new(session("http://example.test".into()));
        app.state.sources.active = source;
        app.state.category_column_focused = true;
        let rows = app.state.category_rows();
        assert_eq!(rows[0], CategoryRow::Search);
        assert_eq!(rows[1], CategoryRow::Header("Browse"));
        let search = rows.iter().position(|r| *r == CategoryRow::Search).unwrap();
        app.state.category_column_index = 2;
        let actions = key_input::handle_key(
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            &mut app.state,
            &app.config,
        );
        assert!(actions.is_empty());
        assert_eq!(app.state.category_column_index, search);
        assert!(!app.state.popups.search_active);
        for action in key_input::handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut app.state,
            &app.config,
        ) {
            app.send(action).await;
        }
        assert!(app.state.popups.search_active);
        app.send(SearchAction::CloseSearchPopup).await;
        let mut terminal = Terminal::new(TestBackend::new(120, 45)).unwrap();
        terminal
            .draw(|frame| {
                app.state.hit_regions = textamp::ui::render(frame, &app.state).hit_regions;
            })
            .unwrap();
        let region = app.state.hit_regions.category_column.clone().unwrap();
        let actions = mouse_input::handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: region.inner.x,
                row: region.inner.y + (search - region.scroll_offset) as u16,
                modifiers: KeyModifiers::NONE,
            },
            &mut app.state,
        );
        for action in actions {
            app.send(action).await;
        }
        assert!(app.state.popups.search_active);
        app.send(SearchAction::CloseSearchPopup).await;
        app.state.category_column_focused = true;
        key_input::handle_key(
            KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
            &mut app.state,
            &app.config,
        );
        assert_eq!(app.state.category_column_index, 0);
    }
}

#[tokio::test]
async fn folder_search_popup_searches_files_and_returns_to_the_matching_row() {
    use textamp::library::models::{FolderColumn, FolderItem, FolderNavigationState};
    use textamp::library::track::TrackOrigin;
    let mut app = App::new(session("http://example.test".into()));
    app.state.sources.active = ActiveSource::Folder("local".into());
    app.state.folder_state = Some(FolderNavigationState::with_root(
        "local".into(),
        FolderColumn::new(
            None,
            "Music".into(),
            vec![
                FolderItem::folder("Tomorrow/".into(), "Tomorrow".into()),
                FolderItem::track(
                    "01 Yesterday.flac".into(),
                    "Yesterday".into(),
                    "one".into(),
                    None,
                    None,
                    None,
                ),
                FolderItem::track(
                    "02 Tomorrow.flac".into(),
                    "Tomorrow".into(),
                    "two".into(),
                    None,
                    None,
                    None,
                ),
            ],
        ),
    ));
    app.send(SearchAction::OpenSearchPopup).await;
    app.send(SearchAction::SetSearchQuery("tomorrow".into()))
        .await;
    let results = app.state.search.results.as_ref().unwrap();
    assert_eq!(results.tracks.len(), 1);
    assert!(
        matches!(&results.tracks[0].origin, TrackOrigin::Folder { source_id, path }
        if source_id == "local" && path == "02 Tomorrow.flac")
    );
    app.send(QueueAction::EnqueueSearchResult).await;
    assert_eq!(app.state.queue.tracks.len(), 1);
    app.send(SearchAction::SelectSearchResult).await;
    assert!(!app.state.popups.search_active);
    assert_eq!(app.state.browse_category, BrowseCategory::Folders);
    assert_eq!(
        app.state
            .folder_state
            .as_ref()
            .unwrap()
            .focused()
            .unwrap()
            .selected_index,
        2
    );
    assert_eq!(app.state.queue.tracks.len(), 1);
}

#[tokio::test]
async fn clearing_search_invalidates_old_results_and_requests() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = App::new(session("http://example.test".into()));
    app.state.sources.active = ActiveSource::Folder("local".into());
    app.send(SearchAction::OpenSearchPopup).await;
    app.state.search.query = "a".into();
    app.state.search.results = Some(Default::default());
    for action in textamp::app::handlers::key_input::handle_key(
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        &mut app.state,
        &app.config,
    ) {
        app.send(action).await;
    }
    assert!(app.state.search.query.is_empty());
    assert!(app.state.search.results.is_none());
}

#[test]
fn audiomuse_form_has_masked_fields_buttons_and_cancel_without_saving() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{backend::TestBackend, Terminal};
    use textamp::app::sources::{audiomuse, dialogs};
    let mut app = App::new(session("http://example.test".into()));
    app.state.popups.library_dialog = Some(dialogs::Dialog::AudioMuse(audiomuse::Form {
        instance: "fixture".into(),
        library_key: "library".into(),
        fields: Default::default(),
        focus: 0,
        error: None,
        task: None,
    }));
    for _ in 0..2 {
        dialogs::key(
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            &mut app.state,
        );
    }
    for c in "secret".chars() {
        dialogs::key(
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
            &mut app.state,
        );
    }
    let mut terminal = Terminal::new(TestBackend::new(100, 35)).unwrap();
    terminal
        .draw(|frame| {
            app.state.hit_regions = textamp::ui::render(frame, &app.state).hit_regions;
        })
        .unwrap();
    let output = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(!output.contains("secret"));
    assert!(output.contains("Disconnect"));
    assert_eq!(app.state.hit_regions.library_dialog.len(), 7);
    dialogs::key(
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        &mut app.state,
    );
    assert!(app.state.popups.library_dialog.is_none());
    assert!(app.state.sources.audiomuse.connections.is_empty());
}

struct App {
    state: AppState,

    audio: AudioPlayer,
    config: Config,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
}
impl App {
    fn new(session: Session) -> Self {
        let (tx, rx) = mpsc::channel(100);
        let mut state = AppState::new();
        state.view = View::Browse;
        state.active_library = Some("navidrome:account-one:1".into());
        state.sources.active = ActiveSource::Navidrome(Box::new(session));
        Self {
            state,

            audio: AudioPlayer::new_without_audio(),
            config: Config::default(),
            tx,
            rx,
        }
    }
    async fn send(&mut self, action: impl Into<Action>) {
        dispatch_action(
            action.into(),
            &mut self.state,
            &mut self.audio,
            &mut self.config,
            &self.tx,
        )
        .await
        .unwrap();
    }
    async fn receive(&mut self) {
        let event = tokio::time::timeout(Duration::from_secs(5), self.rx.recv())
            .await
            .unwrap()
            .unwrap();
        for action in handle_core_event(event, &mut self.state, &self.tx) {
            self.send(action).await;
        }
    }
    async fn catalog(&mut self) {
        self.send(SystemAction::RefreshCategory(
            textamp::app::state::RefreshCategory::Artists,
        ))
        .await;
        while self.state.library_loading {
            self.receive().await;
        }
    }
}

#[test]
fn auth_urls_are_salted_scoped_and_password_free() {
    let client = session("https://example.test/subpath".into()).client;
    let first = client.url("stream", &[("id", "song & 2".into())]).unwrap();
    let second = client.url("stream", &[]).unwrap();
    let params: HashMap<_, _> = first.query_pairs().collect();
    assert_eq!(first.path(), "/subpath/rest/stream");
    assert_eq!(params["id"], "song & 2");
    assert_eq!(
        params["t"],
        format!(
            "{:x}",
            md5::compute(format!("fixture-password{}", params["s"]))
        )
    );
    assert_ne!(
        params["s"],
        second.query_pairs().find(|(k, _)| k == "s").unwrap().1
    );
    assert!(!first.as_str().contains("fixture-password"));
    assert!(!format!("{client:?}").contains("fixture-password"));
    for url in [
        "https://user:secret@example.test",
        "file:///tmp",
        "https://example.test/?token=secret",
        "https://example.test/#x",
    ] {
        assert!(source(url.into()).validate().is_err());
    }
}

#[test]
fn configuration_and_ids_keep_accounts_and_libraries_separate() {
    let mut one = session("https://example.test".into());
    let mut two = one.clone();
    two.source.id = "account-two".into();
    assert_ne!(one.key("same/id"), two.key("same/id"));
    assert!(one.id(&two.key("same/id")).is_err());
    assert_eq!(one.id(&one.key("same/id")).unwrap(), "same/id");
    one.source.libraries = vec![
        textamp::navidrome::MusicFolder {
            id: "1".into(),
            name: "Music".into(),
        },
        textamp::navidrome::MusicFolder {
            id: "2".into(),
            name: "Other".into(),
        },
    ];
    let config = Config {
        navidrome_sources: vec![one.source.clone(), two.source.clone()],
        default_navidrome: Some(textamp::navidrome::Selection {
            source_id: one.source.id.clone(),
            folder: None,
        }),
        ..Default::default()
    };
    let encoded = toml::to_string(&config).unwrap();
    let decoded: Config = toml::from_str(&encoded).unwrap();
    assert_eq!(decoded.navidrome_sources.len(), 2);
    assert!(decoded.default_navidrome.unwrap().folder.is_none());
    assert!(!encoded.contains("fixture-password"));
    let mut state = AppState::new();
    state.sources.navidrome = config.navidrome_sources;
    let choices = textamp::app::sources::choices(&state);
    assert_eq!(
        choices
            .iter()
            .filter(|c| matches!(c, LibraryChoice::Navidrome { .. }))
            .count(),
        4
    );
    assert!(Config::default().default_navidrome.is_none());
}

#[tokio::test]
async fn api_failure_inside_http_success_is_not_empty_success() {
    let (url,_,task) = server(|_,_| (200,json!({"subsonic-response":{"status":"failed","error":{"code":40,"message":"secret must not be echoed"}}}).to_string())).await;
    let error = session(url).client.artists().await.unwrap_err().to_string();
    assert!(error.contains("Authentication failed"));
    assert!(!error.contains("secret"));
    task.abort();
    let (url, _, task) = server(|_, _| (200, "<html>invalid</html>".into())).await;
    assert!(session(url).client.artists().await.is_err());
    task.abort();
}

#[tokio::test]
async fn interrupted_reads_retry_once_but_writes_never_retry() {
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = attempts.clone();
    let (url, _, task) = server(move |m, p| {
        if observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0 {
            (0, String::new())
        } else {
            (200, envelope(fixture(m, p)))
        }
    })
    .await;
    assert_eq!(session(url).client.artists().await.unwrap().len(), 1);
    assert_eq!(attempts.load(std::sync::atomic::Ordering::Relaxed), 2);
    task.abort();
    let (url, requests, task) = server(|_, _| (0, String::new())).await;
    let client = session(url).client;
    assert!(client.artists().await.is_err());
    assert_eq!(requests.lock().await.len(), 2);
    assert!(client
        .call("scrobble", &[("id", "test".into())])
        .await
        .is_err());
    assert_eq!(requests.lock().await.len(), 3);
    task.abort();
}

#[tokio::test]
async fn pagination_rejects_repeated_pages_instead_of_hanging_or_truncating() {
    let (url,_,task) = server(|_,_| (200,envelope(json!({"albumList2":{"album":(0..500).map(|i| json!({"id":i.to_string(),"name":"Album"})).collect::<Vec<_>>()}})))).await;
    let error = session(url)
        .client
        .albums("alphabeticalByName", &[])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("repeated"));
    task.abort();
}

#[tokio::test]
async fn catalog_browse_search_playlist_order_and_library_scope() {
    let (url, requests, task) = server(|m, p| (200, envelope(fixture(m, p)))).await;
    let mut app = App::new(session(url));
    app.catalog().await;
    assert_eq!(
        app.state
            .library
            .all_tracks
            .iter()
            .map(|t| t.title.as_str())
            .collect::<Vec<_>>(),
        ["Song first", "Song second", "Song disc2"]
    );
    assert_eq!(app.state.library.artists.len(), 1);

    let session = app.state.sources.active.navidrome().unwrap().clone();
    app.send(MillerAction::LoadArtistAlbumsForMiller {
        artist_key: session.key("artist"),
        replace_child: false,
    })
    .await;
    assert!(app
        .state
        .artist_nav
        .columns
        .last()
        .unwrap()
        .items
        .iter()
        .any(|i| matches!(i,BrowseItem::Album { title,.. } if title == "Album")));
    app.send(MillerAction::LoadAlbumTracksForMiller {
        album_key: session.key("album"),
        replace_child: false,
    })
    .await;
    assert_eq!(app.state.artist_nav.columns.last().unwrap().items.len(), 3);
    app.state.search.query = "second".into();
    app.send(SearchAction::ExecuteLocalSearch).await;
    assert_eq!(
        app.state.search.results.as_ref().unwrap().tracks[0].title,
        "Song second"
    );
    app.state.browse_category = BrowseCategory::Playlists;
    app.send(MillerAction::LoadPlaylistTracksForMiller {
        playlist_key: session.key("playlist"),
        replace_child: false,
    })
    .await;
    while app.state.playlist_nav.loading {
        app.receive().await;
    }
    assert_eq!(
        app.state.playlist_nav.columns.last().unwrap().items[0].key(),
        session.key("second")
    );
    let requests = requests.lock().await;
    for (method, params) in requests
        .iter()
        .filter(|(m, _)| ["getArtists", "getAlbumList2", "search3"].contains(&m.as_str()))
    {
        assert_eq!(
            params.get("musicFolderId").map(String::as_str),
            Some("1"),
            "{method}"
        );
    }
    assert!(requests
        .iter()
        .all(|(_, p)| !p.contains_key("X-Plex-Token")));
    task.abort();
}

#[tokio::test]
async fn every_structured_album_genre_is_browsable_without_inventing_ai_sections() {
    let (url, _, task) = server(|m, p| {
        let mut response = fixture(m, p);
        if m == "getAlbumList2" {
            response["albumList2"]["album"][0]["genre"] = json!("Legacy single field");
            response["albumList2"]["album"][0]["genres"] = json!([
                {"name":"Ambient"},{"name":"Experimental"},{"name":"R&B / Soul"},
                {"name":" Ambient "},{"name":" "},{"name":"ambient dub"}
            ]);
        }
        (200, envelope(response))
    })
    .await;
    let mut app = App::new(session(url));
    app.catalog().await;
    assert_eq!(
        app.state
            .library
            .album_genres
            .iter()
            .map(|g| g.title.as_str())
            .collect::<Vec<_>>(),
        ["Ambient", "ambient dub", "Experimental", "R&B / Soul"]
    );
    assert!(app.state.category_rows().iter().all(|row| !matches!(
        row,
        textamp::app::state::CategoryRow::Category(
            BrowseCategory::ArtistGenres | BrowseCategory::Moods | BrowseCategory::Styles
        )
    )));
    app.state.browse_category = BrowseCategory::AlbumGenres;
    app.send(MillerAction::LoadGenreAlbumsForMiller {
        genre_key: "Experimental".into(),
        replace_child: false,
    })
    .await;
    let key = app.state.tag_nav.columns.last().unwrap().items[0]
        .key()
        .to_owned();
    assert_eq!(app.state.library.albums[0].rating_key, key);
    app.send(MillerAction::LoadGenreTracksForMiller {
        album_key: key,
        replace_child: false,
    })
    .await;
    assert_eq!(app.state.tag_nav.columns.last().unwrap().tracks.len(), 3);
    assert!(app.state.notifications.last_error.is_none());
    task.abort();
}

#[tokio::test]
async fn empty_genres_do_not_retry_and_an_open_genre_view_receives_the_catalog() {
    let (url, requests, task) = server(|m, p| (200, envelope(fixture(m, p)))).await;
    let mut app = App::new(session(url));
    app.state.browse_category = BrowseCategory::AlbumGenres;
    app.send(BrowseAction::RefreshTagView).await;
    assert!(!app.state.tag_nav.loading);
    assert!(app.state.tag_nav.columns[0].items.is_empty());
    assert!(requests.lock().await.is_empty());
    app.catalog().await;
    assert_eq!(app.state.tag_nav.columns[0].items[0].key(), "Ambient");
    assert!(!app.state.tag_nav.loading);
    task.abort();
}

#[tokio::test]
async fn failures_leave_cached_catalog_and_loading_state_recoverable() {
    let fail = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let check = fail.clone();
    let (url, _, task) = server(move |m, p| {
        if check.load(std::sync::atomic::Ordering::Relaxed) {
            (503, "offline".into())
        } else {
            (200, envelope(fixture(m, p)))
        }
    })
    .await;
    let mut app = App::new(session(url));
    app.catalog().await;
    fail.store(true, std::sync::atomic::Ordering::Relaxed);
    app.catalog().await;
    assert_eq!(app.state.library.all_tracks.len(), 3);
    assert!(!app.state.library.artists_loading);
    assert!(app
        .state
        .notifications
        .last_error
        .as_ref()
        .unwrap()
        .contains("existing data is unchanged"));
    fail.store(false, std::sync::atomic::Ordering::Relaxed);
    app.catalog().await;
    assert_eq!(app.state.library.artists.len(), 1);
    task.abort();
}

#[tokio::test]
async fn disk_catalog_opens_without_server_requests_and_force_refresh_retains_it_on_failure() {
    let fail = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let check = fail.clone();
    let (url, requests, task) = server(move |m, p| {
        if check.load(std::sync::atomic::Ordering::Relaxed) {
            (503, "offline".into())
        } else {
            (200, envelope(fixture(m, p)))
        }
    })
    .await;
    let saved_session = session(url);
    let store = saved_session.cache_store().unwrap();
    let mut first = App::new(saved_session.clone());
    first.catalog().await;
    assert_eq!(first.state.library.all_tracks.len(), 3);
    let before = requests.lock().await.len();
    drop(first);
    fail.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut reopened = App::new(saved_session);
    textamp::app::sources::navidrome::open(&mut reopened.state, &reopened.tx);
    while reopened.state.library_loading {
        reopened.receive().await;
    }
    assert_eq!(
        requests.lock().await.len(),
        before,
        "fresh cache should not need the server"
    );
    assert_eq!(reopened.state.library.all_tracks.len(), 3);
    assert_eq!(reopened.state.library.album_genres[0].title, "Ambient");
    assert_eq!(reopened.state.library.playlists.len(), 1);
    reopened.state.library.playlists.clear();
    for action in [
        DataAction::LoadInitialData,
        DataAction::LoadArtists,
        DataAction::LoadPlaylists,
    ] {
        reopened.send(action).await;
        assert!(
            !reopened.state.library_loading,
            "ordinary loads reuse the in-memory catalog, even with empty playlists"
        );
    }
    assert_eq!(
        requests.lock().await.len(),
        before,
        "only explicit refresh bypasses the cache"
    );
    reopened.catalog().await;
    assert!(
        requests.lock().await.len() > before,
        "forced refresh bypasses the cache"
    );
    assert_eq!(reopened.state.library.all_tracks.len(), 3);
    let cache = store
        .ticket("catalog")
        .read::<Value>(Duration::ZERO)
        .unwrap()
        .unwrap();
    assert_eq!(cache.value["tracks"].as_array().unwrap().len(), 3);
    let encoded = serde_json::to_string(&cache.value).unwrap();
    assert!(!encoded.contains("fixture-password"));
    assert!(!encoded.contains("X-Plex-Token"));
    let original = reopened.state.sources.active.navidrome().unwrap().clone();
    let mut changed = original.clone();
    changed.source.name = "New display label".into();
    assert!(changed
        .cache_store()
        .unwrap()
        .ticket("catalog")
        .read::<Value>(Duration::ZERO)
        .unwrap()
        .is_some());
    for field in 0..4 {
        let mut changed = original.clone();
        match field {
            0 => changed.source.id = "another-account".into(),
            1 => changed.source.username = "another-user".into(),
            2 => changed.source.url.push_str("/different-server"),
            _ => changed.client.folder = Some("another-library".into()),
        }
        assert!(changed
            .cache_store()
            .unwrap()
            .ticket("catalog")
            .read::<Value>(Duration::ZERO)
            .unwrap()
            .is_none());
    }
    reopened
        .state
        .cache_mgmt
        .category_timestamps
        .insert(textamp::app::state::RefreshCategory::Artists, 0);
    reopened
        .send(SystemAction::CheckStaleness(
            textamp::app::state::RefreshCategory::Artists,
        ))
        .await;
    assert!(
        reopened.state.sources.listing.is_none(),
        "navigation does not loop failed refreshes"
    );
    task.abort();
}

#[tokio::test]
async fn album_fetch_refresh_and_enqueue_work_without_a_catalog_entry() {
    let (url, _, task) = server(|m, p| (200, envelope(fixture(m, p)))).await;
    let mut app = App::new(session(url));
    let key = app.state.sources.active.navidrome().unwrap().key("album");
    app.send(MillerAction::LoadAlbumTracksForMiller {
        album_key: key.clone(),
        replace_child: false,
    })
    .await;
    while app.state.sources.nav_tasks.contains_key("album-browse") {
        app.receive().await;
    }
    let col = app.state.artist_nav.columns.last().unwrap();
    assert_eq!(
        col.tracks
            .iter()
            .map(|t| t.title.as_str())
            .collect::<Vec<_>>(),
        ["Song first", "Song second", "Song disc2"]
    );
    app.state
        .artist_nav
        .columns
        .last_mut()
        .unwrap()
        .tracks
        .clear();
    app.send(MillerAction::RefreshAlbumTracks {
        album_key: key.clone(),
    })
    .await;
    while app.state.sources.nav_tasks.contains_key("album-browse") {
        app.receive().await;
    }
    assert_eq!(app.state.artist_nav.columns.last().unwrap().tracks.len(), 3);
    app.send(QueueAction::EnqueueAlbum {
        rating_key: key,
        title: "Album".into(),
    })
    .await;
    while app.state.sources.nav_tasks.contains_key("queue-write") {
        app.receive().await;
    }
    assert_eq!(app.state.queue.tracks.len(), 3);
    assert_eq!(app.state.queue.tracks[2].title, "Song disc2");
    let artist_key = app.state.sources.active.navidrome().unwrap().key("artist");
    app.send(QueueAction::EnqueueArtistTracks {
        artist_key,
        artist_name: "Artist".into(),
    })
    .await;
    while app.state.sources.nav_tasks.contains_key("queue-write") {
        app.receive().await;
    }
    assert_eq!(app.state.queue.tracks.len(), 6);
    task.abort();
}

#[tokio::test]
async fn sonic_extension_uses_native_similarity_and_validates_path_endpoints() {
    let invalid = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let check = invalid.clone();
    let (url, requests, task) = server(move |m, p| {
        let response = match m {
            "getOpenSubsonicExtensions" => json!({"openSubsonicExtensions":[{"name":"sonicSimilarity"}]}),
            "getSonicSimilarTracks" => {
                json!({"sonicMatch":[{"entry":song("second",2,1),"similarity":0.9}]})
            }
            "findSonicPath" => {
                let end = if check.load(std::sync::atomic::Ordering::Relaxed) {
                    "wrong"
                } else {
                    "second"
                };
                json!({"sonicMatch":[{"entry":song("first",1,1)},{"entry":song("disc2",1,2)},{"entry":song(end,2,1)}]})
            }
            _ => fixture(m, p),
        };
        (200, envelope(response))
    })
    .await;
    let mut s = session(url);
    s.extensions.insert("sonicSimilarity".into());
    let mut app = App::new(s);
    app.catalog().await;
    app.send(DataAction::LoadSimilarTracks {
        rating_key: app.state.library.all_tracks[0].rating_key.clone(),
        title: "First".into(),
    })
    .await;
    while app.state.similar.loading {
        app.receive().await;
    }
    assert_eq!(app.state.similar.tracks[0].title, "Song second");
    app.state.adventure.start_track = Some(app.state.library.all_tracks[0].clone());
    app.state.adventure.end_track = Some(app.state.library.all_tracks[1].clone());
    app.send(SettingsAction::SetAdventureLength(5)).await;
    while app.state.adventure.generating {
        app.receive().await;
    }
    assert_eq!(app.state.queue.tracks.len(), 3);
    assert_eq!(app.state.queue.tracks.last().unwrap().title, "Song second");
    app.state.adventure.start_track = Some(app.state.library.all_tracks[0].clone());
    app.state.adventure.end_track = Some(app.state.library.all_tracks[1].clone());
    invalid.store(true, std::sync::atomic::Ordering::Relaxed);
    app.send(SettingsAction::SetAdventureLength(5)).await;
    while app.state.adventure.generating {
        app.receive().await;
    }
    assert!(app
        .state
        .notifications
        .last_error
        .as_ref()
        .is_some_and(|e| e.contains("invalid sonic path")));
    assert!(!requests
        .lock()
        .await
        .iter()
        .any(|(m, _)| m == "getSimilarSongs2"));
    task.abort();
}

#[tokio::test]
async fn stale_completions_cannot_mutate_new_library_or_finish_new_request() {
    let mut app = App::new(session("http://127.0.0.1:1".into()));
    let old = app.state.library_generation;
    app.state.advance_library_generation();
    app.send(NavAction::Completed {
        generation: old,
        request_id: 0,
        slot: "similar",
        result: Ok(vec![SystemAction::SetStatus("stale".into()).into()]),
    })
    .await;
    assert_ne!(
        app.state.notifications.status_message.as_deref(),
        Some("stale")
    );
    app.send(NavAction::Completed {
        generation: app.state.library_generation,
        request_id: 123,
        slot: "similar",
        result: Ok(vec![SystemAction::SetStatus("stale".into()).into()]),
    })
    .await;
    assert_ne!(
        app.state.notifications.status_message.as_deref(),
        Some("stale")
    );
}

#[tokio::test]
async fn provider_biography_and_recommendations_are_scoped() {
    let (url, _, task) = server(|m, p| (200, envelope(fixture(m, p)))).await;
    let mut app = App::new(session(url));
    app.catalog().await;
    let session = app.state.sources.active.navidrome().unwrap().clone();
    app.send(SearchAction::ShowArtistBio {
        artist_key: session.key("artist"),
        artist_name: "Artist".into(),
    })
    .await;
    while app.state.popups.artist_bio.as_ref().unwrap().loading {
        app.receive().await;
    }
    assert_eq!(
        app.state.popups.artist_bio.as_ref().unwrap().document.text,
        "An artist biography."
    );
    app.send(DataAction::LoadSimilarTracks {
        rating_key: session.key("second"),
        title: "Second".into(),
    })
    .await;
    while app.state.similar.loading {
        app.receive().await;
    }
    assert_eq!(app.state.similar.tracks.len(), 1);
    assert_eq!(app.state.similar.tracks[0].rating_key, session.key("first"));
    task.abort();
}

#[tokio::test]
async fn writes_use_post_and_no_audio_device_is_a_recoverable_failure() {
    let (url, requests, task) = server(|m, p| (200, envelope(fixture(m, p)))).await;
    let mut app = App::new(session(url));
    app.catalog().await;
    app.state.queue.tracks = app.state.library.all_tracks.clone();
    app.state.queue.index = Some(0);
    app.send(QueueAction::SaveQueueAsPlaylist("A playlist & name".into()))
        .await;
    while app.state.sources.nav_tasks.contains_key("playlist-write") {
        app.receive().await;
    }
    let seen = requests.lock().await;
    assert!(seen.iter().any(|(m, p)| m == "createPlaylist"
        && p.get("_verb").is_some_and(|v| v == "POST")
        && p.get("name").is_some_and(|v| v == "A playlist & name")));
    drop(seen);
    app.send(PlaybackAction::TogglePlayPause).await;
    assert_eq!(app.state.playback.status, PlayStatus::Stopped);
    assert!(app.state.notifications.last_error.is_some());
    task.abort();
}

#[tokio::test]
#[ignore = "Requires the isolated Navidrome fixture server on localhost:14533; mutates only that fixture"]
async fn live_navidrome_catalog_and_playlist_round_trip() {
    let mut s = session("http://127.0.0.1:14533".into());
    s.client.folder = None;
    let folders = s.client.folders().await.unwrap();
    assert!(!folders.is_empty());
    let songs = s.client.songs("").await.unwrap();
    assert_eq!(
        songs.len(),
        2,
        "Only run against the dedicated two-song fixture"
    );
    assert!(songs
        .iter()
        .all(|s| s.artist.as_deref() == Some("Fixture Artist")));
    let mut app = App::new(s.clone());
    app.catalog().await;
    assert_eq!(app.state.library.all_tracks.len(), 2);
    let id = songs[0].id.clone();
    let name = format!("Textamp test {}", uuid::Uuid::new_v4());
    let response = s
        .client
        .call(
            "createPlaylist",
            &[("name", name.clone()), ("songId", id.clone())],
        )
        .await
        .unwrap();
    let playlist_id = response["playlist"]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        s.client.playlist(&playlist_id).await.unwrap().entry[0].id,
        id
    );
    s.client
        .call(
            "updatePlaylist",
            &[
                ("playlistId", playlist_id.clone()),
                ("name", "Renamed".into()),
            ],
        )
        .await
        .unwrap();
    assert_eq!(
        s.client.playlist(&playlist_id).await.unwrap().name,
        "Renamed"
    );
    s.client.call("star", &[("id", id.clone())]).await.unwrap();
    assert!(s.client.song(&id).await.unwrap().starred.is_some());
    s.client
        .call("setRating", &[("id", id.clone()), ("rating", "4".into())])
        .await
        .unwrap();
    assert_eq!(s.client.song(&id).await.unwrap().user_rating, Some(4));
    s.client
        .call(
            "scrobble",
            &[("id", id.clone()), ("submission", "true".into())],
        )
        .await
        .unwrap();
    s.client
        .call(
            "savePlayQueue",
            &[
                ("id", id.clone()),
                ("current", id),
                ("position", "2000".into()),
            ],
        )
        .await
        .unwrap();
    let queue = s.client.call("getPlayQueue", &[]).await.unwrap();
    assert_eq!(queue["playQueue"]["position"], json!(2000));
    s.client
        .call("deletePlaylist", &[("id", playlist_id)])
        .await
        .unwrap();
    assert!(!s
        .client
        .playlists()
        .await
        .unwrap()
        .iter()
        .any(|p| p.name == name || p.name == "Renamed"));
}

#[tokio::test]
#[ignore = "Requires native audio and the dedicated silent Navidrome fixture on localhost:14533"]
async fn live_native_navidrome_stream_seek_and_visualizers() {
    let mut app = App::new(session("http://127.0.0.1:14533".into()));
    app.audio = AudioPlayer::new().unwrap();
    app.audio.set_volume(0.0);
    app.catalog().await;
    assert_eq!(app.state.library.all_tracks.len(), 2);
    let tracks = app.state.library.all_tracks.clone();
    assert!(tracks.iter().all(|t| t.artist_name() == "Fixture Artist"));
    app.send(QueueAction::PlayTracksNow(tracks)).await;
    while app.state.playback.status == PlayStatus::Buffering {
        app.receive().await;
    }
    assert_eq!(app.state.playback.status, PlayStatus::Playing);
    app.send(PlaybackAction::TogglePlayPause).await;
    app.send(PlaybackAction::Seek(8000)).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if app
                .audio
                .position()
                .is_some_and(|p| p >= Duration::from_secs(8))
            {
                break;
            }
            app.receive().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(app.state.playback.status, PlayStatus::Paused);
    assert!(app.audio.is_paused());
    app.send(SystemAction::LoadWaveform).await;
    while app.state.waveform.generating || app.state.spectrogram.generating {
        app.receive().await;
    }
    assert!(
        app.state.waveform.data.is_some(),
        "{:?}",
        app.state.waveform.error
    );
    assert!(
        app.state.spectrogram.data.is_some(),
        "{:?}",
        app.state.spectrogram.error
    );
    use textamp::app::sources::navidrome::commands::Command;
    app.send(NavAction::Command(Command::SaveQueue)).await;
    while app.state.sources.nav_tasks.contains_key("queue-write") {
        app.receive().await;
    }
    app.send(PlaybackAction::Stop).await;
    app.state.queue.tracks.clear();
    app.send(NavAction::Command(Command::RestoreQueue)).await;
    while app.state.sources.nav_tasks.contains_key("queue-read")
        || app.state.playback.status == PlayStatus::Buffering
    {
        app.receive().await;
    }
    assert_eq!(app.state.queue.tracks.len(), 2);
    tokio::time::timeout(Duration::from_secs(10), async {
        while app
            .audio
            .position()
            .is_none_or(|p| p < Duration::from_secs(8))
        {
            app.receive().await;
        }
    })
    .await
    .unwrap();
    app.send(PlaybackAction::Stop).await;
    assert_eq!(app.state.playback.status, PlayStatus::Stopped);
    // The UI submits Stop immediately; the audio worker applies it asynchronously.
    tokio::time::timeout(Duration::from_secs(2), async {
        while app.audio.is_playing() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn failed_background_similarity_waits_for_refresh_instead_of_retrying_each_tick() {
    let (url, requests, task) = server(|m, p| {
        if m == "getSimilarSongs2" {
            (503, "offline".into())
        } else {
            (200, envelope(fixture(m, p)))
        }
    })
    .await;
    let mut app = App::new(session(url));
    app.catalog().await;
    let key = app.state.library.all_tracks[0].rating_key.clone();
    app.send(DataAction::LoadTrackPaneSimilar {
        rating_key: key.clone(),
    })
    .await;
    app.receive().await;
    assert!(app.state.track_pane_similar.get(&key).unwrap().is_err());
    for _ in 0..10 {
        app.send(DataAction::LoadTrackPaneSimilar {
            rating_key: key.clone(),
        })
        .await;
    }
    assert_eq!(
        requests
            .lock()
            .await
            .iter()
            .filter(|(m, _)| m == "getSimilarSongs2")
            .count(),
        1
    );
    app.catalog().await;
    assert!(!app.state.track_pane_similar.contains_key(&key));
    task.abort();
}

#[tokio::test]
async fn inactive_library_rescan_populates_complete_catalog_without_playback_or_server_writes() {
    use textamp::app::sources::cache;
    let (url, requests, server_task) =
        server(|method, params| (200, envelope(fixture(method, params)))).await;
    let mut session = session(url);
    session.source.id = uuid::Uuid::new_v4().to_string();
    let source = session.source.clone();
    textamp::library::credentials::save_source("navidrome", &source.id, "fixture-password".into())
        .unwrap();
    let choice = LibraryChoice::Navidrome {
        source,
        folder: Some("1".into()),
        name: "Music".into(),
    };
    let store = choice.cache_store().unwrap();
    let mut app = App::new(session.clone());
    app.state.sources.active = ActiveSource::Folder("other-playing-library".into());
    app.state.playback.position_ms = 65432;
    app.state.playback.status = PlayStatus::Paused;
    cache::start(choice.clone(), true, &mut app.state, &app.tx);
    while cache::running(&choice, &app.state) {
        app.receive().await;
    }
    assert_eq!(cache::status(&choice, &app.state), "Cache up to date");
    let catalog = store
        .ticket("catalog")
        .read::<Value>(textamp::library::cache::REFRESH_INTERVAL)
        .unwrap()
        .unwrap()
        .value;
    assert_eq!(catalog["artists"].as_array().unwrap().len(), 1);
    assert_eq!(catalog["albums"].as_array().unwrap().len(), 1);
    assert_eq!(catalog["tracks"].as_array().unwrap().len(), 3);
    assert_eq!(catalog["playlists"].as_array().unwrap().len(), 1);
    assert_eq!(
        app.state.sources.active.folder().map(String::as_str),
        Some("other-playing-library")
    );
    assert_eq!(app.state.playback.position_ms, 65432);
    assert_eq!(app.state.playback.status, PlayStatus::Paused);
    assert!(requests.lock().await.iter().all(|(method, _)| [
        "ping",
        "getMusicFolders",
        "getOpenSubsonicExtensions",
        "getArtists",
        "getAlbumList2",
        "search3",
        "getPlaylists"
    ]
    .contains(&method.as_str())));
    // Reopening from the saved full catalog needs no server.
    server_task.abort();
    app.state.sources.active = ActiveSource::Navidrome(Box::new(session));
    textamp::app::sources::navidrome::open(&mut app.state, &app.tx);
    while app.state.sources.listing.is_some() {
        app.receive().await;
    }
    assert_eq!(app.state.library.all_tracks.len(), 3);
    assert!(app.state.notifications.last_error.is_none());
}

#[tokio::test]
async fn sole_folder_startup_migrates_aggregate_cache_without_network() {
    let mut session = session("http://127.0.0.1:1".into());
    session.source.id = uuid::Uuid::new_v4().to_string();
    session.source.libraries = vec![textamp::navidrome::MusicFolder {
        id: "1".into(),
        name: "Music".into(),
    }];
    let old = textamp::library::cache::Store::new((
        "navidrome",
        &session.source.id,
        session.source.url.trim_end_matches('/'),
        &session.source.username,
        None::<String>,
    ))
    .unwrap();
    old.ticket("catalog").write(&json!({"artists":[], "albums":[], "tracks":[], "playlists":[], "libraries":[{"id":"1","name":"Music"}], "extensions":[]})).unwrap();
    let current = session.cache_store().unwrap();
    assert!(!old.same_scope(&current));
    let mut app = App::new(session);
    textamp::app::sources::navidrome::open(&mut app.state, &app.tx);
    while app.state.sources.listing.is_some() {
        app.receive().await;
    }
    assert!(app.state.notifications.last_error.is_none());
    assert!(current
        .ticket("catalog")
        .read::<Value>(textamp::library::cache::REFRESH_INTERVAL)
        .unwrap()
        .is_some());
}
