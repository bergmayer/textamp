//! API constants for Plex Media Server.
//!
//! Centralizes magic strings to avoid typos and enable easy updates.

// ============================================================================
// HTTP Headers
// ============================================================================

/// Plex product name header.
pub const HEADER_PLEX_PRODUCT: &str = "X-Plex-Product";

/// Plex version header.
pub const HEADER_PLEX_VERSION: &str = "X-Plex-Version";

/// Plex client identifier header (unique device ID).
pub const HEADER_PLEX_CLIENT_ID: &str = "X-Plex-Client-Identifier";

/// Plex device name header.
pub const HEADER_PLEX_DEVICE_NAME: &str = "X-Plex-Device-Name";

/// Plex platform header (OS name).
pub const HEADER_PLEX_PLATFORM: &str = "X-Plex-Platform";

/// Plex authentication token header.
pub const HEADER_PLEX_TOKEN: &str = "X-Plex-Token";

/// Plex session identifier header (unique per playback session).
pub const HEADER_PLEX_SESSION_ID: &str = "X-Plex-Session-Identifier";

// ============================================================================
// Plex API Endpoints
// ============================================================================

/// Library sections (all libraries).
pub const EP_LIBRARY_SECTIONS: &str = "/library/sections";

/// Library metadata base path.
pub const EP_LIBRARY_METADATA: &str = "/library/metadata";

/// Playlists endpoint.
pub const EP_PLAYLISTS: &str = "/playlists";

/// Audio playlists query.
pub const EP_PLAYLISTS_AUDIO: &str = "/playlists?playlistType=audio";

/// Hubs (discovery) endpoint.
pub const EP_HUBS: &str = "/hubs";

/// Search endpoint.
pub const EP_HUBS_SEARCH: &str = "/hubs/search";

/// Play queues endpoint.
pub const EP_PLAY_QUEUES: &str = "/playQueues";

/// Timeline reporting endpoint.
pub const EP_TIMELINE: &str = "/:/timeline";

/// Scrobble endpoint.
pub const EP_SCROBBLE: &str = "/:/scrobble";

/// Photo transcoding endpoint.
pub const EP_PHOTO_TRANSCODE: &str = "/photo/:/transcode";

/// Audio transcoding endpoint (must use /audio/ not /music/ for track streaming).
pub const EP_AUDIO_TRANSCODE: &str = "/audio/:/transcode/universal/start";

// ============================================================================
// Plex Media Types
// ============================================================================

/// Artist type ID in Plex.
pub const TYPE_ARTIST: u8 = 8;

/// Album type ID in Plex.
pub const TYPE_ALBUM: u8 = 9;

/// Track type ID in Plex.
pub const TYPE_TRACK: u8 = 10;

// ============================================================================
// Query Parameters
// ============================================================================

/// Container start parameter for pagination.
pub const PARAM_CONTAINER_START: &str = "X-Plex-Container-Start";

/// Container size parameter for pagination.
pub const PARAM_CONTAINER_SIZE: &str = "X-Plex-Container-Size";

// ============================================================================
// Authentication URLs
// ============================================================================

/// Plex.tv API base URL.
pub const PLEX_TV_API: &str = "https://plex.tv/api/v2";

/// Plex.tv PIN endpoint.
pub const PLEX_TV_PINS: &str = "https://plex.tv/api/v2/pins";

/// Plex.tv user endpoint.
pub const PLEX_TV_USER: &str = "https://plex.tv/api/v2/user";

/// Plex.tv sign-in endpoint.
pub const PLEX_TV_SIGNIN: &str = "https://plex.tv/api/v2/users/signin";

/// Plex.tv resources endpoint (servers).
pub const PLEX_TV_RESOURCES: &str = "https://plex.tv/api/v2/resources";

// ============================================================================
// Default Values
// ============================================================================

/// Default HTTP request timeout in seconds.
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Default search result limit.
pub const DEFAULT_SEARCH_LIMIT: u32 = 50;

/// Default similar items limit.
pub const DEFAULT_SIMILAR_LIMIT: u32 = 10;

/// Timeout for connection testing (in seconds).
/// Keep short - local connections respond in <100ms, remote in <2s.
/// Only relay connections might need longer, but 5s is sufficient.
pub const CONNECTION_TEST_TIMEOUT_SECS: u64 = 5;

// ============================================================================
// Cache Staleness Thresholds
// ============================================================================

/// Cache staleness threshold (72 hours) — Tier 1.
/// Active category is refreshed on view navigation if older than this.
pub const CACHE_STALE_THRESHOLD_SECS: u64 = 72 * 60 * 60;

/// Very stale cache threshold (32 days) — Tier 2.
/// Non-active categories are refreshed on view navigation if older than this.
pub const CACHE_VERY_STALE_THRESHOLD_SECS: u64 = 32 * 24 * 60 * 60;
