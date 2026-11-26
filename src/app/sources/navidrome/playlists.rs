//! Ordered playlist snapshots and bounded, library-scoped album-art prefetch.
use super::Session;
use crate::library::{
    cache::{REFRESH_INTERVAL, Store},
    catalog::{Album, Playlist},
    track::Track,
};
use anyhow::{Context, Result, ensure};
use futures::{StreamExt, stream};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
};
use tokio::sync::mpsc;

#[derive(serde::Serialize, serde::Deserialize)]
struct Snapshot {
    tracks: Vec<Track>,
    #[serde(default)]
    revision: Option<PlaylistRevision>,
}
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PlaylistRevision {
    updated: Option<i64>,
    count: Option<u32>,
    duration: Option<u64>,
}
impl From<&Playlist> for PlaylistRevision {
    fn from(playlist: &Playlist) -> Self {
        Self {
            updated: playlist.updated_at,
            count: playlist.leaf_count,
            duration: playlist.duration,
        }
    }
}
fn revision_matches(
    hit: &crate::library::cache::Hit<Snapshot>,
    expected: Option<&PlaylistRevision>,
) -> bool {
    expected.is_none_or(|expected| {
        hit.value.revision.as_ref().map_or_else(
            || {
                expected
                    .updated
                    .is_none_or(|updated| updated <= hit.timestamp as i64)
                    && expected
                        .count
                        .is_none_or(|count| count as usize == hit.value.tracks.len())
            },
            |revision| revision == expected,
        )
    })
}
fn warm_fingerprint(playlists: &[Playlist], albums: &[Album]) -> String {
    use std::hash::{Hash, Hasher};
    let mut playlists = playlists.iter().collect::<Vec<_>>();
    let mut albums = albums.iter().collect::<Vec<_>>();
    playlists.sort_by(|a, b| a.rating_key.cmp(&b.rating_key));
    albums.sort_by(|a, b| a.rating_key.cmp(&b.rating_key));
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_vec(&(1, playlists, albums))
        .expect("catalog metadata is serializable")
        .hash(&mut hash);
    format!("{:x}", hash.finish())
}

type Gate = tokio::sync::Mutex<()>;
static REQUESTS: OnceLock<Mutex<HashMap<String, Weak<Gate>>>> = OnceLock::new();
pub(super) fn gate(store: &Store, key: &str) -> Arc<Gate> {
    let mut requests = REQUESTS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    requests.retain(|_, gate| gate.strong_count() > 0);
    let identity = format!("{}:{key}", store.scope_key());
    if let Some(gate) = requests.get(&identity).and_then(Weak::upgrade) {
        return gate;
    }
    let gate = Arc::new(Gate::new(()));
    requests.insert(identity, Arc::downgrade(&gate));
    gate
}

async fn cached(
    ticket: crate::library::cache::Ticket,
) -> Result<Option<crate::library::cache::Hit<Snapshot>>> {
    match crate::app::tasks::spawn_blocking(move || ticket.read::<Snapshot>(REFRESH_INTERVAL))
        .await?
    {
        Ok(hit) => Ok(hit),
        Err(error) => {
            tracing::warn!("Read playlist cache: {error:#}");
            Ok(None)
        }
    }
}

