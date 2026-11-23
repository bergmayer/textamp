//! One analysis adapter for radio, remixes and discovery. Navidrome owns
//! identity/playback; AudioMuse can only recommend IDs in the selected catalog.
use super::{Session, effects::discovery};
use crate::app::{AppState, sources::audiomuse};
use crate::library::track::Track;
use anyhow::{Context, Result, ensure};
use futures::{StreamExt, TryStreamExt, stream};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::OnceCell;

pub fn available(state: &AppState) -> bool {
    crate::app::sources::sonic::enabled(state)
        && state.sources.active.navidrome().is_some_and(|s| {
            s.extensions.contains("sonicSimilarity") || audiomuse::connection(state).is_some()
        })
}

pub struct Recommendations {
    pub session: Session,
    sonic_allowed: bool,
    binding: Option<(crate::audiomuse::Connection, String)>,
    direct: OnceCell<Result<crate::audiomuse::Client, String>>,
    catalog: std::sync::Arc<Vec<Track>>,
    catalog_indices: OnceLock<HashMap<String, Option<usize>>>,
}
impl Recommendations {
    pub fn new(state: &AppState, session: &Session) -> Self {
        Self {
            session: session.clone(),
            sonic_allowed: crate::app::sources::sonic::enabled(state),
            binding: crate::app::sources::sonic::enabled(state)
                .then(|| {
                    audiomuse::connection(state)
                        .cloned()
                        .map(|c| (c, audiomuse::key(session)))
                })
                .flatten(),
            direct: OnceCell::new(),
            catalog: state.library.all_tracks.clone(),
            catalog_indices: OnceLock::new(),
        }
    }

