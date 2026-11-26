use serde_json::{Value, json};
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
use tokio::sync::{Mutex, mpsc};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn first_import_allows_album_browsing_before_track_index_finishes() {
    let (release, waiting) = std::sync::mpsc::channel();
    let waiting = std::sync::Mutex::new(waiting);
    let (url, _, task) = server(move |method, params| {
        if method == "search3" && params.get("query").is_none_or(String::is_empty) {
            waiting
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
        (200, envelope(fixture(method, params)))
    })
    .await;
    let session = session(url);
    let mut app = App::new(session.clone());
    textamp::app::sources::navidrome::open(&mut app.state, &app.tx);
    tokio::time::timeout(Duration::from_secs(2), async {
        while app.state.library.artists.is_empty() {
            app.receive().await;
        }
    })
    .await
    .expect("browse metadata must not wait for the track index");
    assert!(app.state.library_loading);
    assert!(app.state.library.all_tracks.is_empty());
    assert!(
        session
            .cache_store()
            .unwrap()
            .ticket("catalog")
            .read::<serde_json::Value>(Duration::from_secs(60))
            .unwrap()
            .is_none()
    );
    app.send(MillerAction::LoadAlbumTracksForMiller {
        album_key: session.key("album"),
        replace_child: false,
    })
    .await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while app.state.artist_nav.columns.last().is_none_or(|column| {
            !column
                .items
                .iter()
                .any(|item| matches!(item, BrowseItem::Track { .. }))
        }) {
            app.receive().await;
        }
    })
    .await
    .expect("album browsing must remain independent of indexing");
    assert!(app.state.library_loading);
    let selected = app.state.artist_nav.columns.last().unwrap().title.clone();
    release.send(()).unwrap();
    while app.state.library_loading {
        app.receive().await;
    }
    assert_eq!(app.state.library.all_tracks.len(), 3);
    assert_eq!(app.state.artist_nav.columns.last().unwrap().title, selected);
    task.abort();
}

#[tokio::test]
async fn random_album_radio_uses_full_source_tracks_even_before_the_track_index_loads() {
    let (url, requests, task) =
        server(|method, params| (200, envelope(fixture(method, params)))).await;
    let session = session(url);
    let mut app = App::new(session.clone());
    app.state.library.albums = std::sync::Arc::new(vec![textamp::library::models::Album {
        rating_key: session.key("album"),
        title: "Album".into(),
        ..Default::default()
    }]);
    assert!(app.state.library.all_tracks.is_empty());
    app.send(RadioAction::StartStation(
        textamp::app::state::ActiveStation {
            source: textamp::library::models::RadioSource::Station("nav-radio/randomAlbum".into()),
            title: "Play Random Albums".into(),
        },
    ))
    .await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while app.state.station_starting.is_some() {
            app.receive().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        app.state.playback_mode,
        textamp::app::state::PlaybackMode::Radio
    );
    assert_eq!(app.state.radio.tracks.len(), 3);
    assert!(app.state.radio.tracks.iter().all(|track|matches!(&track.origin,textamp::library::track::TrackOrigin::Navidrome{source_id,song_id} if source_id==&session.source.id && !song_id.is_empty())));
    assert!(
        requests
            .lock()
            .await
            .iter()
            .any(|(method, _)| method == "getAlbum")
    );
    task.abort();
}

#[test]
fn opensubsonic_artist_identity_survives_mapping_and_older_caches_remain_readable() {
    let id = "550e8400-e29b-41d4-a716-446655440000";
    let artist: textamp::navidrome::Artist = serde_json::from_value(json!({
        "id":"artist", "name":"Artist", "musicBrainzId":id
    }))
    .unwrap();
    let mapped = session("http://example.test".into()).artist(artist);
    assert_eq!(mapped.music_brainz_id.as_deref(), Some(id));
    let cached = serde_json::to_value(&mapped).unwrap();
    assert_eq!(cached["musicBrainzId"], id);
    let restored: textamp::library::catalog::Artist = serde_json::from_value(cached).unwrap();
    assert_eq!(restored.music_brainz_id.as_deref(), Some(id));
    let legacy: textamp::library::catalog::Artist =
        serde_json::from_value(json!({"ratingKey":"artist", "title":"Artist"})).unwrap();
    assert!(legacy.music_brainz_id.is_none());
    let legacy: textamp::navidrome::Artist =
        serde_json::from_value(json!({"id":"artist", "name":"Artist"})).unwrap();
    assert!(legacy.music_brainz_id.is_none());
}

