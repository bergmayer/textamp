//! Application state definitions.
//!
//! Uses the Elm Architecture pattern with a single state struct.
//! Shared navigation, playback, analysis and workflow state for all frontends.

use std::collections::VecDeque;

/// Capacity of the live vectorscope sample buffer. ~2 048 stereo
/// pairs at 48 kHz is roughly 43 ms of audio — long enough for the
/// Lissajous trace to look like a continuous shape, short enough
/// for it to "breathe" with the music. Matches the GUI buffer size.
pub const VECTORSCOPE_BUFFER_LEN: usize = 2_048;

/// Generate `next()` and `prev()` methods for cyclic enums.
///
/// Given variants in order, `next()` advances to the next variant (wrapping around)
/// and `prev()` goes to the previous variant (wrapping around).
macro_rules! cyclic_enum {
    ($name:ident, $($variant:ident),+ $(,)?) => {
        impl $name {
            const CYCLE_ORDER: &'static [$name] = &[$($name::$variant),+];

            pub fn next(&self) -> Self {
                let idx = Self::CYCLE_ORDER.iter().position(|v| v == self).unwrap_or(0);
                Self::CYCLE_ORDER[(idx + 1) % Self::CYCLE_ORDER.len()]
            }

            pub fn prev(&self) -> Self {
                let idx = Self::CYCLE_ORDER.iter().position(|v| v == self).unwrap_or(0);
                Self::CYCLE_ORDER[(idx + Self::CYCLE_ORDER.len() - 1) % Self::CYCLE_ORDER.len()]
            }
        }
    };
}

use crate::app::theme::ThemeName;
use crate::library::models::{Album, Artist, Genre, Playlist, SearchResults, Station, Track};
use crate::miller::{MillerColumn, MillerState};

use crate::media::WaveformData;
use crate::services::FolderNavigationState;
use crate::util::SecretString;
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

    /// Advance the marquee animation by one tick.
    pub fn tick(&mut self) {
        match self.phase {
            MarqueePhase::Waiting => {
                if self.phase_start.elapsed() >= std::time::Duration::from_secs(4) {
                    self.phase = MarqueePhase::Scrolling;
                    self.phase_start = std::time::Instant::now();
                    self.last_scroll = std::time::Instant::now();
                }
            }
            MarqueePhase::Scrolling => {
                if self.last_scroll.elapsed() >= std::time::Duration::from_millis(150) {
                    self.scroll_offset += 1;
                    self.last_scroll = std::time::Instant::now();
                    let max = self.max_scroll();
                    if self.scroll_offset >= max {
                        self.phase = MarqueePhase::PausedAtEnd;
                        self.phase_start = std::time::Instant::now();
                    }
                }
            }
            MarqueePhase::PausedAtEnd => {
                if self.phase_start.elapsed() >= std::time::Duration::from_secs(2) {
                    self.scroll_offset = 0;
                    self.phase = MarqueePhase::Waiting;
                    self.phase_start = std::time::Instant::now();
                }
            }
            MarqueePhase::Inactive => {}
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationTarget {
    Downloads,
    RefreshLibrary,
}

/// A notification to display in the transport bar.
#[derive(Debug, Clone)]
pub struct Notification {
    pub target: Option<NotificationTarget>,
    pub message: String,
    pub notification_type: NotificationType,
}

impl Notification {
    pub fn ongoing(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            notification_type: NotificationType::Ongoing,
            target: None,
        }
    }

    pub fn toast(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            notification_type: NotificationType::Toast,
            target: None,
        }
    }
}

// ============================================================================
// Miller Column Navigation for Browse Views
// ============================================================================

/// What `BrowseItem::AllTracks` is offering "All" of. Mirrors the
/// distinct loaders in `MillerAction` (Library / Compilations /
/// per-compilation-artist / per-artist) so the click-drill match is
/// exhaustive at compile time and impossible to misroute.
#[derive(Debug, Clone)]
pub enum AllTracksScope {
    /// Every track in the library — pinned at the top of the merged
    /// "All Albums" column.
    Library,
    /// Every compilation track — pinned at the top of the merged
    /// Compilations album view.
    AllCompilations,
    /// Every compilation track by a specific artist — pinned at the
    /// top of that artist's compilation-album column.
    CompilationsByArtist {
        artist_key: String,
        artist_name: String,
    },
    /// Every track by a specific artist — pinned at the top of that
    /// artist's albums column.
    Artist {
        artist_key: String,
        artist_name: String,
    },
}

impl AllTracksScope {
    /// Stable identifier used by `BrowseItem::key()`. Sentinel
    /// strings preserve uniqueness for the global scopes; per-artist
    /// scopes use the artist's server rating key directly (which is
    /// also the artwork-cache key).
    pub fn key(&self) -> &str {
        match self {
            AllTracksScope::Library => "__all_library__",
            AllTracksScope::AllCompilations => "__all_comp__",
            AllTracksScope::CompilationsByArtist { artist_key, .. } => artist_key,
            AllTracksScope::Artist { artist_key, .. } => artist_key,
        }
    }

    /// Display name of the underlying artist, when this row is
    /// scoped to one. `None` for the library-wide and all-comps
    /// scopes (their column titles read "All Tracks" or
    /// "Compilations" with no artist suffix).
    pub fn artist_name(&self) -> Option<&str> {
        match self {
            AllTracksScope::Library | AllTracksScope::AllCompilations => None,
            AllTracksScope::CompilationsByArtist { artist_name, .. }
            | AllTracksScope::Artist { artist_name, .. } => Some(artist_name),
        }
    }

    /// server artist rating-key when this row is scoped to one. Used
    /// by the artwork loader to cache per-artist thumbs.
    pub fn artist_key(&self) -> Option<&str> {
        match self {
            AllTracksScope::Library | AllTracksScope::AllCompilations => None,
            AllTracksScope::CompilationsByArtist { artist_key, .. }
            | AllTracksScope::Artist { artist_key, .. } => Some(artist_key),
        }
    }
}

/// Item type in a browse column.
#[derive(Debug, Clone)]
pub enum BrowseItem {
    /// A saved Describe music query; drilling restores its ranked result list.
    Description {
        query: String,
    },
    Artist {
        key: String,
        title: String,
        thumb: Option<String>,
        /// True when Textamp filled in "Unknown Artist" for an empty title.
        is_placeholder: bool,
    },
    Album {
        key: String,
        title: String,
        artist: String,
        year: Option<u16>,
        thumb: Option<String>,
        /// True when Textamp filled in "Unknown Album (...)" for an empty title.
        is_placeholder: bool,
    },
    Track {
        key: String,
        title: String,
        artist_name: Option<String>,
        album_name: Option<String>,
        year: Option<u16>,
        duration_ms: u64,
        track_number: Option<u32>,
        /// A live download job; the visible row is not playable until committed.
        download_id: Option<String>,
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
    /// "All Tracks" entry. The `scope` discriminates which "all"
    /// the row represents — every artist in the library, every
    /// compilation, every compilation by a particular artist, or
    /// every track by a particular artist.
    AllTracks {
        scope: AllTracksScope,
        thumb: Option<String>,
    },
    /// "All Artists" entry - pinned at top of artist list, drills into all albums.
    AllArtists,
    /// Artist biography, photos and radio controls.
    ArtistInfo {
        artist_key: String,
        artist_name: String,
        thumb: Option<String>,
    },
    /// "Compilations" entry - pinned in artist root, drills into compilation albums.
    Compilations,
    /// "Compilation Tracks" entry - pinned in artist's album column, shows tracks by
    /// this artist that appear on compilation albums.
    CompilationTracks {
        artist_key: String,
        artist_name: String,
    },
}

impl BrowseItem {
    pub fn pending_download(&self) -> Option<&str> {
        match self {
            Self::Track { download_id, .. } => download_id.as_deref(),
            _ => None,
        }
    }
    pub fn key(&self) -> &str {
        match self {
            BrowseItem::Description { query } => query,
            BrowseItem::Artist { key, .. } => key,
            BrowseItem::Album { key, .. } => key,
            BrowseItem::Track { key, .. } => key,
            BrowseItem::Genre { key, .. } => key,
            BrowseItem::Playlist { key, .. } => key,
            BrowseItem::AllTracks { scope, .. } => scope.key(),
            BrowseItem::AllArtists => "__all_artists__",
            BrowseItem::ArtistInfo { artist_key, .. } => artist_key,
            BrowseItem::Compilations => "__compilations__",
            BrowseItem::CompilationTracks { artist_key, .. } => artist_key,
        }
    }

    pub fn title(&self) -> &str {
        match self {
            BrowseItem::Description { query } => query,
            BrowseItem::Artist { title, .. } => title,
            BrowseItem::Album { title, .. } => title,
            BrowseItem::Track { title, .. } => title,
            BrowseItem::Genre { title, .. } => title,
            BrowseItem::Playlist { title, .. } => title,
            BrowseItem::AllTracks { .. } => "All Tracks",
            BrowseItem::AllArtists => "All Artists",
            BrowseItem::ArtistInfo { .. } => "Artist Details",
            BrowseItem::Compilations => "Compilations",
            BrowseItem::CompilationTracks { .. } => "Compilations",
        }
    }

    /// Whether this item is a placeholder (Textamp filled in "Unknown ..." for empty metadata).
    pub fn is_placeholder_item(&self) -> bool {
        match self {
            BrowseItem::Artist { is_placeholder, .. } => *is_placeholder,
            BrowseItem::Album { is_placeholder, .. } => *is_placeholder,
            _ => false,
        }
    }

    /// Convert a list of Artists to BrowseItems.
    /// Placeholder items (empty title → "Unknown Artist") are sorted to the end.
    pub fn from_artists(artists: &[Artist]) -> Vec<BrowseItem> {
        let mut items: Vec<BrowseItem> = artists
            .iter()
            .map(|a| {
                let is_empty = a.title.is_empty();
                BrowseItem::Artist {
                    key: a.rating_key.clone(),
                    title: if is_empty {
                        "Unknown Artist".to_string()
                    } else {
                        a.title.clone()
                    },
                    thumb: a.thumb.clone(),
                    is_placeholder: is_empty,
                }
            })
            .collect();
        // Stable-partition: non-placeholders first, placeholders at end
        items.sort_by_key(|item| {
            matches!(
                item,
                BrowseItem::Artist {
                    is_placeholder: true,
                    ..
                }
            )
        });
        items
    }

    /// Convert a list of Albums to BrowseItems.
    /// Placeholder items (empty title → "Unknown Album (...)") are sorted to the end.
    /// If `album_display_artist` is provided, uses it to override the artist name
    /// when all tracks on a non-compilation album share a uniform track artist.
    pub fn from_albums(
        albums: &[Album],
        album_display_artist: &HashMap<String, String>,
    ) -> Vec<BrowseItem> {
        let mut items: Vec<BrowseItem> = albums
            .iter()
            .map(|a| {
                let is_empty = a.title.is_empty();
                let display_artist = album_display_artist
                    .get(&a.rating_key)
                    .map(|s| s.as_str())
                    .unwrap_or_else(|| a.artist_name());
                let (title, year) = if is_empty {
                    (format!("Unknown Album ({})", display_artist), None)
                } else {
                    (a.title.clone(), a.year)
                };
                BrowseItem::Album {
                    key: a.rating_key.clone(),
                    title,
                    artist: display_artist.to_string(),
                    year,
                    thumb: a.thumb.clone(),
                    is_placeholder: is_empty,
                }
            })
            .collect();
        // Stable-partition: non-placeholders first, placeholders at end
        items.sort_by_key(|item| {
            matches!(
                item,
                BrowseItem::Album {
                    is_placeholder: true,
                    ..
                }
            )
        });
        items
    }

    /// Convert a list of Tracks to BrowseItems.
    pub fn from_tracks(tracks: &[Track]) -> Vec<BrowseItem> {
        tracks
            .iter()
            .map(|t| {
                let title = if t.title.is_empty() {
                    t.file_name().unwrap_or("Unknown Track").to_string()
                } else {
                    t.title.clone()
                };
                BrowseItem::Track {
                    key: t.rating_key.clone(),
                    title,
                    artist_name: Some(t.track_artist().to_string()),
                    album_name: Some(t.album_name().to_string()),
                    year: t.year.or(t.parent_year),
                    duration_ms: t.duration_ms(),
                    track_number: t.index,
                    download_id: None,
                }
            })
            .collect()
    }

    /// Convert a list of Genres to BrowseItems.
    pub fn from_genres(genres: &[Genre]) -> Vec<BrowseItem> {
        genres
            .iter()
            .map(|g| BrowseItem::Genre {
                key: g.key.clone(),
                title: g.title.clone(),
            })
            .collect()
    }

    /// Convert a list of Playlists to BrowseItems.
    pub fn from_playlists(playlists: &[Playlist]) -> Vec<BrowseItem> {
        playlists
            .iter()
            .map(|p| BrowseItem::Playlist {
                key: p.rating_key.clone(),
                title: p.title.clone(),
                track_count: p.leaf_count,
            })
            .collect()
    }

    /// Build artist root items with compilation support:
    /// - Inserts "Compilations" pinned item after "All Artists" when compilations exist
    /// - Filters out artists that appear ONLY on compilation albums
    pub fn artist_root_items_with_compilations(
        artists: &[Artist],
        has_compilations: bool,
        compilation_artist_keys: &std::collections::HashSet<String>,
    ) -> Vec<BrowseItem> {
        let mut items = vec![BrowseItem::AllArtists];
        if has_compilations {
            items.push(BrowseItem::Compilations);
        }
        let artist_items: Vec<BrowseItem> = Self::from_artists(artists)
            .into_iter()
            .filter(|item| {
                if compilation_artist_keys.is_empty() {
                    return true;
                }
                match item {
                    BrowseItem::Artist { key, .. } => !compilation_artist_keys.contains(key),
                    _ => true,
                }
            })
            .collect();
        items.extend(artist_items);
        items
    }
}

/// Sort modes available to individual Miller columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColumnSortMode {
    #[default]
    Default, // Alphabetical / track number / playlist order
    ByArtist,
    ByAlbum,     // Track columns: sort by album name
    ByTitle,     // Sort by title
    ByDuration,  // Sort by duration
    ByDateAdded, // Provider-reported newest-first album order
    Shuffled,
}

impl ColumnSortMode {
    /// Human-readable suffix for column headers (empty for Default).
    pub fn header_suffix(&self, descending: bool) -> &'static str {
        match (self, descending) {
            (ColumnSortMode::Default, false) => "",
            (ColumnSortMode::Default, true) => "\u{2193}",
            (ColumnSortMode::ByArtist, false) => "by artist",
            (ColumnSortMode::ByArtist, true) => "by artist \u{2193}",
            (ColumnSortMode::ByAlbum, false) => "by album",
            (ColumnSortMode::ByAlbum, true) => "by album \u{2193}",
            (ColumnSortMode::ByTitle, false) => "by title",
            (ColumnSortMode::ByTitle, true) => "by title \u{2193}",
            (ColumnSortMode::ByDuration, false) => "by duration",
            (ColumnSortMode::ByDuration, true) => "by duration \u{2193}",
            (ColumnSortMode::ByDateAdded, false) => "by date added ↑",
            (ColumnSortMode::ByDateAdded, true) => "by date added ↓",
            (ColumnSortMode::Shuffled, _) => "shuffled",
        }
    }
}

/// Column type for determining available sort options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumnType {
    /// Artist root column: Default, Shuffled
    Artist,
    /// Saved playlist root.
    Playlist,
    /// Album column (artist's albums, genre albums): Default, ByArtist, Shuffled
    Album,
    /// Albums with an authoritative provider date-added order.
    RecentAlbums,
    /// Track column (single album tracks): Default, ByTitle, ByDuration, Shuffled
    Track,
    /// All-tracks / playlist track column: Default, ByArtist, ByAlbum, ByTitle, ByDuration, Shuffled
    AllTracks,
}

impl SortColumnType {
    /// Available sort modes for this column type.
    pub fn available_modes(&self) -> &'static [ColumnSortMode] {
        match self {
            SortColumnType::Artist => &[ColumnSortMode::Default, ColumnSortMode::Shuffled],
            SortColumnType::Playlist => &[
                ColumnSortMode::Default,
                ColumnSortMode::ByTitle,
                ColumnSortMode::Shuffled,
            ],
            SortColumnType::Album => &[
                ColumnSortMode::Default,
                ColumnSortMode::ByTitle,
                ColumnSortMode::ByArtist,
                ColumnSortMode::Shuffled,
            ],
            SortColumnType::RecentAlbums => &[
                ColumnSortMode::Default,
                ColumnSortMode::ByDateAdded,
                ColumnSortMode::ByTitle,
                ColumnSortMode::ByArtist,
                ColumnSortMode::Shuffled,
            ],
            SortColumnType::Track => &[
                ColumnSortMode::Default,
                ColumnSortMode::ByTitle,
                ColumnSortMode::ByDuration,
                ColumnSortMode::Shuffled,
            ],
            SortColumnType::AllTracks => &[
                ColumnSortMode::Default,
                ColumnSortMode::ByArtist,
                ColumnSortMode::ByAlbum,
                ColumnSortMode::ByTitle,
                ColumnSortMode::ByDuration,
                ColumnSortMode::Shuffled,
            ],
        }
    }

    /// Context-specific label for the "Default" sort mode.
    pub fn default_label(&self, is_playlist: bool) -> &'static str {
        match self {
            SortColumnType::Artist => "Artist",
            SortColumnType::Playlist => "Playlist",
            SortColumnType::Album => {
                if is_playlist {
                    "Title"
                } else {
                    "Year"
                }
            }
            SortColumnType::RecentAlbums => "Default",
            SortColumnType::Track => "Track #",
            SortColumnType::AllTracks => {
                if is_playlist {
                    "Playlist order"
                } else {
                    "Library order"
                }
            }
        }
    }
}

/// An option in the sort popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortPopupOption {
    SortMode(ColumnSortMode),
    Direction,
    Artwork,
    GroupByAlbum,
    CompilationOnlyArtists,
    LargeArtwork,
}

/// State for the sort popup (Ctrl+S).
#[derive(Debug, Clone)]
pub struct SortPopupState {
    /// Focused option index in the flattened list.
    pub selected_index: usize,
    /// Which column this applies to.
    pub column_idx: usize,
    /// Display title for the popup header.
    pub column_title: String,
    /// Column type determines available options.
    pub column_type: SortColumnType,
    /// Flattened list of all options.
    pub options: Vec<SortPopupOption>,
    /// Whether this is a playlist track column (affects GroupByAlbum availability).
    pub is_playlist: bool,
    pub is_downloads: bool,
    /// Context-specific label for the "Default" sort mode.
    pub default_label: &'static str,
}

impl SortPopupState {
    /// Build sort popup for the given column type.
    pub fn new(
        column_idx: usize,
        column_title: String,
        column_type: SortColumnType,
        current_mode: ColumnSortMode,
        _artwork_visible: bool,
        is_playlist: bool,
    ) -> Self {
        let mut popup = Self {
            selected_index: 0,
            column_idx,
            column_title,
            column_type,
            options: Vec::new(),
            is_playlist,
            is_downloads: false,
            default_label: column_type.default_label(is_playlist),
        };
        popup.rebuild_options(current_mode);
        popup.selected_index = popup
            .options
            .iter()
            .position(|option| *option == SortPopupOption::SortMode(current_mode))
            .unwrap_or(0);
        popup
    }

    /// Rebuild options (e.g. after mode change to add/remove Direction).
    pub fn rebuild_options(&mut self, current_mode: ColumnSortMode) {
        let old_selection = self.options.get(self.selected_index).copied();
        self.options.clear();

        self.default_label = self.column_type.default_label(self.is_playlist);
        let modes = self.column_type.available_modes();
        for mode in modes {
            self.options.push(SortPopupOption::SortMode(*mode));
        }

        if !matches!(current_mode, ColumnSortMode::Shuffled) {
            self.options.push(SortPopupOption::Direction);
        }

        if matches!(
            self.column_type,
            SortColumnType::Album
                | SortColumnType::RecentAlbums
                | SortColumnType::Track
                | SortColumnType::AllTracks
        ) {
            self.options.push(SortPopupOption::Artwork);
            if matches!(
                self.column_type,
                SortColumnType::Album | SortColumnType::RecentAlbums
            ) {
                self.options.push(SortPopupOption::LargeArtwork);
            }
        }

        // "Group by album" toggles a playlist's track list between
        // flat and album-grouped views. It's only meaningful on the
        // playlist's OWN track column — once the user has drilled
        // into one of those grouped albums, the child column shows
        // a single album's tracks (`SortColumnType::Track`) and
        // grouping again would be a no-op. Same logic for any other
        // single-album track column reached by drilling.
        if self.is_downloads
            || (self.is_playlist
                && matches!(
                    self.column_type,
                    SortColumnType::AllTracks | SortColumnType::Album
                ))
        {
            self.options.push(SortPopupOption::GroupByAlbum);
        }

        if self.column_type == SortColumnType::Artist {
            self.options.push(SortPopupOption::CompilationOnlyArtists);
        }

        // Try to preserve selection
        if let Some(old) = old_selection
            && let Some(pos) = self.options.iter().position(|o| *o == old)
        {
            self.selected_index = pos;
            return;
        }
        // Fallback: select current mode
        self.selected_index = modes.iter().position(|m| *m == current_mode).unwrap_or(0);
    }
}

