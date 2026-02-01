//! Application state definitions.
//!
//! Uses the Elm Architecture pattern with a single state struct.
//! UI modeled after musikcube: Browse (left: categories, right: tracks), Queue, Search, etc.

use crate::api::models::{Album, Artist, Genre, Library, Playlist, PlexServer, Station, Track, SearchResults};
use crate::services::{FolderNavigationState, WaveformData};
use crate::ui::theme::ThemeName;
use std::collections::HashMap;

/// Authentication flow step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthStep {
    /// Initial - checking for stored credentials
    #[default]
    Checking,
    /// Username/password entry form
    Login,
    /// Signing in to Plex.tv
    Authenticating,
    /// Choose from available servers
    ServerSelect,
    /// Connecting to selected server
    Connecting,
}

/// State for the authentication screen flow.
#[derive(Debug, Clone, Default)]
pub struct AuthState {
    /// Current step in the auth flow
    pub step: AuthStep,
    /// Username input field
    pub username_input: String,
    /// Password input field
    pub password_input: String,
    /// Which field is focused: 0=username, 1=password, 2=sign in button
    pub field_index: usize,
    /// Whether currently editing a text field
    pub editing: bool,
    /// Selected server index (for ServerSelect step)
    pub server_index: usize,
    /// Error message to display
    pub error_message: Option<String>,
}

/// Root application state.
#[derive(Debug)]
pub struct AppState {
    // Connection
    pub connection: ConnectionState,
    pub libraries: Vec<Library>,
    pub active_library: Option<String>,
    pub available_servers: Vec<PlexServer>,

    // Authentication flow state
    pub auth_state: AuthState,

    // Navigation (musikcube-style)
    pub view: View,
    pub previous_view: Option<View>,  // For returning from Similar, Help, etc.
    pub help_scroll: u16,  // Scroll position for help screen
    pub browse_category: BrowseCategory,
    pub focus: Focus,

    // Library data cache with pagination info
    pub artists: Vec<Artist>,
    pub artists_total: u32,
    pub artists_loading: bool,
    pub albums: Vec<Album>,
    pub albums_total: u32,
    pub albums_loading: bool,
    pub playlists: Vec<Playlist>,
    pub playlists_loading: bool,

    // Genres, Artist Genres, Album Genres, Moods, and Styles
    pub genres: Vec<Genre>,              // Actual genre tags from files
    pub artist_genres: Vec<Genre>,       // Plex genres at artist level
    pub album_genres: Vec<Genre>,        // Plex genres at album level
    pub moods: Vec<Genre>,               // Moods use same structure as genres
    pub styles: Vec<Genre>,              // Styles use same structure as genres
    pub genres_loading: bool,
    pub artist_genres_loading: bool,
    pub album_genres_loading: bool,
    pub moods_loading: bool,
    pub styles_loading: bool,
    pub genres_index: usize,
    pub genre_content_type: GenreContentType,  // Genres / Artist / Album / Moods / Styles cycle
    pub genre_albums: Vec<Album>,  // Albums in selected genre/mood
    pub genre_albums_index: usize,
    pub genre_tracks: Vec<Track>,  // Tracks for selected album in genre view
    pub genre_tracks_index: usize,
    pub genre_focus_column: usize,  // 0=genres, 1=albums, 2=tracks (Miller columns)
    pub genre_sort_mode: GenreSortMode,

    // Artist view mode (Artist vs Album Artist)
    pub artist_view_mode: ArtistViewMode,

    // Right panel content (depends on browse category and depth)
    // For Artists: first shows albums, then tracks when album selected
    // For Albums/Playlists: shows tracks directly
    pub right_panel_mode: RightPanelMode,
    pub selected_artist_albums: Vec<Album>,
    pub selected_album_tracks: Vec<Track>,
    pub selected_artist_name: String,
    pub selected_album_title: String,
    pub right_panel_loading: bool,

    // Similar content (Plex sonic similarity)
    pub similar_albums: Vec<Album>,
    pub similar_tracks: Vec<Track>,
    pub similar_mode: SimilarMode,
    pub similar_loading: bool,
    pub similar_source_title: String,

    // Playback
    pub playback: PlaybackState,
    pub queue: Vec<Track>,
    pub queue_index: Option<usize>,
    /// Original queue order (for restoring after sort/shuffle)
    pub queue_original: Vec<Track>,
    /// Current queue sort mode
    pub queue_sort_mode: QueueSortMode,
    /// Play history - recently played tracks for scrollback (max ~20)
    pub play_history: Vec<Track>,
    /// Whether user is currently dragging the seek indicator
    pub seeking_drag: bool,
    /// Plex session identifier for timeline reporting.
    /// Generated when starting a new playback context (queue, radio, etc.).
    /// Used to correlate all timeline reports to a single session.
    pub plex_session_id: Option<String>,