    fn catalog_indices(&self) -> &HashMap<String, Option<usize>> {
        self.catalog_indices.get_or_init(|| {
            self.catalog
                .iter()
                .enumerate()
                .filter_map(|(index, track)| {
                    self.session
                        .id(&track.rating_key)
                        .ok()
                        .map(|id| (id, Some(index)))
                })
                .collect()
        })
    }
    pub(super) async fn direct(&self) -> Result<&crate::audiomuse::Client> {
        let result = self.direct.get_or_init(|| async {
            let result: Result<_> = async {
            let (connection, key) = self.binding.as_ref()
                .context("Connect AudioMuse in Settings → Libraries, or enable Navidrome's sonicSimilarity plugin")?;
            let api = audiomuse::client(connection, key).await?;
            api.verify().await?;
            Ok(api)
            }.await;
            result.map_err(|e| format!("{e:#}"))
        }).await;
        result
            .as_ref()
            .map_err(|message| anyhow::anyhow!(message.clone()))
    }
    fn scoped(&self, tracks: Vec<Track>) -> Vec<Track> {
        let mut seen = HashSet::new();
        tracks
            .into_iter()
            .filter(|t| {
                self.session
                    .id(&t.rating_key)
                    .is_ok_and(|id| self.catalog_indices().contains_key(&id))
                    && seen.insert(t.rating_key.clone())
            })
            .collect()
    }
    async fn resolve(&self, ids: Vec<String>, path: bool) -> Result<Vec<Track>> {
        if path {
            ensure!(
                ids.iter().all(|id| self.catalog_indices().contains_key(id))
                    && ids.iter().collect::<HashSet<_>>().len() == ids.len(),
                "Sonic path leaves this library or repeats tracks"
            );
        }
        let mut seen = HashSet::new();
        let ids: Vec<_> = ids
            .into_iter()
            .filter(|id| self.catalog_indices().contains_key(id) && seen.insert(id.clone()))
            .collect();
        stream::iter(ids)
            .map(|id| {
                let cached = self
                    .catalog_indices()
                    .get(&id)
                    .and_then(|index| *index)
                    .and_then(|index| self.catalog.get(index));
                async move {
                    if let Some(track) = cached {
                        return Ok(track.clone());
                    }
                    let song = self.session.client.song(&id).await?;
                    ensure!(
                        song.id == id,
                        "Navidrome returned the wrong recommended track"
                    );
                    Ok(self.session.track(song))
                }
            })
            .buffered(4)
            .try_collect()
            .await
    }
    pub async fn similar(&self, id: &str, sonic: bool) -> Result<Vec<Track>> {
        tokio::time::timeout(Duration::from_secs(30), async {
            if !sonic {
                return discovery::similar(&self.session, id, false)
                    .await
                    .map(|t| self.scoped(t));
            }
            ensure!(
                self.sonic_allowed,
                "Sonic features are disabled for this library"
            );
            ensure!(
                self.catalog_indices().contains_key(id),
                "Sonic seed is outside this library"
            );
            let native = if self.session.extensions.contains("sonicSimilarity") {
                match tokio::time::timeout(
                    Duration::from_secs(8),
                    discovery::similar(&self.session, id, true),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(anyhow::anyhow!("Navidrome sonic request timed out")),
                }
            } else {
                Err(anyhow::anyhow!(
                    "Navidrome sonicSimilarity extension unavailable"
                ))
            };
            let native = native.map(|t| {
                self.scoped(t)
                    .into_iter()
                    .filter(|t| t.rating_key != self.session.key(id))
                    .collect::<Vec<_>>()
            });
            if let Ok(tracks) = &native
                && !tracks.is_empty()
            {
                return native;
            }
            if self.binding.is_none() && self.direct.get().is_none() {
                return native;
            }
            if let Err(error) = &native {
                tracing::debug!(
                    "Navidrome sonic lookup failed; trying direct AudioMuse: {error:#}"
                );
            }
            async {
                let ids = self.direct().await?.similar(id).await?;
                let tracks = self
                    .resolve(ids.into_iter().filter(|t| t != id).collect(), false)
                    .await?;
                ensure!(
                    !tracks.is_empty(),
                    "No analyzed matches yet in this library"
                );
                Ok(tracks)
            }
            .await
        })
        .await
        .context("Analysis request timed out")?
    }
    pub(super) fn ranked_candidate(&self, id: &str) -> Option<&Track> {
        self.catalog.get((*self.catalog_indices().get(id)?)?)
    }

