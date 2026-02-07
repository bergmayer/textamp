//! Application state definitions.
//!
//! Uses the Elm Architecture pattern with a single state struct.
//! UI modeled after musikcube: Browse (left: categories, right: tracks), Queue, Search, etc.

use crate::api::models::{Album, Artist, Genre, Library, Playlist, PlexServer, Station, Track, SearchResults};
use crate::plex::CachedFolder;
use crate::services::{FolderNavigationState, WaveformData};
use crate::ui::theme::ThemeName;
use std::collections::HashMap;

/// Marquee scroll animation phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarqueePhase {
    /// Initial 4-second pause showing truncated text
    Waiting,
    /// Scrolling left, revealing full content
    Scrolling,
    /// 2-second pause at the end with full text visible
    PausedAtEnd,
    /// Text fits in the display width, no scrolling needed
    Inactive,
}

/// State for marquee scroll animation on truncated text.
#[derive(Debug, Clone)]
pub struct MarqueeState {
    /// Key identifying current selection (e.g. "np:5", "miller:2:3")
    pub selection_key: String,
    /// Full un-truncated text being scrolled
    pub full_text: String,
    /// Available display width for the field
    pub display_width: usize,
    /// Current scroll offset (chars from start)
    pub scroll_offset: usize,
    /// Current animation phase
    pub phase: MarqueePhase,
    /// When current phase started
    pub phase_start: std::time::Instant,
    /// When last scroll step happened (for 150ms timing)
    pub last_scroll: std::time::Instant,
}

impl Default for MarqueeState {
    fn default() -> Self {
        let now = std::time::Instant::now();
        Self {
            selection_key: String::new(),
            full_text: String::new(),
            display_width: 0,
            scroll_offset: 0,
            phase: MarqueePhase::Inactive,
            phase_start: now,
            last_scroll: now,
        }
    }
}

impl MarqueeState {
    /// Reset marquee for a new selection.
    pub fn reset(&mut self, key: String, full_text: String, display_width: usize) {
        use unicode_width::UnicodeWidthStr;

        let text_width = UnicodeWidthStr::width(full_text.as_str());
        let now = std::time::Instant::now();

        self.selection_key = key;
        self.full_text = full_text;
        self.display_width = display_width;
        self.scroll_offset = 0;
        self.phase_start = now;
        self.last_scroll = now;

        if text_width <= display_width {
            self.phase = MarqueePhase::Inactive;
        } else {
            self.phase = MarqueePhase::Waiting;
        }
    }

    /// Get the display slice for the current scroll offset.
    /// Returns a string padded to exactly display_width.
    pub fn display_text(&self) -> String {
        if self.phase == MarqueePhase::Inactive || self.full_text.is_empty() {
            return crate::util::pad_right(&self.full_text, self.display_width);
        }

        match self.phase {
            MarqueePhase::Waiting | MarqueePhase::PausedAtEnd if self.scroll_offset == 0 => {
                // Show normally truncated text
                crate::util::pad_right(&self.full_text, self.display_width)
            }
            _ => {
                // Extract substring starting at scroll_offset (by display column)
                let mut col = 0;
                let mut start_byte = 0;
                let mut found_start = false;
                for (i, ch) in self.full_text.char_indices() {
                    let ch_w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                    if col >= self.scroll_offset && !found_start {
                        start_byte = i;
                        found_start = true;
                    }
                    col += ch_w;
                }
                if !found_start {
                    start_byte = self.full_text.len();
                }
                let substr = &self.full_text[start_byte..];
                crate::util::pad_right(substr, self.display_width)
            }
        }
    }

    /// Maximum scroll offset (how far we can scroll).
    pub fn max_scroll(&self) -> usize {
        use unicode_width::UnicodeWidthStr;
        let text_width = UnicodeWidthStr::width(self.full_text.as_str());
        text_width.saturating_sub(self.display_width)
    }
}

/// Notification type - determines display behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationType {
    /// Ongoing operation - stays visible while active
    Ongoing,
    /// Toast - appears briefly then auto-disappears
    Toast,
}

/// A notification to display in the transport bar.
#[derive(Debug, Clone)]
pub struct Notification {
    pub message: String,
    pub notification_type: NotificationType,
}

