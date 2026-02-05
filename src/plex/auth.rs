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
}

/// Server info for persistence.
#[derive(Debug, Clone)]
pub struct ServerInfo {
    pub url: String,
    pub identifier: String,
    pub name: String,
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
        };

        let yaml = serde_yaml::to_string(&stored)
            .map_err(|e| ApiError::AuthFailed(e.to_string()))?;

        std::fs::write(paths.token_file(), yaml)
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

        let yaml = serde_yaml::to_string(&stored)
            .map_err(|e| ApiError::AuthFailed(e.to_string()))?;

        std::fs::write(paths.token_file(), yaml)
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
            serde_yaml::from_str(&contents).ok()
        } else {
            None
        }
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
