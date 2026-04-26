//! Plex authentication handling.
//!
//! Supports both username/password authentication and PIN-based OAuth flow.

use super::constants::*;
use super::error::ApiError;
use super::models::{PlexServer, PlexUser};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const PLEX_TV_URL: &str = "https://plex.tv";
const PLEX_AUTH_URL: &str = "https://app.plex.tv/auth";

/// Plex client identification headers.
#[derive(Debug, Clone)]
pub struct PlexClientInfo {
    pub product: String,
    pub version: String,
    pub client_identifier: String,
    pub device_name: String,
    pub platform: String,
}

impl Default for PlexClientInfo {
    fn default() -> Self {
        Self {
            product: "textamp".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            client_identifier: Uuid::new_v4().to_string(),
            device_name: hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
            platform: std::env::consts::OS.to_string(),
        }
    }
}

/// Plex authentication client.
pub struct PlexAuth {
    http: Client,
    client_info: PlexClientInfo,
}

/// PIN for OAuth authentication flow.
#[derive(Debug, Clone)]
pub struct AuthPin {
    pub id: u64,
    pub code: String,
    pub auth_url: String,
}

/// Stored authentication data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAuth {
    pub token: String,
    pub user_id: Option<u64>,
    pub username: Option<String>,
    pub client_identifier: String,
    #[serde(default)]
    pub server_url: Option<String>,
    #[serde(default)]
    pub server_identifier: Option<String>,
    #[serde(default)]
    pub server_name: Option<String>,
    #[serde(default)]
    pub has_plex_pass: bool,
}

/// Server info for persistence.
#[derive(Debug, Clone)]
pub struct ServerInfo {
    pub url: String,
    pub identifier: String,
    pub name: String,
}

/// Persistent marker recording which account the on-disk caches were
/// populated for. Survives logout; consulted at sign-in to decide
/// whether to keep the cache (same user, recently signed in) or wipe
/// it (different user / stale > 30 days).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountMarker {
    pub username: String,
    /// Unix-epoch second the marker was last refreshed (sign-in or
    /// sign-out time).
    pub last_seen_unix: u64,
}

/// Filesystem path for the account marker — sibling of `auth.toml`
/// in the data directory so XDG and platform fallbacks both apply.
fn account_marker_path(paths: &crate::config::XdgPaths) -> std::path::PathBuf {
    paths.data_dir.join("account_marker.toml")
}

impl PlexAuth {
    /// Create a new PlexAuth with default client info.
    pub fn new() -> Self {
        Self::with_client_info(PlexClientInfo::default())
    }

    /// Create a new PlexAuth with custom client info.
    pub fn with_client_info(client_info: PlexClientInfo) -> Self {
        use std::time::Duration;
        Self {
            http: Client::builder()
                .timeout(Duration::from_secs(10)) // 10s timeout for plex.tv calls
                .build()
                .expect("Failed to create auth HTTP client"),
            client_info,
        }
    }

    /// Create a PlexAuth using the client_identifier from stored auth.
    /// This ensures API calls use the same identifier the token was issued for.
    pub fn from_stored_auth(stored: &StoredAuth) -> Self {
        let mut client_info = PlexClientInfo::default();
        client_info.client_identifier = stored.client_identifier.clone();
        Self::with_client_info(client_info)
    }

    /// Get the client identifier.
    pub fn client_identifier(&self) -> &str {
        &self.client_info.client_identifier
    }

