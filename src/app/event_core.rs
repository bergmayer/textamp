//! Application event payloads.
//!
//! Every event body carried by the application channel lives here. This
//! module imports nothing from ratatui or crossterm so it stays decoupled
//! from the rendering layer.
//!
//! The top-level `Event` enum in `crate::app::event` wraps these payloads
//! and adds the terminal input variants (`Key`, `Mouse`, `Resize`).

use crate::library::models::{Album, Artist, Playlist, Station, Track};
use crate::services::WaveformData;

#[derive(Debug, Clone)]
pub enum DataEvent {
    ArtistsLoaded {
        library_key: String,
        result: Result<Vec<Artist>, crate::app::action::AsyncError>,
    },
    ArtistsPageLoaded {
        library_key: String,
        selected_key: Option<String>,
        artists: Vec<Artist>,
        total: u32,
    },
    ArtistsPageFailed {
        library_key: String,
    },
    AlbumsLoaded(Vec<Album>),
    TracksLoaded(Vec<Track>),
    PlaylistsLoaded {
        server_url: Option<String>,
        result: Result<Vec<Playlist>, crate::app::action::AsyncError>,
    },

    ArtistLoaded(Artist),
    AlbumLoaded(Album),
    AlbumTracksLoaded {
        request_key: String,
        tracks: Vec<Track>,
    },
    ArtistAlbumsLoaded {
        request_key: String,
        albums: Vec<Album>,
    },
    ArtistAllTracksLoaded {
        request_key: String,
        tracks: Vec<Track>,
    },
    CategoryTracksLoaded {
        request_key: String,
        tracks: Vec<Track>,
    },
    CategoryAlbumsLoaded {
        albums: Vec<Album>,
        status_message: String,
    },
    AllAlbumsForMillerLoaded {
        library_key: String,
        request_id: u64,
        replace_child: bool,
        albums: Vec<Album>,
    },
    AllAlbumsForMillerFailed {
        library_key: String,
        request_id: u64,
        error: crate::app::action::AsyncError,
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

    ApiError(String),
}

#[derive(Debug, Clone)]
pub enum PlaybackEvent {
    TrackStarted,
    TrackEnded,
    PlaybackPaused,
    PlaybackResumed,
    PlaybackStopped,
    PlaybackError {
        playback_id: Option<u64>,
        message: String,
    },
    SeekFailed {
        playback_id: u64,
        message: String,
    },
    PositionUpdate(u64),
    BufferingStart,
    BufferingEnd {
        playback_id: u64,
    },

    RetryAfterDelay {
        playback_id: u64,
    },
}

#[derive(Debug, Clone)]
pub enum ArtworkEvent {
    ImageLoaded {
        key: String,
    },
    ImageFailed {
        key: String,
        error: String,
    },
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
    ArtworkCacheStats {
        count: usize,
        total_bytes: u64,
    },
}

#[derive(Debug, Clone)]
pub enum CacheEvent {
    LibraryCacheStats {
        total_bytes: u64,
        breakdown: Vec<(String, u64)>,
    },
    WaveformCacheStats {
        count: usize,
        total_bytes: u64,
    },
}

#[derive(Debug, Clone)]
pub enum VisualizerEvent {
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
    /// Subsequent page appended to an already-built playlist column.
    PlaylistMorePageLoaded {
        library_key: String,
        playlist_key: String,
        offset: u32,
        tracks: Vec<Track>,
        total: Option<u32>,
    },
    PlaylistMorePageFailed {
        library_key: String,
        playlist_key: String,
        offset: u32,
        error: crate::app::action::AsyncError,
    },
}

#[derive(Debug, Clone)]
pub enum UiEvent {
    AdventureLauncherAlbumsLoaded {
        server_url: Option<String>,
        request_id: u64,
        artist_key: String,
        artist_name: String,
        result: Result<Vec<Album>, crate::app::action::AsyncError>,
    },
    AdventureLauncherTracksLoaded {
        server_url: Option<String>,
        request_id: u64,
        album_key: String,
        album_title: String,
        artist_name: String,
        result: Result<Vec<Track>, crate::app::action::AsyncError>,
    },
    ListFilterCompleted {
        version: u64,
        column_results: Vec<crate::app::state::ListFilterResults>,
    },
    DjTracksReady {
        result: Result<Vec<Track>, crate::app::action::AsyncError>,
        insert_next: bool,
    },
    DjBatchReady {
        inserts: Vec<(usize, Vec<Track>)>,
    },
    RemixBatchReady {
        outcome: crate::app::action::AsyncBatchOutcome<Vec<(usize, Vec<Track>)>>,
    },
    RemixDoppelgangerReady {
        outcome: crate::app::action::AsyncBatchOutcome<Vec<(usize, Track)>>,
    },
    ArtistRadioComplete {
        outcome: crate::app::action::AsyncBatchOutcome<Vec<Track>>,
    },
    ArtistBioLoaded {
        generation: u64,
        request_id: u64,
        result: Result<crate::services::biography::Biography, String>,
    },
}