    // Search
    pub search_query: String,
    pub search_results: Option<SearchResults>,
    pub search_loading: bool,

    // Filter (uses API search for large libraries)
    pub filter_results: Option<SearchResults>,
    pub filter_loading: bool,
    pub pending_filter_key: Option<String>,  // Rating key from filter selection
    pub pending_album_key: Option<String>,   // Album to auto-select after loading artist albums
    pub filter_search_version: u64,  // Increments on each search, used for debounce
    pub global_search_version: u64,  // Version counter for global search debouncing

    // UI state
    pub list_state: ListStates,
    pub should_quit: bool,
    pub last_error: Option<String>,
    pub status_message: Option<String>,

    // Input dialog state (for playlist naming, etc.)
    pub input_dialog: Option<InputDialog>,

    // Alt key state for command mode
    pub alt_held: bool,

    // Unified search/filter tab
    pub search_tab: SearchTab,

    // Terminal size
    pub terminal_width: u16,
    pub terminal_height: u16,

    // Image cache (thumb_path -> loaded flag)
    pub image_loaded: HashMap<String, bool>,

    // Settings state
    pub settings_state: SettingsState,

    // Folder browsing state (for Folders category with Miller columns)
    pub folder_state: Option<FolderNavigationState>,

    // Artwork state
    pub artwork_thumb: Option<String>,
    pub artwork_data: Option<Vec<u8>>,
    pub artwork_loading: bool,

    // Radio mode state (legacy)
    pub radio_state: RadioState,

    // NEW: Playback mode (Queue vs Radio)
    pub playback_mode: PlaybackMode,

    // NEW: Radio playback state (continuous)
    pub radio: RadioPlaybackState,

    // NEW: Station navigation (hierarchical)
    pub station_nav: StationNavigationState,

    // Stations state (Plexamp-style radio stations) - legacy, use station_nav instead
    pub stations: Vec<Station>,
    pub stations_loading: bool,
    pub stations_index: usize,

    // Theme
    pub theme: ThemeName,

    // Sonic Adventure state
    pub adventure: AdventureState,

    // Now Playing view mode (Queue vs Recently Played vs Visualizer)
    pub now_playing_mode: NowPlayingMode,
    pub recently_played_albums: Vec<Album>,
    pub recently_played_loading: bool,

    // Playlists view mode (All vs Recently Added vs Recent)
    pub playlists_mode: PlaylistsMode,
    pub recently_added_albums: Vec<Album>,
    pub recently_added_loading: bool,
    pub recent_playlists: Vec<Playlist>,
    pub recent_playlists_loading: bool,

    // Cache management
    pub cache_dirty: bool,
    pub last_input_time: std::time::Instant,
    pub last_cache_save: std::time::Instant,
    pub cache_save_in_progress: bool,

    // Waveform seekbar state
    pub waveform: WaveformState,
}

/// Playback mode - determines behavior (finite queue vs continuous radio).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackMode {
    /// No active playback source
    #[default]
    None,
    /// User-controlled queue (finite, stops when done)
    Queue,
    /// Radio/station playback (continuous, auto-fetches more)
    Radio,
}

/// Radio seed mode for similarity-based radio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadioSeedMode {
    Track,
    Album,
    Artist,
}

impl RadioSeedMode {
    pub fn label(&self) -> &'static str {
        match self {
            RadioSeedMode::Track => "track radio",
            RadioSeedMode::Album => "album radio",
            RadioSeedMode::Artist => "artist radio",
        }
    }
}

/// Active station info.
#[derive(Debug, Clone)]
pub struct ActiveStation {
    pub key: String,
    pub title: String,
}

/// Radio seed for similarity-based radio.
#[derive(Debug, Clone)]
pub struct RadioSeed {
    pub mode: RadioSeedMode,
    pub key: String,
    pub title: String,
}

/// Radio playback state (continuous, auto-queueing).
#[derive(Debug, Clone, Default)]
pub struct RadioPlaybackState {
    /// Active station (None if using seed-based radio)
    pub active_station: Option<ActiveStation>,
    /// Tracks currently loaded for playback
    pub tracks: Vec<Track>,
    /// Current track index within loaded tracks
    pub track_index: Option<usize>,
    /// Whether we're currently fetching more tracks
    pub fetching: bool,
    /// History of played track keys (to avoid repeats)
    pub history: Vec<String>,
    /// For similarity-based radio: the seed info
    pub seed: Option<RadioSeed>,

    // Time Travel Radio state - for chronological continuation
    /// Sorted list of decade values (e.g., ["1950", "1960", ...])
    pub time_travel_decades: Vec<String>,
    /// Current position in decades list (next decade to fetch from)
    pub time_travel_index: usize,
}

impl RadioPlaybackState {
    /// Get the current track.
    pub fn current_track(&self) -> Option<&Track> {
        self.track_index.and_then(|idx| self.tracks.get(idx))
    }

