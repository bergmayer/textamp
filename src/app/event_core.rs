//! Application event payloads.
//!
//! Every event body carried by the application channel lives here. This
//! module imports nothing from ratatui or crossterm so it stays decoupled
//! from the rendering layer.
//!
//! The top-level `Event` enum in `crate::app::event` wraps these payloads
//! and adds the terminal input variants (`Key`, `Mouse`, `Resize`).

use crate::library::models::{Album, Artist, Station, Track};
use crate::media::WaveformData;

#[derive(Debug, Clone)]
pub enum DataEvent {
    SimilarLoadFailed {
        request_key: String,
        message: String,
    },
    SimilarAlbumsLoaded {
        request_key: String,
        albums: Vec<Album>,
    },
    SimilarTracksLoaded {
        request_key: String,
        tracks: Vec<Track>,
    },
    /// Result of a `LoadTrackPaneSimilar` request — stored in the
    /// per-track HashMap, not the popup-shared `state.similar`.
    TrackPaneSimilarLoaded {
        server_url: Option<String>,
        rating_key: String,
        result: Result<Vec<Track>, crate::app::action::AsyncError>,
    },
    SimilarArtistsLoaded {
        request_key: String,
        artists: Vec<Artist>,
    },
    RelatedDataLoaded {
        request_key: String,
        groups: Vec<crate::app::state::RelatedArtistGroup>,
    },
    ScopedLoadError {
        request_key: String,
        message: String,
    },
}

#[derive(Debug, Clone)]
pub enum PlaybackEvent {
    TrackEnded,

    PlaybackError {
        playback_id: Option<u64>,
        message: String,
    },
    SeekFailed {
        playback_id: u64,
        message: String,
    },

    BufferingEnd {
        playback_id: u64,
    },

    RetryAfterDelay {
        playback_id: u64,
    },
}

#[derive(Debug, Clone)]
pub enum ArtworkEvent {
    ArtworkLoaded {
        generation: u64,
        thumb_path: String,
        data: Vec<u8>,
    },
    ArtworkFailed {
        generation: u64,
        thumb_path: String,
    },
    AlbumArtLoaded {
        generation: u64,
        key: String,
        data: Vec<u8>,
    },
    AlbumArtFailed {
        generation: u64,
        key: String,
    },
}

#[derive(Debug, Clone)]
pub enum CacheEvent {
    WaveformCacheStats { count: usize, total_bytes: u64 },
}

#[derive(Debug, Clone)]
pub enum VisualizerEvent {
    AnalysisProgress {
        track_key: String,
        progress: crate::app::state::AnalysisProgress,
    },
    WaveformGenerated {
        track_key: String,
        data: WaveformData,
    },
    WaveformFailed {
        track_key: String,
        error: String,
    },
    WaveformCacheHit {
        track_key: String,
        data: WaveformData,
    },
    WaveformRetry(String),
    SpectrogramGenerated {
        track_key: String,
        data: crate::media::SpectrogramData,
    },
    SpectrogramCacheMiss {
        track_key: String,
    },
    SpectrogramFailed {
        track_key: String,
        error: String,
    },
    SpectrogramCacheHit {
        track_key: String,
        data: crate::media::SpectrogramData,
    },
}

#[derive(Debug, Clone)]
pub enum RadioEvent {
    StationTracksLoaded {
        station: crate::app::state::ActiveStation,
        tracks: Vec<Track>,
        time_travel_decades: Vec<String>,
        time_travel_index: Option<usize>,
    },
    StationLoadFailed {
        error: crate::app::action::AsyncError,
    },
    StationChildrenFailed(crate::app::action::AsyncError),
    StationChildrenLoaded {
        station_key: String,
        station_title: String,
        children: Vec<Station>,
    },
    RadioTracksLoaded {
        result: Result<Vec<Track>, crate::app::action::AsyncError>,
        time_travel_index: Option<usize>,
    },
}

/// Catalog playlist paging is independent of radio playback and its generation.
#[derive(Debug, Clone)]
pub enum PlaylistEvent {
    PlaylistTracksForMillerFailed {
        library_key: String,
        request_id: u64,
        playlist_key: String,
        error: crate::app::action::AsyncError,
    },
    /// First page of a lazy-loaded playlist column. `total` is the
    /// server-reported total — once the column has that many tracks
    /// the GUI stops asking for more.
    PlaylistFirstPageLoaded {
        library_key: String,
        request_id: u64,
        playlist_key: String,
        tracks: Vec<Track>,
        total: Option<u32>,
    },
}

#[derive(Debug, Clone)]
pub enum UiEvent {
    ListFilterCompleted {
        version: u64,
        generation: u64,
        results: crate::app::state::ListFilterResults,
    },

    ArtistInfoLoaded {
        generation: u64,
        request_id: u64,
        artist_key: String,
        result: Result<crate::services::biography::Biography, String>,
    },
    ArtistBioLoaded {
        generation: u64,
        request_id: u64,
        result: Result<crate::services::biography::Biography, String>,
    },
}
