//! OpenSubsonic transport and native metadata. No application, server, or UI dependencies.
use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

pub mod models;
pub use models::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Selection {
    pub source_id: String,
    pub folder: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub url: String,
    pub username: String,
    #[serde(default)]
    pub libraries: Vec<MusicFolder>,
}

impl Source {
    /// One saved account per server URL and username; music folders stay distinct.
    pub fn same_account(&self, other: &Self) -> bool {
        self.username == other.username
            && matches!(
                (reqwest::Url::parse(self.url.trim_end_matches('/')),
                 reqwest::Url::parse(other.url.trim_end_matches('/'))),
                (Ok(a), Ok(b)) if a == b
            )
    }

    /// A sole music folder and the server-wide view name the same library.
    pub fn canonical_folder(&self, folder: Option<String>) -> Option<String> {
        folder.or_else(|| match self.libraries.as_slice() {
            [only] => Some(only.id.clone()),
            _ => None,
        })
    }
    pub fn validate(&self) -> Result<()> {
        let url = reqwest::Url::parse(&self.url).context("Invalid Navidrome URL")?;
        if !matches!(url.scheme(), "https" | "http")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("Use an http(s) server URL without credentials, query, or fragment");
        }
        if self.id.is_empty() || self.name.trim().is_empty() || self.username.trim().is_empty() {
            bail!("Navidrome requires a name, username, and unique source ID");
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: reqwest::Url,
    username: String,
    password: crate::util::SecretString,
    pub folder: Option<String>,
}
impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NavidromeClient")
            .field("folder", &self.folder)
            .finish_non_exhaustive()
    }
}