impl Notification {
    pub fn ongoing(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            notification_type: NotificationType::Ongoing,
        }
    }

    pub fn toast(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            notification_type: NotificationType::Toast,
        }
    }
}

// ============================================================================
// Miller Column Navigation for Browse Views
// ============================================================================

/// Item type in a browse column.
#[derive(Debug, Clone)]
pub enum BrowseItem {
    Artist {
        key: String,
        title: String,
    },
    Album {
        key: String,
        title: String,
        artist: String,
        year: Option<u16>,
    },
    Track {
        key: String,
        title: String,
        duration_ms: u64,
        track_number: Option<u32>,
    },
    Genre {
        key: String,
        title: String,
    },
    Playlist {
        key: String,
        title: String,
        track_count: Option<u32>,
    },
    /// "All Tracks" entry - shows all tracks by an artist.
    AllTracks {
        artist_key: String,
        artist_name: String,
    },
}

impl BrowseItem {
    pub fn key(&self) -> &str {
        match self {
            BrowseItem::Artist { key, .. } => key,
            BrowseItem::Album { key, .. } => key,
            BrowseItem::Track { key, .. } => key,
            BrowseItem::Genre { key, .. } => key,
            BrowseItem::Playlist { key, .. } => key,
            BrowseItem::AllTracks { artist_key, .. } => artist_key,
        }
    }

    pub fn title(&self) -> &str {
        match self {
            BrowseItem::Artist { title, .. } => title,
            BrowseItem::Album { title, .. } => title,
            BrowseItem::Track { title, .. } => title,
            BrowseItem::Genre { title, .. } => title,
            BrowseItem::Playlist { title, .. } => title,
            BrowseItem::AllTracks { .. } => "► All Tracks",
        }
    }

    pub fn is_drillable(&self) -> bool {
        // AllTracks is drillable (loads tracks column), Track is not
        !matches!(self, BrowseItem::Track { .. })
    }

    /// Convert a list of Artists to BrowseItems.
    pub fn from_artists(artists: &[Artist]) -> Vec<BrowseItem> {
        artists.iter().map(|a| BrowseItem::Artist {
            key: a.rating_key.clone(),
            title: a.title.clone(),
        }).collect()
    }

    /// Convert a list of Albums to BrowseItems.
    pub fn from_albums(albums: &[Album]) -> Vec<BrowseItem> {
        albums.iter().map(|a| BrowseItem::Album {
            key: a.rating_key.clone(),
            title: a.title.clone(),
            artist: a.parent_title.clone().unwrap_or_default(),
            year: a.year,
        }).collect()
    }

    /// Convert a list of Tracks to BrowseItems.
    pub fn from_tracks(tracks: &[Track]) -> Vec<BrowseItem> {
        tracks.iter().map(|t| BrowseItem::Track {
            key: t.rating_key.clone(),
            title: t.title.clone(),
            duration_ms: t.duration_ms(),
            track_number: t.index,
        }).collect()
    }

    /// Convert a list of Genres to BrowseItems.
    pub fn from_genres(genres: &[Genre]) -> Vec<BrowseItem> {
        genres.iter().map(|g| BrowseItem::Genre {
            key: g.key.clone(),
            title: g.title.clone(),
        }).collect()
    }

    /// Convert a list of Playlists to BrowseItems.
    pub fn from_playlists(playlists: &[Playlist]) -> Vec<BrowseItem> {
        playlists.iter().map(|p| BrowseItem::Playlist {
            key: p.rating_key.clone(),
            title: p.title.clone(),
            track_count: p.leaf_count,
        }).collect()
    }
}

/// A single column in the Miller columns browse view.
#[derive(Debug, Clone)]
pub struct BrowseColumn {
    /// Column title (shown in header for root column only)
    pub title: String,
    /// Items in this column
    pub items: Vec<BrowseItem>,
    /// Currently selected index
    pub selected_index: usize,
    /// Full Track objects for track columns (used for playback with media info)
    pub tracks: Vec<crate::plex::models::Track>,
    /// Original items before shuffle (None if not shuffled)
    original_items: Option<Vec<BrowseItem>>,
    /// Original tracks before shuffle (None if not shuffled)
    original_tracks: Option<Vec<crate::plex::models::Track>>,
}

