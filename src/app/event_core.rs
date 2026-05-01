//! Platform-neutral event payloads.
//!
//! Every event body carried by the application channel lives here. This
//! module imports nothing from ratatui / crossterm / iced so it compiles
//! identically under every UI feature flag.
//!
//! The top-level `Event` enum in `crate::app::event` wraps these payloads.
//! Under `feature = "tui"`, `Event` also carries crossterm input variants
//! (`Key`, `Mouse`, `Resize`); under `feature = "gui"` it does not.

use crate::plex::models::{Album, Artist, Genre, Hub, Library, Playlist, PlexServer, Station, Track, SearchResults};
use crate::services::WaveformData;

#[derive(Debug, Clone)]
pub enum AuthEvent {
    AuthSuccess { token: String, username: String, server_url: String, servers: Vec<PlexServer>, client_identifier: String, has_plex_pass: bool },
    AuthFailed(String),
    AuthShowLogin,
    AuthServersReady { token: String, username: String, servers: Vec<PlexServer>, client_identifier: String, has_plex_pass: bool },
    AuthLoginFailed(String),
    AuthPinReady { code: String, pin_id: u64 },
    ServersDiscovered(Vec<PlexServer>),
    ServerDiscoveryFailed(String),
    ServerConnectionSucceeded { server_name: String, url: String },
    ServerConnectionFailed { server_name: String },
}

#[derive(Debug, Clone)]
pub enum DataEvent {
    LibrariesLoaded(Vec<Library>),
    ServerLibrariesLoaded { server_identifier: String, server_name: String, libraries: Vec<Library> },
    ArtistsLoaded(Vec<Artist>),
    AlbumsLoaded(Vec<Album>),
    TracksLoaded(Vec<Track>),
    PlaylistsLoaded(Vec<Playlist>),
    HomeHubsLoaded(Vec<Hub>),
    ArtistLoaded(Artist),
    AlbumLoaded(Album),
    AlbumTracksLoaded(Vec<Track>),
    ArtistAlbumsLoaded(Vec<Album>),
    ArtistAllTracksLoaded(Vec<Track>),
    CategoryTracksLoaded(Vec<Track>),
    CategoryAlbumsLoaded { albums: Vec<Album>, status_message: String },
    DataLoadError(String),
    AllAlbumsForMillerLoaded(Vec<Album>),
    SimilarAlbumsLoaded(Vec<Album>),
    SimilarTracksLoaded(Vec<Track>),
    /// Result of a `LoadTrackPaneSimilar` request — stored in the
    /// per-track HashMap, not the popup-shared `state.similar`.
    TrackPaneSimilarLoaded { rating_key: String, tracks: Vec<Track> },
    SimilarArtistsLoaded(Vec<Artist>),
    RelatedDataLoaded { groups: Vec<crate::app::state::RelatedArtistGroup> },
    SearchCompleted(SearchResults),
    TrackSearchCompleted { version: u64, tracks: Vec<Track> },
    /// Adventure-launcher-specific track search result. Carries a
    /// per-launcher version so stale callbacks (issued before the
    /// user kept typing) can be discarded by the events handler.
    AdventureTrackSearchCompleted { version: u64, tracks: Vec<Track> },
    ApiError(String),
}

#[derive(Debug, Clone)]
pub enum PlaybackEvent {
    TrackStarted,
    TrackEnded,
    PlaybackPaused,
    PlaybackResumed,
    PlaybackStopped,
    PlaybackError(String),
    PositionUpdate(u64),
    BufferingStart,
    BufferingEnd,
    RetryAfterDelay,
}

#[derive(Debug, Clone)]
pub enum ArtworkEvent {
    ImageLoaded { key: String },
    ImageFailed { key: String, error: String },
    ArtworkLoaded { thumb_path: String, data: Vec<u8> },
    ArtworkFailed { thumb_path: String },
    AlbumArtLoaded { key: String, data: Vec<u8> },
    AlbumArtFailed { key: String },
    ArtworkCacheStats { count: usize, total_bytes: u64 },
}

#[derive(Debug, Clone)]
pub enum FolderEvent {
    FoldersPreloaded { library_key: String, folder_state: crate::services::FolderNavigationState },
    SubfoldersPreloaded {
        library_key: String,
        entries: Vec<(String, crate::plex::CachedFolder)>,
        done: bool,
        valid_keys: Option<std::collections::HashSet<String>>,
    },
    SubfolderRefreshed { folder_key: String, cached_folder: crate::plex::CachedFolder },
    FolderRootLoaded { library_key: String, lib_title: String, items: Vec<crate::plex::models::FolderItem> },
    FolderContentsLoaded { folder_key: String, items: Vec<crate::plex::models::FolderItem>, folder_path: Option<String>, item_path: Option<String> },
    FolderLoadFailed(String),
    FolderRefreshLoaded { folder_key: String, items: Vec<crate::plex::models::FolderItem>, folder_path: Option<String> },
    FolderPathDiscovered { folder_key: String, path: String },
}