    /// Get the display title for the radio.
    pub fn title(&self) -> String {
        if let Some(station) = &self.active_station {
            station.title.clone()
        } else if let Some(seed) = &self.seed {
            format!("{}: {}", seed.mode.label(), seed.title)
        } else {
            "Radio".to_string()
        }
    }

    /// Clear all state.
    pub fn clear(&mut self) {
        self.active_station = None;
        self.tracks.clear();
        self.track_index = None;
        self.fetching = false;
        self.history.clear();
        self.seed = None;
        // Clear Time Travel state
        self.time_travel_decades.clear();
        self.time_travel_index = 0;
    }
}

/// A single column in the station Miller columns view.
#[derive(Debug, Clone, Default)]
pub struct StationColumn {
    /// Key for this station category (None for root)
    pub key: Option<String>,
    /// Display title
    pub title: String,
    /// Stations in this column
    pub stations: Vec<Station>,
    /// Currently selected index
    pub selected_index: usize,
}

impl StationColumn {
    /// Create a new column.
    pub fn new(key: Option<String>, title: String, stations: Vec<Station>) -> Self {
        Self {
            key,
            title,
            stations,
            selected_index: 0,
        }
    }

    /// Get the selected station, if any.
    pub fn selected_station(&self) -> Option<&Station> {
        self.stations.get(self.selected_index)
    }
}

/// Station navigation state for hierarchical stations (Miller columns style).
#[derive(Debug, Clone, Default)]
pub struct StationNavigationState {
    /// Columns from left to right (root is first)
    pub columns: Vec<StationColumn>,
    /// Which column currently has focus (0-indexed)
    pub focused_column: usize,
    /// Loading indicator
    pub loading: bool,
}

impl StationNavigationState {
    /// Create a new empty station navigation state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the focused column.
    pub fn focused(&self) -> Option<&StationColumn> {
        self.columns.get(self.focused_column)
    }

    /// Get the focused column mutably.
    pub fn focused_mut(&mut self) -> Option<&mut StationColumn> {
        self.columns.get_mut(self.focused_column)
    }

    /// Get the selected station in the focused column.
    pub fn selected_station(&self) -> Option<&Station> {
        self.focused().and_then(|c| c.selected_station())
    }

    /// Check if we're at the root column.
    pub fn is_at_root(&self) -> bool {
        self.focused_column == 0
    }

    /// Check if we can go left (focus previous column).
    pub fn can_go_left(&self) -> bool {
        self.focused_column > 0
    }

    /// Move focus left.
    pub fn focus_left(&mut self) {
        if self.focused_column > 0 {
            self.focused_column -= 1;
        }
    }

    /// Move focus right (if there's a column to the right).
    pub fn focus_right(&mut self) -> bool {
        if self.focused_column + 1 < self.columns.len() {
            self.focused_column += 1;
            true
        } else {
            false
        }
    }

    /// Add a new column to the right, removing any columns after current focus.
    pub fn push_column(&mut self, column: StationColumn) {
        // Remove columns to the right of focus
        self.columns.truncate(self.focused_column + 1);
        // Add new column
        self.columns.push(column);
        // Move focus to new column
        self.focused_column = self.columns.len() - 1;
    }

    /// Get the current title (focused column's title).
    pub fn current_title(&self) -> &str {
        self.focused().map(|c| c.title.as_str()).unwrap_or("Stations")
    }

    /// Navigate up in current column.
    pub fn move_up(&mut self) {
        if let Some(col) = self.focused_mut() {
            if col.selected_index > 0 {
                col.selected_index -= 1;
            }
        }
    }

    /// Navigate down in current column.
    pub fn move_down(&mut self) {
        if let Some(col) = self.focused_mut() {
            let max = col.stations.len().saturating_sub(1);
            if col.selected_index < max {
                col.selected_index += 1;
            }
        }
    }

    /// Get the number of columns.
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }
}

// Legacy compatibility - keep RadioMode and RadioState for now
/// Radio playback mode (legacy - use PlaybackMode instead).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RadioMode {
    #[default]
    Off,
    /// Track radio - plays similar tracks based on a seed track
    Track,
    /// Album radio - plays album then similar albums
    Album,
    /// Artist radio - plays artist's tracks then similar
    Artist,
}

impl RadioMode {
    pub fn label(&self) -> &'static str {
        match self {
            RadioMode::Off => "",
            RadioMode::Track => "track radio",
            RadioMode::Album => "album radio",
            RadioMode::Artist => "artist radio",
        }
    }
}

/// Radio state for auto-queueing similar content (legacy).
#[derive(Debug, Clone, Default)]
pub struct RadioState {
    /// Current radio mode
    pub mode: RadioMode,
    /// Seed track for track radio
    pub seed_track_key: Option<String>,
    /// Seed track title (for display)
    pub seed_title: String,
    /// Whether we're currently fetching more tracks
    pub fetching: bool,
    /// History of played track keys (to avoid repeats)
    pub history: Vec<String>,
}