/// Pinned "▶ Play …" row that sits above the real items of a tracks
/// column. The variant decides which `QueueAction` activates when the
/// user presses Enter / clicks on the row:
/// - `Album`    → `PlayAlbumNow`    (server fetches every album track)
/// - `Playlist` → `PlayPlaylistNow` (server re-fetches; catches the
///   tail of a lazy-paged playlist the user hasn't scrolled to)
/// - `AllTracks` → `PlayTracksNow(col.tracks.clone())` (no fetch; the
///   column already has every track — used for artist All Tracks,
///   compilations, library-wide All Tracks)
#[derive(Debug, Clone)]
pub enum PlayAllRow {
    ArtistRadio {
        artist_key: String,
        artist_name: String,
    },
    Album {
        rating_key: String,
        title: String,
    },
    Playlist {
        rating_key: String,
        title: String,
    },
    AllTracks {
        label: String,
    },
}

impl PlayAllRow {
    /// Label shown on the synthetic row in the column (sans the
    /// leading "▶ " glyph, which the renderer prepends).
    pub fn label(&self) -> &str {
        match self {
            PlayAllRow::ArtistRadio { .. } => "Artist Mix",
            PlayAllRow::Album { .. } => "Play album",
            PlayAllRow::Playlist { .. } => "Play playlist",
            PlayAllRow::AllTracks { label } => label,
        }
    }
}

/// Exclusive cursor position within a Miller column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnHeaderSelection {
    Play,
    AlbumDetails,
    Items,
}

/// A detail column owns its request lease, so closing/replacing it cancels work.
#[derive(Debug, Clone)]
pub struct ArtistInfoDetails {
    pub artist_key: String,
    pub artist_name: String,
    pub request_id: u64,
    pub document: crate::services::biography::Biography,
    pub image_index: usize,
    /// Owns the in-flight request; completion clears the lease.
    pub task: Option<crate::app::tasks::TaskLease>,
}
#[derive(Debug, Clone)]
pub enum BrowseDetails {
    Album {
        lines: Vec<String>,
        artwork_key: String,
    },
    Artist(ArtistInfoDetails),
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
    pub tracks: Vec<crate::library::models::Track>,
    /// Original items before shuffle/sort (None if in original order)
    original_items: Option<Vec<BrowseItem>>,
    /// Original tracks before shuffle/sort (None if in original order)
    original_tracks: Option<Vec<crate::library::models::Track>>,
    /// Sort mode for this column.
    pub sort_mode: ColumnSortMode,
    /// Sort direction: true = ascending (default)
    pub sort_ascending: bool,
    /// Newest-first album keys reported by the provider, retained across other sorts.
    pub date_added_order: Option<Vec<String>>,
    /// Album artwork visibility for this column.
    pub artwork_visible: bool,
    /// Playlist tracks grouped by album
    pub grouped_by_album: bool,
    /// Album group indices into tracks.
    pub album_groups: Option<Vec<Vec<usize>>>,
    /// When `Some`, the column is a tracks list with a synthetic
    /// "▶ Play …" row pinned above the real items. The TUI renders
    /// this as the column's first row; pressing Enter on it (or
    /// clicking it) plays the whole list. See `PlayAllRow` for the
    /// per-variant dispatch.
    pub play_all_row: Option<PlayAllRow>,
    /// Cursor position among the pinned actions and track items.
    pub header_selection: ColumnHeaderSelection,
    pub details: Option<BrowseDetails>,
    /// Multi-select set: indices into `items` (and `tracks`) that the
    /// user has shift- or cmd-clicked. Empty means single-selection
    /// mode (the regular `selected_index` cursor). Populated by the
    /// GUI's miller-row click handler; the TUI doesn't currently
    /// emit multi-clicks but reading the set is harmless.
    pub playlist_editing: bool,
    /// Checked server playlist positions while displaying album groups.
    pub grouped_playlist_marks: std::collections::BTreeSet<usize>,
    /// Original playlist positions in a drilled album, including duplicate songs.
    pub playlist_positions: Vec<usize>,
    pub selected_set: std::collections::BTreeSet<usize>,
    /// Anchor for shift+click range select — the row of the last
    /// non-shift click. `None` until the first click.
    pub selection_anchor: Option<usize>,

    /// When `Some`, this column is a lazy-paginated playlist tracks
    /// column. The GUI's scroll handler reads this to decide
    /// whether to fire `LoadMorePlaylistTracks`. `None` for every
    /// other column type.
    pub lazy: Option<LazyPlaylist>,
}

/// Pagination state for playlist-tracks columns whose source list is
/// too big to fetch in one round-trip (smart playlists like "Recently
/// Added" can resolve to tens of thousands of tracks).
#[derive(Debug, Clone)]
pub struct LazyPlaylist {
    /// server rating key — used to re-issue a fetch for the next page.
    pub key: String,
    /// Total tracks the server says exist for this column. Once the
    /// in-memory `tracks.len()` reaches this number the GUI stops
    /// firing `LoadMorePlaylistTracks`. `None` means "we don't yet
    /// know how many" (server didn't return `totalSize`).
    pub total: Option<u32>,
    /// True while a page fetch is in flight so the scroll handler
    /// doesn't fire duplicate requests.
    pub loading: bool,
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
            sort_mode: ColumnSortMode::Default,
            sort_ascending: true,
            date_added_order: None,
            artwork_visible: false,
            grouped_by_album: false,
            album_groups: None,
            play_all_row: None,
            header_selection: ColumnHeaderSelection::Items,
            details: None,
            playlist_editing: false,
            grouped_playlist_marks: Default::default(),
            playlist_positions: Vec::new(),
            selected_set: std::collections::BTreeSet::new(),
            selection_anchor: None,
            lazy: None,
        }
    }

    /// Create a column with full track objects stored for playback.
    pub fn new_with_tracks(
        title: impl Into<String>,
        items: Vec<BrowseItem>,
        tracks: Vec<crate::library::models::Track>,
    ) -> Self {
        Self {
            title: title.into(),
            items,
            selected_index: 0,
            tracks,
            original_items: None,
            original_tracks: None,
            sort_mode: ColumnSortMode::Default,
            sort_ascending: true,
            date_added_order: None,
            artwork_visible: false,
            grouped_by_album: false,
            album_groups: None,
            play_all_row: None,
            header_selection: ColumnHeaderSelection::Items,
            details: None,
            playlist_editing: false,
            grouped_playlist_marks: Default::default(),
            playlist_positions: Vec::new(),
            selected_set: std::collections::BTreeSet::new(),
            selection_anchor: None,
            lazy: None,
        }
    }

    /// Replace loaded content while retaining control state and selection identities.
    pub fn replace_content(&mut self, items: Vec<BrowseItem>, tracks: Vec<Track>) {
        if self.playlist_editing
            && self.grouped_by_album
            && !tracks
                .iter()
                .map(|t| &t.rating_key)
                .eq(self.tracks.iter().map(|t| &t.rating_key))
        {
            self.end_playlist_editing();
        }
        let selected = self.selected_item().map(|item| item.key().to_owned());
        let marked: std::collections::HashSet<_> = self
            .selected_set
            .iter()
            .filter_map(|index| self.items.get(*index))
            .map(|item| item.key().to_owned())
            .collect();
        let anchor = self
            .selection_anchor
            .and_then(|index| self.items.get(index))
            .map(|item| item.key().to_owned());
        let sort = self.sort_mode;
        let ascending = self.sort_ascending;
        let grouped = self.grouped_by_album;
        self.items = items;
        self.tracks = tracks;
        self.clear_originals();
        self.sort_mode = crate::app::state::ColumnSortMode::Default;
        self.sort_ascending = true;
        self.grouped_by_album = false;
        self.album_groups = None;
        self.apply_sort(sort);
        if self.sort_ascending != ascending {
            self.items.reverse();
            self.tracks.reverse();
            self.sort_ascending = ascending;
        }
        if grouped {
            self.group_by_album();
        }
        self.selected_index = selected
            .and_then(|key| self.items.iter().position(|item| item.key() == key))
            .unwrap_or(0);
        self.selected_set = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| marked.contains(item.key()))
            .map(|(index, _)| index)
            .collect();
        self.selection_anchor =
            anchor.and_then(|key| self.items.iter().position(|item| item.key() == key));
    }

    pub fn detail_lines(&self) -> Option<std::borrow::Cow<'_, [String]>> {
        match self.details.as_ref()? {
            BrowseDetails::Album { lines, .. } => Some(std::borrow::Cow::Borrowed(lines)),
            BrowseDetails::Artist(info) => {
                let mut lines = vec![super::details::artist_text(info).into()];
                if !info.document.related_artists.is_empty() {
                    lines.push(String::new());
                    lines.push(super::details::Kind::Artist.related_label().into());
                    lines.extend(
                        info.document
                            .related_artists
                            .iter()
                            .map(|artist| artist.title.clone()),
                    );
                }
                Some(std::borrow::Cow::Owned(lines))
            }
        }
    }
    pub fn detail_art_key(&self) -> Option<&str> {
        match self.details.as_ref()? {
            BrowseDetails::Album { artwork_key, .. } => Some(artwork_key),
            BrowseDetails::Artist(info) => Some(&info.artist_key),
        }
    }
    pub fn selected_item(&self) -> Option<&BrowseItem> {
        self.items.get(self.selected_index)
    }

    pub fn has_album_details(&self) -> bool {
        self.details.is_none() && matches!(self.play_all_row, Some(PlayAllRow::Album { .. }))
    }

    /// Whether this column is currently shuffled.
    pub fn is_shuffled(&self) -> bool {
        self.sort_mode == ColumnSortMode::Shuffled
    }

    /// Whether original items are saved (for restore).
    pub fn has_originals(&self) -> bool {
        self.original_items.is_some()
    }

    /// Shuffle items (and tracks in parallel). Saves originals for restore.
    /// Pinned items (AllArtists, AllTracks) at index 0 are excluded from shuffle.
    /// Placeholder items (is_placeholder: true) are kept at the end.
    pub fn shuffle(&mut self) {
        use rand::seq::SliceRandom;
        self.sort_mode = ColumnSortMode::Shuffled;
        // Save originals (fresh copy each time for re-shuffle)
        self.original_items = Some(self.items.clone());
        self.original_tracks = if self.tracks.is_empty() {
            None
        } else {
            Some(self.tracks.clone())
        };

        // Count pinned items at start (AllArtists, AllTracks, Compilations, CompilationTracks, ArtistInfo)
        let start = self.pinned_count();

        // Find placeholder items pinned at end
        let placeholder_start = self
            .items
            .iter()
            .rposition(|item| !item.is_placeholder_item())
            .map(|i| i + 1)
            .unwrap_or(self.items.len());
        let end = placeholder_start;

        // Build index permutation for shuffleable items (exclude pinned start + placeholder tail)
        let mut indices: Vec<usize> = (start..end).collect();
        let mut rng = rand::rng();
        indices.shuffle(&mut rng);

        let orig_items = self.original_items.as_ref().unwrap();
        let mut new_items: Vec<BrowseItem> = Vec::with_capacity(self.items.len());
        // Copy pinned items at start (preserve order)
        new_items.extend(orig_items[..start].iter().cloned());
        new_items.extend(indices.iter().map(|&i| orig_items[i].clone()));
        // Append placeholder tail (unchanged order)
        new_items.extend(orig_items[end..].iter().cloned());
        self.items = new_items;

        if let Some(ref orig_tracks) = self.original_tracks {
            let mut new_tracks = Vec::with_capacity(orig_tracks.len());
            // Copy pinned track slots at start
            new_tracks.extend(orig_tracks[..start].iter().cloned());
            new_tracks.extend(indices.iter().filter_map(|&i| orig_tracks.get(i).cloned()));
            // Tracks don't have placeholders, but keep consistent length
            for i in end..orig_tracks.len() {
                if let Some(t) = orig_tracks.get(i) {
                    new_tracks.push(t.clone());
                }
            }
            self.tracks = new_tracks;
        }

        self.selected_index = 0;
    }

    /// Clear stored originals so current order becomes the new baseline.
    /// Preserves sort_mode so the header suffix and cycle position remain correct.
    pub fn clear_originals(&mut self) {
        self.original_items = None;
        self.original_tracks = None;
    }

    /// Restore original order (clears sort mode to Default).
    pub fn unshuffle(&mut self) {
        if let Some(items) = self.original_items.take() {
            self.items = items;
        }
        if let Some(tracks) = self.original_tracks.take() {
            self.tracks = tracks;
        }
        self.sort_mode = ColumnSortMode::Default;
        self.sort_ascending = true;
        self.selected_index = 0;
    }

    /// Sort album items by artist name (case-insensitive), then by year.
    /// Saves originals for restore. Pinned items at index 0 are excluded.
    pub fn sort_by_artist(&mut self) {
        if self.sort_mode == ColumnSortMode::ByArtist {
            return;
        }
        // Save originals if not already saved
        if self.original_items.is_none() {
            self.original_items = Some(self.items.clone());
            self.original_tracks = if self.tracks.is_empty() {
                None
            } else {
                Some(self.tracks.clone())
            };
        }
        // Count how many pinned items are at the start
        let start = self.pinned_count();
        self.items[start..].sort_by_cached_key(|item| match item {
            BrowseItem::Album { artist, year, .. } => (artist.to_lowercase(), *year),
            BrowseItem::Track { artist_name, .. } => (
                artist_name.as_deref().unwrap_or_default().to_lowercase(),
                None,
            ),
            _ => (String::new(), None),
        });
        if start < self.tracks.len() {
            self.tracks[start..].sort_by_cached_key(|track| track.track_artist().to_lowercase());
        }
        self.sort_mode = ColumnSortMode::ByArtist;
        self.selected_index = 0;
    }

    /// Sort track items by title (case-insensitive).
    /// Saves originals for restore. Pinned items at start are excluded.
    pub fn sort_by_title(&mut self) {
        if self.sort_mode == ColumnSortMode::ByTitle {
            return;
        }
        self.save_originals();
        let start = self.pinned_count();
        // Sort items
        self.items[start..].sort_by_cached_key(|item| item.title().to_lowercase());
        // Sort tracks in parallel
        if start < self.tracks.len() {
            self.tracks[start..].sort_by_cached_key(|t| t.title.to_lowercase());
        }
        self.sort_mode = ColumnSortMode::ByTitle;
        self.selected_index = 0;
    }

    /// Sort track items by duration (ascending).
    /// Saves originals for restore. Pinned items at start are excluded.
    pub fn sort_by_duration(&mut self) {
        if self.sort_mode == ColumnSortMode::ByDuration {
            return;
        }
        self.save_originals();
        let start = self.pinned_count();
        // Sort items
        self.items[start..].sort_by(|a, b| {
            let a_dur = if let BrowseItem::Track { duration_ms, .. } = a {
                *duration_ms
            } else {
                0
            };
            let b_dur = if let BrowseItem::Track { duration_ms, .. } = b {
                *duration_ms
            } else {
                0
            };
            a_dur.cmp(&b_dur)
        });
        // Sort tracks in parallel
        if start < self.tracks.len() {
            self.tracks[start..].sort_by_key(|a| a.duration_ms());
        }
        self.sort_mode = ColumnSortMode::ByDuration;
        self.selected_index = 0;
    }

    /// Sort track items by album name (case-insensitive), then track number.
    /// Saves originals for restore.
    pub fn sort_by_album(&mut self) {
        if self.sort_mode == ColumnSortMode::ByAlbum {
            return;
        }
        self.save_originals();
        let start = self.pinned_count();
        // Sort items
        self.items[start..].sort_by_cached_key(|item| match item {
            BrowseItem::Track {
                album_name,
                track_number,
                ..
            } => (
                album_name.as_deref().unwrap_or("").to_lowercase(),
                *track_number,
            ),
            _ => (String::new(), None),
        });
        // Sort tracks in parallel
        if start < self.tracks.len() {
            self.tracks[start..].sort_by_cached_key(|t| (t.album_name().to_lowercase(), t.index));
        }
        self.sort_mode = ColumnSortMode::ByAlbum;
        self.selected_index = 0;
    }

    /// Pending downloads are visible rows, but are not playable music yet.
    pub fn ready_tracks_from(&self, start: usize) -> Vec<Track> {
        self.tracks
            .iter()
            .enumerate()
            .skip(start)
            .filter(|(index, _)| {
                let rows = if self.grouped_by_album {
                    self.original_items.as_deref().unwrap_or(&self.items)
                } else {
                    &self.items
                };
                rows.get(*index)
                    .is_none_or(|item| item.pending_download().is_none())
            })
            .map(|(_, track)| track.clone())
            .collect()
    }
    pub fn ready_tracks(&self) -> Vec<Track> {
        self.ready_tracks_from(0)
    }

    pub fn end_playlist_editing(&mut self) {
        if self.playlist_editing {
            self.playlist_editing = false;
            self.grouped_playlist_marks.clear();
            self.selected_set.clear();
            self.selection_anchor = None;
        }
    }

    /// Apply the selected column sort mode.
    pub fn apply_sort(&mut self, mode: ColumnSortMode) {
        if self.sort_mode != mode && mode != ColumnSortMode::ByDateAdded {
            self.sort_ascending = true;
        }
        match mode {
            ColumnSortMode::Default => self.unshuffle(),
            ColumnSortMode::Shuffled => self.shuffle(),
            ColumnSortMode::ByArtist => self.sort_by_artist(),
            ColumnSortMode::ByTitle => self.sort_by_title(),
            ColumnSortMode::ByDuration => self.sort_by_duration(),
            ColumnSortMode::ByAlbum => self.sort_by_album(),
            ColumnSortMode::ByDateAdded => self.sort_by_date_added(),
        }
    }

    /// Restore the provider's newest-first order, without inferring dates from titles.
    pub fn sort_by_date_added(&mut self) {
        if self.sort_mode == ColumnSortMode::ByDateAdded {
            return;
        }
        let Some(order) = &self.date_added_order else {
            return;
        };
        let positions: std::collections::HashMap<_, _> = order
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, key)| (key, i))
            .collect();
        self.save_originals();
        let start = self.pinned_count();
        self.items[start..]
            .sort_by_key(|item| positions.get(item.key()).copied().unwrap_or(usize::MAX));
        self.sort_mode = ColumnSortMode::ByDateAdded;
        self.sort_ascending = false;
        self.selected_index = start;
    }

    /// Save originals if not already saved (used by sort methods).
    fn save_originals(&mut self) {
        if self.original_items.is_none() {
            self.original_items = Some(self.items.clone());
            self.original_tracks = if self.tracks.is_empty() {
                None
            } else {
                Some(self.tracks.clone())
            };
        }
    }

    /// Count pinned items at the start of the column.
    pub fn pinned_count(&self) -> usize {
        self.items
            .iter()
            .take_while(|item| {
                matches!(
                    item,
                    BrowseItem::AllArtists
                        | BrowseItem::AllTracks { .. }
                        | BrowseItem::ArtistInfo { .. }
                        | BrowseItem::Compilations
                        | BrowseItem::CompilationTracks { .. }
                )
            })
            .count()
    }

    /// Group tracks by album for playlist columns.
    ///
    /// Saves originals, replaces items with BrowseItem::Album entries,
    /// and stores track index groups in `album_groups`.
    pub fn group_by_album(&mut self) {
        use std::collections::HashMap;

        if self.tracks.is_empty() {
            return;
        }

        self.save_originals();
        self.grouped_by_album = true;

        // Group track indices by album key, preserving first-seen order
        let mut key_to_group: HashMap<String, usize> = HashMap::new();
        let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
        for (i, track) in self.tracks.iter().enumerate() {
            let album_key = super::downloads::album_key(track).into_owned();
            if let Some(&group_idx) = key_to_group.get(&album_key) {
                groups[group_idx].1.push(i);
            } else {
                let group_idx = groups.len();
                key_to_group.insert(album_key.clone(), group_idx);
                groups.push((album_key, vec![i]));
            }
        }

        // Build album items from first track of each group
        let mut album_items = Vec::with_capacity(groups.len());
        let mut album_groups = Vec::with_capacity(groups.len());
        for (album_key, indices) in groups {
            let first = &self.tracks[indices[0]];
            album_items.push(BrowseItem::Album {
                key: album_key,
                title: first.album_name().to_string(),
                thumb: first.parent_thumb.clone().or_else(|| first.thumb.clone()),
                artist: first.artist_name().to_string(),
                year: first.year.or(first.parent_year),
                is_placeholder: false,
            });
            album_groups.push(indices);
        }

        self.items = album_items;
        self.album_groups = Some(album_groups);
        self.selected_index = 0;
    }

    /// Restore original track view from album grouping.
    pub fn ungroup_by_album(&mut self) {
        self.grouped_by_album = false;
        self.album_groups = None;

        if let Some(items) = self.original_items.take() {
            self.items = items;
        }
        if let Some(tracks) = self.original_tracks.take() {
            self.tracks = tracks;
        }
        self.selected_index = 0;
    }
}

impl MillerColumn for BrowseColumn {
    fn item_count(&self) -> usize {
        self.items.len()
    }
    fn selected_index(&self) -> usize {
        self.selected_index
    }
    fn set_selected_index(&mut self, idx: usize) {
        self.selected_index = idx;
    }
}

/// Navigation state for Miller column browsing.
pub type BrowseNavigationState = MillerState<BrowseColumn>;

/// Type-specific methods for browse navigation.
impl MillerState<BrowseColumn> {
    /// Initialize with a root column.
    pub fn with_root(title: impl Into<String>, items: Vec<BrowseItem>) -> Self {
        Self {
            columns: vec![BrowseColumn::new(title, items)],
            focused_column: 0,
            loading: false,
        }
    }

