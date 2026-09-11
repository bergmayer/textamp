//! Optional analysis service. Playback and library identity stay with Navidrome.
//! Only authentication, read-only export and discovery endpoints are used here.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::Duration;

fn track_ids(value: &Value, limit: usize) -> Result<Vec<String>> {
    let rows = value
        .as_array()
        .context("Missing AudioMuse track results")?;
    ensure!(rows.len() <= limit, "Too many AudioMuse track results");
    rows.iter()
        .map(|row| {
            row["item_id"]
                .as_str()
                .filter(|id| !id.is_empty() && id.len() <= 4096)
                .map(str::to_owned)
                .context("Invalid AudioMuse track ID")
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Connection {
    pub url: String,
    pub username: String,
    /// AudioMuse's configured music-server name, not a filesystem path.
    pub server: String,
}
impl Connection {
    pub fn validate(&self) -> Result<reqwest::Url> {
        let url = reqwest::Url::parse(&format!("{}/", self.url.trim_end_matches('/')))
            .context("Invalid AudioMuse URL")?;
        ensure!(
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "Use an http(s) AudioMuse URL without credentials, query or fragment"
        );
        ensure!(
            !self.username.trim().is_empty() && !self.server.trim().is_empty(),
            "Enter an AudioMuse username and music-server name"
        );
        Ok(url)
    }
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: reqwest::Url,
    server: String,
    cookie: reqwest::header::HeaderValue,
}
impl Client {
    pub async fn connect(connection: &Connection, password: &str) -> Result<Self> {
        let base = connection.validate()?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(45))
            // Never send passwords/session cookies through redirects.
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let response = http
            .post(base.join("auth")?)
            .header("X-Requested-With", "XMLHttpRequest")
            .json(&json!({"user": connection.username, "password": password}))
            .send()
            .await
            .map_err(|e| e.without_url())
            .context("AudioMuse sign-in")?;
        ensure!(
            response.status().is_success(),
            "AudioMuse sign-in failed (HTTP {})",
            response.status().as_u16()
        );
        let cookie = response
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .filter_map(|v| v.split(';').next())
            .find(|v| v.starts_with("audiomuse_jwt="))
            .context("AudioMuse did not issue a session; check credentials")?;
        let mut cookie = reqwest::header::HeaderValue::from_str(cookie)?;
        cookie.set_sensitive(true);
        Ok(Self {
            http,
            base,
            server: connection.server.clone(),
            cookie,
        })
    }
    async fn request(
        &self,
        path: &str,
        params: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<Value> {
        let mut url = self.base.join(path)?;
        url.query_pairs_mut()
            .append_pair("server", &self.server)
            .extend_pairs(params.iter().map(|(k, v)| (*k, v.as_str())));
        let request = match body {
            Some(body) => self.http.post(url).json(&body),
            None => self.http.get(url),
        }
        .header(reqwest::header::COOKIE, self.cookie.clone());
        let mut response = request
            .send()
            .await
            .map_err(|e| e.without_url())
            .context("Contact AudioMuse")?;
        let status = response.status();
        ensure!(
            status.is_success(),
            "AudioMuse {path}: HTTP {}{}",
            status.as_u16(),
            match status.as_u16() {
                401 | 403 => " (check credentials)",
                404 => " (feature unavailable)",
                503 => " (analysis index not ready)",
                _ => "",
            }
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| e.without_url())? {
            ensure!(
                bytes.len() + chunk.len() <= 8 * 1024 * 1024,
                "AudioMuse response too large"
            );
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).context("Invalid AudioMuse response")?;
        ensure!(
            value.get("error").is_none(),
            "AudioMuse could not complete {path}"
        );
        Ok(value)
    }
    /// Verify both provider and explicit server binding before accepting any IDs.
    pub async fn verify(&self) -> Result<()> {
        let servers = self.request("api/servers", &[], None).await?;
        let server = servers["servers"]
            .as_array()
            .context("Missing AudioMuse server list")?
            .iter()
            .find(|s| {
                s["name"].as_str() == Some(&self.server)
                    || s["server_id"].as_str() == Some(&self.server)
            })
            .context("Music-server name not found in AudioMuse")?;
        ensure!(
            server["server_type"].as_str() == Some("navidrome"),
            "Select a Navidrome music server in AudioMuse"
        );
        let page: Page<Analysis> = serde_json::from_value(
            self.request(
                "api/sync",
                &[
                    ("limit", "1".into()),
                    ("include_embeddings", "false".into()),
                ],
                None,
            )
            .await?,
        )?;
        ensure!(
            page.provider_type == "navidrome",
            "AudioMuse returned another provider's IDs"
        );
        Ok(())
    }
    /// Read-only sonic discovery. IDs are translated by AudioMuse for `server`.
    pub async fn similar(&self, seed: &str) -> Result<Vec<String>> {
        self.neighbors("item_id", seed).await
    }
    pub async fn mood(&self, mood: &str) -> Result<Vec<String>> {
        let centroids = self
            .request("api/mood_centroids", &[("mood", mood.into())], None)
            .await?;
        let rows = centroids[mood]
            .as_array()
            .context("AudioMuse mood unavailable")?;
        let indices: Vec<_> = rows.iter().filter_map(|r| r["index"].as_u64()).collect();
        use rand::prelude::IndexedRandom;
        let index = *indices
            .choose(&mut rand::rng())
            .context("No analyzed mood centroids yet")?;
        let result = self
            .request(
                "api/similar_tracks",
                &[
                    ("mood", mood.into()),
                    ("centroid_index", index.to_string()),
                    ("n", "100".into()),
                    ("eliminate_duplicates", "true".into()),
                ],
                None,
            )
            .await?;
        track_ids(&result, 100)
    }
    pub async fn moods(&self) -> Result<Vec<String>> {
        let value = self.request("api/mood_centroids", &[], None).await?;
        let object = value.as_object().context("Missing AudioMuse moods")?;
        ensure!(object.len() <= 1000, "Too many AudioMuse moods");
        Ok(object
            .iter()
            .filter(|(_, v)| v.as_array().is_some_and(|rows| !rows.is_empty()))
            .map(|(name, _)| name.clone())
            .collect())
    }
    async fn neighbors(&self, field: &str, value: &str) -> Result<Vec<String>> {
        let result = self
            .request(
                "api/similar_tracks",
                &[
                    (field, value.into()),
                    ("n", "100".into()),
                    ("eliminate_duplicates", "true".into()),
                    ("radius_similarity", "false".into()),
                ],
                None,
            )
            .await?;
        track_ids(&result, 100)
    }
    pub async fn path(&self, start: &str, end: &str, count: usize) -> Result<Vec<String>> {
        ensure!(
            (2..=100).contains(&count) && start != end,
            "Choose distinct sonic path endpoints and 2–100 tracks"
        );
        let result = self
            .request(
                "api/find_path",
                &[
                    ("start_song_id", start.into()),
                    ("end_song_id", end.into()),
                    ("max_steps", count.to_string()),
                    ("path_fix_size", "false".into()),
                    ("path_space", "audio".into()),
                ],
                None,
            )
            .await?;
        track_ids(&result["path"], count)
    }
    pub async fn readiness(&self, feature: Feature) -> Result<Readiness> {
        let (path, flag) = match feature {
            Feature::Describe => ("api/clap/stats", "clap_enabled"),
            Feature::Lyrics => ("api/lyrics/stats", "lyrics_enabled"),
            _ => anyhow::bail!("Not a text-search feature"),
        };
        let value = self.request(path, &[], None).await?;
        Ok(Readiness {
            enabled: value[flag]
                .as_bool()
                .context("Missing AudioMuse feature status")?,
            songs: value["song_count"]
                .as_u64()
                .or_else(|| value["num_embeddings"].as_u64())
                .unwrap_or(0),
        })
    }
    pub async fn analysis(&self, id: &str) -> Result<Option<Analysis>> {
        let response: Page<Analysis> = serde_json::from_value(
            self.request(
                "api/sync",
                &[("ids", id.into()), ("include_embeddings", "false".into())],
                None,
            )
            .await?,
        )?;
        ensure!(
            response.provider_type == "navidrome" && !response.has_more,
            "Wrong AudioMuse analysis response"
        );
        ensure!(
            response.tracks.len() <= 1 && response.tracks.iter().all(|t| t.id == id),
            "Wrong AudioMuse track ID"
        );
        Ok(response.tracks.into_iter().next().map(|mut track| {
            track.clean();
            track
        }))
    }
    pub async fn search(&self, feature: Feature, query: &str) -> Result<Vec<String>> {
        ensure!(
            query.trim().chars().count() >= 3,
            "Enter at least three characters"
        );
        let ready = self.readiness(feature).await?;
        ensure!(
            ready.enabled,
            "{} is disabled in AudioMuse",
            feature.label()
        );
        ensure!(
            ready.songs > 0,
            "{} has no analyzed tracks yet",
            feature.label()
        );
        let path = match feature {
            Feature::Describe => "api/clap/search",
            Feature::Lyrics => "api/lyrics/search/text",
            _ => unreachable!(),
        };
        let value = self
            .request(
                path,
                &[],
                Some(json!({"query": query, "limit": 200, "server": self.server})),
            )
            .await?;
        let items = value["results"]
            .as_array()
            .context("Missing AudioMuse search results")?;
        items
            .iter()
            .map(|v| {
                v["item_id"]
                    .as_str()
                    .map(str::to_owned)
                    .context("Missing AudioMuse track ID")
            })
            .collect()
    }
    /// Transactional refresh: an incomplete/erroring manifest never replaces the cache.
    /// Requests are sequential, bounded, and omit embeddings/audio entirely.
    pub async fn refresh(
        &self,
        previous: &Snapshot,
        allowed: &HashSet<String>,
    ) -> Result<Snapshot> {
        self.refresh_with_progress(previous, allowed, |_| {}).await
    }

    pub async fn refresh_with_progress(
        &self,
        previous: &Snapshot,
        allowed: &HashSet<String>,
        mut progress: impl FnMut(SyncProgress),
    ) -> Result<Snapshot> {
        let mut manifest = BTreeMap::new();
        let mut tracks = BTreeMap::new();
        // With no local index, fetch metadata directly in pages. Building an
        // index first and then requesting every ID in 100-song batches adds
        // a separate manifest pass and hundreds of unnecessary round trips.
        let cold = previous.tracks.is_empty();
        let mut page = 1;
        let mut total = None;
        let mut partial = false;
        let mut preview_sent = false;
        loop {
            let response: Page<Analysis> = serde_json::from_value(
                self.request(
                    "api/sync",
                    &[
                        if cold {
                            ("include_embeddings", "false".into())
                        } else {
                            ("fields", "index".into())
                        },
                        ("limit", "1000".into()),
                        ("page", page.to_string()),
                    ],
                    None,
                )
                .await?,
            )?;
            ensure!(
                response.provider_type == "navidrome",
                "Wrong AudioMuse provider"
            );
            ensure!(
                manifest.len() + response.tracks.len() <= 2_000_000,
                "AudioMuse library exceeds sync limit"
            );
            let count = response.tracks.len();
            if page == 1 {
                total = response.total_tracks;
            }
            partial |= response.total_tracks != total;
            let before = manifest.len();
            for mut entry in response.tracks {
                partial |= manifest
                    .insert(entry.id.clone(), entry.fp.clone())
                    .is_some();
                if cold && allowed.contains(&entry.id) {
                    entry.clean();
                    tracks.insert(entry.id.clone(), entry);
                }
            }
            progress(SyncProgress::Index {
                loaded: manifest.len(),
                total: response.total_tracks,
            });
            if cold && !preview_sent && !tracks.is_empty() {
                // Show the first usable page promptly, but do not commit it to
                // disk. updated=0 means no complete refresh has finished yet.
                progress(SyncProgress::Preview(std::sync::Arc::new(Snapshot {
                    tracks: tracks.clone(),
                    available: total.unwrap_or(manifest.len()),
                    partial: true,
                    ..Default::default()
                })));
                preview_sent = true;
            }
            ensure!(
                count == 0 || manifest.len() > before,
                "AudioMuse returned a repeated page"
            );
            if !response.has_more {
                break;
            }
            ensure!(
                count > 0 && response.next_page == Some(page + 1),
                "Invalid AudioMuse pagination"
            );
            page += 1;
        }
        partial |= total != Some(manifest.len());
        if partial {
            // Offset pagination is not a snapshot while a scan is inserting rows.
            // Keep previously known, still-in-library analysis until a stable pass.
            tracks.extend(
                previous
                    .tracks
                    .iter()
                    .filter(|(id, _)| allowed.contains(*id))
                    .map(|(id, t)| (id.clone(), t.clone())),
            );
        }
        let mut changed = Vec::new();
        let mut unresolved = 0;
        for (id, fp) in &manifest {
            if !allowed.contains(id) {
                continue;
            }
            match tracks.get(id).or_else(|| previous.tracks.get(id)) {
                Some(track) if &track.fp == fp => {
                    tracks.insert(id.clone(), track.clone());
                }
                _ => changed.push(id.clone()),
            }
        }
        // Gunicorn's default request-line limit is 4094 bytes. Navidrome IDs
        // plus URL-escaped separators fit comfortably in batches of 100.
        for (batch, ids) in changed.chunks(100).enumerate() {
            let response: Page<Analysis> = serde_json::from_value(
                self.request(
                    "api/sync",
                    &[
                        ("ids", ids.join(",")),
                        ("include_embeddings", "false".into()),
                    ],
                    None,
                )
                .await?,
            )?;
            ensure!(
                response.provider_type == "navidrome"
                    && !response.has_more
                    && !response.tracks.is_empty(),
                "Incomplete AudioMuse analysis response"
            );
            let expected: HashSet<_> = ids.iter().collect();
            let mut received = HashSet::new();
            for mut track in response.tracks {
                if !expected.contains(&track.id) {
                    // The server can translate a canonical audio fingerprint to
                    // a different duplicate file. Never attach that analysis to
                    // the requested song, even if the returned ID is in-library.
                    partial = true;
                    continue;
                }
                ensure!(
                    received.insert(track.id.clone()),
                    "AudioMuse returned duplicate analysis for the same track"
                );
                track.clean();
                tracks.insert(track.id.clone(), track);
            }
            for id in ids.iter().filter(|id| !received.contains(*id)) {
                unresolved += 1;
                if let Some(old) = previous.tracks.get(id) {
                    tracks.insert(id.clone(), old.clone());
                }
            }
            progress(SyncProgress::Tracks {
                loaded: (batch * 100 + ids.len()),
                total: changed.len(),
            });
        }
        Ok(Snapshot {
            tracks,
            updated: crate::library::cache::now(),
            available: manifest.len(),
            partial: partial || unresolved > 0,
            unresolved,
        })
    }
}

#[derive(Debug, Deserialize)]
struct Page<T> {
    tracks: Vec<T>,
    provider_type: String,
    has_more: bool,
    next_page: Option<usize>,
    total_tracks: Option<usize>,
}
pub struct Readiness {
    pub enabled: bool,
    pub songs: u64,
}

#[derive(Debug, Clone)]
pub enum SyncProgress {
    Preview(std::sync::Arc<Snapshot>),
    Index { loaded: usize, total: Option<usize> },
    Tracks { loaded: usize, total: usize },
}
impl std::fmt::Display for SyncProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Preview(snapshot) => {
                write!(f, "AudioMuse: {} tracks · loading…", snapshot.tracks.len())
            }
            Self::Index {
                loaded,
                total: Some(total),
            } => write!(f, "AudioMuse index: {loaded}/{total}"),
            Self::Index {
                loaded,
                total: None,
            } => write!(f, "AudioMuse index: {loaded}"),
            Self::Tracks { loaded, total } => write!(f, "AudioMuse analysis: {loaded}/{total}"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub partial: bool,
    /// Requested IDs for which the server did not return an exact match.
    #[serde(default)]
    pub unresolved: usize,
    pub tracks: BTreeMap<String, Analysis>,
    pub updated: u64,
    pub available: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Analysis {
    pub id: String,
    pub fp: String,
    pub tempo: Option<f32>,
    pub energy: Option<f32>,
    pub key: Option<String>,
    pub scale: Option<String>,
    #[serde(default)]
    pub mood_vector: Option<String>,
    #[serde(default)]
    pub other_features: Option<String>,
}
impl Analysis {
    fn clean(&mut self) {
        for text in [
            &mut self.key,
            &mut self.scale,
            &mut self.mood_vector,
            &mut self.other_features,
        ]
        .into_iter()
        .flatten()
        {
            *text = crate::util::sanitize_display_text(text)
                .chars()
                .take(4096)
                .collect();
        }
        self.tempo = self.tempo.filter(|v| v.is_finite() && *v > 0.0);
        self.energy = self
            .energy
            .filter(|v| v.is_finite() && (0.0..=1.0).contains(v));
    }
    pub fn labels(&self) -> BTreeMap<String, f32> {
        let mut labels = BTreeMap::<String, f32>::new();
        for pair in [self.mood_vector.as_deref(), self.other_features.as_deref()]
            .into_iter()
            .flatten()
            .flat_map(|s| s.split(','))
        {
            let Some((label, score)) = pair.rsplit_once(':') else {
                continue;
            };
            let Ok(score) = score.parse::<f32>() else {
                continue;
            };
            // Zero-filled features from a disabled model are not classifications.
            if !label.trim().is_empty() && score.is_finite() && score > 0.0 && score <= 1.0 {
                labels
                    .entry(label.trim().to_lowercase())
                    .and_modify(|v| *v = v.max(score))
                    .or_insert(score);
            }
        }
        labels
    }
    pub fn musical_key(&self) -> Option<String> {
        Some(format!("{} {}", self.key.as_ref()?, self.scale.as_ref()?))
    }
    pub fn summary(&self) -> String {
        let number = |v: Option<f32>| v.map_or_else(|| "Unavailable".into(), |v| format!("{v:.1}"));
        let labels = self
            .labels()
            .into_iter()
            .map(|(k, v)| format!("{k}: {v:.3}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("Tempo: {} BPM\nEnergy: {}\nKey: {}\n\nInferred labels (model scores, not tag values or probabilities):\n{}",
            number(self.tempo), number(self.energy), self.musical_key().unwrap_or_else(|| "Unavailable".into()), labels)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    Labels,
    Tempo,
    Energy,
    Key,
    Analyzed,
    Describe,
    Lyrics,
}
impl Feature {
    pub const ALL: [Self; 7] = [
        Self::Labels,
        Self::Tempo,
        Self::Energy,
        Self::Key,
        Self::Analyzed,
        Self::Describe,
        Self::Lyrics,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Labels => "AI labels",
            Self::Tempo => "Tempo",
            Self::Energy => "Energy",
            Self::Key => "Musical key",
            Self::Analyzed => "Analyzed tracks",
            Self::Describe => "Describe music…",
            Self::Lyrics => "Search lyrics…",
        }
    }
    pub fn is_search(self) -> bool {
        matches!(self, Self::Describe | Self::Lyrics)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Filter {
    All,
    Label(String),
    Tempo(u8),
    Energy(u8),
    Key(String),
}
impl Filter {
    pub fn matches(&self, track: &Analysis) -> bool {
        match self {
            Self::All => true,
            Self::Label(label) => track.labels().contains_key(label),
            Self::Key(key) => track.musical_key().as_ref() == Some(key),
            Self::Tempo(bucket) => track.tempo.is_some_and(|v| tempo_bucket(v) == *bucket),
            Self::Energy(bucket) => track.energy.is_some_and(|v| energy_bucket(v) == *bucket),
        }
    }
}
fn tempo_bucket(v: f32) -> u8 {
    if v < 90.0 {
        0
    } else if v < 120.0 {
        1
    } else if v < 160.0 {
        2
    } else {
        3
    }
}
fn energy_bucket(v: f32) -> u8 {
    if v < 1.0 / 3.0 {
        0
    } else if v < 2.0 / 3.0 {
        1
    } else {
        2
    }
}
impl Snapshot {
    pub fn groups(&self, feature: Feature) -> Vec<(String, Filter)> {
        match feature {
            Feature::Labels => self
                .tracks
                .values()
                .flat_map(|t| t.labels().into_keys())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|s| (s.clone(), Filter::Label(s)))
                .collect(),
            Feature::Key => self
                .tracks
                .values()
                .filter_map(Analysis::musical_key)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|s| (s.clone(), Filter::Key(s)))
                .collect(),
            Feature::Tempo => ["Under 90 BPM", "90–119 BPM", "120–159 BPM", "160+ BPM"]
                .into_iter()
                .enumerate()
                .map(|(i, s)| (s.into(), Filter::Tempo(i as u8)))
                .collect(),
            Feature::Energy => ["Low", "Medium", "High"]
                .into_iter()
                .enumerate()
                .map(|(i, s)| (s.into(), Filter::Energy(i as u8)))
                .collect(),
            _ => vec![],
        }
    }
}