/// Open the ordered playlist and retain its last complete snapshot on failure.
pub(super) async fn tracks(
    session: &Session,
    key: &str,
    playlist: Option<&Playlist>,
) -> Result<Vec<Track>> {
    load_revision(
        session,
        key,
        PlaylistLoad::Browse,
        playlist.map(PlaylistRevision::from),
    )
    .await
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum PlaylistLoad {
    Browse,
    Warm,
    Refresh,
}
async fn load_revision(
    session: &Session,
    key: &str,
    mode: PlaylistLoad,
    revision: Option<PlaylistRevision>,
) -> Result<Vec<Track>> {
    let id = session.id(key)?;
    let store = session.cache_store()?;
    let ticket = store.ticket(&format!("playlist:{id}"));
    let usable = |hit: &crate::library::cache::Hit<Snapshot>| {
        let confirmed = revision
            .as_ref()
            .is_some_and(|rev| rev.updated.is_some() && hit.value.revision.as_ref() == Some(rev));
        (!hit.stale || confirmed) && revision_matches(hit, revision.as_ref())
    };
    let gate = gate(&store, &id);
    // A fresh snapshot stays readable during a slow refresh or edit.
    let _guard = match gate.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            if mode != PlaylistLoad::Refresh
                && let Some(hit) = cached(ticket.clone()).await?
                && usable(&hit)
            {
                return Ok(hit.value.tracks);
            }
            gate.lock().await
        }
    };
    let previous = if mode == PlaylistLoad::Refresh {
        None
    } else {
        match cached(ticket.clone()).await? {
            Some(hit) if usable(&hit) => return Ok(hit.value.tracks),
            hit if mode == PlaylistLoad::Browse => hit,
            _ => None,
        }
    };
    let playlist = match session.client.playlist(&id).await {
        Ok(playlist) => playlist,
        Err(error) => match previous {
            Some(hit) => {
                tracing::warn!(%error, "Playlist refresh failed; using cached contents");
                return Ok(hit.value.tracks);
            }
            None => return Err(error),
        },
    };
    ensure!(playlist.id == id, "Server returned another playlist");
    ensure!(
        playlist
            .song_count
            .is_none_or(|n| n as usize == playlist.entry.len()),
        "Playlist contents are incomplete; cached playlist retained"
    );
    let snapshot = Snapshot {
        revision,
        tracks: playlist
            .entry
            .into_iter()
            .map(|song| session.track(song))
            .collect(),
    };
    let snapshot = crate::app::tasks::spawn_blocking(move || {
        if let Err(error) = ticket.write(&snapshot) {
            if mode != PlaylistLoad::Browse {
                return Err(error.context("Save playlist cache"));
            }
            tracing::warn!("Save playlist cache: {error:#}");
        }
        Ok::<_, anyhow::Error>(snapshot)
    })
    .await??;
    Ok(snapshot.tracks)
}

pub(super) async fn save_edited(
    ticket: crate::library::cache::Ticket,
    playlist: &Playlist,
    tracks: &[Track],
) -> Result<()> {
    let snapshot = Snapshot {
        revision: Some(PlaylistRevision::from(playlist)),
        tracks: tracks.to_vec(),
    };
    crate::app::tasks::spawn_blocking(move || ticket.write(&snapshot)).await??;
    Ok(())
}