    /// Get the selected item in the focused column.
    pub fn selected_item(&self) -> Option<&BrowseItem> {
        self.focused().and_then(|c| c.selected_item())
    }

    /// Reset to a single root column.
    pub fn reset(&mut self, title: impl Into<String>, items: Vec<BrowseItem>) {
        self.columns = vec![BrowseColumn::new(title, items)];
        self.focused_column = 0;
        self.loading = false;
    }

    /// Push or replace a child column based on the auto_drill flag.
    /// When auto_drill is true, replaces the child column at focused_column+1
    /// without changing focus. When false, behaves like push_column.
    pub fn drill_column(&mut self, column: BrowseColumn, auto_drill: bool) {
        if auto_drill {
            self.replace_child_column(column);
        } else {
            self.push_column(column);
        }
    }

    /// Update root column items without resetting navigation.
    /// Preserves drill-down columns, selections, and focused column.
    pub fn update_root_items(&mut self, title: impl Into<String>, items: Vec<BrowseItem>) {
        if let Some(col) = self.columns.get_mut(0) {
            col.title = title.into();
            let old_idx = col.selected_index;
            let selected_key = col.selected_item().map(|item| item.key().to_owned());
            col.items = items;
            col.selected_index = selected_key
                .and_then(|key| col.items.iter().position(|item| item.key() == key))
                .unwrap_or_else(|| old_idx.min(col.items.len().saturating_sub(1)));
        } else {
            // No columns yet - initialize
            self.columns = vec![BrowseColumn::new(title, items)];
            self.focused_column = 0;
        }
        self.loading = false;
    }
}

/// Step in the multi-artist radio picker flow.
/// State for the multi-artist radio picker popup.
#[derive(Debug, Clone)]
pub struct ArtistRadioPickerState {
    pub query: String,
    pub filtered_artists: Vec<Artist>,
    pub selected_artists: Vec<Artist>,
    pub focus: SearchFocus,
    pub item_index: usize,
    pub scroll_pin: Option<usize>,
}

/// Exclusive full-screen viewer mode; the underlying library view is retained.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FullscreenView {
    #[default]
    Closed,
    Visualizer,
    Artwork,
}
impl FullscreenView {
    pub fn is_open(self) -> bool {
        self != Self::Closed
    }
    pub fn is_artwork(self) -> bool {
        self == Self::Artwork
    }
}

/// Desktop presentation modes, independent of the terminal's full-screen viewer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DesktopFullscreenView {
    #[default]
    Closed,
    Artwork,
    ArtworkTransport,
    Visualizer,
    VisualizerTransport,
}
impl DesktopFullscreenView {
    pub const MODES: [Self; 4] = [
        Self::Artwork,
        Self::ArtworkTransport,
        Self::Visualizer,
        Self::VisualizerTransport,
    ];
    pub fn is_open(self) -> bool {
        self != Self::Closed
    }
    pub fn is_artwork(self) -> bool {
        matches!(self, Self::Artwork | Self::ArtworkTransport)
    }
    pub fn has_transport(self) -> bool {
        matches!(self, Self::ArtworkTransport | Self::VisualizerTransport)
    }
    pub fn cycle(self, forward: bool) -> Self {
        let index = Self::MODES
            .iter()
            .position(|mode| *mode == self)
            .unwrap_or(0);
        Self::MODES[(index + if forward { 1 } else { 3 }) % Self::MODES.len()]
    }
}

/// State for the artist bio popup (F4).
#[derive(Debug, Clone)]
pub struct ArtistBioPopup {
    pub artist_key: String,
    /// Artist name displayed in the title.
    pub artist_name: String,
    pub document: crate::services::biography::Biography,
    /// Scroll offset for long bios (clamped in render).
    pub scroll: u16,
    pub google_focused: bool,
    pub image_index: usize,
    /// Owns the in-flight request; completion clears the lease.
    pub task: Option<crate::app::tasks::TaskLease>,
}

/// Snapshot of queue state for undo.
#[derive(Debug, Clone)]
pub struct QueueSnapshot {
    pub recorded_at: std::time::Instant,
    pub contents: QueueContents,
    pub description: String,
}

/// Undo owns either a queue or radio session, never duplicate copies of both.
#[derive(Debug, Clone)]
pub enum QueueContents {
    Queue {
        tracks: Vec<Track>,
        index: Option<usize>,
    },
    Radio(Box<RadioPlaybackState>),
}

/// Artwork rendering mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArtworkMode {
    /// Auto-detect best protocol (Kitty/iTerm2/Sixel/Halfblocks)
    #[default]
    Auto,
    /// Force halfblocks (ANSI) rendering
    Halfblocks,
    /// Braille character rendering (2x4 dot resolution per cell)
    Braille,
}

impl ArtworkMode {
    pub fn all() -> &'static [ArtworkMode] {
        &[
            ArtworkMode::Auto,
            ArtworkMode::Halfblocks,
            ArtworkMode::Braille,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            ArtworkMode::Auto => "auto",
            ArtworkMode::Halfblocks => "halfblocks",
            ArtworkMode::Braille => "braille",
        }
    }

    pub fn from_config(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "halfblocks" | "ansi" => ArtworkMode::Halfblocks,
            "braille" => ArtworkMode::Braille,
            _ => ArtworkMode::Auto,
        }
    }
}

cyclic_enum!(ArtworkMode, Auto, Halfblocks, Braille);

/// Cache management state.
#[derive(Debug)]
pub struct CacheManagement {
    pub failures: HashMap<RefreshCategory, RefreshFailure>,

    /// Per-category timestamps (Unix epoch secs) for when each category was last refreshed.
    pub category_timestamps: HashMap<RefreshCategory, u64>,
    pub dirty: bool,
    pub last_input_time: std::time::Instant,

    /// Cheap timestamp checks, not a network polling interval.
    pub next_refresh_check: Option<std::time::Instant>,

    pub background_refresh: std::collections::HashSet<RefreshCategory>,
}

impl Default for CacheManagement {
    fn default() -> Self {
        Self {
            failures: HashMap::new(),

            category_timestamps: HashMap::new(),
            dirty: false,
            last_input_time: std::time::Instant::now(),

            next_refresh_check: None,

            background_refresh: std::collections::HashSet::new(),
        }
    }
}

#[derive(Debug)]
pub struct RefreshFailure {
    pub attempts: usize,
    /// None means automatic retries are exhausted (or inappropriate).
    pub retry_at: Option<std::time::Instant>,
}

/// Notification/toast state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Notifications {
    pub status_target: Option<NotificationTarget>,
    pub toast_message: Option<String>,
    pub toast_show_time: Option<std::time::Instant>,
    pub last_error: Option<String>,
    pub error_target: Option<NotificationTarget>,
    pub status_message: Option<String>,
    pub status_show_time: Option<std::time::Instant>,
}

/// Scroll pin state for viewport preservation on click.
#[derive(Debug, Clone, Default)]
pub struct ScrollPins {
    pub settings_textamp: Option<usize>,
    pub category: Option<usize>,
    pub browse: Option<(usize, usize)>,
    pub browse_click_time: Option<std::time::Instant>,
    /// Last clicked item in browse Miller columns: (col_idx, item_idx) for double-click detection.
    pub browse_last_click: Option<(usize, usize)>,
    pub search: Option<usize>,
    pub queue: Option<usize>,
    pub queue_click_time: Option<std::time::Instant>,
    pub queue_drag: Option<super::queue_pointer::Drag>,
    pub similar: Option<usize>,
    pub similar_click_time: Option<std::time::Instant>,
    pub related: Option<usize>,
    pub related_click_time: Option<std::time::Instant>,
    pub search_click_time: Option<std::time::Instant>,
    pub art_cooldown: Option<std::time::Instant>,
    pub scroll_cooldown: Option<std::time::Instant>,
    pub scrollbar_drag: Option<ScrollbarDrag>,
}

/// The active modal, with an explicit suspended parent for confirmations.
#[derive(Debug, Clone, Default)]
pub enum Modal {
    #[default]
    Closed,
    Sort(Box<SortPopupState>),
    AdventureLauncher(Box<AdventureLauncherState>),
    ArtistRadioPicker(Box<ArtistRadioPickerState>),
    ArtistBio(Box<ArtistBioPopup>),
    Text(Box<TextPopup>),
    Lyrics(Box<super::lyrics::Panel>),
    Input(Box<InputDialog>),
    Library(Box<crate::app::sources::dialogs::Dialog>),
    LibraryPicker,
    Search,
    Confirmation {
        dialog: Box<ConfirmDialog>,
        parent: Box<Modal>,
    },
}

/// Popup transitions preserve a parent only while a confirmation is open.
#[derive(Debug, Clone, Default)]
pub struct Popups {
    modal: Modal,
    pub library_picker_index: usize,
}

macro_rules! modal_accessors {
    ($get:ident, $get_mut:ident, $set:ident, $take:ident, $variant:ident, $ty:ty) => {
        pub fn $get(&self) -> Option<&$ty> {
            match self.base() {
                Modal::$variant(value) => Some(value),
                _ => None,
            }
        }
        pub fn $get_mut(&mut self) -> Option<&mut $ty> {
            match self.base_mut() {
                Modal::$variant(value) => Some(value),
                _ => None,
            }
        }
        pub fn $set(&mut self, value: Option<$ty>) {
            if let Some(value) = value {
                self.modal = Modal::$variant(Box::new(value));
            } else {
                self.$take();
            }
        }
        pub fn $take(&mut self) -> Option<$ty> {
            if !matches!(self.base(), Modal::$variant(_)) {
                return None;
            }
            match std::mem::take(self.base_mut()) {
                Modal::$variant(value) => Some(*value),
                _ => unreachable!(),
            }
        }
    };
}

impl Popups {
    pub fn modal(&self) -> &Modal {
        &self.modal
    }
    fn base(&self) -> &Modal {
        match &self.modal {
            Modal::Confirmation { parent, .. } => parent,
            modal => modal,
        }
    }
    fn base_mut(&mut self) -> &mut Modal {
        match &mut self.modal {
            Modal::Confirmation { parent, .. } => parent,
            modal => modal,
        }
    }
    pub fn close_all(&mut self) {
        self.modal = Modal::Closed;
    }
    modal_accessors!(sort, sort_mut, set_sort, take_sort, Sort, SortPopupState);
    modal_accessors!(
        adventure_launcher,
        adventure_launcher_mut,
        set_adventure_launcher,
        take_adventure_launcher,
        AdventureLauncher,
        AdventureLauncherState
    );
    modal_accessors!(
        artist_radio_picker,
        artist_radio_picker_mut,
        set_artist_radio_picker,
        take_artist_radio_picker,
        ArtistRadioPicker,
        ArtistRadioPickerState
    );
    modal_accessors!(
        artist_bio,
        artist_bio_mut,
        set_artist_bio,
        take_artist_bio,
        ArtistBio,
        ArtistBioPopup
    );
    modal_accessors!(text, text_mut, set_text, take_text, Text, TextPopup);
    modal_accessors!(
        lyrics,
        lyrics_mut,
        set_lyrics,
        take_lyrics,
        Lyrics,
        super::lyrics::Panel
    );
    modal_accessors!(
        input_dialog,
        input_dialog_mut,
        set_input_dialog,
        take_input_dialog,
        Input,
        InputDialog
    );
    modal_accessors!(
        library_dialog,
        library_dialog_mut,
        set_library_dialog,
        take_library_dialog,
        Library,
        crate::app::sources::dialogs::Dialog
    );
    pub fn library_picker_active(&self) -> bool {
        matches!(self.base(), Modal::LibraryPicker)
    }
    pub fn set_library_picker_active(&mut self, active: bool) {
        if active {
            self.modal = Modal::LibraryPicker;
        } else if self.library_picker_active() {
            *self.base_mut() = Modal::Closed;
        }
    }
    pub fn search_active(&self) -> bool {
        matches!(self.base(), Modal::Search)
    }
    pub fn set_search_active(&mut self, active: bool) {
        if active {
            self.modal = Modal::Search;
        } else if self.search_active() {
            *self.base_mut() = Modal::Closed;
        }
    }
    pub fn confirm_dialog(&self) -> Option<&ConfirmDialog> {
        match &self.modal {
            Modal::Confirmation { dialog, .. } => Some(dialog),
            _ => None,
        }
    }
    pub fn confirm_dialog_mut(&mut self) -> Option<&mut ConfirmDialog> {
        match &mut self.modal {
            Modal::Confirmation { dialog, .. } => Some(dialog),
            _ => None,
        }
    }
    pub fn take_confirm_dialog(&mut self) -> Option<ConfirmDialog> {
        if !matches!(self.modal, Modal::Confirmation { .. }) {
            return None;
        }
        match std::mem::take(&mut self.modal) {
            Modal::Confirmation { dialog, parent } => {
                self.modal = *parent;
                Some(*dialog)
            }
            _ => unreachable!(),
        }
    }
    pub fn set_confirm_dialog(&mut self, dialog: Option<ConfirmDialog>) {
        self.take_confirm_dialog();
        if let Some(dialog) = dialog {
            self.modal = Modal::Confirmation {
                dialog: Box::new(dialog),
                parent: Box::new(match std::mem::take(&mut self.modal) {
                    parent @ Modal::Library(_) => parent,
                    _ => Modal::Closed,
                }),
            };
        }
    }
}

/// A fetch completion is accepted only for this pending thumbnail and generation.
#[derive(Debug, Default)]
pub enum ArtworkFetch {
    #[default]
    Idle,
    Loading {
        thumb: String,
    },
}
impl ArtworkFetch {
    pub fn pending_thumb(&self) -> Option<&str> {
        match self {
            Self::Loading { thumb } => Some(thumb),
            Self::Idle => None,
        }
    }
    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading { .. })
    }
}

#[derive(Debug, Default)]
pub enum ArtworkNavigation {
    #[default]
    Settled,
    Moving {
        last_motion: std::time::Instant,
    },
}

/// Artwork state.
#[derive(Debug, Default)]
pub struct ArtworkState {
    pub current_thumb: Option<String>,
    pub current_data: Option<Vec<u8>>,
    pub fetch: ArtworkFetch,
    pub grid_cache: HashMap<String, Vec<u8>>,
    grid_cache_order: std::collections::VecDeque<String>,
    grid_cache_bytes: usize,
    pub grid_generation: u64,
    pub grid_revision: u64,
    pub grid_pending: std::collections::HashSet<String>,
    pub grid_requested: std::collections::VecDeque<(String, String)>,
    pub cache_stats: Option<(usize, u64)>,
    pub graphical_mode: ArtworkMode,
    pub column_visibility: HashMap<String, bool>,
    pub mode: ArtworkMode,
    /// Rapid navigation defers fetching until the most recent motion settles.
    pub navigation: ArtworkNavigation,
}

impl ArtworkState {
    const MAX_GRID_CACHE_ENTRIES: usize = 256;
    const MAX_GRID_CACHE_BYTES: usize = 128 * 1024 * 1024;

    pub fn insert_grid_art(&mut self, key: String, data: Vec<u8>) {
        if data.len() > Self::MAX_GRID_CACHE_BYTES {
            tracing::debug!(
                "Skipping oversized in-memory artwork item: {} bytes",
                data.len()
            );
            return;
        }
        if let Some(previous) = self.grid_cache.remove(&key) {
            self.grid_cache_bytes = self.grid_cache_bytes.saturating_sub(previous.len());
            if let Some(position) = self.grid_cache_order.iter().position(|item| item == &key) {
                self.grid_cache_order.remove(position);
            }
        }
        self.grid_cache_bytes = self.grid_cache_bytes.saturating_add(data.len());
        self.grid_revision = self.grid_revision.wrapping_add(1);
        self.grid_cache.insert(key.clone(), data);
        self.grid_cache_order.push_back(key);

        while self.grid_cache.len() > Self::MAX_GRID_CACHE_ENTRIES
            || self.grid_cache_bytes > Self::MAX_GRID_CACHE_BYTES
        {
            let Some(oldest) = self.grid_cache_order.pop_front() else {
                self.clear_grid_art();
                break;
            };
            if let Some(removed) = self.grid_cache.remove(&oldest) {
                self.grid_cache_bytes = self.grid_cache_bytes.saturating_sub(removed.len());
            }
        }
    }

    /// Retain the latest visible requests while another bounded batch is running.
    pub fn request_grid_art(&mut self, images: impl IntoIterator<Item = (String, String)>) {
        for (key, thumb) in images {
            if self.grid_cache.contains_key(&key)
                || self.grid_pending.contains(&key)
                || self.grid_requested.iter().any(|(queued, _)| queued == &key)
            {
                continue;
            }
            if self.grid_requested.len() == 100 {
                self.grid_requested.pop_front();
            }
            self.grid_requested.push_back((key, thumb));
        }
    }

    pub fn clear_grid_art(&mut self) {
        self.grid_requested.clear();
        self.grid_generation = self.grid_generation.wrapping_add(1);
        self.grid_revision = self.grid_revision.wrapping_add(1);
        self.grid_cache.clear();
        self.grid_cache_order.clear();
        self.grid_cache_bytes = 0;
    }
}

/// Idle threshold after which the lazy-art gate reopens. Tuned by the
/// user — short enough that the wait feels intentional, long enough to
/// absorb a held-down arrow key without flapping.
pub const ART_LOAD_PAUSE_MS: u64 = 1000;

/// Similarity requests retain either an outstanding seed or a failure.
#[derive(Debug, Clone, Default)]
pub enum SimilarRequest {
    #[default]
    Ready,
    Loading {
        key: String,
    },
    Failed {
        message: String,
    },
}
impl SimilarRequest {
    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading { .. })
    }
    pub fn loading_key(&self) -> Option<&str> {
        match self {
            Self::Loading { key } => Some(key),
            _ => None,
        }
    }
    pub fn error(&self) -> Option<&String> {
        match self {
            Self::Failed { message } => Some(message),
            _ => None,
        }
    }
}

/// Similar content view state (server sonic similarity).
#[derive(Debug, Clone, Default)]
pub struct SimilarViewState {
    /// Last requested seed, retained after completion for retry.
    pub source_key: Option<String>,
    pub request: SimilarRequest,
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
    pub artists: Vec<Artist>,
    pub mode: SimilarMode,
    pub source_title: String,
    /// Album key for Tab cycling in Similar view (tracks → albums).
    pub tab_album_key: Option<String>,
    /// Album title for Tab cycling footer display.
    pub tab_album_title: Option<String>,
    /// Track key for Tab cycling in Similar view (albums → tracks).
    pub tab_track_key: Option<String>,
    /// Track title for Tab cycling footer display.
    pub tab_track_title: Option<String>,
}

/// Source of a related artist entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelatedSource {
    Navidrome,

    /// From server "Similar" metadata tags on the artist.
    SimilarTag,
    /// From textamp artist_aliases.
    Alias,
}

/// A group of albums under a related artist.
#[derive(Debug, Clone)]
pub struct RelatedArtistGroup {
    pub artist: Artist,
    pub albums: Vec<Album>,
    pub source: RelatedSource,
}

/// Related artists view state (Ctrl+R).
#[derive(Debug, Clone, Default)]
pub struct RelatedViewState {
    pub groups: Vec<RelatedArtistGroup>,
    pub loading: bool,
    pub source_title: String,
    pub source_key: String,
}

pub type CompilationState = crate::services::compilations::CompilationIndex;

/// Library data — artists, albums, playlists, genres, and derived data.
///
/// Contains all cached data from the server library API, plus derived data
/// like compilation detection and artist aliases.
#[derive(Debug, Default)]
pub struct LibraryData {
    pub catalog_loading: crate::app::catalog_loading::State,
    pub(crate) catalog_preview_tracks: Option<Vec<crate::library::models::Track>>,
    // Core collections
    pub artists: std::sync::Arc<Vec<Artist>>,
    pub artists_total: u32,
    pub artists_loading: bool,
    pub albums: std::sync::Arc<Vec<Album>>,
    pub albums_total: u32,
    pub albums_loading: bool,
    pub playlists: std::sync::Arc<Vec<Playlist>>,
    pub playlists_loading: bool,

    // All tracks cache (for compilation detection + track-level artist derivation)
    pub all_tracks: std::sync::Arc<Vec<Track>>,
    pub track_lookup: crate::services::catalog_lookup::TrackLookup,
    pub search_cache: crate::services::catalog_search::Cache,
    pub visited_album_tracks: HashMap<String, Vec<Track>>,

    // Track-level artist list (derived from all_tracks original_title)
    pub track_artists: Vec<Artist>,

    // Compilation detection
    pub compilations: CompilationState,

    // Artist aliases (uniform track artists that differ from album artist)
    pub artist_aliases:
        std::sync::Arc<std::collections::HashMap<String, std::collections::HashSet<String>>>,
    pub album_display_artist: std::collections::HashMap<String, String>,

    // Tag-style lists (each is its own top-level section).
    pub artist_genres: Vec<Genre>,
    pub album_genres: std::sync::Arc<Vec<Genre>>,
    pub moods: Vec<Genre>,
    pub styles: Vec<Genre>,
    pub decades: Vec<Genre>,
    pub years: Vec<Genre>,
    pub collections: Vec<Genre>,
    pub countries: Vec<Genre>,
    pub labels: Vec<Genre>,
    pub formats: Vec<Genre>,
    pub studios: Vec<Genre>,
    pub artist_genres_loading: bool,
    pub album_genres_loading: bool,
    pub moods_loading: bool,
    pub styles_loading: bool,
    pub decades_loading: bool,
    pub years_loading: bool,
    pub collections_loading: bool,
    pub countries_loading: bool,
    pub labels_loading: bool,
    pub formats_loading: bool,
    pub studios_loading: bool,
    pub tag_albums: Vec<Album>,
    pub tag_albums_index: usize,