impl BrowseColumn {
    pub fn new(title: impl Into<String>, items: Vec<BrowseItem>) -> Self {
        Self {
            title: title.into(),
            items,
            selected_index: 0,
            tracks: vec![],
            original_items: None,
            original_tracks: None,
        }
    }

    /// Create a column with full track objects stored for playback.
    pub fn new_with_tracks(title: impl Into<String>, items: Vec<BrowseItem>, tracks: Vec<crate::plex::models::Track>) -> Self {
        Self {
            title: title.into(),
            items,
            selected_index: 0,
            tracks,
            original_items: None,
            original_tracks: None,
        }
    }

    pub fn selected_item(&self) -> Option<&BrowseItem> {
        self.items.get(self.selected_index)
    }

    /// Whether this column is currently shuffled.
    pub fn is_shuffled(&self) -> bool {
        self.original_items.is_some()
    }

    /// Shuffle items (and tracks in parallel). Saves originals for restore.
    pub fn shuffle(&mut self) {
        use rand::seq::SliceRandom;
        // Save originals (fresh copy each time for re-shuffle)
        self.original_items = Some(self.items.clone());
        self.original_tracks = if self.tracks.is_empty() { None } else { Some(self.tracks.clone()) };

        // Build index permutation and apply to both vecs
        let mut indices: Vec<usize> = (0..self.items.len()).collect();
        let mut rng = rand::rng();
        indices.shuffle(&mut rng);

        let orig_items = self.original_items.as_ref().unwrap();
        self.items = indices.iter().map(|&i| orig_items[i].clone()).collect();

        if let Some(ref orig_tracks) = self.original_tracks {
            self.tracks = indices.iter().filter_map(|&i| orig_tracks.get(i).cloned()).collect();
        }

        self.selected_index = 0;
    }

    /// Restore original order.
    pub fn unshuffle(&mut self) {
        if let Some(items) = self.original_items.take() {
            self.items = items;
        }
        if let Some(tracks) = self.original_tracks.take() {
            self.tracks = tracks;
        }
        self.selected_index = 0;
    }
}

/// Navigation state for Miller column browsing.
#[derive(Debug, Clone, Default)]
pub struct BrowseNavigationState {
    /// Columns from left to right
    pub columns: Vec<BrowseColumn>,
    /// Which column currently has focus (0-indexed)
    pub focused_column: usize,
    /// Loading indicator
    pub loading: bool,
}

