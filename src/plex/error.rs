//! API error types.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ApiError {
    #[error("HTTP client initialization failed: {0}")]
    ClientInitialization(String),

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// Transport failure whose source URL may contain credentials. The
    /// sanitized message deliberately omits the underlying reqwest error.
    #[error("Connection failed: {0}")]
    Connection(String),

    #[error("JSON parsing failed: {0}")]
    Json(#[from] serde_json::Error),

    #[error("No server selected")]
    NoServerSelected,

    #[error("Not authenticated")]
    NotAuthenticated,

    #[error("Invalid token")]
    InvalidToken,

    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    #[error("No media available for track")]
    NoMediaAvailable,

    #[error("Server error: {status} - {message}")]
    ServerError { status: u16, message: String },

    #[error("Library not found: {0}")]
    LibraryNotFound(String),

    #[error("Item not found: {0}")]
    ItemNotFound(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Invalid header value: {0}")]
    InvalidHeader(String),

    #[error("HTTP response exceeded the {limit_bytes} byte safety limit")]
    ResponseTooLarge { limit_bytes: usize },
}

impl ApiError {
    /// Returns true if the error indicates the server is unreachable
    /// (connection refused, timeout, DNS failure, etc.).
    pub fn is_connection_error(&self) -> bool {
        match self {
            ApiError::Http(e) => e.is_connect() || e.is_timeout(),
            ApiError::Connection(_) => true,
            ApiError::NoServerSelected => true,
            _ => false,
        }
    }
}
