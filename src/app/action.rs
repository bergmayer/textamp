//! Application actions.
//!
//! Actions are commands that modify state or trigger side effects.

use crate::api::models::Track;
use super::state::{BrowseCategory, View};

/// Application actions/commands.
#[derive(Debug, Clone)]
pub enum Action {
    // Navigation (musikcube-style)
    SetView(View),
    SetCategory(BrowseCategory),
    ToggleFocus,

    // Data loading
    LoadInitialData,
    LoadLibraries,
    SelectLibrary(String),
    LoadArtists,
    LoadAlbums,
    LoadPlaylists,
    LoadArtistAlbums,      // Load albums for selected artist (right panel)
    LoadArtistAllTracks,   // Load all tracks by the selected artist
    LoadSelectedAlbumTracks, // Load tracks for selected album (drill down)
    LoadAlbumTracks { rating_key: String }, // Load tracks for a specific album by key
    LoadCategoryTracks,    // Load tracks directly (for Albums/Playlists categories)
    GoBackInRightPanel,    // Go from tracks back to albums view
    LoadSimilarAlbums { rating_key: String, title: String },
    LoadSimilarTracks { rating_key: String, title: String },

    // Playback control
    Play,
    Pause,
    TogglePlayPause,
    Stop,
    Next,
    Previous,
    Seek(u64),
    SeekRelative(i64),
    SetVolume(f32),
    VolumeUp,
    VolumeDown,
    ToggleMute,
    ToggleShuffle,
    CycleRepeat,

    // Queue management
    PlayTrack(Track),
    PlayTrackFromCategory(usize),
    PlayAlbum { rating_key: String },  // Load album tracks and play them
    QueueTrack(Track),
    EnqueueSelection,  // Alt+E: Add current selection to end of queue
    EnqueueAlbum { rating_key: String, title: String },  // Load album tracks and add to queue
    ClearQueue,
    RemoveFromQueue(usize),
    ShuffleQueue,
    JumpToQueueIndex(usize),      // Jump to and play a specific queue index
    PromptSavePlaylist,           // Alt+P: Prompt user for playlist name
    SaveQueueAsPlaylist(String),  // Save queue with given name

    // Genres, Artist Genres, Album Genres, Moods, and Styles
    LoadGenres,
    LoadArtistGenres,     // Load Plex genres at artist level
    LoadAlbumGenres,      // Load Plex genres at album level
    LoadMoods,
    LoadStyles,           // Load Plex styles
    LoadGenreAlbums,           // Load albums in selected genre (file tags)
    LoadArtistGenreAlbums,     // Load albums in selected artist genre
    LoadAlbumGenreAlbums,      // Load albums in selected album genre
    LoadMoodAlbums,            // Load albums in selected mood
    LoadStyleAlbums,           // Load albums in selected style
    LoadGenreTracks { rating_key: String }, // Load tracks for album in genre Miller columns
    PlayGenreTrack(usize),     // Play track from genre tracks list
    CycleGenreContentType,     // Ctrl+G when in genres: cycle Genres -> Artist -> Album -> Moods -> Styles
    CycleGenreSort,            // Cycle through sort modes
    LoadGenreArtists,          // Legacy - now prefer LoadGenreAlbums

    // Artist view mode cycling
    CycleArtistViewMode, // Ctrl+A when in Artists: cycle Artist → Album Artist → Album

    // Now Playing view mode cycling
    CycleNowPlayingMode, // Ctrl+N when already in Now Playing: cycle Queue → Recently Played
    LoadRecentlyPlayedAlbums,
    PlayRecentlyPlayedAlbum(usize),  // Play album at index in recently played list

    // Playlists view mode cycling
    CyclePlaylistsMode, // Ctrl+P when already in Playlists: cycle All → Recently Added → Recent
    LoadRecentlyAddedAlbums,
    LoadRecentPlaylists,

    // Search/Filter
    AppendSearchChar(char),
    DeleteSearchChar,
    ExecuteSearch,
    ClearSearch,
    SelectFilterResult,
    ExecuteFilterSearch,  // API search for filter view

    // UI
    ListUp,
    ListDown,
    ListPageUp,
    ListPageDown,
    ListTop,
    ListBottom,
    SelectItem,

    // Settings
    OpenSettings,
    SettingsSelect,
    SettingsDiscoverServers,
    SelectServer(String),
    SaveSettings,
    SaveCredentials, // Save username/password from settings
    ClearCache,      // Clear all cached data and reload

    // Folder navigation
    LoadFolderRoot,
    NavigateIntoFolder(String),
    NavigateUpFolder,
    PlayFolderTracks,

    // Radio mode
    StartTrackRadio { track_key: String, title: String },
    StartAlbumRadio { album_key: String, title: String },
    StartArtistRadio { artist_key: String, title: String },
    StopRadio,
    FetchMoreRadioTracks,
    JumpToRadioTrack(usize),  // Jump to track in radio queue without clearing

    // Stations (Plexamp-style radio stations)
    LoadStations,
    PlayStation(String), // station key
    DrillIntoStation(String, String), // station key, station title
    NavigateStationsBack,

    // Authentication
    StartAuth,
    SettingsSignIn, // Sign in with username/password from settings
    AuthSignIn,     // Submit login form (from Auth screen)
    AuthSelectServer, // Select server and connect (from Auth screen)
    Logout,

    // Artwork
    LoadArtwork,

    // Waveform (for Seekbar visualizer)
    LoadWaveform,

    // System
    Quit,
    ShowError(String),
    ClearError,
    SetStatus(String),
    ClearStatus,
    Refresh,
    RefreshCategory(crate::app::state::RefreshCategory),

    // Theme
    CycleTheme,

    // Sonic Adventure
    StartAdventure,                      // Begin adventure mode
    SetAdventureStart(Track),            // Set start track
    SetAdventureEnd(Track),              // Set end track
    SetAdventureLength(usize),           // Set length and start generation
    CancelAdventure,                     // Cancel adventure mode
    AdventureComplete(Vec<Track>),       // Adventure ready
    AdventureError(String),              // Generation failed
}
