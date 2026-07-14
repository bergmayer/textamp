//! Application event payloads.
//!
//! Every event body carried by the application channel lives here. This
//! module imports nothing from ratatui or crossterm so it stays decoupled
//! from the rendering layer.
//!
//! The top-level `Event` enum in `crate::app::event` wraps these payloads
//! and adds the terminal input variants (`Key`, `Mouse`, `Resize`).

use crate::plex::models::{Album, Artist, Genre, Hub, Library, Playlist, PlexServer, Station, Track, SearchResults};
use crate::services::WaveformData;
use crate::util::SecretString;

#[derive(Clone)]
pub enum AuthEvent {
    AuthSuccess {
        token: SecretString,
        username: String,
        server_url: String,
        server_identifier: Option<String>,
        servers: Vec<PlexServer>,
        client_identifier: String,
        has_plex_pass: bool,
    },
    AuthFailed(String),
    AuthShowLogin,
    AuthServersReady { token: SecretString, username: String, servers: Vec<PlexServer>, client_identifier: String, has_plex_pass: bool },
    AuthLoginFailed(String),
    /// Blocking account-marker/cache maintenance completed after sign-in.
    AuthStoragePrepared { warning: Option<String> },
    AuthPinReady { code: String, pin_id: u64 },
    ServersDiscovered(Vec<PlexServer>),
    ServerDiscoveryFailed(String),
    ServerConnectionSucceeded { server_name: String, url: String },
    ServerConnectionFailed { server_name: String },
}

