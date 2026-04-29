//! Hit-test region registry.
//!
//! Populated during render, consumed by mouse_input handlers.
//! Eliminates duplicated layout calculations between render and click handling.
//!
//! Stored as `RefCell<HitRegions>` on `AppState` (matching the `RefCell<MarqueeState>` pattern).
//! Render populates it through `&AppState` immutable ref; mouse handler reads it then drops the
//! borrow before mutating state.

use ratatui::layout::Rect;

/// Registry of all clickable regions, populated each frame during render.
#[derive(Debug, Default, Clone)]
pub struct HitRegions {
    // ── Modal popups (checked first, highest priority) ──────────────────────

    /// Confirm dialog (Yes/No buttons).
    pub confirm_dialog: Option<DialogRegions>,

    /// Library picker (F3 popup).
    pub library_picker: Option<PopupListRegions>,

    /// Search/filter popup (Ctrl+F overlay).
    pub search_popup: Option<SearchPopupRegions>,

    /// Sort popup (Ctrl+S).
    pub sort_popup: Option<SortPopupRegions>,

    /// Artist radio picker popup.
    pub artist_radio_picker: Option<PopupListRegions>,

    /// Adventure launcher popup.
    pub adventure_launcher: Option<AdventureLauncherRegions>,

    // ── Chrome (always present when not in Auth view) ───────────────────────

    /// Tab bar (top row).
    pub tab_bar: Option<TabBarRegions>,

    /// Transport bar (playback controls, seek bar).
    pub transport: Option<TransportRegions>,

    /// Command/shortcut bar (bottom rows).
    pub command_bar: Option<CommandBarRegions>,

    // ── Content (view-dependent, mutually exclusive) ────────────────────────

    /// Category selector column (Browse view, column 0).
    pub category_column: Option<CategoryColumnRegion>,

    /// Browse alphabet jump strip (between category column and the
    /// first Miller column when on Library + sortable).
    pub alphabet_strip: Option<AlphabetStripRegions>,

    /// Miller columns (Browse view, content columns).
    pub miller_columns: Option<MillerRegions>,

    /// Track-details pane regions (right-side pane on Browse view
    /// when a Track row is focused). Holds the Play button rect
    /// and the per-similar-track row rects so the mouse handler
    /// can dispatch the right action.
    pub track_pane: Option<TrackPaneRegions>,

    /// Queue view content areas.
    pub queue_content: Option<QueueRegions>,

    /// Now Playing visualizer content areas.
    pub now_playing_content: Option<NowPlayingRegions>,

    /// Now Playing left sidebar buttons (Radio / DJ Modes / Remix /
    /// Clear Queue). Cleared and rebuilt every render of the queue
    /// view. Stored as a flat list of `(area, which)` pairs because
    /// the set is small and the click site only needs sequential
    /// hit-testing.
    pub now_playing_sidebar:
        Option<Vec<(Rect, crate::ui::screens::now_playing::NpSidebarButton)>>,

    /// Similar popup (outer rect).
    pub similar_content: Option<SimilarRegions>,

    /// Related popup (outer rect).
    pub related_content: Option<RelatedRegions>,

}