    // Library sub-mode for Alt+S cycling
    pub library_sub_mode: LibrarySubMode,

    // Right panel content
    pub right_panel_mode: RightPanelMode,
    pub selected_artist_albums: Vec<Album>,
    pub selected_album_tracks: Vec<Track>,
    pub selected_artist_name: String,
    pub selected_album_title: String,
    pub right_panel_loading: bool,
    /// Identity of the in-flight right-panel request. Late completions for a
    /// prior selection are discarded.
    pub right_panel_request_key: Option<String>,
}

/// Search/filter state.
#[derive(Debug, Clone, Default)]
pub struct SearchState {
    pub query: String,
    pub results: Option<SearchResults>,

    pub focus: SearchFocus,
    pub pending_album_key: Option<String>,
    pub pending_track_key: Option<String>,
    pub tab: SearchTab,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RepeatMode {
    #[default]
    Off,
    Track,
    Queue,
}

/// Queue and playback mode state.
#[derive(Debug, Default)]
pub struct QueueState {
    pub repeat: RepeatMode,
    pub selection_anchor: Option<usize>,
    pub tracks: Vec<Track>,
    pub index: Option<usize>,
    pub selected: std::collections::BTreeSet<usize>,
    pub original: Vec<Track>,
    pub sort_mode: QueueSortMode,
    pub history: VecDeque<Track>,
    pub undo_snapshot: Option<QueueSnapshot>,
    pub shuffle_undo_queue: Option<Vec<Track>>,
    pub shuffle_undo_index: Option<usize>,
}

/// Main terminal workspace retained while the queue has keyboard focus.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TuiWorkspace {
    #[default]
    Library,
    NowPlaying,
}
impl TuiWorkspace {
    /// Resolve the retained terminal workspace independently of queue focus.
    pub fn view(self) -> View {
        match self {
            Self::Library => View::Browse,
            Self::NowPlaying => View::NowPlaying,
        }
    }
}

/// Root application state.
#[derive(Debug)]
pub struct AppState {
    pub downloads: super::downloads::State,
    pub player_sync: super::player_sync::State,
    pub staged_settings: Option<super::settings_transfer::Draft>,
    pub terminal_text_selection: Option<(super::terminal_text::Target, std::ops::Range<usize>)>,
    pub presentation_anchors: Option<super::view_continuity::BrowseAnchors>,
    pub browse_viewport: Option<super::view_continuity::BrowseAnchors>,
    pub windowed_presentation: super::windowed_presentation::Presentation,
    /// Remembered GUI arrangement, retained while the Text presentation is active.
    pub gui_multiple_windows: bool,
    pub sources: crate::app::sources::SourceState,
    // Connection
    pub active_library: Option<String>,

    pub connected_server_url: Option<String>,

    pub connection_generation: u64,

    /// Monotonic identity of the selected server/library pair. Background
    /// results carrying an older generation are discarded centrally.
    pub library_generation: u64,
    pub library_navigation: super::navigation_history::LibraryNavigation,
    pub jump_history: crate::app::navigation_history::History,
    pub pending_library_details: Option<crate::app::library_details::Pending>,

    // Authentication flow state

    // Navigation
    pub view: View,
    pub previous_view: Option<View>,
    pub help_scroll: u16,
    pub browse_category: BrowseCategory,
    pub focus: Focus,

    // Category column (column 0 in Browse view)
    /// Whether the category selector column has keyboard focus.
    pub category_column_focused: bool,
    /// Selected index in the category column. Maps into the rows
    /// returned by `category_rows()`, which respects `hidden_sections`.
    pub category_column_index: usize,
    /// Sections the user has hidden from the leftmost browse column
    /// (persisted via UiConfig). Hidden sections still exist in code
    /// but are filtered out of `category_rows()`.
    pub hidden_sections: Vec<BrowseCategory>,
    pub desktop_library_root: LibraryRoot,
    pub tui_queue_visible: bool,
    /// Visibility of the terminal library sidebar.
    pub tui_sidebar_visible: bool,
    pub tui_motion: super::tui_motion::Motion,
    pub tui_description_submit_focused: bool,
    pub playlist_group_filter: Option<String>,
    pub tui_sidebar_group: Option<(String, usize)>,
    /// Inline Describe Music query and focus, independent of its retained results.
    pub tui_description_query: String,
    pub tui_description_focused: bool,
    /// Standalone Shift state reported by enhanced terminal keyboard input.
    pub tui_shift_held: bool,
    /// Terminal header cursor, separate from the retained item selection.
    pub tui_artist_focus: Option<super::presentation::ArtistLink>,
    pub tui_footer_focus: Option<usize>,
    pub tui_queue_type_focused: bool,
    pub tui_transport_focus: Option<super::transport_controls::TransportControl>,
    pub tui_column_header: Option<super::column_controls::ColumnHeaderFocus>,
    pub tui_queue_workspace: TuiWorkspace,
    pub desktop_frontend: bool,
    pub desktop_hidden_library_roots: Vec<LibraryRoot>,
    pub desktop_player_visible: bool,
    /// Whether the host presents visualizers and needs their audio analysis.
    pub visualizers_available: bool,
    /// Whether the host uses a navigable playback waveform without visualizers.
    pub playback_waveform_available: bool,
    pub desktop_player_fullscreen: DesktopFullscreenView,
    pub remix_allow_same_artist: bool,
    pub show_compilation_only_artists: bool,
    pub large_artwork: bool,
    pub singles_track_limit: u32,
    pub revealed_compilation_artist: Option<String>,
    pub desktop_collapsed_sections: Vec<String>,
    pub hidden_collections: Vec<crate::app::sources::navidrome::commands::CollectionKind>,

    // Library data (artists, albums, playlists, genres, etc.)
    pub library: LibraryData,

    // Similar content (server sonic similarity)
    pub similar: SimilarViewState,

    // Related artists (Ctrl+R)
    pub related: RelatedViewState,

    // Playback
    pub playback: PlaybackState,
    pub queue: QueueState,
    /// Last-wins generation for async "replace queue and play" loads.
    pub queue_play_request_id: u64,
    /// Whether user is currently dragging the seek indicator
    pub seek_drag: Option<ratatui::layout::Rect>,
    pub volume_drag: bool,
    /// Consecutive playback errors (for auto-skip with limit)
    pub consecutive_playback_errors: u32,

    // Search
    pub search: SearchState,

    // UI state
    pub list_state: ListStates,
    pub should_quit: bool,

    pub notifications: Notifications,

    // Popups (sort, radio launcher, adventure, artist radio picker, bio, dialogs, library picker, search)
    pub popups: Popups,

    // Modifier bar display: shows Alt or Ctrl+Alt bar until this deadline.
    // Set on any Alt+key / Ctrl+Alt+key press; cleared on non-modifier keypress or timeout.
    pub alt_bar_until: Option<std::time::Instant>,

    // Volume slider: shows until this deadline, then auto-hides.
    pub volume_slider_until: Option<std::time::Instant>,

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

    pub folder_play_request_id: u64,

    // Miller column navigation for browse categories
    pub artist_nav: BrowseNavigationState,
    /// Generations for asynchronous Miller-column loads. Late completions from
    /// a previous cursor position or category are ignored.
    pub artist_nav_request_id: u64,
    /// Shared nav state for all tag-style sections (album genres, artist
    /// genres, moods, styles, decades, years, collections, countries,
    /// labels, formats, studios). Reset when the user switches between
    /// these sections.
    pub tag_nav: BrowseNavigationState,
    pub tag_nav_request_id: u64,
    pub playlist_nav: BrowseNavigationState,
    pub playlist_nav_request_id: u64,
    pub playlist_revision: u64,

    // Playlist tracks cache (playlist_key -> cached tracks with timestamp)

    // Artwork state
    pub artwork: ArtworkState,

    /// Maximum server streaming bitrate in kbps; 0 requests the original file.
    pub transcode_kbps: u32,

    /// Whether audio output is available. False when audio init failed/timed out.
    pub audio_available: bool,

    // Playback mode (Queue vs Radio)
    pub playback_mode: PlaybackMode,
    pub radio_generation: u64,
    /// The pending station or refill belongs to the current playback context.
    pub radio_task: Option<crate::app::tasks::TaskLease>,
    /// Candidate station title; active playback remains unchanged until it loads.
    pub station_starting: Option<StationStart>,
    pub station_navigation_generation: u64,

    // Radio playback state (continuous)
    pub radio: RadioPlaybackState,

    // Station navigation (hierarchical)
    pub station_nav: StationNavigationState,
    pub station_detail: Option<String>,

    /// Cached station children (mood/style/decade sub-lists, keyed by station key).
    pub station_children_cache: std::collections::HashMap<String, Vec<Station>>,

    // Theme
    pub theme: ThemeName,

    /// Desktop player and library are visible together.
    pub desktop_split_view: bool,

    /// Manual horizontal scroll offset (in ribbon slots) for the Library
    /// screen. Only consulted when
    /// `miller_scroll_manual` is true — otherwise the renderer
    /// auto-anchors the ribbon to the focused column.
    pub miller_scroll_col: usize,
    /// A newly opened or replaced child must be fully revealed.
    pub miller_reveal_right: bool,
    /// When true, the user has manually positioned the horizontal
    /// scrollbar (via click or drag) and the renderer should honour
    /// `miller_scroll_col` instead of auto-following the focused
    /// column. Cleared on any keystroke so keyboard navigation snaps
    /// the ribbon back to focus.
    pub miller_scroll_manual: bool,
    /// Active horizontal-scrollbar drag. `Some(g)` means the user is
    /// dragging the scrollbar thumb; `g` is the grab offset in cells
    /// from the left edge of the thumb to the mouse click position so
    /// the thumb tracks the cursor instead of jumping.
    pub miller_h_drag_grab: Option<u16>,

    // Journey state
    pub adventure: AdventureState,
    pub adventure_request_id: u64,
    pub adventure_launcher_request_id: u64,
    pub artist_bio_request_id: u64,

    // Now Playing panel focus (track list vs stations)
    pub now_playing_focus: NowPlayingFocus,
    /// Index of the highlighted sidebar button on the now-playing
    /// screen (0..4). Only meaningful when `now_playing_focus ==
    /// Sidebar`.

    /// Alphabet jump strip (Library category only). Whether the strip
    /// has keyboard focus, and which letter index is highlighted.
    /// Index space matches `ALPHABET_STRIP_LETTERS`: 0=%, 1=0, 2..=27=a..z.
    pub alphabet_strip_focused: bool,
    pub alphabet_strip_index: usize,

    /// Sonically-similar tracks displayed in the right-side track
    /// details pane, keyed by the focused track's `rating_key`.
    /// Populated lazily by the tick handler when a track row is
    /// selected and never queried before. Empty `Vec` means the API
    /// returned no similar tracks (distinct from "not yet loaded").
    pub track_pane_similar: HashMap<String, Result<Vec<Track>, String>>,
    /// Track keys whose similar-tracks fetch is currently in flight.
    pub track_pane_similar_loading: std::collections::HashSet<String>,

    /// Multi-select "expand" mode. While `true`, Up/Down arrows
    /// extend the selection on the focused track list (Miller
    /// column or queue) instead of just moving the cursor. Toggled
    /// on/off by Space; Shift+Space turns it on without clearing
    /// the existing selection. Auto-clears whenever the view or
    /// focused column changes (so it can't strand on a list the
    /// user has navigated away from).
    pub select_mode: bool,

    /// Track-details pane has keyboard focus. While `true`, Up/Down
    /// navigates between playback actions (0/1) and the
    /// Sonically-Similar entries (2..). Enter opens one reusable
    /// adjacent detail pane; Left returns to the previous column. Auto-cleared
    /// whenever the focused track changes or the user navigates
    /// away from the Browse view.
    pub track_pane_focused: bool,
    /// Index of the highlighted row inside the focused track-details
    /// pane: 0/1 = playback, 2.. = Sonically-Similar tracks.
    pub track_pane_index: usize,
    /// One reusable related-track detail column, anchored to the original track.
    pub sonic_details: Option<crate::app::sonic_details::State>,

    /// Live audio sample tap from the audio backend, drained each
    /// Tick into `vectorscope_buffer` for the live visualizers. `None` when no audio backend is available
    /// (`AudioPlayer::new_without_audio`).
    pub vectorscope_tap: Option<crate::audio::SampleTap>,
    /// Rolling stereo sample buffer for live visualizers. Capped at `VECTORSCOPE_BUFFER_LEN`
    /// — once full, oldest samples are overwritten in-place.
    pub vectorscope_buffer: std::collections::VecDeque<(f32, f32)>,
    /// Sampled PCM levels and bounded history for the optional Studio Meters view.
    pub studio_meters: crate::app::meters::StudioMeters,

    /// Per-tick counter, used to drive simple animated text in
    /// loading placeholders ("Loading", "Loading.", "Loading..",
    /// "Loading…"). Wraps freely; consumers use `% 4` etc.
    pub loading_tick: u32,

    // Fullscreen presentation and selected visualizer
    pub fullscreen: FullscreenView,
    pub visualizer_tab: VisualizerTab,
    /// Whether the visualizer tab bar is focused (for arrow key navigation)
    pub visualizer_tab_focused: bool,

    // Cache management
    pub cache_mgmt: CacheManagement,

    // Waveform seekbar state
    pub waveform: WaveformState,

    // Spectrogram state
    pub spectrogram: SpectrogramState,

    // Inline list filter state (/ key in browse view)
    pub list_filter: ListFilterState,

    /// Shared command-palette state (`:` in the TUI). The field is
    /// always present so render/dispatch code doesn't need feature
    /// gates; the GUI just never sets `open = true`.
    pub palette: PaletteState,

    // Marquee state is updated by Tick and explicit render feedback.
    pub marquee: MarqueeState,
    /// Second marquee for subtitle row (2-row track display in playlists)
    pub marquee_subtitle: MarqueeState,

    /// Last successfully rendered hit-test geometry, installed by the event loop.
    /// Populated each frame by render code, consumed by mouse_input handlers.
    pub hit_regions: crate::app::presentation::HitRegions,

    // Library switch loading state
    pub library_loading: bool,

    // Remote player control
    /// Library cache total bytes on disk. Computed on startup and after clears.
    pub library_cache_stats: Option<(u64, Vec<(String, u64)>)>,
    /// Waveform cache stats: (file_count, total_bytes). Computed on startup and after clears.
    pub waveform_cache_stats: Option<(usize, u64)>,
    // Scroll pins and cooldowns
    pub scroll: ScrollPins,

    /// Whether the track-details pane is currently open. The pane is
    /// a *derived view* of `focused_track()` — it never stores its own
    /// Track, so it can't drift out of sync with the column selection.
    /// Past versions held the track as state, which led to a class of
    /// "pane shows the wrong track after Up/Down" bugs.
    ///
    /// Open: explicit drill (Enter / Right / click) on a Track row.
    /// Close: Ctrl+W, Esc inside the pane, navigating away from the
    /// Browse view, or any other "rightward column close" gesture.
    /// While open, the pane content automatically follows whichever
    /// Track row is focused; if focus moves to a non-Track row the
    /// pane simply renders nothing for that frame and reappears when
    /// a Track is focused again.
    pub track_pane_open: bool,

    /// Per-service "external search enabled" toggles, mirrored from
    /// `config.ui.enable_*_search`. Renderers (palette / context menu
    /// / menu bar) read these to decide whether to surface each
    /// service's search entry. The dispatcher also re-checks against
    /// the canonical `Config` so a stale mirror still cannot leak
    /// requests through to disabled services.
    pub external_search: ExternalSearchSettings,
    pub web_search_provider: crate::services::web_search::Provider,

    /// Mirror of `config.ui.library_view_settings` so event handlers
    /// (which don't get `&Config`) can read saved per-playlist
    /// "Group by album" / "Show artwork" toggles when a playlist
    /// tracks column lands. The dispatcher keeps this in sync with
    /// the canonical config on every `SavePlaylistView` /
    /// `PrunePlaylistViews`.
    pub last_playlist_destinations: HashMap<String, String>,
    pub playlist_views: HashMap<String, HashMap<String, crate::config::settings::PlaylistView>>,
}

/// Mirror of the three "Search ⟨service⟩" toggles from `UiConfig`.
#[derive(Debug, Clone, Copy)]
pub struct ExternalSearchSettings {
    pub apple_music: bool,
    pub spotify: bool,
    pub youtube: bool,
}

impl Default for ExternalSearchSettings {
    fn default() -> Self {
        Self {
            apple_music: true,
            spotify: false,
            youtube: true,
        }
    }
}

/// Which view a scrollbar drag is operating on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollbarView {
    Browse,
    Folder,
    Queue,
    Similar,
    Related,
    Help,
}

/// State for an active scrollbar drag operation.
#[derive(Debug, Clone)]
pub struct ScrollbarDrag {
    pub view: ScrollbarView,
    pub col_idx: usize,
    pub total_items: usize,
    pub visible_items: usize,
    pub track_y_start: u16,
    pub track_height: u16,
    pub grab_offset: u16,
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

impl QueueState {
    /// Move a row while keeping the playing index attached to its track.
    /// Returns the final row, or None for an invalid/no-op source.
    pub fn move_track(&mut self, from: usize, to: usize) -> Option<usize> {
        move_queue_occurrence(&mut self.tracks, &mut self.index, from, to)
    }
}

pub(crate) fn move_queue_occurrence(
    tracks: &mut [Track],
    playing: &mut Option<usize>,
    from: usize,
    to: usize,
) -> Option<usize> {
    if from >= tracks.len() || from == to {
        return None;
    }
    let destination = to.min(tracks.len() - 1);
    // Rotate only the affected range. Adjacent keyboard moves stay O(1),
    // rather than shifting the entire queue twice through remove/insert.
    if from < destination {
        tracks[from..=destination].rotate_left(1);
    } else {
        tracks[destination..=from].rotate_right(1);
    }
    *playing = playing.map(|index| {
        if index == from {
            destination
        } else if from < index && destination >= index {
            index - 1
        } else if from > index && destination <= index {
            index + 1
        } else {
            index
        }
    });
    Some(destination)
}

/// Active station info.
#[derive(Debug, Clone)]
pub struct ActiveStation {
    pub source: crate::library::models::RadioSource,
    pub title: String,
}

/// One pending station switch. A continuation owns no audio: the current
/// playback instance finishes normally while discovery prepares its successors.
#[derive(Debug, Clone)]
pub struct StationStart {
    pub title: String,
    pub continue_playback: Option<u64>,
}

/// Radio seed for similarity-based radio.
#[derive(Debug, Clone)]
pub struct RadioSeed {
    pub mode: RadioSeedMode,
    pub key: String,
    pub title: String,
}

/// An in-flight refill either buffers ahead or owes the user one advance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RadioRefill {
    #[default]
    Idle,
    Prefetching,
    Waiting,
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
    /// Pending refill and whether playback must advance when it finishes.
    pub refill: RadioRefill,
    /// For similarity-based radio: the seed info
    pub seed: Option<RadioSeed>,

    // Time Travel Mix state - for chronological continuation
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
        *self = Self::default();
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

impl MillerColumn for StationColumn {
    fn item_count(&self) -> usize {
        self.stations.len()
    }
    fn selected_index(&self) -> usize {
        self.selected_index
    }
    fn set_selected_index(&mut self, idx: usize) {
        self.selected_index = idx;
    }
}

/// Station navigation state for hierarchical stations (Miller columns style).
pub type StationNavigationState = MillerState<StationColumn>;

/// Type-specific methods for station navigation.
impl MillerState<StationColumn> {
    /// Get the selected station in the focused column.
    pub fn selected_station(&self) -> Option<&Station> {
        self.focused().and_then(|c| c.selected_station())
    }
}

impl AppState {
    /// Requested windowed mode, including a presentation handoff that is still preparing.
    pub fn application_mode(&self) -> Option<super::launch::ApplicationMode> {
        use super::launch::{ApplicationMode, WindowedMode};
        let presentation = self
            .windowed_presentation
            .pending()
            .map(|pending| pending.destination)
            .or_else(|| self.windowed_presentation.active())?;
        Some(match presentation {
            WindowedMode::Text => ApplicationMode::Text,
            WindowedMode::Gui if self.gui_multiple_windows => ApplicationMode::GuiMultiple,
            WindowedMode::Gui => ApplicationMode::GuiSingle,
        })
    }
    /// Artwork encoding is the only preference scoped to the presentation.
    pub fn artwork_presentation(&self) -> super::launch::WindowedMode {
        if self.desktop_frontend {
            super::launch::WindowedMode::Gui
        } else {
            super::launch::WindowedMode::Text
        }
    }
    pub fn artwork_mode(&self) -> ArtworkMode {
        match self.artwork_presentation() {
            super::launch::WindowedMode::Gui => self.artwork.graphical_mode,
            super::launch::WindowedMode::Text => self.artwork.mode,
        }
    }
    pub fn initialize_column_artwork(&self, column: &mut BrowseColumn) {
        column.artwork_visible =
            column_artwork_key(self.browse_category, std::slice::from_ref(column), 0)
                .map(|key| {
                    self.artwork
                        .column_visibility
                        .get(&key)
                        .copied()
                        .unwrap_or(true)
                })
                .unwrap_or(false);
    }
    pub fn column_artwork_key(&self, column_index: usize) -> Option<String> {
        column_artwork_key(
            self.browse_category,
            &self.browse_nav()?.columns,
            column_index,
        )
    }

