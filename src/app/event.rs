//! Application events.
//!
//! Events represent things that happen (input, async completions, etc.).

use crate::api::models::{Album, Artist, Genre, Hub, Library, Playlist, PlexServer, Station, Track, SearchResults};
use crate::services::WaveformData;
use crossterm::event::{KeyEvent, MouseEvent};

/// Application events.
#[derive(Debug, Clone)]
pub enum Event {
    // Terminal input events
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),

    // Periodic tick for animations/updates
    Tick,

    // Authentication events
    AuthSuccess { token: String, username: String, server_url: String, servers: Vec<PlexServer>, client_identifier: String, has_plex_pass: bool },
    AuthFailed(String),
    AuthShowLogin,  // No stored token - show login form
    AuthServersReady { token: String, username: String, servers: Vec<PlexServer>, client_identifier: String, has_plex_pass: bool },  // Login succeeded, select server
    AuthLoginFailed(String),  // Login failed with message
    AuthPinReady { code: String, pin_id: u64 },
    ServersDiscovered(Vec<PlexServer>),
    ServerDiscoveryFailed(String),
    ServerConnectionSucceeded { server_name: String, url: String },
    ServerConnectionFailed { server_name: String },

    // API response events
    LibrariesLoaded(Vec<Library>),
    /// Libraries loaded from another server (for multi-server support).
    ServerLibrariesLoaded {
        server_identifier: String,
        server_name: String,
        libraries: Vec<Library>,
    },
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
    /// All albums loaded asynchronously for the "All Artists" Miller column.
    AllAlbumsForMillerLoaded(Vec<Album>),
    SimilarAlbumsLoaded(Vec<Album>),
    SimilarTracksLoaded(Vec<Track>),
    RelatedDataLoaded { groups: Vec<crate::app::state::RelatedArtistGroup> },
    SearchCompleted(SearchResults),
    TrackSearchCompleted { version: u64, tracks: Vec<Track> },

    // API errors
    ApiError(String),

    // Playback events
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

    // Image loading events
    ImageLoaded { key: String },
    ImageFailed { key: String, error: String },

    // Artwork loading (non-blocking)
    ArtworkLoaded { thumb_path: String, data: Vec<u8> },
    ArtworkFailed { thumb_path: String },

    // Album art grid loading (for cover art view)
    AlbumArtLoaded { key: String, data: Vec<u8> },
    AlbumArtFailed { key: String },

    // Folder preloading (background)
    FoldersPreloaded { library_key: String, folder_state: crate::services::FolderNavigationState },
    /// Background subfolder pre-caching completed a batch.
    SubfoldersPreloaded {
        library_key: String,
        entries: Vec<(String, crate::plex::CachedFolder)>,
        done: bool,
    },
    /// Background subfolder warm-cache re-fetch completed.
    SubfolderRefreshed {
        folder_key: String,
        cached_folder: crate::plex::CachedFolder,
    },
    /// Async path discovery completed for a folder column.
    FolderPathDiscovered {
        folder_key: String,
        path: String,
    },
    /// Artwork cache stats computed in background.
    ArtworkCacheStats {
        count: usize,
        total_bytes: u64,
    },
    /// Library cache size and per-field breakdown for active library.
    LibraryCacheStats {
        total_bytes: u64,
        breakdown: Vec<(String, u64)>,
    },
    /// Waveform cache stats computed in background.
    WaveformCacheStats {
        count: usize,
        total_bytes: u64,
    },

    // Background data preloading (all events include library_key for race condition safety)
    ArtistsPreloaded { library_key: String, artists: Vec<Artist> },
    AlbumsPreloaded { library_key: String, albums: Vec<Album> },
    PlaylistsPreloaded { library_key: String, playlists: Vec<Playlist> },
    GenresPreloaded { library_key: String, genres: Vec<Genre> },
    ArtistGenresPreloaded { library_key: String, genres: Vec<Genre> },
    AlbumGenresPreloaded { library_key: String, genres: Vec<Genre> },
    MoodsPreloaded { library_key: String, moods: Vec<Genre> },
    StylesPreloaded { library_key: String, styles: Vec<Genre> },
    StationsPreloaded { library_key: String, stations: Vec<Station> },
    /// All tracks preloaded for compilation detection and track-level artist derivation.
    AllTracksPreloaded { library_key: String, tracks: Vec<Track> },
    /// A background preload failed (clear tracking so notification doesn't hang).
    PreloadFailed { category: String },
    /// Background compilation detection completed.
    CompilationsDetected {
        library_key: String,
        albums: Vec<Album>,
        artist_only_keys: std::collections::HashSet<String>,
        track_artist_keys: std::collections::HashSet<String>,
        /// Maps artist_key → Vec<album_rating_key> for compilation appearances.
        artist_compilation_map: std::collections::HashMap<String, Vec<String>>,
        /// Single-artist "compilations" mapped to actual artist key.
        single_artist_compilations: std::collections::HashMap<String, Vec<Album>>,
    },
    // Library switch (async cache load)
    LibraryCacheLoaded { library_key: String, cached: Box<crate::plex::CacheData> },
    LibraryCacheLoadFailed { library_key: String },

    // Cache management
    CacheSaved,
    CacheRefreshCompleted {
        category: crate::app::state::RefreshCategory,
        changed: bool,
    },

    // Waveform generation
    WaveformGenerated { track_key: String, data: WaveformData },
    WaveformFailed { track_key: String, error: String },
    WaveformCacheHit { track_key: String, data: WaveformData },
    WaveformRetry(String),

    // Spectrogram generation
    SpectrogramGenerated { track_key: String, data: crate::plex::SpectrogramData },
    SpectrogramFailed { track_key: String, error: String },
    SpectrogramCacheHit { track_key: String, data: crate::plex::SpectrogramData },

    // Station loading (background)
    StationTracksLoaded { station_key: String, station_title: String, tracks: Vec<Track>, time_travel_decades: Vec<String> },
    StationLoadFailed { station_key: String, error: String },
    StationChildrenLoaded { station_key: String, station_title: String, children: Vec<Station> },

    // Radio track fetching (background)
    RadioTracksLoaded { tracks: Vec<Track>, time_travel_index: Option<usize> },

    // Playlist tracks loading (non-blocking)
    PlaylistTracksForMillerLoaded { playlist_key: String, tracks: Vec<Track> },
    PlaylistTracksForMillerFailed { playlist_key: String, error: String },

    // Playlist tracks preloading (background, cache-only, no Miller column push)
    PlaylistTracksPreloaded { playlist_key: String, tracks: Vec<Track> },

    // Adventure launcher drill-down events
    AdventureLauncherAlbumsLoaded { artist_key: String, artist_name: String, albums: Vec<Album> },
    AdventureLauncherTracksLoaded { album_key: String, album_title: String, artist_name: String, tracks: Vec<Track> },

    // Inline list filter
    ListFilterCompleted {
        version: u64,
        results: crate::app::state::ListFilterResults,
    },

    // DJ mode
    DjTracksReady { tracks: Vec<Track>, insert_next: bool, error: Option<String> },
    /// Batch result from inserter DJ modes: (original_queue_index, tracks_to_insert_after)
    DjBatchReady { inserts: Vec<(usize, Vec<Track>)> },

    // Queue remix
    /// Batch result from remix operations: (original_queue_index, tracks_to_insert_after)
    RemixBatchReady { inserts: Vec<(usize, Vec<Track>)> },
    /// Doppelganger remix result: (queue_index, replacement_track)
    RemixDoppelgangerReady { replacements: Vec<(usize, Track)> },

    // Multi-artist radio
    ArtistRadioComplete { tracks: Vec<Track> },

    // Artist bio popup (F4)
    ArtistBioLoaded { artist_name: String, bio: String, thumb: Option<String> },
    ArtistBioArtworkLoaded { data: Vec<u8>, thumb: String },

    // Remote player control
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
