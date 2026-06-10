//! Remote Plex player control.
//!
//! Controls remote Plex players using a hybrid approach:
//! 1. Server proxy: commands sent to `{server}/player/playback/{cmd}` with
//!    `X-Plex-Target-Client-Identifier` header. Works for players the server
//!    can see on the LAN (e.g. Apple TV with "Advertise as player" enabled).
//! 2. Direct connection: commands sent to `{player_uri}/player/playback/{cmd}`.
//!    Works for players advertising on the local network (e.g. Plexamp with
//!    "Allow player to be controlled" enabled).

use super::constants::*;
use super::error::ApiError;
use super::models::Track;
use crate::util::truncate_to_boundary;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

/// Client for controlling a remote Plex player device.
#[derive(Debug, Clone)]
pub struct RemotePlayerClient {
    http: Client,
    /// Plex auth token.
    token: String,
    /// Our client identifier (textamp's device ID).
    client_id: String,
    /// The target player's client identifier.
    target_client_id: String,
    /// The Plex server URL.
    server_url: String,
    /// The Plex server's machine identifier.
    server_machine_id: String,
    /// Direct URI for the player (if it advertises on the LAN).
    player_uri: Option<String>,
}

/// Timeline status from a remote player poll.
#[derive(Debug, Clone)]
pub struct RemoteTimelineStatus {
    /// Whether an active session was found for the target player.
    pub session_found: bool,
    pub playing: bool,
    pub position_ms: u64,
    pub track_key: Option<String>,
    pub finished: bool,
}

/// Plex sessions response for timeline polling.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SessionsResponse {
    #[serde(default)]
    metadata: Vec<SessionMetadata>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionMetadata {
    #[serde(default)]
    rating_key: Option<String>,
    #[serde(default, rename = "viewOffset")]
    view_offset: u64,
    #[serde(default)]
    #[allow(dead_code)]
    duration: u64,
    #[serde(default, rename = "Player")]
    player: Option<SessionPlayer>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionPlayer {
    #[serde(default)]
    machine_identifier: String,
    #[serde(default)]
    state: String,
}

/// Play queue response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayQueueResponse {
    #[serde(default, rename = "playQueueID")]
    play_queue_id: u64,
}