    fn loaded_stations(&self) -> impl Iterator<Item = &Station> {
        self.station_nav
            .columns
            .iter()
            .flat_map(|column| &column.stations)
    }

    pub fn station_by_key(&self, key: &str) -> Option<&Station> {
        self.loaded_stations().find(|station| station.key == key)
    }

    /// Resolve the active library's station instead of constructing a provider URL.
    /// Parent columns retain the root stations while a category is open.
    pub fn random_album_station(&self) -> Option<&Station> {
        self.active_library.as_ref()?;
        self.loaded_stations()
            .find(|station| station.kind() == crate::library::models::StationKind::RandomAlbum)
    }

    /// A new playback context invalidates outstanding station/refill requests.
    pub fn set_playback_mode(&mut self, mode: PlaybackMode) {
        self.radio_task = None;
        self.radio_generation = self.radio_generation.wrapping_add(1);
        self.station_starting = None;
        self.playback_mode = mode;
        // Completions from the previous context are now rejected; their
        // loading flags cannot remain authoritative (including undo snapshots).
        self.radio.refill = RadioRefill::Idle;
    }

    /// Invalidate account- and connection-scoped work without conflating it
    /// with navigation between libraries on the same server.
    pub fn advance_connection_generation(&mut self) {
        self.sources.nav_connection_task = None;
        self.connection_generation = self.connection_generation.wrapping_add(1);
    }

    /// Start a new server/library context and invalidate every asynchronous
    /// request whose result could otherwise be mistaken for current data.
    pub fn advance_library_generation(&mut self) {
        self.playlist_group_filter = None;
        self.station_detail = None;
        self.jump_history.clear();
        self.revealed_compilation_artist = None;
        self.radio_task = None;
        self.library_generation = self.library_generation.wrapping_add(1);
        self.sources.nav_tasks.remove("random-albums");
        self.sources.random_albums = Default::default();
        for nav in [
            &mut self.artist_nav,
            &mut self.playlist_nav,
            &mut self.tag_nav,
        ] {
            nav.columns
                .retain(|column| !matches!(column.details, Some(BrowseDetails::Artist(_))));
            nav.focused_column = nav.focused_column.min(nav.columns.len().saturating_sub(1));
        }
        self.sources.audiomuse.snapshot = None;
        self.sources.audiomuse.refresh_failed = false;
        self.cache_mgmt.next_refresh_check = None;
        self.sources.sonic_tasks.clear();
        self.station_starting = None;
        self.radio.refill = RadioRefill::Idle;
        self.station_nav.loading = false;
        self.station_nav.loading = false;
        // Results from the old generation will be discarded, so their
        // progress entries must be discarded at the same boundary.
        self.cache_mgmt.background_refresh.clear();
        self.cache_mgmt.failures.clear();
        self.queue_play_request_id = self.queue_play_request_id.wrapping_add(1);
        self.folder_play_request_id = self.folder_play_request_id.wrapping_add(1);
        self.artist_nav_request_id = self.artist_nav_request_id.wrapping_add(1);
        self.tag_nav_request_id = self.tag_nav_request_id.wrapping_add(1);
        self.playlist_nav_request_id = self.playlist_nav_request_id.wrapping_add(1);
        self.adventure_request_id = self.adventure_request_id.wrapping_add(1);
        self.adventure_launcher_request_id = self.adventure_launcher_request_id.wrapping_add(1);
        self.artist_bio_request_id = self.artist_bio_request_id.wrapping_add(1);
        self.popups.set_artist_bio(None);
    }

    /// Create a new application state with defaults.
    pub fn new() -> Self {
        Self {
            staged_settings: None,
            terminal_text_selection: None,
            presentation_anchors: None,
            browse_viewport: None,
            windowed_presentation: Default::default(),
            gui_multiple_windows: false,
            sources: Default::default(),

            downloads: Default::default(),
            player_sync: Default::default(),
            active_library: None,

            connected_server_url: None,

            connection_generation: 0,

            library_generation: 0,
            library_navigation: Default::default(),
            jump_history: Default::default(),
            pending_library_details: None,

            view: View::Browse,
            previous_view: None,
            help_scroll: 0,
            browse_category: BrowseCategory::Library,
            focus: Focus::Left,
            category_column_focused: true,
            category_column_index: 0,
            hidden_sections: BrowseCategory::hidden_by_default().to_vec(),
            desktop_library_root: LibraryRoot::Artists,
            tui_queue_visible: true,
            tui_sidebar_visible: true,
            tui_motion: Default::default(),
            tui_description_submit_focused: false,
            playlist_group_filter: None,
            tui_sidebar_group: None,
            tui_description_query: String::new(),
            tui_description_focused: false,
            tui_shift_held: false,
            tui_column_header: None,
            tui_footer_focus: None,
            tui_queue_type_focused: false,
            tui_transport_focus: None,
            tui_artist_focus: None,
            tui_queue_workspace: TuiWorkspace::Library,
            desktop_frontend: false,
            desktop_hidden_library_roots: vec![],
            desktop_player_visible: false,
            visualizers_available: true,
            playback_waveform_available: false,
            desktop_player_fullscreen: DesktopFullscreenView::Closed,
            remix_allow_same_artist: false,
            show_compilation_only_artists: false,
            large_artwork: false,
            singles_track_limit: crate::services::singles::DEFAULT_LIMIT,
            revealed_compilation_artist: None,
            desktop_collapsed_sections: vec![],
            hidden_collections: Vec::new(),
            library: LibraryData::default(),
            similar: SimilarViewState::default(),
            related: RelatedViewState::default(),
            playback: PlaybackState::default(),
            queue: QueueState::default(),
            queue_play_request_id: 0,
            seek_drag: None,
            volume_drag: false,
            consecutive_playback_errors: 0,

            search: SearchState::default(),
            list_state: ListStates::default(),
            should_quit: false,

            notifications: Notifications::default(),
            popups: Popups::default(),
            alt_bar_until: None,
            volume_slider_until: None,
            search_tab: SearchTab::default(),
            terminal_width: 80,
            terminal_height: 24,
            image_loaded: HashMap::new(),
            settings_state: SettingsState::default(),
            folder_state: None,

            folder_play_request_id: 0,

            artist_nav: BrowseNavigationState::new(),
            artist_nav_request_id: 0,
            tag_nav: BrowseNavigationState::new(),
            tag_nav_request_id: 0,
            playlist_nav: BrowseNavigationState::new(),
            playlist_nav_request_id: 0,
            playlist_revision: 0,

            artwork: ArtworkState::default(),
            transcode_kbps: 0,
            audio_available: true,
            playback_mode: PlaybackMode::None,
            radio_generation: 0,
            radio_task: None,
            station_starting: None,
            station_navigation_generation: 0,
            radio: RadioPlaybackState::default(),
            station_nav: StationNavigationState::default(),
            station_detail: None,
            station_children_cache: std::collections::HashMap::new(),
            theme: ThemeName::default(),
            desktop_split_view: false,
            miller_scroll_col: 0,
            miller_reveal_right: false,
            miller_scroll_manual: false,
            miller_h_drag_grab: None,
            adventure: AdventureState::default(),
            adventure_request_id: 0,
            adventure_launcher_request_id: 0,
            artist_bio_request_id: 0,
            now_playing_focus: NowPlayingFocus::default(),
            alphabet_strip_focused: false,
            alphabet_strip_index: 0,
            track_pane_similar: HashMap::new(),
            track_pane_similar_loading: std::collections::HashSet::new(),
            select_mode: false,
            track_pane_focused: false,
            track_pane_index: 0,
            sonic_details: None,
            vectorscope_tap: None,
            studio_meters: crate::app::meters::StudioMeters::default(),
            vectorscope_buffer: std::collections::VecDeque::with_capacity(VECTORSCOPE_BUFFER_LEN),
            loading_tick: 0,
            fullscreen: FullscreenView::Closed,
            visualizer_tab: VisualizerTab::default(),
            visualizer_tab_focused: false,
            cache_mgmt: CacheManagement::default(),
            waveform: WaveformState::default(),
            spectrogram: SpectrogramState::default(),
            list_filter: ListFilterState::default(),
            palette: PaletteState::default(),
            marquee: MarqueeState::default(),
            marquee_subtitle: MarqueeState::default(),
            hit_regions: crate::app::presentation::HitRegions::default(),
            library_loading: false,

            library_cache_stats: None,
            waveform_cache_stats: None,
            scroll: ScrollPins::default(),
            track_pane_open: false,
            external_search: ExternalSearchSettings::default(),
            web_search_provider: Default::default(),
            last_playlist_destinations: Default::default(),
            playlist_views: HashMap::new(),
        }
    }

    /// Visibility choices for the active provider, including currently hidden views.
    pub fn sidebar_sections(&self) -> Vec<SidebarSection> {
        use crate::app::sources::navidrome::commands::CollectionKind;
        let mut sections: Vec<_> = BrowseCategory::all()
            .iter()
            .copied()
            .filter(|c| {
                if self.sources.active.folder().is_some() {
                    *c == BrowseCategory::Folders
                } else if self.sources.active.navidrome().is_some() {
                    matches!(
                        c,
                        BrowseCategory::Library
                            | BrowseCategory::Folders
                            | BrowseCategory::AlbumGenres
                            | BrowseCategory::Decades
                            | BrowseCategory::Playlists
                    )
                } else {
                    true
                }
            })
            .map(SidebarSection::Category)
            .collect();
        if self.sources.active.navidrome().is_some() {
            sections.extend(CollectionKind::SIDEBAR.map(SidebarSection::Collection));
            if crate::app::sources::sonic::enabled(self)
                && crate::app::sources::audiomuse::connection(self).is_some()
            {
                sections.extend(
                    crate::audiomuse::Feature::ALL
                        .map(|f| SidebarSection::Collection(CollectionKind::AudioMuse(f))),
                );
            }
        }
        sections
    }

    /// Visible sidebar destinations and selectable disclosure headings.
    /// Rendering and input use this exact ordering after collapsed groups are hidden.
    pub fn category_rows(&self) -> Vec<CategoryRow> {
        let playlist_indices: std::collections::HashMap<_, _> = self
            .library
            .playlists
            .iter()
            .enumerate()
            .map(|(index, playlist)| (playlist.rating_key.as_str(), index))
            .collect();
        let mut rows: Vec<_> = crate::app::sidebar::visible_rows(self)
            .into_iter()
            .filter_map(|row| {
                if row.header {
                    return Some(CategoryRow::Header(match row.label.as_str() {
                        "Radio" => "Radio",
                        "Albums" => "Albums",
                        "AI" => "AI",
                        "Playlists" => "Playlists",
                        _ => return None,
                    }));
                }
                if let Some(root) = row.library_root {
                    return Some(CategoryRow::Root(root));
                }
                if let Some(key) = row.playlist_key {
                    return playlist_indices
                        .get(key.as_str())
                        .copied()
                        .map(CategoryRow::Playlist);
                }
                row.actions.iter().find_map(|action| match action {
                    crate::app::Action::Navigation(
                        crate::app::action::NavigationAction::OpenRadioStation(key),
                    ) => self
                        .station_nav
                        .columns
                        .first()?
                        .stations
                        .iter()
                        .position(|s| &s.key == key)
                        .map(CategoryRow::Station),
                    crate::app::Action::Navigation(
                        crate::app::action::NavigationAction::SetCategory { category, .. },
                    ) => Some(CategoryRow::Category(*category)),
                    crate::app::Action::Source(crate::app::sources::SourceAction::Navidrome(
                        action,
                    )) => match action.as_ref() {
                        crate::app::sources::navidrome::NavAction::Command(
                            crate::app::sources::navidrome::commands::Command::Collection(kind),
                        ) => Some(CategoryRow::NavidromeCollection(*kind)),
                        _ => None,
                    },
                    _ => None,
                })
            })
            .collect();
        if rows.is_empty() {
            rows.push(CategoryRow::Search);
        }
        rows
    }

    /// Source-specific destinations before sidebar grouping and visibility projection.
    pub fn raw_category_rows(&self) -> Vec<CategoryRow> {
        if matches!(self.sources.active, crate::app::sources::ActiveSource::None) {
            return vec![
                CategoryRow::Search,
                CategoryRow::Header("Browse"),
                CategoryRow::Category(BrowseCategory::Library),
            ];
        }
        if self.sources.active.folder().is_some() {
            return vec![
                CategoryRow::Search,
                CategoryRow::Header("Browse"),
                CategoryRow::Category(BrowseCategory::Folders),
            ]
            .into_iter()
            .filter(
                |row| !matches!(row, CategoryRow::Category(c) if self.hidden_sections.contains(c)),
            )
            .collect();
        }
        let hidden = &self.hidden_sections;
        let mut rows =
            Vec::with_capacity(BrowseCategory::all().len() + self.library.playlists.len() + 2);

        rows.push(CategoryRow::Search);
        rows.push(CategoryRow::Header("Browse"));
        for &c in BrowseCategory::top_rows() {
            if self.sources.active.navidrome().is_some()
                && !matches!(
                    c,
                    BrowseCategory::Library
                        | BrowseCategory::AlbumGenres
                        | BrowseCategory::Decades
                        | BrowseCategory::Folders
                )
            {
                continue;
            }
            if c == BrowseCategory::Playlists {
                continue;
            }
            if hidden.contains(&c) {
                continue;
            }
            rows.push(CategoryRow::Category(c));
        }
        if self.sources.active.navidrome().is_some() {
            use crate::app::sources::navidrome::commands::CollectionKind;
            rows.extend(
                CollectionKind::SIDEBAR
                    .into_iter()
                    .filter(|kind| !self.hidden_collections.contains(kind))
                    .map(CategoryRow::NavidromeCollection),
            );
            if crate::app::sources::sonic::enabled(self)
                && crate::app::sources::audiomuse::connection(self).is_some()
            {
                let entries: Vec<_> = crate::audiomuse::Feature::ALL
                    .into_iter()
                    .map(CollectionKind::AudioMuse)
                    .filter(|kind| !self.hidden_collections.contains(kind))
                    .map(CategoryRow::NavidromeCollection)
                    .collect();
                if !entries.is_empty() {
                    rows.push(CategoryRow::Header("AudioMuse"));
                    rows.extend(entries);
                }
            }
            if !hidden.contains(&BrowseCategory::Playlists) && !self.library.playlists.is_empty() {
                rows.push(CategoryRow::Header("Playlists"));
                rows.extend((0..self.library.playlists.len()).map(CategoryRow::Playlist));
            }
            return rows;
        }

        rows
    }

    /// Get the BrowseNavigationState for the current browse category.
    /// Returns None for Folders (which uses FolderNavigationState instead).
    pub fn browse_nav(&self) -> Option<&BrowseNavigationState> {
        match self.browse_category {
            BrowseCategory::Library => Some(&self.artist_nav),
            BrowseCategory::Playlists => Some(&self.playlist_nav),
            BrowseCategory::Folders => None,
            cat if cat.is_tag_section() => Some(&self.tag_nav),
            _ => None,
        }
    }

    /// Playlist names always live in the sidebar; their internal root stays hidden.
    pub fn browse_column_offset(&self) -> usize {
        usize::from(
            self.browse_category == BrowseCategory::Playlists
                || (self.browse_category == BrowseCategory::Library
                    && self.sources.nav_collection.is_none()
                    && matches!(
                        self.desktop_library_root,
                        LibraryRoot::Albums | LibraryRoot::Compilations | LibraryRoot::Tracks
                    )),
        )
    }

    /// View Options eligibility shared by the popup and terminal header buttons.
    pub fn column_options_type(&self, col_idx: usize) -> Option<SortColumnType> {
        let nav = self.browse_nav()?;
        let col = nav.columns.get(col_idx)?;
        if super::downloads::is_root_column(self, col_idx) {
            Some(SortColumnType::AllTracks)
        } else if col
            .items
            .iter()
            .take(3)
            .any(|i| matches!(i, BrowseItem::Artist { .. }))
        {
            Some(SortColumnType::Artist)
        } else if col
            .items
            .iter()
            .take(4)
            .any(|i| matches!(i, BrowseItem::Album { .. }))
        {
            Some(if col.date_added_order.is_some() {
                SortColumnType::RecentAlbums
            } else {
                SortColumnType::Album
            })
        } else if col
            .items
            .iter()
            .any(|item| matches!(item, BrowseItem::Playlist { .. }))
            || (self.browse_category == BrowseCategory::Playlists
                && col_idx == 0
                && col.lazy.is_none())
        {
            Some(SortColumnType::Playlist)
        } else if col.grouped_by_album {
            Some(SortColumnType::Album)
        } else if col.lazy.is_some() || matches!(col.items.first(), Some(BrowseItem::Track { .. }))
        {
            Some(if self.is_special_track_column(nav, col_idx) {
                SortColumnType::AllTracks
            } else {
                SortColumnType::Track
            })
        } else {
            None
        }
    }

    /// Get a mutable reference to the BrowseNavigationState for the current browse category.
    /// Returns None for Folders (which uses FolderNavigationState instead).
    pub fn browse_nav_mut(&mut self) -> Option<&mut BrowseNavigationState> {
        match self.browse_category {
            BrowseCategory::Library => Some(&mut self.artist_nav),
            BrowseCategory::Playlists => Some(&mut self.playlist_nav),
            BrowseCategory::Folders => None,
            cat if cat.is_tag_section() => Some(&mut self.tag_nav),
            _ => None,
        }
    }

    /// Whether the alphabet jump strip should be visible. Mirrors the
    /// render-side condition so keyboard handlers can reach the strip
    /// only when it's actually on screen.
    pub fn alphabet_strip_visible(&self) -> bool {
        self.browse_category == BrowseCategory::Library
            && self.sources.nav_collection.is_none()
            && self
                .artist_nav
                .columns
                .first()
                .is_some_and(|c| !c.items.is_empty() && c.sort_mode != ColumnSortMode::Shuffled)
    }

    /// The track currently highlighted in the focused Miller column,
    /// if any. Used by the right-side track details pane: the pane
    /// shows iff this returns `Some`.
    /// Artist information belongs to the selected destination and its adjacent child.
    pub fn artist_info_column_index(&self) -> Option<usize> {
        let nav = self.browse_nav()?;
        let focused = nav.focused_column;
        let column = nav.columns.get(focused)?;
        if matches!(column.details, Some(BrowseDetails::Artist(_))) {
            return Some(focused);
        }
        let BrowseItem::ArtistInfo { artist_key, .. } = column.selected_item()? else {
            return None;
        };
        nav.columns
            .get(focused + 1)
            .and_then(|column| match &column.details {
                Some(BrowseDetails::Artist(info)) if &info.artist_key == artist_key => {
                    Some(focused + 1)
                }
                _ => None,
            })
    }
    pub fn focused_track(&self) -> Option<&crate::library::models::Track> {
        if self.browse_category == BrowseCategory::Folders {
            let column = self.folder_state.as_ref()?.focused()?;
            let item = column
                .items
                .get(column.selected_index)
                .filter(|item| item.is_track())?;
            return column
                .tracks
                .get(item.rating_key.as_ref().unwrap_or(&item.key));
        }
        let nav = self.browse_nav()?;
        let col = nav.columns.get(nav.focused_column)?;
        let item = col.items.get(col.selected_index)?;
        if !matches!(item, BrowseItem::Track { .. }) || item.pending_download().is_some() {
            return None;
        }
        col.tracks.get(col.selected_index)
    }

    /// The track to render in the details pane.
    ///
    /// The pane is a derived view: when `track_pane_open` is true and
    /// a Track row is focused, this returns that focused track. The
    /// pane content automatically follows the focused row — there is
    /// no separately stored "the track the pane was opened on", so
    /// the pane cannot drift out of sync with the column selection.
    ///
    /// Returns `None` when the pane is closed *or* when the focused
    /// row isn't a Track (e.g. the user moved selection to an Album
    /// or Artist row); in the latter case the renderer simply skips
    /// the pane for that frame and it reappears when a Track row is
    /// focused again.
    pub fn pane_track(&self) -> Option<&crate::library::models::Track> {
        if !self.track_pane_open {
            return None;
        }
        self.focused_track()
    }

    /// The album currently highlighted in the focused Miller column,
    /// if any. Returns `(rating_key, title)`. Used by the palette
    /// to surface "Play Album" / "Artist Bio" context-aware entries.
    pub fn focused_album(&self) -> Option<(String, String)> {
        let nav = self.browse_nav()?;
        let col = nav.columns.get(nav.focused_column)?;
        let item = col.items.get(col.selected_index)?;
        match item {
            BrowseItem::Album { key, title, .. } => Some((key.clone(), title.clone())),
            _ => None,
        }
    }

    /// Visible ordered track list and highlighted row for palette commands.
    /// Never fall back to a retained navigation column from another view.
    pub fn palette_track_list(&self) -> Option<(&[Track], usize)> {
        if self.palette.player_context {
            return Some((self.playback_tracks(), self.list_state.queue_index));
        }
        if self.view == View::Browse
            && self
                .browse_nav()
                .and_then(|nav| nav.focused())
                .and_then(|col| col.selected_item())
                .is_some_and(|item| item.pending_download().is_some())
        {
            return None;
        }


        if self.popups.search_active() || matches!(self.view, View::Search | View::Similar) {
            return crate::app::result_details::track_list(self);
        }
        if let Some(list) = crate::app::sonic_details::palette_tracks(self) {
            return Some(list);
        }
        let (tracks, index): (&[Track], usize) = match self.view {
            View::Queue | View::NowPlaying => (self.playback_tracks(), self.list_state.queue_index),
            View::Browse if !self.category_column_focused => {
                if self.palette_target_is_similar() {
                    let parent = self.focused_track()?;
                    (
                        self.track_pane_similar
                            .get(&parent.rating_key)?
                            .as_ref()
                            .ok()?,
                        self.track_pane_index - 3,
                    )
                } else {
                    self.focused_track()?;
                    let col = self.browse_nav()?.focused()?;
                    (&col.tracks, col.selected_index)
                }
            }
            _ => return None,
        };
        tracks.get(index)?;
        Some((tracks, index))
    }

