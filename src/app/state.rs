//! Application state definitions.
//!
//! Uses the Elm Architecture pattern with a single state struct.
//! UI modeled after musikcube: Browse (left: categories, right: tracks), Queue, Search, etc.

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

use crate::services::{FolderNavigationState, WaveformData, MAX_HISTORY_SIZE};
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
    },
    Genre {
        key: String,
        title: String,
    },
    /// Genre category selector in column 0 (All, Artist, Album, Mood, Style).
    GenreCategory {
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
    /// every track by a particular artist. Was previously
    /// overloaded onto a single `artist_key: String` with sentinel
    /// prefixes (`__all_library__`, `__all_comp__`, `__comp_tracks:`),
    /// which silently misrouted on typos.
    AllTracks {
        scope: AllTracksScope,
        thumb: Option<String>,
    },
    /// "All Artists" entry - pinned at top of artist list, drills into all albums.
    AllArtists,
    /// "Artist Radio" entry - starts server radio seeded from this artist.
    ArtistRadio {
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
    pub fn key(&self) -> &str {
        match self {
            BrowseItem::Artist { key, .. } => key,
            BrowseItem::Album { key, .. } => key,
            BrowseItem::Track { key, .. } => key,
            BrowseItem::Genre { key, .. } => key,
            BrowseItem::GenreCategory { key, .. } => key,
            BrowseItem::Playlist { key, .. } => key,
            BrowseItem::AllTracks { scope, .. } => scope.key(),
            BrowseItem::AllArtists => "__all_artists__",
            BrowseItem::ArtistRadio { artist_key, .. } => artist_key,
            BrowseItem::Compilations => "__compilations__",
            BrowseItem::CompilationTracks { artist_key, .. } => artist_key,
        }
    }

    pub fn title(&self) -> &str {
        match self {
            BrowseItem::Artist { title, .. } => title,
            BrowseItem::Album { title, .. } => title,
            BrowseItem::Track { title, .. } => title,
            BrowseItem::Genre { title, .. } => title,
            BrowseItem::GenreCategory { title, .. } => title,
            BrowseItem::Playlist { title, .. } => title,
            BrowseItem::AllTracks { .. } => "All Tracks",
            BrowseItem::AllArtists => "All Artists",
            BrowseItem::ArtistRadio { .. } => "Artist Radio",
            BrowseItem::Compilations => "Compilations",
            BrowseItem::CompilationTracks { .. } => "Compilations",
        }
    }

    pub fn is_drillable(&self) -> bool {
        // AllTracks/Compilations/CompilationTracks are drillable, Track and ArtistRadio are not
        !matches!(
            self,
            BrowseItem::Track { .. } | BrowseItem::ArtistRadio { .. }
        )
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

    /// Build artist root items: pinned items at top, then artist items.
    /// `compilation_artist_keys` are hidden (they only appear on compilations).
    /// `has_compilations` adds a Compilations pinned item.
    pub fn artist_root_items(artists: &[Artist]) -> Vec<BrowseItem> {
        let mut items = vec![BrowseItem::AllArtists];
        items.extend(Self::from_artists(artists));
        items
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

/// Per-column sort mode (replaces global TrackViewMode and sorted_by_artist).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColumnSortMode {
    #[default]
    Default, // Alphabetical / track number / playlist order
    ByArtist,
    ByAlbum,    // Track columns: sort by album name
    ByTitle,    // Sort by title
    ByDuration, // Sort by duration
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
            (ColumnSortMode::Shuffled, _) => "shuffled",
        }
    }
}

/// Column type for determining available sort options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumnType {
    /// Artist root column: Default, Shuffled
    Artist,
    /// Album column (artist's albums, genre albums): Default, ByArtist, Shuffled
    Album,
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
            SortColumnType::Album => &[
                ColumnSortMode::Default,
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
            SortColumnType::Album => {
                if is_playlist {
                    "Title"
                } else {
                    "Year"
                }
            }
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
        let mut options = Vec::new();

        let default_label = column_type.default_label(is_playlist);
        let modes = column_type.available_modes();
        for mode in modes {
            options.push(SortPopupOption::SortMode(*mode));
        }

        // Direction option (available for all non-Shuffled modes)
        if !matches!(current_mode, ColumnSortMode::Shuffled) {
            options.push(SortPopupOption::Direction);
        }

        // Artwork toggle for album columns
        if matches!(column_type, SortColumnType::Album) {
            options.push(SortPopupOption::Artwork);
        }

        // Group by album toggle for playlist columns (available in both track and album views)
        if is_playlist {
            options.push(SortPopupOption::GroupByAlbum);
        }

        // Find the initial selection (match current sort mode)
        let initial = modes.iter().position(|m| *m == current_mode).unwrap_or(0);

        Self {
            selected_index: initial,
            column_idx,
            column_title,
            column_type,
            options,
            is_playlist,
            default_label,
        }
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

        if matches!(self.column_type, SortColumnType::Album) {
            self.options.push(SortPopupOption::Artwork);
        }

        // "Group by album" toggles a playlist's track list between
        // flat and album-grouped views. It's only meaningful on the
        // playlist's OWN track column — once the user has drilled
        // into one of those grouped albums, the child column shows
        // a single album's tracks (`SortColumnType::Track`) and
        // grouping again would be a no-op. Same logic for any other
        // single-album track column reached by drilling.
        if self.is_playlist
            && matches!(
                self.column_type,
                SortColumnType::AllTracks | SortColumnType::Album
            )
        {
            self.options.push(SortPopupOption::GroupByAlbum);
        }

        // Try to preserve selection
        if let Some(old) = old_selection {
            if let Some(pos) = self.options.iter().position(|o| *o == old) {
                self.selected_index = pos;
                return;
            }
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
    Album { rating_key: String, title: String },
    Playlist { rating_key: String, title: String },
    AllTracks { label: String },
}

impl PlayAllRow {
    /// Label shown on the synthetic row in the column (sans the
    /// leading "▶ " glyph, which the renderer prepends).
    pub fn label(&self) -> &str {
        match self {
            PlayAllRow::Album { .. } => "Play album",
            PlayAllRow::Playlist { .. } => "Play playlist",
            PlayAllRow::AllTracks { label } => label,
        }
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
    pub tracks: Vec<crate::library::models::Track>,
    /// Original items before shuffle/sort (None if in original order)
    original_items: Option<Vec<BrowseItem>>,
    /// Original tracks before shuffle/sort (None if in original order)
    original_tracks: Option<Vec<crate::library::models::Track>>,
    /// Per-column sort mode (replaces global track_view_mode and sorted_by_artist)
    pub sort_mode: ColumnSortMode,
    /// Sort direction: true = ascending (default)
    pub sort_ascending: bool,
    /// Album artwork visible (album columns only, replaces global album_art_view)
    pub artwork_visible: bool,
    /// Playlist tracks grouped by album
    pub grouped_by_album: bool,
    /// Album group indices into tracks (replaces global track_album_groups)
    pub album_groups: Option<Vec<Vec<usize>>>,
    /// When `Some`, the column is a tracks list with a synthetic
    /// "▶ Play …" row pinned above the real items. The TUI renders
    /// this as the column's first row; pressing Enter on it (or
    /// clicking it) plays the whole list. See `PlayAllRow` for the
    /// per-variant dispatch.
    pub play_all_row: Option<PlayAllRow>,
    /// True when the cursor is parked on the synthetic Play row
    /// (rather than one of the real `items`). Newly-pushed tracks
    /// columns default to this so the user can immediately press
    /// Enter to play the whole album / playlist. `↓` clears it and
    /// drops focus to `items[selected_index]`; `↑` from `items[0]`
    /// sets it again. Ignored when `play_all_row` is `None`.
    pub on_play_row: bool,
    /// Multi-select set: indices into `items` (and `tracks`) that the
    /// user has shift- or cmd-clicked. Empty means single-selection
    /// mode (the regular `selected_index` cursor). Populated by the
    /// GUI's miller-row click handler; the TUI doesn't currently
    /// emit multi-clicks but reading the set is harmless.
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
            artwork_visible: false,
            grouped_by_album: false,
            album_groups: None,
            play_all_row: None,
            on_play_row: false,
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
            artwork_visible: false,
            grouped_by_album: false,
            album_groups: None,
            play_all_row: None,
            on_play_row: false,
            selected_set: std::collections::BTreeSet::new(),
            selection_anchor: None,
            lazy: None,
        }
    }

    pub fn selected_item(&self) -> Option<&BrowseItem> {
        self.items.get(self.selected_index)
    }

    /// Whether this column is currently shuffled.
    pub fn is_shuffled(&self) -> bool {
        self.sort_mode == ColumnSortMode::Shuffled
    }

    /// Whether items are currently sorted by artist name.
    pub fn is_sorted_by_artist(&self) -> bool {
        self.sort_mode == ColumnSortMode::ByArtist
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

        // Count pinned items at start (AllArtists, AllTracks, Compilations, CompilationTracks, ArtistRadio)
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
            _ => (String::new(), None),
        });
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

    /// Apply a sort mode (unified dispatcher for sort popup).
    pub fn apply_sort(&mut self, mode: ColumnSortMode) {
        match mode {
            ColumnSortMode::Default => self.unshuffle(),
            ColumnSortMode::Shuffled => self.shuffle(),
            ColumnSortMode::ByArtist => self.sort_by_artist(),
            ColumnSortMode::ByTitle => self.sort_by_title(),
            ColumnSortMode::ByDuration => self.sort_by_duration(),
            ColumnSortMode::ByAlbum => self.sort_by_album(),
        }
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
                        | BrowseItem::ArtistRadio { .. }
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
            let album_key = track.parent_rating_key.clone().unwrap_or_default();
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
                thumb: first.parent_thumb.clone(),
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
            col.items = items;
            col.selected_index = old_idx.min(col.items.len().saturating_sub(1));
        } else {
            // No columns yet - initialize
            self.columns = vec![BrowseColumn::new(title, items)];
            self.focused_column = 0;
        }
        self.loading = false;
    }
}