pub(crate) async fn warm(
    session: &Session,
    library_key: &str,
    playlists: Arc<Vec<Playlist>>,
    albums: Arc<Vec<Album>>,
    force: bool,
) -> Result<()> {
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicBool, Ordering};
    let cache = crate::media::ArtworkCache::for_library(library_key);
    let fingerprint = {
        let playlists = playlists.clone();
        let albums = albums.clone();
        crate::app::tasks::spawn_blocking(move || warm_fingerprint(&playlists, &albums)).await?
    };
    let store = session.cache_store()?;
    let warm_ticket = store.ticket("playlist-artwork-warm");
    let read_ticket = warm_ticket.clone();
    let check = cache.clone();
    let expected = fingerprint.clone();
    let snapshot_lists = playlists.clone();
    let scope = session.clone();
    if !force
        && crate::app::tasks::spawn_blocking(move || {
            check.warm_summary_matches(&expected)
                && read_ticket
                    .read::<String>(REFRESH_INTERVAL)
                    .ok()
                    .flatten()
                    .is_some_and(|hit| !hit.stale && hit.value == expected)
                && snapshot_lists.iter().all(|list| {
                    scope
                        .id(&list.rating_key)
                        .is_ok_and(|id| store.ticket(&format!("playlist:{id}")).exists())
                })
        })
        .await?
    {
        return Ok(());
    }
    let reference_revision = cache.begin_refresh();
    let prune = cache.clone();
    crate::app::tasks::spawn_blocking(move || prune.prune_to_size(crate::media::ARTWORK_LIMIT))
        .await?;
    let known: HashMap<_, _> = albums
        .iter()
        .map(|a| (a.rating_key.clone(), a.thumb.clone()))
        .collect();
    let catalog_refs: HashSet<_> = known
        .iter()
        .flat_map(|(key, thumb)| std::iter::once(key.clone()).chain(thumb.clone()))
        .collect();
    let catalog_covers: Vec<_> = known
        .iter()
        .map(|(key, thumb)| (key.clone(), thumb.clone()))
        .collect();
    // Queue each playlist album once so slow images cannot block playlist metadata.
    let (covers_tx, covers_rx) = mpsc::unbounded_channel();
    let playlist_work = async move {
        let mut issue = None;
        let mut references = HashSet::new();
        let mut lists = stream::iter(playlists.iter().cloned())
            .map(|p| async move {
                let revision = PlaylistRevision::from(&p);
                let mode = if force {
                    PlaylistLoad::Refresh
                } else {
                    PlaylistLoad::Warm
                };
                load_revision(session, &p.rating_key, mode, Some(revision)).await
            })
            .buffer_unordered(8);
        while let Some(result) = lists.next().await {
            match result {
                Ok(tracks) => {
                    for track in tracks {
                        if let Some(album) = track.parent_rating_key {
                            let thumb = known
                                .get(&album)
                                .cloned()
                                .flatten()
                                .or(track.parent_thumb)
                                .or(track.thumb);
                            let fresh = references.insert(album.clone());
                            references.extend(thumb.iter().cloned());
                            if fresh {
                                let _ = covers_tx.send((album, thumb));
                            }
                        }
                    }
                }
                Err(error) => {
                    issue.get_or_insert(error);
                }
            }
        }
        (issue, references)
    };
    let artwork_store = session.cache_store()?;
    let full = AtomicBool::new(false);
    let artwork_work = async {
        let mut issue = None;
        let mut seen = HashSet::new();
        let playlist_covers = stream::unfold(covers_rx, |mut receiver| async move {
            receiver.recv().await.map(|cover| (cover, receiver))
        });
        let covers = stream::select_with_strategy(
            Box::pin(playlist_covers),
            stream::iter(catalog_covers),
            |_: &mut ()| futures::stream::PollNext::Left,
        )
        .filter_map(move |(key, hint)| {
            futures::future::ready(seen.insert(key.clone()).then_some((key, hint)))
        });
        let downloads = covers
            .map(|(key, hint)| {
                let cache = cache.clone();
                let full = &full;
                let artwork_store = &artwork_store;
                async move {
                    let read = cache.clone();
                    let album_key = key.clone();
                    let expected_cover = hint.clone();
                    if crate::app::tasks::spawn_blocking(move || {
                        read.contains_cover(&album_key, expected_cover.as_deref())
                    })
                    .await?
                        || full.load(Ordering::Relaxed)
                    {
                        return Ok(());
                    }
                    let thumb = if let Some(thumb) = hint {
                        thumb
                    } else {
                        let album = session.client.album(&session.id(&key)?).await?;
                        album
                            .cover_art
                            .map(|id| session.key(&id))
                            .unwrap_or_else(|| key.clone())
                    };
                    let gate = gate(artwork_store, &format!("artwork:{thumb}"));
                    let _permit = gate.lock().await;
                    if full.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    let read = cache.clone();
                    let cover_key = thumb.clone();
                    let album_key = key.clone();
                    if crate::app::tasks::spawn_blocking(move || {
                        if !read.contains(&cover_key) {
                            return Ok(false);
                        }
                        read.alias(&album_key, &cover_key)?;
                        Ok::<_, anyhow::Error>(true)
                    })
                    .await??
                    {
                        return Ok(());
                    }
                    let data = super::effects::media::download_cover(session, &thumb).await?;
                    let stored = crate::app::tasks::spawn_blocking(move || {
                        if !cache.prefetch(&thumb, &data)? {
                            return Ok(false);
                        }
                        cache.alias(&key, &thumb)?;
                        Ok::<_, anyhow::Error>(true)
                    })
                    .await??;
                    if !stored {
                        full.store(true, Ordering::Relaxed);
                    }
                    Ok::<_, anyhow::Error>(())
                }
            })
            .buffer_unordered(4);
        futures::pin_mut!(downloads);
        while let Some(result) = downloads.next().await {
            if let Err(error) = result {
                issue.get_or_insert(error);
            }
        }
        issue
    };
    let ((playlist_issue, playlist_refs), artwork_issue) =
        tokio::join!(playlist_work, artwork_work);
    if let Some(issue) = playlist_issue.or(artwork_issue) {
        return Err(issue).context("Cache playlists and album covers");
    }
    crate::app::tasks::spawn_blocking(move || {
        cache.reconcile_playlists(&catalog_refs, &playlist_refs, reference_revision)?;
        cache.finish_warm_summary(&fingerprint, reference_revision)?;
        warm_ticket.write(&fingerprint)
    })
    .await??;
    Ok(())
}