    pub async fn mood(&self, mood: &str) -> Result<Vec<Track>> {
        tokio::time::timeout(Duration::from_secs(30), async {
            ensure!(
                self.sonic_allowed,
                "Sonic features are disabled for this library"
            );
            let ids = self.direct().await?.mood(mood).await?;
            let tracks = self.resolve(ids, false).await?;
            ensure!(
                !tracks.is_empty(),
                "No analyzed mood matches yet in this library"
            );
            Ok(tracks)
        })
        .await
        .context("Analysis request timed out")?
    }
    /// Artist affinity prefers the server's artist metadata. If it cannot
    /// supply recommendations, an analyzed track is an explicit sonic seed.
    pub async fn artist(&self, artist: &str, seed: Option<&str>) -> Result<Vec<Track>> {
        let native = self.similar(artist, false).await;
        if native.as_ref().is_ok_and(|tracks| !tracks.is_empty()) {
            return native;
        }
        if let Some(seed) = seed.filter(|_| {
            self.sonic_allowed
                && (self.binding.is_some()
                    || self.direct.initialized()
                    || self.session.extensions.contains("sonicSimilarity"))
        }) {
            return self.similar(seed, true).await;
        }
        native
    }
    pub async fn path(&self, start: &str, end: &str, count: usize) -> Result<Vec<Track>> {
        tokio::time::timeout(Duration::from_secs(30), async {
            ensure!(
                self.sonic_allowed,
                "Sonic features are disabled for this library"
            );
            ensure!(
                start != end
                    && self.catalog_indices().contains_key(start)
                    && self.catalog_indices().contains_key(end),
                "Choose distinct sonic path endpoints in this library"
            );
            let native = async {
                ensure!(
                    self.session.extensions.contains("sonicSimilarity"),
                    "Navidrome sonic paths unavailable"
                );
                let value = self
                    .session
                    .client
                    .call(
                        "findSonicPath",
                        &[
                            ("startSongId", start.into()),
                            ("endSongId", end.into()),
                            ("count", count.to_string()),
                        ],
                    )
                    .await?;
                let tracks = discovery::sonic_matches(&value)?
                    .into_iter()
                    .map(|s| self.session.track(s))
                    .collect();
                self.validate_path(tracks, start, end, count)
            };
            let native = match tokio::time::timeout(Duration::from_secs(8), native).await {
                Ok(result) => result,
                Err(_) => Err(anyhow::anyhow!("Navidrome sonic path timed out")),
            };
            if native.is_ok() {
                return native;
            }
            async {
                let ids = self.direct().await?.path(start, end, count).await?;
                let tracks = self.resolve(ids, true).await?;
                self.validate_path(tracks, start, end, count)
            }
            .await
            .with_context(|| native.unwrap_err().to_string())
        })
        .await
        .context("Analysis request timed out")?
    }
    fn validate_path(
        &self,
        tracks: Vec<Track>,
        start: &str,
        end: &str,
        count: usize,
    ) -> Result<Vec<Track>> {
        let keys: HashSet<_> = tracks.iter().map(|t| &t.rating_key).collect();
        ensure!(
            tracks.len() >= 2
                && tracks.len() <= count
                && keys.len() == tracks.len()
                && tracks
                    .first()
                    .is_some_and(|t| t.rating_key == self.session.key(start))
                && tracks
                    .last()
                    .is_some_and(|t| t.rating_key == self.session.key(end))
                && tracks.iter().all(|t| self
                    .session
                    .id(&t.rating_key)
                    .is_ok_and(|id| self.catalog_indices().contains_key(&id))),
            "Server returned an invalid sonic path"
        );
        Ok(tracks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn cached_recommendations_need_no_network_and_keep_scope_and_order() {
        let source = crate::navidrome::Source {
            id: "fixture".into(),
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
        let mut state = AppState::new();
        state.library.all_tracks = std::sync::Arc::new(
            ["a", "b"]
                .map(|id| Track {
                    rating_key: session.key(id),
                    title: id.to_uppercase(),
                    ..Default::default()
                })
                .into(),
        );
        let api = Recommendations::new(&state, &session);
        let tracks = api
            .resolve(["b", "foreign", "b", "a"].map(str::to_owned).into(), false)
            .await
            .unwrap();
        assert_eq!(
            tracks
                .iter()
                .map(|track| track.title.as_str())
                .collect::<Vec<_>>(),
            ["B", "A"]
        );
    }

    #[tokio::test]
    async fn analysis_requests_preserve_library_scope_and_ranking_pages() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let invalid = Arc::new(AtomicBool::new(false));
        let check = invalid.clone();
        let refreshed = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let check = check.clone();
                let refreshed = refreshed.clone();
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    let mut chunk = [0; 4096];
                    let body_start = loop {
                        let n = socket.read(&mut chunk).await.unwrap();
                        if n == 0 {
                            return;
                        }
                        bytes.extend_from_slice(&chunk[..n]);
                        if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                            break i + 4;
                        }
                    };
                    let text = String::from_utf8(bytes[..body_start].to_vec()).unwrap();
                    let length = text
                        .lines()
                        .filter_map(|line| line.split_once(':'))
                        .find(|(key, _)| key.eq_ignore_ascii_case("Content-Length"))
                        .map_or(0, |(_, len)| len.trim().parse::<usize>().unwrap());
                    while bytes.len() < body_start + length {
                        let n = socket.read(&mut chunk).await.unwrap();
                        assert_ne!(n, 0, "Incomplete test request");
                        bytes.extend_from_slice(&chunk[..n]);
                    }
                    let url = reqwest::Url::parse(&format!(
                        "http://local{}",
                        text.split_whitespace().nth(1).unwrap()
                    ))
                    .unwrap();
                    let p: std::collections::HashMap<_, _> = url.query_pairs().collect();
                    let mut status = 200;
                    let value = match url.path() {
                        "/auth" => json!({}),
                        "/api/similar_tracks" => {
                            assert_eq!(p["server"], "fixture");
                            assert!(text.starts_with("GET "));
                            json!([{"item_id":"seed"},{"item_id":"match"},{"item_id":"outside"},{"item_id":"match"}])
                        }
                        "/api/find_path" => {
                            assert_eq!(p["server"], "fixture");
                            let middle = if check.load(Ordering::Relaxed) {
                                "outside"
                            } else {
                                "match"
                            };
                            json!({"path":[{"item_id":"seed"},{"item_id":middle},{"item_id":"end"}]})
                        }
                        "/api/servers" => {
                            json!({"servers":[{"name":"fixture","server_id":"registry-id","server_type":"navidrome"}]})
                        }
                        "/api/sync" => {
                            json!({"total_tracks":0,"tracks":[],"provider_type":"navidrome","has_more":false,"next_page":null})
                        }
                        "/plugins/distant_rank/v1/rank" => {
                            assert!(text.starts_with("POST "));
                            assert!(text.contains("audiomuse_jwt=fixture"));
                            let request: serde_json::Value =
                                serde_json::from_slice(&bytes[body_start..]).unwrap();
                            assert_eq!(request["server_id"], "registry-id");
                            assert_eq!(request["queue"], json!(["seed", "seed", "end"]));
                            assert_eq!(request["music_folder_id"], "test-folder");
                            let offset = request["offset"].as_u64().unwrap();
                            let mut response = json!({
                                "server_id": "registry-id", "mode": request["mode"], "offset": offset,
                                "generation": "catalog-a",
                                "scope": {"server_enforced": true, "folder_enforced": false, "music_folder_id": "test-folder"}
                            });
                            match offset {
                                0 => {
                                    assert!(request.get("generation").is_none());
                                    if request["mode"] == "whole_queue" {
                                        response["candidates"] =
                                            json!([{"id":"outside"},{"id":"seed"},{"id":"match"}]);
                                        response["next_offset"] = json!(3);
                                    } else {
                                        response["positions"] = json!([
                                            {"position":0,"id":"seed","candidates":[{"id":"outside"},{"id":"match"}],"next_offset":2},
                                            {"position":1,"id":"seed","candidates":[{"id":"outside"},{"id":"match"}],"next_offset":2},
                                            {"position":2,"id":"end","candidates":[],"error":{"code":"MISSING_ANALYSIS"}}
                                        ]);
                                    }
                                }
                                2 | 3 => {
                                    assert_eq!(request["generation"], "catalog-a");
                                    if request["mode"] == "whole_queue" {
                                        if refreshed.swap(true, Ordering::Relaxed) {
                                            response["candidates"] = json!([{"id":"other"}]);
                                        } else {
                                            status = 409;
                                            response = json!({"error":{"code":"INDEX_CHANGED"}});
                                        }
                                    } else {
                                        response["positions"] = json!([
                                            {"position":0,"id":"seed","candidates":[{"id":"other"}]},
                                            {"position":1,"id":"seed","candidates":[{"id":"other"}]},
                                            {"position":2,"id":"end","candidates":[],"error":{"code":"MISSING_ANALYSIS"}}
                                        ]);
                                    }
                                }
                                other => panic!("unexpected rank offset {other}"),
                            }
                            response
                        }
                        "/rest/getSong" => {
                            let id = &p["id"];
                            assert_ne!(id, "outside", "Never fetch another library's song");
                            json!({"subsonic-response":{"status":"ok","song":{"id":id,"title":id,"artist":format!("Artist {id}")}}})
                        }
                        "/rest/getSonicSimilarTracks" | "/rest/findSonicPath" => {
                            json!({"subsonic-response":{"status":"failed","error":{"code":70,"message":"index not ready"}}})
                        }
                        other => panic!("unexpected request {other}"),
                    };
                    let body = value.to_string();
                    let reply = format!(
                        "HTTP/1.1 {status} Test\r\nSet-Cookie: audiomuse_jwt=fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(reply.as_bytes()).await;
                });
            }
        });
        let source = crate::navidrome::Source {
            id: "test".into(),
            name: "test".into(),
            url: url.clone(),
            username: "test".into(),
            libraries: vec![],
        };
        let session = Session {
            client: crate::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
            source,
            extensions: HashSet::new(),
        };
        let connection = crate::audiomuse::Connection {
            url,
            username: "test".into(),
            server: "fixture".into(),
        };
        let direct = crate::audiomuse::Client::connect(&connection, "fixture")
            .await
            .unwrap();
        let mut api = Recommendations {
            session,
            sonic_allowed: true,
            binding: None,
            direct: OnceCell::new(),
            catalog: Default::default(),
            catalog_indices: OnceLock::from(
                ["seed", "match", "end"]
                    .into_iter()
                    .map(|id| (id.to_owned(), None))
                    .collect::<HashMap<_, _>>(),
            ),
        };
        assert!(api.direct.set(Ok(direct)).is_ok()); // Inject a fixture session, never write credentials.
        for plugin in [false, true] {
            if plugin {
                api.session.extensions.insert("sonicSimilarity".into());
            }
            let tracks = api.similar("seed", true).await.unwrap();
            assert_eq!(tracks.len(), 1);
            assert_eq!(tracks[0].rating_key, api.session.key("match"));
            let tracks = api.path("seed", "end", 5).await.unwrap();
            assert_eq!(
                tracks.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
                ["seed", "match", "end"]
            );
        }
        api.sonic_allowed = false;
        assert!(
            api.similar("seed", true)
                .await
                .unwrap_err()
                .to_string()
                .contains("disabled")
        );
        assert!(
            api.path("seed", "end", 5)
                .await
                .unwrap_err()
                .to_string()
                .contains("disabled")
        );
        api.sonic_allowed = true;
        invalid.store(true, Ordering::Relaxed);
        assert!(
            format!("{:#}", api.path("seed", "end", 5).await.unwrap_err())
                .contains("leaves this library")
        );
        assert!(api.similar("outside", true).await.is_err());
        api.session.client.folder = Some("test-folder".into());
        api.catalog = Arc::new(
            ["seed", "match", "other", "end"]
                .map(|id| Track {
                    title: id.into(),
                    rating_key: api.session.key(id),
                    ..Default::default()
                })
                .into(),
        );
        api.catalog_indices = OnceLock::new();
        let ids = ["seed", "seed", "end"].map(str::to_owned);
        for (mode, expected_failures) in [
            (crate::audiomuse::ranking::Mode::WholeQueue, 0),
            (crate::audiomuse::ranking::Mode::PerTrack, 1),
        ] {
            let outcome = super::super::remix::rank_queue(&api, mode, &ids)
                .await
                .unwrap();
            assert_eq!(outcome.attempted, 3);
            assert_eq!(outcome.failed, expected_failures);
            assert_eq!(
                outcome
                    .items
                    .iter()
                    .map(|(i, track)| (*i, track.title.as_str()))
                    .collect::<Vec<_>>(),
                [(0, "match"), (1, "other")],
                "Use ranking order, skip other folders and original IDs, and avoid duplicate picks"
            );
        }
        api.direct = OnceCell::new();
        assert!(
            api.similar("seed", true).await.is_err(),
            "No random fallback for unavailable analysis"
        );
        task.abort();
    }
}