#[derive(Debug, Clone)]
pub enum PreloadEvent {
    ArtistsPreloaded { library_key: String, artists: Vec<Artist> },
    AlbumsPreloaded { library_key: String, albums: Vec<Album> },
    PlaylistsPreloaded { library_key: String, playlists: Vec<Playlist> },
    ArtistGenresPreloaded { library_key: String, genres: Vec<Genre> },
    AlbumGenresPreloaded { library_key: String, genres: Vec<Genre> },
    MoodsPreloaded { library_key: String, moods: Vec<Genre> },
    StylesPreloaded { library_key: String, styles: Vec<Genre> },
    /// Generic tag-list preload for new tag-style sections (decade, year,
    /// collection, country, label, format, studio). The category param
    /// tells the events handler which `library` field to populate.
    TagListPreloaded {
        library_key: String,
        category: crate::app::state::RefreshCategory,
        items: Vec<Genre>,
    },
    StationsPreloaded { library_key: String, stations: Vec<Station> },
    AllTracksPreloaded { library_key: String, tracks: Vec<Track> },
    PreloadFailed { category: String },
    CompilationsDetected {
        library_key: String,
        albums: Vec<Album>,
        artist_only_keys: std::collections::HashSet<String>,
        track_artist_keys: std::collections::HashSet<String>,
        artist_compilation_map: std::collections::HashMap<String, Vec<String>>,
        single_artist_compilations: std::collections::HashMap<String, Vec<Album>>,
    },
    LibraryCacheLoaded { library_key: String, cached: Box<crate::plex::CacheData> },
    LibraryCacheLoadFailed { library_key: String },
    PlaylistTracksPreloaded { playlist_key: String, tracks: Vec<Track> },
}

#[derive(Debug, Clone)]
pub enum CacheEvent {
    CacheSaved,
    CacheRefreshCompleted { category: crate::app::state::RefreshCategory, changed: bool },
    LibraryCacheStats { total_bytes: u64, breakdown: Vec<(String, u64)> },
    WaveformCacheStats { count: usize, total_bytes: u64 },
}

#[derive(Debug, Clone)]
pub enum VisualizerEvent {
    WaveformGenerated { track_key: String, data: WaveformData },
    WaveformFailed { track_key: String, error: String },
    WaveformCacheHit { track_key: String, data: WaveformData },
    WaveformRetry(String),
    SpectrogramGenerated { track_key: String, data: crate::plex::SpectrogramData },
    SpectrogramFailed { track_key: String, error: String },
    SpectrogramCacheHit { track_key: String, data: crate::plex::SpectrogramData },
}

#[derive(Debug, Clone)]
pub enum RadioEvent {
    StationTracksLoaded { station_key: String, station_title: String, tracks: Vec<Track>, time_travel_decades: Vec<String> },
    StationLoadFailed { station_key: String, error: String },
    StationChildrenLoaded { station_key: String, station_title: String, children: Vec<Station> },
    RadioTracksLoaded { tracks: Vec<Track>, time_travel_index: Option<usize> },
    PlaylistTracksForMillerLoaded { playlist_key: String, tracks: Vec<Track> },
    PlaylistTracksForMillerFailed { playlist_key: String, error: String },
    /// First page of a lazy-loaded playlist column. `total` is the
    /// server-reported total — once the column has that many tracks
    /// the GUI stops asking for more.
    PlaylistFirstPageLoaded { playlist_key: String, tracks: Vec<Track>, total: Option<u32> },
    /// Subsequent page appended to an already-built playlist column.
    PlaylistMorePageLoaded { playlist_key: String, tracks: Vec<Track>, total: Option<u32> },
    PlaylistMorePageFailed { playlist_key: String, error: String },
}

#[derive(Debug, Clone)]
pub enum UiEvent {
    AdventureLauncherAlbumsLoaded { artist_key: String, artist_name: String, albums: Vec<Album> },
    AdventureLauncherTracksLoaded { album_key: String, album_title: String, artist_name: String, tracks: Vec<Track> },
    ListFilterCompleted { version: u64, results: crate::app::state::ListFilterResults },
    DjTracksReady { tracks: Vec<Track>, insert_next: bool, error: Option<String> },
    DjBatchReady { inserts: Vec<(usize, Vec<Track>)> },
    RemixBatchReady { inserts: Vec<(usize, Vec<Track>)> },
    RemixDoppelgangerReady { replacements: Vec<(usize, Track)> },
    ArtistRadioComplete { tracks: Vec<Track> },
    ArtistBioLoaded { artist_name: String, bio: String, thumb: Option<String> },
    ArtistBioArtworkLoaded { data: Vec<u8>, thumb: String },
}

#[derive(Debug, Clone)]
pub enum RemoteEvent {
    PlayersDiscovered(Vec<crate::plex::models::RemotePlayer>),
    PlayerDiscoveryFailed(String),
    RemotePlayerStatus {
        session_found: bool,
        playing: bool,
        position_ms: u64,
        track_key: Option<String>,
        finished: bool,
    },
    RemotePlayerError(String),
}