/// Step in the multi-artist radio picker flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtistRadioPickerStep {
    /// Enter number of artists to blend
    EnterCount,
    /// Select artists from filtered list
    SelectArtists,
}

/// State for the multi-artist radio picker popup.
#[derive(Debug, Clone)]
pub struct ArtistRadioPickerState {
    pub step: ArtistRadioPickerStep,
    pub max_artists: usize,
    pub count_input: String,
    pub query: String,
    pub filtered_artists: Vec<Artist>,
    pub selected_artists: Vec<Artist>,
    pub focus: SearchFocus,
    pub item_index: usize,
    pub scroll_pin: Option<usize>,
}

/// State for the artist bio popup (F4).
#[derive(Debug, Clone)]
pub struct ArtistBioPopup {
    /// Artist name displayed in the title.
    pub artist_name: String,
    pub document: crate::services::biography::Biography,
    /// Scroll offset for long bios (clamped in render).
    pub scroll: u16,
    pub google_focused: bool,
    /// Loading state.
    pub loading: bool,
    pub image_index: usize,
    pub task: Option<crate::app::tasks::TaskLease>,
}

/// Snapshot of queue state for undo.
#[derive(Debug, Clone)]
pub struct QueueSnapshot {
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

/// How the Library screen's Miller columns share the horizontal
/// space when more than two are open. The GUI honours this in its
/// scrolling-mode renderer; the TUI uses it for the Niri-style
/// ribbon. Serialised lowercase in `config.toml` for human edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MillerLayoutMode {
    /// Every visible column shrinks to fit. The whole stack is always
    /// on screen.
    #[default]
    Shrinking,
    /// Every column keeps its starting width (half the inner area).
    /// The stack extends past the screen and the viewport scrolls as
    /// the user drills deeper. A scroll indicator hints at off-screen
    /// columns.
    Scrolling,
}