impl Client {
    pub fn new(
        source: &Source,
        password: crate::util::SecretString,
        folder: Option<String>,
    ) -> Result<Self> {
        source.validate()?;
        if password.is_empty() {
            bail!("Navidrome password is missing; use C in Libraries to set credentials");
        }
        Ok(Self {
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            base: reqwest::Url::parse(&format!("{}/rest/", source.url.trim_end_matches('/')))?,
            username: source.username.clone(),
            password,
            folder: source.canonical_folder(folder),
        })
    }
    pub fn http(&self) -> reqwest::Client {
        self.http.clone()
    }
    /// URLs are capabilities: never log or serialize the returned value.
    pub fn url(&self, method: &str, params: &[(&str, String)]) -> Result<reqwest::Url> {
        if !method.bytes().all(|c| c.is_ascii_alphanumeric()) {
            bail!("Invalid API method");
        }
        let mut url = self.base.join(method)?;
        let salt = uuid::Uuid::new_v4().simple().to_string();
        let mut input = zeroize::Zeroizing::new(self.password.to_string());
        input.push_str(&salt);
        let token = format!("{:x}", md5::compute(input.as_bytes()));
        url.query_pairs_mut()
            .extend_pairs([
                ("u", self.username.as_str()),
                ("t", token.as_str()),
                ("s", salt.as_str()),
                ("v", "1.16.1"),
                ("c", "Textamp"),
                ("f", "json"),
            ])
            .extend_pairs(params.iter().map(|(k, v)| (*k, v.as_str())));
        Ok(url)
    }
    pub async fn call(&self, method: &str, params: &[(&str, String)]) -> Result<Value> {
        let write = matches!(
            method,
            "createPlaylist"
                | "updatePlaylist"
                | "deletePlaylist"
                | "star"
                | "unstar"
                | "setRating"
                | "scrobble"
                | "savePlayQueue"
        );
        let read_only =
            method.starts_with("get") || matches!(method, "ping" | "search3" | "findSonicPath");
        let mut retried = false;
        let body = loop {
            let request = if write {
                self.http.post(self.url(method, &[])?).form(params)
            } else {
                self.http.get(self.url(method, params)?)
            };
            match self.read(request, 32 * 1024 * 1024).await {
                Ok(body) => break body,
                Err(error) if read_only && !retried && interrupted_request(&error) => {
                    retried = true;
                    tracing::warn!("Retrying interrupted Navidrome {method} request once");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                Err(error) => return Err(error.context(format!("Navidrome {method}"))),
            }
        };
        let value: Value =
            serde_json::from_slice(&body).context("Navidrome returned malformed JSON")?;
        let response = value
            .get("subsonic-response")
            .context("Missing Subsonic response")?;
        match response.get("status").and_then(Value::as_str) {
            Some("ok") => Ok(response.clone()),
            Some("failed") => {
                let code = response["error"]["code"].as_u64().unwrap_or(0);
                // Do not echo untrusted response bodies (which can contain credentials/HTML).
                let message = match code {
                    40 | 41 => "Authentication failed; check account credentials",
                    50 => "This account does not have permission",
                    70 => "The requested item is no longer available",
                    20 | 30 => "The server does not support this API version",
                    10 => "The server rejected the request parameters",
                    _ => "The server could not complete the request",
                };
                bail!("Navidrome {method}: {message} (API {code})")
            }
            _ => bail!("Navidrome returned an invalid response status"),
        }
    }
    pub async fn bytes(&self, url: reqwest::Url, limit: usize) -> Result<Vec<u8>> {
        self.read(self.http.get(url), limit).await
    }
    async fn read(&self, request: reqwest::RequestBuilder, limit: usize) -> Result<Vec<u8>> {
        tokio::time::timeout(Duration::from_secs(30), async {
            let mut response = request.send().await.map_err(|e| e.without_url())?;
            if !response.status().is_success() {
                bail!("HTTP {}", response.status());
            }
            if response.content_length().is_some_and(|n| n > limit as u64) {
                bail!("Response exceeds size limit");
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|e| e.without_url())? {
                if bytes.len().saturating_add(chunk.len()) > limit {
                    bail!("Response exceeds size limit");
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(bytes)
        })
        .await
        .context("Navidrome request timed out")?
    }
    pub fn scope(&self) -> Vec<(&'static str, String)> {
        self.folder
            .iter()
            .map(|id| ("musicFolderId", id.clone()))
            .collect()
    }
    pub async fn folders(&self) -> Result<Vec<MusicFolder>> {
        let value = self.call("getMusicFolders", &[]).await?;
        array(&value["musicFolders"], "musicFolder")
    }
    pub async fn artists(&self) -> Result<Vec<Artist>> {
        let value = self.call("getArtists", &self.scope()).await?;
        let groups: Vec<Value> = array(&value["artists"], "index")?;
        let mut artists = Vec::new();
        for group in groups {
            artists.extend(array::<Artist>(&group, "artist")?);
        }
        Ok(artists)
    }
    pub async fn albums(&self, kind: &str, extra: &[(&str, String)]) -> Result<Vec<Album>> {
        let mut params = self.scope();
        params.extend_from_slice(extra);
        params.push(("type", kind.into()));
        self.pages(
            "getAlbumList2",
            "albumList2",
            "album",
            params,
            "size",
            "offset",
        )
        .await
    }
    pub async fn artist(&self, id: &str) -> Result<Artist> {
        object(
            &self.call("getArtist", &[("id", id.into())]).await?,
            "artist",
        )
    }
    pub async fn album(&self, id: &str) -> Result<Album> {
        object(&self.call("getAlbum", &[("id", id.into())]).await?, "album")
    }
    pub async fn song(&self, id: &str) -> Result<Song> {
        object(&self.call("getSong", &[("id", id.into())]).await?, "song")
    }
    pub async fn playlists(&self) -> Result<Vec<Playlist>> {
        array(
            &self.call("getPlaylists", &[]).await?["playlists"],
            "playlist",
        )
    }
    pub async fn playlist(&self, id: &str) -> Result<Playlist> {
        object(
            &self.call("getPlaylist", &[("id", id.into())]).await?,
            "playlist",
        )
    }
    pub async fn songs(&self, query: &str) -> Result<Vec<Song>> {
        let mut params = self.scope();
        params.extend([
            ("query", query.into()),
            ("artistCount", "0".into()),
            ("albumCount", "0".into()),
        ]);
        self.pages(
            "search3",
            "searchResult3",
            "song",
            params,
            "songCount",
            "songOffset",
        )
        .await
    }
    pub async fn pages<T: DeserializeOwned>(
        &self,
        method: &str,
        root: &str,
        field: &str,
        mut params: Vec<(&str, String)>,
        count: &str,
        offset: &str,
    ) -> Result<Vec<T>> {
        let mut result = Vec::new();
        let mut seen = std::collections::HashSet::new();
        params.push((count, "500".into()));
        // An explicit bound prevents endless loops on broken pagination. Never return a truncated success.
        for page in 0..2000 {
            let mut query = params.clone();
            query.push((offset, (page * 500).to_string()));
            let value = self.call(method, &query).await?;
            let batch: Vec<Value> = array(&value[root], field)?;
            let count = batch.len();
            for item in batch {
                let id = item["id"].as_str().context("Item has no string ID")?;
                if !seen.insert(id.to_owned()) {
                    bail!("Navidrome pagination repeated an item; retry after the library scan finishes");
                }
                result.push(serde_json::from_value(item).context("Invalid Navidrome item")?);
            }
            if count < 500 {
                return Ok(result);
            }
        }
        bail!("Navidrome catalog exceeds the supported request limit")
    }
}

fn interrupted_request(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<reqwest::Error>()
        .is_some_and(|e| e.is_connect() || e.is_timeout() || e.is_body() || e.is_request())
        || error
            .downcast_ref::<tokio::time::error::Elapsed>()
            .is_some()
}

pub fn object<T: DeserializeOwned>(value: &Value, field: &str) -> Result<T> {
    serde_json::from_value(
        value
            .get(field)
            .with_context(|| format!("Missing {field} response"))?
            .clone(),
    )
    .with_context(|| format!("Malformed {field} response"))
}
pub fn array<T: DeserializeOwned>(value: &Value, field: &str) -> Result<Vec<T>> {
    if !value.is_object() {
        bail!("Missing response container for {field}");
    }
    match value.get(field) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(items) => {
            serde_json::from_value(items.clone()).with_context(|| format!("Malformed {field} list"))
        }
    }
}