    pub fn palette_target_track(&self) -> Option<Track> {
        let (tracks, index) = self.palette_track_list()?;
        tracks.get(index).cloned()
    }

    /// Whether the palette's context-aware target is a Sonically-
    /// Similar row inside the track pane (vs. a Miller-column row).
    /// Used to gate per-list commands like "Play Track and Following"
    /// that don't make sense on a free-floating similar track.
    pub fn palette_target_is_similar(&self) -> bool {
        if self.palette.player_context {
            return false;
        }
        if crate::app::sonic_details::focused(self) {
            return crate::app::sonic_details::visible(self)
                .is_some_and(|c| c.details.action_index >= 3)
                && crate::app::sonic_details::palette_tracks(self).is_some();
        }
        self.view == View::Browse
            && !self.category_column_focused
            && self.track_pane_focused
            && self.track_pane_index > 2
            && self
                .focused_track()
                .and_then(|p| self.track_pane_similar.get(&p.rating_key))
                .and_then(|result| result.as_ref().ok())
                .map(|v| v.get(self.track_pane_index - 3).is_some())
                .unwrap_or(false)
    }

    /// Build the album-artist list, separating confirmed compilations.
    pub fn build_artist_root_items(&self) -> Vec<BrowseItem> {
        let mut hidden = self.library.compilations.artist_keys.clone();
        if self.show_compilation_only_artists {
            hidden.clear();
        } else if let Some(key) = &self.revealed_compilation_artist {
            hidden.remove(key);
        }
        let mut items = BrowseItem::artist_root_items_with_compilations(
            &self.library.artists,
            !self.desktop_frontend && !self.library.compilations.albums.is_empty(),
            &hidden,
        );
        if self.show_compilation_only_artists || self.revealed_compilation_artist.is_some() {
            let mut keys: std::collections::HashSet<_> =
                items.iter().map(|item| item.key().to_owned()).collect();
            let extras: Vec<_> = self
                .library
                .track_artists
                .iter()
                .filter(|artist| {
                    self.library
                        .compilations
                        .artist_keys
                        .contains(&artist.rating_key)
                        && (self.show_compilation_only_artists
                            || self.revealed_compilation_artist.as_ref()
                                == Some(&artist.rating_key))
                        && keys.insert(artist.rating_key.clone())
                })
                .cloned()
                .collect();
            if !extras.is_empty() {
                items.extend(BrowseItem::from_artists(&extras));
                let pinned = items
                    .iter()
                    .take_while(|item| !matches!(item, BrowseItem::Artist { .. }))
                    .count();
                items[pinned..].sort_by_cached_key(|item| {
                    (
                        item.is_placeholder_item(),
                        crate::services::artist_alias_service::normalize_artist_name(item.title()),
                    )
                });
            }
        }
        items
    }

    pub fn artist_destination_for_track(&self, track: &Track) -> Option<(String, String)> {
        let name =
            crate::services::artist_alias_service::normalize_artist_name(track.track_artist());
        self.library
            .artists
            .iter()
            .chain(&self.library.track_artists)
            .find(|artist| {
                crate::services::artist_alias_service::normalize_artist_name(&artist.title) == name
            })
            .or_else(|| {
                self.library
                    .artists
                    .iter()
                    .chain(&self.library.track_artists)
                    .find(|artist| {
                        Some(&artist.rating_key) == track.grandparent_rating_key.as_ref()
                            && track.track_artist() == track.artist_name()
                    })
            })
            .map(|artist| (artist.rating_key.clone(), artist.title.clone()))
    }
    pub fn set_compilation_artists_visible(&mut self, show: bool) {
        let selected = self
            .artist_nav
            .columns
            .first()
            .and_then(|column| column.selected_item())
            .map(|item| item.key().to_owned());
        self.show_compilation_only_artists = show;
        self.revealed_compilation_artist =
            selected.filter(|key| !show && self.library.compilations.artist_keys.contains(key));
        let items = self.build_artist_root_items();
        if !self.artist_nav.columns.is_empty() {
            self.artist_nav.update_root_items("artists", items);
        }
    }
    pub fn is_ghost_artist(&self, key: &str) -> bool {
        !self.show_compilation_only_artists
            && self.revealed_compilation_artist.as_deref() == Some(key)
            && self.library.compilations.artist_keys.contains(key)
    }

    /// Select a catalog playlist, rebuilding a missing navigation root when necessary.
    pub fn select_playlist(&mut self, key: &str) -> bool {
        if !self
            .library
            .playlists
            .iter()
            .any(|playlist| playlist.rating_key == key)
        {
            return false;
        }
        if self
            .playlist_nav
            .columns
            .first()
            .is_none_or(|column| !column.items.iter().any(|item| item.key() == key))
        {
            self.playlist_nav.update_root_items(
                "playlists",
                BrowseItem::from_playlists(&self.library.playlists),
            );
        }
        let Some(root) = self.playlist_nav.columns.first_mut() else {
            return false;
        };
        let Some(index) = root.items.iter().position(|item| item.key() == key) else {
            return false;
        };
        let changed = root.selected_index != index;
        root.selected_index = index;
        if changed {
            self.playlist_nav.columns.truncate(1);
            self.playlist_nav.focused_column = 0;
        }
        if let Some(row) = self.category_rows().iter().position(|row| matches!(row, CategoryRow::Playlist(index) if self.library.playlists.get(*index).is_some_and(|playlist| playlist.rating_key == key))) {
            self.category_column_index = row;
        }
        true
    }

    /// Replace playlist metadata while retaining the selected identity and its loaded tracks.
    pub fn replace_playlists(&mut self, mut playlists: Vec<crate::library::models::Playlist>) {
        let sidebar_playlist = self
            .category_rows()
            .get(self.category_column_index)
            .and_then(|row| match row {
                CategoryRow::Playlist(index) => self
                    .library
                    .playlists
                    .get(*index)
                    .map(|playlist| playlist.rating_key.clone()),
                _ => None,
            });
        self.playlist_revision = self.playlist_revision.wrapping_add(1);
        let selected = self
            .playlist_nav
            .columns
            .first()
            .and_then(|column| column.selected_item())
            .map(|item| item.key().to_owned());
        playlists.sort_by_key(|playlist| crate::app::handlers::helpers::sort_key(&playlist.title));
        self.library.playlists = std::sync::Arc::new(playlists);
        self.playlist_nav.update_root_items(
            "playlists",
            BrowseItem::from_playlists(&self.library.playlists),
        );
        if let Some(key) = sidebar_playlist
            && let Some(index) = self.category_rows().iter().position(|row| matches!(row, CategoryRow::Playlist(index) if self.library.playlists.get(*index).is_some_and(|playlist| playlist.rating_key == key))) { self.category_column_index = index; }
        if selected.is_some_and(|key| {
            !self
                .library
                .playlists
                .iter()
                .any(|playlist| playlist.rating_key == key)
        }) {
            self.sources.nav_tasks.remove("playlist-browse");
            self.playlist_nav_request_id = self.playlist_nav_request_id.wrapping_add(1);
            self.playlist_nav.columns.truncate(1);
            self.playlist_nav.focused_column = 0;
        }
    }

    /// Switch to a new view, deactivating the inline filter and clearing queue multi-select.
    pub fn set_view(&mut self, view: View) {
        if view != View::Queue {
            self.tui_queue_type_focused = false;
        }
        self.fullscreen = crate::app::state::FullscreenView::Closed;
        if self.view == view {
            return;
        }
        if view != View::Browse {
            self.tui_description_focused = false;
            self.tui_description_submit_focused = false;
        }
        if self.list_filter.active {
            self.list_filter.deactivate();
        }
        // Overlay entry owns its return target for every route (palette, mouse,
        // keyboard and provider actions). Refreshing/cycling an overlay keeps it.
        if matches!(view, View::Similar | View::Related | View::Settings) {
            if !matches!(self.view, View::Similar | View::Related | View::Settings) {
                self.previous_view = Some(self.view);
            }
        } else if !matches!(view, View::Queue | View::NowPlaying) {
            self.queue.selected.clear();
        }
        if view == View::Queue && matches!(self.view, View::Browse | View::NowPlaying) {
            self.tui_queue_workspace = if self.view == View::Browse {
                TuiWorkspace::Library
            } else {
                TuiWorkspace::NowPlaying
            };
        }
        self.select_mode = false;
        self.view = view;
    }

    /// Set browse category, sync category_column_index, and (usually)
    /// unfocus the sections column.
    ///
    /// Focus rule:
    /// - If the new category equals the current one (no real change),
    ///   leave `category_column_focused` alone. This preserves the
    ///   "Library is selected on launch" default, which earlier got
    ///   stomped by startup paths that called `set_browse_category(Library)`
    ///   even though it was already Library.
    /// - If `preserve_sections_focus` is set, this is a sections-column
    ///   Up/Down sweep and the user is still arrow-keying through the
    ///   sections column. Don't steal focus.
    /// - Otherwise (an explicit Right / Enter / click drill, or a
    ///   `Ctrl+L|P|G|O` shortcut, or palette command), unfocus the
    ///   sections column so focus moves onto the rightward content.
    pub fn set_browse_category(&mut self, cat: BrowseCategory, preserve_sections_focus: bool) {
        if self.sources.nav_collection.take().is_some() {
            self.sources.nav_tasks.remove("collection");
            self.sources.nav_tasks.remove("audiomuse");
            self.artist_nav_request_id = self.artist_nav_request_id.wrapping_add(1);
            self.artist_nav =
                BrowseNavigationState::with_root("artists", self.build_artist_root_items());
        }
        let was_same = self.browse_category == cat;
        self.browse_category = cat;
        self.category_column_index = self.row_index_for_category(cat);
        // Only unfocus the sections column when (a) the category
        // actually changed, AND (b) this isn't an auto-drill (the
        // user is sweeping selection through the sections column with
        // Up/Down and we want the rightward content to follow without
        // stealing keyboard focus).
        if !was_same && !preserve_sections_focus {
            self.category_column_focused = false;
        }
        // Strip is Library-only; clear the focus flag whenever the
        // active category changes so it can't strand on a hidden strip.
        self.alphabet_strip_focused = false;
        // Multi-select mode is per-list — switching category abandons
        // the list it was active on, so reset the flag.
        self.select_mode = false;
    }

    /// Resolve a category to the row index it occupies in
    /// `category_rows()` — the canonical sections-column ordering.
    /// `BrowseCategory::all()` is *not* the right index source: it
    /// includes `Playlists` and reorders things relative to what the
    /// renderer iterates (`BrowseCategory::top_rows()` minus
    /// `hidden_sections`). Using `all()` to set
    /// `category_column_index` was the source of teleporting-cursor
    /// bugs as the user arrowed through the sections column.
    pub(crate) fn row_index_for_category(&self, cat: BrowseCategory) -> usize {
        let rows = self.category_rows();
        let fallback = rows
            .iter()
            .position(|r| !matches!(r, CategoryRow::Header(_)))
            .unwrap_or(0);
        if cat == BrowseCategory::Library
            && let Some(kind) = self.sources.nav_collection
        {
            return rows
                .iter()
                .position(|row| *row == crate::app::sidebar::collection_row(kind))
                .unwrap_or(fallback);
        }
        rows.iter()
            .position(|r| matches!(r, CategoryRow::Category(c) if *c == cat) || (cat == BrowseCategory::Library && matches!(r, CategoryRow::Root(root) if *root == self.desktop_library_root)))
            .unwrap_or(fallback)
    }

    /// Focus the category column, syncing category_column_index to match
    /// the active browse_category so the highlight is always correct.
    /// Single-focus rule: also drops pane focus so only one surface
    /// paints as focused.
    pub fn focus_category_column(&mut self) {
        self.category_column_index = self.row_index_for_category(self.browse_category);
        self.category_column_focused = true;
        self.alphabet_strip_focused = false;
        self.select_mode = false;
        self.track_pane_focused = false;
    }

    pub fn set_error(&mut self, msg: String) {
        crate::util::activity_error(&msg);
        self.notifications.last_error = Some(msg);
        self.notifications.error_target = None;
    }

    pub fn set_recoverable_error(&mut self, msg: String, target: NotificationTarget) {
        self.set_error(msg);
        self.notifications.error_target = Some(target);
    }

    /// Clear the current error.
    pub fn clear_error(&mut self) {
        self.notifications.last_error = None;
        self.notifications.error_target = None;
    }

    /// Set a status message (auto-clears after 5 seconds).
    pub fn set_status(&mut self, msg: String) {
        self.notifications.status_target = None;
        tracing::info!(target: "textamp::activity", "Status · {}", msg.escape_debug());
        self.notifications.status_message = Some(msg);
        self.notifications.status_show_time = Some(std::time::Instant::now());
    }

    /// Clear the status message.
    pub fn clear_status(&mut self) {
        self.notifications.status_message = None;
        self.notifications.status_target = None;
        self.notifications.status_show_time = None;
    }

    /// Convert radio playback to queue mode, returning a snapshot for undo.
    pub fn convert_radio_to_queue(&mut self, description: &str) -> QueueSnapshot {
        let snapshot = QueueSnapshot {
            recorded_at: std::time::Instant::now(),
            contents: QueueContents::Radio(Box::new(self.radio.clone())),
            description: description.to_string(),
        };
        // Take tracks from radio instead of cloning (avoids redundant allocation)
        self.queue.tracks = std::mem::take(&mut self.radio.tracks);
        self.queue.index = self.radio.track_index;
        self.set_playback_mode(PlaybackMode::Queue);
        if let Some(idx) = self.queue.index {
            self.list_state.queue_index = idx;
        }
        self.radio.clear();
        snapshot
    }

    /// Whether the given column in the given nav is a "special track column"
    /// that supports the Ctrl+V view cycle (tracks/shuffled/by-album/by-artist/covers).
    ///
    /// Special track columns are those where the user cannot already tell
    /// artist/album from the Miller context:
    /// - Playlist track columns
    /// - All Library Tracks (parent AllTracks `__all_library__`)
    /// - Compilation All Tracks (parent AllTracks `__comp_tracks:*`)
    /// - Per-artist All Tracks (parent AllTracks item)
    /// - Compilation album track columns (parent album in `compilation_albums`)
    pub fn is_special_track_column(&self, nav: &BrowseNavigationState, col_idx: usize) -> bool {
        let col = match nav.columns.get(col_idx) {
            Some(c) => c,
            None => return false,
        };
        let first_is_track = col
            .items
            .first()
            .is_some_and(|item| matches!(item, BrowseItem::Track { .. }));
        if !first_is_track {
            return false;
        }

        if self.desktop_library_root == LibraryRoot::Tracks
            && self.browse_category == BrowseCategory::Library
            && col_idx == 1
            && self.sources.nav_collection.is_none()
        {
            return true;
        }
        // AudioMuse results mix artists and albums, just like playlist tracks.
        if self.browse_category == BrowseCategory::Library
            && matches!(
                self.sources.nav_collection,
                Some(crate::app::sources::navidrome::commands::CollectionKind::AudioMuse(_))
            )
        {
            return true;
        }

        // Playlist track columns (always special)
        if self.browse_category == BrowseCategory::Playlists {
            return true;
        }

        // Check parent item for AllTracks or compilation album
        if col_idx > 0
            && let Some(parent_item) = nav.columns.get(col_idx - 1).and_then(|p| p.selected_item())
        {
            match parent_item {
                // Per-artist All Tracks, All Library Tracks, Compilation All Tracks
                BrowseItem::AllTracks { .. } => return true,
                // Compilation Tracks for a specific artist
                BrowseItem::CompilationTracks { .. } => return true,
                // Compilation album track column
                BrowseItem::Album { key, .. }
                    if self
                        .library
                        .compilations
                        .albums
                        .iter()
                        .any(|a| a.rating_key == *key) =>
                {
                    return true;
                }
                _ => {}
            }
        }

        false
    }

    /// Get the current notification to display (ongoing takes priority over toast).
    /// Returns None if no notification should be shown.
    /// Progress follows the operation's task lease and cannot expire as a toast.
    pub fn operation_progress(&self) -> Option<&'static str> {
        if self.sources.nav_tasks.contains_key("remix") {
            Some("Generating queue edit…")
        } else if self.sources.nav_tasks.contains_key("audiomuse") {
            Some("Searching music…")
        } else {
            None
        }
    }