    /// Authenticate with username and password.
    ///
    /// This is the simplest authentication method for personal use.
    pub async fn authenticate_password(
        &self,
        username: &str,
        password: &str,
    ) -> Result<String, ApiError> {
        let url = format!("{}/users/sign_in.json", PLEX_TV_URL);

        tracing::info!("Authenticating user: {}", username);

        let response = self
            .http
            .post(&url)
            .header(HEADER_PLEX_PRODUCT, &self.client_info.product)
            .header(HEADER_PLEX_VERSION, &self.client_info.version)
            .header(HEADER_PLEX_CLIENT_ID, &self.client_info.client_identifier)
            .header(HEADER_PLEX_DEVICE_NAME, &self.client_info.device_name)
            .header(HEADER_PLEX_PLATFORM, &self.client_info.platform)
            .header("Accept", "application/json")
            .form(&[
                ("user[login]", username),
                ("user[password]", password),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            tracing::error!("Auth failed: {} - {}", status, text);
            return Err(ApiError::AuthFailed(format!("Status {}: {}", status, text)));
        }

        let text = response.text().await?;
        tracing::debug!("Sign-in response: {}", &text[..text.len().min(500)]);

        let data: SignInResponse = serde_json::from_str(&text)
            .map_err(|e| {
                tracing::error!("Failed to parse sign-in response: {}", e);
                ApiError::AuthFailed(format!("Parse error: {}", e))
            })?;

        tracing::info!("Authentication successful, got token");
        Ok(data.user.auth_token)
    }

    /// Create a PIN for OAuth authentication (Step 1).
    ///
    /// Returns a PIN that the user must authorize at the auth_url.
    pub async fn create_pin(&self) -> Result<AuthPin, ApiError> {
        let url = format!("{}/api/v2/pins", PLEX_TV_URL);

        let response = self
            .http
            .post(&url)
            .header(HEADER_PLEX_PRODUCT, &self.client_info.product)
            .header(HEADER_PLEX_CLIENT_ID, &self.client_info.client_identifier)
            .form(&[("strong", "true")])
            .send()
            .await?;

        let pin_response: PinResponse = response.json().await?;

        let auth_url = format!(
            "{}#?clientID={}&code={}&context%5Bdevice%5D%5Bproduct%5D={}",
            PLEX_AUTH_URL,
            urlencoding::encode(&self.client_info.client_identifier),
            pin_response.code,
            urlencoding::encode(&self.client_info.product)
        );

        Ok(AuthPin {
            id: pin_response.id,
            code: pin_response.code,
            auth_url,
        })
    }

    /// Check if a PIN has been authorized (Step 2).
    ///
    /// Returns the auth token if authorized, None if still pending.
    pub async fn check_pin(&self, pin_id: u64, code: &str) -> Result<Option<String>, ApiError> {
        let url = format!("{}/api/v2/pins/{}", PLEX_TV_URL, pin_id);

        let response = self
            .http
            .get(&url)
            .header(HEADER_PLEX_CLIENT_ID, &self.client_info.client_identifier)
            .query(&[("code", code)])
            .send()
            .await?;

        let pin_response: PinCheckResponse = response.json().await?;
        Ok(pin_response.auth_token)
    }

    /// Verify an existing token is still valid.
    pub async fn verify_token(&self, token: &str) -> Result<PlexUser, ApiError> {
        let url = format!("{}/api/v2/user", PLEX_TV_URL);

        let response = self
            .http
            .get(&url)
            .header(HEADER_PLEX_PRODUCT, &self.client_info.product)
            .header(HEADER_PLEX_CLIENT_ID, &self.client_info.client_identifier)
            .header(HEADER_PLEX_TOKEN, token)
            .header("Accept", "application/json")
            .send()
            .await?;

        if response.status() == 401 {
            return Err(ApiError::InvalidToken);
        }

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            tracing::error!("Token verify failed: {} - {}", status, text);
            return Err(ApiError::AuthFailed(format!("Status {}", status)));
        }

        let text = response.text().await?;
        tracing::debug!("User response: {}", &text[..text.len().min(500)]);

        let user: PlexUser = serde_json::from_str(&text)
            .map_err(|e| {
                tracing::error!("Failed to parse user response: {}", e);
                ApiError::AuthFailed(format!("Parse error: {}", e))
            })?;
        Ok(user)
    }

