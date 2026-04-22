//! Application state definitions.
//!
//! Uses the Elm Architecture pattern with a single state struct.
//! UI modeled after musikcube: Browse (left: categories, right: tracks), Queue, Search, etc.

use std::collections::VecDeque;

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

use crate::plex::models::{Album, Artist, Genre, Library, Playlist, PlexServer, RemotePlayer, Station, Track, SearchResults};
use crate::miller::{MillerColumn, MillerState};
use crate::plex::{CachedFolder, CachedPlaylistTracks};
use crate::services::{FolderNavigationState, WaveformData, MAX_HISTORY_SIZE};
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
    /// Genre category selector in column 0 (All, Library, Artist, Album, Mood, Style).
    GenreCategory {
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
        thumb: Option<String>,
    },
    /// "All Artists" entry - pinned at top of artist list, drills into all albums.
    AllArtists,
    /// "Artist Radio" entry - starts Plex radio seeded from this artist.
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
            BrowseItem::AllTracks { artist_key, .. } => artist_key,
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
        !matches!(self, BrowseItem::Track { .. } | BrowseItem::ArtistRadio { .. })
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
        let mut items: Vec<BrowseItem> = artists.iter().map(|a| {
            let is_empty = a.title.is_empty();
            BrowseItem::Artist {
                key: a.rating_key.clone(),
                title: if is_empty { "Unknown Artist".to_string() } else { a.title.clone() },
                thumb: a.thumb.clone(),
                is_placeholder: is_empty,
            }
        }).collect();
        // Stable-partition: non-placeholders first, placeholders at end
        items.sort_by_key(|item| matches!(item, BrowseItem::Artist { is_placeholder: true, .. }));
        items
    }

    /// Convert a list of Albums to BrowseItems.
    /// Placeholder items (empty title → "Unknown Album (...)") are sorted to the end.
    /// If `album_display_artist` is provided, uses it to override the artist name
    /// when all tracks on a non-compilation album share a uniform track artist.
    pub fn from_albums(albums: &[Album], album_display_artist: &HashMap<String, String>) -> Vec<BrowseItem> {
        let mut items: Vec<BrowseItem> = albums.iter().map(|a| {
            let is_empty = a.title.is_empty();
            let display_artist = album_display_artist.get(&a.rating_key)
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
        }).collect();
        // Stable-partition: non-placeholders first, placeholders at end
        items.sort_by_key(|item| matches!(item, BrowseItem::Album { is_placeholder: true, .. }));
        items
    }

    /// Convert a list of Tracks to BrowseItems.
    pub fn from_tracks(tracks: &[Track]) -> Vec<BrowseItem> {
        tracks.iter().map(|t| {
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
    Default,      // Alphabetical / track number / playlist order
    ByArtist,
    ByAlbum,      // Track columns: sort by album name
    ByTitle,      // Sort by title
    ByDuration,   // Sort by duration
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
            SortColumnType::Album => &[ColumnSortMode::Default, ColumnSortMode::ByTitle, ColumnSortMode::ByArtist, ColumnSortMode::Shuffled],
            SortColumnType::Track => &[ColumnSortMode::Default, ColumnSortMode::ByTitle, ColumnSortMode::ByDuration, ColumnSortMode::Shuffled],
            SortColumnType::AllTracks => &[ColumnSortMode::Default, ColumnSortMode::ByArtist, ColumnSortMode::ByAlbum, ColumnSortMode::ByTitle, ColumnSortMode::ByDuration, ColumnSortMode::Shuffled],
        }
    }

    /// Context-specific label for the "Default" sort mode.
    pub fn default_label(&self, is_playlist: bool) -> &'static str {
        match self {
            SortColumnType::Artist => "Artist",
            SortColumnType::Album => if is_playlist { "Title" } else { "Year" },
            SortColumnType::Track => "Track #",
            SortColumnType::AllTracks => if is_playlist { "Playlist order" } else { "Library order" },
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
    pub fn new(column_idx: usize, column_title: String, column_type: SortColumnType, current_mode: ColumnSortMode, _artwork_visible: bool, is_playlist: bool) -> Self {
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

        if self.is_playlist {
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
    /// Original items before shuffle/sort (None if in original order)
    original_items: Option<Vec<BrowseItem>>,
    /// Original tracks before shuffle/sort (None if in original order)
    original_tracks: Option<Vec<crate::plex::models::Track>>,
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
            sort_mode: ColumnSortMode::Default,
            sort_ascending: true,
            artwork_visible: false,
            grouped_by_album: false,
            album_groups: None,
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
        self.original_tracks = if self.tracks.is_empty() { None } else { Some(self.tracks.clone()) };

        // Count pinned items at start (AllArtists, AllTracks, Compilations, CompilationTracks, ArtistRadio)
        let start = self.pinned_count();

        // Find placeholder items pinned at end
        let placeholder_start = self.items.iter().rposition(|item| !item.is_placeholder_item())
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
        if self.sort_mode == ColumnSortMode::ByArtist { return; }
        // Save originals if not already saved
        if self.original_items.is_none() {
            self.original_items = Some(self.items.clone());
            self.original_tracks = if self.tracks.is_empty() { None } else { Some(self.tracks.clone()) };
        }
        // Count how many pinned items are at the start
        let start = self.pinned_count();
        self.items[start..].sort_by_cached_key(|item| {
            match item {
                BrowseItem::Album { artist, year, .. } => (artist.to_lowercase(), *year),
                _ => (String::new(), None),
            }
        });
        self.sort_mode = ColumnSortMode::ByArtist;
        self.selected_index = 0;
    }

    /// Sort track items by title (case-insensitive).
    /// Saves originals for restore. Pinned items at start are excluded.
    pub fn sort_by_title(&mut self) {
        if self.sort_mode == ColumnSortMode::ByTitle { return; }
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
        if self.sort_mode == ColumnSortMode::ByDuration { return; }
        self.save_originals();
        let start = self.pinned_count();
        // Sort items
        self.items[start..].sort_by(|a, b| {
            let a_dur = if let BrowseItem::Track { duration_ms, .. } = a { *duration_ms } else { 0 };
            let b_dur = if let BrowseItem::Track { duration_ms, .. } = b { *duration_ms } else { 0 };
            a_dur.cmp(&b_dur)
        });
        // Sort tracks in parallel
        if start < self.tracks.len() {
            self.tracks[start..].sort_by(|a, b| {
                a.duration_ms().cmp(&b.duration_ms())
            });
        }
        self.sort_mode = ColumnSortMode::ByDuration;
        self.selected_index = 0;
    }

    /// Sort track items by album name (case-insensitive), then track number.
    /// Saves originals for restore.
    pub fn sort_by_album(&mut self) {
        if self.sort_mode == ColumnSortMode::ByAlbum { return; }
        self.save_originals();
        let start = self.pinned_count();
        // Sort items
        self.items[start..].sort_by_cached_key(|item| {
            match item {
                BrowseItem::Track { album_name, track_number, .. } => {
                    (album_name.as_deref().unwrap_or("").to_lowercase(), *track_number)
                }
                _ => (String::new(), None),
            }
        });
        // Sort tracks in parallel
        if start < self.tracks.len() {
            self.tracks[start..].sort_by_cached_key(|t| {
                (t.album_name().to_lowercase(), t.index)
            });
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
            self.original_tracks = if self.tracks.is_empty() { None } else { Some(self.tracks.clone()) };
        }
    }

    /// Count pinned items at the start of the column.
    pub fn pinned_count(&self) -> usize {
        self.items.iter().take_while(|item| {
            matches!(item, BrowseItem::AllArtists | BrowseItem::AllTracks { .. }
                | BrowseItem::ArtistRadio { .. } | BrowseItem::Compilations
                | BrowseItem::CompilationTracks { .. })
        }).count()
    }

    /// Group tracks by album for playlist columns.
    ///
    /// Saves originals, replaces items with BrowseItem::Album entries,
    /// and stores track index groups in `album_groups`.
    pub fn group_by_album(&mut self) {
        use std::collections::HashMap;

        if self.tracks.is_empty() { return; }

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
    /// Artist biography text.
    pub bio: String,
    /// Scroll offset for long bios (clamped in render).
    pub scroll: u16,
    /// Loading state.
    pub loading: bool,
    /// Artist artwork image data (fetched from Plex).
    pub artwork_data: Option<Vec<u8>>,
    /// Artist artwork thumb path (for cache keying).
    pub artwork_thumb: Option<String>,
}

/// Snapshot of queue state for undo.
#[derive(Debug, Clone)]
pub struct QueueSnapshot {
    pub queue: Vec<Track>,
    pub queue_index: Option<usize>,
    pub description: String,
    /// Saved radio state for undoing radio-to-queue conversion.
    pub radio_snapshot: Option<RadioPlaybackState>,
    /// Saved legacy RadioState (Alt+R) for undo.
    pub radio_state_snapshot: Option<RadioState>,
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
        &[ArtworkMode::Auto, ArtworkMode::Halfblocks, ArtworkMode::Braille]
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
    /// Per-category timestamps (Unix epoch secs) for when each category was last refreshed.
    pub category_timestamps: HashMap<RefreshCategory, u64>,
    pub dirty: bool,
    pub last_input_time: std::time::Instant,
    pub last_save: std::time::Instant,
    pub save_in_progress: bool,
    pub background_refresh: std::collections::HashSet<RefreshCategory>,
    /// Categories currently being preloaded from the server (initial load, not refresh).
    pub preloads_in_progress: std::collections::HashSet<String>,
    /// Total number of preloads started in the current batch (for progress display).
    pub preloads_total: usize,
}

impl Default for CacheManagement {
    fn default() -> Self {
        Self {
            category_timestamps: HashMap::new(),
            dirty: false,
            last_input_time: std::time::Instant::now(),
            last_save: std::time::Instant::now(),
            save_in_progress: false,
            background_refresh: std::collections::HashSet::new(),
            preloads_in_progress: std::collections::HashSet::new(),
            preloads_total: 0,
        }
    }
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
    pub browse: Option<(usize, usize)>,
    pub browse_click_time: Option<std::time::Instant>,
    /// Last clicked item in browse Miller columns: (col_idx, item_idx) for double-click detection.
    pub browse_last_click: Option<(usize, usize)>,
    pub search: Option<usize>,
    pub station: Option<usize>,
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
    pub station_back_highlighted: bool,
}

/// Popup state container.
#[derive(Debug, Clone, Default)]
pub struct Popups {
    pub sort: Option<SortPopupState>,
    pub radio_launcher: Option<RadioLauncherState>,
    pub adventure_launcher: Option<AdventureLauncherState>,
    pub artist_radio_picker: Option<ArtistRadioPickerState>,
    pub artist_bio: Option<ArtistBioPopup>,
    pub input_dialog: Option<InputDialog>,
    pub confirm_dialog: Option<ConfirmDialog>,
    pub library_picker_active: bool,
    pub library_picker_index: usize,
    pub search_active: bool,
}

impl Popups {
    /// Close all modal popups. Call before opening a new popup so only one
    /// is ever visible at a time.
    pub fn close_all(&mut self) {
        self.sort = None;
        self.radio_launcher = None;
        self.adventure_launcher = None;
        self.artist_radio_picker = None;
        self.artist_bio = None;
        self.input_dialog = None;
        self.confirm_dialog = None;
        self.library_picker_active = false;
        self.search_active = false;
    }
}

/// Artwork state.
#[derive(Debug)]
pub struct ArtworkState {
    pub current_thumb: Option<String>,
    pub current_data: Option<Vec<u8>>,
    pub loading: bool,
    pub grid_cache: HashMap<String, Vec<u8>>,
    pub grid_pending: std::collections::HashSet<String>,
    pub cache_stats: Option<(usize, u64)>,
    pub default_visible: bool,
    pub mode: ArtworkMode,
}

impl Default for ArtworkState {
    fn default() -> Self {
        Self {
            current_thumb: None,
            current_data: None,
            loading: false,
            grid_cache: HashMap::new(),
            grid_pending: std::collections::HashSet::new(),
            cache_stats: None,
            default_visible: false,
            mode: ArtworkMode::default(),
        }
    }
}

/// Remote player control state.
#[derive(Debug, Default)]
pub struct RemoteControl {
    pub output_target: OutputTarget,
    pub players: Vec<RemotePlayer>,
    pub discovering: bool,
    pub playback: RemotePlaybackState,
}

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

/// Similar content view state (Plex sonic similarity).
#[derive(Debug, Clone, Default)]
pub struct SimilarViewState {
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
    pub artists: Vec<Artist>,
    pub mode: SimilarMode,
    pub loading: bool,
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
    /// From Plex /related API.
    Plex,
    /// From Plex "Similar" metadata tags on the artist.
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

/// Compilation detection state.
#[derive(Debug, Clone, Default)]
pub struct CompilationState {
    /// Albums confirmed as true compilations (multi-artist).
    pub albums: Vec<Album>,
    /// Artist keys that appear ONLY on compilations (no solo albums) — hidden from artist list.
    pub artist_keys: std::collections::HashSet<String>,
    /// All artist keys that appear on any compilation track — used to show "Compilation Tracks" item.
    pub track_artist_keys: std::collections::HashSet<String>,
    /// Maps artist_key → Vec<album_rating_key> for compilation appearances.
    pub artist_map: std::collections::HashMap<String, Vec<String>>,
    /// Single-artist "compilations" (greatest hits etc.) mapped to their actual artist.
    /// Maps artist_key → Vec<Album> so they can appear as normal albums under that artist.
    pub single_artist: std::collections::HashMap<String, Vec<Album>>,
    /// Whether compilation detection has run for current library.
    pub detected: bool,
}

/// Library data — artists, albums, playlists, genres, and derived data.
///
/// Contains all cached data from the Plex library API, plus derived data
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

    // Genres, Artist Genres, Album Genres, Moods, and Styles
    pub genres: Vec<Genre>,
    pub artist_genres: Vec<Genre>,
    pub album_genres: Vec<Genre>,
    pub moods: Vec<Genre>,
    pub styles: Vec<Genre>,
    pub genres_loading: bool,
    pub artist_genres_loading: bool,
    pub album_genres_loading: bool,
    pub moods_loading: bool,
    pub styles_loading: bool,
    pub genres_index: usize,
    pub genre_content_type: GenreContentType,
    pub genre_albums: Vec<Album>,
    pub genre_albums_index: usize,

    // Library sub-mode for Alt+S cycling
    pub library_sub_mode: LibrarySubMode,

    // Right panel content
    pub right_panel_mode: RightPanelMode,
    pub selected_artist_albums: Vec<Album>,
    pub selected_album_tracks: Vec<Track>,
    pub selected_artist_name: String,
    pub selected_album_title: String,
    pub right_panel_loading: bool,
}

/// Search/filter state.
#[derive(Debug, Clone, Default)]
pub struct SearchState {
    pub query: String,
    pub results: Option<SearchResults>,
    pub track_loading: bool,
    pub track_version: u64,
    pub focus: SearchFocus,
    pub pending_album_key: Option<String>,
    pub pending_track_key: Option<String>,
    pub tab: SearchTab,
}

/// Queue and playback mode state.
#[derive(Debug)]
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

impl Default for QueueState {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            index: None,
            selected: std::collections::BTreeSet::new(),
            original: Vec::new(),
            sort_mode: QueueSortMode::default(),
            history: VecDeque::new(),
            undo_snapshot: None,
            shuffle_undo_queue: None,
            shuffle_undo_index: None,
        }
    }
}

/// Root application state.
#[derive(Debug)]
pub struct AppState {
    // Connection
    pub connection: ConnectionState,
    pub libraries: Vec<Library>,
    pub active_library: Option<String>,
    pub available_servers: Vec<PlexServer>,
    pub connected_server_url: Option<String>,

    /// Libraries from all available servers: (server_identifier, server_name, libraries).
    pub all_server_libraries: Vec<(String, String, Vec<Library>)>,
    pub active_server_id: Option<String>,

    // Authentication flow state
    pub auth_state: AuthState,
    pub is_fresh_login: bool,

    // Navigation (musikcube-style)
    pub view: View,
    pub previous_view: Option<View>,
    pub help_scroll: u16,
    pub browse_category: BrowseCategory,
    pub focus: Focus,

    // Category column (column 0 in Browse view)
    /// Whether the category selector column has keyboard focus.
    pub category_column_focused: bool,
    /// Selected index in the category column (0=Library, 1=Playlists, 2=Genres, 3=Folders).
    pub category_column_index: usize,

    // Library data (artists, albums, playlists, genres, etc.)
    pub library: LibraryData,

    // Similar content (Plex sonic similarity)
    pub similar: SimilarViewState,

    // Related artists (Ctrl+R)
    pub related: RelatedViewState,

    // Playback
    pub playback: PlaybackState,
    pub queue: QueueState,
    /// Whether user is currently dragging the seek indicator
    pub seeking_drag: bool,
    pub volume_drag: bool,
    /// Consecutive playback errors (for auto-skip with limit)
    pub consecutive_playback_errors: u32,
    /// Plex session identifier for timeline reporting.
    pub plex_session_id: Option<String>,
    /// Last time a progress report was sent to Plex (for periodic ~10s updates).
    pub last_progress_report: Option<std::time::Instant>,

    // Search
    pub search: SearchState,

    // UI state
    pub list_state: ListStates,
    pub should_quit: bool,
    /// Cache data built during quit, saved after terminal is restored.
    pub pending_cache_save: Option<crate::plex::CacheData>,
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
    /// Cached subfolder contents: folder_key -> CachedFolder with timestamp.
    /// Each entry has its own timestamp for individual staleness tracking.
    /// At 32+ days, entries are served as warm cache and re-fetched in background on access.
    /// Subfolders are only cached when navigated to (lazy caching).
    pub folder_contents_cache: HashMap<String, CachedFolder>,
    /// Folder key currently being loaded asynchronously (prevents duplicate spawns).
    pub pending_folder_load: Option<String>,
    /// Whether a subfolder preload crawl is currently active.
    pub subfolder_preload_active: bool,
    /// Cancel flag for the subfolder preload task (set on library switch).
    pub subfolder_preload_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Whether to keep subfolder cache entries indefinitely (per-library setting).
    pub keep_subfolder_cache: bool,

    // Miller column navigation for browse categories
    pub artist_nav: BrowseNavigationState,
    pub genre_nav: BrowseNavigationState,
    pub playlist_nav: BrowseNavigationState,

    // Playlist tracks cache (playlist_key -> cached tracks with timestamp)
    pub playlist_tracks_cache: HashMap<String, CachedPlaylistTracks>,

    // Artwork state
    pub artwork: ArtworkState,

    // Radio mode state (legacy)
    pub radio_state: RadioState,

    /// Transcode bitrate in kbps. 0 = disabled (direct play), e.g. 256 = transcode to 256kbps MP3.
    pub transcode_kbps: u32,

    /// Whether audio output is available. False when audio init failed/timed out.
    pub audio_available: bool,

    // NEW: Playback mode (Queue vs Radio)
    pub playback_mode: PlaybackMode,

    // NEW: Radio playback state (continuous)
    pub radio: RadioPlaybackState,

    // NEW: Station navigation (hierarchical)
    pub station_nav: StationNavigationState,

    // Stations state (Plexamp-style radio stations) - legacy, use station_nav instead
    pub stations: Vec<Station>,
    pub stations_loading: bool,
    /// Cached station children (mood/style/decade sub-lists, keyed by station key).
    pub station_children_cache: std::collections::HashMap<String, Vec<Station>>,

    // Theme
    pub theme: ThemeName,

    // Sonic Adventure state
    pub adventure: AdventureState,

    // DJ mode state (Guest DJ modes that modify queue behavior)
    pub dj: DjState,

    // Now Playing panel focus (track list vs stations)
    pub now_playing_focus: NowPlayingFocus,

    // Visualizer tab (Waveform / Spectrum / Spectrogram)
    pub visualizer_tab: VisualizerTab,
    /// Whether the visualizer tab bar is focused (for arrow key navigation)
    pub visualizer_tab_focused: bool,

    // Genre tab (All / Library / Artist / Album / Mood / Style)
    pub genre_tab: GenreTab,
    /// Whether the genre tab bar itself is focused (for arrow key navigation)
    pub genre_tab_focused: bool,

    // Cache management
    pub cache_mgmt: CacheManagement,

    // Waveform seekbar state
    pub waveform: WaveformState,

    // Spectrogram state
    pub spectrogram: SpectrogramState,

    // Inline list filter state (/ key in browse view)
    pub list_filter: ListFilterState,

    /// Auto-drill flag: when true, the next load action replaces the child column
    /// instead of pushing a new one, and does not change focus.
    pub auto_drill_pending: bool,

    // Marquee scroll animation state (RefCell for interior mutability during render)
    pub marquee: std::cell::RefCell<MarqueeState>,
    /// Second marquee for subtitle row (2-row track display in playlists)
    pub marquee_subtitle: std::cell::RefCell<MarqueeState>,

    /// Hit-test region registry (RefCell for interior mutability during render).
    /// Populated each frame by render code, consumed by mouse_input handlers.
    pub hit_regions: std::cell::RefCell<crate::ui::hit_regions::HitRegions>,

    // Library switch loading state
    pub library_loading: bool,

    // Remote player control
    pub remote: RemoteControl,

    // (default_artwork_visible, artwork_mode, album_art_cache, album_art_pending, artwork_cache_stats moved to artwork)
    /// Library cache total bytes on disk. Computed on startup and after clears.
    pub library_cache_stats: Option<(u64, Vec<(String, u64)>)>,
    /// Waveform cache stats: (file_count, total_bytes). Computed on startup and after clears.
    pub waveform_cache_stats: Option<(usize, u64)>,
    // Scroll pins and cooldowns
    pub scroll: ScrollPins,
}

/// Which view a scrollbar drag is operating on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollbarView {
    Browse,
    Folder,
    Queue,
    Station,
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

    /// Station nav column keys from root to playing station (for ancestor ♪ indicator).
    pub playing_station_ancestors: Vec<String>,
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
        self.playing_station_ancestors.clear();
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

    /// Get the current title (focused column's title).
    pub fn current_title(&self) -> &str {
        self.focused().map(|c| c.title.as_str()).unwrap_or("Stations")
    }

    /// Backward-compatible alias for `truncate_right()`.
    pub fn truncate_right_columns(&mut self) {
        self.truncate_right();
    }
}

