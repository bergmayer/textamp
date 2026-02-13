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
    NextView,   // Tab: cycle Library→Playlists→Genres→Folders→NowPlaying
    PrevView,   // Shift+Tab: cycle backwards
    NextMode,   // Shift+Down: cycle modes within current category
    PrevMode,   // Shift+Up: cycle modes backwards
    ToggleFocus,

    // Data loading
    LoadInitialData,
    LoadLibraries,
    SelectLibrary(String),
    /// Switch to a library on a different server: (library_key, server_identifier)
    SelectLibraryOnServer(String, String),
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
    StartPendingPlayback,
    RetryCurrentTrack,    // Replay current track without resetting error counter

    // Queue management
    PlayTrack(Track),
    PlayTrackFromCategory(usize),
    PlayAlbum { rating_key: String },  // Load album tracks and play them
    EnqueueSelection,  // Alt+Q: Add current selection to end of queue
    EnqueueAlbum { rating_key: String, title: String },  // Load album tracks and add to queue
    ClearQueue,
    RemoveFromQueue(usize),
    ToggleQueueShuffle,
    ToggleBrowseShuffle,
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
    CycleGenreContentType,     // Ctrl+G when in genres: cycle Genres -> Artist -> Album -> Moods -> Styles
    RefreshGenreView,          // Refresh genre view after mode change (shared logic)

    // Artist view mode cycling
    CycleArtistViewMode, // Ctrl+L when in Library: cycle Artist ↔ AlbumArtist
    RefreshArtistView,   // Refresh artist view after mode change (shared logic)
    CycleLibrarySubMode, // Alt+S in Library: cycle Normal → AllByArtist → AllShuffled

    // Now Playing view mode cycling
    CycleNowPlayingMode, // Ctrl+N when already in Now Playing: cycle Queue → Recently Played
    RefreshNowPlayingView, // Refresh now playing view after mode change (shared logic)

    // Genre tab cycling
    CycleGenreTab,       // Ctrl+G when already in Genres: cycle All/Library/Artist/Album/Mood/Style
    SetGenreTab(crate::app::state::GenreTab), // Direct tab selection (mouse clicks)

    // Search
    AppendSearchChar(char),
    DeleteSearchChar,
    ExecuteLocalSearch,
    ClearSearch,
    SelectSearchResult,

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
    ClearCache,      // Clear all cached data and reload (legacy, kept for Account section)
    ClearLibraryCache,      // Clear main library cache only (artists, albums, etc.)
    ClearArtworkCache,      // Clear artwork disk cache
    ClearSubfolderCache,    // Clear subfolder cache entries
    StartSubfolderCrawl,    // Manual subfolder crawl for current library
    StopSubfolderCrawl,     // Cancel active subfolder crawl
    ToggleKeepFolderCache,  // Toggle per-library keep_folder_cache setting

    // Folder navigation
    LoadFolderRoot,
    NavigateIntoFolder(String),
    NavigateUpFolder,
    PlayFolderTracks,
    /// Refresh a specific subfolder (F5 when focused on a subfolder column).
    /// This is the only way subfolder caches get manually refreshed.
    RefreshSubfolder(String),

    // Miller column navigation for Artists view
    LoadArtistAlbumsForMiller { artist_key: String },
    LoadAlbumTracksForMiller { album_key: String },
    LoadArtistAllTracksForMiller { artist_key: String },  // Load all tracks by artist (from "All Tracks" entry)
    LoadAllAlbumsForMiller,  // Load all albums as a Miller column (from "All Artists" entry)
    PlayTrackFromMiller { column_index: usize, track_index: usize },

    // Miller column navigation for Genres view
    LoadGenreAlbumsForMiller { genre_key: String },
    LoadGenreTracksForMiller { album_key: String },
    PlayGenreTrackFromMiller { column_index: usize, track_index: usize },

    // Miller column navigation for Playlists view
    LoadPlaylistTracksForMiller { playlist_key: String },
    PlayPlaylistTrackFromMiller { column_index: usize, track_index: usize },

    // Radio mode
    StopRadio,
    JumpToRadioTrack(usize),  // Jump to track in radio queue without clearing
    PlayCurrentRadioTrack,    // Play current track in radio mode (stays in Radio playback mode)

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
    ToggleAlbumArtView,
    /// Load album art for a batch of albums: Vec<(rating_key, thumb_path)>
    LoadAlbumArt(Vec<(String, String)>),

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
    /// Check cache staleness on view navigation (tier-1: 72h for this category, tier-2: 32d for all others).
    CheckStaleness(crate::app::state::RefreshCategory),

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

    // Inline list filter
    ActivateListFilter,
    DeactivateListFilter,
    SelectFilteredItem,       // Select the currently highlighted filtered item (drill down/play)
    FilteredListUp,           // Navigate up within filtered results
    FilteredListDown,         // Navigate down within filtered results
    AppendListFilterChar(char),
    DeleteListFilterChar,
    ClearListFilter,
    ExecuteListFilter,

    // Search popup (Ctrl+F)
    OpenSearchPopup,
    CloseSearchPopup,

    // Radio launcher popup
    OpenRadioLauncher,
    CloseRadioLauncher,
    RadioLauncherSearch,
    RadioLauncherSelectResult,
    /// Start radio using Plex playQueue API (full heuristics: similarity, popularity, taste, freshness)
    StartPlexRadio { key: String, title: String },

    // Adventure launcher popup (Sonic Adventure from Radio section)
    OpenAdventureLauncher,
    CloseAdventureLauncher,
    AdventureLauncherSearch,
    AdventureLauncherDrillArtist { key: String, name: String },
    AdventureLauncherDrillAlbum { key: String, title: String, artist_name: String },
    AdventureLauncherSelectTrack,
    AdventureLauncherBack,

    // Library picker popup (Ctrl+Alt+S)
    OpenLibraryPicker,
    CloseLibraryPicker,

    // Remote player control
    DiscoverPlayers,
    SetOutputTarget(crate::app::state::OutputTarget),
}