#[cfg(test)]
mod pipeline_tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn all_playlists_cache_while_artwork_is_stalled() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let images = Arc::new(tokio::sync::Semaphore::new(0));
        let downloads = images.clone();
        let server = tokio::spawn(async move {
            let mut replies = tokio::task::JoinSet::new();
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let images = downloads.clone();
                replies.spawn(async move {
                    let mut request = vec![0; 8192];
                    let length = socket.read(&mut request).await.unwrap();
                    let request = String::from_utf8_lossy(&request[..length]);
                    let path = request.split_whitespace().nth(1).unwrap();
                    let url = reqwest::Url::parse(&format!("http://fixture{path}")).unwrap();
                    let id = url.query_pairs().find(|(key, _)| key == "id").unwrap().1.into_owned();
                    let (kind, body) = match url.path() {
                        "/rest/getPlaylist" => ("application/json", json!({"subsonic-response":{"status":"ok","playlist":{
                            "id":id,"name":id,"songCount":1,"entry":[{"id":format!("song-{id}"),"title":"Song","albumId":format!("album-{id}"),"album":"Album","artist":"Artist","coverArt":format!("cover-{id}")}]
                        }}}).to_string().into_bytes()),
                        "/rest/getCoverArt" => {
                            images.acquire().await.unwrap().forget();
                            let image = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
                            let mut output = std::io::Cursor::new(Vec::new());
                            image.write_to(&mut output, image::ImageFormat::Png).unwrap();
                            ("image/png", output.into_inner())
                        }
                        other => panic!("Unexpected request {other}"),
                    };
                    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
                    socket.write_all(&body).await.unwrap();
                });
            }
        });
        let source = crate::navidrome::Source {
            id: format!("metadata-first-{}", address.port()),
            name: "Fixture".into(),
            url: format!("http://{address}"),
            username: "fixture".into(),
            libraries: vec![],
        };
        let session = Session {
            client: crate::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
            source,
            extensions: Default::default(),
        };
        let playlists: Vec<_> = (0..32)
            .map(|index| Playlist {
                rating_key: session.key(&index.to_string()),
                title: format!("List {index}"),
                key: session.key(&index.to_string()),
                playlist_type: "audio".into(),
                composite: None,
                duration: None,
                leaf_count: Some(1),
                added_at: None,
                updated_at: None,
                smart: false,
            })
            .collect();
        let worker = session.clone();
        let fill = tokio::spawn(async move {
            warm(
                &worker,
                &worker.source.id,
                playlists.into(),
                vec![].into(),
                false,
            )
            .await
        });
        let store = session.cache_store().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if (0..32).all(|index| {
                    store
                        .ticket(&format!("playlist:{index}"))
                        .read::<Snapshot>(REFRESH_INTERVAL)
                        .unwrap()
                        .is_some()
                }) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Every playlist must persist before artwork can finish");
        assert!(!fill.is_finished());
        images.add_permits(32);
        fill.await.unwrap().unwrap();
        server.abort();
        for index in 0..32 {
            let tracks = tracks(&session, &session.key(&index.to_string()), None)
                .await
                .unwrap();
            assert_eq!(tracks[0].rating_key, session.key(&format!("song-{index}")));
        }
    }

    #[tokio::test]
    async fn artwork_warming_overlaps_slow_playlist_downloads() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let cover_started = Arc::new(tokio::sync::Semaphore::new(0));
        let cover_count = Arc::new(AtomicUsize::new(0));
        let gate = cover_started.clone();
        let count = cover_count.clone();
        let server = tokio::spawn(async move {
            let mut replies = tokio::task::JoinSet::new();
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let gate = gate.clone();
                let count = count.clone();
                replies.spawn(async move {
                    let mut request = vec![0; 8192];
                    let length = socket.read(&mut request).await.unwrap();
                    let request = String::from_utf8_lossy(&request[..length]);
                    let path = request.split_whitespace().nth(1).unwrap();
                    let url = reqwest::Url::parse(&format!("http://fixture{path}")).unwrap();
                    let parameters: HashMap<_, _> = url.query_pairs().into_owned().collect();
                    let (kind, body) = match url.path() {
                        "/rest/getPlaylist" => {
                            if parameters["id"] == "slow" { gate.acquire().await.unwrap().forget(); }
                            ("application/json", json!({"subsonic-response":{"status":"ok","playlist":{
                                "id":parameters["id"],"name":"Playlist","songCount":if parameters["id"] == "broken" { 2 } else { 1 },
                                "entry":[{"id":"song","title":"Song","albumId":"album","album":"Album","artist":"Artist"}]
                            }}}).to_string().into_bytes())
                        }
                        "/rest/getCoverArt" => {
                            count.fetch_add(1, Ordering::SeqCst);
                            gate.add_permits(1);
                            let image = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
                            let mut output = std::io::Cursor::new(Vec::new());
                            image.write_to(&mut output, image::ImageFormat::Png).unwrap();
                            ("image/png", output.into_inner())
                        }
                        other => panic!("Unexpected fixture request {other}"),
                    };
                    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
                    socket.write_all(&body).await.unwrap();
                });
            }
        });
        let source = crate::navidrome::Source {
            id: format!("pipeline-{}", address.port()),
            name: "Fixture".into(),
            url: format!("http://{address}"),
            username: "fixture".into(),
            libraries: vec![],
        };
        let session = Session {
            client: crate::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
            source,
            extensions: Default::default(),
        };
        let playlists: Vec<_> = ["fast", "slow"]
            .into_iter()
            .map(|id| Playlist {
                rating_key: session.key(id),
                title: id.into(),
                key: session.key(id),
                playlist_type: "audio".into(),
                composite: None,
                duration: None,
                leaf_count: Some(1),
                added_at: None,
                updated_at: None,
                smart: false,
            })
            .collect();
        let albums = vec![
            Album {
                rating_key: session.key("album"),
                thumb: Some(session.key("cover")),
                ..Default::default()
            },
            Album {
                rating_key: session.key("catalog-only"),
                thumb: Some(session.key("catalog-only-cover")),
                ..Default::default()
            },
        ];
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            warm(
                &session,
                &format!("pipeline-{}", address.port()),
                playlists.clone().into(),
                albums.clone().into(),
                true,
            ),
        )
        .await;
        result
            .expect("Artwork must not wait for every playlist")
            .unwrap();
        assert_eq!(
            cover_count.load(Ordering::SeqCst),
            2,
            "The full catalog is warmed and playlist duplicates share images"
        );
        warm(
            &session,
            &format!("pipeline-{}", address.port()),
            playlists.clone().into(),
            albums.clone().into(),
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            cover_count.load(Ordering::SeqCst),
            2,
            "A warm cache requires no artwork download"
        );
        let library_key = format!("pipeline-{}", address.port());
        let cache = crate::media::ArtworkCache::for_library(&library_key);
        assert!(cache.save("orphan", b"playlist-only"));
        cache
            .reconcile_playlists(
                &Default::default(),
                &["orphan".into()].into(),
                cache.begin_refresh(),
            )
            .unwrap();
        let mut broken = playlists[0].clone();
        broken.rating_key = session.key("broken");
        assert!(
            warm(
                &session,
                &library_key,
                vec![broken].into(),
                albums.clone().into(),
                true
            )
            .await
            .is_err()
        );
        assert!(
            cache.contains("orphan"),
            "A partial refresh must not remove playlist artwork"
        );
        warm(&session, &library_key, vec![].into(), albums.into(), false)
            .await
            .unwrap();
        assert!(!cache.contains("orphan"));
        assert!(cache.contains(&session.key("catalog-only")));
        server.abort();
    }
}