impl MillerLayoutMode {
    pub fn name(&self) -> &'static str {
        match self {
            MillerLayoutMode::Shrinking => "shrinking",
            MillerLayoutMode::Scrolling => "scrolling",
        }
    }

    pub fn toggled(&self) -> Self {
        match self {
            MillerLayoutMode::Shrinking => MillerLayoutMode::Scrolling,
            MillerLayoutMode::Scrolling => MillerLayoutMode::Shrinking,
        }
    }
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
#[derive(Debug, Clone, Default)]
pub struct Notifications {
    pub toast_message: Option<String>,
    pub toast_show_time: Option<std::time::Instant>,
    pub last_error: Option<String>,
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
    pub similar: Option<usize>,
    pub similar_click_time: Option<std::time::Instant>,
    pub related: Option<usize>,
    pub related_click_time: Option<std::time::Instant>,
    pub search_click_time: Option<std::time::Instant>,
    pub art_cooldown: Option<std::time::Instant>,
    pub scroll_cooldown: Option<std::time::Instant>,
    pub scrollbar_drag: Option<ScrollbarDrag>,
}

/// Popup state container.
#[derive(Debug, Clone, Default)]
pub struct Popups {
    pub sort: Option<SortPopupState>,
    pub adventure_launcher: Option<AdventureLauncherState>,
    pub artist_radio_picker: Option<ArtistRadioPickerState>,
    pub artist_bio: Option<ArtistBioPopup>,
    pub text: Option<TextPopup>,
    pub input_dialog: Option<InputDialog>,
    pub library_dialog: Option<crate::app::sources::dialogs::Dialog>,
    pub confirm_dialog: Option<ConfirmDialog>,
    pub library_picker_active: bool,
    pub library_picker_index: usize,
    pub search_active: bool,
}

impl Popups {
    /// Close all modal popups. Call before opening a new popup so only one
    /// is ever visible at a time.
    pub fn close_all(&mut self) {
        self.text = None;
        self.sort = None;
        self.adventure_launcher = None;
        self.artist_radio_picker = None;
        self.artist_bio = None;
        self.input_dialog = None;
        self.library_dialog = None;
        self.confirm_dialog = None;
        self.library_picker_active = false;
        self.search_active = false;
    }
}

/// Artwork state.
#[derive(Debug, Default)]
pub struct ArtworkState {
    pub current_thumb: Option<String>,
    pub current_data: Option<Vec<u8>>,
    pub loading: bool,
    /// Exact thumb path whose completion may replace `current_data`.
    pub pending_thumb: Option<String>,
    pub grid_cache: HashMap<String, Vec<u8>>,
    grid_cache_order: std::collections::VecDeque<String>,
    grid_cache_bytes: usize,
    pub grid_generation: u64,
    pub grid_pending: std::collections::HashSet<String>,
    pub cache_stats: Option<(usize, u64)>,
    pub default_visible: bool,
    pub mode: ArtworkMode,
    /// When true, `SystemAction::LoadAlbumArt` returns immediately
    /// without reading the disk cache or starting fetches. Both
    /// front-ends raise this while the user is rapidly navigating
    /// (keyboard scroll, mouse wheel, alphabet jumps) so the per-key
    /// disk I/O doesn't stall the UI. Once motion has been still for
    /// `ART_LOAD_PAUSE_MS` (1 s), the front-end's tick handler clears
    /// the flag and re-collects the viewport batch.
    pub suppress_loads: bool,

    /// Wall-clock instant of the most recent rapid-navigation gesture.
    /// `suppress_loads` clears once `last_motion_at.elapsed() >=
    /// ART_LOAD_PAUSE_MS`. `None` until the first motion event so the
    /// initial render isn't held back.
    pub last_motion_at: Option<std::time::Instant>,
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

    pub fn clear_grid_art(&mut self) {
        self.grid_generation = self.grid_generation.wrapping_add(1);
        self.grid_cache.clear();
        self.grid_cache_order.clear();
        self.grid_cache_bytes = 0;
    }
}

/// Idle threshold after which the lazy-art gate reopens. Tuned by the
/// user — short enough that the wait feels intentional, long enough to
/// absorb a held-down arrow key without flapping.
pub const ART_LOAD_PAUSE_MS: u64 = 1000;

/// DJ mode state (Guest DJ modes that modify queue behavior).
#[derive(Debug, Clone, Default)]
pub struct DjState {
    pub active_mode: Option<DjMode>,
    /// Track keys already inserted by DJ, to avoid repeats.
    pub history: Vec<String>,
    /// True while a DJ insert is in-flight (prevents duplicates).
    pub inserting: bool,
    /// True when the last track played was a DJ-inserted track.
    pub last_was_inserted: bool,
}

/// Similar content view state (server sonic similarity).
#[derive(Debug, Clone, Default)]
pub struct SimilarViewState {
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
    pub artists: Vec<Artist>,
    pub mode: SimilarMode,
    pub loading: bool,
    pub source_title: String,
    /// Identity of the in-flight request currently allowed to update this view.
    pub request_key: Option<String>,
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
    // Core collections
    pub artists: Vec<Artist>,
    pub artists_total: u32,
    pub artists_loading: bool,
    pub albums: Vec<Album>,
    pub albums_total: u32,
    pub albums_loading: bool,
    pub playlists: Vec<Playlist>,
    pub playlists_loading: bool,