impl AppState {
    /// Create a new application state with defaults.
    pub fn new() -> Self {
        Self {
            connection: ConnectionState::Disconnected,
            libraries: Vec::new(),
            active_library: None,
            available_servers: Vec::new(),
            auth_state: AuthState::default(),
            view: View::Auth,
            previous_view: None,
            help_scroll: 0,
            browse_category: BrowseCategory::Artists,
            focus: Focus::Left,
            artists: Vec::new(),
            artists_total: 0,
            artists_loading: false,
            albums: Vec::new(),
            albums_total: 0,
            albums_loading: false,
            playlists: Vec::new(),
            playlists_loading: false,
            genres: Vec::new(),
            artist_genres: Vec::new(),
            album_genres: Vec::new(),
            moods: Vec::new(),
            styles: Vec::new(),
            genres_loading: false,
            artist_genres_loading: false,
            album_genres_loading: false,
            moods_loading: false,
            styles_loading: false,
            genres_index: 0,
            genre_content_type: GenreContentType::default(),
            genre_albums: Vec::new(),
            genre_albums_index: 0,
            genre_tracks: Vec::new(),
            genre_tracks_index: 0,
            genre_focus_column: 0,
            genre_sort_mode: GenreSortMode::default(),
            artist_view_mode: ArtistViewMode::default(),
            right_panel_mode: RightPanelMode::Empty,
            selected_artist_albums: Vec::new(),
            selected_album_tracks: Vec::new(),
            selected_artist_name: String::new(),
            selected_album_title: String::new(),
            right_panel_loading: false,
            similar_albums: Vec::new(),
            similar_tracks: Vec::new(),
            similar_mode: SimilarMode::Albums,
            similar_loading: false,
            similar_source_title: String::new(),
            playback: PlaybackState::default(),
            queue: Vec::new(),
            queue_index: None,
            queue_original: Vec::new(),
            queue_sort_mode: QueueSortMode::default(),
            play_history: Vec::new(),
            seeking_drag: false,
            plex_session_id: None,
            search_query: String::new(),
            search_results: None,
            search_loading: false,
            filter_results: None,
            filter_loading: false,
            pending_filter_key: None,
            pending_album_key: None,
            filter_search_version: 0,
            global_search_version: 0,
            list_state: ListStates::default(),
            should_quit: false,
            last_error: None,
            status_message: None,
            input_dialog: None,
            alt_held: false,
            search_tab: SearchTab::default(),
            terminal_width: 80,
            terminal_height: 24,
            image_loaded: HashMap::new(),
            settings_state: SettingsState::default(),
            folder_state: None,
            artwork_thumb: None,
            artwork_data: None,
            artwork_loading: false,
            radio_state: RadioState::default(),
            playback_mode: PlaybackMode::None,
            radio: RadioPlaybackState::default(),
            station_nav: StationNavigationState::default(),
            stations: Vec::new(),
            stations_loading: false,
            stations_index: 0,
            theme: ThemeName::default(),
            adventure: AdventureState::default(),
            now_playing_mode: NowPlayingMode::default(),
            recently_played_albums: Vec::new(),
            recently_played_loading: false,
            playlists_mode: PlaylistsMode::default(),
            recently_added_albums: Vec::new(),
            recently_added_loading: false,
            recent_playlists: Vec::new(),
            recent_playlists_loading: false,
            cache_dirty: false,
            last_input_time: std::time::Instant::now(),
            last_cache_save: std::time::Instant::now(),
            cache_save_in_progress: false,
            waveform: WaveformState::default(),
        }
    }

    /// Set an error message to display.
    pub fn set_error(&mut self, msg: String) {
        self.last_error = Some(msg);
    }

    /// Clear the current error.
    pub fn clear_error(&mut self) {
        self.last_error = None;
    }

    /// Set a status message.
    pub fn set_status(&mut self, msg: String) {
        self.status_message = Some(msg);
    }

    /// Get the currently playing track (mode-aware).
    pub fn current_track(&self) -> Option<&Track> {
        match self.playback_mode {
            PlaybackMode::Queue | PlaybackMode::None => {
                self.queue_index.and_then(|idx| self.queue.get(idx))
            }
            PlaybackMode::Radio => {
                self.radio.current_track()
            }
        }
    }

    /// Check if music is currently playing.
    pub fn is_playing(&self) -> bool {
        self.playback.status == PlayStatus::Playing
    }