#[cfg(test)]
mod freshness_tests {
    use super::*;
    #[tokio::test]
    async fn completed_warm_skips_traversal_until_either_cache_is_cleared() {
        let library = uuid::Uuid::new_v4().to_string();
        let source = crate::navidrome::Source {
            id: library.clone(),
            name: "Fixture".into(),
            url: "http://127.0.0.1:1".into(),
            username: "fixture".into(),
            libraries: vec![],
        };
        let session = Session {
            client: crate::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
            source,
            extensions: Default::default(),
        };
        warm(&session, &library, vec![].into(), vec![].into(), false)
            .await
            .unwrap();
        let ticket = session
            .cache_store()
            .unwrap()
            .ticket("playlist-artwork-warm");
        let fingerprint = ticket
            .read::<String>(REFRESH_INTERVAL)
            .unwrap()
            .unwrap()
            .value;
        let earlier = crate::library::cache::now() - 60;
        ticket.write_at(&fingerprint, earlier).unwrap();
        warm(&session, &library, vec![].into(), vec![].into(), false)
            .await
            .unwrap();
        assert_eq!(
            ticket
                .read::<String>(REFRESH_INTERVAL)
                .unwrap()
                .unwrap()
                .timestamp,
            earlier
        );
        let artwork = crate::media::ArtworkCache::for_library(&library);
        artwork.clear_all();
        warm(&session, &library, vec![].into(), vec![].into(), false)
            .await
            .unwrap();
        assert!(
            ticket
                .read::<String>(REFRESH_INTERVAL)
                .unwrap()
                .unwrap()
                .timestamp
                > earlier
        );
        session.cache_store().unwrap().clear().unwrap();
        warm(&session, &library, vec![].into(), vec![].into(), false)
            .await
            .unwrap();
        assert_eq!(
            ticket
                .read::<String>(REFRESH_INTERVAL)
                .unwrap()
                .unwrap()
                .value,
            fingerprint
        );
        assert!(artwork.warm_summary_matches(&fingerprint));
    }
    #[test]
    fn unchanged_track_count_does_not_hide_a_new_playlist_revision() {
        let hit = crate::library::cache::Hit {
            value: Snapshot {
                tracks: vec![],
                revision: Some(PlaylistRevision {
                    updated: Some(10),
                    count: Some(0),
                    duration: Some(0),
                }),
            },
            timestamp: 20,
            stale: false,
        };
        assert!(revision_matches(
            &hit,
            Some(&PlaylistRevision {
                updated: Some(10),
                count: Some(0),
                duration: Some(0)
            })
        ));
        assert!(!revision_matches(
            &hit,
            Some(&PlaylistRevision {
                updated: Some(11),
                count: Some(0),
                duration: Some(0)
            })
        ));
    }
}
