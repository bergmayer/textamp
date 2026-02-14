//! Hub models for Plex home screen.

use serde::{Deserialize, Serialize};

/// A hub/section on the home screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hub {
    pub hub_identifier: String,
    pub title: String,
    #[serde(rename = "type")]
    pub hub_type: String,
    #[serde(default)]
    pub hub_key: Option<String>,
    #[serde(default)]
    pub more: bool,
    #[serde(default)]
    pub size: u32,
    #[serde(default, rename = "Metadata")]
    pub metadata: Vec<HubMetadata>,
}

impl Hub {
    /// Get the hub type as an enum.
    pub fn get_type(&self) -> HubType {
        HubType::from(self.hub_identifier.as_str())
    }

    /// Check if this hub is music-related.
    pub fn is_music(&self) -> bool {
        // Check hub_identifier for music-related content
        self.hub_identifier.contains("music")
            || self.hub_type == "artist"
            || self.hub_type == "album"
            || self.hub_type == "track"
            || self.hub_type == "playlist"
    }
}

/// Hub type classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubType {
    MixesForYou,
    RecentlyPlayed,
    RecentlyAdded,
    OnThisDay,
    Stations,
    SimilarArtists,
    SimilarAlbums,
    Playlists,
    Custom,
}

impl From<&str> for HubType {
    fn from(s: &str) -> Self {
        match s {
            "music.recent.added" => HubType::RecentlyAdded,
            "music.recent.played" => HubType::RecentlyPlayed,
            "music.playlists" => HubType::Playlists,
            "hub.music.mixes" | "music.mixes" => HubType::MixesForYou,
            "music.onthisday" => HubType::OnThisDay,
            "music.stations" => HubType::Stations,
            s if s.contains("similar") => HubType::SimilarArtists,
            _ => HubType::Custom,
        }
    }
}

/// Hub metadata item (can be artist, album, track, or playlist).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubMetadata {
    pub rating_key: String,
    pub key: String,
    pub title: String,
    #[serde(rename = "type")]
    pub item_type: String,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub art: Option<String>,
    // Album/Track specific
    #[serde(default)]
    pub parent_title: Option<String>,
    #[serde(default)]
    pub grandparent_title: Option<String>,
    #[serde(default)]
    pub year: Option<u16>,
}

impl HubMetadata {
    /// Convert to a HubItem enum based on type.
    pub fn to_hub_item(&self) -> HubItem {
        match self.item_type.as_str() {
            "artist" => HubItem::Artist(self.clone()),
            "album" => HubItem::Album(self.clone()),
            "track" => HubItem::Track(self.clone()),
            "playlist" => HubItem::Playlist(self.clone()),
            _ => HubItem::Unknown(self.clone()),
        }
    }
}

/// Enumeration of possible hub items.
#[derive(Debug, Clone)]
pub enum HubItem {
    Artist(HubMetadata),
    Album(HubMetadata),
    Track(HubMetadata),
    Playlist(HubMetadata),
    Unknown(HubMetadata),
}

/// Response wrapper for hubs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct HubsResponse {
    pub media_container: HubsContainer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct HubsContainer {
    #[serde(default, rename = "Hub")]
    pub hub: Vec<Hub>,
}

/// A radio station from Plex.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Station {
    pub key: String,
    pub title: String,
    #[serde(rename = "type")]
    pub station_type: String,
    #[serde(default)]
    pub identifier: Option<String>,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub art: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

impl Station {
    /// Get the station kind based on its identifier or title.
    pub fn kind(&self) -> StationKind {
        let id = self.identifier.as_deref().unwrap_or("");
        let title_lower = self.title.to_lowercase();

        if id.contains("library") || title_lower.contains("library") {
            StationKind::LibraryRadio
        } else if id.contains("deepcuts") || title_lower.contains("deep cuts") {
            StationKind::DeepCuts
        } else if id.contains("timetravel") || title_lower.contains("time travel") {
            StationKind::TimeTravel
        } else if id.contains("randomalbum") || title_lower.contains("random album") {
            StationKind::RandomAlbum
        } else if id.contains("onthisday") || title_lower.contains("on this day") {
            StationKind::OnThisDay
        } else if id.contains("style") || title_lower.contains("style") {
            StationKind::Style
        } else if id.contains("mood") || title_lower.contains("mood") {
            StationKind::Mood
        } else if id.contains("decade") || title_lower.contains("decade") {
            StationKind::Decade
        } else if id.contains("artistmix") || title_lower.contains("artist mix") {
            StationKind::ArtistMix
        } else if id.contains("albummix") || title_lower.contains("album mix") {
            StationKind::AlbumMix
        } else {
            StationKind::Other
        }
    }

    /// Check if this station is a visual separator (non-selectable).
    pub fn is_separator(&self) -> bool {
        self.station_type == "separator"
    }

    /// Check if this station is a DJ mode item.
    pub fn is_dj_mode(&self) -> bool {
        self.station_type == "dj_mode"
    }

    /// Check if this station is a remix action item.
    pub fn is_remix(&self) -> bool {
        self.station_type == "remix"
    }

    /// Check if this station is an action item (non-playable, triggers an action).
    pub fn is_action(&self) -> bool {
        self.station_type == "action"
    }

    /// Check if this station is a category with sub-stations.
    /// Categories like "Mood Radio", "Style Radio", "Decade Radio" have children.
    pub fn is_category(&self) -> bool {
        // Synthetic category stations have type "station.category"
        if self.station_type == "station.category" {
            return true;
        }
        // Station types that are containers (have sub-stations)
        let kind = self.kind();
        matches!(kind, StationKind::Mood | StationKind::Style | StationKind::Decade)
    }
}

/// Known station types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StationKind {
    ArtistMix,
    AlbumMix,
    LibraryRadio,
    DeepCuts,
    TimeTravel,
    RandomAlbum,
    OnThisDay,
    Style,
    Mood,
    Decade,
    Other,
}

impl StationKind {
    pub fn label(&self) -> &'static str {
        match self {
            StationKind::ArtistMix => "Artist Mix Builder",
            StationKind::AlbumMix => "Album Mix Builder",
            StationKind::LibraryRadio => "Library Radio",
            StationKind::DeepCuts => "Deep Cuts Radio",
            StationKind::TimeTravel => "Time Travel Radio",
            StationKind::RandomAlbum => "Random Album Radio",
            StationKind::OnThisDay => "On This Day",
            StationKind::Style => "Style Radio",
            StationKind::Mood => "Mood Radio",
            StationKind::Decade => "Decade Radio",
            StationKind::Other => "Radio",
        }
    }
}

/// Response for stations hub request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StationsResponse {
    pub media_container: StationsContainer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StationsContainer {
    #[serde(default, rename = "Hub")]
    pub hub: Vec<StationsHub>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StationsHub {
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default, rename = "Directory")]
    pub directory: Vec<Station>,
    #[serde(default, rename = "Metadata")]
    pub metadata: Vec<Station>,
}

impl StationsHub {
    /// Get all stations from either Directory or Metadata fields.
    pub fn stations(&self) -> Vec<Station> {
        if !self.directory.is_empty() {
            self.directory.clone()
        } else {
            self.metadata.clone()
        }
    }
}

/// Response for playQueue creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlayQueueResponse {
    pub media_container: PlayQueueContainer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayQueueContainer {
    #[serde(default)]
    pub play_queue_id: Option<u64>,
    #[serde(default)]
    pub play_queue_selected_item_id: Option<u64>,
    #[serde(default, rename = "Metadata")]
    pub metadata: Vec<super::Track>,
}