    pub fn current_notification(&self) -> Option<Notification> {
        if let Some(message) = self.operation_progress() {
            return Some(Notification::ongoing(message));
        }
        // Priority 1: Adventure mode notifications (ongoing)
        if self.adventure.active && self.adventure.generating {
            return Some(Notification::ongoing("🌟 Generating sonic bridge..."));
        }

        if self.notifications.last_error.is_none()
            && let Some(message) = super::activity::message(self)
        {
            let mut notification = Notification::ongoing(message);
            notification.target = super::activity::target(self);
            return Some(notification);
        }

        // Priority 5: Background refresh (ongoing)
        if !self.cache_mgmt.background_refresh.is_empty() {
            let categories: Vec<_> = self
                .cache_mgmt
                .background_refresh
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

        // Priority 6: Waveform generation (ongoing)
        if self.waveform.generating() {
            return Some(Notification::ongoing("Generating waveform..."));
        }

        // Priority 7: Cache saving (ongoing)

        // Priority 8: Toast notifications (transient)
        if let Some(ref msg) = self.notifications.toast_message {
            return Some(Notification::toast(msg.clone()));
        }

        // Priority 9: Status messages (transient)
        if let Some(ref msg) = self.notifications.status_message {
            let mut notification = Notification::toast(msg.clone());
            notification.target = self.notifications.status_target;
            return Some(notification);
        }

        None
    }

    /// Tracks in the active playback context, excluding the inactive queue.
    pub fn playback_tracks(&self) -> &[Track] {
        match self.playback_mode {
            PlaybackMode::Radio => &self.radio.tracks,
            PlaybackMode::Queue | PlaybackMode::None => &self.queue.tracks,
        }
    }

    /// Refill requests depend on the old track list. A separately requested station
    /// switch remains valid when the active list is edited.
    pub(crate) fn invalidate_radio_refill(&mut self) {
        if self.station_starting.is_none() && self.radio.refill != RadioRefill::Idle {
            self.radio_task = None;
            self.radio_generation = self.radio_generation.wrapping_add(1);
            self.radio.refill = RadioRefill::Idle;
        }
    }

    pub(crate) fn playback_buffer_mut(&mut self) -> (&mut Vec<Track>, &mut Option<usize>) {
        if self.playback_mode == PlaybackMode::Radio {
            self.invalidate_radio_refill();
            (&mut self.radio.tracks, &mut self.radio.track_index)
        } else {
            (&mut self.queue.tracks, &mut self.queue.index)
        }
    }

    /// Get the currently playing track (mode-aware).
    pub fn current_track(&self) -> Option<&Track> {
        match self.playback_mode {
            PlaybackMode::Queue | PlaybackMode::None => {
                self.queue.index.and_then(|idx| self.queue.tracks.get(idx))
            }
            PlaybackMode::Radio => self.radio.current_track(),
        }
    }

    /// Available artwork for the playing item, including a cached album image.
    pub fn current_artwork_bytes(&self) -> Option<&[u8]> {
        self.artwork.current_data.as_deref().or_else(|| {
            self.current_track().and_then(|track| {
                self.artwork
                    .grid_cache
                    .get(&track.rating_key)
                    .or_else(|| {
                        track
                            .parent_rating_key
                            .as_ref()
                            .and_then(|key| self.artwork.grid_cache.get(key))
                    })
                    .map(Vec::as_slice)
            })
        })
    }

    /// The next queued item, without cloning tracks or changing playback.
    pub fn next_track(&self) -> Option<&Track> {
        match self.playback_mode {
            PlaybackMode::Radio => self
                .radio
                .track_index
                .and_then(|index| self.radio.tracks.get(index + 1)),
            PlaybackMode::Queue | PlaybackMode::None => self.queue.index.and_then(|index| {
                self.queue.tracks.get(index + 1).or_else(|| {
                    (self.queue.repeat == RepeatMode::Queue)
                        .then(|| self.queue.tracks.first())
                        .flatten()
                })
            }),
        }
    }

    /// Update prepared metadata in the active context, never the inactive queue.
    pub fn current_track_mut(&mut self) -> Option<&mut Track> {
        match self.playback_mode {
            PlaybackMode::Radio => self
                .radio
                .track_index
                .and_then(|i| self.radio.tracks.get_mut(i)),
            PlaybackMode::Queue | PlaybackMode::None => {
                self.queue.index.and_then(|i| self.queue.tracks.get_mut(i))
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
            BrowseCategory::Library => self.library.artists.len(),
            BrowseCategory::Playlists => self.library.playlists.len(),
            BrowseCategory::Folders => 0, // Handled separately via folder_state
            cat if cat.is_tag_section() => self.tag_list_for(cat).len(),
            _ => 0,
        }
    }

    /// Return the tag-list backing storage for a tag section.
    pub fn tag_list_for(&self, cat: BrowseCategory) -> &[Genre] {
        match cat {
            BrowseCategory::AlbumGenres => &self.library.album_genres,
            BrowseCategory::ArtistGenres => &self.library.artist_genres,
            BrowseCategory::Moods => &self.library.moods,
            BrowseCategory::Styles => &self.library.styles,
            BrowseCategory::Decades => &self.library.decades,
            BrowseCategory::Years => &self.library.years,
            BrowseCategory::Collections => &self.library.collections,
            BrowseCategory::Countries => &self.library.countries,
            BrowseCategory::Labels => &self.library.labels,
            BrowseCategory::Formats => &self.library.formats,
            BrowseCategory::Studios => &self.library.studios,
            _ => &[],
        }
    }

    /// Whether the tag list for the active section is currently loading.
    pub fn current_tag_loading(&self) -> bool {
        match self.browse_category {
            BrowseCategory::AlbumGenres => self.library.album_genres_loading,
            BrowseCategory::ArtistGenres => self.library.artist_genres_loading,
            BrowseCategory::Moods => self.library.moods_loading,
            BrowseCategory::Styles => self.library.styles_loading,
            BrowseCategory::Decades => self.library.decades_loading,
            BrowseCategory::Years => self.library.years_loading,
            BrowseCategory::Collections => self.library.collections_loading,
            BrowseCategory::Countries => self.library.countries_loading,
            BrowseCategory::Labels => self.library.labels_loading,
            BrowseCategory::Formats => self.library.formats_loading,
            BrowseCategory::Studios => self.library.studios_loading,
            _ => false,
        }
    }

    /// Get the current category index. For tag sections, reads the
    /// selected_index of the tag_nav root column.
    pub fn category_index(&self) -> usize {
        match self.browse_category {
            BrowseCategory::Library => self.list_state.artists_index,
            BrowseCategory::Playlists => self.list_state.playlists_index,
            BrowseCategory::Folders => 0,
            cat if cat.is_tag_section() => self
                .tag_nav
                .columns
                .first()
                .map(|c| c.selected_index)
                .unwrap_or(0),
            _ => 0,
        }
    }

    /// Set the current category index.
    pub fn set_category_index(&mut self, idx: usize) {
        match self.browse_category {
            BrowseCategory::Library => self.list_state.artists_index = idx,
            BrowseCategory::Playlists => self.list_state.playlists_index = idx,
            BrowseCategory::Folders => {}
            cat if cat.is_tag_section() => {
                if let Some(c) = self.tag_nav.columns.first_mut() {
                    c.selected_index = idx;
                }
            }
            _ => {}
        }
    }

    /// Get the selected category item's rating key.
    pub fn selected_category_key(&self) -> Option<String> {
        match self.browse_category {
            BrowseCategory::Library => self
                .library
                .artists
                .get(self.list_state.artists_index)
                .map(|a| a.rating_key.clone()),
            BrowseCategory::Playlists => self
                .library
                .playlists
                .get(self.list_state.playlists_index)
                .map(|p| p.rating_key.clone()),
            BrowseCategory::Folders => None,
            cat if cat.is_tag_section() => {
                let list = self.tag_list_for(cat);
                let idx = self.category_index();
                list.get(idx).map(|g| g.effective_key().to_string())
            }
            _ => None,
        }
    }
}

pub fn column_artwork_key(
    _category: BrowseCategory,
    columns: &[BrowseColumn],
    column_index: usize,
) -> Option<String> {
    let column = columns.get(column_index)?;
    let kind = if let Some(details) = &column.details {
        match details {
            BrowseDetails::Artist(_) => "artist-details",
            BrowseDetails::Album { .. } => "album-details",
        }
    } else {
        column.items.iter().find_map(|item| match item {
            BrowseItem::Album { .. } => Some("albums"),
            BrowseItem::Track { .. } => Some("tracks"),
            _ => None,
        })?
    };
    Some(format!("type:{kind}"))
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Current application view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Browse library (main view with left: categories, right: tracks)
    Browse,
    /// Queue view — shows queue/radio tracks with stations panel and artwork
    Queue,
    /// Now Playing view — shows artwork, track info, and visualizer (waveform/spectrum/spectrogram)
    NowPlaying,
    /// Unified Search/Filter screen with tabs
    Search,
    /// Similar albums view
    Similar,
    /// Related artists view (Ctrl+R)
    Related,
    /// Help / keybindings
    Help,
    /// Settings screen
    Settings,
}

/// Visualizer tab for the Now Playing view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisualizerTab {
    #[default]
    Waveform,
    Spectrum,
    Spectrogram,
    /// Stereo Lissajous vectorscope. Drives X with the right
    /// channel and Y with the left, plotting the live audio's
    /// stereo image as a figure-of-eight trace. Both the TUI
    /// (braille glyphs) and the GUI (canvas) render this from
    /// the shared sample-tap pipeline.
    Vectorscope,
    Landscape,
    Meters,
}

cyclic_enum!(
    VisualizerTab,
    Waveform,
    Spectrum,
    Spectrogram,
    Vectorscope,
    Landscape,
    Meters
);

impl VisualizerTab {
    pub const ALL: [Self; 6] = [
        Self::Waveform,
        Self::Spectrum,
        Self::Spectrogram,
        Self::Vectorscope,
        Self::Landscape,
        Self::Meters,
    ];

    /// Shared tab labels/layout for rendering and hit testing. On narrow
    /// panes, show a window beginning at the selected tab.
    pub fn visible_tabs(self, width: u16) -> Vec<(Self, &'static str)> {
        let compact = width < 100;
        let labels = ["wave", "spec", "gram", "XY", "land", "meters"];
        let tabs: Vec<_> = Self::ALL
            .iter()
            .enumerate()
            .map(|(i, &tab)| (tab, if compact { labels[i] } else { tab.name() }))
            .collect();
        let total: usize = tabs.iter().map(|(_, name)| name.len() + 5).sum();
        let start = if total.saturating_sub(3) > width as usize {
            self as usize
        } else {
            0
        };
        let mut used = 0;
        tabs.into_iter()
            .skip(start)
            .take_while(|(_, label)| {
                used += label.len() + if used == 0 { 2 } else { 5 };
                used <= width as usize
            })
            .collect()
    }

    pub fn hit_tab(self, width: u16, column: u16) -> Option<Self> {
        let mut x = 0;
        for (tab, label) in self.visible_tabs(width) {
            let end = x + label.len() as u16 + 2;
            if (x..end).contains(&column) {
                return Some(tab);
            }
            x = end + 3;
        }
        None
    }

    pub fn allows_canvas_seek(self) -> bool {
        !matches!(self, Self::Landscape | Self::Meters)
    }

    pub fn name(&self) -> &'static str {
        match self {
            VisualizerTab::Waveform => "Waveform",
            VisualizerTab::Spectrum => "Spectrum",
            VisualizerTab::Spectrogram => "Spectrogram",
            VisualizerTab::Vectorscope => "Vectorscope",
            VisualizerTab::Landscape => "Landscape",
            VisualizerTab::Meters => "Meters",
        }
    }
}

/// Search tab in unified search view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchTab {
    /// All results combined
    #[default]
    Global,
    /// Artists only
    Artists,
    /// Albums only
    Albums,
    /// Playlists only
    Playlists,
    /// Tracks only (uses API search)
    Tracks,
    /// Genres only
    Genres,
}

cyclic_enum!(
    SearchTab, Global, Artists, Albums, Playlists, Tracks, Genres
);

impl SearchTab {
    pub fn all() -> &'static [SearchTab] {
        Self::CYCLE_ORDER
    }

    pub fn name(&self) -> &'static str {
        match self {
            SearchTab::Global => "all",
            SearchTab::Artists => "artists",
            SearchTab::Albums => "albums",
            SearchTab::Playlists => "playlists",
            SearchTab::Tracks => "tracks",
            SearchTab::Genres => "genres",
        }
    }
}

/// Desktop library destinations; the terminal keeps its existing category layout.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
pub enum LibraryRoot {
    Downloads,
    Mp3Players,
    #[default]
    Artists,
    Albums,
    Compilations,
    Singles,
    Tracks,
    Radio,
    Journey,
    DescribeMusic,
}
impl LibraryRoot {
    pub const ALL: [Self; 6] = [
        Self::Downloads,
        Self::Artists,
        Self::Albums,
        Self::Compilations,
        Self::Singles,
        Self::Tracks,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Downloads => "Downloads",
            Self::Mp3Players => "Audio Player",
            Self::Artists => "Artists",
            Self::Albums => "Albums",
            Self::Compilations => "Compilations",
            Self::Singles => "Singles",
            Self::Tracks => "Tracks",
            Self::Radio => "Radio",
            Self::Journey => "Journey",
            Self::DescribeMusic => "Describe Music",
        }
    }
}

/// Browse category type (what's shown in left panel).
///
/// Each variant other than Library / Playlists / Folders is a "tag"
/// section: a flat list of values fetched from server (album genres,
/// moods, decades, etc.) that drills into albums. They all share the
/// `tag_nav` state, which is reset when the user switches between
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum BrowseCategory {
    Library,
    Playlists,
    Folders,
    AlbumGenres,
    ArtistGenres,
    Moods,
    Styles,
    Decades,
    Years,
    Collections,
    Countries,
    Labels,
    Formats,
    Studios,
}

impl BrowseCategory {
    pub fn all() -> &'static [BrowseCategory] {
        &[
            BrowseCategory::Library,
            BrowseCategory::Playlists,
            BrowseCategory::Folders,
            BrowseCategory::AlbumGenres,
            BrowseCategory::ArtistGenres,
            BrowseCategory::Moods,
            BrowseCategory::Styles,
            BrowseCategory::Decades,
            BrowseCategory::Years,
            BrowseCategory::Collections,
            BrowseCategory::Countries,
            BrowseCategory::Labels,
            BrowseCategory::Formats,
            BrowseCategory::Studios,
        ]
    }

    /// Categories shown as fixed rows at the top of the leftmost
    /// column. Playlists are listed individually below this set
    /// (see `AppState::category_rows`) rather than collapsed into a
    /// single "Playlists" entry.
    pub fn top_rows() -> &'static [BrowseCategory] {
        &[
            BrowseCategory::Library,
            BrowseCategory::Folders,
            BrowseCategory::AlbumGenres,
            BrowseCategory::ArtistGenres,
            BrowseCategory::Moods,
            BrowseCategory::Styles,
            BrowseCategory::Decades,
            BrowseCategory::Years,
            BrowseCategory::Collections,
            BrowseCategory::Countries,
            BrowseCategory::Labels,
            BrowseCategory::Formats,
            BrowseCategory::Studios,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            BrowseCategory::Library => "library",
            BrowseCategory::Playlists => "playlists",
            BrowseCategory::Folders => "folders",
            BrowseCategory::AlbumGenres => "genres",
            BrowseCategory::ArtistGenres => "artist genres",
            BrowseCategory::Moods => "moods",
            BrowseCategory::Styles => "styles",
            BrowseCategory::Decades => "decades",
            BrowseCategory::Years => "years",
            BrowseCategory::Collections => "collections",
            BrowseCategory::Countries => "countries",
            BrowseCategory::Labels => "labels",
            BrowseCategory::Formats => "formats",
            BrowseCategory::Studios => "studios",
        }
    }

    /// Display label for the category column (capitalized).
    pub fn display_label(&self) -> &'static str {
        match self {
            BrowseCategory::Library => "Library",
            BrowseCategory::Playlists => "Playlists",
            BrowseCategory::Folders => "Folders",
            BrowseCategory::AlbumGenres => "Genres",
            BrowseCategory::ArtistGenres => "Artist Genres",
            BrowseCategory::Moods => "Moods",
            BrowseCategory::Styles => "Styles",
            BrowseCategory::Decades => "Decades",
            BrowseCategory::Years => "Years",
            BrowseCategory::Collections => "Collections",
            BrowseCategory::Countries => "Countries",
            BrowseCategory::Labels => "Labels",
            BrowseCategory::Formats => "Formats",
            BrowseCategory::Studios => "Studios",
        }
    }

    /// True if this section is one of the tag-style sections (i.e.
    /// uses `tag_nav` and a flat list of strings → albums).
    pub fn is_tag_section(&self) -> bool {
        matches!(
            self,
            BrowseCategory::AlbumGenres
                | BrowseCategory::ArtistGenres
                | BrowseCategory::Moods
                | BrowseCategory::Styles
                | BrowseCategory::Decades
                | BrowseCategory::Years
                | BrowseCategory::Collections
                | BrowseCategory::Countries
                | BrowseCategory::Labels
                | BrowseCategory::Formats
                | BrowseCategory::Studios
        )
    }

    /// Sections hidden by default — sparsely populated in most server
    /// libraries. Users can toggle visibility from the Settings panel.
    pub fn hidden_by_default() -> &'static [BrowseCategory] {
        &[
            BrowseCategory::Collections,
            BrowseCategory::Countries,
            BrowseCategory::Labels,
            BrowseCategory::Formats,
            BrowseCategory::Studios,
        ]
    }
}

/// A single navigable row in the leftmost "category" column.
///
/// `Category(_)` covers Library / Genres / Folders (the fixed top
/// rows); `Playlist(i)` is the i-th entry of `state.library.playlists`
/// promoted to a top-level row, matching the GUI's design where each
/// playlist gets its own clickable line below the divider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoryRow {
    Root(LibraryRoot),
    Search,
    NavidromeCollection(crate::app::sources::navidrome::commands::CollectionKind),
    Category(BrowseCategory),
    Playlist(usize),
    Station(usize),
    /// A selectable disclosure heading shared by sidebar keyboard and mouse navigation.
    Header(&'static str),
}

/// A configurable sidebar section, independent of its current visible position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarSection {
    Category(BrowseCategory),
    Collection(crate::app::sources::navidrome::commands::CollectionKind),
}
impl SidebarSection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Category(c) => c.display_label(),
            Self::Collection(c) => c.label(),
        }
    }
    pub fn hidden(self, state: &AppState) -> bool {
        match self {
            Self::Category(c) => state.hidden_sections.contains(&c),
            Self::Collection(c) => state.hidden_collections.contains(&c),
        }
    }
}

/// Shared command-palette state. `open == false` means we're in
/// the normal input mode; the field is left present in shared state
/// so render fns and the event loop can read it without feature
/// gates spreading across the codebase.
///
/// The palette holds the fuzzy-match query as a plain String + a
/// char-cursor; the TUI input layer wraps these in a
/// `tui_input::Input` to leverage that crate's edit semantics
/// (Ctrl+A / Ctrl+E / etc.) without leaking the `tui-input` type
/// into the shared model.
///
/// `entries` is the materialized candidate list — it's rebuilt each
/// time the user types, mixing the static command registry with
/// runtime content (radio stations, playlists, …). `matches` indexes
/// into `entries` post-fuzzy-sort.
#[derive(Debug, Clone, Default)]
pub struct PaletteState {
    pub playlist_addition: Option<(u64, super::sources::navidrome::playlist_edit::Addition)>,
    pub history: super::command_history::History,
    pub path: Vec<String>,
    pub player_context: bool,
    pub open: bool,
    pub query: String,
    pub cursor: usize,
    pub selected: usize,
    pub entries: Vec<PaletteEntry>,
    pub matches: Vec<usize>,
}

/// One materialized row in the palette. The display string is owned
/// because some rows are built from runtime data (e.g. radio station
/// titles); the executable side is in `command`. `aliases` carries
/// extra fuzzy-search terms that don't appear in the rendered label
/// — typing "del", "remove", "skip", "mute", etc. should surface the
/// matching command even though that exact word isn't in the label.
#[derive(Debug, Clone)]
pub struct PaletteEntry {
    pub label: String,
    pub hint: String,
    pub command: PaletteCommandKind,
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TextPopup {
    pub lyrics_query: Option<String>,
    pub title: String,
    pub text: String,
    pub scroll: u16,
    pub request_id: u64,
}

/// A frontend-independent command selected through a menu or palette.
#[derive(Debug, Clone)]
pub enum PaletteCommandKind {
    PlayerSync(super::player_sync::Menu),
    LyricsControl(super::lyrics::Control),
    QueueTool(super::presentation::NpSidebarButton),
    Downloads(super::downloads::Menu),
    CycleApplicationMode,
    SetApplicationMode(super::launch::ApplicationMode),
    ArtistDetail(super::details::ArtistCommand),
    GotoSingles,
    OpenCommandGroup(Vec<String>),
    ToggleFullscreen,
    FullscreenArtwork,
    DesktopFullscreen(DesktopFullscreenView),
    OpenRadioBrowser,
    OpenAiBrowser,
    PlayRandomAlbums,
    LibraryMix,
    StopRadio,
    Navidrome(crate::app::sources::navidrome::commands::Command),
    Quit,
    GotoLibrary,
    GotoGenres,
    GotoFolders,
    GotoQueue,
    GotoNowPlaying,
    OpenHelp,
    OpenSettings,
    OpenSearch,
    SaveQueue,
    ExportPlaylist {
        format: crate::services::playlist_export::Format,
        queue: bool,
    },
    SaveDescriptionPlaylist,
    ClearQueue,
    ToggleFilter,
    /// Show the desktop player alongside the library.
    ToggleSplitView,
    Refresh,
    PlayPause,
    StopPlayback,
    NextTrack,
    PrevTrack,
    RemixGemini,
    RemixTwofer,
    RemixStretch,
    RemixDoppelganger,
    RemixOppositeDay,
    RemixEnemies,
    RemixShuffle,
    RemixUndoShuffle,
    /// Start a station identified by its station URL, not an artist rating key.
    PlayStation(String),
    ConfigureStation(String),
    BrowseStations {
        key: String,
        title: String,
    },
    StationsBack,
    ArtistRadio,
    /// Pick one album at random from the active library and play it
    /// once (clear queue + load tracks). Distinct from
    /// "Play Random Albums" which is a continuous station.
    /// Reveal a randomly chosen catalog album without changing playback.
    GoToRandomAlbum,
    /// Drill into the currently-playing track's album in the Library
    /// view (so the user lands on it as if they'd browsed there).
    OpenInLibrary,
    JumpToArtist(Box<Track>),
    JumpToAlbum(Box<Track>),
    /// Open the sort popup for the focused column.
    OpenSort,
    /// Toggle album-art tiles on the focused album column.
    ToggleArtwork,
    ToggleCompilationArtists,
    /// Toggle group-by-album on a playlist tracks column.
    ToggleGroupByAlbum,
    /// Play just the focused track (replace queue with this one
    /// track). Context-aware: only surfaced when the focused row is
    /// a Track.
    PlayFocusedTrack,
    /// Play the focused track plus every track after it in the
    /// current view (replace queue). Context-aware on Track rows.
    PlayFocusedTrackAndFollowing,
    /// Open the system browser to search Apple Music / Spotify /
    /// YouTube for the current selection (artist / album / track /
    /// now-playing).
    SearchAppleMusic,
    SearchSpotify,
    SearchYouTube,
    /// Play the currently-focused album immediately (replaces the
    /// queue with its tracks). Surfaced when a Browse album row is
    /// focused.
    PlayFocusedAlbum,
    /// Open the Artist Bio popup for whatever artist the current
    /// row resolves to (track → its artist; album → album artist;
    /// artist row → that artist; falls back to now-playing).
    ShowArtistBio,
    /// Apply a specific sort mode to the focused Miller column
    /// (same dispatch the sort popup uses). Surfaced as individual
    /// "Sort: …" entries in the palette so the user can pick a
    /// mode without going through the popup.
    ApplySort(ColumnSortMode),
    /// Flip the sort direction (ascending ↔ descending) on the
    /// focused column.
    ReverseSort,
    /// Close the focused Miller column (and any cols to its
    /// right). Same effect as the Ctrl+W shortcut.
    CloseColumn,
    /// Open the Journey launcher pre-seeded with the
    /// focused track as the starting song. Context-aware: surfaced
    /// when a Track row is focused (or a similar-track row is
    /// highlighted in the track-details pane). Distinct from the
    /// generic "Journey" entry, which opens the launcher
    /// with no starting track and asks the user to pick one.
    SonicAdventureFromFocusedTrack,
    /// Generic Journey entry-point — opens the launcher
    /// with no preselected starting track, so the user picks one
    /// inside the dialog. Always available via the static registry,
    /// menu bar, and keyboard shortcut.
    SonicAdventure,
    /// Carries a single shared `track_context::ContextKind` plus the
    /// track it targets. The palette materializer builds these from
    /// the shared `track_context_entries` list so the contextual
    /// section stays in lockstep with the GUI's right-click menu —
    /// adding or reordering entries in `app::track_context` is
    /// the only edit needed.
    FromTrackContext {
        kind: crate::app::track_context::ContextKind,
        track: Box<crate::library::models::Track>,
    },
    /// Open the F3 library-picker popup.
    SwitchLibrary,
    /// Toggle scrolling Miller layout (the `\` shortcut).
    /// Remove the focused row from the play queue (Del shortcut).
    RemoveFocusedFromQueue,
    /// Undo the last queue edit (Ctrl+Z shortcut).
    UndoQueueEdit,
    /// Playback / transport controls. These mirror the keyboard
    /// shortcuts so the palette is a complete index of the app's
    /// functions — every action that has a key binding is reachable
    /// by typing its name (or a synonym).
    VolumeUp,
    VolumeDown,
    ToggleMute,
    ToggleAutoPlay,
    ToggleAutoMix,
    SetQueueRepeat(RepeatMode),
    ShowLyrics,
    SeekForward,
    SeekBackward,
    /// Queue mutation helpers — context-aware (operate on the
    /// focused row / selection in whatever view the user popped the
    /// palette from). Mirror the Ctrl+E / Ctrl+Shift+E shortcuts and
    /// the Shift+↑↓ reorder.
    EnqueueSelectionEnd,
    EnqueueSelectionNext,
    MoveQueueSelectionUp,
    MoveQueueSelectionDown,
}

impl PaletteState {
    pub fn close(&mut self) {
        self.playlist_addition = None;
        self.path.clear();
        self.player_context = false;
        self.open = false;
        self.query.clear();
        self.cursor = 0;
        self.selected = 0;
        self.entries.clear();
        self.matches.clear();
    }
}

/// Library sub-mode for Alt+S cycling: Normal → All Albums (by artist) → All Albums (shuffled).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibrarySubMode {
    #[default]
    Normal, // Standard artist list with drill-down
    AllByArtist, // All albums sorted by artist
    AllShuffled, // All albums shuffled
}

