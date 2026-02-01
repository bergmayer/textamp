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
    AuthSuccess { token: String, username: String, server_url: String, servers: Vec<PlexServer> },
    AuthFailed(String),
    AuthShowLogin,  // No stored token - show login form
    AuthServersReady { token: String, username: String, servers: Vec<PlexServer> },  // Login succeeded, select server
    AuthLoginFailed(String),  // Login failed with message
    AuthPinReady { code: String, pin_id: u64 },
    ServersDiscovered(Vec<PlexServer>),
    ServerDiscoveryFailed(String),

    // API response events
    LibrariesLoaded(Vec<Library>),
    ArtistsLoaded(Vec<Artist>),
    AlbumsLoaded(Vec<Album>),
    TracksLoaded(Vec<Track>),
    PlaylistsLoaded(Vec<Playlist>),
    HomeHubsLoaded(Vec<Hub>),
    ArtistLoaded(Artist),
    AlbumLoaded(Album),
    AlbumTracksLoaded(Vec<Track>),
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

    // Image loading events
    ImageLoaded { key: String },
    ImageFailed { key: String, error: String },

    // Artwork loading (non-blocking)
    ArtworkLoaded { thumb_path: String, data: Vec<u8> },
    ArtworkFailed { thumb_path: String },

    // Folder preloading (background)
    FoldersPreloaded { folder_state: crate::services::FolderNavigationState },

    // Background data preloading
    ArtistsPreloaded(Vec<Artist>),
    AlbumsPreloaded(Vec<Album>),
    PlaylistsPreloaded(Vec<Playlist>),
    GenresPreloaded(Vec<Genre>),
    ArtistGenresPreloaded(Vec<Genre>),
    AlbumGenresPreloaded(Vec<Genre>),
    MoodsPreloaded(Vec<Genre>),
    StylesPreloaded(Vec<Genre>),
    StationsPreloaded(Vec<Station>),
    RecentlyAddedPreloaded(Vec<Album>),
    RecentlyPlayedPreloaded(Vec<Album>),
    RecentPlaylistsPreloaded(Vec<Playlist>),

    // Cache management
    CacheSaved,

    // Waveform generation
    WaveformGenerated { track_key: String, data: WaveformData },
    WaveformFailed { track_key: String, error: String },
    WaveformCacheHit { track_key: String, data: WaveformData },

    // Station loading (background)
    StationTracksLoaded { station_key: String, station_title: String, tracks: Vec<Track> },
    StationLoadFailed { station_key: String, error: String },
    StationChildrenLoaded { station_key: String, station_title: String, children: Vec<Station> },

    // Album radio loading (background)
    AlbumRadioTracksLoaded { tracks: Vec<Track> },
    AlbumRadioLoadFailed { error: String },
}