impl BrowseNavigationState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialize with a root column.
    pub fn with_root(title: impl Into<String>, items: Vec<BrowseItem>) -> Self {
        Self {
            columns: vec![BrowseColumn::new(title, items)],
            focused_column: 0,
            loading: false,
        }
    }

    /// Get the focused column.
    pub fn focused(&self) -> Option<&BrowseColumn> {
        self.columns.get(self.focused_column)
    }

    /// Get the focused column mutably.
    pub fn focused_mut(&mut self) -> Option<&mut BrowseColumn> {
        self.columns.get_mut(self.focused_column)
    }

    /// Get the selected item in the focused column.
    pub fn selected_item(&self) -> Option<&BrowseItem> {
        self.focused().and_then(|c| c.selected_item())
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
    pub fn push_column(&mut self, column: BrowseColumn) {
        // Remove columns to the right of focus
        self.columns.truncate(self.focused_column + 1);
        // Add new column
        self.columns.push(column);
        // Move focus to new column
        self.focused_column = self.columns.len() - 1;
    }

    /// Clear columns to the right of the focused column.
    pub fn truncate_right(&mut self) {
        self.columns.truncate(self.focused_column + 1);
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
            let max = col.items.len().saturating_sub(1);
            if col.selected_index < max {
                col.selected_index += 1;
            }
        }
    }

    /// Move to a specific index in the focused column.
    pub fn move_to(&mut self, index: usize) {
        if let Some(col) = self.focused_mut() {
            if index < col.items.len() {
                col.selected_index = index;
            }
        }
    }

    /// Get the number of columns.
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    /// Reset to a single root column.
    pub fn reset(&mut self, title: impl Into<String>, items: Vec<BrowseItem>) {
        self.columns = vec![BrowseColumn::new(title, items)];
        self.focused_column = 0;
        self.loading = false;
    }

    /// Check if empty (no columns).
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }
}

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
    /// Plex Pass status (cached during auth flow for server selection)
    pub has_plex_pass: bool,
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
    /// Consecutive playback errors (for auto-skip with limit)
    pub consecutive_playback_errors: u32,
    /// Plex session identifier for timeline reporting.
    /// Generated when starting a new playback context (queue, radio, etc.).
    /// Used to correlate all timeline reports to a single session.
    pub plex_session_id: Option<String>,
    /// Last time a progress report was sent to Plex (for periodic ~10s updates).
    pub last_progress_report: Option<std::time::Instant>,

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
    pub status_show_time: Option<std::time::Instant>,

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
    /// Cached subfolder contents: folder_key -> CachedFolder with timestamp.
    /// Each entry has its own timestamp for individual staleness tracking.
    /// Entries older than 32 days are deleted on cache load (not refreshed).
    /// Subfolders are only cached when navigated to (lazy caching).
    pub folder_contents_cache: HashMap<String, CachedFolder>,

    // Miller column navigation for browse categories
    pub artist_nav: BrowseNavigationState,
    pub genre_nav: BrowseNavigationState,
    pub playlist_nav: BrowseNavigationState,

    // In-memory playlist tracks cache (playlist_key -> tracks)
    pub playlist_tracks_cache: HashMap<String, Vec<Track>>,

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

    // Now Playing view mode (Queue vs Now Playing)
    pub now_playing_mode: NowPlayingMode,
    pub recently_played_albums: Vec<Album>,
    pub recently_played_loading: bool,

    // Playlists view mode (All vs Stations vs Recently Added vs Recently Played)
    pub playlists_mode: PlaylistsMode,
    pub recently_added_albums: Vec<Album>,
    pub recently_added_loading: bool,

    // Cache management
    pub cache_dirty: bool,
    pub last_input_time: std::time::Instant,
    pub last_cache_save: std::time::Instant,
    pub cache_save_in_progress: bool,
    pub background_refresh_in_progress: std::collections::HashSet<RefreshCategory>,

    // Waveform seekbar state
    pub waveform: WaveformState,

    // Toast notification
    pub toast_message: Option<String>,
    pub toast_show_time: Option<std::time::Instant>,

    // Confirmation dialog
    pub confirm_dialog: Option<ConfirmDialog>,

    // Inline list filter state
    pub list_filter_active: bool,
    pub list_filter_query: String,
    pub list_filter_version: u64,
    pub list_filter_loading: bool,
    pub list_filter_results: Option<ListFilterResults>,
    pub list_filter_selected: usize,  // Index into matched_indices (which filtered result is selected)
    pub list_filter_category: BrowseCategory,  // Which category the filter applies to
    pub list_filter_column: usize,  // Which column index the filter applies to

    // Search popup state (Ctrl+F - floating dialog, not a full view)
    pub search_popup_active: bool,

    // Library picker popup state (Ctrl+Alt+S)
    pub library_picker_active: bool,
    pub library_picker_index: usize,

    // Marquee scroll animation state (RefCell for interior mutability during render)
    pub marquee: std::cell::RefCell<MarqueeState>,

    // Library switch loading state
    pub library_loading: bool,
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
            RadioSeedMode::Track => "sonic track radio",
            RadioSeedMode::Album => "sonic album radio",
            RadioSeedMode::Artist => "sonic artist radio",
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
    /// Original stations before shuffle (None if not shuffled)
    original_stations: Option<Vec<Station>>,
}

impl StationColumn {
    /// Create a new column.
    pub fn new(key: Option<String>, title: String, stations: Vec<Station>) -> Self {
        Self {
            key,
            title,
            stations,
            selected_index: 0,
            original_stations: None,
        }
    }

    /// Get the selected station, if any.
    pub fn selected_station(&self) -> Option<&Station> {
        self.stations.get(self.selected_index)
    }