    /// Get the current category list length.
    pub fn category_len(&self) -> usize {
        match self.browse_category {
            BrowseCategory::Artists => {
                match self.artist_view_mode {
                    ArtistViewMode::Artist | ArtistViewMode::AlbumArtist => self.artists.len(),
                    ArtistViewMode::Album => self.albums.len(),
                }
            }
            BrowseCategory::Playlists => {
                match self.playlists_mode {
                    PlaylistsMode::All => self.playlists.len(),
                    PlaylistsMode::RecentlyAdded => self.recently_added_albums.len(),
                    PlaylistsMode::RecentPlaylists => self.recent_playlists.len(),
                }
            }
            BrowseCategory::Stations => self.stations.len(),
            BrowseCategory::Genres => self.current_genre_list().len(),
            BrowseCategory::Folders => 0, // Handled separately via folder_state
        }
    }

    /// Get the current genre list based on content type.
    pub fn current_genre_list(&self) -> &Vec<Genre> {
        match self.genre_content_type {
            GenreContentType::Genres => &self.genres,
            GenreContentType::ArtistGenres => &self.artist_genres,
            GenreContentType::AlbumGenres => &self.album_genres,
            GenreContentType::Moods => &self.moods,
            GenreContentType::Styles => &self.styles,
        }
    }

    /// Get the current category index.
    pub fn category_index(&self) -> usize {
        match self.browse_category {
            BrowseCategory::Artists => {
                match self.artist_view_mode {
                    ArtistViewMode::Artist | ArtistViewMode::AlbumArtist => self.list_state.artists_index,
                    ArtistViewMode::Album => self.list_state.albums_index,
                }
            }
            BrowseCategory::Playlists => self.list_state.playlists_index,
            BrowseCategory::Stations => self.stations_index,
            BrowseCategory::Genres => self.genres_index,
            BrowseCategory::Folders => 0, // Handled separately via folder_state
        }
    }

    /// Set the current category index.
    pub fn set_category_index(&mut self, idx: usize) {
        match self.browse_category {
            BrowseCategory::Artists => {
                match self.artist_view_mode {
                    ArtistViewMode::Artist | ArtistViewMode::AlbumArtist => self.list_state.artists_index = idx,
                    ArtistViewMode::Album => self.list_state.albums_index = idx,
                }
            }
            BrowseCategory::Playlists => self.list_state.playlists_index = idx,
            BrowseCategory::Stations => self.stations_index = idx,
            BrowseCategory::Genres => self.genres_index = idx,
            BrowseCategory::Folders => {}, // Handled separately via folder_state
        }
    }

    /// Get the selected category item's rating key.
    pub fn selected_category_key(&self) -> Option<String> {
        match self.browse_category {
            BrowseCategory::Artists => {
                match self.artist_view_mode {
                    ArtistViewMode::Artist | ArtistViewMode::AlbumArtist => {
                        self.artists.get(self.list_state.artists_index)
                            .map(|a| a.rating_key.clone())
                    }
                    ArtistViewMode::Album => {
                        self.albums.get(self.list_state.albums_index)
                            .map(|a| a.rating_key.clone())
                    }
                }
            }
            BrowseCategory::Playlists => {
                // Different sources based on playlists mode
                match self.playlists_mode {
                    PlaylistsMode::All => {
                        self.playlists.get(self.list_state.playlists_index)
                            .map(|p| p.rating_key.clone())
                    }
                    PlaylistsMode::RecentlyAdded => {
                        self.recently_added_albums.get(self.list_state.playlists_index)
                            .map(|a| a.rating_key.clone())
                    }
                    PlaylistsMode::RecentPlaylists => {
                        self.recent_playlists.get(self.list_state.playlists_index)
                            .map(|p| p.rating_key.clone())
                    }
                }
            }
            BrowseCategory::Stations => self.stations.get(self.stations_index)
                .map(|s| s.key.clone()),
            BrowseCategory::Genres => self.current_genre_list().get(self.genres_index)
                .map(|g| g.effective_key().to_string()),
            BrowseCategory::Folders => None, // Handled separately via folder_state
        }
    }

    /// Get the selected category item's title for display.
    pub fn selected_category_title(&self) -> Option<String> {
        match self.browse_category {
            BrowseCategory::Artists => {
                match self.artist_view_mode {
                    ArtistViewMode::Artist | ArtistViewMode::AlbumArtist => {
                        self.artists.get(self.list_state.artists_index)
                            .map(|a| a.title.clone())
                    }
                    ArtistViewMode::Album => {
                        self.albums.get(self.list_state.albums_index)
                            .map(|a| a.title.clone())
                    }
                }
            }
            BrowseCategory::Playlists => {
                // Different sources based on playlists mode
                match self.playlists_mode {
                    PlaylistsMode::All => {
                        self.playlists.get(self.list_state.playlists_index)
                            .map(|p| p.title.clone())
                    }
                    PlaylistsMode::RecentlyAdded => {
                        self.recently_added_albums.get(self.list_state.playlists_index)
                            .map(|a| a.title.clone())
                    }
                    PlaylistsMode::RecentPlaylists => {
                        self.recent_playlists.get(self.list_state.playlists_index)
                            .map(|p| p.title.clone())
                    }
                }
            }
            BrowseCategory::Stations => self.stations.get(self.stations_index)
                .map(|s| s.title.clone()),
            BrowseCategory::Genres => self.current_genre_list().get(self.genres_index)
                .map(|g| g.title.clone()),
            BrowseCategory::Folders => None, // Handled separately via folder_state
        }
    }

