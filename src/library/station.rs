//! Shared radio intent and presentation, independent of provider transports.
use serde::{Deserialize, Serialize};

/// Standard recipes for adapters backed by a catalog or directory listings.
/// server supplies its own server station list; all use the same station UI.
pub fn standard_stations(
    prefix: &str,
    capabilities: super::capabilities::Capabilities,
) -> Vec<Station> {
    use super::capabilities::Feature::*;
    [
        ("library", "Library Radio", false, LibraryRadio),
        ("randomArtist", "Random Artist Radio", false, ArtistRadio),
        ("randomAlbum", "Random Album Radio", false, AlbumRadio),
        ("deepCuts", "Deep Cuts Radio", false, PlayCounts),
        ("timeTravel", "Time Travel Radio", false, ReleaseYears),
        ("onThisDay", "On This Day", false, ReleaseDates),
        ("mood", "Mood Radio", true, Moods),
        ("style", "Style / Genre Radio", true, Genres),
        ("decade", "Decade Radio", true, ReleaseYears),
        ("artistMix", "Artist Mix", false, RelatedArtists),
        ("albumMix", "Album Mix", false, SonicSimilarity),
        ("sonic", "Sonic Radio", false, SonicSimilarity),
    ]
    .into_iter()
    .filter(|(_, _, _, feature)| capabilities.supports(*feature))
    .map(|(id, title, category, _)| Station {
        key: format!("{prefix}/{id}"),
        title: title.into(),
        station_type: if category {
            "station.category"
        } else {
            "station"
        }
        .into(),
        identifier: Some(id.into()),
        thumb: None,
        art: None,
        description: None,
    })
    .collect()
}

/// The request that produces a radio queue, shared by startup and refill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RadioSource {
    Artist(String),
    /// Track identity used to seed sonic recommendations, independent of provider.
    Sonic(String),
    Station(String),
}

impl RadioSource {
    pub fn station_key(&self) -> Option<&str> {
        match self {
            Self::Station(key) => Some(key),
            Self::Artist(_) | Self::Sonic(_) => None,
        }
    }
}

/// A station offered by a library. Keys are opaque to shared input and UI.
/// Serde names preserve existing cache and server wire compatibility.
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
        let id = self.identifier.as_deref().unwrap_or("").to_lowercase();
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
        // Filtered choices retain the mood/style/decade identifier, but their
        // query identifies a playable leaf, not another navigation category.
        if self.key.contains("?id=") || self.key.contains("?title=") {
            return false;
        }
        // Synthetic category stations have type "station.category"
        if self.station_type == "station.category" {
            return true;
        }
        // Station types that are containers (have sub-stations)
        let kind = self.kind();
        matches!(
            kind,
            StationKind::Mood | StationKind::Style | StationKind::Decade
        )
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
