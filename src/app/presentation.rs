//! Render feedback shared by the renderer and input reducer.
//! Rendering builds a fresh value; only the event loop installs it in AppState.

use ratatui::layout::Rect;

/// Registry of all clickable regions, populated each frame during render.
#[derive(Debug, Default, Clone)]
pub struct HitRegions {
    /// Visible overflowing titles retain their clock between terminal frames.
    pub header_clocks: Vec<(usize, String, usize, std::time::Instant)>,
    pub download_cancel: Vec<(Rect, String)>,
    pub download_status: Option<Rect>,
    /// Visible grapheme boundaries of the focused single-line editor.
    pub text_input: Option<TextInputRegion>,
    /// First visible display item of each rendered column.
    pub miller_offsets: Vec<(usize, usize)>,
    /// Visible artwork item bounds with their column and original item index.
    pub miller_item_rows: Vec<(usize, Rect, usize)>,
    pub artist_links: Vec<ArtistLink>,
    pub detail_links: Vec<DetailLink>,
    /// Full ribbon length and leftmost visible slot from the last frame.
    pub miller_viewport: Option<(usize, usize)>,
    /// Close affordance for the topmost terminal overlay.
    pub popup_close: Option<Rect>,
    pub popup_outer: Option<Rect>,
    pub motion_hidden: Vec<Rect>,
    pub player_overlay: Option<Rect>,
    pub player_toggle: Option<Rect>,
    pub description_input: Option<Rect>,
    pub sidebar_group: Option<(Rect, usize)>,
    pub description_save: Option<Rect>,
    pub fullscreen_modes: Vec<(Rect, usize)>,
    pub fullscreen_exit: Option<Rect>,
    pub settings_sections: Vec<(Rect, crate::app::state::SettingsSection)>,
    pub settings_pages: Vec<(Rect, crate::app::state::SettingsPage)>,
    pub settings_preview: Option<Rect>,
    pub settings_apply: Option<Rect>,
    pub settings_back: Option<Rect>,
    pub settings_categories: Option<Rect>,
    pub settings_about: Option<ScrollableTextRegion>,
    pub library_option_pages: Vec<(Rect, crate::app::sources::options::OptionsPage)>,
    pub library_option_scroll: usize,
    pub biography_radio: Option<Rect>,
    pub biography_google: Option<Rect>,
    pub biography_text: Option<ScrollableTextRegion>,
    pub settings_textamp: Option<SettingsContentRegion>,
    /// Actual wrapped help content and viewport, including split layouts.
    pub help: Option<ScrollableTextRegion>,
    // ── Modal popups (checked first, highest priority) ──────────────────────
    /// Confirm dialog (Yes/No buttons).
    pub confirm_dialog: Option<DialogRegions>,
    /// Ordered form controls / account action buttons.
    pub library_dialog: Vec<Rect>,

    /// Library picker (F3 popup).
    pub library_picker: Option<PopupListRegions>,
    pub library_manager_rows: Option<SettingsContentRegion>,
    pub library_manager_actions: Vec<(Rect, crate::app::Action)>,

    /// Search/filter popup (Ctrl+F overlay).
    pub search_popup: Option<SearchPopupRegions>,

    /// Command palette overlay (`:` palette).
    pub command_palette: Option<CommandPaletteRegions>,

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

    /// Navigation buttons on the fixed footer.
    pub footer_controls: Vec<(Rect, usize)>,
    pub description_submit: Option<Rect>,

    // ── Content (view-dependent, mutually exclusive) ────────────────────────
    /// Category selector column (Browse view, column 0).
    pub category_column: Option<CategoryColumnRegion>,
    /// Noninteractive Search landing panel, replacing the retained browse columns.
    pub search_landing: Option<Rect>,

    /// Browse alphabet jump strip (between category column and the
    /// first Miller column when on Library + sortable).
    pub alphabet_strip: Option<AlphabetStripRegions>,

    /// Miller columns (Browse view, content columns).
    pub miller_columns: Option<MillerRegions>,
    pub player_sync_column: Option<Rect>,
    pub player_sync_rows: Vec<(Rect, usize, crate::app::player_sync::Command)>,
    pub station_rows: Vec<(Rect, usize, String)>,
    pub station_play: Option<(Rect, String)>,
    pub queue_menus: Vec<(Rect, super::queue_controls::Menu)>,