#[tokio::test]
async fn compilations_browse_from_server_and_offline_cache_and_reset_on_refresh() {
    let (url, requests, task) = server(|method, params| {
        let albums = json!([
            {"id":"solo","name":"Solo","artist":"A","artistId":"a","songCount":1},
            {"id":"comp","name":"Compilation","artist":"A","artistId":"a","songCount":3,"isCompilation":true},
            {"id":"hits","name":"Greatest Hits","artist":"C","artistId":"va","songCount":2,"isCompilation":true},
            {"id":"excerpt","name":"Mutant Pop: 78/79","artist":"Various Artists","artistId":"va","songCount":1}
        ]);
        let songs = json!([
            {"id":"solo1","title":"Solo","albumId":"solo","artist":"A","artistId":"a","albumArtist":"A","track":1},
            {"id":"comp1","title":"A song","albumId":"comp","artist":"A","artistId":"a","albumArtist":"A","track":1},
            {"id":"comp2","title":"B song","albumId":"comp","artist":"B","artistId":"b","albumArtist":"A","track":2},
            {"id":"comp3","title":"D song","albumId":"comp","artist":"D","artistId":"d","albumArtist":"A","track":3},
            {"id":"hits1","title":"C one","albumId":"hits","artist":"C","artistId":"va","albumArtist":"C","track":1},
            {"id":"hits2","title":"C two","albumId":"hits","artist":"C","artistId":"va","albumArtist":"C","track":2},
            {"id":"excerpt1","title":"All Time Low","albumId":"excerpt","artist":"2.3","artistId":"two-three","albumArtist":"Various Artists","track":1}
        ]);
        let payload = match method {
            "getArtists" => json!({"artists":{"index":[{"name":"A","artist":[
                {"id":"a","name":"A"},{"id":"b","name":"B"},{"id":"va","name":"Various Artists"},
                {"id":"two-three","name":"2.3"}
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
    assert!(
        root.iter()
            .any(|item| matches!(item, BrowseItem::Compilations))
    );
    assert!(root.iter().any(|item| item.key() == session.key("a")));
    assert!(root.iter().any(|item| item.key() == "track_artist:c"));
    assert!(
        !root
            .iter()
            .any(|item| item.key() == session.key("b") || item.key() == session.key("va"))
    );
    assert!(
        !root
            .iter()
            .any(|item| item.key() == session.key("two-three"))
    );
    assert_eq!(app.state.library.compilations.albums.len(), 2);
    assert_eq!(
        app.state.library.compilations.artist_map[&session.key("two-three")],
        [session.key("excerpt")]
    );
    assert!(
        app.state
            .library
            .artists
            .iter()
            .any(|artist| artist.rating_key == session.key("two-three"))
    );

    app.send(MillerAction::LoadArtistAlbumsForMiller {
        artist_key: session.key("a"),
        replace_child: true,
    })
    .await;
    let items = &app.state.artist_nav.columns.last().unwrap().items;
    assert!(matches!(items.first(), Some(BrowseItem::AllTracks { .. })));
    assert!(
        !items
            .iter()
            .any(|item| matches!(item, BrowseItem::ArtistInfo { .. }))
    );
    assert!(items.iter().any(|item| item.key() == session.key("solo")));
    assert!(!items.iter().any(|item| item.key() == session.key("comp")));
    assert!(matches!(
        items.get(1),
        Some(BrowseItem::CompilationTracks { .. })
    ));
    // A compilation-only artist, reached by search, needs no getArtist fallback.
    app.send(MillerAction::LoadArtistAlbumsForMiller {
        artist_key: session.key("b"),
        replace_child: true,
    })
    .await;
    assert!(
        app.state
            .artist_nav
            .columns
            .last()
            .unwrap()
            .items
            .iter()
            .any(|item| matches!(item, BrowseItem::CompilationTracks { .. }))
    );
    assert!(!app.state.sources.nav_tasks.contains_key("artist-browse"));
    app.send(MillerAction::LoadCompilationAlbumsForMiller {
        artist_key: session.key("b"),
        artist_name: "B".into(),
        replace_child: true,
    })
    .await;
    assert!(
        app.state
            .artist_nav
            .columns
            .last()
            .unwrap()
            .items
            .iter()
            .any(|item| item.key() == session.key("comp"))
    );
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
    assert!(
        app.state
            .search
            .results
            .as_ref()
            .unwrap()
            .artists
            .iter()
            .any(|a| a.rating_key == "track_artist:d")
    );
    assert!(
        !app.state.artist_nav.columns[0]
            .items
            .iter()
            .any(|item| item.key() == "track_artist:d")
    );
    app.send(MillerAction::LoadArtistAlbumsForMiller {
        artist_key: "track_artist:d".into(),
        replace_child: true,
    })
    .await;
    assert!(
        app.state
            .artist_nav
            .columns
            .last()
            .unwrap()
            .items
            .iter()
            .any(|item| matches!(item, BrowseItem::CompilationTracks { .. }))
    );
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
        from_cache: true,
        generation: app.state.library_generation.wrapping_add(1),
        request_id: app.state.sources.list_id,
        refreshing: false,
        timestamp: 0,
        result: Ok(empty()),
    })
    .await;
    assert_eq!(app.state.library.compilations.albums.len(), 2);
    app.send(NavAction::Catalog {
        from_cache: true,
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
    assert!(
        !app.state.artist_nav.columns[0]
            .items
            .iter()
            .any(|item| matches!(item, BrowseItem::Compilations))
    );
}

#[tokio::test]
async fn compilation_performer_keeps_its_page_and_cached_portrait_but_is_excluded_from_recommendations()
 {
    let performer =
        json!({"id":"credit-id","name":"Guest","albumCount":0,"coverArt":"cover-guest"});
    let (url, requests, server) = server(move |method, params| {
        let response = match method {
            "search3" => {
                assert_eq!(params["musicFolderId"], "1");
                assert_eq!(params["query"], "Guest");
                json!({"searchResult3":{"artist":[performer]}})
            }
            "getArtistInfo2" if params["id"] == "credit-id" => json!({"artistInfo2":{"biography":"Performer biography."}}),
            "getArtistInfo2" => json!({"artistInfo2":{"biography":"Album artist biography.","similarArtist":[performer,{"id":"ordinary","name":"Peer"}]}}),
            "getCoverArt" => return (200, "P3\n1 1\n255\n10 80 120\n".into()),
            _ => fixture(method, params),
        };
        (200, envelope(response))
    }).await;
    let session = session(url);
    let mut app = App::new(session.clone());
    let guest = "track_artist:guest";
    let owner = session.key("owner");
    app.state.library.artists = Arc::new(
        [(&*owner, "Owner"), (guest, "Guest")]
            .map(|(key, title)| textamp::library::models::Artist {
                rating_key: key.into(),
                title: title.into(),
                ..Default::default()
            })
            .into(),
    );
    app.state
        .library
        .compilations
        .artist_keys
        .insert(guest.into());
    app.state
        .library
        .compilations
        .artist_map
        .insert(guest.into(), vec![session.key("comp")]);
    app.state.library.albums = Arc::new(vec![textamp::library::models::Album {
        rating_key: session.key("album"),
        parent_rating_key: Some(owner.clone()),
        ..Default::default()
    }]);
    let scope = app.state.active_library.clone().unwrap();
    let portrait = session
        .artwork(&session.key("cover-guest"), &scope)
        .await
        .unwrap();
    for (key, name) in [(&*owner, "Owner"), (guest, "Guest")] {
        app.send(BrowseAction::OpenInLibrary {
            artist_key: if key == guest {
                session.key("credit-id")
            } else {
                key.into()
            },
            artist_name: name.into(),
            album_key: None,
            album_title: None,
        })
        .await;
        while app
            .state
            .artist_nav
            .columns
            .last()
            .and_then(|c| c.artist_header.as_ref())
            .is_none_or(|h| h.task.is_some())
            || (key == guest && !app.state.artwork.grid_cache.contains_key(guest))
        {
            app.receive().await;
        }
        let column = app.state.artist_nav.columns.last().unwrap();
        let header = column.artist_header.as_ref().unwrap();
        assert_eq!(header.artist_key, key);
        if key == guest {
            assert_eq!(header.excerpt(), "Performer biography.");
            assert_eq!(app.state.artwork.grid_cache[guest], portrait);
            #[cfg(feature = "mobile")]
            {
                let art = textamp::mobile::character_art(
                    &app.state.artwork.grid_cache[guest],
                    "halfblocks",
                )
                .unwrap();
                assert!(
                    art["cells"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|cell| cell[1] == json!([10, 80, 120]))
                );
            }
            assert!(
                column
                    .items
                    .iter()
                    .any(|i| matches!(i, BrowseItem::CompilationTracks { .. }))
            );
        } else {
            assert_eq!(
                header
                    .document
                    .related_artists
                    .iter()
                    .map(|a| a.title.as_str())
                    .collect::<Vec<_>>(),
                ["Peer"]
            );
        }
    }
    assert_eq!(
        requests
            .lock()
            .await
            .iter()
            .filter(|(method, _)| method == "getCoverArt")
            .count(),
        1
    );
    server.abort();
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
    app.state.library.selected_artist_albums = app.state.library.albums.as_ref().clone();
    app.state.list_state.right_albums_index = 1; // Row zero is All Tracks.
    app.send(DataAction::LoadSelectedAlbumTracks).await;
    assert_eq!(app.state.artist_nav.columns.last().unwrap().tracks.len(), 3);
    assert!(
        app.state
            .artist_nav
            .columns
            .last()
            .unwrap()
            .tracks
            .iter()
            .all(|t| t.parent_rating_key.as_deref() == Some(session.key("album").as_str()))
    );
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
        app.state.popups.input_dialog().unwrap().title,
        "Save Station as Playlist"
    );
    app.send(QueueAction::SaveQueueAsPlaylist("Station fixture".into()))
        .await;
    while app.state.sources.nav_tasks.contains_key("playlist-write") {
        app.receive().await;
    }
    assert!(
        app.state
            .library
            .playlists
            .iter()
            .any(|playlist| playlist.title == "Station fixture")
    );
    assert!(
        app.state.playlist_nav.columns[0]
            .items
            .iter()
            .any(|item| item.title() == "Station fixture")
    );
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
        assert!(
            app.state
                .notifications
                .last_error
                .as_deref()
                .is_none_or(|error| !error.contains("has no Navidrome handler"))
        );
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
    let folder = app.state.folder_state.as_ref().unwrap().focused().unwrap();
    assert_eq!(folder.tracks.len(), folder.items.len());
    for item in &folder.items {
        let track = folder
            .tracks
            .get(item.rating_key.as_ref().unwrap())
            .unwrap();
        assert_eq!(track.title, item.title);
    }
    app.send(FolderAction::PlayFolderTracks).await;
    assert_eq!(app.state.queue.tracks.len(), 2);
    assert_eq!(app.state.queue.tracks[0].rating_key, session.key("second"));

    // A refresh can remove catalog entries while a folder is still open.
    // Enqueue must report that absence, not try a Plex folder endpoint.
    app.state.set_view(View::Browse);
    app.state.set_playback_mode(PlaybackMode::Queue);
    std::sync::Arc::make_mut(&mut app.state.library.all_tracks).clear();
    app.state.category_column_focused = false;
    for action in [
        QueueAction::EnqueueSelection,
        QueueAction::EnqueueSelectionNext,
    ] {
        app.state.clear_error();
        app.send(action).await;
        assert!(
            app.state
                .notifications
                .last_error
                .as_deref()
                .is_some_and(|error| error.contains("Refresh Library"))
        );
        assert_eq!(app.state.queue.tracks.len(), 2);
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    server_task.abort();
}

#[tokio::test]
async fn remix_completion_applies_to_the_whole_unchanged_queue_after_playback_advances() {
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
    assert!(
        app.state
            .queue
            .tracks
            .iter()
            .all(|track| track.rating_key == session.key("replacement"))
    );
    assert_eq!(app.state.playback.status, PlayStatus::Stopped);
}

#[tokio::test]
async fn opposite_day_refreshes_artwork_without_resuming_paused_playback() {
    use textamp::app::event::{ArtworkEvent, PlaybackEvent};
    use textamp::app::state::{ArtworkFetch, PlaybackMode};

    let (url, requests, server) = server(|method, params| {
        assert_eq!(
            method, "getCoverArt",
            "Remixing and loading artwork must not stream audio"
        );
        assert_eq!(params["id"], "replacement-cover");
        (200, "replacement cover".into())
    })
    .await;
    let session = session(url);
    let mut app = App::new(session.clone());
    let track = |id: &str, cover: &str, duration: u32| {
        let mut data = song(id, 1, 1);
        data["coverArt"] = json!(cover);
        data["duration"] = json!(duration);
        session.track(serde_json::from_value(data).unwrap())
    };
    app.state.set_playback_mode(PlaybackMode::Queue);
    let original = track("original", "old-cover", 60);
    let replacement = track("replacement", "replacement-cover", 140);
    app.state.queue.tracks = vec![original.clone()];
    app.state.queue.index = Some(0);
    app.state.playback.status = PlayStatus::Paused;
    app.state.playback.position_ms = 25_000;
    app.state.playback.duration_ms = original.duration_ms();
    app.state.consecutive_playback_errors = 4;
    app.state.waveform.track_key = Some(original.rating_key.clone());
    app.state.artwork.current_thumb = original.thumb.clone();
    app.state.artwork.current_data = Some(b"old cover".to_vec());
    let old_id = app.state.playback.request_id;
    let old_thumb = original.thumb.clone().unwrap();
    app.state.artwork.fetch = ArtworkFetch::Loading {
        thumb: old_thumb.clone(),
    };

    app.send(NavAction::QueueEdit {
        expected: vec![original.rating_key],
        action: Box::new(
            QueueAction::RemixOppositeDayReady(AsyncBatchOutcome::new(
                vec![(0, replacement.clone())],
                1,
            ))
            .into(),
        ),
    })
    .await;
    assert_eq!(
        app.state.current_track().unwrap().rating_key,
        replacement.rating_key
    );
    assert_eq!(app.state.playback.status, PlayStatus::Stopped);
    assert_eq!(app.state.playback.position_ms, 0);
    assert_eq!(app.state.playback.duration_ms, replacement.duration_ms());
    assert_eq!(app.state.consecutive_playback_errors, 0);
    assert!(app.state.waveform.track_key.is_none());
    assert!(app.state.current_artwork_bytes().is_none());
    assert_eq!(
        app.state.artwork.fetch.pending_thumb(),
        replacement.thumb.as_deref()
    );

    for stale in [
        Event::Artwork(ArtworkEvent::ArtworkLoaded {
            generation: app.state.artwork.grid_generation,
            thumb_path: old_thumb,
            data: b"late old cover".to_vec(),
        }),
        PlaybackEvent::RetryAfterDelay {
            playback_id: old_id,
        }
        .into(),
        PlaybackEvent::PlaybackError {
            playback_id: Some(old_id),
            message: "late decoder failure".into(),
        }
        .into(),
    ] {
        assert!(handle_core_event(stale, &mut app.state, &app.tx).is_empty());
    }
    assert_eq!(app.state.consecutive_playback_errors, 0);
    assert!(app.state.current_artwork_bytes().is_none());
    while app.state.artwork.fetch.is_loading() {
        app.receive().await;
    }
    assert_eq!(
        app.state.current_artwork_bytes(),
        Some(b"replacement cover".as_slice())
    );
    assert_eq!(app.state.playback.status, PlayStatus::Stopped);
    assert_eq!(requests.lock().await.len(), 1);
    server.abort();
}

#[tokio::test]
async fn twofer_handles_single_tracks_and_does_not_sample_already_queued_tracks() {
    use textamp::app::state::PlaybackMode;
    let session = session("http://127.0.0.1:1".into()); // Twofer must not need a server.
    for queued in [1, 30] {
        let mut app = App::new(session.clone());
        // Explicit Twofer overrides the general same-artist preference.
        app.state.remix_allow_same_artist = false;
        app.state.set_playback_mode(PlaybackMode::Queue);
        app.state.library.all_tracks = std::sync::Arc::new(
            (0..queued + 1)
                .map(|i| {
                    let mut item = song(&i.to_string(), 1, 1);
                    if i == queued {
                        item["albumId"] = json!("another-album");
                        item["album"] = json!("Another Album");
                    }
                    session.track(serde_json::from_value(item).unwrap())
                })
                .collect(),
        );
        app.state.queue.tracks = app.state.library.all_tracks[..queued].to_vec();
        app.state.queue.index = Some(queued - 1);
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
        assert_eq!(
            app.state.current_track().unwrap().rating_key,
            session.key(&(queued - 1).to_string())
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
async fn gemini_obeys_album_and_artist_policy_and_adds_actual_recommendations() {
    use textamp::app::state::PlaybackMode;
    for allow_artist in [false, true] {
        let make = |id: &str, artist: &str, album: &str| {
            let mut row = song(id, 1, 1);
            row["artistId"] = json!(artist);
            row["artist"] = json!(artist);
            row["albumArtist"] = json!(artist);
            row["albumId"] = json!(album);
            row["album"] = json!(album);
            row
        };
        let rows = vec![
            make("seed", "A", "queued"),
            make("same-album", "B", "queued"),
            make("same-artist", "A", "different"),
            make("good", "C", "third"),
        ];
        let response_rows = rows.clone();
        let (url, requests, task) = server(move |method, params| {
            let body = if method == "getSonicSimilarTracks" {
                json!({"sonicMatch": response_rows[1..].iter().map(|row| json!({"entry":row,"similarity":0.9})).collect::<Vec<_>>()})
            } else {fixture(method, params)};
            (200, envelope(body))
        }).await;
        let mut session = session(url);
        session.extensions.insert("sonicSimilarity".into());
        let mut app = App::new(session.clone());
        app.state.remix_allow_same_artist = allow_artist;
        app.state.library.all_tracks = std::sync::Arc::new(
            rows.into_iter()
                .map(|row| session.track(serde_json::from_value(row).unwrap()))
                .collect(),
        );
        app.state.set_playback_mode(PlaybackMode::Queue);
        app.state.queue.tracks = vec![app.state.library.all_tracks[0].clone()];
        app.state.queue.index = Some(0);
        app.send(QueueAction::RemixGemini).await;
        while app.state.sources.nav_tasks.contains_key("remix") {
            app.receive().await;
        }
        assert_eq!(
            app.state.queue.tracks.len(),
            2,
            "Twins must actually add a track"
        );
        assert_eq!(
            app.state.queue.tracks[1].rating_key,
            session.key(if allow_artist { "same-artist" } else { "good" })
        );
        assert_ne!(
            app.state.queue.tracks[1].parent_rating_key,
            app.state.queue.tracks[0].parent_rating_key
        );
        assert!(app.state.notifications.last_error.is_none());
        assert!(
            requests
                .lock()
                .await
                .iter()
                .any(|(method, _)| method == "getSonicSimilarTracks")
        );
        assert!(app.state.queue.undo_snapshot.is_some());
        task.abort();
    }
}

#[tokio::test]
async fn gemini_excludes_duplicate_recommendation_albums_across_seed_requests() {
    use textamp::app::state::PlaybackMode;
    let mut rows = Vec::new();
    for (id, artist, album) in [
        ("seed1", "A", "original"),
        ("seed2", "A", "original"),
        ("pick1", "B", "new-album"),
        ("duplicate-album", "C", "new-album"),
        ("pick2", "D", "another-album"),
    ] {
        let mut row = song(id, 1, 1);
        row["artistId"] = json!(artist);
        row["artist"] = json!(artist);
        row["albumArtist"] = json!(artist);
        row["albumId"] = json!(album);
        row["album"] = json!(album);
        rows.push(row);
    }
    let candidates = rows[2..].to_vec();
    let (url, _, task) = server(move |method, params| (200,envelope(if method == "getSonicSimilarTracks" {
        json!({"sonicMatch":candidates.iter().map(|row|json!({"entry":row,"similarity":0.9})).collect::<Vec<_>>()})
    } else {fixture(method,params)}))).await;
    let mut session = session(url);
    session.extensions.insert("sonicSimilarity".into());
    let mut app = App::new(session.clone());
    app.state.library.all_tracks = std::sync::Arc::new(
        rows.into_iter()
            .map(|row| session.track(serde_json::from_value(row).unwrap()))
            .collect(),
    );
    app.state.set_playback_mode(PlaybackMode::Queue);
    app.state.queue.tracks = app.state.library.all_tracks[..2].to_vec();
    app.state.queue.index = Some(0);
    app.send(QueueAction::RemixGemini).await;
    while app.state.sources.nav_tasks.contains_key("remix") {
        app.receive().await;
    }
    assert_eq!(
        app.state
            .queue
            .tracks
            .iter()
            .map(|track| track.rating_key.clone())
            .collect::<Vec<_>>(),
        ["seed1", "pick1", "seed2", "pick2"].map(|id| session.key(id))
    );
    task.abort();
}

#[tokio::test]
async fn changing_queue_contents_cancels_pending_edits_but_playback_progress_does_not() {
    for mutation in [
        QueueAction::EnqueueTracksNext(vec![textamp::library::models::Track {
            rating_key: "extra".into(),
            ..Default::default()
        }]),
        QueueAction::ReorderSelection { from: 0, to: 2 },
        QueueAction::RemoveFromQueue(0),
    ] {
        let session = session("http://127.0.0.1:1".into());
        let mut app = App::new(session.clone());
        app.state.queue.tracks = ["first", "second"]
            .map(|id| session.track(serde_json::from_value(song(id, 1, 1)).unwrap()))
            .to_vec();
        app.state.library.all_tracks = std::sync::Arc::new(app.state.queue.tracks.clone());
        app.state.queue.index = Some(0);
        app.send(QueueAction::RemixTwofer).await;
        let request_id = app.state.sources.nav_tasks["remix"].0;
        app.state.queue.index = Some(1);
        app.state.playback.position_ms = 1200;
        app.send(QueueAction::SetRepeat(
            textamp::app::state::RepeatMode::Track,
        ))
        .await;
        assert_eq!(app.state.sources.nav_tasks["remix"].0, request_id);
        app.send(mutation).await;
        assert!(!app.state.sources.nav_tasks.contains_key("remix"));
        let expected: Vec<_> = app
            .state
            .queue
            .tracks
            .iter()
            .map(|track| track.rating_key.clone())
            .collect();
        app.send(NavAction::Completed {
            generation: app.state.library_generation,
            request_id,
            slot: "remix",
            result: Ok(vec![QueueAction::ClearQueue.into()]),
        })
        .await;
        assert_eq!(
            app.state
                .queue
                .tracks
                .iter()
                .map(|track| track.rating_key.clone())
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[tokio::test]
async fn new_queue_edit_replaces_inflight_work_and_rejects_its_late_completion() {
    use textamp::app::{presentation::NpSidebarButton, queue_controls};
    let session = session("http://127.0.0.1:1".into());
    let mut app = App::new(session.clone());
    let seed = session.track(serde_json::from_value(song("first", 1, 1)).unwrap());
    app.state.library.all_tracks = std::sync::Arc::new(vec![seed.clone()]);
    app.state.queue.tracks = vec![seed];
    app.state.queue.index = Some(0);
    app.send(QueueAction::RemixTwofer).await;
    let old_id = app.state.sources.nav_tasks["remix"].0;
    assert!(
        queue_controls::definition(&app.state, NpSidebarButton::Twofer)
            .disabled_reason
            .is_none()
    );
    app.send(QueueAction::RemixTwofer).await;
    let new_id = app.state.sources.nav_tasks["remix"].0;
    assert_ne!(old_id, new_id);
    app.send(NavAction::Completed {
        generation: app.state.library_generation,
        request_id: old_id,
        slot: "remix",
        result: Ok(vec![
            QueueAction::RemixBatchReady(AsyncBatchOutcome::new(
                vec![(
                    0,
                    vec![session.track(serde_json::from_value(song("stale", 2, 2)).unwrap())],
                )],
                1,
            ))
            .into(),
        ]),
    })
    .await;
    assert_eq!(app.state.sources.nav_tasks["remix"].0, new_id);
    assert_eq!(app.state.queue.tracks.len(), 1);
    while app.state.sources.nav_tasks.contains_key("remix") {
        app.receive().await;
    }
    assert_eq!(app.state.queue.tracks.len(), 1);
}

#[tokio::test]
async fn remix_policy_change_cancels_work_and_rejects_old_completion() {
    use textamp::app::state::PlaybackMode;
    let session = session("http://127.0.0.1:1".into());
    let mut app = App::new(session.clone());
    app.state.remix_allow_same_artist = true;
    app.state.set_playback_mode(PlaybackMode::Queue);
    let seed = session.track(serde_json::from_value(song("first", 1, 1)).unwrap());
    app.state.library.all_tracks = std::sync::Arc::new(vec![seed.clone()]);
    app.state.queue.tracks = vec![seed];
    app.state.queue.index = Some(0);
    app.send(QueueAction::RemixTwofer).await;
    let request_id = app.state.sources.nav_tasks["remix"].0;
    app.send(SettingsAction::SetRemixAllowSameArtist(false))
        .await;
    assert!(!app.state.sources.nav_tasks.contains_key("remix"));
    assert!(!app.config.ui.remix_allow_same_artist);
    app.send(NavAction::Completed {
        generation: app.state.library_generation,
        request_id,
        slot: "remix",
        result: Ok(vec![
            QueueAction::RemixBatchReady(AsyncBatchOutcome::new(
                vec![(0, vec![Default::default()])],
                1,
            ))
            .into(),
        ]),
    })
    .await;
    assert_eq!(app.state.queue.tracks.len(), 1);
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
            candidate["artist"] = json!("Other Artist");
            candidate["albumArtist"] = json!("Other Artist");
            candidate["albumId"] = json!("another-album");
            candidate["album"] = json!("Another Album");
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
        app.state.remix_allow_same_artist = true;
        for track in std::sync::Arc::make_mut(&mut app.state.library.all_tracks) {
            if track.rating_key == session.key("second") {
                track.parent_rating_key = Some(session.key("another-album"));
                track.parent_title = Some("Another Album".into());
            }
        }
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
        assert!(
            app.state
                .queue
                .tracks
                .iter()
                .any(|t| t.rating_key == session.key("second"))
        );
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
    assert!(
        app.state.station_nav.columns[0]
            .stations()
            .iter()
            .any(|s| s.title == "Random Artist Mix")
    );
    assert!(
        app.state.station_nav.columns[0]
            .stations()
            .iter()
            .any(|s| s.title == "Play Random Albums")
    );
    // Isolate station requests from the catalog's background cache fill.
    while app
        .state
        .settings_state
        .cache_scans
        .values()
        .any(|scan| matches!(scan.work, textamp::app::sources::cache::Work::Warm(_)))
    {
        app.receive().await;
    }
    seen.lock().await.clear();
    // A drilled category replaces the current list; root stations remain in
    // station_nav and must still supply the shortcut's key and title.
    app.state
        .station_nav
        .push_column(textamp::app::state::StationColumn::new(
            Some("category".into()),
            "Category".into(),
            vec![],
        ));
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
        "Play Random Albums"
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
    let requests = seen.lock().await;
    assert!(
        requests
            .iter()
            .all(|(method, params)| match method.as_str() {
                "scrobble" | "getCoverArt" | "stream" => true,
                "getAlbum" => params.get("id").is_some_and(|id| id == "album"),
                _ => false,
            }),
        "{:?}",
        requests
            .iter()
            .map(|(method, _)| method)
            .collect::<Vec<_>>()
    );
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
        app.state.library.all_tracks = std::sync::Arc::new(
            ["first", "second", "disc2"]
                .iter()
                .map(|id| session.track(serde_json::from_value(song(id, 1, 1)).unwrap()))
                .collect(),
        );
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
        assert!(
            requests
                .iter()
                .all(|(method, _)| method == "getSonicSimilarTracks")
        );
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
            std::sync::Arc::make_mut(&mut app.state.library.all_tracks).push(
                textamp::library::models::Track {
                    rating_key: format!("navidrome:account-one:{artist}-{i}"),
                    grandparent_rating_key: Some(artist.into()),
                    ..Default::default()
                },
            );
        }
    }
    {
        let station = "nav-radio/randomArtist";
        app.send(RadioAction::PlayStation(station.into())).await;
        while app.state.station_starting.is_some() {
            app.receive().await;
        }
        assert_eq!(app.state.radio.tracks.len(), 25);
        assert!(
            app.state
                .radio
                .tracks
                .iter()
                .all(|t| t.grandparent_rating_key.as_deref() == Some("large"))
        );
        let catalog = app.state.library.all_tracks.clone();
        std::sync::Arc::make_mut(&mut app.state.library.all_tracks)
            .retain(|t| t.grandparent_rating_key.as_deref() == Some("small"));
        app.send(RadioAction::PlayStation(station.into())).await;
        while app.state.station_starting.is_some() {
            app.receive().await;
        }
        assert!(
            app.state
                .notifications
                .last_error
                .as_ref()
                .unwrap()
                .contains("at least 25 tracks")
        );
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
        "createPlaylist" => {
            json!({"playlist":{"id":"saved-playlist","name":_params.get("name").cloned().unwrap_or_default(),"songCount":1}})
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
                if method == "getLyricsBySongId" {
                    params.insert("_url_query".into(), url.query().unwrap_or_default().into());
                    params.insert(
                        "_content_type".into(),
                        header
                            .lines()
                            .find(|line| line.to_ascii_lowercase().starts_with("content-type:"))
                            .unwrap_or_default()
                            .to_ascii_lowercase(),
                    );
                }
                if len > 0 {
                    let form = reqwest::Url::parse(&format!(
                        "http://localhost/?{}",
                        String::from_utf8_lossy(&bytes[end..end + len])
                    ))
                    .unwrap();
                    let song_ids: Vec<_> = form
                        .query_pairs()
                        .filter(|(k, _)| k == "songId")
                        .map(|(_, v)| v.into_owned())
                        .collect();
                    let removals: Vec<_> = form
                        .query_pairs()
                        .filter(|(k, _)| k == "songIndexToRemove")
                        .map(|(_, v)| v.parse::<usize>().unwrap())
                        .collect();
                    params.insert(
                        "_removedIndices".into(),
                        serde_json::to_string(&removals).unwrap(),
                    );
                    let additions: Vec<_> = form
                        .query_pairs()
                        .filter(|(k, _)| k == "songIdToAdd")
                        .map(|(_, v)| v.into_owned())
                        .collect();
                    params.insert(
                        "_additions".into(),
                        serde_json::to_string(&additions).unwrap(),
                    );
                    params.insert("_songIds".into(), serde_json::to_string(&song_ids).unwrap());
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
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
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
    app.state.library.playlists = std::sync::Arc::new(
        ["Recently Played", "All Music", "My ❤️ mix"]
            .iter()
            .enumerate()
            .map(|(i, name)| {
                session.playlist(
                    serde_json::from_value(json!({"id":i.to_string(),"name":name})).unwrap(),
                )
            })
            .collect(),
    );
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
    assert!(
        rows[first_playlist..]
            .iter()
            .all(|r| matches!(r, CategoryRow::Playlist(_)))
    );
    assert_eq!(rows[first_playlist..].len(), 3);
    app.state.sources.active = ActiveSource::None;
    assert!(
        !app.state
            .category_rows()
            .iter()
            .any(|r| matches!(r, CategoryRow::NavidromeCollection(_)))
    );
}

#[test]
fn system_sidebar_keyboard_mouse_and_overflow_use_the_rendered_rows() {
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use ratatui::{Terminal, backend::TestBackend};
    use textamp::app::handlers::{key_input, mouse_input};
    use textamp::app::sources::navidrome::commands::CollectionKind;
    use textamp::app::state::{BrowseNavigationState, CategoryRow};
    let session = session("http://example.test".into());
    let mut app = App::new(session.clone());
    app.state.category_column_focused = true;
    app.state.library.playlists = std::sync::Arc::new(
        (0..40)
            .map(|i| {
                session.playlist(
                    serde_json::from_value(
                        json!({"id":i.to_string(),"name":format!("Saved {i:02}")}),
                    )
                    .unwrap(),
                )
            })
            .collect(),
    );
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
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
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
    use textamp::app::sources::navidrome::commands::{Collection, CollectionKind, load_collection};
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
        from_cache: true,
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
    app.state.library.all_tracks = std::sync::Arc::new(
        ["one", "two"]
            .iter()
            .map(|id| session.track(serde_json::from_value(song(id, 1, 1)).unwrap()))
            .collect(),
    );
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
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
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
    use ratatui::{Terminal, backend::TestBackend};
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
        assert!(!app.state.popups.search_active());
        for action in key_input::handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut app.state,
            &app.config,
        ) {
            app.send(action).await;
        }
        assert!(app.state.popups.search_active());
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
        assert!(app.state.popups.search_active());
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
    assert!(!app.state.popups.search_active());
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
    use ratatui::{Terminal, backend::TestBackend};
    use textamp::app::sources::{audiomuse, dialogs};
    let mut app = App::new(session("http://example.test".into()));
    app.state
        .popups
        .set_library_dialog(Some(dialogs::Dialog::AudioMuse(audiomuse::Form {
            instance: "fixture".into(),
            library_key: "library".into(),
            fields: Default::default(),
            focus: 0,
            error: None,
            task: None,
        })));
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
    assert!(app.state.popups.library_dialog().is_none());
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
    assert!(
        client
            .call("scrobble", &[("id", "test".into())])
            .await
            .is_err()
    );
    assert_eq!(requests.lock().await.len(), 3);
    task.abort();
}

#[tokio::test]
async fn catalog_pages_overlap_in_a_bounded_window_and_keep_offset_order() {
    async fn read_offset(socket: &mut tokio::net::TcpStream) -> usize {
        let mut bytes = Vec::new();
        let mut chunk = [0; 4096];
        while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = socket.read(&mut chunk).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&chunk[..n]);
        }
        let text = String::from_utf8(bytes).unwrap();
        let path = text.split_whitespace().nth(1).unwrap();
        reqwest::Url::parse(&format!("http://fixture{path}"))
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == "offset")
            .unwrap()
            .1
            .parse()
            .unwrap()
    }
    async fn reply(mut socket: tokio::net::TcpStream, offset: usize) {
        let count = if offset == 2000 { 1 } else { 500 };
        let body = envelope(json!({"albumList2":{"album":(offset..offset+count)
            .map(|id|json!({"id":id.to_string(),"name":"Album"})).collect::<Vec<_>>()}}));
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = session(format!("http://{}", listener.local_addr().unwrap())).client;
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        assert_eq!(read_offset(&mut socket).await, 0);
        reply(socket, 0).await;
        let mut pending = Vec::new();
        // Replies are withheld until the complete four-page window arrives.
        for _ in 0..4 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let offset = read_offset(&mut socket).await;
            pending.push((socket, offset));
        }
        let mut offsets: Vec<_> = pending.iter().map(|(_, offset)| *offset).collect();
        offsets.sort_unstable();
        assert_eq!(offsets, [500, 1000, 1500, 2000]);
        for (socket, offset) in pending.into_iter().rev() {
            reply(socket, offset).await;
        }
    });
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        client.albums("alphabeticalByName", &[]),
    )
    .await;
    if result.is_err() {
        server.abort();
    }
    let albums = result
        .expect("pages must overlap instead of waiting for earlier replies")
        .unwrap();
    assert_eq!(
        albums
            .iter()
            .map(|album| album.id.parse::<usize>().unwrap())
            .collect::<Vec<_>>(),
        (0..2001).collect::<Vec<_>>()
    );
    server.await.unwrap();
}

#[tokio::test]
async fn failed_catalog_page_rejects_partial_results_and_stops_pagination() {
    let (url, requests, task) = server(|method, params| {
        assert_eq!(method, "getAlbumList2");
        let offset = params["offset"].parse::<usize>().unwrap();
        if offset == 500 {
            return (503, String::new());
        }
        (
            200,
            envelope(json!({"albumList2":{"album":(offset..offset + 500)
            .map(|id| json!({"id": id.to_string(), "name": "Album"})).collect::<Vec<_>>()}})),
        )
    })
    .await;
    let result = session(url).client.albums("alphabeticalByName", &[]).await;
    let error = format!("{:#}", result.unwrap_err());
    assert!(error.contains("503"), "{error}");
    let seen = requests.lock().await;
    assert!(
        seen.len() <= 5,
        "only the initial page and one window may start"
    );
    assert!(
        seen.iter()
            .all(|(_, params)| params["offset"].parse::<usize>().unwrap() <= 2000)
    );
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
    assert!(
        app.state
            .artist_nav
            .columns
            .last()
            .unwrap()
            .items
            .iter()
            .any(|i| matches!(i,BrowseItem::Album { title,.. } if title == "Album"))
    );
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
    assert!(
        requests
            .iter()
            .all(|(_, p)| !p.contains_key("X-Plex-Token"))
    );
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
    assert!(
        app.state
            .notifications
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("503"))
    );
    assert_eq!(
        app.state.notifications.error_target,
        Some(textamp::app::state::NotificationTarget::RefreshLibrary)
    );
    fail.store(false, std::sync::atomic::Ordering::Relaxed);
    app.catalog().await;
    assert_eq!(app.state.library.artists.len(), 1);
    task.abort();
}

#[tokio::test]
async fn failed_catalog_retry_keeps_cached_tracks_and_recovers() {
    use textamp::app::state::{NotificationTarget, RefreshCategory};
    let fail = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let check = fail.clone();
    let (url, _, task) = server(move |method, params| {
        if check.load(std::sync::atomic::Ordering::Relaxed) {
            (503, "offline".into())
        } else {
            (200, envelope(fixture(method, params)))
        }
    })
    .await;
    let mut app = App::new(session(url));
    app.catalog().await;
    assert_eq!(app.state.library.all_tracks.len(), 3);
    fail.store(true, std::sync::atomic::Ordering::Relaxed);
    app.catalog().await;
    assert_eq!(app.state.library.all_tracks.len(), 3);
    assert_eq!(
        app.state.notifications.error_target,
        Some(NotificationTarget::RefreshLibrary)
    );
    assert!(
        !app.state
            .notifications
            .last_error
            .as_deref()
            .unwrap()
            .contains("F5")
    );
    fail.store(false, std::sync::atomic::Ordering::Relaxed);
    app.send(SystemAction::RefreshCategory(RefreshCategory::Artists))
        .await;
    assert!(app.state.notifications.last_error.is_none());
    while app.state.library_loading {
        app.receive().await;
    }
    assert!(app.state.notifications.error_target.is_none());
    assert_eq!(app.state.library.all_tracks.len(), 3);
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
    std::sync::Arc::make_mut(&mut reopened.state.library.playlists).clear();
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
    assert!(
        changed
            .cache_store()
            .unwrap()
            .ticket("catalog")
            .read::<Value>(Duration::ZERO)
            .unwrap()
            .is_some()
    );
    for field in 0..4 {
        let mut changed = original.clone();
        match field {
            0 => changed.source.id = "another-account".into(),
            1 => changed.source.username = "another-user".into(),
            2 => changed.source.url.push_str("/different-server"),
            _ => changed.client.folder = Some("another-library".into()),
        }
        assert!(
            changed
                .cache_store()
                .unwrap()
                .ticket("catalog")
                .read::<Value>(Duration::ZERO)
                .unwrap()
                .is_none()
        );
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
        let mut response = match m {
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
        fn distinct_artists(value: &mut Value) {
            match value {
                Value::Object(object) => {
                    if let Some(id) = object.get("id").and_then(Value::as_str).map(str::to_owned) {
                        if ["first", "second", "disc2"].contains(&id.as_str()) {
                            object.insert("artist".into(), json!(format!("Artist {id}")));
                            object.insert("artistId".into(), json!(format!("artist-{id}")));
                        }
                    }
                    for child in object.values_mut() { distinct_artists(child); }
                }
                Value::Array(items) => for child in items { distinct_artists(child); },
                _ => {}
            }
        }
        distinct_artists(&mut response);
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
    while app.state.similar.request.is_loading() {
        app.receive().await;
    }
    assert_eq!(app.state.similar.tracks[0].title, "Song second");
    use textamp::app::state::JourneyStage;
    for fail in [false, true] {
        invalid.store(fail, std::sync::atomic::Ordering::Relaxed);
        app.send(SearchAction::OpenAdventureLauncher).await;
        let launcher = app.state.popups.adventure_launcher_mut().unwrap();
        launcher.start_track = Some(app.state.library.all_tracks[0].clone());
        launcher.end_track = Some(app.state.library.all_tracks[1].clone());
        launcher.track_count_input = "5".into();
        app.send(SearchAction::AdventureLauncherGenerate).await;
        while matches!(
            app.state.popups.adventure_launcher().unwrap().stage,
            JourneyStage::Generating { .. }
        ) {
            app.receive().await;
        }
        let stage = &app.state.popups.adventure_launcher().unwrap().stage;
        if fail {
            assert!(
                matches!(stage, JourneyStage::Failed(error) if error.contains("invalid sonic path"))
            );
        } else {
            assert!(matches!(stage, JourneyStage::Ready(tracks) if tracks.len() == 3));
            assert!(app.state.queue.tracks.is_empty());
            app.send(SearchAction::AdventureLauncherPlay).await;
            assert_eq!(app.state.queue.tracks.len(), 3);
            assert_eq!(app.state.queue.tracks.last().unwrap().title, "Song second");
        }
    }
    assert!(
        !requests
            .lock()
            .await
            .iter()
            .any(|(m, _)| m == "getSimilarSongs2")
    );
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
    while app.state.popups.artist_bio().unwrap().task.is_some() {
        app.receive().await;
    }
    let text = &app.state.popups.artist_bio().unwrap().document.text;
    assert_eq!(text, "An artist biography.");
    app.send(DataAction::LoadSimilarTracks {
        rating_key: session.key("second"),
        title: "Second".into(),
    })
    .await;
    while app.state.similar.request.is_loading() {
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
    app.state.queue.tracks = app.state.library.all_tracks.as_ref().clone();
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
    assert!(
        songs
            .iter()
            .all(|s| s.artist.as_deref() == Some("Fixture Artist"))
    );
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
    assert!(
        !s.client
            .playlists()
            .await
            .unwrap()
            .iter()
            .any(|p| p.name == name || p.name == "Renamed")
    );
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
    app.send(QueueAction::PlayTracksNow(tracks.to_vec())).await;
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
    while app.state.waveform.generating() || app.state.spectrogram.generating() {
        app.receive().await;
    }
    assert!(
        app.state.waveform.data().is_some(),
        "{:?}",
        app.state.waveform.error()
    );
    assert!(
        app.state.spectrogram.data().is_some(),
        "{:?}",
        app.state.spectrogram.error()
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
    assert_eq!(cache::status(&choice, &app.state), "Library up to date");
    assert!(
        store
            .ticket("browse-catalog-v1")
            .read::<Value>(textamp::library::cache::REFRESH_INTERVAL)
            .unwrap()
            .is_some(),
        "inactive scans must also prepare compact browsing metadata"
    );
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
    assert!(requests.lock().await.iter().all(|(method, _)| {
        [
            "ping",
            "getMusicFolders",
            "getOpenSubsonicExtensions",
            "getArtists",
            "getAlbumList2",
            "search3",
            "getPlaylists",
            "getPlaylist",
            "getAlbum",
            "getCoverArt",
        ]
        .contains(&method.as_str())
    }));
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
    assert!(
        current
            .ticket("catalog")
            .read::<Value>(textamp::library::cache::REFRESH_INTERVAL)
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn description_playlist_save_uses_snapshot_order_and_does_not_touch_queue() {
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use ratatui::{Terminal, backend::TestBackend};
    use textamp::app::sources::navidrome::commands::CollectionKind;
    use textamp::app::{
        command_palette,
        handlers::{key_input, mouse_input},
        sources::audiomuse,
        state::{BrowseColumn, PaletteCommandKind},
    };
    let (url, requests, task) = server(|m, p| (200, envelope(fixture(m, p)))).await;
    let session = session(url);
    let mut app = App::new(session.clone());
    let tracks: Vec<_> = ["third", "first", "second"]
        .into_iter()
        .map(|id| session.track(serde_json::from_value(song(id, 1, 1)).unwrap()))
        .collect();
    app.state.sources.nav_collection = Some(CollectionKind::AudioMuse(
        textamp::audiomuse::Feature::Describe,
    ));
    app.state.category_column_focused = false;
    app.state.artist_nav.columns = vec![
        BrowseColumn::new("Description history", vec![]),
        BrowseColumn::new_with_tracks(
            "Warm acoustic songs",
            BrowseItem::from_tracks(&tracks),
            tracks.clone(),
        ),
    ];
    app.state.artist_nav.focused_column = 1;
    app.state.artist_nav.columns[1].selected_index = 2;
    app.state.queue.tracks = vec![tracks[1].clone()];
    app.state.queue.index = Some(0);
    app.state.playback.position_ms = 1200;
    let entry = command_palette::materialize_entries(&app.state)
        .into_iter()
        .find(|e| e.label == "Save as playlist")
        .unwrap();
    assert!(matches!(
        entry.command,
        PaletteCommandKind::SaveDescriptionPlaylist
    ));
    for action in command_palette::run(entry.command, &mut app.state) {
        app.send(action).await;
    }
    assert_eq!(
        app.state.popups.input_dialog().unwrap().input.as_str(),
        "Warm acoustic songs"
    );
    // Cancel writes nothing and leaves the results open.
    for action in key_input::handle_key(
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        &mut app.state,
        &app.config,
    ) {
        app.send(action).await;
    }
    assert!(requests.lock().await.is_empty());
    app.state.terminal_width = 120;
    app.state.terminal_height = 32;
    let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
    terminal
        .draw(|f| {
            app.state
                .apply_render_feedback(textamp::ui::render(f, &app.state))
        })
        .unwrap();
    let button = app.state.hit_regions.description_save.unwrap();
    let actions = mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: button.x,
            row: button.y,
            modifiers: KeyModifiers::NONE,
        },
        &mut app.state,
    );
    for action in actions {
        app.send(action).await;
    }
    assert!(app.state.popups.input_dialog().is_some());
    // The captured displayed list, not a later selection/list, is saved.
    app.state.artist_nav.columns[1].tracks.reverse();
    app.state.popups.input_dialog_mut().unwrap().input = " Saved description ".into();
    for action in key_input::handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut app.state,
        &app.config,
    ) {
        app.send(action).await;
    }
    while app.state.sources.nav_tasks.contains_key("playlist-write") {
        app.receive().await;
    }
    let calls = requests.lock().await;
    let writes: Vec<_> = calls
        .iter()
        .filter(|(m, _)| m == "createPlaylist")
        .collect();
    assert_eq!(writes.len(), 1);
    let params = &writes[0].1;
    assert_eq!(params["name"], "Saved description");
    assert_eq!(params["_verb"], "POST");
    assert_eq!(
        serde_json::from_str::<Vec<String>>(&params["_songIds"]).unwrap(),
        ["third", "first", "second"]
    );
    assert!(!params.contains_key("playlistId"));
    drop(calls);
    assert_eq!(app.state.queue.tracks.len(), 1);
    assert_eq!(app.state.queue.tracks[0].rating_key, tracks[1].rating_key);
    assert_eq!(app.state.queue.index, Some(0));
    assert_eq!(app.state.playback.position_ms, 1200);
    assert!(
        app.state
            .notifications
            .status_message
            .as_deref()
            .unwrap()
            .contains("3 tracks")
    );
    // A pending dialog cannot write into a different library.
    let action = audiomuse::Command::SavePlaylist {
        library_key: "different library".into(),
        name: "Wrong".into(),
        tracks: tracks.clone(),
    };
    assert!(
        dispatch_action(
            action.into(),
            &mut app.state,
            &mut app.audio,
            &mut app.config,
            &app.tx
        )
        .await
        .is_err()
    );
    app.state.artist_nav.columns[1].tracks.clear();
    assert!(
        !command_palette::materialize_entries(&app.state)
            .iter()
            .any(|e| e.label == "Save as playlist")
    );
    task.abort();
}

#[tokio::test]
async fn description_playlist_failure_preserves_results_and_reports_error() {
    use textamp::app::sources::audiomuse;
    let (url, requests, task) = server(|m, p| {
        if m == "createPlaylist" {
            (500, "failure".into())
        } else {
            (200, envelope(fixture(m, p)))
        }
    })
    .await;
    let session = session(url);
    let mut app = App::new(session.clone());
    let tracks = vec![session.track(serde_json::from_value(song("one", 1, 1)).unwrap())];
    app.send(audiomuse::Command::SavePlaylist {
        library_key: audiomuse::key(&session),
        name: "Fixture".into(),
        tracks,
    })
    .await;
    while app.state.sources.nav_tasks.contains_key("playlist-write") {
        app.receive().await;
    }
    assert!(app.state.notifications.last_error.is_some());
    assert!(
        !app.state
            .notifications
            .status_message
            .as_deref()
            .is_some_and(|s| s.starts_with("Saved"))
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
    assert!(app.state.queue.tracks.is_empty());
    task.abort();
}

#[tokio::test]
async fn jump_to_artist_and_album_leave_overlays_and_analysis_for_real_library_columns() {
    use textamp::app::sources::navidrome::commands::CollectionKind;
    use textamp::app::{command_palette, state::PaletteCommandKind};
    let (url, _, task) = server(|m, p| (200, envelope(fixture(m, p)))).await;
    let session = session(url);
    let mut app = App::new(session.clone());
    app.catalog().await;
    let track = session.track(serde_json::from_value(song("song", 1, 1)).unwrap());
    for album in [false, true] {
        app.state.sources.nav_collection = Some(CollectionKind::AudioMuse(
            textamp::audiomuse::Feature::Describe,
        ));
        app.state.search.pending_album_key = Some("stale-album".into());
        app.state.popups.set_search_active(true);
        app.state.view = View::Queue;
        let cmd = if album {
            PaletteCommandKind::JumpToAlbum(Box::new(track.clone()))
        } else {
            PaletteCommandKind::JumpToArtist(Box::new(track.clone()))
        };
        for action in command_palette::run(cmd, &mut app.state) {
            app.send(action).await;
        }
        while !app.state.sources.nav_tasks.is_empty() {
            app.receive().await;
        }
        assert!(!app.state.popups.search_active());
        assert!(app.state.sources.nav_collection.is_none());
        assert_eq!(app.state.view, View::Browse);
        assert!(app.state.search.pending_album_key.is_none());
        let selected = app.state.artist_nav.focused().unwrap();
        assert_eq!(
            app.state.artist_nav.columns.len(),
            if album { 3 } else { 2 }
        );
        assert_eq!(
            app.state.artist_nav.focused_column + 1,
            app.state.artist_nav.columns.len()
        );
        if album {
            assert!(
                matches!(selected.play_all_row.as_ref(), Some(textamp::app::state::PlayAllRow::Album { rating_key, .. }) if rating_key == &session.key("album"))
            );
            assert!(!selected.tracks.is_empty());
        } else {
            assert_eq!(
                selected.artist_header.as_ref().unwrap().artist_key,
                session.key("artist")
            );
        }
        assert!(app.state.queue.tracks.is_empty());
    }
    task.abort();
}

#[tokio::test]
async fn similarity_failure_stays_in_results_and_retry_recovers_without_changing_playback() {
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use textamp::app::event::DataEvent;
    use textamp::app::handlers::{key_input, mouse_input};
    let failing = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let check = failing.clone();
    let (url, requests, task) = server(move |m,p| {
        if m == "getOpenSubsonicExtensions" {
            return (200, envelope(json!({"openSubsonicExtensions":[{"name":"sonicSimilarity"}]})));
        }
        if m == "getSonicSimilarTracks" {
            if check.load(std::sync::atomic::Ordering::Relaxed) {
                return (200, json!({"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":0,"message":"Analysis service unavailable"}}}).to_string());
            }
            return (200, envelope(json!({"sonicMatch":[{"entry":song("second",2,1),"similarity":0.9}]})));
        }
        (200, envelope(fixture(m,p)))
    }).await;
    let mut session = session(url);
    session.extensions.insert("sonicSimilarity".into());
    let seed = session.key("first");
    let mut app = App::new(session);
    app.catalog().await;
    app.state.view = View::Queue;
    app.state.queue.tracks = app.state.library.all_tracks.as_ref().clone();
    app.state.queue.index = Some(0);
    app.state.list_state.queue_index = 1;
    let queue = app.state.queue.tracks.clone();
    app.send(DataAction::LoadSimilarTracks {
        rating_key: seed.clone(),
        title: "First".into(),
    })
    .await;
    while app.state.similar.request.is_loading() {
        app.receive().await;
    }
    assert!(
        app.state
            .similar
            .request
            .error()
            .as_ref()
            .unwrap()
            .contains("getSonicSimilarTracks")
    );
    assert!(app.state.notifications.last_error.is_none());
    assert_eq!(app.state.previous_view, Some(View::Queue));
    assert_eq!(app.state.similar.source_key.as_deref(), Some(seed.as_str()));

    app.state.terminal_width = 100;
    app.state.terminal_height = 32;
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 32)).unwrap();
    terminal
        .draw(|frame| {
            app.state
                .apply_render_feedback(textamp::ui::render(frame, &app.state))
        })
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    if let Ok(dir) = std::env::var("TEXTAMP_SETTINGS_PREVIEW_DIR") {
        let cells: Vec<_> = terminal.backend().buffer().content.iter().map(|c| serde_json::json!({"s":c.symbol(),"fg":format!("{:?}",c.fg),"bg":format!("{:?}",c.bg)})).collect();
        std::fs::write(
            std::path::Path::new(&dir).join("similar-error-100x32.json"),
            serde_json::to_vec(&serde_json::json!({"width":100,"height":32,"cells":cells}))
                .unwrap(),
        )
        .unwrap();
    }
    assert!(text.contains("Couldn’t load similar tracks"));
    assert!(text.contains("[F5] retry"));
    assert!(!text.contains("No similar tracks found"));
    let rect = app
        .state
        .hit_regions
        .similar_content
        .as_ref()
        .unwrap()
        .retry_hint
        .unwrap();
    let click = mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        },
        &mut app.state,
    );
    let keys = key_input::handle_key(
        KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE),
        &mut app.state,
        &app.config,
    );
    for actions in [&click, &keys] {
        assert!(
            matches!(actions.as_slice(), [Action::Data(DataAction::LoadSimilarTracks {rating_key,..})] if rating_key == &seed),
            "{actions:?}"
        );
    }
    failing.store(false, std::sync::atomic::Ordering::Relaxed);
    for action in keys {
        app.send(action).await;
    }
    assert!(app.state.similar.request.error().is_none());
    while app.state.similar.request.is_loading() {
        app.receive().await;
    }
    assert!(app.state.similar.request.error().is_none());
    assert_eq!(app.state.similar.tracks[0].title, "Song second");
    handle_core_event(
        Event::Data(DataEvent::SimilarLoadFailed {
            request_key: seed,
            message: "late failure".into(),
        }),
        &mut app.state,
        &app.tx,
    );
    assert!(app.state.similar.request.error().is_none());
    assert_eq!(
        app.state
            .queue
            .tracks
            .iter()
            .map(|t| &t.rating_key)
            .collect::<Vec<_>>(),
        queue.iter().map(|t| &t.rating_key).collect::<Vec<_>>()
    );
    assert_eq!(app.state.queue.index, Some(0));
    assert_eq!(app.state.list_state.queue_index, 1);
    assert!(
        !requests
            .lock()
            .await
            .iter()
            .any(|r| r.0 == "getSimilarSongs2")
    );
    task.abort();
}

#[tokio::test]
async fn inline_similar_music_excludes_seed_and_same_artist() {
    let (url, _, task) = server(|method, params| {
        let mut other = song("other", 1, 1);
        other["artist"] = json!("Another Artist");
        other["albumArtist"] = json!("Another Artist");
        other["artistId"] = json!("other-artist");
        other["albumId"] = json!("other-album");
        let payload = match method {
            "search3" => {
                json!({"searchResult3":{"song":[song("first",1,1),song("second",2,1),other]}})
            }
            "getSimilarSongs2" => {
                json!({"similarSongs2":{"song":[song("first",1,1),song("second",2,1),other]}})
            }
            _ => fixture(method, params),
        };
        (200, envelope(payload))
    })
    .await;
    let session = session(url);
    let mut app = App::new(session.clone());
    app.catalog().await;
    let key = session.key("first");
    app.send(DataAction::LoadTrackPaneSimilar {
        rating_key: key.clone(),
    })
    .await;
    while app.state.track_pane_similar_loading.contains(&key) {
        app.receive().await;
    }
    let matches = app.state.track_pane_similar[&key].as_ref().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].rating_key, session.key("other"));
    assert_eq!(app.state.view, View::Browse);
    assert!(app.state.queue.tracks.is_empty());
    task.abort();
}

#[tokio::test]
async fn miller_artist_info_loads_in_adjacent_column_without_modal_or_playback() {
    use textamp::app::state::{AllTracksScope, BrowseColumn, BrowseDetails};
    let (url, _, task) = server(|method, params| (200, envelope(fixture(method, params)))).await;
    let session = session(url);
    let mut app = App::new(session.clone());
    app.catalog().await;
    let key = session.key("artist");
    app.state.artist_nav.columns = vec![BrowseColumn::new(
        "albums",
        vec![
            BrowseItem::ArtistInfo {
                artist_key: key.clone(),
                artist_name: "Artist".into(),
                thumb: None,
            },
            BrowseItem::AllTracks {
                scope: AllTracksScope::Artist {
                    artist_key: key.clone(),
                    artist_name: "Artist".into(),
                },
                thumb: None,
            },
        ],
    )];
    app.send(SearchAction::ShowArtistInfo {
        artist_key: key.clone(),
        artist_name: "Artist".into(),
    })
    .await;
    assert_eq!(app.state.artist_nav.focused_column, 0);
    assert_eq!(app.state.artist_nav.columns.len(), 2);
    assert!(app.state.popups.artist_bio().is_none());
    while matches!(&app.state.artist_nav.columns[1].details,Some(BrowseDetails::Artist(info)) if info.task.is_some())
    {
        app.receive().await;
    }
    assert!(
        matches!(&app.state.artist_nav.columns[1].details,Some(BrowseDetails::Artist(info)) if info.document.text.contains("An artist biography") && info.task.is_none())
    );
    assert!(app.state.queue.tracks.is_empty());
    assert_eq!(app.state.playback.status, PlayStatus::Stopped);
    assert_eq!(app.state.artist_info_column_index(), Some(1));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|frame| {
            app.state
                .apply_render_feedback(textamp::ui::render(frame, &app.state))
        })
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Artist Info"));
    assert!(text.contains("artist biography"));
    assert!(text.contains("Artist Mix"));
    task.abort();
}

#[tokio::test]
async fn fresh_catalog_prunes_deleted_playlist_views_without_changing_playback() {
    use textamp::config::settings::PlaylistView;
    let (url, _, task) = server(|method, params| (200, envelope(fixture(method, params)))).await;
    let session = session(url);
    let mut app = App::new(session.clone());
    let library = app.state.active_library.clone().unwrap();
    let live = session.key("playlist");
    let deleted = session.key("deleted-playlist");
    let view = PlaylistView {
        group_by_album: true,
    };
    for key in [&live, &deleted] {
        app.config
            .ui
            .library_view_settings
            .entry(library.clone())
            .or_default()
            .playlists
            .insert(key.clone(), view);
        app.state
            .playlist_views
            .entry(library.clone())
            .or_default()
            .insert(key.clone(), view);
    }
    app.catalog().await;
    assert_eq!(
        app.config.ui.library_view_settings[&library]
            .playlists
            .len(),
        1
    );
    assert_eq!(
        app.config.ui.library_view_settings[&library].playlists[&live],
        view
    );
    assert_eq!(app.state.playlist_views[&library].len(), 1);
    assert_eq!(app.state.playback.status, PlayStatus::Stopped);
    assert!(app.state.queue.tracks.is_empty());
    task.abort();
}

#[tokio::test]
async fn catalog_scan_precaches_playlist_covers_and_ordered_tracks_and_rescan_reuses_images() {
    let (url, requests, server) = server(|method,params| {
        let payload = match method {
            "getPlaylist" => {
                let mut entries:Vec<_>=(0..105).map(|index| {
                    let mut entry=song(&format!("song-{index}"),1,1);
                    entry["albumId"]=json!(format!("outside-{index}"));
                    entry
                }).collect();
                entries.push(entries[0].clone());
                json!({"playlist":{"id":"playlist","name":"Large playlist","songCount":106,"entry":entries}})
            },
            "getAlbum" => json!({"album":{"id":params["id"],"name":"Outside folder","coverArt":format!("art-{}",params["id"])}}),
            _=>fixture(method,params),
        };
        (200,envelope(payload))
    }).await;
    let mut session = session(url);
    session.source.id = format!("art-fixture-{}", uuid::Uuid::new_v4());
    let mut app = App::new(session.clone());
    app.state.active_library = Some(format!("navidrome:{}:1", session.source.id));
    app.catalog().await;
    textamp::library::credentials::save_source(
        "navidrome",
        &session.source.id,
        "fixture-password".into(),
    )
    .unwrap();
    let choice = LibraryChoice::Navidrome {
        source: session.source.clone(),
        folder: session.client.folder.clone(),
        name: "Fixture".into(),
    };
    assert!(textamp::app::sources::cache::running(&choice, &app.state));
    while textamp::app::sources::cache::running(&choice, &app.state) {
        app.receive().await;
    }
    assert_eq!(
        requests
            .lock()
            .await
            .iter()
            .filter(|(method, _)| method == "getCoverArt")
            .count(),
        106
    );
    textamp::app::sources::cache::start(choice.clone(), true, &mut app.state, &app.tx);
    while textamp::app::sources::cache::running(&choice, &app.state) {
        app.receive().await;
    }
    let art = textamp::media::ArtworkCache::for_library(app.state.active_library.as_ref().unwrap());
    for index in 0..105 {
        assert!(
            art.load(&session.key(&format!("outside-{index}")), 0)
                .is_some(),
            "album {index}: {:?}; {:?}",
            app.state.notifications.status_message,
            requests
                .lock()
                .await
                .iter()
                .map(|(m, _)| m)
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(
        requests
            .lock()
            .await
            .iter()
            .filter(|(method, _)| method == "getCoverArt")
            .count(),
        106
    );
    let calls = requests.lock().await.len();
    server.abort();
    app.state.browse_category = BrowseCategory::Playlists;
    app.send(MillerAction::LoadPlaylistTracksForMiller {
        playlist_key: session.key("playlist"),
        replace_child: false,
    })
    .await;
    while app.state.playlist_nav.loading {
        app.receive().await;
    }
    let tracks = &app.state.playlist_nav.columns.last().unwrap().tracks;
    assert_eq!(tracks.len(), 106);
    assert_eq!(
        tracks.first().unwrap().rating_key,
        tracks.last().unwrap().rating_key
    );
    assert_eq!(tracks[1].rating_key, session.key("song-1"));
    assert_eq!(requests.lock().await.len(), calls);
}

#[tokio::test]
async fn failed_playlist_refresh_preserves_the_last_ordered_snapshot() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let fail = Arc::new(AtomicBool::new(false));
    let failure = fail.clone();
    let (url, requests, server) = server(move |method, params| {
        if method == "getPlaylist" && failure.load(Ordering::SeqCst) {
            (503, "Unavailable".into())
        } else {
            (200, envelope(fixture(method, params)))
        }
    })
    .await;
    let session = session(url);
    let mut app = App::new(session.clone());
    app.catalog().await;
    app.state.browse_category = BrowseCategory::Playlists;
    app.send(MillerAction::LoadPlaylistTracksForMiller {
        playlist_key: session.key("playlist"),
        replace_child: false,
    })
    .await;
    while app.state.playlist_nav.loading {
        app.receive().await;
    }
    fail.store(true, Ordering::SeqCst);
    app.catalog().await;
    std::sync::Arc::make_mut(&mut app.state.library.playlists)[0].updated_at = Some(1);
    app.state.browse_category = BrowseCategory::Playlists;
    app.send(MillerAction::LoadPlaylistTracksForMiller {
        playlist_key: session.key("playlist"),
        replace_child: false,
    })
    .await;
    while app.state.playlist_nav.loading {
        app.receive().await;
    }
    let tracks = &app.state.playlist_nav.columns.last().unwrap().tracks;
    assert_eq!(
        tracks
            .iter()
            .map(|t| t.rating_key.clone())
            .collect::<Vec<_>>(),
        vec![session.key("second"), session.key("first")]
    );
    assert_eq!(
        requests
            .lock()
            .await
            .iter()
            .filter(|(method, _)| method == "getPlaylist")
            .count(),
        2
    );
    server.abort();
}

#[tokio::test]
async fn recently_added_date_sort_restores_server_order_after_title_sort_and_reverse() {
    use textamp::app::sources::navidrome::commands::{CollectionKind, Command};
    use textamp::app::state::{ColumnSortMode, SortColumnType};
    let (url, _, task) = server(|method, params| {
        if method == "getAlbumList2" && params.get("type").is_some_and(|kind| kind == "newest") {
            (
                200,
                envelope(json!({"albumList2":{"album":[
                    {"id":"newest","name":"Zulu","year":1990},
                    {"id":"middle","name":"Alpha","year":2026},
                    {"id":"oldest","name":"Mike","year":2020}
                ]}})),
            )
        } else {
            (200, envelope(fixture(method, params)))
        }
    })
    .await;
    let mut app = App::new(session(url));
    app.send(NavAction::Command(Command::Collection(
        CollectionKind::RecentlyAdded,
    )))
    .await;
    while app.state.artist_nav.loading {
        app.receive().await;
    }
    assert_eq!(
        app.state.column_options_type(0),
        Some(SortColumnType::RecentAlbums)
    );
    let titles = |state: &AppState| {
        state.artist_nav.columns[0]
            .items
            .iter()
            .map(|i| i.title().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(titles(&app.state), ["Zulu", "Alpha", "Mike"]);
    app.send(SearchAction::ApplyFocusedSortMode(ColumnSortMode::ByTitle))
        .await;
    assert_eq!(titles(&app.state), ["Alpha", "Mike", "Zulu"]);
    app.send(SearchAction::ApplyFocusedSortMode(
        ColumnSortMode::ByDateAdded,
    ))
    .await;
    assert_eq!(titles(&app.state), ["Zulu", "Alpha", "Mike"]);
    assert!(!app.state.artist_nav.columns[0].sort_ascending);
    app.send(SearchAction::ReverseFocusedSortDirection).await;
    assert_eq!(titles(&app.state), ["Mike", "Alpha", "Zulu"]);
    assert!(app.state.artist_nav.columns[0].sort_ascending);
    app.send(SearchAction::ApplyFocusedSortMode(ColumnSortMode::ByTitle))
        .await;
    assert_eq!(titles(&app.state), ["Alpha", "Mike", "Zulu"]);
    assert!(app.state.artist_nav.columns[0].sort_ascending);
    assert!(app.state.queue.tracks.is_empty());
    task.abort();
}

#[tokio::test]
async fn unified_account_catalog_and_search_include_both_music_folders_without_scope_parameters() {
    let (url,requests,task)=server(|method,params| {
        assert!(!params.contains_key("musicFolderId"));
        let value=match method {
            "getMusicFolders"=>json!({"musicFolders":{"musicFolder":[{"id":"1","name":"Main"},{"id":"2","name":"Collections"}]}}),
            "getArtists"=>json!({"artists":{"index":[{"name":"U","artist":[{"id":"one","name":"Unified One"},{"id":"two","name":"Unified Two"}]}]}}),
            "getAlbumList2"=>json!({"albumList2":{"album":[{"id":"album-one","name":"Album One","artist":"Unified One","artistId":"one"},{"id":"album-two","name":"Album Two","artist":"Unified Two","artistId":"two"}]}}),
            "search3"=>json!({"searchResult3":{"song":[{"id":"track-one","title":"First","albumId":"album-one","artistId":"one","artist":"Unified One"},{"id":"track-two","title":"Second","albumId":"album-two","artistId":"two","artist":"Unified Two"}]}}),
            "getPlaylists"=>json!({"playlists":{"playlist":[]}}),
            _=>fixture(method,params),
        };
        (200,envelope(value))
    }).await;
    let mut source = source(url);
    source.libraries = vec![
        textamp::navidrome::MusicFolder {
            id: "1".into(),
            name: "Main".into(),
        },
        textamp::navidrome::MusicFolder {
            id: "2".into(),
            name: "Collections".into(),
        },
    ];
    let session = Session {
        client: Client::new(&source, "fixture-password".into(), None).unwrap(),
        source,
        extensions: Default::default(),
    };
    assert!(session.client.folder.is_none());
    let mut app = App::new(session);
    app.state.active_library = Some("navidrome:account-one:all".into());
    app.catalog().await;
    assert_eq!(app.state.library.albums.len(), 2);
    assert_eq!(app.state.library.all_tracks.len(), 2);
    app.send(SearchAction::SetSearchQuery("Unified".into()))
        .await;
    let artists = &app.state.search.results.as_ref().unwrap().artists;
    assert_eq!(artists.len(), 2);
    assert!(artists.iter().any(|a| a.title == "Unified One"));
    assert!(artists.iter().any(|a| a.title == "Unified Two"));
    assert!(
        requests
            .lock()
            .await
            .iter()
            .filter(|(m, _)| matches!(m.as_str(), "getArtists" | "getAlbumList2" | "search3"))
            .all(|(_, p)| !p.contains_key("musicFolderId"))
    );
    assert!(app.state.queue.tracks.is_empty());
    task.abort();
}

#[tokio::test]
async fn random_search_and_name_jumps_select_the_same_artist_and_album_ancestors() {
    use textamp::app::{
        command_palette,
        state::{PaletteCommandKind, SearchTab},
    };
    let (url, _, task) = server(|m, p| (200, envelope(fixture(m, p)))).await;
    let session = session(url);
    let mut app = App::new(session.clone());
    app.catalog().await;
    for route in 0..4 {
        app.state.view = View::Browse;
        let actions = match route {
            0 => command_palette::run(PaletteCommandKind::GoToRandomAlbum, &mut app.state),
            1 => {
                app.state.popups.set_search_active(true);
                app.state.search.tab = SearchTab::Albums;
                app.state.search.results = Some(textamp::library::models::SearchResults {
                    albums: app.state.library.albums.as_ref().clone(),
                    ..Default::default()
                });
                app.state.list_state.search_item_index = 0;
                vec![SearchAction::SelectSearchResult.into()]
            }
            2 => command_palette::run(
                PaletteCommandKind::JumpToAlbum(Box::new(
                    session.track(serde_json::from_value(song("song", 1, 1)).unwrap()),
                )),
                &mut app.state,
            ),
            _ => vec![
                BrowseAction::OpenInLibrary {
                    artist_key: session.key("artist"),
                    artist_name: "Artist".into(),
                    album_key: Some(session.key("album")),
                    album_title: Some("Album".into()),
                }
                .into(),
            ],
        };
        for action in actions {
            app.send(action).await;
        }
        while !app.state.sources.nav_tasks.is_empty() {
            app.receive().await;
        }
        assert!(
            matches!(app.state.artist_nav.columns[0].selected_item(),Some(BrowseItem::Artist{key,..}) if key==&session.key("artist")),
            "route {route}"
        );
        assert!(
            matches!(app.state.artist_nav.columns[1].selected_item(),Some(BrowseItem::Album{key,..}) if key==&session.key("album")),
            "route {route}"
        );
        assert_eq!(app.state.artist_nav.columns.len(), 3);
        let column = app.state.artist_nav.focused().unwrap();
        assert!(
            matches!(&column.play_all_row, Some(textamp::app::state::PlayAllRow::Album { rating_key, .. }) if rating_key == &session.key("album")),
            "route {route}"
        );
        assert!(!column.tracks.is_empty(), "route {route}");
        assert!(app.state.queue.tracks.is_empty());
    }
    task.abort();
}

#[tokio::test]
async fn seeded_radio_uses_one_chooser_and_refills_from_evolving_or_fixed_seed() {
    use textamp::app::state::{PlayStatus, PlaybackMode, RadioRefill, TrackPickerPurpose};
    let (url, seen, server) = server(|method, params| {
        let payload = if method == "getSonicSimilarTracks" {
            let matches: Vec<_> = ["first", "second", "disc2"]
                .into_iter()
                .filter(|id| params.get("id").map(String::as_str) != Some(*id))
                .map(|id| json!({"entry":song(id,1,1),"similarity":0.9}))
                .collect();
            json!({"sonicMatch":matches})
        } else {
            fixture(method, params)
        };
        (200, envelope(payload))
    })
    .await;
    for fixed in [false, true] {
        let mut session = session(url.clone());
        session.extensions.insert("sonicSimilarity".into());
        let mut app = App::new(session.clone());
        app.state.library.all_tracks = std::sync::Arc::new(
            ["first", "second", "disc2"]
                .iter()
                .map(|id| session.track(serde_json::from_value(song(id, 1, 1)).unwrap()))
                .collect(),
        );
        let seed = app.state.library.all_tracks[0].clone();
        app.state.queue.tracks = vec![seed.clone()];
        app.state.queue.index = Some(0);
        app.state.set_playback_mode(PlaybackMode::Queue);
        app.state.playback.status = PlayStatus::Paused;
        app.state.playback.request_id = 7;
        app.send(BrowseAction::LoadStations).await;
        seen.lock().await.clear();
        app.send(RadioAction::PlayStation(
            if fixed {
                "nav-radio/stayClose"
            } else {
                "nav-radio/sonic"
            }
            .into(),
        ))
        .await;
        let picker = app
            .state
            .popups
            .adventure_launcher()
            .expect("station opens a single seed chooser");
        assert_eq!(
            picker.purpose,
            if fixed {
                TrackPickerPurpose::StayCloseRadio
            } else {
                TrackPickerPurpose::SonicRadio
            }
        );
        assert_eq!(
            picker.start_track.as_ref().unwrap().rating_key,
            seed.rating_key
        );
        assert!(seen.lock().await.is_empty());
        app.send(SearchAction::StartPickedTrackRadio).await;
        while app.state.station_starting.is_some() {
            app.receive().await;
        }
        assert_eq!(app.state.playback.status, PlayStatus::Paused);
        assert_eq!(app.state.playback.request_id, 7);
        assert_eq!(app.state.radio.tracks[0].rating_key, seed.rating_key);
        assert_eq!(app.state.radio.tracks.len(), 3);
        app.state.radio.track_index = Some(2);
        let current = app.state.current_track().unwrap().clone();
        textamp::app::sources::radio::refill(&app.tx, &mut app.state);
        while app.state.radio.refill != RadioRefill::Idle {
            app.receive().await;
        }
        assert!(
            app.state.radio.tracks.len() > 3,
            "radio continues beyond its initial batch"
        );
        assert_eq!(
            app.state.current_track().unwrap().rating_key,
            current.rating_key
        );
        let requests = seen.lock().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].1["id"], "first");
        assert_eq!(
            requests[1].1["id"],
            if fixed {
                "first".to_owned()
            } else {
                session.id(&current.rating_key).unwrap()
            }
        );
    }
    server.abort();
}

#[test]
fn track_rating_choices_stay_inside_the_submenu_even_after_use() {
    use textamp::app::sources::navidrome::commands::Command;
    use textamp::app::{
        command_catalog, command_history, command_palette, state::PaletteCommandKind,
    };
    let session = session("http://fixture.invalid".into());
    let mut app = App::new(session.clone());
    app.state
        .queue
        .tracks
        .push(session.track(serde_json::from_value(song("first", 1, 1)).unwrap()));
    app.state.view = View::Queue;
    let rating = PaletteCommandKind::Navidrome(Command::Rating(5));
    app.state
        .palette
        .history
        .remember(command_history::key(&rating).unwrap());
    let group = command_catalog::path(&rating);
    assert_eq!(group, ["Edit", "Track Rating"]);
    for query in ["", "stars", "rate song"] {
        command_palette::open_with_query(&mut app.state, query);
        let palette = &app.state.palette;
        let mut visible = palette.matches.iter().map(|&i| &palette.entries[i].command);
        assert!(
            !visible
                .clone()
                .any(|c| matches!(c, PaletteCommandKind::Navidrome(Command::Rating(_))))
        );
        if !query.is_empty() {
            assert!(visible.any(
                |c| matches!(c, PaletteCommandKind::OpenCommandGroup(path) if path == &group)
            ));
        }
    }
    command_palette::run(PaletteCommandKind::OpenCommandGroup(group), &mut app.state);
    let palette = &app.state.palette;
    let mut ratings = palette
        .matches
        .iter()
        .filter_map(|&i| match &palette.entries[i].command {
            PaletteCommandKind::Navidrome(Command::Rating(stars)) => Some(*stars),
            _ => None,
        })
        .collect::<Vec<_>>();
    ratings.sort_unstable();
    assert_eq!(ratings, [0, 1, 2, 3, 4, 5]);
}

#[tokio::test]
async fn saved_playlist_appears_immediately_and_refreshes_only_playlists() {
    let saved = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let server_saved = saved.clone();
    let (url, requests, task) = server(move |method, params| {
        let playlist = json!({"id":"new-playlist","name":"A Saved Queue","songCount":1});
        let result = match method {
            "createPlaylist" => {
                server_saved.store(true, std::sync::atomic::Ordering::SeqCst);
                json!({"playlist":playlist})
            }
            "getPlaylists" if server_saved.load(std::sync::atomic::Ordering::SeqCst) => {
                json!({"playlists":{"playlist":[playlist]}})
            }
            _ => fixture(method, params),
        };
        (200, envelope(result))
    })
    .await;
    let session = session(url);
    let mut app = App::new(session.clone());
    app.catalog().await;
    app.state.queue.tracks =
        vec![session.track(serde_json::from_value(song("first", 1, 1)).unwrap())];
    let store = session.cache_store().unwrap();
    let catalog = store.ticket("catalog");
    let browse = store.ticket("browse-catalog-v1");
    let original_catalog = catalog
        .read::<serde_json::Value>(Duration::from_secs(60))
        .unwrap()
        .unwrap()
        .value;
    let original_browse = browse
        .read::<serde_json::Value>(Duration::from_secs(60))
        .unwrap()
        .unwrap()
        .value;
    requests.lock().await.clear();
    app.send(QueueAction::SaveQueueAsPlaylist("A Saved Queue".into()))
        .await;
    while app.state.sources.nav_tasks.contains_key("playlist-write") {
        app.receive().await;
    }
    assert!(
        app.state
            .library
            .playlists
            .iter()
            .any(|playlist| playlist.title == "A Saved Queue")
    );
    assert!(
        app.state.playlist_nav.columns[0]
            .items
            .iter()
            .any(|item| item.title() == "A Saved Queue")
    );
    while app.state.sources.nav_tasks.contains_key("playlist-list") {
        app.receive().await;
    }
    assert!(
        app.state
            .library
            .playlists
            .iter()
            .any(|playlist| playlist.title == "A Saved Queue")
    );
    assert_eq!(
        catalog
            .read::<serde_json::Value>(Duration::from_secs(60))
            .unwrap()
            .unwrap()
            .value,
        original_catalog
    );
    assert_eq!(
        browse
            .read::<serde_json::Value>(Duration::from_secs(60))
            .unwrap()
            .unwrap()
            .value,
        original_browse
    );
    let mut reopened = App::new(session.clone());
    textamp::app::sources::navidrome::open(&mut reopened.state, &reopened.tx);
    while reopened.state.library_loading {
        reopened.receive().await;
    }
    assert!(
        reopened
            .state
            .library
            .playlists
            .iter()
            .any(|p| p.title == "A Saved Queue")
    );
    let calls = requests.lock().await;
    assert_eq!(
        calls
            .iter()
            .filter(|(method, _)| method == "createPlaylist")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|(method, _)| method == "getPlaylists")
            .count(),
        1
    );
    assert!(
        !calls.iter().any(|(method, _)| matches!(
            method.as_str(),
            "getArtists" | "getAlbumList2" | "search3"
        ))
    );
    assert_eq!(app.state.queue.tracks.len(), 1);
    task.abort();
}

#[tokio::test]
async fn random_albums_load_on_demand_once_and_refresh_explicitly() {
    use textamp::app::sources::navidrome::{
        commands::{CollectionKind, Command},
        random_albums::State,
    };
    let (url, seen, task) = server(|method, params| {
        let response = if method == "getAlbumList2" && params.get("type").is_some_and(|kind|kind == "random") {
            json!({"albumList2":{"album":(0..100).map(|index|json!({"id":format!("random-{index}"),"name":format!("Random {index}")})).collect::<Vec<_>>()}})
        } else { fixture(method, params) };
        (200, envelope(response))
    }).await;
    let mut app = App::new(session(url));
    textamp::app::sources::navidrome::open(&mut app.state, &app.tx);
    textamp::app::sources::navidrome::open(&mut app.state, &app.tx);
    while app.state.library_loading {
        app.receive().await;
    }
    assert!(matches!(app.state.sources.random_albums, State::Idle));
    assert!(
        !seen
            .lock()
            .await
            .iter()
            .any(|(method, params)| method == "getAlbumList2"
                && params.get("type").is_some_and(|kind| kind == "random"))
    );
    assert_eq!(app.state.sources.nav_collection, None);
    assert!(app.state.queue.tracks.is_empty());
    assert_eq!(app.state.view, View::Browse);
    for _ in 0..2 {
        app.send(NavAction::Command(Command::Collection(
            CollectionKind::RandomAlbums,
        )))
        .await;
        while app.state.artist_nav.loading {
            app.receive().await;
        }
        assert_eq!(app.state.artist_nav.columns[0].items.len(), 100);
        assert_eq!(app.state.artist_nav.columns[0].items[0].title(), "Random 0");
    }
    let requests = seen.lock().await;
    let random: Vec<_> = requests
        .iter()
        .filter(|(method, params)| {
            method == "getAlbumList2" && params.get("type").is_some_and(|kind| kind == "random")
        })
        .collect();
    assert_eq!(random.len(), 1);
    assert_eq!(random[0].1["size"], "100");
    assert_eq!(random[0].1["musicFolderId"], "1");
    drop(requests);
    let actions = textamp::app::handlers::helpers::refresh_current_view(&mut app.state);
    for action in actions {
        app.send(action).await;
    }
    while app.state.artist_nav.loading {
        app.receive().await;
    }
    let requests = seen.lock().await;
    assert_eq!(
        requests
            .iter()
            .filter(|(method, params)| method == "getAlbumList2"
                && params.get("type").is_some_and(|kind| kind == "random"))
            .count(),
        2
    );
    drop(requests);
    let generation = app.state.library_generation;
    app.state.advance_library_generation();
    app.send(NavAction::RandomAlbumsLoaded {
        generation,
        result: Ok(vec![]),
    })
    .await;
    assert!(matches!(app.state.sources.random_albums, State::Idle));
    task.abort();
}

#[tokio::test]
async fn large_cached_track_search_is_replaceable_and_keeps_catalog_shared() {
    let mut app = App::new(session("http://127.0.0.1:1".into()));
    app.state.library.all_tracks = std::sync::Arc::new(
        (0..1024)
            .map(|i| textamp::library::models::Track {
                rating_key: format!("song-{i}"),
                title: if i == 1023 {
                    "Newest Result".into()
                } else {
                    "Other Song".into()
                },
                ..Default::default()
            })
            .collect(),
    );
    app.state.library.artists = std::sync::Arc::new(
        (0..1024)
            .map(|i| textamp::library::models::Artist {
                rating_key: format!("artist-{i}"),
                title: if i == 1023 {
                    "Newest Artist".into()
                } else {
                    "Other Artist".into()
                },
                ..Default::default()
            })
            .collect(),
    );
    app.state.library.albums = std::sync::Arc::new(
        (0..1024)
            .map(|i| textamp::library::models::Album {
                rating_key: format!("album-{i}"),
                title: if i == 1023 {
                    "Newest Album".into()
                } else {
                    "Other Album".into()
                },
                ..Default::default()
            })
            .collect(),
    );
    let catalog = app.state.library.all_tracks.clone();
    app.send(SearchAction::SetSearchQuery("Other".into())).await;
    assert!(app.state.sources.nav_tasks.contains_key("catalog-search"));
    assert!(std::sync::Arc::strong_count(&catalog) >= 3);
    app.send(SearchAction::SetSearchQuery("Newest".into()))
        .await;
    while app.state.sources.nav_tasks.contains_key("catalog-search") {
        app.receive().await;
    }
    let found = &app.state.search.results.as_ref().unwrap().tracks;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title, "Newest Result");
    assert_eq!(
        app.state.search.results.as_ref().unwrap().artists[0].title,
        "Newest Artist"
    );
    assert_eq!(
        app.state.search.results.as_ref().unwrap().albums[0].title,
        "Newest Album"
    );
    app.send(NavAction::SearchReady {
        query: "Other".into(),
        catalog_id: app.state.sources.list_id,
        results: Default::default(),
    })
    .await;
    app.send(NavAction::SearchReady {
        query: "Newest".into(),
        catalog_id: app.state.sources.list_id.wrapping_sub(1),
        results: Default::default(),
    })
    .await;
    assert_eq!(app.state.search.results.as_ref().unwrap().tracks.len(), 1);

    assert!(std::sync::Arc::ptr_eq(
        &catalog,
        &app.state.library.all_tracks
    ));
    app.send(SearchAction::SetSearchQuery("Other".into())).await;
    app.send(SearchAction::SetSearchQuery(String::new())).await;
    assert!(!app.state.sources.nav_tasks.contains_key("catalog-search"));
    assert!(app.state.search.results.is_none());
}

#[tokio::test]
async fn cached_startup_opens_track_navigation_after_full_metadata_arrives() {
    let (url, requests, server) =
        server(|method, params| (200, envelope(fixture(method, params)))).await;
    let session = session(url);
    let mut imported = App::new(session.clone());
    imported.catalog().await;
    let before = requests
        .lock()
        .await
        .iter()
        .filter(|(method, _)| method == "search3")
        .count();
    let mut app = App::new(session);
    app.state.view = View::Browse;
    textamp::app::sources::navidrome::open(&mut app.state, &app.tx);
    while app.state.library.artists.is_empty() {
        app.receive().await;
    }
    assert!(matches!(
        app.state.library.catalog_loading,
        textamp::app::catalog_loading::State::Browsing { .. }
    ));
    assert_eq!(app.state.library.all_tracks.len(), 3);
    app.send(MillerAction::LoadAllLibraryTracksForMiller {
        replace_child: false,
    })
    .await;
    assert_eq!(app.state.artist_nav.columns.last().unwrap().tracks.len(), 3);
    while app.state.library_loading {
        app.receive().await;
    }
    assert!(matches!(
        app.state.library.catalog_loading,
        textamp::app::catalog_loading::State::Ready
    ));
    assert_eq!(app.state.artist_nav.columns.last().unwrap().tracks.len(), 3);
    assert_eq!(
        requests
            .lock()
            .await
            .iter()
            .filter(|(method, _)| method == "search3")
            .count(),
        before
    );
    server.abort();
}

#[tokio::test]
async fn empty_split_album_cache_retries_and_restores_both_performers() {
    let (url, requests, task) = server(|method, params| {
        let value = match method {
            "getAlbum" => json!({"album":{"id":"split","name":"Split 7 inch","songCount":2,"song":[]}}),
            "search3" => json!({"searchResult3":{"song":[
                {"id":"side-a","title":"Side A","albumId":"split","album":"Split 7 inch","artist":"Artist A"},
                {"id":"side-b","title":"Side B","albumId":"split","album":"Split 7 inch","artist":"Artist B"}
            ]}}),
            _ => fixture(method, params),
        };
        (200, envelope(value))
    }).await;
    let session = session(url);
    let key = session.key("split");
    let ticket = session
        .cache_store()
        .unwrap()
        .ticket(&format!("album-tracks:{key}"));
    ticket
        .write(&Vec::<textamp::library::models::Track>::new())
        .unwrap();
    let tracks = session.album_tracks(&key).await.unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].track_artist(), "Artist A");
    assert_eq!(tracks[1].track_artist(), "Artist B");
    assert!(
        requests
            .lock()
            .await
            .iter()
            .any(|(method, _)| method == "getAlbum")
    );
    task.abort();
}

