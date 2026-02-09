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
    SimilarAlbumsLoaded(Vec<Album>),
    SimilarTracksLoaded(Vec<Track>),
    SearchCompleted(SearchResults),
    GlobalSearchCompleted { version: u64, results: SearchResults },
    FilterSearchCompleted { version: u64, results: SearchResults },

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
    /// Artwork cache stats computed in background.
    ArtworkCacheStats {
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
    RecentlyAddedPreloaded { library_key: String, albums: Vec<Album> },
    RecentlyPlayedPreloaded { library_key: String, albums: Vec<Album> },

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

    // Station loading (background)
    StationTracksLoaded { station_key: String, station_title: String, tracks: Vec<Track>, time_travel_decades: Vec<String> },
    StationLoadFailed { station_key: String, error: String },
    StationChildrenLoaded { station_key: String, station_title: String, children: Vec<Station> },

    // Album radio loading (background)
    AlbumRadioTracksLoaded { tracks: Vec<Track> },
    AlbumRadioLoadFailed { error: String },

    // Radio track fetching (background)
    RadioTracksLoaded { tracks: Vec<Track>, time_travel_index: Option<usize> },

    // Playlist tracks loading (non-blocking)
    PlaylistTracksForMillerLoaded { playlist_key: String, tracks: Vec<Track> },
    PlaylistTracksForMillerFailed { playlist_key: String, error: String },

    // Playlist tracks preloading (background, cache-only, no Miller column push)
    PlaylistTracksPreloaded { playlist_key: String, tracks: Vec<Track> },

    // Inline list filter
    ListFilterCompleted {
        version: u64,
        results: crate::app::state::ListFilterResults,
    },
}