    /// Track-details pane regions (right-side pane on Browse view
    /// when a Track row is focused). Holds the Play button rect
    /// and the per-similar-track row rects so the mouse handler
    /// can dispatch the right action.
    pub track_pane: Option<TrackPaneRegions>,
    pub sonic_details: Option<TrackPaneRegions>,

    /// Queue view content areas.
    pub queue_content: Option<QueueRegions>,

    /// Now Playing visualizer content areas.
    pub now_playing_content: Option<NowPlayingRegions>,
    pub graphical_visualizer: Option<Rect>,

    /// Similar popup (outer rect).
    pub similar_content: Option<SimilarRegions>,

    /// Related popup (outer rect).
    pub related_content: Option<RelatedRegions>,

    /// Niri-style horizontal scrollbar at the bottom of the Browse
    /// view in scrolling Miller-column mode. Populated when there
    /// are more ribbon slots than fit on screen; the mouse handler
    /// translates click / drag positions into a manual ribbon
    /// scroll position.
    pub miller_h_scrollbar: Option<MillerHScrollbar>,
}

/// Cell geometry for native input-method placement and pointer selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextInputRegion {
    pub target: crate::app::terminal_text::Target,
    pub area: Rect,
    /// UTF-16 document offset and terminal column, including the final caret.
    pub carets: Vec<(usize, u16)>,
}
impl TextInputRegion {
    /// Clamp a document position to the nearest visible grapheme boundary.
    pub fn column(&self, offset: usize) -> u16 {
        self.carets
            .iter()
            .min_by_key(|(index, _)| index.abs_diff(offset))
            .map_or(self.area.x, |(_, column)| *column)
    }
    /// Resolve a pointer column to the nearest visible document boundary.
    pub fn offset(&self, column: u16) -> usize {
        self.carets
            .iter()
            .min_by_key(|(_, x)| x.abs_diff(column))
            .map_or(0, |(index, _)| *index)
    }
}

/// A library destination bound to the source generation that rendered its label.
#[derive(Debug, Clone)]
pub struct ArtistLink {
    pub area: Rect,
    pub generation: u64,
    pub key: String,
    pub name: String,
}

impl ArtistLink {
    /// Resolve a visible name through the shared library-details navigation path.
    pub fn action(&self, generation: u64) -> Option<super::Action> {
        (self.generation == generation).then(|| {
            super::action::BrowseAction::OpenDetailsInLibrary {
                track: Box::new(crate::library::models::Track {
                    grandparent_rating_key: Some(self.key.clone()),
                    grandparent_title: Some(self.name.clone()),
                    ..Default::default()
                }),
                destination: super::library_details::Destination::Artist,
            }
            .into()
        })
    }
}

/// A rendered track or album name retaining its library and destination identity.
#[derive(Debug, Clone)]
pub struct DetailLink {
    pub area: Rect,
    pub generation: u64,
    pub track_key: String,
    pub playback_id: u64,
    pub destination: super::library_details::Destination,
}
impl DetailLink {
    /// Ignore stale render feedback after a library change.
    pub fn action(&self, state: &super::AppState) -> Option<super::Action> {
        (self.generation == state.library_generation
            && self.playback_id == state.playback.request_id)
            .then_some(())?;
        state
            .current_track()
            .filter(|track| track.rating_key == self.track_key)
            .map(|track| {
                super::action::BrowseAction::OpenDetailsInLibrary {
                    track: Box::new(track.clone()),
                    destination: self.destination,
                }
                .into()
            })
    }
}

/// Visible selectable rows in settings; headings and spacers have no hit target.
#[derive(Debug, Clone)]
pub struct SettingsContentRegion {
    pub inner: Rect,
    pub rows: Vec<(Rect, usize)>,
    pub scroll_offset: usize,
    pub total_lines: usize,
}

impl HitRegions {
    /// Reset all regions. Called at the start of each render frame.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

// ── Sub-structs ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ScrollableTextRegion {
    pub area: Rect,
    pub lines: usize,
    pub visible: usize,
}
impl ScrollableTextRegion {
    pub fn max_scroll(&self) -> u16 {
        self.lines
            .saturating_sub(self.visible)
            .min(u16::MAX as usize) as u16
    }
}