#[tokio::test]
async fn artwork_batch_publishes_ready_images_without_cancelling_inflight_work() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (finish, release) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        socket.read(&mut request).await.unwrap();
        release.await.unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nslow")
            .await
            .unwrap();
    });
    let session = session(url);
    let fast = session.key("fast-artwork-progress");
    let slow = session.key("slow-artwork-progress");
    let mut app = App::new(session);
    assert!(
        textamp::media::ArtworkCache::for_library(app.state.active_library.as_deref().unwrap())
            .save(&fast, b"fast")
    );
    app.send(SystemAction::LoadAlbumArt(vec![
        (fast.clone(), fast.clone()),
        (slow.clone(), slow.clone()),
    ]))
    .await;
    let request_id = app.state.sources.nav_tasks["art-grid"].0;
    while !app.state.artwork.grid_cache.contains_key(&fast) {
        app.receive().await;
    }
    assert!(
        app.state.sources.nav_tasks.contains_key("art-grid"),
        "fast art must publish before the slow image finishes"
    );
    app.send(SystemAction::LoadAlbumArt(vec![(
        slow.clone(),
        slow.clone(),
    )]))
    .await;
    assert_eq!(
        app.state.sources.nav_tasks["art-grid"].0, request_id,
        "new viewport requests must not cancel blocking image jobs"
    );
    assert!(app.state.artwork.grid_pending.contains(&slow));
    app.send(NavAction::Progress {
        generation: app.state.library_generation,
        request_id: request_id + 1,
        slot: "art-grid",
        actions: vec![SystemAction::SetStatus("stale-artwork".into()).into()],
    })
    .await;
    assert_ne!(
        app.state.notifications.status_message.as_deref(),
        Some("stale-artwork")
    );
    finish.send(()).unwrap();
    while app.state.sources.nav_tasks.contains_key("art-grid") {
        app.receive().await;
    }
    assert_eq!(app.state.artwork.grid_cache[&slow], b"slow");
    server.await.unwrap();
}