    // All tracks cache (for compilation detection + track-level artist derivation)
    pub all_tracks: Vec<Track>,

    // Track-level artist list (derived from all_tracks original_title)
    pub track_artists: Vec<Artist>,

    // Compilation detection
    pub compilations: CompilationState,

    // Artist aliases (uniform track artists that differ from album artist)
    pub artist_aliases: std::collections::HashMap<String, std::collections::HashSet<String>>,
    pub album_display_artist: std::collections::HashMap<String, String>,

    // Tag-style lists (each is its own top-level section).
    // The legacy `genres` field has been dropped — Album Genres and
    // Library Genres hit the same server endpoint, so we only keep
    // album_genres.
    pub artist_genres: Vec<Genre>,
    pub album_genres: Vec<Genre>,
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

/// Queue and playback mode state.
#[derive(Debug, Default)]
pub struct QueueState {
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

/// Root application state.
#[derive(Debug)]
pub struct AppState {
    pub sources: crate::app::sources::SourceState,
    // Connection
    pub active_library: Option<String>,

    pub connected_server_url: Option<String>,

    pub connection_generation: u64,

    /// Monotonic identity of the selected server/library pair. Background
    /// results carrying an older generation are discarded centrally.
    pub library_generation: u64,

    // Authentication flow state

    // Navigation (musikcube-style)
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

    // Playlist tracks cache (playlist_key -> cached tracks with timestamp)

    // Artwork state
    pub artwork: ArtworkState,

    // Radio mode state (legacy)
    /// Transcode bitrate in kbps. 0 = disabled (direct play), e.g. 256 = transcode to 256kbps MP3.
    pub transcode_kbps: u32,

    /// Whether audio output is available. False when audio init failed/timed out.
    pub audio_available: bool,

    // NEW: Playback mode (Queue vs Radio)
    pub playback_mode: PlaybackMode,
    pub radio_generation: u64,
    /// The pending station or refill belongs to the current playback context.
    pub radio_task: Option<crate::app::tasks::TaskLease>,
    /// Candidate station title; active playback remains unchanged until it loads.
    pub station_starting: Option<StationStart>,
    pub station_navigation_generation: u64,

    // NEW: Radio playback state (continuous)
    pub radio: RadioPlaybackState,

    // NEW: Station navigation (hierarchical)
    pub station_nav: StationNavigationState,

    // Stations state (continuous radio stations) - legacy, use station_nav instead
    pub stations: Vec<Station>,
    pub stations_loading: bool,
    /// Cached station children (mood/style/decade sub-lists, keyed by station key).
    pub station_children_cache: std::collections::HashMap<String, Vec<Station>>,

    // Theme
    pub theme: ThemeName,

    /// TUI-only: tall-monitor split view. When true, the Library
    /// (Browse) screen renders in the top half and the Now Playing
    /// screen renders in the bottom half — both at once, instead of
    /// the usual Tab-toggled single view.
    pub tall_mode: bool,

    /// TUI-only: how the Library Miller columns share horizontal space.
    /// `Shrinking` (default) compresses every column to fit.
    /// `Scrolling` keeps each column at half-screen width and scrolls
    /// the viewport horizontally as the user drills.
    pub miller_layout: MillerLayoutMode,

    /// Manual horizontal scroll offset (in ribbon slots) for the Library
    /// screen when `miller_layout == Scrolling`. Only consulted when
    /// `miller_scroll_manual` is true — otherwise the renderer
    /// auto-anchors the ribbon to the focused column.
    pub miller_scroll_col: usize,
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

    // Sonic Adventure state
    pub adventure: AdventureState,
    pub adventure_request_id: u64,
    pub adventure_launcher_request_id: u64,
    pub artist_bio_request_id: u64,

    // DJ mode state (Guest DJ modes that modify queue behavior)
    pub dj: DjState,

    // Now Playing panel focus (track list vs stations)
    pub now_playing_focus: NowPlayingFocus,
    /// Index of the highlighted sidebar button on the now-playing
    /// screen (0..4). Only meaningful when `now_playing_focus ==
    /// Sidebar`.
    pub now_playing_sidebar_index: usize,

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
    /// navigates between the Play button (index 0) and the
    /// Sonically-Similar entries (indices 1..). Right or Left
    /// returns focus to the focused Miller column. Auto-cleared
    /// whenever the focused track changes or the user navigates
    /// away from the Browse view.
    pub track_pane_focused: bool,
    /// Index of the highlighted row inside the focused track-details
    /// pane: 0 = Play button, 1..=N = Sonically-Similar tracks.
    pub track_pane_index: usize,

    /// Live audio sample tap from the audio backend, drained each
    /// Tick into `vectorscope_buffer` for the vectorscope
    /// visualizer. `None` when no audio backend is available
    /// (`AudioPlayer::new_without_audio`).
    pub vectorscope_tap: Option<crate::audio::SampleTap>,
    /// Rolling stereo sample buffer for the TUI vectorscope
    /// (Lissajous XY trace). Capped at `VECTORSCOPE_BUFFER_LEN`
    /// — once full, oldest samples are overwritten in-place.
    pub vectorscope_buffer: std::collections::VecDeque<(f32, f32)>,
    /// Sampled PCM levels and bounded history for the optional Studio Meters view.
    pub studio_meters: crate::app::meters::StudioMeters,

    /// Per-tick counter, used to drive simple animated text in
    /// loading placeholders ("Loading", "Loading.", "Loading..",
    /// "Loading…"). Wraps freely; consumers use `% 4` etc.
    pub loading_tick: u32,

    // Visualizer tab (existing plots, spectral landscape, or studio meters)
    pub visualizer_tab: VisualizerTab,
    /// Whether the visualizer tab bar is focused (for arrow key navigation)
    pub visualizer_tab_focused: bool,

    // (Genre tab system removed — each tag type is now its own
    // top-level section in the browse category column.)