impl RemotePlayerClient {
    /// Create a new remote player client.
    pub fn new(
        token: String,
        client_id: String,
        target_client_id: String,
        server_url: String,
        server_machine_id: String,
        player_uri: Option<String>,
    ) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(5))
            .danger_accept_invalid_certs(true)
            .build()
            .expect("Failed to create remote player HTTP client");

        Self {
            http,
            token,
            client_id,
            target_client_id,
            server_url,
            server_machine_id,
            player_uri,
        }
    }

    /// Common headers for all player commands.
    fn add_headers(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        request
            .header(HEADER_PLEX_TOKEN, &self.token)
            .header(HEADER_PLEX_CLIENT_ID, &self.client_id)
            .header(HEADER_PLEX_PRODUCT, "textamp")
            .header("X-Plex-Target-Client-Identifier", &self.target_client_id)
            .header("Accept", "application/json")
    }

    /// Send a player command, trying server proxy first, then direct.
    async fn send_command(&self, command: &str) -> Result<(), ApiError> {
        // Try 1: Server proxy
        let proxy_url = format!("{}/player/playback/{}", self.server_url, command);
        let proxy_resp = self.add_headers(self.http.get(&proxy_url))
            .query(&[("commandID", "1")])
            .send()
            .await;

        match proxy_resp {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!("Remote {}: success (server proxy)", command);
                return Ok(());
            }
            Ok(resp) => {
                tracing::info!("Remote {}: server proxy returned {}", command, resp.status());
            }
            Err(e) => {
                tracing::info!("Remote {}: server proxy failed: {}", command, e);
            }
        }

        // Try 2: Direct to player
        if let Some(ref uri) = self.player_uri {
            let direct_url = format!("{}/player/playback/{}", uri, command);
            let direct_resp = self.add_headers(self.http.get(&direct_url))
                .query(&[("commandID", "1")])
                .send()
                .await;

            match direct_resp {
                Ok(resp) if resp.status().is_success() => {
                    tracing::info!("Remote {}: success (direct)", command);
                    return Ok(());
                }
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    tracing::warn!("Remote {}: direct returned {} - {}", command, status, text);
                    return Err(ApiError::ServerError {
                        status: status.as_u16(),
                        message: format!("Remote {} failed", command),
                    });
                }
                Err(e) => {
                    tracing::warn!("Remote {}: direct connection failed: {}", command, e);
                    return Err(ApiError::ServerError {
                        status: 0,
                        message: format!("Remote {} failed: {}", command, e),
                    });
                }
            }
        }

        Err(ApiError::ServerError {
            status: 0,
            message: format!("Remote {} failed: no working connection", command),
        })
    }

    /// Create a play queue on the server for the given track.
    async fn create_play_queue(&self, track: &Track) -> Result<u64, ApiError> {
        let url = format!("{}/playQueues", self.server_url);
        let uri = format!(
            "server://{}/com.plexapp.plugins.library/library/metadata/{}",
            self.server_machine_id, track.rating_key
        );

        let request = self.add_headers(self.http.post(&url))
            .query(&[
                ("type", "audio"),
                ("uri", &uri),
                ("shuffle", "0"),
                ("repeat", "0"),
                ("includeChapters", "1"),
                ("includeRelated", "1"),
            ]);

        let response = request.send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            tracing::warn!("Create play queue failed ({}): {}", status, text);
            return Err(ApiError::ServerError {
                status: status.as_u16(),
                message: format!("Failed to create play queue: {}", text),
            });
        }

        let body = response.text().await.unwrap_or_default();

        // Try to parse the playQueueID from MediaContainer wrapper
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wrapper {
            #[serde(default, rename = "playQueueID")]
            play_queue_id: u64,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Container {
            media_container: Option<Wrapper>,
        }
        if let Ok(c) = serde_json::from_str::<Container>(&body) {
            if let Some(mc) = c.media_container {
                if mc.play_queue_id > 0 {
                    tracing::info!("Created play queue: {}", mc.play_queue_id);
                    return Ok(mc.play_queue_id);
                }
            }
        }

        // Try flat response
        if let Ok(pq) = serde_json::from_str::<PlayQueueResponse>(&body) {
            if pq.play_queue_id > 0 {
                tracing::info!("Created play queue: {}", pq.play_queue_id);
                return Ok(pq.play_queue_id);
            }
        }

        tracing::warn!("Could not parse play queue ID from response: {}", truncate_to_boundary(&body, 200));
        Ok(0)
    }

    /// Play a specific track on the remote player.
    ///
    /// Creates a play queue on the server, then sends a playMedia command
    /// trying server proxy first, then direct to the player.
    pub async fn play_media(&self, track: &Track, _library_key: &str) -> Result<(), ApiError> {
        let key = format!("/library/metadata/{}", track.rating_key);

        // Create a play queue first
        let queue_id = self.create_play_queue(track).await.unwrap_or(0);

        // Parse server address components for the player to connect back to the server
        let server_no_scheme = self.server_url
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        let address = server_no_scheme.split(':').next().unwrap_or("");
        let port = self.server_url.split(':').last().unwrap_or("32400");
        let protocol = if self.server_url.starts_with("https") { "https" } else { "http" };

        let mut params: Vec<(&str, String)> = vec![
            ("key", key.clone()),
            ("machineIdentifier", self.server_machine_id.clone()),
            ("address", address.to_string()),
            ("protocol", protocol.to_string()),
            ("port", port.to_string()),
            ("type", "music".to_string()),
            ("commandID", "1".to_string()),
        ];
        if queue_id > 0 {
            params.push(("containerKey", format!("/playQueues/{}?own=1&window=200", queue_id)));
        }

        let query: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();

        tracing::info!(
            "Remote playMedia: target={}, key={}, queue={}, server={}",
            self.target_client_id, key, queue_id, self.server_url
        );

        // Try 1: Server proxy
        let proxy_url = format!("{}/player/playback/playMedia", self.server_url);
        let proxy_resp = self.add_headers(self.http.get(&proxy_url))
            .query(&query)
            .send()
            .await;

        match proxy_resp {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!("Remote playMedia: success (server proxy) for {}", track.title);
                return Ok(());
            }
            Ok(resp) => {
                tracing::info!("Remote playMedia: server proxy returned {}", resp.status());
            }
            Err(e) => {
                tracing::info!("Remote playMedia: server proxy failed: {}", e);
            }
        }

        // Try 2: Direct to player
        if let Some(ref uri) = self.player_uri {
            let direct_url = format!("{}/player/playback/playMedia", uri);
            let direct_resp = self.add_headers(self.http.get(&direct_url))
                .query(&query)
                .send()
                .await;

            match direct_resp {
                Ok(resp) if resp.status().is_success() => {
                    tracing::info!("Remote playMedia: success (direct) for {}", track.title);
                    return Ok(());
                }
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    tracing::error!("Remote playMedia: direct returned {} - {}", status, text);
                    return Err(ApiError::ServerError {
                        status: status.as_u16(),
                        message: format!("Remote play failed: {}", text),
                    });
                }
                Err(e) => {
                    tracing::error!("Remote playMedia: direct failed: {}", e);
                    return Err(ApiError::ServerError {
                        status: 0,
                        message: format!("Remote play failed: {}", e),
                    });
                }
            }
        }

        Err(ApiError::ServerError {
            status: 0,
            message: "Remote play failed: no working connection".to_string(),
        })
    }

    /// Pause playback on the remote player.
    pub async fn pause(&self) -> Result<(), ApiError> {
        self.send_command("pause").await
    }

    /// Resume playback on the remote player.
    pub async fn resume(&self) -> Result<(), ApiError> {
        self.send_command("play").await
    }

    /// Stop playback on the remote player.
    pub async fn stop(&self) -> Result<(), ApiError> {
        self.send_command("stop").await
    }

    /// Seek to an absolute position on the remote player.
    pub async fn seek_to(&self, offset_ms: u64) -> Result<(), ApiError> {
        let proxy_url = format!("{}/player/playback/seekTo", self.server_url);
        let proxy_resp = self.add_headers(self.http.get(&proxy_url))
            .query(&[("offset", &offset_ms.to_string()), ("commandID", &"1".to_string())])
            .send()
            .await;

        if let Ok(resp) = proxy_resp {
            if resp.status().is_success() { return Ok(()); }
        }

        if let Some(ref uri) = self.player_uri {
            let direct_url = format!("{}/player/playback/seekTo", uri);
            let resp = self.add_headers(self.http.get(&direct_url))
                .query(&[("offset", &offset_ms.to_string()), ("commandID", &"1".to_string())])
                .send()
                .await?;
            if resp.status().is_success() { return Ok(()); }
        }

        Err(ApiError::ServerError { status: 0, message: "Remote seek failed".to_string() })
    }

    /// Set volume on the remote player (0-100).
    pub async fn set_volume(&self, percent: u32) -> Result<(), ApiError> {
        let proxy_url = format!("{}/player/playback/setParameters", self.server_url);
        let proxy_resp = self.add_headers(self.http.get(&proxy_url))
            .query(&[("volume", &percent.to_string()), ("commandID", &"1".to_string())])
            .send()
            .await;

        if let Ok(resp) = proxy_resp {
            if resp.status().is_success() { return Ok(()); }
        }

        if let Some(ref uri) = self.player_uri {
            let direct_url = format!("{}/player/playback/setParameters", uri);
            let resp = self.add_headers(self.http.get(&direct_url))
                .query(&[("volume", &percent.to_string()), ("commandID", &"1".to_string())])
                .send()
                .await?;
            if resp.status().is_success() { return Ok(()); }
        }

        Err(ApiError::ServerError { status: 0, message: "Remote volume set failed".to_string() })
    }

    /// Poll the Plex server's active sessions to find the target player's status.
    pub async fn poll_timeline(&self) -> Result<RemoteTimelineStatus, ApiError> {
        let url = format!("{}/status/sessions", self.server_url);
        let response = self.http.get(&url)
            .header(HEADER_PLEX_TOKEN, &self.token)
            .header("Accept", "application/json")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(ApiError::ServerError {
                status: response.status().as_u16(),
                message: "Session poll failed".to_string(),
            });
        }

        let body = response.text().await.unwrap_or_default();

        // Parse MediaContainer wrapper
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Container {
            media_container: Option<SessionsResponse>,
        }

        let sessions = if let Ok(c) = serde_json::from_str::<Container>(&body) {
            c.media_container.unwrap_or(SessionsResponse { metadata: vec![] })
        } else if let Ok(s) = serde_json::from_str::<SessionsResponse>(&body) {
            s
        } else {
            tracing::warn!("Could not parse sessions response");
            return Ok(RemoteTimelineStatus {
                session_found: false,
                playing: false,
                position_ms: 0,
                track_key: None,
                finished: false,
            });
        };

        // Find the session for our target player
        for session in &sessions.metadata {
            if let Some(ref player) = session.player {
                if player.machine_identifier == self.target_client_id {
                    return Ok(RemoteTimelineStatus {
                        session_found: true,
                        playing: player.state == "playing",
                        position_ms: session.view_offset,
                        track_key: session.rating_key.clone(),
                        finished: player.state == "stopped",
                    });
                }
            }
        }

        // No active session for this player — keep current state
        Ok(RemoteTimelineStatus {
            session_found: false,
            playing: false,
            position_ms: 0,
            track_key: None,
            finished: false,
        })
    }
}