    /// Whether this column is currently shuffled.
    pub fn is_shuffled(&self) -> bool {
        self.original_stations.is_some()
    }

    /// Shuffle stations. Saves originals for restore.
    pub fn shuffle(&mut self) {
        use rand::seq::SliceRandom;
        self.original_stations = Some(self.stations.clone());
        let mut rng = rand::rng();
        self.stations.shuffle(&mut rng);
        self.selected_index = 0;
    }

    /// Restore original order.
    pub fn unshuffle(&mut self) {
        if let Some(stations) = self.original_stations.take() {
            self.stations = stations;
        }
        self.selected_index = 0;
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
        self.truncate_right_columns();
        // Add new column
        self.columns.push(column);
        // Move focus to new column
        self.focused_column = self.columns.len() - 1;
    }

    /// Clear columns to the right of the focused column.
    /// Call this when selection changes to prevent stale column data.
    pub fn truncate_right_columns(&mut self) {
        self.columns.truncate(self.focused_column + 1);
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

// Sonic radio (Alt+R) - separate from station-based radio (via Ctrl+G Stations)
// radio_state is used for sonic radio seeded from user selection.
// This is distinct from RadioPlaybackState which is for Plexamp stations.
/// Radio mode for sonic radio (track/album/artist radio via Alt+R).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RadioMode {
    #[default]
    Off,
    /// Sonic track radio - plays similar tracks based on a seed track
    Track,
    /// Sonic album radio - plays album then similar albums
    Album,
    /// Sonic artist radio - plays artist's tracks then similar
    Artist,
}

impl RadioMode {
    pub fn label(&self) -> &'static str {
        match self {
            RadioMode::Off => "",
            RadioMode::Track => "sonic track radio",
            RadioMode::Album => "sonic album radio",
            RadioMode::Artist => "sonic artist radio",
        }
    }
}