    // Cache management
    pub cache_mgmt: CacheManagement,

    // Waveform seekbar state
    pub waveform: WaveformState,

    // Spectrogram state
    pub spectrogram: SpectrogramState,

    // Inline list filter state (/ key in browse view)
    pub list_filter: ListFilterState,

    /// TUI command-palette overlay state (`:` to open). The field is
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

    // (default_artwork_visible, artwork_mode, album_art_cache, album_art_pending, artwork_cache_stats moved to artwork)
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

    /// Mirror of `config.ui.library_view_settings` so event handlers
    /// (which don't get `&Config`) can read saved per-playlist
    /// "Group by album" / "Show artwork" toggles when a playlist
    /// tracks column lands. The dispatcher keeps this in sync with
    /// the canonical config on every `SavePlaylistView` /
    /// `PrunePlaylistViews`.
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
            spotify: true,
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

/// Active DJ mode that modifies queue behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DjMode {
    /// Inserts a short Sonic Adventure between each pair of tracks
    Stretch,
    /// Inserts the most sonically similar track after each track
    Gemini,
    /// Keeps the mood going with sonically similar tracks to the current one
    Freeze,
    /// Inserts another track by the same artist after each track
    Twofer,
    /// Keeps the mood going with tracks from the same era
    Contempo,
    /// Keeps queueing tracks from the same artist
    Groupie,
    // Friendganger deferred — requires Sonic Analysis on shared libraries
}

impl DjMode {
    pub const ALL: [Self; 6] = [
        Self::Freeze,
        Self::Contempo,
        Self::Groupie,
        Self::Gemini,
        Self::Twofer,
        Self::Stretch,
    ];
    pub fn name(&self) -> &'static str {
        match self {
            DjMode::Stretch => "DJ Stretch",
            DjMode::Gemini => "DJ Gemini",
            DjMode::Freeze => "DJ Freeze",
            DjMode::Twofer => "DJ Twofer",
            DjMode::Contempo => "DJ Contempo",
            DjMode::Groupie => "DJ Groupie",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            DjMode::Stretch => "Inserts a sonic bridge between current and next",
            DjMode::Gemini => "Inserts a sonically similar track on each transition",
            DjMode::Freeze => "Keeps the mood going with sonically similar tracks",
            DjMode::Twofer => "Inserts a same-artist track when next differs",
            DjMode::Contempo => "Keeps the mood going with tracks from the same era",
            DjMode::Groupie => "Queues tracks from current and related artists",
        }
    }

    /// Number of tracks this mode inserts per transition.
    pub fn insert_count(&self) -> usize {
        match self {
            DjMode::Gemini | DjMode::Twofer | DjMode::Stretch => 1,
            DjMode::Freeze | DjMode::Contempo | DjMode::Groupie => 2,
        }
    }

    /// Interleaving modes insert between original queue tracks (alternate: original → DJ → original).
    /// Continuous modes insert after every track (original queue tracks get pushed down).
    pub fn is_interleaving(&self) -> bool {
        matches!(self, DjMode::Gemini | DjMode::Twofer | DjMode::Stretch)
    }

    pub fn key(&self) -> &'static str {
        match self {
            DjMode::Stretch => "dj:stretch",
            DjMode::Gemini => "dj:gemini",
            DjMode::Freeze => "dj:freeze",
            DjMode::Twofer => "dj:twofer",
            DjMode::Contempo => "dj:contempo",
            DjMode::Groupie => "dj:groupie",
        }
    }