/// Horizontal scrollbar for the scrolling Miller-column ribbon.
/// `rail` is the full clickable strip (one row tall, full content
/// width). The thumb visually highlights the segment
/// `[thumb_x, thumb_x + thumb_w)` inside the rail.
#[derive(Debug, Clone)]
pub struct MillerHScrollbar {
    pub rail: Rect,
    pub thumb_x: u16,
    pub thumb_w: u16,
    /// Total ribbon slots — used to map a click x-position into a
    /// ribbon scroll offset (`(rel_x * total) / rail.width`).
    pub total: usize,
    pub visible: usize,
}

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

/// Command palette overlay regions. Clicking inside `outer` but not
/// on a `rows` rect is a no-op (keeps the palette open). Clicking
/// outside `outer` cancels (closes) the palette. Clicking a row
/// selects that row and executes its command.
#[derive(Debug, Clone)]
pub struct CommandPaletteRegions {
    /// Outer popup rect (border included).
    pub outer: Rect,
    /// One rect per visible row, paired with the index into
    /// `state.palette.matches` (NOT `state.palette.entries` —
    /// `matches` is the filtered slice the renderer iterates over).
    pub rows: Vec<(Rect, usize)>,
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

/// Single Journey form controls and visible endpoint/preview rows.
#[derive(Debug, Clone)]
pub struct AdventureLauncherRegions {
    pub outer: Rect,
    pub results: Rect,
    pub controls: Vec<(Rect, super::journey::JourneyControl)>,
    pub tabs: Vec<(Rect, super::state::SearchTab)>,
    pub rows: Vec<(Rect, usize)>,
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
    pub area: Rect,
    pub controls: Vec<(Rect, super::transport_controls::TransportControl)>,
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
/// rows. Index space matches `state.track_pane_index`: 0 and 1 are
/// playback buttons; 2..=N+1 are similar tracks in order.
#[derive(Debug, Clone)]
pub struct TrackPaneRegions {
    pub scroll: u16,
    pub max_scroll: u16,
    pub outer: Rect,
    pub play_button: Rect,
    pub add_button: Rect,
    pub next_button: Rect,
    /// (rect, index-into-similar-list) per visible similar track.
    pub similar_rows: Vec<(Rect, usize)>,
    /// Hit rect for the small "x" close glyph in the top-right
    /// corner. The pane is treated as a standard rightmost column
    /// for closing purposes — clicking the glyph clears the pane.
    pub close_x: Option<Rect>,
}

/// Category selector column region (Browse view, column 0).
#[derive(Debug, Clone)]
pub struct CategoryColumnRegion {
    pub scroll_offset: usize,
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
    /// Wrapped album metadata scroll limit, absent for item columns.
    pub details_max_scroll: Option<usize>,
    /// Which column index in the navigation this represents.
    pub col_idx: usize,
    /// Outer area (including border).
    pub area: Rect,
    /// Inner area (border excluded).
    pub inner: Rect,
    /// Rows per item (1 or 2). Meaningless when `is_art_mode` is true.
    pub rows_per_item: u16,
    /// Whether this column is in artwork grid mode.
    pub is_art_mode: bool,
    /// Hit rect for the small "x" close glyph in the top-right
    /// corner. `None` for the root column (which isn't closeable).
    /// A click anywhere in this rect closes this column and every
    /// column to its right.
    pub close_x: Option<Rect>,
    /// Artist Mix icon next to the column close control.
    /// Opens this column’s existing sort/artwork/grouping options.
    pub view_options: Option<Rect>,
    /// Terminal header controls, including the title/filter field.
    pub controls: Vec<(Rect, super::column_controls::ColumnControl)>,
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
    /// Track list area (middle column).
    pub track_list: Rect,
    /// Track list inner area (border excluded).
    pub track_list_inner: Rect,
    /// Artwork area (right column).
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
    /// Footer retry action, without a keyboard focus stop.
    pub retry_hint: Option<Rect>,
    /// Border enclosing the results list, separate from the whole popup.
    pub list_outer: Rect,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NpSidebarButton {
    AutoMix,
    Shuffle,
    Repeat,
    Twins,
    Twofer,
    Stretch,
    Doppelganger,
    OppositeDay,
    Enemies,
    Save,
    Undo,
    ClearQueue,
}

impl NpSidebarButton {
    pub fn label(self) -> &'static str {
        match self {
            Self::AutoMix => "Auto-Mix",
            Self::Shuffle => "Shuffle",
            Self::Repeat => "Repeat",
            Self::Twins => "Twins",
            Self::Twofer => "Twofer",
            Self::Stretch => "Stretch",
            Self::Doppelganger => "Doppelganger",
            Self::OppositeDay => "Opposite Day",
            Self::Enemies => "Enemies",
            Self::Save => "Save queue as playlist",
            Self::Undo => "Undo queue edit",
            Self::ClearQueue => "Clear Queue",
        }
    }
}

/// Canonical item-count wording for library summaries.
pub fn item_count_label(count: usize) -> String {
    format!("{count} {}", if count == 1 { "item" } else { "items" })
}

impl super::AppState {
    /// Visible item count for the current library destination.
    pub fn browse_item_count(&self) -> Option<usize> {
        use super::state::{TuiWorkspace, View};
        if self.view == View::NowPlaying
            || (self.view == View::Queue && self.tui_queue_workspace == TuiWorkspace::NowPlaying)
        {
            return None;
        }
        let (column, length) = self
            .browse_nav()
            .and_then(|nav| {
                nav.columns
                    .get(nav.focused_column)
                    .map(|column| (nav.focused_column, column.items.len()))
            })
            .or_else(|| {
                self.folder_state.as_ref().and_then(|nav| {
                    nav.columns
                        .get(nav.focused_column)
                        .map(|column| (nav.focused_column, column.items.len()))
                })
            })?;
        Some(
            self.list_filter
                .results
                .as_ref()
                .filter(|_| {
                    !self.list_filter.query.is_empty()
                        && self.list_filter.category == self.browse_category
                        && self.list_filter.column == column
                })
                .map_or(length, |results| results.matched_indices.len()),
        )
    }