    /// Add a track to play history (keeps last 20).
    pub fn add_to_history(&mut self, track: Track) {
        // Don't add duplicates consecutively
        if self.play_history.last().map(|t| &t.rating_key) == Some(&track.rating_key) {
            return;
        }
        self.play_history.push(track);
        // Keep only last 20
        if self.play_history.len() > 20 {
            self.play_history.remove(0);
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Connection state to Plex server.
#[derive(Debug, Clone)]
pub enum ConnectionState {
    Disconnected,
    Authenticating,
    AuthPending { pin_code: String, pin_id: u64 },
    Connecting,
    Connected { username: String },
    Error(String),
}

/// Current view (musikcube-style).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Authentication screen
    Auth,
    /// Browse library (main view with left: categories, right: tracks)
    Browse,
    /// Unified Now Playing screen - shows queue/playlist/station tracks with history
    NowPlaying,
    /// Unified Search/Filter screen with tabs
    Search,
    /// Similar albums view
    Similar,
    /// Help / keybindings
    Help,
    /// Settings screen
    Settings,
}

/// Now Playing view mode - cycles with Ctrl+N.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NowPlayingMode {
    /// Show current queue/radio tracks
    #[default]
    Queue,
    /// Show recently played albums (like Plexamp)
    RecentlyPlayed,
    /// Now Playing view with artwork and waveform seekbar
    NowPlaying,
}

impl NowPlayingMode {
    pub fn next(&self) -> Self {
        match self {
            NowPlayingMode::Queue => NowPlayingMode::RecentlyPlayed,
            NowPlayingMode::RecentlyPlayed => NowPlayingMode::NowPlaying,
            NowPlayingMode::NowPlaying => NowPlayingMode::Queue,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            NowPlayingMode::Queue => "queue",
            NowPlayingMode::RecentlyPlayed => "recently played",
            NowPlayingMode::NowPlaying => "now playing",
        }
    }
}

/// Playlists view mode - cycles with Ctrl+P.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaylistsMode {
    /// Show all playlists
    #[default]
    All,
    /// Show recently added albums
    RecentlyAdded,
    /// Show recently accessed playlists
    RecentPlaylists,
}

impl PlaylistsMode {
    pub fn next(&self) -> Self {
        match self {
            PlaylistsMode::All => PlaylistsMode::RecentlyAdded,
            PlaylistsMode::RecentlyAdded => PlaylistsMode::RecentPlaylists,
            PlaylistsMode::RecentPlaylists => PlaylistsMode::All,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            PlaylistsMode::All => "playlists",
            PlaylistsMode::RecentlyAdded => "recently added",
            PlaylistsMode::RecentPlaylists => "recent playlists",
        }
    }
}

/// Search tab in unified search view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchTab {
    /// Global search across all content
    #[default]
    Global,
    /// Filter artists
    Artists,
    /// Filter by album artist tag
    AlbumArtists,
    /// Filter albums by title
    Albums,
    /// Filter playlists
    Playlists,
    /// Filter tracks
    Tracks,
    /// Filter genres
    Genres,
}

impl SearchTab {
    pub fn all() -> &'static [SearchTab] {
        &[
            SearchTab::Global,
            SearchTab::Artists,
            SearchTab::AlbumArtists,
            SearchTab::Albums,
            SearchTab::Playlists,
            SearchTab::Tracks,
            SearchTab::Genres,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            SearchTab::Global => "All",
            SearchTab::Artists => "Artists",
            SearchTab::AlbumArtists => "Album Artists",
            SearchTab::Albums => "Albums",
            SearchTab::Playlists => "Playlists",
            SearchTab::Tracks => "Tracks",
            SearchTab::Genres => "Genres",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            SearchTab::Global => SearchTab::Artists,
            SearchTab::Artists => SearchTab::AlbumArtists,
            SearchTab::AlbumArtists => SearchTab::Albums,
            SearchTab::Albums => SearchTab::Playlists,
            SearchTab::Playlists => SearchTab::Tracks,
            SearchTab::Tracks => SearchTab::Genres,
            SearchTab::Genres => SearchTab::Global,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            SearchTab::Global => SearchTab::Genres,
            SearchTab::Artists => SearchTab::Global,
            SearchTab::AlbumArtists => SearchTab::Artists,
            SearchTab::Albums => SearchTab::AlbumArtists,
            SearchTab::Playlists => SearchTab::Albums,
            SearchTab::Tracks => SearchTab::Playlists,
            SearchTab::Genres => SearchTab::Tracks,
        }
    }
}