    /// Get available servers for an authenticated user.
    pub async fn get_servers(&self, token: &str) -> Result<Vec<PlexServer>, ApiError> {
        let url = format!("{}/api/v2/resources", PLEX_TV_URL);

        let response = self
            .http
            .get(&url)
            .header(HEADER_PLEX_TOKEN, token)
            .header(HEADER_PLEX_CLIENT_ID, &self.client_info.client_identifier)
            .header("Accept", "application/json")
            .query(&[("includeHttps", "1"), ("includeRelay", "1")])
            .send()
            .await?;

        let resources: Vec<ResourceResponse> = response.json().await?;

        let servers = resources
            .into_iter()
            .filter(|r| r.provides.contains("server"))
            .map(|r| PlexServer {
                name: r.name,
                client_identifier: r.client_identifier,
                connections: r
                    .connections
                    .into_iter()
                    .map(|c| super::models::ServerConnection {
                        uri: c.uri,
                        local: c.local,
                        relay: c.relay,
                    })
                    .collect(),
                owned: r.owned,
            })
            .collect();

        Ok(servers)
    }

    /// Save auth token to file.
    pub fn save_token(&self, token: &str, user: Option<&PlexUser>) -> Result<(), ApiError> {
        let paths = crate::config::XdgPaths::new("textamp");
        paths.ensure_dirs().map_err(|e| ApiError::AuthFailed(e.to_string()))?;

        // Preserve existing server info if present
        let existing = Self::load_token();
        let stored = StoredAuth {
            token: token.to_string(),
            user_id: user.map(|u| u.id),
            username: user.map(|u| u.username.clone()),
            client_identifier: self.client_info.client_identifier.clone(),
            server_url: existing.as_ref().and_then(|e| e.server_url.clone()),
            server_identifier: existing.as_ref().and_then(|e| e.server_identifier.clone()),
            server_name: existing.as_ref().and_then(|e| e.server_name.clone()),
            has_plex_pass: user.map(|u| u.has_plex_pass()).unwrap_or(false),
        };

        let toml_str = toml::to_string(&stored)
            .map_err(|e| ApiError::AuthFailed(e.to_string()))?;

        std::fs::write(paths.token_file(), toml_str)
            .map_err(|e| ApiError::AuthFailed(e.to_string()))?;

        Ok(())
    }

    /// Update server info in stored auth (preserves token and other fields).
    pub fn update_server_info(server_info: &ServerInfo) -> Result<(), ApiError> {
        let paths = crate::config::XdgPaths::new("textamp");

        let Some(mut stored) = Self::load_token() else {
            return Err(ApiError::AuthFailed("No stored auth to update".to_string()));
        };

        stored.server_url = Some(server_info.url.clone());
        stored.server_identifier = Some(server_info.identifier.clone());
        stored.server_name = Some(server_info.name.clone());

        let toml_str = toml::to_string(&stored)
            .map_err(|e| ApiError::AuthFailed(e.to_string()))?;

        std::fs::write(paths.token_file(), toml_str)
            .map_err(|e| ApiError::AuthFailed(e.to_string()))?;

        tracing::info!("Persisted server info: {} ({})", server_info.name, server_info.url);
        Ok(())
    }

    /// Load stored auth token.
    pub fn load_token() -> Option<StoredAuth> {
        let paths = crate::config::XdgPaths::new("textamp");
        let path = paths.token_file();

        if path.exists() {
            let contents = std::fs::read_to_string(path).ok()?;
            toml::from_str(&contents).ok()
        } else {
            None
        }
    }

    /// Persist a marker recording which Plex account was last signed
    /// in plus the unix-epoch second the session ended (or started).
    /// This file survives `delete_token`; the marker is what lets
    /// sign-in decide whether the on-disk caches still belong to the
    /// user logging in.
    pub fn save_account_marker(username: &str) -> Result<(), std::io::Error> {
        let paths = crate::config::XdgPaths::new("textamp");
        paths.ensure_dirs()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let marker = AccountMarker { username: username.to_string(), last_seen_unix: now };
        let toml_str = toml::to_string(&marker)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(account_marker_path(&paths), toml_str)
    }