    pub fn from_key(key: &str) -> Option<DjMode> {
        match key {
            "dj:stretch" => Some(DjMode::Stretch),
            "dj:gemini" => Some(DjMode::Gemini),
            "dj:freeze" => Some(DjMode::Freeze),
            "dj:twofer" => Some(DjMode::Twofer),
            "dj:contempo" => Some(DjMode::Contempo),
            "dj:groupie" => Some(DjMode::Groupie),
            _ => None,
        }
    }
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
        if from >= self.tracks.len() || from == to {
            return None;
        }
        let destination = to.min(self.tracks.len() - 1);
        // Rotate only the affected range. Adjacent keyboard moves stay O(1),
        // rather than shifting the entire queue twice through remove/insert.
        if from < destination {
            self.tracks[from..=destination].rotate_left(1);
        } else {
            self.tracks[destination..=from].rotate_right(1);
        }
        self.index = self.index.map(|index| {
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

    pub fn unshuffled_stations(&self) -> &[Station] {
        self.original_stations.as_deref().unwrap_or(&self.stations)
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

    /// Get the current title (focused column's title).
    pub fn current_title(&self) -> &str {
        self.focused()
            .map(|c| c.title.as_str())
            .unwrap_or("Stations")
    }

    /// Backward-compatible alias for `truncate_right()`.
    pub fn truncate_right_columns(&mut self) {
        self.truncate_right();
    }
}

impl AppState {
    fn loaded_stations(&self) -> impl Iterator<Item = &Station> {
        self.station_nav
            .columns
            .iter()
            .flat_map(|column| &column.stations)
            .chain(&self.stations)
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
        self.dj.inserting = false;
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
        self.radio_task = None;
        self.library_generation = self.library_generation.wrapping_add(1);
        self.sources.audiomuse.snapshot = None;
        self.sources.audiomuse.refresh_failed = false;
        self.cache_mgmt.next_refresh_check = None;
        self.sources.sonic_tasks.clear();
        self.station_starting = None;
        self.radio.refill = RadioRefill::Idle;
        self.stations_loading = false;
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
        self.popups.artist_bio = None;
    }

    /// Create a new application state with defaults.
    pub fn new() -> Self {
        Self {
            sources: Default::default(),

            active_library: None,

            connected_server_url: None,

            connection_generation: 0,

            library_generation: 0,

            view: View::Browse,
            previous_view: None,
            help_scroll: 0,
            browse_category: BrowseCategory::Library,
            focus: Focus::Left,
            category_column_focused: true,
            category_column_index: 2, // after Search and the Browse heading
            hidden_sections: BrowseCategory::hidden_by_default().to_vec(),
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
            stations: Vec::new(),
            stations_loading: false,
            station_children_cache: std::collections::HashMap::new(),
            theme: ThemeName::default(),
            tall_mode: false,
            miller_layout: MillerLayoutMode::default(),
            miller_scroll_col: 0,
            miller_scroll_manual: false,
            miller_h_drag_grab: None,
            adventure: AdventureState::default(),
            adventure_request_id: 0,
            adventure_launcher_request_id: 0,
            artist_bio_request_id: 0,
            dj: DjState::default(),
            now_playing_focus: NowPlayingFocus::default(),
            now_playing_sidebar_index: 0,
            alphabet_strip_focused: false,
            alphabet_strip_index: 0,
            track_pane_similar: HashMap::new(),
            track_pane_similar_loading: std::collections::HashSet::new(),
            select_mode: false,
            track_pane_focused: false,
            track_pane_index: 0,
            vectorscope_tap: None,
            studio_meters: crate::app::meters::StudioMeters::default(),
            vectorscope_buffer: std::collections::VecDeque::with_capacity(VECTORSCOPE_BUFFER_LEN),
            loading_tick: 0,
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

    /// Visible sidebar rows: Search, Browse (including system lists), AudioMuse, playlists.
    /// Rendering and input share this ordering; headings are not selectable.
    /// `category_column_index` indexes into this list, not the settings choices.
    pub fn category_rows(&self) -> Vec<CategoryRow> {
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
                    BrowseCategory::Library | BrowseCategory::AlbumGenres | BrowseCategory::Folders
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
    pub fn focused_track(&self) -> Option<&crate::library::models::Track> {
        let nav = self.browse_nav()?;
        let col = nav.columns.get(nav.focused_column)?;
        let item = col.items.get(col.selected_index)?;
        if !matches!(item, BrowseItem::Track { .. }) {
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
                        self.track_pane_index - 1,
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
        self.view == View::Browse
            && !self.category_column_focused
            && self.track_pane_focused
            && self.track_pane_index > 0
            && self
                .focused_track()
                .and_then(|p| self.track_pane_similar.get(&p.rating_key))
                .and_then(|result| result.as_ref().ok())
                .map(|v| v.get(self.track_pane_index - 1).is_some())
                .unwrap_or(false)
    }

    /// Build the album-artist list, separating confirmed compilations.
    pub fn build_artist_root_items(&self) -> Vec<BrowseItem> {
        BrowseItem::artist_root_items_with_compilations(
            &self.library.artists,
            !self.library.compilations.albums.is_empty(),
            &self.library.compilations.artist_keys,
        )
    }

    /// Switch to a new view, deactivating the inline filter and clearing queue multi-select.
    pub fn set_view(&mut self, view: View) {
        if self.view == view {
            return;
        }
        if self.list_filter.active {
            self.list_filter.deactivate();
        }
        if !matches!(view, View::Queue | View::NowPlaying) {
            self.queue.selected.clear();
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
        if cat == BrowseCategory::Library {
            if let Some(kind) = self.sources.nav_collection {
                return rows
                    .iter()
                    .position(
                        |row| matches!(row, CategoryRow::NavidromeCollection(k) if *k == kind),
                    )
                    .unwrap_or(fallback);
            }
        }
        rows.iter()
            .position(|r| matches!(r, CategoryRow::Category(c) if *c == cat))
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
        self.notifications.last_error = Some(msg);
    }

    /// Clear the current error.
    pub fn clear_error(&mut self) {
        self.notifications.last_error = None;
    }

    /// Set a status message (auto-clears after 5 seconds).
    pub fn set_status(&mut self, msg: String) {
        self.notifications.status_message = Some(msg);
        self.notifications.status_show_time = Some(std::time::Instant::now());
    }

    /// Clear the status message.
    pub fn clear_status(&mut self) {
        self.notifications.status_message = None;
        self.notifications.status_show_time = None;
    }

    /// Convert radio playback to queue mode, returning a snapshot for undo.
    pub fn convert_radio_to_queue(&mut self, description: &str) -> QueueSnapshot {
        let snapshot = QueueSnapshot {
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

        // Playlist track columns (always special)
        if self.browse_category == BrowseCategory::Playlists {
            return true;
        }

        // Check parent item for AllTracks or compilation album
        if col_idx > 0 {
            if let Some(parent_item) = nav.columns.get(col_idx - 1).and_then(|p| p.selected_item())
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
        }

        false
    }

    /// Set a toast notification (auto-clears after 5 seconds).
    pub fn set_toast(&mut self, msg: impl Into<String>) {
        self.notifications.toast_message = Some(msg.into());
        self.notifications.toast_show_time = Some(std::time::Instant::now());
    }

    /// Get the current notification to display (ongoing takes priority over toast).
    /// Returns None if no notification should be shown.
    pub fn current_notification(&self) -> Option<Notification> {
        // Priority 1: Adventure mode notifications (ongoing)
        if self.adventure.active && self.adventure.generating {
            return Some(Notification::ongoing("🌟 Generating sonic bridge..."));
        }

        // Priority 2: Library loading (ongoing)
        if self.library_loading {
            return Some(Notification::ongoing("Loading library..."));
        }

        // Priority 3: Preloads in progress (initial library data loading)

        // Priority 4: Station loading (ongoing)
        if self.station_nav.loading {
            return Some(Notification::ongoing("Loading station..."));
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
        if self.waveform.generating {
            return Some(Notification::ongoing("Generating waveform..."));
        }

        // Priority 7: Cache saving (ongoing)

        // Priority 8: Toast notifications (transient)
        if let Some(ref msg) = self.notifications.toast_message {
            return Some(Notification::toast(msg.clone()));
        }

        // Priority 9: Status messages (transient)
        if let Some(ref msg) = self.notifications.status_message {
            return Some(Notification::toast(msg.clone()));
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

    /// Get the currently playing track (mode-aware).
    pub fn current_track(&self) -> Option<&Track> {
        match self.playback_mode {
            PlaybackMode::Queue | PlaybackMode::None => {
                self.queue.index.and_then(|idx| self.queue.tracks.get(idx))
            }
            PlaybackMode::Radio => self.radio.current_track(),
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

    /// Whether the tag list for the active section is empty (used to
    /// decide whether to dispatch a load action when entering it).
    pub fn current_tag_list_is_empty(&self) -> bool {
        self.tag_list_for(self.browse_category).is_empty()
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

    /// Get the selected category item's title for display.
    pub fn selected_category_title(&self) -> Option<String> {
        match self.browse_category {
            BrowseCategory::Library => self
                .library
                .artists
                .get(self.list_state.artists_index)
                .map(|a| a.title.clone()),
            BrowseCategory::Playlists => self
                .library
                .playlists
                .get(self.list_state.playlists_index)
                .map(|p| p.title.clone()),
            BrowseCategory::Folders => None,
            cat if cat.is_tag_section() => {
                let list = self.tag_list_for(cat);
                let idx = self.category_index();
                list.get(idx).map(|g| g.title.clone())
            }
            _ => None,
        }
    }

    /// Add a track to play history.
    pub fn add_to_history(&mut self, track: Track) {
        // Don't add duplicates consecutively
        if self.queue.history.back().map(|t| &t.rating_key) == Some(&track.rating_key) {
            return;
        }
        self.queue.history.push_back(track);
        while self.queue.history.len() > MAX_HISTORY_SIZE {
            self.queue.history.pop_front();
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Current view (musikcube-style).
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
            VisualizerTab::Waveform => "waveform",
            VisualizerTab::Spectrum => "spectrum",
            VisualizerTab::Spectrogram => "spectrogram",
            VisualizerTab::Vectorscope => "vectorscope",
            VisualizerTab::Landscape => "spectral landscape",
            VisualizerTab::Meters => "studio meters",
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

cyclic_enum!(SearchTab, Global, Artists, Albums, Playlists, Tracks, Genres);

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
            BrowseCategory::AlbumGenres => "album genres",
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
            BrowseCategory::AlbumGenres => "Album Genres",
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
    Search,
    NavidromeCollection(crate::app::sources::navidrome::commands::CollectionKind),
    Category(BrowseCategory),
    Playlist(usize),
    /// A labelled separator row. Renders as chrome (no
    /// selection, skipped by Up/Down navigation, ignored by mouse).
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

/// TUI command-palette overlay state. `open == false` means we're in
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
    pub title: String,
    pub text: String,
    pub scroll: u16,
    pub request_id: u64,
}

/// The dispatchable command attached to a palette entry. Mirror of
/// `app::command_palette::PaletteCommand` but lives in shared state so
/// the GUI never needs to import the TUI module.
#[derive(Debug, Clone)]
pub enum PaletteCommandKind {
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
    OpenSimilar,
    OpenRelated,
    SaveQueue,
    ClearQueue,
    ToggleFilter,
    /// TUI-only: flip the tall-monitor split view (Library on top
    /// half, Now Playing on bottom half).
    ToggleTallMode,
    Refresh,
    PlayPause,
    StopPlayback,
    NextTrack,
    PrevTrack,
    ToggleDj(DjMode),
    RemixGemini,
    RemixTwofer,
    RemixStretch,
    RemixDoppelganger,
    RemixShuffle,
    RemixUndoShuffle,
    /// Start a station identified by its station URL, not an artist rating key.
    PlayStation(String),
    BrowseStations {
        key: String,
        title: String,
    },
    StationsBack,
    ArtistRadio,
    /// Pick one album at random from the active library and play it
    /// once (clear queue + load tracks). Distinct from
    /// "Random Album Radio" which is a continuous station.
    RandomAlbum,
    /// Drill into the currently-playing track's album in the Library
    /// view (so the user lands on it as if they'd browsed there).
    OpenInLibrary,
    /// Open the sort popup for the focused column.
    OpenSort,
    /// Toggle album-art tiles on the focused album column.
    ToggleArtwork,
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
    /// Open the Sonic Adventure launcher pre-seeded with the
    /// focused track as the starting song. Context-aware: surfaced
    /// when a Track row is focused (or a similar-track row is
    /// highlighted in the track-details pane). Distinct from the
    /// generic "Sonic Adventure" entry, which opens the launcher
    /// with no starting track and asks the user to pick one.
    SonicAdventureFromFocusedTrack,
    /// Generic Sonic Adventure entry-point — opens the launcher
    /// with no preselected starting track, so the user picks one
    /// inside the dialog. Always available via the static registry,
    /// menu bar, and keyboard shortcut.
    SonicAdventure,
    /// Carries a single shared `track_context::ContextKind` plus the
    /// track it targets. The palette materializer builds these from
    /// the shared `track_context_entries` list so the contextual
    /// section stays in lockstep with the GUI's right-click menu —
    /// adding or reordering entries in `services::track_context` is
    /// the only edit needed.
    FromTrackContext {
        kind: crate::services::track_context::ContextKind,
        track: Box<crate::library::models::Track>,
    },
    /// Open the F3 library-picker popup.
    SwitchLibrary,
    /// Toggle scrolling Miller layout (the `\` shortcut).
    ToggleScrollingMiller,
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

/// Focus within the Now Playing queue view. Sidebar covers the
/// left-hand action buttons (Radio / DJ Modes / Remix / Clear) so
/// the user can keyboard-navigate them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NowPlayingFocus {
    #[default]
    Tracks,
    Sidebar,
    /// The right-side artwork panel on the queue screen. Up/Down or
    /// Left/Right arrows can land here as the third stop in the
    /// horizontal sidebar→tracks→artwork progression. Enter opens
    /// the artist bio popup.
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
    /// True once server has accepted (or we have queued) the played/scrobble
    /// report for this track. Reset only when a new track starts.
    pub scrobble_reported: bool,
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
            scrobble_reported: false,
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
    /// Number of silent retries attempted for current track.
    pub retry_count: u8,
}

/// Spectrogram state for Now Playing visualizer.
#[derive(Debug, Clone, Default)]
pub struct SpectrogramState {
    /// Cached spectrogram data for current track.
    pub data: Option<crate::media::SpectrogramData>,
    /// Track key this spectrogram is for.
    pub track_key: Option<String>,
    /// Whether spectrogram is being generated.
    pub generating: bool,
    /// Error message if generation failed.
    pub error: Option<String>,
}

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
    EnterTrackCount,
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

/// Adventure launcher popup state. The original 3-step wizard
/// (FindStartTrack → EnterTrackCount → FindEndTrack) was replaced in
/// the GUI with a single-screen form that always shows all three
/// fields; `step` now indicates which field the user is currently
/// editing (so the search panel knows whether a chosen result becomes
/// `start_track` or `end_track`). Both tracks are stored separately
/// so a "Reverse" button can swap them and a "Generate" button can
/// fire only when both are set.
#[derive(Debug, Clone)]
pub struct AdventureLauncherState {
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
    Theme(crate::app::theme::ThemeName),
    Artwork(ArtworkMode),

    Transcode,
    ExternalSearch(crate::services::external_search::SearchTarget),
    Sidebar(SidebarSection),
}

impl AppState {
    pub fn textamp_settings(&self) -> Vec<TextampSetting> {
        use crate::services::external_search::SearchTarget;
        use TextampSetting::*;
        let mut items: Vec<_> = crate::app::theme::ThemeName::all()
            .iter()
            .copied()
            .map(Theme)
            .chain(ArtworkMode::all().iter().copied().map(Artwork))
            .collect();

        if self
            .sources
            .active
            .capabilities()
            .supports(crate::library::capabilities::Feature::Transcoding)
        {
            items.push(Transcode);
        }
        items.extend(
            [
                SearchTarget::AppleMusic,
                SearchTarget::Spotify,
                SearchTarget::YouTube,
            ]
            .map(ExternalSearch),
        );
        items.extend(self.sidebar_sections().into_iter().map(Sidebar));
        items
    }
}

/// Settings screen state.
#[derive(Debug, Clone, Default)]
pub struct SettingsState {
    pub cache_scans: std::collections::HashMap<String, crate::app::sources::cache::ScanState>,
    pub scan_request: u64,
    pub cache_request: u64,
    pub cache_entries: Vec<(crate::app::sources::LibraryChoice, Result<u64, String>)>,
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
    AudioMuseSearch(crate::audiomuse::Feature),
    NavidromePlaylistName { id: String },
    NavidromeName(String),
    FolderLocation,
    FolderName(String),
    SavePlaylist,
    AdventureLength,
}
impl InputDialogAction {
    pub fn submit_label(&self) -> &'static str {
        match self {
            Self::AudioMuseSearch(_) => "Search",
            Self::FolderLocation => "Continue",
            Self::AdventureLength => "Generate",
            _ => "Save",
        }
    }
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
    pub fn progress_label(&self) -> &'static str {
        if *self == Self::AllTracks {
            "Tracks"
        } else {
            self.cache_key()
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

    /// Whether this category belongs to the playlist/dynamic timestamp group.
    pub fn is_playlist_group(&self) -> bool {
        matches!(self, RefreshCategory::Playlists)
    }

    /// Get a stable key for serializing to disk cache.
    pub fn cache_key(&self) -> &'static str {
        self.display_name()
    }

    /// Look up a RefreshCategory from its cache key string.
    pub fn from_cache_key(key: &str) -> Option<Self> {
        RefreshCategory::all()
            .iter()
            .find(|c| c.cache_key() == key)
            .copied()
    }

    /// Get display name for status messages and toasts.
    pub fn display_name(&self) -> &'static str {
        match self {
            RefreshCategory::Artists => "Artists",
            RefreshCategory::AlbumArtists => "Album Artists",
            RefreshCategory::Albums => "Albums",
            RefreshCategory::Playlists => "Playlists",
            RefreshCategory::ArtistGenres => "Artist Genres",
            RefreshCategory::AlbumGenres => "Album Genres",
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
    ClearArtworkCache,

    Quit,
}

/// Inline list filter state (/ key in browse view).
#[derive(Debug, Clone)]
pub struct ListFilterState {
    pub active: bool,
    pub query: String,
    pub version: u64,
    pub loading: bool,
    pub results: Option<ListFilterResults>,
    /// Precomputed results for every visible Miller column. Renderers only
    /// slice these indices; they never rescan an entire library per frame.
    pub column_results: Vec<ListFilterResults>,
    /// Index into matched_indices (which filtered result is selected).
    pub selected: usize,
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
            loading: false,
            results: None,
            column_results: Vec::new(),
            selected: 0,
            category: BrowseCategory::Library,
            column: 0,
        }
    }
}

impl ListFilterState {
    /// Deactivate the filter, clearing all state.
    pub fn deactivate(&mut self) {
        self.version = self.version.wrapping_add(1);
        self.active = false;
        self.query.clear();
        self.results = None;
        self.column_results.clear();
        self.loading = false;
        self.selected = 0;
    }
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