#[tokio::test]
async fn composed_artwork_is_cached_independently_of_source_covers() {
    let session = session("http://127.0.0.1:9".into());
    let thumb = session.key("mosaic-cache-source");
    let mut app = App::new(session);
    let cache =
        textamp::media::ArtworkCache::for_library(app.state.active_library.as_deref().unwrap());
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    assert!(cache.save(&thumb, png.get_ref()));
    let key = "mosaic:test-persistent".to_string();
    let spec = format!("mosaic:{}", serde_json::to_string(&vec![&thumb]).unwrap());
    app.send(SystemAction::LoadAlbumArt(vec![(
        key.clone(),
        spec.clone(),
    )]))
    .await;
    while app.state.sources.nav_tasks.contains_key("art-grid") {
        app.receive().await;
    }
    let first = app
        .state
        .artwork
        .grid_cache
        .get(&key)
        .expect("composed mosaic")
        .clone();
    assert!(cache.save(&thumb, b"invalid cover"));
    app.state.artwork.grid_cache.remove(&key);
    app.send(SystemAction::LoadAlbumArt(vec![(key.clone(), spec)]))
        .await;
    while app.state.sources.nav_tasks.contains_key("art-grid") {
        app.receive().await;
    }
    assert_eq!(
        app.state.artwork.grid_cache[&key], first,
        "reopening must use the composed mosaic without decoding its covers again"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cached_album_destinations_open_without_waiting_for_the_server() {
    use textamp::app::sources::navidrome::commands::{Collection, CollectionKind};
    let (release, waiting) = std::sync::mpsc::channel();
    let waiting = std::sync::Mutex::new(waiting);
    let (url, requests, task) = server(move |method, params| {
        if method == "getAlbumList2" {
            waiting
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
        (200, envelope(fixture(method, params)))
    })
    .await;
    let session = session(url);
    let albums = vec![session.album(
        serde_json::from_value(json!({"id":"cached-album","name":"Cached Album"})).unwrap(),
    )];
    session
        .cache_store()
        .unwrap()
        .ticket("albums-recently-added")
        .write(&Collection::Albums(albums))
        .unwrap();
    let mut app = App::new(session.clone());
    app.send(CollectionKind::RecentlyAdded.action()).await;
    while app.state.artist_nav.loading {
        app.receive().await;
    }
    assert_eq!(
        app.state.artist_nav.columns[0].items[0].key(),
        session.key("cached-album")
    );
    assert!(app.state.artist_nav.columns[0].artwork_visible);
    assert!(app.state.sources.nav_tasks.contains_key("collection"));
    release.send(()).unwrap();
    while app.state.sources.nav_tasks.contains_key("collection") {
        app.receive().await;
    }
    assert_eq!(requests.lock().await.len(), 1);
    task.abort();
}

#[tokio::test]
async fn enhanced_lyrics_use_authenticated_post_and_follow_toggle_preserves_text() {
    let (url,requests,server)=server(|method,params| {
        assert_eq!(method,"getLyricsBySongId");
        assert_eq!(params["_verb"],"POST");assert_eq!(params["_url_query"],"");
        assert!(params["_content_type"].contains("application/x-www-form-urlencoded"));
        for (key,value) in [("id","song"),("enhanced","true"),("u","fixture"),("v","1.16.1"),("f","json")] {assert_eq!(params[key],value);}
        assert!(params.contains_key("t") && params.contains_key("s") && params.contains_key("c"));
        (200,envelope(json!({"lyricsList":{"structuredLyrics":[{"lang":"en","synced":true,"line":[{"start":0,"value":"Fixture opening"},{"start":1000,"value":"Fixture next"}]}]}})))
    }).await;
    let session = session(url);
    let mut app = App::new(session.clone());
    let track = session.track(serde_json::from_value(song("song", 1, 1)).unwrap());
    app.state.queue.tracks = vec![track];
    app.state.queue.index = Some(0);
    app.send(PlaybackAction::ShowLyrics).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        while app
            .state
            .popups
            .lyrics()
            .is_some_and(|panel| matches!(panel.content, textamp::app::lyrics::Content::Loading))
        {
            app.receive().await;
        }
    })
    .await
    .unwrap();
    app.state.playback.position_ms = 1500;
    assert_eq!(
        textamp::app::lyrics::view(&app.state).unwrap().active,
        Some(1)
    );
    app.send(PlaybackAction::ToggleLyricsSync).await;
    let view = textamp::app::lyrics::view(&app.state).unwrap();
    assert_eq!(view.active, None);
    assert_eq!(view.lines.len(), 2);
    assert_eq!(requests.lock().await.len(), 1);
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn playlist_edits_append_albums_and_preserve_duplicate_occurrences_on_server() {
    use textamp::app::sources::navidrome::{
        commands::Command,
        playlist_edit::{self, Addition, Edit},
    };
    use textamp::app::state::{BrowseColumn, PaletteCommandKind, PlayAllRow};
    let order = Arc::new(std::sync::Mutex::new(vec!["first".to_string()]));
    let stored = order.clone();
    let (url,requests,server)=server(move |method,params|{
        let mut songs=stored.lock().unwrap();
        let playlist=json!({"id":"playlist","name":"Favorites","songCount":songs.len(),"entry":songs.iter().map(|s|song(s,1,1)).collect::<Vec<_>>()});
        let result=match method{
            "getPlaylists"=>json!({"playlists":{"playlist":[playlist]}}),
            "getPlaylist"=>json!({"playlist":playlist}),
            "updatePlaylist"=>{assert_eq!(params["playlistId"],"playlist");for index in serde_json::from_str::<Vec<usize>>(&params["_removedIndices"]).unwrap(){songs.remove(index);}
                songs.extend(serde_json::from_str::<Vec<String>>(&params["_additions"]).unwrap());json!({})},
            "createPlaylist"=>{assert_eq!(params["playlistId"],"playlist");*songs=serde_json::from_str(&params["_songIds"]).unwrap();json!({})},
            _=>fixture(method,params),
        };
        (200,envelope(result))
    }).await;
    let session = session(url);
    let mut app = App::new(session.clone());
    let playlist = session.key("playlist");
    let metadata = session.playlist(session.client.playlist("playlist").await.unwrap());
    app.state.replace_playlists(vec![metadata]);
    let first = session.track(serde_json::from_value(song("first", 1, 1)).unwrap());
    let mut col = BrowseColumn::new_with_tracks(
        "Favorites",
        BrowseItem::from_tracks(std::slice::from_ref(&first)),
        vec![first],
    );
    col.play_all_row = Some(PlayAllRow::Playlist {
        rating_key: playlist.clone(),
        title: "Favorites".into(),
    });
    app.state.playlist_nav.push_column(col);
    app.state.browse_category = BrowseCategory::Playlists;
    async fn perform(app: &mut App, command: Command) {
        app.send(NavAction::Command(command)).await;
        while app.state.sources.nav_tasks.contains_key("playlist-write")
            || app.state.sources.nav_tasks.contains_key("playlist-list")
        {
            app.receive().await;
        }
    }
    for addition in [
        Addition::Track(session.key("second")),
        Addition::Album(session.key("album")),
    ] {
        app.send(NavAction::Command(Command::ChoosePlaylist(addition)))
            .await;
        assert!(app.state.palette.open);
        let entry = app
            .state
            .palette
            .entries
            .iter()
            .find(|e| e.label == "Favorites")
            .unwrap();
        let PaletteCommandKind::Navidrome(command) = entry.command.clone() else {
            panic!("Missing playlist choice")
        };
        app.state.palette.close();
        perform(&mut app, command).await;
    }
    assert_eq!(
        *order.lock().unwrap(),
        ["first", "second", "first", "second", "disc2"]
    );
    for (index, edit, expected) in [
        (
            2,
            Edit::Up,
            vec!["first", "first", "second", "second", "disc2"],
        ),
        (
            1,
            Edit::Down,
            vec!["first", "second", "first", "second", "disc2"],
        ),
        (2, Edit::Remove, vec!["first", "second", "second", "disc2"]),
    ] {
        let command =
            playlist_edit::row_command(&app.state, &app.state.playlist_nav.columns[1], index, edit)
                .unwrap();
        perform(&mut app, command).await;
        assert_eq!(*order.lock().unwrap(), expected);
        assert_eq!(
            session
                .playlist_tracks(&playlist)
                .await
                .unwrap()
                .iter()
                .map(|t| session.id(&t.rating_key).unwrap())
                .collect::<Vec<_>>(),
            expected
        );
    }
    let command = playlist_edit::row_command(
        &app.state,
        &app.state.playlist_nav.columns[1],
        1,
        Edit::Remove,
    )
    .unwrap();
    order.lock().unwrap().reverse();
    requests.lock().await.clear();
    perform(&mut app, command).await;
    assert!(
        requests
            .lock()
            .await
            .iter()
            .all(|(method, _)| method != "updatePlaylist" && method != "createPlaylist")
    );
    assert_eq!(
        app.state.playlist_nav.columns[1].tracks[0].rating_key,
        session.key("disc2")
    );
    // Select both occurrences of "second" and append them in one request.
    let col = &mut app.state.playlist_nav.columns[1];
    col.selected_set = [1, 2].into();
    col.selected_index = 0; // Keyboard focus may be outside the marked set.
    col.header_selection = textamp::app::state::ColumnHeaderSelection::Items;
    app.state.playlist_nav.focused_column = 1;
    app.state.category_column_focused = false;
    let entry = textamp::app::command_palette::materialize_entries(&app.state)
        .into_iter()
        .find(|e| e.label == "Add Selected Tracks to Playlist…")
        .expect("batch palette action");
    let PaletteCommandKind::Navidrome(Command::ChoosePlaylist(addition)) = entry.command else {
        panic!("batch selection lost")
    };
    app.state.playlist_nav.columns[1].selected_set.clear();
    perform(
        &mut app,
        Command::Append {
            playlist: playlist.clone(),
            addition,
        },
    )
    .await;
    assert_eq!(
        *order.lock().unwrap(),
        ["disc2", "second", "second", "first", "second", "second"]
    );
    let col = &mut app.state.playlist_nav.columns[1];
    col.selected_set = [1, 3].into();
    // A context click outside the selection must only target that occurrence.
    assert!(
        matches!(playlist_edit::row_command(&app.state,&app.state.playlist_nav.columns[1],2,Edit::Remove),Some(Command::EditEntry{entries,..}) if entries.len()==1 && entries[0].0==2)
    );
    for (index, edit, after, marked) in [
        (
            3,
            Edit::Up,
            vec!["second", "disc2", "first", "second", "second", "second"],
            vec![0, 2],
        ),
        (
            2,
            Edit::Down,
            vec!["disc2", "second", "second", "first", "second", "second"],
            vec![1, 3],
        ),
        (
            3,
            Edit::Remove,
            vec!["disc2", "second", "second", "second"],
            vec![],
        ),
    ] {
        let col = &app.state.playlist_nav.columns[1];
        let c = playlist_edit::row_command(&app.state, col, index, edit).unwrap();
        requests.lock().await.clear();
        perform(&mut app, c).await;
        assert_eq!(*order.lock().unwrap(), after);
        assert_eq!(
            app.state.playlist_nav.columns[1]
                .selected_set
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            marked
        );
        assert_eq!(
            requests
                .lock()
                .await
                .iter()
                .filter(|(m, _)| m == "createPlaylist" || m == "updatePlaylist")
                .count(),
            1
        );
    }
    server.abort();
}

#[tokio::test]
async fn create_new_playlist_from_chooser_keeps_selected_occurrences_and_allows_first_playlist() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use textamp::app::sources::navidrome::{commands::Command, playlist_edit::Addition};
    use textamp::app::{command_palette, handlers::key_input};
    let (url, requests, task) = server(|method, _| {
        let playlist = json!({"id":"new","name":"Road trip","songCount":3});
        let content = match method {
            "createPlaylist" => json!({"playlist":playlist}),
            "getPlaylists" => json!({"playlists":{"playlist":[playlist]}}),
            _ => panic!("unexpected call {method}"),
        };
        (200, envelope(content))
    })
    .await;
    let session = session(url);
    let mut app = App::new(session.clone());
    let addition = Addition::Tracks(
        ["second", "first", "second"]
            .map(|id| session.key(id))
            .into(),
    );
    for cancel in [true, false] {
        app.send(NavAction::Command(Command::ChoosePlaylist(
            addition.clone(),
        )))
        .await;
        assert!(app.state.palette.open);
        app.state.palette.query = "no such playlist".into();
        command_palette::refresh_matches(&mut app.state);
        assert_eq!(app.state.palette.matches.len(), 1);
        let choice = &app.state.palette.entries[app.state.palette.matches[0]];
        assert_eq!(choice.label, "Create New Playlist…");
        let command = choice.command.clone();
        app.state.palette.close();
        for action in command_palette::run(command, &mut app.state) {
            app.send(action).await;
        }
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert!(key_input::handle_key(enter, &mut app.state, &app.config).is_empty());
        let dialog = app
            .state
            .popups
            .input_dialog_mut()
            .expect("Blank names keep the form open");
        dialog.input = "  Road trip  ".into();
        let key = if cancel {
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
        } else {
            enter
        };
        for action in key_input::handle_key(key, &mut app.state, &app.config) {
            app.send(action).await;
        }
        if cancel {
            assert!(requests.lock().await.is_empty());
        }
    }
    while app.state.sources.nav_tasks.contains_key("playlist-write")
        || app.state.sources.nav_tasks.contains_key("playlist-list")
    {
        app.receive().await;
    }
    let calls = requests.lock().await;
    let mut writes = calls
        .iter()
        .filter(|(method, _)| method == "createPlaylist");
    let (_, params) = writes.next().unwrap();
    assert_eq!(params["name"], "Road trip");
    assert_eq!(params["_songIds"], r#"["second","first","second"]"#);
    assert!(writes.next().is_none());
    assert_eq!(
        app.state.library.playlists[0].rating_key,
        session.key("new")
    );
    assert!(app.state.queue.tracks.is_empty());
    task.abort();
}

#[tokio::test]
async fn create_new_playlist_from_file_creates_an_empty_server_playlist() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use textamp::app::{command_catalog, command_palette, handlers::key_input};

    let (url, requests, task) = server(|method, _| {
        let playlist = json!({"id":"empty","name":"Weekend","songCount":0});
        let content = match method {
            "createPlaylist" => json!({"playlist":playlist}),
            "getPlaylists" => json!({"playlists":{"playlist":[playlist]}}),
            _ => panic!("unexpected call {method}"),
        };
        (200, envelope(content))
    })
    .await;
    let session = session(url);
    let mut app = App::new(session.clone());
    let entry = command_palette::materialize_entries(&app.state)
        .into_iter()
        .find(|e| e.label == "Create New Playlist…")
        .unwrap();
    assert_eq!(command_catalog::path(&entry.command), ["File"]);
    for action in command_palette::run(entry.command, &mut app.state) {
        app.send(action).await;
    }
    app.state.popups.input_dialog_mut().unwrap().input = " Weekend ".into();
    for action in key_input::handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut app.state,
        &app.config,
    ) {
        app.send(action).await;
    }
    while app.state.sources.nav_tasks.contains_key("playlist-write")
        || app.state.sources.nav_tasks.contains_key("playlist-list")
    {
        app.receive().await;
    }
    let calls = requests.lock().await;
    let writes: Vec<_> = calls
        .iter()
        .filter(|(method, _)| method == "createPlaylist")
        .collect();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].1["name"], "Weekend");
    assert!(!writes[0].1.contains_key("songId"));
    assert_eq!(
        app.state.library.playlists[0].rating_key,
        session.key("empty")
    );
    task.abort();
}