cyclic_enum!(LibrarySubMode, Normal, AllByArtist, AllShuffled);

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

/// Keyboard focus within the queue and current-track areas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NowPlayingFocus {
    #[default]
    Tracks,
    Tools,
    /// The current artwork beside the track metadata.
    Artwork,
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
    Artists,
}

/// Reporting is independent of transport: pausing does not reset this state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScrobbleState {
    #[default]
    Pending,
    Queued,
}

/// Playback state.
#[derive(Debug, Clone)]
pub struct PlaybackState {
    /// Monotonic identity of the current local playback attempt. Async audio
    /// completions and delayed retries are ignored when this no longer matches.
    pub request_id: u64,
    /// Identity of the current URL/preparation effect. This advances before
    /// transcode negotiation so a late decision cannot start an old track.
    pub preparation_id: u64,
    pub status: PlayStatus,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub volume: f32,
    pub muted: bool,
    pub auto_mix: bool,
    /// Queued once the played/scrobble report is scheduled for this track.
    /// Reset only when a new track starts.
    pub scrobble: ScrobbleState,
    /// When the current track transitioned to Playing (for grace period on TrackEnded detection).
    pub playback_started_at: Option<std::time::Instant>,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            request_id: 0,
            preparation_id: 0,
            status: PlayStatus::Stopped,
            position_ms: 0,
            duration_ms: 0,
            volume: 0.8,
            muted: false,
            auto_mix: false,
            scrobble: ScrobbleState::Pending,
            playback_started_at: None,
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

#[derive(Debug, Clone)]
pub enum AnalysisProgress {
    Downloading { received: usize, total: Option<u64> },
    Decoding,
}
impl AnalysisProgress {
    pub fn message(&self) -> String {
        match self {
            Self::Downloading {
                received,
                total: Some(total),
            } if *total > 0 => {
                format!(
                    "Waveform: downloading audio… {}%",
                    (*received as u64).saturating_mul(100) / total
                )
            }
            Self::Downloading { received, .. } => {
                format!(
                    "Waveform: downloading audio… {:.1} MiB",
                    *received as f64 / 1_048_576.
                )
            }
            Self::Decoding => "Analyzing audio…".into(),
        }
    }
}

/// Lifecycle of analysis for one track. Successful data and failures are exclusive.
#[derive(Debug, Clone, Default)]
pub enum AnalysisPhase<T> {
    #[default]
    Idle,
    Running {
        progress: Option<AnalysisProgress>,
    },
    WaitingToRetry,
    Ready(std::sync::Arc<T>),
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct TrackAnalysis<T> {
    /// Owns cancellation, including a shared waveform/spectrogram download.
    pub task: Option<crate::app::tasks::TaskLease>,
    pub track_key: Option<String>,
    pub phase: AnalysisPhase<T>,
    pub retry_count: u8,
}
impl<T> Default for TrackAnalysis<T> {
    fn default() -> Self {
        Self {
            task: None,
            track_key: None,
            phase: AnalysisPhase::Idle,
            retry_count: 0,
        }
    }
}
impl<T> TrackAnalysis<T> {
    pub fn data(&self) -> Option<&T> {
        if let AnalysisPhase::Ready(data) = &self.phase {
            Some(data.as_ref())
        } else {
            None
        }
    }
    pub fn shared_data(&self) -> Option<std::sync::Arc<T>> {
        if let AnalysisPhase::Ready(data) = &self.phase {
            Some(data.clone())
        } else {
            None
        }
    }
    pub fn error(&self) -> Option<&String> {
        if let AnalysisPhase::Failed(error) = &self.phase {
            Some(error)
        } else {
            None
        }
    }
    pub fn generating(&self) -> bool {
        matches!(
            self.phase,
            AnalysisPhase::Running { .. } | AnalysisPhase::WaitingToRetry
        )
    }
    pub fn progress(&self) -> Option<&AnalysisProgress> {
        if let AnalysisPhase::Running { progress } = &self.phase {
            progress.as_ref()
        } else {
            None
        }
    }
    pub fn start(&mut self) {
        self.phase = AnalysisPhase::Running { progress: None };
    }
    pub fn report(&mut self, value: AnalysisProgress) {
        if let AnalysisPhase::Running { progress } = &mut self.phase {
            *progress = Some(value);
        }
    }
    pub fn succeed(&mut self, data: T) {
        self.phase = AnalysisPhase::Ready(std::sync::Arc::new(data));
    }
    pub fn fail(&mut self, error: String) {
        self.phase = AnalysisPhase::Failed(error);
    }
}
pub type WaveformState = TrackAnalysis<WaveformData>;
pub type SpectrogramState = TrackAnalysis<crate::media::SpectrogramData>;

/// Search popup focus state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchFocus {
    #[default]
    Input,
    Results,
}

/// Adventure launcher step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdventureStep {
    FindStartTrack,
    FindEndTrack,
}

/// Drill level within the adventure launcher search.
#[derive(Debug, Clone)]
pub enum AdventureDrillLevel {
    Search,
    ArtistAlbums {
        artist_key: String,
        artist_name: String,
        albums: Vec<Album>,
    },
    AlbumTracks {
        album_key: String,
        album_title: String,
        artist_name: String,
        tracks: Vec<Track>,
    },
}

/// Adventure launcher form. `step` selects the field receiving search results.
/// Start and end tracks are stored separately for reversal and generation.
#[derive(Debug, Clone)]
pub struct AdventureLauncherState {
    pub terminal_focus: super::journey::JourneyControl,
    pub purpose: TrackPickerPurpose,
    pub stage: JourneyStage,
    pub step: AdventureStep,
    pub query: String,
    pub results: Option<SearchResults>,
    pub focus: SearchFocus,
    pub item_index: usize,
    pub loading: bool,
    pub drill: AdventureDrillLevel,
    pub start_track: Option<Track>,
    pub end_track: Option<Track>,
    pub track_count_input: String,
    pub scroll_pin: Option<usize>,
    pub search_tab: SearchTab,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TrackPickerPurpose {
    #[default]
    Journey,
    SonicRadio,
    StayCloseRadio,
}

/// Exclusive phases of choosing, generating and reviewing a Journey.
#[derive(Debug, Clone, Default)]
pub enum JourneyStage {
    #[default]
    Choosing,
    Generating,
    Ready(Vec<Track>),
    Failed(String),
}

/// List selection states for different views.
#[derive(Debug, Default)]
pub struct ListStates {
    pub artists_index: usize,
    pub albums_index: usize,
    pub playlists_index: usize,
    pub right_albums_index: usize, // Albums in right panel (for artist drill-down)
    pub tracks_index: usize,
    pub queue_index: usize,
    pub similar_index: usize,
    pub related_index: usize,
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
    Libraries,
    Textamp,
    About,
}

impl SettingsSection {
    pub fn all() -> &'static [SettingsSection] {
        &[
            SettingsSection::Libraries,
            SettingsSection::Textamp,
            SettingsSection::About,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            SettingsSection::Libraries => "libraries",
            SettingsSection::Textamp => "textamp",
            SettingsSection::About => "about",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            SettingsSection::Libraries => SettingsSection::Textamp,
            SettingsSection::Textamp => SettingsSection::About,
            SettingsSection::About => SettingsSection::Libraries,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            SettingsSection::Textamp => SettingsSection::Libraries,
            SettingsSection::Libraries => SettingsSection::About,
            SettingsSection::About => SettingsSection::Textamp,
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

/// Ordered settings rows shared by rendering and keyboard activation. Provider
/// capabilities determine the rows, so hidden controls cannot shift action indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextampSetting {
    LibraryRootVisible(LibraryRoot),
    RemixAllowSameArtist,
    SinglesTrackLimit,
    Theme(crate::app::theme::ThemeName),
    Presentation(super::launch::ApplicationMode),
    Artwork(ArtworkMode),

    Transcode,
    ExternalSearch(crate::services::external_search::SearchTarget),
    WebSearch(crate::services::web_search::Provider),
    Sidebar(SidebarSection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsPage {
    #[default]
    Themes,
    Audio,
    Sidebar,
    Services,
}

impl SettingsPage {
    pub const ALL: [Self; 4] = [Self::Themes, Self::Audio, Self::Sidebar, Self::Services];
    pub fn name(self) -> &'static str {
        match self {
            Self::Themes => "Appearance",
            Self::Audio => "Audio",
            Self::Sidebar => "Sidebar",
            Self::Services => "Search",
        }
    }
    pub fn step(self, forward: bool) -> Self {
        let index = Self::ALL.iter().position(|p| *p == self).unwrap_or(0);
        Self::ALL[(index + if forward { 1 } else { Self::ALL.len() - 1 }) % Self::ALL.len()]
    }
}

impl AppState {
    pub fn textamp_settings(&self) -> Vec<TextampSetting> {
        self.textamp_settings_for(self.settings_state.page)
    }

    pub fn textamp_settings_for(&self, page: SettingsPage) -> Vec<TextampSetting> {
        use crate::services::external_search::SearchTarget;
        use TextampSetting::*;
        if page == SettingsPage::Themes {
            return crate::app::theme::ThemeName::all()
                .iter()
                .copied()
                .map(Theme)
                .chain(
                    super::launch::ApplicationMode::ALL
                        .into_iter()
                        .filter(|mode| self.windowed_presentation.can_activate(mode.presentation()))
                        .map(Presentation),
                )
                .chain(
                    ArtworkMode::all()
                        .iter()
                        .copied()
                        .filter(|_| self.active_library.is_some())
                        .map(Artwork),
                )
                .collect();
        }
        if page == SettingsPage::Sidebar {
            return std::iter::once(SinglesTrackLimit)
                .chain(
                    crate::app::sidebar::sidebar_options(self)
                        .into_iter()
                        .map(|choice| choice.target.setting()),
                )
                .collect();
        }
        if page == SettingsPage::Services {
            return [
                SearchTarget::AppleMusic,
                SearchTarget::Spotify,
                SearchTarget::YouTube,
            ]
            .into_iter()
            .map(ExternalSearch)
            .chain(
                crate::services::web_search::Provider::ALL
                    .into_iter()
                    .map(WebSearch),
            )
            .collect();
        }
        if page == SettingsPage::Audio {
            let mut items = Vec::new();
            if self
                .sources
                .active
                .capabilities()
                .supports(crate::library::capabilities::Feature::Transcoding)
            {
                items.push(Transcode);
            }
            items.push(RemixAllowSameArtist);
            return items;
        }
        Vec::new()
    }
}

/// Settings screen state.
#[derive(Debug, Clone, Default)]
pub struct SettingsState {
    pub about_taps: u8,
    pub activity_history: Option<std::sync::Arc<str>>,
    pub page: SettingsPage,
    /// Return from a connection or rename form to its library setup page.
    pub library_return: Option<crate::app::sources::options::Options>,
    pub cache_scans: std::collections::HashMap<String, crate::app::sources::cache::ScanState>,
    pub scan_request: u64,
    pub cache_request: u64,
    pub cache_entries: Vec<(crate::app::sources::LibraryChoice, Result<u64, String>)>,
    pub artwork_cache_entries: std::collections::HashMap<String, (usize, u64)>,
    pub cache_task: Option<crate::app::tasks::TaskLease>,

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
    /// Active server account name, also used by the library manager.
    pub username_input: String,
    /// Scroll offset for the About section (display-only, no selectable items)
    pub scroll: u16,
}

impl SettingsState {
    /// One flat category list; app pages and library management share navigation.
    pub const CATEGORY_COUNT: usize = SettingsPage::ALL.len() + 2;

    pub fn category_index(&self) -> usize {
        match self.section {
            SettingsSection::Libraries => 0,
            SettingsSection::Textamp => {
                1 + SettingsPage::ALL
                    .iter()
                    .position(|p| *p == self.page)
                    .unwrap_or(0)
            }
            SettingsSection::About => Self::CATEGORY_COUNT - 1,
        }
    }

    pub fn select_category(&mut self, index: usize) {
        let index = index.min(Self::CATEGORY_COUNT - 1);
        if index == self.category_index() {
            return;
        }
        self.about_taps = 0;
        self.activity_history = None;
        self.section = if index == 0 {
            SettingsSection::Libraries
        } else if index == Self::CATEGORY_COUNT - 1 {
            SettingsSection::About
        } else {
            self.page = SettingsPage::ALL[index - 1];
            SettingsSection::Textamp
        };
        self.item_index = 0;
        self.scroll = 0;
    }
}

/// Input dialog for text entry (playlist names, etc.).
#[derive(Debug, Clone)]
pub struct InputDialog {
    /// Dialog title
    pub title: String,
    /// Current input text
    pub input: SecretString,
    /// Action to dispatch on confirm
    pub action_type: InputDialogAction,
}

/// What action to take when input dialog is confirmed.
#[derive(Debug, Clone)]
pub enum InputDialogAction {
    SinglesTrackLimit,
    AudioMuseSearch(crate::audiomuse::Feature),
    SaveDescriptionPlaylist {
        library_key: String,
        tracks: Vec<Track>,
    },
    CreatePlaylist {
        generation: u64,
        addition: Option<super::sources::navidrome::playlist_edit::Addition>,
    },
    NavidromePlaylistName {
        id: String,
    },
    NavidromeName(String),
    FolderLocation,
    FolderName(String),
    SavePlaylist,
    ExportPlaylist {
        generation: u64,
        tracks: Vec<Track>,
        format: crate::services::playlist_export::Format,
    },
    AdventureLength,
}
impl InputDialogAction {
    pub fn submit_label(&self) -> &'static str {
        match self {
            Self::AudioMuseSearch(_) => "Search",
            Self::CreatePlaylist { .. } => "Create",
            Self::FolderLocation => "Continue",
            Self::AdventureLength => "Generate",
            Self::ExportPlaylist { .. } => "Export",
            _ => "Save",
        }
    }
}

/// Journey creation state.
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
    ArtistGenres,
    AlbumGenres,
    Moods,
    Styles,
    Decades,
    Years,
    Collections,
    Countries,
    Labels,
    Formats,
    Studios,
    Stations,
    AllTracks,
    Folders,
}

impl RefreshCategory {
    /// Album-artist browsing shares the same underlying artist request.
    pub fn canonical(self) -> Self {
        if self == Self::AlbumArtists {
            Self::Artists
        } else {
            self
        }
    }

    /// Get all categories in priority order.
    pub fn all() -> &'static [RefreshCategory] {
        &[
            RefreshCategory::Artists,
            RefreshCategory::AlbumArtists,
            RefreshCategory::Albums,
            RefreshCategory::Playlists,
            RefreshCategory::ArtistGenres,
            RefreshCategory::AlbumGenres,
            RefreshCategory::Moods,
            RefreshCategory::Styles,
            RefreshCategory::Decades,
            RefreshCategory::Years,
            RefreshCategory::Collections,
            RefreshCategory::Countries,
            RefreshCategory::Labels,
            RefreshCategory::Formats,
            RefreshCategory::Studios,
            RefreshCategory::Stations,
            RefreshCategory::AllTracks,
            RefreshCategory::Folders,
        ]
    }

    /// Get a stable key for serializing to disk cache.
    pub fn cache_key(&self) -> &'static str {
        self.display_name()
    }

    /// Get display name for status messages and toasts.
    pub fn display_name(&self) -> &'static str {
        match self {
            RefreshCategory::Artists => "Artists",
            RefreshCategory::AlbumArtists => "Album Artists",
            RefreshCategory::Albums => "Albums",
            RefreshCategory::Playlists => "Playlists",
            RefreshCategory::ArtistGenres => "Artist Genres",
            RefreshCategory::AlbumGenres => "Genres",
            RefreshCategory::Moods => "Moods",
            RefreshCategory::Styles => "Styles",
            RefreshCategory::Decades => "Decades",
            RefreshCategory::Years => "Years",
            RefreshCategory::Collections => "Collections",
            RefreshCategory::Countries => "Countries",
            RefreshCategory::Labels => "Labels",
            RefreshCategory::Formats => "Formats",
            RefreshCategory::Studios => "Studios",
            RefreshCategory::Stations => "Stations",
            RefreshCategory::AllTracks => "All Tracks",
            RefreshCategory::Folders => "Folders",
        }
    }

    /// Map a tag-style BrowseCategory to its RefreshCategory.
    pub fn for_tag_section(cat: BrowseCategory) -> Option<RefreshCategory> {
        Some(match cat {
            BrowseCategory::AlbumGenres => RefreshCategory::AlbumGenres,
            BrowseCategory::ArtistGenres => RefreshCategory::ArtistGenres,
            BrowseCategory::Moods => RefreshCategory::Moods,
            BrowseCategory::Styles => RefreshCategory::Styles,
            BrowseCategory::Decades => RefreshCategory::Decades,
            BrowseCategory::Years => RefreshCategory::Years,
            BrowseCategory::Collections => RefreshCategory::Collections,
            BrowseCategory::Countries => RefreshCategory::Countries,
            BrowseCategory::Labels => RefreshCategory::Labels,
            BrowseCategory::Formats => RefreshCategory::Formats,
            BrowseCategory::Studios => RefreshCategory::Studios,
            _ => return None,
        })
    }
}

/// Confirmation dialog for user prompts.
#[derive(Debug, Clone)]
pub struct ConfirmDialog {
    pub title: String,
    pub message: String,
    pub on_confirm: ConfirmAction,
    /// Which button is currently selected (true = Yes, false = No).
    pub selected_yes: bool,
}

/// Action to take when confirmation dialog is confirmed.
#[derive(Debug, Clone)]
pub enum ConfirmAction {
    NavidromeDeletePlaylist(String),
    NavidromeReplacePlaylist(String),
    RemoveNavidrome(String),
    RemoveFolder(String),

    RefreshCache,
    ClearLibraryCache,
    ClearSourceCache(crate::app::sources::LibraryChoice),
    ClearArtworkCache(crate::app::sources::LibraryChoice),

    Quit,
}

/// Inline list filter state (/ key in browse view).
#[derive(Debug, Clone)]
pub struct ListFilterState {
    pub active: bool,
    pub query: String,
    pub version: u64,
    epoch: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub loading: bool,
    pub results: Option<ListFilterResults>,
    /// Index into matched_indices (which filtered result is selected).
    pub selected: usize,
    /// Source index to retain when the records change without a query edit.
    pub refresh_anchor: Option<usize>,
    /// Which category the filter applies to.
    pub category: BrowseCategory,
    /// Which column index the filter applies to.
    pub column: usize,
}

impl Default for ListFilterState {
    fn default() -> Self {
        Self {
            active: false,
            query: String::new(),
            version: 0,
            epoch: Default::default(),
            loading: false,
            results: None,
            selected: 0,
            refresh_anchor: None,
            category: BrowseCategory::Library,
            column: 0,
        }
    }
}

impl ListFilterState {
    /// Invalidate pending workers without interrupting the application thread.
    pub fn advance_version(&mut self) {
        use std::sync::atomic::Ordering;
        self.version = self.epoch.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
    }

    /// A worker may finish only while it owns the current filter generation.
    pub(crate) fn cancellation(&self) -> impl FnMut() -> bool + Send + 'static {
        let epoch = self.epoch.clone();
        let version = self.version;
        move || epoch.load(std::sync::atomic::Ordering::Acquire) != version
    }

    /// Retain a jump's filter while cancelling its worker and sharing the generation counter.
    pub(crate) fn take_for_navigation(&mut self) -> Self {
        self.advance_version();
        let replacement = Self {
            version: self.version,
            epoch: self.epoch.clone(),
            ..Default::default()
        };
        std::mem::replace(self, replacement)
    }

    /// Deactivate the filter, clearing all state.
    pub fn deactivate(&mut self) {
        self.advance_version();
        self.active = false;
        self.query.clear();
        self.results = None;
        self.loading = false;
        self.selected = 0;
        self.refresh_anchor = None;
    }
}

pub use crate::services::ListFilterResults;

#[cfg(test)]
mod filter_generation_tests {
    use super::ListFilterState;
    #[test]
    fn restoring_a_saved_filter_never_revives_an_obsolete_worker() {
        let mut filter = ListFilterState::default();
        filter.advance_version();
        let mut saved = filter.clone();
        let mut old_work = filter.cancellation();
        filter.advance_version();
        let latest = filter.version;
        assert!(old_work());
        saved.advance_version();
        assert!(saved.version > latest);
        assert!(old_work());
        assert!(!saved.cancellation()());
    }
}

impl AppState {
    /// Artwork-independent analysis identity shared by foreground and speculative work.
    pub fn media_cache_scope(&self) -> String {
        format!(
            "{}\0{}",
            self.connected_server_url
                .as_deref()
                .unwrap_or("unknown-server"),
            self.active_library.as_deref().unwrap_or("unknown-library")
        )
    }
}