/// Browse category type (what's shown in left panel).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseCategory {
    Artists,
    Playlists,
    Stations,
    Genres,
    Folders,
}

impl BrowseCategory {
    pub fn all() -> &'static [BrowseCategory] {
        &[
            BrowseCategory::Artists,
            BrowseCategory::Playlists,
            BrowseCategory::Stations,
            BrowseCategory::Genres,
            BrowseCategory::Folders,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            BrowseCategory::Artists => "artists",
            BrowseCategory::Playlists => "playlists",
            BrowseCategory::Stations => "stations",
            BrowseCategory::Genres => "genres",
            BrowseCategory::Folders => "folders",
        }
    }

    pub fn shortcut(&self) -> char {
        match self {
            BrowseCategory::Artists => 'a',
            BrowseCategory::Playlists => 'p',
            BrowseCategory::Stations => 't',
            BrowseCategory::Genres => 'e',
            BrowseCategory::Folders => 'o',
        }
    }
}

/// Artist/Album view mode - cycles through Artist, Album Artist, and Album (by title).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArtistViewMode {
    #[default]
    Artist,
    AlbumArtist,
    Album,
}

impl ArtistViewMode {
    /// Cycle to the next mode (Artist → Album Artist → Album → Artist).
    pub fn next(&self) -> Self {
        match self {
            ArtistViewMode::Artist => ArtistViewMode::AlbumArtist,
            ArtistViewMode::AlbumArtist => ArtistViewMode::Album,
            ArtistViewMode::Album => ArtistViewMode::Artist,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            ArtistViewMode::Artist => "artists",
            ArtistViewMode::AlbumArtist => "album artists",
            ArtistViewMode::Album => "albums",
        }
    }
}

/// Genre content type - genres, normalized genres, or moods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GenreContentType {
    #[default]
    Genres,
    ArtistGenres,
    AlbumGenres,
    Moods,
    Styles,
}

impl GenreContentType {
    /// Cycle to the next content type (Genres -> Artist -> Album -> Moods -> Styles -> Genres).
    pub fn next(&self) -> Self {
        match self {
            GenreContentType::Genres => GenreContentType::ArtistGenres,
            GenreContentType::ArtistGenres => GenreContentType::AlbumGenres,
            GenreContentType::AlbumGenres => GenreContentType::Moods,
            GenreContentType::Moods => GenreContentType::Styles,
            GenreContentType::Styles => GenreContentType::Genres,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            GenreContentType::Genres => "genres",
            GenreContentType::ArtistGenres => "artist genres",
            GenreContentType::AlbumGenres => "album genres",
            GenreContentType::Moods => "moods",
            GenreContentType::Styles => "styles",
        }
    }
}

/// Sort mode for albums within a genre/mood.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GenreSortMode {
    #[default]
    Artist,
    AlbumArtist,
    AlbumTitle,
}

impl GenreSortMode {
    pub fn next(&self) -> Self {
        match self {
            GenreSortMode::Artist => GenreSortMode::AlbumArtist,
            GenreSortMode::AlbumArtist => GenreSortMode::AlbumTitle,
            GenreSortMode::AlbumTitle => GenreSortMode::Artist,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            GenreSortMode::Artist => "artist",
            GenreSortMode::AlbumArtist => "album artist",
            GenreSortMode::AlbumTitle => "album",
        }
    }
}

/// Sort mode for the play queue in Now Playing view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QueueSortMode {
    /// Original queue order (as items were added)
    #[default]
    QueueOrder,
    /// Grouped by album
    Album,
    /// Shuffled order
    Shuffle,
}

impl QueueSortMode {
    pub fn next(&self) -> Self {
        match self {
            QueueSortMode::QueueOrder => QueueSortMode::Album,
            QueueSortMode::Album => QueueSortMode::Shuffle,
            QueueSortMode::Shuffle => QueueSortMode::QueueOrder,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            QueueSortMode::QueueOrder => "queue order",
            QueueSortMode::Album => "by artist/album",
            QueueSortMode::Shuffle => "shuffled",
        }
    }
}

/// UI focus (which panel has keyboard focus).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// Left panel (category list)
    Left,
    /// Right panel (albums or tracks)
    Right,
}

/// What the right panel is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RightPanelMode {
    /// Showing nothing (no selection)
    #[default]
    Empty,
    /// Showing albums for selected artist
    ArtistAlbums,
    /// Showing tracks for selected album (drilled down from artist)
    AlbumTracks,
    /// Showing tracks directly (for Albums or Playlists category)
    CategoryTracks,
    /// Showing albums for selected genre/mood
    CategoryAlbums,
}

/// What the similar view is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SimilarMode {
    #[default]
    Albums,
    Tracks,
}