/// Sonic radio state for similarity-based playback (Alt+R).
/// Distinct from RadioPlaybackState which handles Plexamp stations (via Ctrl+G).
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
            consecutive_playback_errors: 0,
            plex_session_id: None,
            last_progress_report: None,
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
            status_show_time: None,
            input_dialog: None,
            alt_held: false,
            search_tab: SearchTab::default(),
            terminal_width: 80,
            terminal_height: 24,
            image_loaded: HashMap::new(),
            settings_state: SettingsState::default(),
            folder_state: None,
            folder_contents_cache: HashMap::new(),
            artist_nav: BrowseNavigationState::new(),
            genre_nav: BrowseNavigationState::new(),
            playlist_nav: BrowseNavigationState::new(),
            playlist_tracks_cache: HashMap::new(),
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
            cache_dirty: false,
            last_input_time: std::time::Instant::now(),
            last_cache_save: std::time::Instant::now(),
            cache_save_in_progress: false,
            background_refresh_in_progress: std::collections::HashSet::new(),
            waveform: WaveformState::default(),
            toast_message: None,
            toast_show_time: None,
            confirm_dialog: None,
            list_filter_active: false,
            list_filter_query: String::new(),
            list_filter_version: 0,
            list_filter_loading: false,
            list_filter_results: None,
            list_filter_selected: 0,
            list_filter_category: BrowseCategory::Artists,
            list_filter_column: 0,
            search_popup_active: false,
            library_picker_active: false,
            library_picker_index: 0,
            marquee: std::cell::RefCell::new(MarqueeState::default()),
            library_loading: false,
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

    /// Set a status message (auto-clears after 5 seconds).
    pub fn set_status(&mut self, msg: String) {
        self.status_message = Some(msg);
        self.status_show_time = Some(std::time::Instant::now());
    }

    /// Clear the status message.
    pub fn clear_status(&mut self) {
        self.status_message = None;
        self.status_show_time = None;
    }

    /// Set a toast notification (auto-clears after 5 seconds).
    pub fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast_message = Some(msg.into());
        self.toast_show_time = Some(std::time::Instant::now());
    }

    /// Get the current notification to display (ongoing takes priority over toast).
    /// Returns None if no notification should be shown.
    pub fn current_notification(&self) -> Option<Notification> {
        // Priority 1: Adventure mode notifications (ongoing)
        if self.adventure.active {
            if self.adventure.generating {
                return Some(Notification::ongoing("🌟 Generating sonic bridge..."));
            }
            // Don't show adventure selection messages here - they're in the transport text
        }

        // Priority 2: Library loading (ongoing)
        if self.library_loading {
            return Some(Notification::ongoing("Loading library..."));
        }

        // Priority 3: Station loading (ongoing)
        if self.station_nav.loading {
            return Some(Notification::ongoing("Loading station..."));
        }

        // Priority 4: Background refresh (ongoing)
        if !self.background_refresh_in_progress.is_empty() {
            let categories: Vec<_> = self.background_refresh_in_progress
                .iter()
                .map(|c| c.display_name())
                .collect();
            let msg = if categories.len() == 1 {
                format!("Refreshing {}...", categories[0])
            } else {
                format!("Refreshing {}...", categories.join(", "))
            };
            return Some(Notification::ongoing(msg));
        }

        // Priority 5: Waveform generation (ongoing)
        if self.waveform.generating {
            return Some(Notification::ongoing("Generating waveform..."));
        }

        // Priority 6: Cache saving (ongoing)
        if self.cache_save_in_progress {
            return Some(Notification::ongoing("Saving cache..."));
        }

        // Priority 7: Toast notifications (transient)
        if let Some(ref msg) = self.toast_message {
            return Some(Notification::toast(msg.clone()));
        }

        // Priority 8: Status messages (transient)
        if let Some(ref msg) = self.status_message {
            return Some(Notification::toast(msg.clone()));
        }

        None
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
                    PlaylistsMode::Stations => 0, // Handled via station_nav
                    PlaylistsMode::RecentlyAdded => self.recently_added_albums.len(),
                    PlaylistsMode::RecentlyPlayed => self.recently_played_albums.len(),
                }
            }
            BrowseCategory::Genres => {
                // Stations are in the genre cycle, but use station_nav for rendering
                if self.genre_content_type == GenreContentType::Stations {
                    0 // Handled separately via station_nav
                } else {
                    self.current_genre_list().len()
                }
            }
            BrowseCategory::Folders => 0, // Handled separately via folder_state
        }
    }

    /// Get the current genre list based on content type.
    /// Note: Returns empty vec for Stations since stations use station_nav.
    pub fn current_genre_list(&self) -> &Vec<Genre> {
        match self.genre_content_type {
            GenreContentType::Genres => &self.genres,
            GenreContentType::ArtistGenres => &self.artist_genres,
            GenreContentType::AlbumGenres => &self.album_genres,
            GenreContentType::Moods => &self.moods,
            GenreContentType::Styles => &self.styles,
            GenreContentType::Stations => &self.genres, // Stations use station_nav, not genre list
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
                    PlaylistsMode::Stations => None, // Handled via station_nav
                    PlaylistsMode::RecentlyAdded => {
                        self.recently_added_albums.get(self.list_state.playlists_index)
                            .map(|a| a.rating_key.clone())
                    }
                    PlaylistsMode::RecentlyPlayed => None, // Handled via playlist_nav
                }
            }
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
                    PlaylistsMode::Stations => None, // Handled via station_nav
                    PlaylistsMode::RecentlyAdded => {
                        self.recently_added_albums.get(self.list_state.playlists_index)
                            .map(|a| a.title.clone())
                    }
                    PlaylistsMode::RecentlyPlayed => None, // Handled via playlist_nav
                }
            }
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
    Connected { username: String, has_plex_pass: bool },
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
    /// Now Playing view with artwork and waveform seekbar
    NowPlaying,
}

