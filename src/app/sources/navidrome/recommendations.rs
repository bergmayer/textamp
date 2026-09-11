//! One analysis adapter for radio, DJs, remixes and discovery. Navidrome owns
//! identity/playback; AudioMuse can only recommend IDs in the selected catalog.
use super::{effects::discovery, Session};
use crate::app::{sources::audiomuse, AppState};
use crate::library::track::Track;
use anyhow::{ensure, Context, Result};
use futures::{stream, StreamExt, TryStreamExt};
use std::collections::HashSet;
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
    allowed: HashSet<String>,
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
            allowed: state
                .library
                .all_tracks
                .iter()
                .filter_map(|t| session.id(&t.rating_key).ok())
                .collect(),
        }
    }
    async fn direct(&self) -> Result<&crate::audiomuse::Client> {
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
                    .is_ok_and(|id| self.allowed.contains(&id))
                    && seen.insert(t.rating_key.clone())
            })
            .collect()
    }
    async fn resolve(&self, ids: Vec<String>, path: bool) -> Result<Vec<Track>> {
        if path {
            ensure!(
                ids.iter().all(|id| self.allowed.contains(id))
                    && ids.iter().collect::<HashSet<_>>().len() == ids.len(),
                "Sonic path leaves this library or repeats tracks"
            );
        }
        let mut seen = HashSet::new();
        let ids: Vec<_> = ids
            .into_iter()
            .filter(|id| self.allowed.contains(id) && seen.insert(id.clone()))
            .collect();
        stream::iter(ids)
            .map(|id| async move {
                let song = self.session.client.song(&id).await?;
                ensure!(
                    song.id == id,
                    "Navidrome returned the wrong recommended track"
                );
                Ok(self.session.track(song))
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
                self.allowed.contains(id),
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
            if let Ok(tracks) = &native {
                if !tracks.is_empty() {
                    return native;
                }
            }
            let result = async {
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
            .await;
            result.with_context(|| {
                native.err().map_or_else(
                    || "Navidrome returned no analyzed matches".into(),
                    |e| e.to_string(),
                )
            })
        })
        .await
        .context("Analysis request timed out")?
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
                start != end && self.allowed.contains(start) && self.allowed.contains(end),
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
                    .is_ok_and(|id| self.allowed.contains(&id))),
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
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn native_and_direct_analysis_share_scope_paths_and_failure_rules() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let invalid = Arc::new(AtomicBool::new(false));
        let check = invalid.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let check = check.clone();
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    let mut chunk = [0; 4096];
                    loop {
                        let n = socket.read(&mut chunk).await.unwrap();
                        if n == 0 {
                            return;
                        }
                        bytes.extend_from_slice(&chunk[..n]);
                        if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let text = String::from_utf8_lossy(&bytes);
                    let url = reqwest::Url::parse(&format!(
                        "http://local{}",
                        text.split_whitespace().nth(1).unwrap()
                    ))
                    .unwrap();
                    let p: std::collections::HashMap<_, _> = url.query_pairs().collect();
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
                        "/rest/getSong" => {
                            let id = &p["id"];
                            assert_ne!(id, "outside", "Never fetch another library's song");
                            json!({"subsonic-response":{"status":"ok","song":{"id":id,"title":id}}})
                        }
                        "/rest/getSonicSimilarTracks" | "/rest/findSonicPath" => {
                            json!({"subsonic-response":{"status":"failed","error":{"code":70,"message":"index not ready"}}})
                        }
                        other => panic!("unexpected request {other}"),
                    };
                    let body = value.to_string();
                    let reply = format!("HTTP/1.1 200 OK\r\nSet-Cookie: audiomuse_jwt=fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
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
            allowed: ["seed", "match", "end"].map(str::to_owned).into(),
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
        assert!(api
            .similar("seed", true)
            .await
            .unwrap_err()
            .to_string()
            .contains("disabled"));
        assert!(api
            .path("seed", "end", 5)
            .await
            .unwrap_err()
            .to_string()
            .contains("disabled"));
        api.sonic_allowed = true;
        invalid.store(true, Ordering::Relaxed);
        assert!(
            format!("{:#}", api.path("seed", "end", 5).await.unwrap_err())
                .contains("leaves this library")
        );
        assert!(api.similar("outside", true).await.is_err());
        api.direct = OnceCell::new();
        assert!(
            api.similar("seed", true).await.is_err(),
            "No random fallback for unavailable analysis"
        );
        task.abort();
    }
}