impl HitRegions {
    /// Reset all regions. Called at the start of each render frame.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

// ── Sub-structs ─────────────────────────────────────────────────────────────

/// Confirm dialog regions (Yes/No buttons).
#[derive(Debug, Clone)]
pub struct DialogRegions {
    pub outer: Rect,
    pub yes_button: Rect,
    pub no_button: Rect,
}

/// A simple popup with a list of items.
#[derive(Debug, Clone)]
pub struct PopupListRegions {
    /// Outer popup rect (border included).
    pub outer: Rect,
    /// Inner area where items are rendered.
    pub items_area: Rect,
    /// Number of items in the list.
    pub item_count: usize,
}

/// Search/filter popup regions.
#[derive(Debug, Clone)]
pub struct SearchPopupRegions {
    /// Outer popup rect.
    pub outer: Rect,
    /// Tab bar area (2 rows).
    pub tab_area: Rect,
    /// Search input area (3 rows with border).
    pub input_area: Rect,
    /// Results list area.
    pub results_area: Rect,
}

/// Sort popup regions.
#[derive(Debug, Clone)]
pub struct SortPopupRegions {
    /// Outer popup rect (border included).
    pub outer: Rect,
    /// Inner area where options are rendered.
    pub inner: Rect,
    /// Number of options.
    pub option_count: usize,
}

/// Adventure launcher popup regions (multi-step).
#[derive(Debug, Clone)]
pub struct AdventureLauncherRegions {
    /// Outer popup rect.
    pub outer: Rect,
    /// Inner area (border excluded).
    pub inner: Rect,
    /// Number of items currently visible in the results list.
    pub item_count: usize,
    /// Y offset where the results list starts (relative to inner.y).
    pub results_y_offset: u16,
}

/// Tab bar regions.
#[derive(Debug, Clone)]
pub struct TabBarRegions {
    /// Library name label (clickable to open library picker).
    pub library_label: Option<Rect>,
    /// Quit button.
    pub quit_button: Option<Rect>,
    /// Navigation tabs: (rect, tab_index).
    pub tabs: Vec<(Rect, usize)>,
}

/// Transport bar regions.
#[derive(Debug, Clone)]
pub struct TransportRegions {
    /// Play/pause button area.
    pub play_pause: Rect,
    /// Seek bar area.
    pub seekbar: Rect,
    /// Previous track button.
    pub prev_track: Rect,
    /// Next track button.
    pub next_track: Rect,
    /// Track info area (clickable to go to Now Playing).
    pub track_info: Option<Rect>,
    /// Search/filter icon area (left of speaker icon).
    pub search_icon: Option<Rect>,
    /// Speaker icon area (clickable to show volume slider).
    pub speaker_icon: Option<Rect>,
    /// Volume slider bar area (when visible).
    pub volume_slider: Option<Rect>,
}

/// Command/shortcut bar regions.
#[derive(Debug, Clone)]
pub struct CommandBarRegions {
    /// Top row items: (rect, action_key).
    pub top_row: Vec<(Rect, String)>,
    /// Bottom row items: (rect, action_key).
    pub bottom_row: Vec<(Rect, String)>,
}

/// Browse alphabet jump strip — one rect per visible letter, in
/// `ALPHABET_STRIP_LETTERS` order (or reversed for descending sort).
#[derive(Debug, Clone)]
pub struct AlphabetStripRegions {
    pub area: Rect,
    /// (rect, alphabet-strip index) per letter cell. The index is
    /// into `ALPHABET_STRIP_LETTERS`, not the rendered visual order
    /// — that way descending sort doesn't flip click handling.
    pub letters: Vec<(Rect, usize)>,
}

/// Track-details pane click regions — Play button + similar-track
/// rows. Index space matches `state.track_pane_index`: 0 is the
/// Play button, 1..=N are the similar tracks in order.
#[derive(Debug, Clone)]
pub struct TrackPaneRegions {
    pub outer: Rect,
    pub play_button: Rect,
    /// (rect, index-into-similar-list) per visible similar track.
    pub similar_rows: Vec<(Rect, usize)>,
}

/// Category selector column region (Browse view, column 0).
#[derive(Debug, Clone)]
pub struct CategoryColumnRegion {
    /// Outer area (including border).
    pub area: Rect,
    /// Inner area (border excluded).
    pub inner: Rect,
    /// Number of category items.
    pub item_count: usize,
}

/// A single Miller column's registered geometry.
#[derive(Debug, Clone)]
pub struct MillerColumnRegion {
    /// Which column index in the navigation this represents.
    pub col_idx: usize,
    /// Outer area (including border).
    pub area: Rect,
    /// Inner area (border excluded).
    pub inner: Rect,
    /// Rows per item (1 or 2).
    pub rows_per_item: u16,
    /// Whether this column is in artwork grid mode.
    pub is_art_mode: bool,
}

/// Miller columns layout regions.
#[derive(Debug, Clone)]
pub struct MillerRegions {
    /// Full area encompassing all columns.
    pub area: Rect,
    /// Per-column geometry.
    pub columns: Vec<MillerColumnRegion>,
}

/// Queue view regions.
#[derive(Debug, Clone)]
pub struct QueueRegions {
    /// Station panel area (left, below artwork).
    pub station_panel: Rect,
    /// Station panel inner area (border excluded).
    pub station_inner: Rect,
    /// Track list area (right side).
    pub track_list: Rect,
    /// Track list inner area (border excluded).
    pub track_list_inner: Rect,
    /// Artwork area (top-left).
    pub art_area: Rect,
}

/// Now Playing visualizer regions.
#[derive(Debug, Clone)]
pub struct NowPlayingRegions {
    /// Visualizer panel tab bar area (1 row for waveform/spectrum/spectrogram tabs).
    pub visualizer_tab_area: Rect,
    /// Visualizer panel content area (seekable waveform/spectrum/spectrogram area).
    pub visualizer_content_area: Rect,
}

/// Similar popup regions.
#[derive(Debug, Clone)]
pub struct SimilarRegions {
    /// Outer popup rect.
    pub outer: Rect,
    /// Inner content area (border excluded).
    pub inner: Rect,
    /// Rows per item (always 2 for similar view).
    pub rows_per_item: u16,
    /// Footer [Tab] hint clickable area.
    pub tab_hint: Option<Rect>,
}

/// Related popup regions.
#[derive(Debug, Clone)]
pub struct RelatedRegions {
    /// Outer popup rect.
    pub outer: Rect,
    /// Inner content area (border excluded).
    pub inner: Rect,
}