impl std::fmt::Debug for AuthEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AuthSuccess {
                username,
                server_url,
                server_identifier,
                servers,
                client_identifier,
                has_plex_pass,
                ..
            } => formatter
                .debug_struct("AuthSuccess")
                .field("token", &"[REDACTED]")
                .field("username", username)
                .field("server_url", server_url)
                .field("server_identifier", server_identifier)
                .field("server_count", &servers.len())
                .field("client_identifier", client_identifier)
                .field("has_plex_pass", has_plex_pass)
                .finish(),
            Self::AuthServersReady {
                username,
                servers,
                client_identifier,
                has_plex_pass,
                ..
            } => formatter
                .debug_struct("AuthServersReady")
                .field("token", &"[REDACTED]")
                .field("username", username)
                .field("server_count", &servers.len())
                .field("client_identifier", client_identifier)
                .field("has_plex_pass", has_plex_pass)
                .finish(),
            Self::AuthFailed(error) => formatter.debug_tuple("AuthFailed").field(error).finish(),
            Self::AuthShowLogin => formatter.write_str("AuthShowLogin"),
            Self::AuthLoginFailed(error) => formatter
                .debug_tuple("AuthLoginFailed")
                .field(error)
                .finish(),
            Self::AuthStoragePrepared { warning } => formatter
                .debug_struct("AuthStoragePrepared")
                .field("warning", warning)
                .finish(),
            Self::AuthPinReady { pin_id, .. } => formatter
                .debug_struct("AuthPinReady")
                .field("code", &"[REDACTED]")
                .field("pin_id", pin_id)
                .finish(),
            Self::ServersDiscovered(servers) => formatter
                .debug_tuple("ServersDiscovered")
                .field(&servers.len())
                .finish(),
            Self::ServerDiscoveryFailed(error) => formatter
                .debug_tuple("ServerDiscoveryFailed")
                .field(error)
                .finish(),
            Self::ServerConnectionSucceeded { server_name, url } => formatter
                .debug_struct("ServerConnectionSucceeded")
                .field("server_name", server_name)
                .field("url", url)
                .finish(),
            Self::ServerConnectionFailed { server_name } => formatter
                .debug_struct("ServerConnectionFailed")
                .field("server_name", server_name)
                .finish(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum DataEvent {
    LibrariesLoaded {
        server_url: Option<String>,
        result: Result<Vec<Library>, crate::app::action::AsyncError>,
    },
    ServerLibrariesLoaded { server_identifier: String, server_name: String, libraries: Vec<Library> },
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
    ArtistsPageFailed { library_key: String },
    AlbumsLoaded(Vec<Album>),
    TracksLoaded(Vec<Track>),
    PlaylistsLoaded {
        server_url: Option<String>,
        result: Result<Vec<Playlist>, crate::app::action::AsyncError>,
    },
    HomeHubsLoaded(Vec<Hub>),
    ArtistLoaded(Artist),
    AlbumLoaded(Album),
    AlbumTracksLoaded { request_key: String, tracks: Vec<Track> },
    ArtistAlbumsLoaded { request_key: String, albums: Vec<Album> },
    ArtistAllTracksLoaded { request_key: String, tracks: Vec<Track> },
    CategoryTracksLoaded { request_key: String, tracks: Vec<Track> },
    CategoryAlbumsLoaded { albums: Vec<Album>, status_message: String },
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
    SimilarAlbumsLoaded { request_key: String, albums: Vec<Album> },
    SimilarTracksLoaded { request_key: String, tracks: Vec<Track> },
    /// Result of a `LoadTrackPaneSimilar` request — stored in the
    /// per-track HashMap, not the popup-shared `state.similar`.
    TrackPaneSimilarLoaded {
        server_url: Option<String>,
        rating_key: String,
        result: Result<Vec<Track>, crate::app::action::AsyncError>,
    },
    SimilarArtistsLoaded { request_key: String, artists: Vec<Artist> },
    RelatedDataLoaded { request_key: String, groups: Vec<crate::app::state::RelatedArtistGroup> },
    ScopedLoadError { request_key: String, message: String, connection_error: bool },
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
    PlaybackError { playback_id: Option<u64>, message: String },
    PositionUpdate(u64),
    BufferingStart,
    BufferingEnd { playback_id: u64 },
    TranscodeUrlReady {
        preparation_id: u64,
        track_key: String,
        result: Result<String, crate::app::action::AsyncError>,
    },
    RetryAfterDelay { playback_id: u64 },
}

#[derive(Debug, Clone)]
pub enum ArtworkEvent {
    ImageLoaded { key: String },
    ImageFailed { key: String, error: String },
    ArtworkLoaded { generation: u64, thumb_path: String, data: Vec<u8> },
    ArtworkFailed { generation: u64, thumb_path: String },
    AlbumArtLoaded { generation: u64, key: String, data: Vec<u8> },
    AlbumArtFailed { generation: u64, key: String },
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
    SubfolderRefreshed { library_key: String, folder_key: String, cached_folder: crate::plex::CachedFolder },
    FolderRootLoaded { library_key: String, lib_title: String, items: Vec<crate::plex::models::FolderItem> },
    FolderContentsLoaded { library_key: String, folder_key: String, items: Vec<crate::plex::models::FolderItem>, folder_path: Option<String>, item_path: Option<String>, replace_child: bool },
    FolderLoadFailed { library_key: String, pending_folder_key: Option<String>, message: String },
    FolderRefreshLoaded { library_key: String, folder_key: String, items: Vec<crate::plex::models::FolderItem>, folder_path: Option<String> },
    FolderPathDiscovered { library_key: String, folder_key: String, path: String },
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
        request_id: u64,
        albums: Vec<Album>,
        artist_only_keys: std::collections::HashSet<String>,
        track_artist_keys: std::collections::HashSet<String>,
        artist_compilation_map: std::collections::HashMap<String, Vec<String>>,
        single_artist_compilations: std::collections::HashMap<String, Vec<Album>>,
    },
    CompilationDetectionFailed { library_key: String, request_id: u64, error: String },
    LibraryCacheLoaded { library_key: String, cached: Box<crate::plex::CacheData> },
    LibraryCacheLoadFailed { library_key: String },
    PlaylistTracksPreloaded {
        library_key: String,
        playlist_key: String,
        tracks: Vec<Track>,
    },
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
    PlaylistTracksForMillerFailed {
        library_key: String,
        request_id: u64,
        playlist_key: String,
        error: crate::app::action::AsyncError,
    },
    /// First page of a lazy-loaded playlist column. `total` is the
    /// server-reported total — once the column has that many tracks
    /// the GUI stops asking for more.
    PlaylistFirstPageLoaded { library_key: String, request_id: u64, playlist_key: String, tracks: Vec<Track>, total: Option<u32> },
    /// Subsequent page appended to an already-built playlist column.
    PlaylistMorePageLoaded { library_key: String, playlist_key: String, offset: u32, tracks: Vec<Track>, total: Option<u32> },
    PlaylistMorePageFailed { library_key: String, playlist_key: String, offset: u32, error: crate::app::action::AsyncError },
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
    ListFilterCompleted { version: u64, column_results: Vec<crate::app::state::ListFilterResults> },
    DjTracksReady { tracks: Vec<Track>, insert_next: bool, error: Option<String> },
    DjBatchReady { inserts: Vec<(usize, Vec<Track>)> },
    RemixBatchReady { inserts: Vec<(usize, Vec<Track>)> },
    RemixDoppelgangerReady { replacements: Vec<(usize, Track)> },
    ArtistRadioComplete { tracks: Vec<Track> },
    ArtistBioLoaded { server_url: Option<String>, request_id: u64, artist_name: String, bio: String, thumb: Option<String> },
    ArtistBioArtworkLoaded { server_url: Option<String>, request_id: u64, data: Vec<u8>, thumb: String },
}

#[derive(Debug, Clone)]
pub enum RemoteCommandKind {
    Pause,
    Resume,
    Stop { was_playing: bool, position_ms: u64 },
    Seek { position_ms: u64 },
    SetVolume,
}

#[derive(Debug, Clone)]
pub enum RemoteEvent {
    PlayersDiscovered(Vec<crate::plex::models::RemotePlayer>),
    PlayerDiscoveryFailed(String),
    RemotePlayerStatus {
        player_id: String,
        session_found: bool,
        playing: bool,
        position_ms: u64,
        track_key: Option<String>,
        finished: bool,
    },
    RemotePlayResult {
        player_id: String,
        playback_id: u64,
        track_key: String,
        error: Option<String>,
    },
    RemoteCommandResult {
        player_id: String,
        command: RemoteCommandKind,
        error: Option<String>,
    },
    RemotePlayerError { player_id: String, error: String },
}