// Radio state for Alt+R Plex radio, separate from station-based radio (via Radio section).
// radio_state is used for radio seeded from user selection via Plex playQueues API.
// This is distinct from RadioPlaybackState which is for Plexamp stations.
/// Radio mode for Plex radio (Alt+R).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RadioMode {
    #[default]
    Off,
    /// Active Plex radio — seeded from a track, album, or artist
    Active,
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
            connected_server_url: None,
            all_server_libraries: Vec::new(),
            active_server_id: None,
            auth_state: AuthState::default(),
            is_fresh_login: false,
            view: View::Auth,
            previous_view: None,
            help_scroll: 0,
            browse_category: BrowseCategory::Library,
            focus: Focus::Left,
            category_column_focused: true,
            category_column_index: 0,
            library: LibraryData::default(),
            similar: SimilarViewState::default(),
            related: RelatedViewState::default(),
            playback: PlaybackState::default(),
            queue: QueueState::default(),
            seeking_drag: false,
            volume_drag: false,
            consecutive_playback_errors: 0,
            plex_session_id: None,
            last_progress_report: None,
            search: SearchState::default(),
            list_state: ListStates::default(),
            should_quit: false,
            pending_cache_save: None,
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
            folder_contents_cache: HashMap::new(),
            pending_folder_load: None,
            subfolder_preload_active: false,
            subfolder_preload_cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            keep_subfolder_cache: false,
            artist_nav: BrowseNavigationState::new(),
            genre_nav: BrowseNavigationState::new(),
            playlist_nav: BrowseNavigationState::new(),
            playlist_tracks_cache: HashMap::new(),
            artwork: ArtworkState::default(),
            radio_state: RadioState::default(),
            transcode_kbps: 0,
            audio_available: true,
            playback_mode: PlaybackMode::None,
            radio: RadioPlaybackState::default(),
            station_nav: StationNavigationState::default(),
            stations: Vec::new(),
            stations_loading: false,
            station_children_cache: std::collections::HashMap::new(),
            theme: ThemeName::default(),
            adventure: AdventureState::default(),
            dj: DjState::default(),
            now_playing_focus: NowPlayingFocus::default(),
            visualizer_tab: VisualizerTab::default(),
            visualizer_tab_focused: false,
            genre_tab: GenreTab::default(),
            genre_tab_focused: false,
            cache_mgmt: CacheManagement::default(),
            waveform: WaveformState::default(),
            spectrogram: SpectrogramState::default(),
            list_filter: ListFilterState::default(),
            auto_drill_pending: false,
            marquee: std::cell::RefCell::new(MarqueeState::default()),
            marquee_subtitle: std::cell::RefCell::new(MarqueeState::default()),
            hit_regions: std::cell::RefCell::new(crate::ui::hit_regions::HitRegions::default()),
            library_loading: false,
            remote: RemoteControl::default(),
            library_cache_stats: None,
            waveform_cache_stats: None,
            scroll: ScrollPins::default(),
        }
    }

    /// Get the BrowseNavigationState for the current browse category.
    /// Returns None for Folders (which uses FolderNavigationState instead).
    pub fn browse_nav(&self) -> Option<&BrowseNavigationState> {
        match self.browse_category {
            BrowseCategory::Library => Some(&self.artist_nav),
            BrowseCategory::Genres => Some(&self.genre_nav),
            BrowseCategory::Playlists => Some(&self.playlist_nav),
            BrowseCategory::Folders => None,
        }
    }

    /// Get a mutable reference to the BrowseNavigationState for the current browse category.
    /// Returns None for Folders (which uses FolderNavigationState instead).
    pub fn browse_nav_mut(&mut self) -> Option<&mut BrowseNavigationState> {
        match self.browse_category {
            BrowseCategory::Library => Some(&mut self.artist_nav),
            BrowseCategory::Genres => Some(&mut self.genre_nav),
            BrowseCategory::Playlists => Some(&mut self.playlist_nav),
            BrowseCategory::Folders => None,
        }
    }

    /// Build artist root items, using compilation-aware version if compilations are detected.
    pub fn build_artist_root_items(&self) -> Vec<BrowseItem> {
        if self.library.compilations.detected {
            BrowseItem::artist_root_items_with_compilations(
                &self.library.artists,
                !self.library.compilations.albums.is_empty(),
                &self.library.compilations.artist_keys,
            )
        } else {
            BrowseItem::artist_root_items(&self.library.artists)
        }
    }

    /// Build track-level artist list from `all_tracks`.
    ///
    /// Scans all tracks, collects unique artist names from `original_title`
    /// (falling back to `grandparent_title`), and creates Artist entries.
    /// For names matching an existing Plex artist, uses that Artist.
    /// For others, creates a synthetic Artist entry.
    pub fn build_track_artists(&mut self) {
        use std::collections::HashMap;

        if self.library.all_tracks.is_empty() {
            return;
        }

        // Build name→Artist lookup from Plex artists (case-insensitive)
        let plex_artist_by_name: HashMap<String, &Artist> = self.library.artists.iter()
            .map(|a| (a.title.to_lowercase(), a))
            .collect();

        // Collect unique artist names from tracks
        let mut seen: HashMap<String, Artist> = HashMap::new();
        for track in &self.library.all_tracks {
            let artist_name = track.original_title.as_deref()
                .unwrap_or_else(|| track.artist_name());
            if artist_name.is_empty() {
                continue;
            }
            let key_lower = artist_name.to_lowercase();
            if seen.contains_key(&key_lower) {
                continue;
            }

            // Try to find matching Plex artist
            if let Some(plex_artist) = plex_artist_by_name.get(&key_lower) {
                seen.insert(key_lower, (*plex_artist).clone());
            } else {
                // Create synthetic artist entry
                // Use grandparent_rating_key if available, otherwise hash the name
                let rating_key = track.grandparent_rating_key.clone()
                    .unwrap_or_else(|| format!("track_artist:{}", key_lower));
                seen.insert(key_lower, Artist {
                    rating_key,
                    title: artist_name.to_string(),
                    thumb: None,
                    ..Artist::default()
                });
            }
        }

        let mut track_artists: Vec<Artist> = seen.into_values().collect();
        track_artists.sort_by(|a, b| {
            crate::app::handlers::helpers::sort_key(&a.title)
                .cmp(&crate::app::handlers::helpers::sort_key(&b.title))
        });
        self.library.track_artists = track_artists;
        tracing::info!("Built {} track-level artists from {} tracks", self.library.track_artists.len(), self.library.all_tracks.len());
    }

    /// Compute artist aliases from uniform track artists on non-compilation albums.
    ///
    /// When ALL tracks on a non-compilation album share the same `original_title` that
    /// differs from the album artist, that track artist is an alias of the album artist.
    /// Example: Robert Pollard (album artist) → "Guided by Voices" (track artist alias).
    pub fn build_artist_aliases(&mut self) {
        let (aliases, album_display) = crate::services::artist_alias_service::compute_aliases(
            &self.library.all_tracks,
            &self.library.albums,
        );

        let alias_count: usize = aliases.values().map(|s| s.len()).sum();
        tracing::info!(
            "Built {} artist aliases across {} artists, {} album display overrides",
            alias_count,
            aliases.len(),
            album_display.len(),
        );

        self.library.artist_aliases = aliases;
        self.library.album_display_artist = album_display;
    }

    /// Switch to a new view, deactivating the inline filter and clearing queue multi-select.
    pub fn set_view(&mut self, view: View) {
        if self.list_filter.active {
            self.list_filter.deactivate();
        }
        if view != View::Queue {
            self.queue.selected.clear();
        }
        self.view = view;
    }

    /// Set browse category, sync category_column_index, and unfocus category column.
    pub fn set_browse_category(&mut self, cat: BrowseCategory) {
        self.browse_category = cat;
        self.category_column_index = BrowseCategory::all().iter()
            .position(|c| *c == cat).unwrap_or(0);
        self.category_column_focused = false;
    }

    /// Focus the category column, syncing category_column_index to match
    /// the active browse_category so the highlight is always correct.
    pub fn focus_category_column(&mut self) {
        self.category_column_index = BrowseCategory::all().iter()
            .position(|c| *c == self.browse_category).unwrap_or(0);
        self.category_column_focused = true;
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
            queue: self.radio.tracks.clone(),
            queue_index: self.radio.track_index,
            description: description.to_string(),
            radio_snapshot: Some(self.radio.clone()),
            radio_state_snapshot: Some(self.radio_state.clone()),
        };
        // Take tracks from radio instead of cloning (avoids redundant allocation)
        self.queue.tracks = std::mem::take(&mut self.radio.tracks);
        self.queue.index = self.radio.track_index;
        self.playback_mode = PlaybackMode::Queue;
        if let Some(idx) = self.queue.index {
            self.list_state.queue_index = idx;
        }
        self.radio.clear();
        self.radio_state = RadioState::default();
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
        let first_is_track = col.items.first().map_or(false, |item| matches!(item, BrowseItem::Track { .. }));
        if !first_is_track {
            return false;
        }

        // Playlist track columns (always special)
        if self.browse_category == BrowseCategory::Playlists {
            return true;
        }

        // Check parent item for AllTracks or compilation album
        if col_idx > 0 {
            if let Some(parent_item) = nav.columns.get(col_idx - 1).and_then(|p| p.selected_item()) {
                match parent_item {
                    // Per-artist All Tracks, All Library Tracks, Compilation All Tracks
                    BrowseItem::AllTracks { .. } => return true,
                    // Compilation Tracks for a specific artist
                    BrowseItem::CompilationTracks { .. } => return true,
                    // Compilation album track column
                    BrowseItem::Album { key, .. } => {
                        if self.library.compilations.albums.iter().any(|a| a.rating_key == *key) {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
        }

        false
    }

    /// Whether multiple servers have music libraries available.
    pub fn has_multiple_servers(&self) -> bool {
        self.all_server_libraries.len() > 1
    }

    /// Get the server name for the currently active library.
    pub fn active_server_name(&self) -> Option<&str> {
        let server_id = self.active_server_id.as_ref()?;
        self.all_server_libraries.iter()
            .find(|(id, _, _)| id == server_id)
            .map(|(_, name, _)| name.as_str())
    }

    /// Get all music libraries across all servers, with server info.
    /// Returns: Vec<(server_id, server_name, library)>
    pub fn all_libraries_with_servers(&self) -> Vec<(&str, &str, &Library)> {
        self.all_server_libraries.iter()
            .flat_map(|(id, name, libs)| {
                libs.iter().map(move |lib| (id.as_str(), name.as_str(), lib))
            })
            .collect()
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
        if !self.cache_mgmt.preloads_in_progress.is_empty() {
            let done = self.cache_mgmt.preloads_total.saturating_sub(self.cache_mgmt.preloads_in_progress.len());
            let total = self.cache_mgmt.preloads_total;
            let msg = if total > 0 {
                format!("Loading library data ({}/{})...", done, total)
            } else {
                "Loading library data...".to_string()
            };
            return Some(Notification::ongoing(msg));
        }

        // Priority 4: Station loading (ongoing)
        if self.station_nav.loading {
            return Some(Notification::ongoing("Loading station..."));
        }

        // Priority 5: Background refresh (ongoing)
        if !self.cache_mgmt.background_refresh.is_empty() {
            let categories: Vec<_> = self.cache_mgmt.background_refresh
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
        if self.cache_mgmt.save_in_progress {
            return Some(Notification::ongoing("Saving cache..."));
        }

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

    /// Get the currently playing track (mode-aware).
    pub fn current_track(&self) -> Option<&Track> {
        match self.playback_mode {
            PlaybackMode::Queue | PlaybackMode::None => {
                self.queue.index.and_then(|idx| self.queue.tracks.get(idx))
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
            BrowseCategory::Library => self.library.artists.len(),
            BrowseCategory::Playlists => self.library.playlists.len(),
            BrowseCategory::Genres => self.current_genre_list().len(),
            BrowseCategory::Folders => 0, // Handled separately via folder_state
        }
    }

    /// Get the current genre list based on content type.
    pub fn current_genre_list(&self) -> &[Genre] {
        match self.library.genre_content_type {
            GenreContentType::Genres => &self.library.genres,
            GenreContentType::ArtistGenres => &self.library.artist_genres,
            GenreContentType::AlbumGenres => &self.library.album_genres,
            GenreContentType::Moods => &self.library.moods,
            GenreContentType::Styles => &self.library.styles,
        }
    }

    /// Get the current category index.
    pub fn category_index(&self) -> usize {
        match self.browse_category {
            BrowseCategory::Library => self.list_state.artists_index,
            BrowseCategory::Playlists => self.list_state.playlists_index,
            BrowseCategory::Genres => self.library.genres_index,
            BrowseCategory::Folders => 0, // Handled separately via folder_state
        }
    }

    /// Set the current category index.
    pub fn set_category_index(&mut self, idx: usize) {
        match self.browse_category {
            BrowseCategory::Library => self.list_state.artists_index = idx,
            BrowseCategory::Playlists => self.list_state.playlists_index = idx,
            BrowseCategory::Genres => self.library.genres_index = idx,
            BrowseCategory::Folders => {}, // Handled separately via folder_state
        }
    }

    /// Get the selected category item's rating key.
    pub fn selected_category_key(&self) -> Option<String> {
        match self.browse_category {
            BrowseCategory::Library => {
                self.library.artists.get(self.list_state.artists_index)
                    .map(|a| a.rating_key.clone())
            }
            BrowseCategory::Playlists => {
                self.library.playlists.get(self.list_state.playlists_index)
                    .map(|p| p.rating_key.clone())
            }
            BrowseCategory::Genres => self.current_genre_list().get(self.library.genres_index)
                .map(|g| g.effective_key().to_string()),
            BrowseCategory::Folders => None, // Handled separately via folder_state
        }
    }

    /// Get the selected category item's title for display.
    pub fn selected_category_title(&self) -> Option<String> {
        match self.browse_category {
            BrowseCategory::Library => {
                self.library.artists.get(self.list_state.artists_index)
                    .map(|a| a.title.clone())
            }
            BrowseCategory::Playlists => {
                self.library.playlists.get(self.list_state.playlists_index)
                    .map(|p| p.title.clone())
            }
            BrowseCategory::Genres => self.current_genre_list().get(self.library.genres_index)
                .map(|g| g.title.clone()),
            BrowseCategory::Folders => None, // Handled separately via folder_state
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
}

cyclic_enum!(VisualizerTab, Waveform, Spectrum, Spectrogram);

impl VisualizerTab {
    pub fn name(&self) -> &'static str {
        match self {
            VisualizerTab::Waveform => "waveform",
            VisualizerTab::Spectrum => "spectrum",
            VisualizerTab::Spectrogram => "spectrogram",
        }
    }
}

/// Genre tab for the genre category tab bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GenreTab {
    /// Merged list of all genre types with suffixes
    #[default]
    All,
    /// Library genres (actual tags from music files)
    Library,
    /// Artist-level genres (Plex-generated)
    Artist,
    /// Album-level genres (Plex-generated)
    Album,
    /// Moods (Plex analysis-based)
    Mood,
    /// Styles (Plex analysis-based)
    Style,
}

cyclic_enum!(GenreTab, All, Library, Artist, Album, Mood, Style);

impl GenreTab {
    pub fn name(&self) -> &'static str {
        match self {
            GenreTab::All => "genres",
            GenreTab::Library => "library genres",
            GenreTab::Artist => "artist genres",
            GenreTab::Album => "album genres",
            GenreTab::Mood => "moods",
            GenreTab::Style => "styles",
        }
    }

    /// Convert to the underlying GenreContentType (None for All tab).
    pub fn to_content_type(&self) -> Option<GenreContentType> {
        match self {
            GenreTab::All => None,
            GenreTab::Library => Some(GenreContentType::Genres),
            GenreTab::Artist => Some(GenreContentType::ArtistGenres),
            GenreTab::Album => Some(GenreContentType::AlbumGenres),
            GenreTab::Mood => Some(GenreContentType::Moods),
            GenreTab::Style => Some(GenreContentType::Styles),
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseCategory {
    Library,
    Playlists,
    Genres,
    Folders,
}

impl BrowseCategory {
    pub fn all() -> &'static [BrowseCategory] {
        &[
            BrowseCategory::Library,
            BrowseCategory::Playlists,
            BrowseCategory::Genres,
            BrowseCategory::Folders,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            BrowseCategory::Library => "library",
            BrowseCategory::Playlists => "playlists",
            BrowseCategory::Genres => "genres",
            BrowseCategory::Folders => "folders",
        }
    }

    pub fn shortcut(&self) -> char {
        match self {
            BrowseCategory::Library => 'l',
            BrowseCategory::Playlists => 'p',
            BrowseCategory::Genres => 'g',
            BrowseCategory::Folders => 'o',
        }
    }
}

/// Library sub-mode for Alt+S cycling: Normal → All Albums (by artist) → All Albums (shuffled).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibrarySubMode {
    #[default]
    Normal,        // Standard artist list with drill-down
    AllByArtist,   // All albums sorted by artist
    AllShuffled,   // All albums shuffled
}

cyclic_enum!(LibrarySubMode, Normal, AllByArtist, AllShuffled);


/// Genre content type - genres, normalized genres, moods, or styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GenreContentType {
    #[default]
    Genres,
    ArtistGenres,
    AlbumGenres,
    Moods,
    Styles,
}

cyclic_enum!(GenreContentType, Genres, ArtistGenres, AlbumGenres, Moods, Styles);

impl GenreContentType {
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

/// Focus within the Now Playing queue view (track list vs stations panel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NowPlayingFocus {
    #[default]
    Tracks,
    Stations,
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
    pub status: PlayStatus,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub volume: f32,
    pub muted: bool,
    /// When the current track transitioned to Playing (for grace period on TrackEnded detection).
    pub playback_started_at: Option<std::time::Instant>,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            status: PlayStatus::Stopped,
            position_ms: 0,
            duration_ms: 0,
            volume: 0.8,
            muted: false,
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
    pub data: Option<crate::plex::SpectrogramData>,
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

/// Radio launcher tab (artist-only radio).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RadioLauncherTab {
    #[default]
    All,
    Artists,
}

cyclic_enum!(RadioLauncherTab, All, Artists);

impl RadioLauncherTab {
    pub fn all() -> &'static [RadioLauncherTab] {
        Self::CYCLE_ORDER
    }

    pub fn name(&self) -> &'static str {
        match self {
            RadioLauncherTab::All => "All",
            RadioLauncherTab::Artists => "Artists",
        }
    }
}

/// Radio launcher popup state.
#[derive(Debug, Clone)]
pub struct RadioLauncherState {
    pub query: String,
    pub results: Option<SearchResults>,
    pub focus: SearchFocus,
    pub tab: RadioLauncherTab,
    pub item_index: usize,
    pub loading: bool,
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
    ArtistAlbums { artist_key: String, artist_name: String, albums: Vec<Album> },
    AlbumTracks { album_key: String, album_title: String, artist_name: String, tracks: Vec<Track> },
}

/// Adventure launcher popup state (3-step: find start → set count → find end).
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
    pub right_albums_index: usize,  // Albums in right panel (for artist drill-down)
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
    Account,
    Textamp,
    About,
}

impl SettingsSection {
    pub fn all() -> &'static [SettingsSection] {
        &[
            SettingsSection::Account,
            SettingsSection::Textamp,
            SettingsSection::About,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            SettingsSection::Account => "account",
            SettingsSection::Textamp => "textamp",
            SettingsSection::About => "about",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            SettingsSection::Account => SettingsSection::Textamp,
            SettingsSection::Textamp => SettingsSection::About,
            SettingsSection::About => SettingsSection::Account,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            SettingsSection::Account => SettingsSection::About,
            SettingsSection::Textamp => SettingsSection::Account,
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
    /// Scroll offset for the About section (display-only, no selectable items)
    pub scroll: u16,
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
    Genres,
    ArtistGenres,
    AlbumGenres,
    Moods,
    Styles,
    Stations,
    AllTracks,
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
            RefreshCategory::Genres,
            RefreshCategory::ArtistGenres,
            RefreshCategory::AlbumGenres,
            RefreshCategory::Moods,
            RefreshCategory::Styles,
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
        RefreshCategory::all().iter().find(|c| c.cache_key() == key).copied()
    }

    /// Get display name for status messages and toasts.
    pub fn display_name(&self) -> &'static str {
        match self {
            RefreshCategory::Artists => "Artists",
            RefreshCategory::AlbumArtists => "Album Artists",
            RefreshCategory::Albums => "Albums",
            RefreshCategory::Playlists => "Playlists",
            RefreshCategory::Genres => "Genres",
            RefreshCategory::ArtistGenres => "Artist Genres",
            RefreshCategory::AlbumGenres => "Album Genres",
            RefreshCategory::Moods => "Moods",
            RefreshCategory::Styles => "Styles",
            RefreshCategory::Stations => "Stations",
            RefreshCategory::AllTracks => "All Tracks",
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
    /// Which button is currently selected (true = Yes, false = No).
    pub selected_yes: bool,
}

/// Action to take when confirmation dialog is confirmed.
#[derive(Debug, Clone)]
pub enum ConfirmAction {
    RefreshCache,
    ClearLibraryCache,
    ClearArtworkCache,
    ClearSubfolderCache,
    Quit,
}

/// Output target for playback — Local (default) or Remote (Plex player device).
#[derive(Debug, Clone, Default)]
pub enum OutputTarget {
    #[default]
    Local,
    Remote {
        player_id: String,
        player_name: String,
        /// Direct URI for players that advertise on the local network (e.g. "http://192.168.1.5:32500").
        player_uri: Option<String>,
    },
}

/// State for tracking remote player playback (polling, position interpolation).
#[derive(Debug, Clone)]
pub struct RemotePlaybackState {
    /// Last time we polled the remote player for status.
    pub last_poll: Option<std::time::Instant>,
    /// Track key reported by the remote player (for detecting track changes).
    pub current_track_key: Option<String>,
    /// Position baseline from the last successful poll (ms).
    pub baseline_position: u64,
    /// When the baseline was set — used to interpolate smoothly between polls.
    pub baseline_time: Option<std::time::Instant>,
}

impl Default for RemotePlaybackState {
    fn default() -> Self {
        Self {
            last_poll: None,
            current_track_key: None,
            baseline_position: 0,
            baseline_time: None,
        }
    }
}

/// Inline list filter state (/ key in browse view).
#[derive(Debug, Clone)]
pub struct ListFilterState {
    pub active: bool,
    pub query: String,
    pub version: u64,
    pub loading: bool,
    pub results: Option<ListFilterResults>,
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
            selected: 0,
            category: BrowseCategory::Library,
            column: 0,
        }
    }
}

impl ListFilterState {
    /// Deactivate the filter, clearing all state.
    pub fn deactivate(&mut self) {
        self.active = false;
        self.query.clear();
        self.results = None;
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