    /// Load the persisted account marker if present. Returns `None`
    /// when nothing has been written yet (first-run / fresh install).
    pub fn load_account_marker() -> Option<AccountMarker> {
        let paths = crate::config::XdgPaths::new("textamp");
        let p = account_marker_path(&paths);
        if !p.exists() { return None; }
        let contents = std::fs::read_to_string(p).ok()?;
        toml::from_str(&contents).ok()
    }

    /// Get available remote players (devices that provide "player" capability).
    ///
    /// Queries the plex.tv Resources API with full client identification headers.
    ///
    /// WORKAROUND: Apple TV frequently fails to report `presence: true` via the
    /// plex.tv cloud API, even when running and reachable on the local network.
    /// Both Plexamp and textamp are affected — the only reliable fix is toggling
    /// the Apple TV's "Allow access" setting off and back on. As a workaround,
    /// we cache Apple TV entries on first successful discovery and inject them
    /// into future results when they're missing. Remove this once Apple/Plex
    /// fix the presence reporting.
    pub async fn get_players(&self, token: &str) -> Result<Vec<super::models::RemotePlayer>, ApiError> {
        let url = format!("{}/api/v2/resources", PLEX_TV_URL);

        let response = self
            .http
            .get(&url)
            .header(HEADER_PLEX_TOKEN, token)
            .header(HEADER_PLEX_CLIENT_ID, &self.client_info.client_identifier)
            .header(HEADER_PLEX_PRODUCT, &self.client_info.product)
            .header(HEADER_PLEX_VERSION, &self.client_info.version)
            .header(HEADER_PLEX_DEVICE_NAME, &self.client_info.device_name)
            .header(HEADER_PLEX_PLATFORM, &self.client_info.platform)
            .header("Accept", "application/json")
            .query(&[("includeHttps", "1"), ("includeRelay", "1"), ("includeIPv6", "1")])
            .send()
            .await?;

        let resources: Vec<ResourceResponse> = response.json().await?;

        tracing::info!("Resources API returned {} devices:", resources.len());
        for r in &resources {
            tracing::info!(
                "  {} (product={:?}, provides={}, presence={}, owned={}, connections={})",
                r.name,
                r.product,
                r.provides,
                r.presence,
                r.owned,
                r.connections.len()
            );
        }

        // Collect Apple TV entries (present or not) for caching
        let apple_tvs_from_api: Vec<super::models::RemotePlayer> = resources
            .iter()
            .filter(|r| {
                let is_player = r.provides.contains("player") || r.provides.contains("client");
                is_player && Self::is_apple_tv(r)
            })
            .map(|r| super::models::RemotePlayer {
                name: r.name.clone(),
                client_identifier: r.client_identifier.clone(),
                connections: r.connections.iter()
                    .map(|c| super::models::ServerConnection {
                        uri: c.uri.clone(), local: c.local, relay: c.relay,
                    })
                    .collect(),
                owned: r.owned,
                product: r.product.clone().unwrap_or_default(),
                platform: r.platform.clone().unwrap_or_default(),
            })
            .collect();

        // Cache Apple TV entries if any were found in the API response
        if !apple_tvs_from_api.is_empty() {
            Self::save_apple_tv_cache(&apple_tvs_from_api);
        }

        let mut players: Vec<super::models::RemotePlayer> = resources
            .into_iter()
            .filter(|r| {
                let is_player = r.provides.contains("player") || r.provides.contains("client");
                tracing::info!("  filter: {} provides={} presence={} -> pass={}", r.name, r.provides, r.presence, is_player && r.presence);
                is_player && r.presence
            })
            .map(|r| super::models::RemotePlayer {
                name: r.name,
                client_identifier: r.client_identifier,
                connections: r
                    .connections
                    .into_iter()
                    .map(|c| super::models::ServerConnection {
                        uri: c.uri,
                        local: c.local,
                        relay: c.relay,
                    })
                    .collect(),
                owned: r.owned,
                product: r.product.unwrap_or_default(),
                platform: r.platform.unwrap_or_default(),
            })
            .collect();

        // If no Apple TV is present in the live results, add cached ones
        let has_live_apple_tv = players.iter().any(|p| {
            Self::is_apple_tv_player(p)
        });
        if !has_live_apple_tv {
            if let Some(cached) = Self::load_apple_tv_cache() {
                for atv in cached {
                    if !players.iter().any(|p| p.client_identifier == atv.client_identifier) {
                        tracing::info!("Adding cached Apple TV: {} ({})", atv.name, atv.product);
                        players.push(atv);
                    }
                }
            }
        }

        Ok(players)
    }