/// Playback state.
#[derive(Debug, Clone)]
pub struct PlaybackState {
    pub status: PlayStatus,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub volume: f32,
    pub muted: bool,
    pub shuffle: bool,
    pub repeat_mode: RepeatMode,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            status: PlayStatus::Stopped,
            position_ms: 0,
            duration_ms: 0,
            volume: 0.8,
            muted: false,
            shuffle: false,
            repeat_mode: RepeatMode::Off,
        }
    }
}

/// Playback status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayStatus {
    Stopped,
    Playing,
    Paused,
    Buffering,
}

/// Repeat mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatMode {
    Off,
    All,
    One,
}

impl RepeatMode {
    pub fn next(self) -> Self {
        match self {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            RepeatMode::Off => "      ",
            RepeatMode::All => "repeat",
            RepeatMode::One => "rep. 1",
        }
    }
}

/// Waveform seekbar state.
#[derive(Debug, Clone, Default)]
pub struct WaveformState {
    /// Cached waveform data for current track.
    pub data: Option<WaveformData>,
    /// Track key this waveform is for.
    pub track_key: Option<String>,
    /// Whether waveform is being generated.
    pub generating: bool,
    /// Error message if generation failed.
    pub error: Option<String>,
}

/// Search result section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchSection {
    #[default]
    Artists,
    Albums,
    Tracks,
}

/// List selection states for different views.
#[derive(Debug, Default)]
pub struct ListStates {
    pub artists_index: usize,
    pub albums_index: usize,
    pub playlists_index: usize,
    pub right_albums_index: usize,  // Albums in right panel (for artist drill-down)
    pub tracks_index: usize,
    pub queue_index: usize,
    pub similar_index: usize,
    pub recently_played_index: usize,  // Recently played albums list
    pub search_section: SearchSection,
    pub search_item_index: usize,
}

impl ListStates {
    /// Reset all indices.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Settings screen section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsSection {
    #[default]
    Server,
    Libraries,
    Playback,
    Interface,
    Data,
    About,
}

impl SettingsSection {
    pub fn all() -> &'static [SettingsSection] {
        &[
            SettingsSection::Server,
            SettingsSection::Libraries,
            SettingsSection::Playback,
            SettingsSection::Interface,
            SettingsSection::Data,
            SettingsSection::About,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            SettingsSection::Server => "Server",
            SettingsSection::Libraries => "Libraries",
            SettingsSection::Playback => "Playback",
            SettingsSection::Interface => "Interface",
            SettingsSection::Data => "Data",
            SettingsSection::About => "About",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            SettingsSection::Server => SettingsSection::Libraries,
            SettingsSection::Libraries => SettingsSection::Playback,
            SettingsSection::Playback => SettingsSection::Interface,
            SettingsSection::Interface => SettingsSection::Data,
            SettingsSection::Data => SettingsSection::About,
            SettingsSection::About => SettingsSection::Server,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            SettingsSection::Server => SettingsSection::About,
            SettingsSection::Libraries => SettingsSection::Server,
            SettingsSection::Playback => SettingsSection::Libraries,
            SettingsSection::Interface => SettingsSection::Playback,
            SettingsSection::Data => SettingsSection::Interface,
            SettingsSection::About => SettingsSection::Data,
        }
    }
}

/// Settings screen focus (which panel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsFocus {
    /// Sections panel (left)
    #[default]
    Sections,
    /// Content panel (right)
    Content,
}

/// Settings screen state.
#[derive(Debug, Clone, Default)]
pub struct SettingsState {
    /// Which panel has focus
    pub focus: SettingsFocus,
    /// Which settings section is focused
    pub section: SettingsSection,
    /// Which item within the section is selected
    pub item_index: usize,
    /// Edit mode for current item
    pub editing: bool,
    /// Pending server discovery
    pub discovering_servers: bool,
    /// Username being edited (Server section)
    pub username_input: String,
    /// Password being edited (Server section)
    pub password_input: String,
    /// Which credential field is being edited (None = not editing credentials)
    pub editing_credential: Option<CredentialField>,
}

/// Which credential field is being edited in settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialField {
    Username,
    Password,
}

/// Input dialog for text entry (playlist names, etc.).
#[derive(Debug, Clone)]
pub struct InputDialog {
    /// Dialog title
    pub title: String,
    /// Current input text
    pub input: String,
    /// Action to dispatch on confirm
    pub action_type: InputDialogAction,
}

/// What action to take when input dialog is confirmed.
#[derive(Debug, Clone)]
pub enum InputDialogAction {
    SavePlaylist,
    AdventureLength,
}

/// Sonic Adventure creation state.
#[derive(Debug, Clone, Default)]
pub struct AdventureState {
    /// Adventure mode is active
    pub active: bool,
    /// Start track for the sonic bridge
    pub start_track: Option<Track>,
    /// End track for the sonic bridge
    pub end_track: Option<Track>,
    /// Desired track count (5-100)
    pub requested_length: usize,
    /// Currently generating the adventure
    pub generating: bool,
}
