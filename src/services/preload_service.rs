//! Preload service for background data fetching.
//!
//! Provides utilities for spawning background tasks that preload library data.
//! This module helps reduce boilerplate in the event loop while keeping the
//! actual Event handling in the app layer.
//!
//! # Design
//!
//! The preload pattern is:
//! 1. Extract client connection info (server_url, token)
//! 2. Clone necessary parameters for the async task
//! 3. Spawn a tokio task that creates a temporary client
//! 4. Make the API call and send result through event channel
//!
//! This service provides helpers for steps 1-2 to reduce boilerplate.

use crate::plex::PlexClient;

/// Connection parameters needed to create a PlexClient in a background task.
///
/// IMPORTANT: The client_identifier MUST match the one the token was issued for,
/// otherwise Plex will reject requests with 400 errors.
#[derive(Debug, Clone)]
pub struct ConnectionParams {
    /// Server URL.
    pub server_url: String,
    /// Auth token (optional).
    pub token: Option<String>,
    /// Client identifier - must match token's issuance identifier.
    pub client_identifier: String,
}

impl ConnectionParams {
    /// Extract connection parameters from a PlexClient.
    ///
    /// Returns None if the client doesn't have a server URL configured.
    pub fn from_client(client: &PlexClient) -> Option<Self> {
        client.server_url().map(|url| Self {
            server_url: url.to_string(),
            token: client.token().map(|s| s.to_string()),
            client_identifier: client.client_identifier().to_string(),
        })
    }

    /// Create a new PlexClient with these connection parameters.
    pub fn create_client(&self) -> PlexClient {
        PlexClient::new_with_url(&self.server_url, self.token.as_deref(), &self.client_identifier)
    }
}

/// Service for preloading library data.
pub struct PreloadService;

impl PreloadService {
    /// Get connection parameters if available.
    ///
    /// This is a convenience wrapper around `ConnectionParams::from_client`.
    pub fn get_connection(client: &PlexClient) -> Option<ConnectionParams> {
        ConnectionParams::from_client(client)
    }

    /// Check if preloading is possible (i.e., client has a server configured).
    pub fn can_preload(client: &PlexClient) -> bool {
        client.server_url().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_params_create_client() {
        let params = ConnectionParams {
            server_url: "http://localhost:32400".to_string(),
            token: Some("test-token".to_string()),
            client_identifier: "test-client-id".to_string(),
        };

        let client = params.create_client();
        assert_eq!(client.server_url(), Some("http://localhost:32400"));
        assert_eq!(client.token(), Some("test-token"));
        assert_eq!(client.client_identifier(), "test-client-id");
    }

    #[test]
    fn test_connection_params_no_token() {
        let params = ConnectionParams {
            server_url: "http://localhost:32400".to_string(),
            token: None,
            client_identifier: "test-client-id".to_string(),
        };

        let client = params.create_client();
        assert_eq!(client.server_url(), Some("http://localhost:32400"));
        assert_eq!(client.token(), None);
        assert_eq!(client.client_identifier(), "test-client-id");
    }
}