    /// Current count, including saved-audio size in the Downloads destination.
    pub fn browse_item_summary(&self) -> Option<String> {
        self.browse_item_count().map(|count| {
            if self.desktop_library_root == super::state::LibraryRoot::Downloads {
                super::downloads::summary(self, count)
            } else {
                item_count_label(count)
            }
        })
    }
}

pub struct RenderFeedback {
    pub hit_regions: HitRegions,
    pub marquee: super::state::MarqueeState,
    pub marquee_subtitle: super::state::MarqueeState,
}

impl super::AppState {
    pub fn apply_render_feedback(&mut self, feedback: RenderFeedback) {
        if let Some((count, start)) = feedback.hit_regions.miller_viewport
            && (self.miller_reveal_right
                || self
                    .hit_regions
                    .miller_viewport
                    .is_some_and(|(previous, _)| count > previous))
        {
            self.miller_scroll_col = start;
            self.miller_scroll_manual = true;
        }
        self.miller_reveal_right = false;
        self.hit_regions = feedback.hit_regions;
        self.marquee = feedback.marquee;
        self.marquee_subtitle = feedback.marquee_subtitle;
    }
}

use crate::app::state::{AppState, BrowseCategory, BrowseItem, BrowseNavigationState};

pub fn is_two_row_browse_column(
    state: &AppState,
    col: &crate::app::state::BrowseColumn,
    col_idx: usize,
    nav: &BrowseNavigationState,
) -> bool {
    let first_is_track = col
        .items
        .first()
        .is_some_and(|item| matches!(item, BrowseItem::Track { .. }));
    let first_is_album = col
        .items
        .first()
        .is_some_and(|item| matches!(item, BrowseItem::Album { .. }));

    // Special track columns always get two-row display
    if first_is_track && state.is_special_track_column(nav, col_idx) {
        return true;
    }

    // Album columns in "All Artists" mode
    if first_is_album
        && (nav
            .columns
            .first()
            .and_then(|c| c.selected_item())
            .is_some_and(|item| matches!(item, BrowseItem::AllArtists))
            || (state.browse_category == BrowseCategory::Library
                && state.library.library_sub_mode != crate::app::state::LibrarySubMode::Normal
                && col_idx == 0))
    {
        return true;
    }

    // Genre/mood album columns
    if first_is_album && state.browse_category.is_tag_section() {
        return true;
    }

    if first_is_album && col.grouped_by_album {
        return true;
    }

    false
}