    /// Check if a ResourceResponse is an Apple TV device.
    /// Part of the Apple TV presence workaround — see get_players() doc comment.
    fn is_apple_tv(r: &ResourceResponse) -> bool {
        let product = r.product.as_deref().unwrap_or("");
        let platform = r.platform.as_deref().unwrap_or("");
        let name_lower = r.name.to_lowercase();
        product.contains("Plex for Apple TV")
            || platform.contains("tvOS")
            || name_lower.contains("apple tv")
    }

    /// Check if a RemotePlayer is an Apple TV device.
    /// Part of the Apple TV presence workaround — see get_players() doc comment.
    fn is_apple_tv_player(p: &super::models::RemotePlayer) -> bool {
        p.product.contains("Plex for Apple TV")
            || p.platform.contains("tvOS")
            || p.name.to_lowercase().contains("apple tv")
    }

    /// Cache path for Apple TV player data.
    /// Part of the Apple TV presence workaround — see get_players() doc comment.
    fn apple_tv_cache_path() -> std::path::PathBuf {
        let paths = crate::config::XdgPaths::new("textamp");
        paths.cache_dir.join("apple_tv_players.json")
    }

    /// Save Apple TV players to cache.
    /// Part of the Apple TV presence workaround — see get_players() doc comment.
    fn save_apple_tv_cache(players: &[super::models::RemotePlayer]) {
        let path = Self::apple_tv_cache_path();
        match serde_json::to_string(players) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    tracing::warn!("Failed to save Apple TV cache: {}", e);
                }
            }
            Err(e) => tracing::warn!("Failed to serialize Apple TV cache: {}", e),
        }
    }

    /// Load cached Apple TV players.
    /// Part of the Apple TV presence workaround — see get_players() doc comment.
    fn load_apple_tv_cache() -> Option<Vec<super::models::RemotePlayer>> {
        let path = Self::apple_tv_cache_path();
        let data = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// Delete stored auth token (logout).
    pub fn delete_token() -> Result<(), std::io::Error> {
        let paths = crate::config::XdgPaths::new("textamp");
        let path = paths.token_file();

        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}

impl Default for PlexAuth {
    fn default() -> Self {
        Self::new()
    }
}

// Response types for Plex auth API

#[derive(Debug, Deserialize)]
struct SignInResponse {
    user: SignInUser,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignInUser {
    auth_token: String,
}

#[derive(Debug, Deserialize)]
struct PinResponse {
    id: u64,
    code: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PinCheckResponse {
    #[allow(dead_code)]
    id: u64,
    #[allow(dead_code)]
    code: String,
    auth_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResourceResponse {
    name: String,
    client_identifier: String,
    provides: String,
    #[serde(default)]
    owned: bool,
    #[serde(default)]
    presence: bool,
    #[serde(default)]
    product: Option<String>,
    #[serde(default)]
    platform: Option<String>,
    #[serde(default)]
    connections: Vec<ConnectionResponse>,
}

#[derive(Debug, Deserialize)]
struct ConnectionResponse {
    uri: String,
    #[serde(default)]
    local: bool,
    #[serde(default)]
    relay: bool,
}
