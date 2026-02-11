//! Plex API data models.

mod artist;
mod folder;
mod genre;
mod hub;
mod library;
mod playlist;
mod search;
mod serde_helpers;
mod sonic;

pub use artist::{Album, AlbumsResponse, Artist, ArtistsResponse, Media, MediaPart, Track, TracksResponse};
pub use folder::{
    FolderColumn, FolderContainer, FolderDirectory, FolderItem, FolderItemType,
    FolderMetadata, FolderNavigationState, FolderResponse,
};
pub use genre::{Genre, GenresResponse};
pub use hub::{Hub, HubItem, HubType, HubsResponse, PlayQueueResponse, Station, StationKind, StationsResponse};
pub use library::{Library, LibrarySectionsResponse, LibraryType};
pub use playlist::{Playlist, PlaylistsResponse};
pub use search::{SearchResponse, SearchResults};
pub use sonic::{RelatedHub, RelatedResponse, SonicSimilar};

use serde::{Deserialize, Serialize};

/// Common wrapper for Plex API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaContainer<T> {
    #[serde(default)]
    pub size: u32,
    #[serde(default)]
    pub total_size: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
    #[serde(flatten)]
    pub content: T,
}

/// Plex user information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlexUser {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub subscription: Option<PlexSubscription>,
}

impl PlexUser {
    /// Check if user has an active Plex Pass subscription.
    pub fn has_plex_pass(&self) -> bool {
        self.subscription.as_ref().map(|s| s.active).unwrap_or(false)
    }
}

/// Plex subscription information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlexSubscription {
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub plan: Option<String>,
}

/// Plex server information.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexServer {
    pub name: String,
    pub client_identifier: String,
    #[serde(default)]
    pub connections: Vec<ServerConnection>,
    #[serde(default)]
    pub owned: bool,
}

/// Server connection details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConnection {
    pub uri: String,
    #[serde(default)]
    pub local: bool,
    #[serde(default)]
    pub relay: bool,
}

/// A remote Plex player device (Apple TV, Plexamp on phone, etc.).
#[derive(Debug, Clone)]
pub struct RemotePlayer {
    pub name: String,
    pub client_identifier: String,
    pub connections: Vec<ServerConnection>,
    pub owned: bool,
    pub product: String,
    pub platform: String,
}