impl NowPlayingMode {
    pub fn next(&self) -> Self {
        match self {
            NowPlayingMode::Queue => NowPlayingMode::NowPlaying,
            NowPlayingMode::NowPlaying => NowPlayingMode::Queue,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            NowPlayingMode::Queue => NowPlayingMode::NowPlaying,
            NowPlayingMode::NowPlaying => NowPlayingMode::Queue,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            NowPlayingMode::Queue => "queue",
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
    /// Show radio stations
    Stations,
    /// Show recently added albums
    RecentlyAdded,
    /// Show recently played albums
    RecentlyPlayed,
}

impl PlaylistsMode {
    pub fn next(&self) -> Self {
        match self {
            PlaylistsMode::All => PlaylistsMode::Stations,
            PlaylistsMode::Stations => PlaylistsMode::RecentlyAdded,
            PlaylistsMode::RecentlyAdded => PlaylistsMode::RecentlyPlayed,
            PlaylistsMode::RecentlyPlayed => PlaylistsMode::All,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            PlaylistsMode::All => PlaylistsMode::RecentlyPlayed,
            PlaylistsMode::Stations => PlaylistsMode::All,
            PlaylistsMode::RecentlyAdded => PlaylistsMode::Stations,
            PlaylistsMode::RecentlyPlayed => PlaylistsMode::RecentlyAdded,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            PlaylistsMode::All => "playlists",
            PlaylistsMode::Stations => "stations",
            PlaylistsMode::RecentlyAdded => "recently added",
            PlaylistsMode::RecentlyPlayed => "recently played",
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
    Genres,
    Folders,
}

impl BrowseCategory {
    pub fn all() -> &'static [BrowseCategory] {
        &[
            BrowseCategory::Artists,
            BrowseCategory::Playlists,
            BrowseCategory::Genres,
            BrowseCategory::Folders,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            BrowseCategory::Artists => "artists",
            BrowseCategory::Playlists => "playlists",
            BrowseCategory::Genres => "genres",
            BrowseCategory::Folders => "folders",
        }
    }

    pub fn shortcut(&self) -> char {
        match self {
            BrowseCategory::Artists => 'a',
            BrowseCategory::Playlists => 'p',
            BrowseCategory::Genres => 'g',
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

    /// Cycle to the previous mode (Artist ← Album Artist ← Album ← Artist).
    pub fn prev(&self) -> Self {
        match self {
            ArtistViewMode::Artist => ArtistViewMode::Album,
            ArtistViewMode::AlbumArtist => ArtistViewMode::Artist,
            ArtistViewMode::Album => ArtistViewMode::AlbumArtist,
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

/// Genre content type - genres, normalized genres, moods, styles, or stations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GenreContentType {
    #[default]
    Genres,
    ArtistGenres,
    AlbumGenres,
    Moods,
    Styles,
    Stations,
}

impl GenreContentType {
    /// Cycle to the next content type (Genres -> Artist -> Album -> Moods -> Styles -> Stations -> Genres).
    pub fn next(&self) -> Self {
        match self {
            GenreContentType::Genres => GenreContentType::ArtistGenres,
            GenreContentType::ArtistGenres => GenreContentType::AlbumGenres,
            GenreContentType::AlbumGenres => GenreContentType::Moods,
            GenreContentType::Moods => GenreContentType::Styles,
            GenreContentType::Styles => GenreContentType::Stations,
            GenreContentType::Stations => GenreContentType::Genres,
        }
    }

    /// Cycle to the previous content type (Genres <- Artist <- Album <- Moods <- Styles <- Stations <- Genres).
    pub fn prev(&self) -> Self {
        match self {
            GenreContentType::Genres => GenreContentType::Stations,
            GenreContentType::ArtistGenres => GenreContentType::Genres,
            GenreContentType::AlbumGenres => GenreContentType::ArtistGenres,
            GenreContentType::Moods => GenreContentType::AlbumGenres,
            GenreContentType::Styles => GenreContentType::Moods,
            GenreContentType::Stations => GenreContentType::Styles,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            GenreContentType::Genres => "genres",
            GenreContentType::ArtistGenres => "artist genres",
            GenreContentType::AlbumGenres => "album genres",
            GenreContentType::Moods => "moods",
            GenreContentType::Styles => "styles",
            GenreContentType::Stations => "stations",
        }
    }
}

/// Sort mode for the play queue in Now Playing view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QueueSortMode {
    /// Original queue order (as items were added)
    #[default]
    QueueOrder,
    /// Shuffled order
    Shuffle,
}

impl QueueSortMode {
    pub fn name(&self) -> &'static str {
        match self {
            QueueSortMode::QueueOrder => "queue order",
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
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            status: PlayStatus::Stopped,
            position_ms: 0,
            duration_ms: 0,
            volume: 0.8,
            muted: false,
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
    Account,
    Libraries,
    Playback,
    Interface,
    About,
}

impl SettingsSection {
    pub fn all() -> &'static [SettingsSection] {
        &[
            SettingsSection::Account,
            SettingsSection::Libraries,
            SettingsSection::Playback,
            SettingsSection::Interface,
            SettingsSection::About,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            SettingsSection::Account => "Account",
            SettingsSection::Libraries => "Libraries",
            SettingsSection::Playback => "Playback",
            SettingsSection::Interface => "Interface",
            SettingsSection::About => "About",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            SettingsSection::Account => SettingsSection::Libraries,
            SettingsSection::Libraries => SettingsSection::Playback,
            SettingsSection::Playback => SettingsSection::Interface,
            SettingsSection::Interface => SettingsSection::About,
            SettingsSection::About => SettingsSection::Account,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            SettingsSection::Account => SettingsSection::About,
            SettingsSection::Libraries => SettingsSection::Account,
            SettingsSection::Playback => SettingsSection::Libraries,
            SettingsSection::Interface => SettingsSection::Playback,
            SettingsSection::About => SettingsSection::Interface,
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
    /// Username being edited (Account section sign-in)
    pub username_input: String,
    /// Password being edited (Account section sign-in)
    pub password_input: String,
    /// Which credential field is being edited (None = not editing credentials)
    pub editing_credential: Option<CredentialField>,
    /// Whether the Account section is in sign-in mode (showing login form)
    pub signing_in: bool,
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

/// Category for cache refresh operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefreshCategory {
    Artists,
    AlbumArtists,
    Albums,
    Playlists,
    RecentlyAdded,
    Genres,
    ArtistGenres,
    AlbumGenres,
    Moods,
    Styles,
    Stations,
    RecentlyPlayed,
    Folders,
}

impl RefreshCategory {
    /// Get all categories in priority order.
    pub fn all() -> &'static [RefreshCategory] {
        &[
            RefreshCategory::Artists,
            RefreshCategory::AlbumArtists,
            RefreshCategory::Albums,
            RefreshCategory::Playlists,
            RefreshCategory::RecentlyAdded,
            RefreshCategory::RecentlyPlayed,
            RefreshCategory::Genres,
            RefreshCategory::ArtistGenres,
            RefreshCategory::AlbumGenres,
            RefreshCategory::Moods,
            RefreshCategory::Styles,
            RefreshCategory::Stations,
            RefreshCategory::Folders,
        ]
    }

    /// Get display name for status messages and toasts.
    pub fn display_name(&self) -> &'static str {
        match self {
            RefreshCategory::Artists => "Artists",
            RefreshCategory::AlbumArtists => "Album Artists",
            RefreshCategory::Albums => "Albums",
            RefreshCategory::Playlists => "Playlists",
            RefreshCategory::RecentlyAdded => "Recently Added",
            RefreshCategory::RecentlyPlayed => "Recently Played",
            RefreshCategory::Genres => "Genres",
            RefreshCategory::ArtistGenres => "Artist Genres",
            RefreshCategory::AlbumGenres => "Album Genres",
            RefreshCategory::Moods => "Moods",
            RefreshCategory::Styles => "Styles",
            RefreshCategory::Stations => "Stations",
            RefreshCategory::Folders => "Folders",
        }
    }
}

/// Confirmation dialog for user prompts.
#[derive(Debug, Clone)]
pub struct ConfirmDialog {
    pub title: String,
    pub message: String,
    pub on_confirm: ConfirmAction,
}

/// Action to take when confirmation dialog is confirmed.
#[derive(Debug, Clone)]
pub enum ConfirmAction {
    RefreshCache,
}

/// Results from inline list filter.
#[derive(Debug, Clone, Default)]
pub struct ListFilterResults {
    /// Indices of matched items in the original list (in priority order)
    pub matched_indices: Vec<usize>,
    /// Total number of matches found
    pub total_matches: usize,
    /// Whether there are more results beyond the limit
    pub has_more: bool,
}
