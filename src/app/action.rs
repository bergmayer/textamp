//! Application actions.
//!
//! Actions are commands that modify state or trigger side effects.
//! Organized into sub-enums matching dispatch handler modules.

use crate::plex::models::Track;
use super::state::{BrowseCategory, View};

/// Top-level action routing — each variant maps to a dispatch handler module.
#[derive(Debug, Clone)]
pub enum Action {
    System(SystemAction),
    Navigation(NavigationAction),
    Data(DataAction),
    Miller(MillerAction),
    Playback(PlaybackAction),
    Queue(QueueAction),
    Search(SearchAction),
    Browse(BrowseAction),
    Folders(FolderAction),
    Radio(RadioAction),
    Settings(SettingsAction),
}

// ============================================================================
// Sub-enums
// ============================================================================

#[derive(Debug, Clone)]
pub enum SystemAction {
    Quit,
    ShowError(String),
    ClearError,
    SetStatus(String),
    ClearStatus,
    RefreshCategory(crate::app::state::RefreshCategory),
    CheckStaleness(crate::app::state::RefreshCategory),
    LoadArtwork,
    LoadWaveform,
    LoadSpectrogram,
    /// Load album art for a batch of albums: Vec<(rating_key, thumb_path)>
    LoadAlbumArt(Vec<(String, String)>),
    /// Open the system browser to a third-party music search (Apple
    /// Music / Spotify / YouTube). When `query` is None the dispatcher
    /// derives one from the currently selected artist/album/track (or
    /// the now-playing track) via `build_external_search_query`.
    /// Pass `Some(...)` from a right-click context menu so the search
    /// targets the right-clicked row even when it's not the focused
    /// one. The dispatcher also gates the action against the per-target
    /// "Search ⟨service⟩" toggle in `UiConfig` — disabled targets are
    /// silently skipped with a status notification.
    OpenExternalSearch {
        target: crate::services::external_search::SearchTarget,
        query: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub enum NavigationAction {
    SetView(View),
    NextView,
    PrevView,
    SetCategory(BrowseCategory),
    ToggleFocus,
}

#[derive(Debug, Clone)]
pub enum DataAction {
    LoadInitialData,
    LoadArtists,
    LoadPlaylists,
    LoadArtistAlbums,
    LoadArtistAllTracks,
    LoadSelectedAlbumTracks,
    LoadAlbumTracks { rating_key: String },
    LoadCategoryTracks,
    GoBackInRightPanel,
    LoadSimilarAlbums { rating_key: String, title: String },
    LoadSimilarTracks { rating_key: String, title: String },
    /// Lazy-load sonically similar tracks for the right-side
    /// track-details pane. Stores results in
    /// `state.track_pane_similar` keyed by `rating_key` without
    /// switching the view.
    LoadTrackPaneSimilar { rating_key: String },
    LoadSimilarArtists { artist_key: String, title: String },
    LoadRelated { artist_key: String, title: String },
    ListUp,
    ListDown,
    ListPageUp,
    ListPageDown,
    ListTop,
    ListBottom,
}

#[derive(Debug, Clone)]
pub enum MillerAction {
    LoadArtistAlbumsForMiller { artist_key: String },
    LoadAlbumTracksForMiller { album_key: String },
    LoadArtistAllTracksForMiller { artist_key: String },
    LoadAllAlbumsForMiller,
    PlayTrackFromMiller { column_index: usize, track_index: usize, single_track: bool },
    LoadGenreAlbumsForMiller { genre_key: String },
    LoadGenreTracksForMiller { album_key: String },
    PlayGenreTrackFromMiller { column_index: usize, track_index: usize, single_track: bool },
    LoadPlaylistTracksForMiller { playlist_key: String },
    /// Fetch the next page of a lazy-loaded playlist tracks column.
    /// `offset` is how many tracks the column already has; the server
    /// returns the next chunk after that.
    LoadMorePlaylistTracks { playlist_key: String, offset: u32 },
    PlayPlaylistTrackFromMiller { column_index: usize, track_index: usize, single_track: bool },
    RefreshAlbumTracks { album_key: String },
    LoadCompilationsForMiller,
    LoadCompilationAlbumsForMiller { artist_key: String, artist_name: String },
    LoadCompilationAllTracksForMiller { artist_key: String, artist_name: String },
    LoadAllCompilationTracksForMiller,
    LoadAllLibraryTracksForMiller,
}

#[derive(Debug, Clone)]
pub enum PlaybackAction {
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
    RetryCurrentTrack,
}

#[derive(Debug, Clone)]
pub enum QueueAction {
    PlayTrack(Track),
    PlayTrackFromCategory(usize),
    PlayAlbum { rating_key: String },
    PlayArtistTracks { artist_key: String },
    PlayTracksNow(Vec<Track>),
    PlaySearchResult,
    EnqueueSelection,
    EnqueueSelectionNext,
    EnqueueAlbum { rating_key: String, title: String },
    EnqueueArtistTracks { artist_key: String, artist_name: String },
    PlayAlbumNow { rating_key: String, title: String },
    PlayPlaylistNow { playlist_key: String, title: String },
    EnqueueTrack(Track),
    EnqueueSearchResult,
    EnqueueSearchResultNext,
    EnqueueAlbumNext { rating_key: String, title: String },
    EnqueueArtistTracksNext { artist_key: String, artist_name: String },
    EnqueueTracksNext(Vec<Track>),
    ClearQueue,
    RemoveFromQueue(usize),
    ToggleQueueShuffle,
    JumpToQueueIndex(usize),
    PromptSavePlaylist,
    SaveQueueAsPlaylist(String),
    RemixGemini,
    RemixTwofer,
    RemixStretch,
    RemixDoppelganger,
    RemixShuffle,
    RemixUndoShuffle,
    UndoLastRemix,
    MoveQueueTrackUp,
    MoveQueueTrackDown,
    /// Drag-and-drop reorder: pull the track at `from` and insert it at
    /// `to` (with `to` interpreted in the post-removal index space).
    MoveQueueTrack { from: usize, to: usize },
    MoveSelectedTracksUp,
    MoveSelectedTracksDown,
    RemoveSelectedFromQueue,
    RemixBatchReady(Vec<(usize, Vec<Track>)>),
    RemixDoppelgangerReady(Vec<(usize, Track)>),
}

#[derive(Debug, Clone)]
pub enum SearchAction {
    ExecuteLocalSearch,
    SelectSearchResult,
    ActivateListFilter,
    DeactivateListFilter,
    SelectFilteredItem,
    FilteredListUp,
    FilteredListDown,
    AppendListFilterChar(char),
    DeleteListFilterChar,
    /// Replace the list filter query with the given string in one shot
    /// and re-run the filter. Activates the filter if currently inactive;
    /// deactivates it when the new query is empty. Used by the GUI
    /// `text_input` (which delivers the full edited value on each edit)
    /// rather than the TUI's char-at-a-time path.
    SetListFilterQuery(String),
    OpenSearchPopup,
    CloseSearchPopup,
    /// Replace the global-search query (text input handler) and
    /// kick off a fresh `ExecuteLocalSearch`. Mirrors the way the
    /// adventure launcher's `AdventureLauncherSetQuery` works.
    SetSearchQuery(String),
    /// Switch the active result-category tab in the global search
    /// popup (Global / Artists / Albums / Tracks / Playlists / Genres).
    SetSearchTab(crate::app::state::SearchTab),
    CloseRadioLauncher,
    RadioLauncherSearch,
    RadioLauncherSelectResult,
    OpenAdventureLauncher,
    /// Open the Sonic Adventure launcher with a pre-selected start
    /// track — skips the start-track search step and goes straight to
    /// "enter track count". Used by the track context menu and the
    /// Now Playing "Adventure" action when the source already
    /// identifies a specific track.
    OpenAdventureLauncherWithStart { start_track: Box<crate::plex::models::Track> },
    CloseAdventureLauncher,
    AdventureLauncherSearch,
    AdventureLauncherDrillArtist { key: String, name: String },
    AdventureLauncherDrillAlbum { key: String, title: String, artist_name: String },
    AdventureLauncherSelectTrack,
    AdventureLauncherBack,
    /// Switch which adventure-launcher field is being edited
    /// (start track vs end track). The single-screen popup uses this
    /// to route the search panel's selections to the right slot.
    AdventureLauncherSetStep(crate::app::state::AdventureStep),
    /// Swap the launcher's `start_track` and `end_track` slots.
    /// Greyed out in the UI when both slots are empty (no-op then).
    AdventureLauncherReverse,
    /// Replace the track-count input with the given string. Used by
    /// the GUI text-input handler.
    AdventureLauncherSetTrackCount(String),
    /// Clear `start_track` (X-button on the start-track row).
    AdventureLauncherClearStart,
    /// Clear `end_track` (X-button on the end-track row).
    AdventureLauncherClearEnd,
    /// Fire generation when the user clicks the "Generate" button.
    /// Validates start/end/count and dispatches the same path the
    /// old multi-step launcher used on FindEndTrack select.
    AdventureLauncherGenerate,
    /// Replace the search query (TextInput handler) and re-run search.
    AdventureLauncherSetQuery(String),
    OpenLibraryPicker,
    CloseLibraryPicker,
    OpenSortPopup,
    CloseSortPopup,
    /// Menu-driven sort actions. These let the GUI expose sort controls
    /// in the menu bar without going through the modal popup. They
    /// always operate on the focused Miller column.
    ApplyFocusedSortMode(crate::app::state::ColumnSortMode),
    ReverseFocusedSortDirection,
    ToggleFocusedColumnArtwork,
    ToggleFocusedColumnGrouping,
    OpenArtistRadioPicker,
    CloseArtistRadioPicker,
    ArtistRadioPickerSearch,
    ArtistRadioPickerSetCount,
    ArtistRadioPickerToggleArtist,
    ArtistRadioPickerLaunch,
    ShowArtistBio { artist_key: String, artist_name: String },
}

#[derive(Debug, Clone)]
pub enum BrowseAction {
    LoadStations,
    /// Load tag-list data for a tag-style section (album genres, artist
    /// genres, moods, styles, decades, years, collections, countries,
    /// labels, formats, studios). The handler maps the section to the
    /// matching Plex client method.
    LoadTagList(crate::app::state::BrowseCategory),
    /// Load albums for the currently-selected tag in the active tag
    /// section (column 0 → column 1 drill).
    LoadTagAlbums,
    /// Populate the root column of `tag_nav` with the current section's
    /// tag list. Re-run on section switch.
    RefreshTagView,
    /// Open the track-details pane. The pane is a derived view of
    /// the currently-focused row, so this action carries no Track
    /// payload — the renderer reads `state.focused_track()` every
    /// frame. The handler simply flips `state.track_pane_open` to
    /// true and gives keyboard focus to the pane.
    OpenTrackDetails,
    /// GUI: close the track-details pane.
    CloseTrackDetails,
    /// "Open in Library": switch to the Library category, drill into
    /// the given artist, and (when `album_key` is `Some`) auto-select
    /// the album underneath. Used by the right-click "Open in Library"
    /// item from a track or album row in any view.
    OpenInLibrary {
        artist_key: String,
        artist_name: String,
        album_key: Option<String>,
        album_title: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub enum FolderAction {
    LoadFolderRoot,
    NavigateIntoFolder(String),
    PlayFolderTracks,
    PlayFolderTrack { track_index: usize },
    RefreshSubfolder(String),
}

#[derive(Debug, Clone)]
pub enum RadioAction {
    JumpToRadioTrack(usize),
    PlayCurrentRadioTrack,
    StartPlexRadio { key: String, title: String },
    PlayStation(String),
    DrillIntoStation(String, String),
    NavigateStationsBack,
    ToggleDjMode(crate::app::state::DjMode),
    DjModeProcess,
    DjModeTracksReady(Vec<Track>, bool, Option<String>),
    DjModeBatchReady(Vec<(usize, Vec<Track>)>),
}

#[derive(Debug, Clone)]
pub enum SettingsAction {
    Logout,
    AuthSignIn,
    AuthSelectServer,
    OpenSettings,
    SaveCredentials,
    SettingsSelect,
    SettingsSignIn,
    SelectServer(String),
    SelectLibrary(String),
    SelectLibraryOnServer(String, String),
    SaveSettings,
    ClearLibraryCache,
    ClearArtworkCache,
    ClearSubfolderCache,
    /// Recompute on-disk cache stats (library breakdown + artwork +
    /// waveform sizes) and post the results back via CacheEvent /
    /// ArtworkEvent. Used by the Settings popup to make sure the
    /// Cache tab shows fresh sizes the moment it opens.
    RefreshCacheStats,
    /// Wipe library + artwork caches, then trigger a fresh load from
    /// the server. The "Refresh all cache" button in Settings → Cache.
    RefreshAllCache,
    StartSubfolderCrawl,
    StopSubfolderCrawl,
    ToggleKeepSubfolderCache,
    DiscoverPlayers,
    SetOutputTarget(crate::app::state::OutputTarget),
    SetAdventureLength(usize),
    CancelAdventure,
    AdventureComplete(Vec<Track>),
    AdventureError(String),
    ArtistRadioComplete(Vec<Track>),
    /// Toggle whether the named external-search service is offered in
    /// the palette / context menu / menu bar. Mirrors `UiConfig`'s
    /// per-service flag onto `AppState::external_search` and persists
    /// the change to the config file. Sent by Settings checkboxes.
    ToggleExternalSearchService(crate::services::external_search::SearchTarget),
    /// Toggle whether a top-level browse section is shown in the
    /// leftmost browse column. Updates both `AppState::hidden_sections`
    /// and `UiConfig::hidden_sections` (persisted).
    ToggleSectionVisibility(crate::app::state::BrowseCategory),
    /// TUI-only: flip the Library Miller-column layout between
    /// shrinking (every column compressed to fit) and scrolling
    /// (each column at half-screen width, viewport scrolls as the
    /// user drills). Persisted via `UiConfig::miller_layout`. Bound
    /// to `\` in the browse view and a checkbox in Settings.
    ToggleMillerLayout,
    /// Persist the (library, playlist) -> view-toggles mapping. Sent
    /// when the user changes a playlist tracks column's "Group by
    /// album" or "Show album artwork" toggles. Settings disk-saver
    /// fires on every change so the toggles survive restart.
    SavePlaylistView {
        library_key: String,
        playlist_key: String,
        view: crate::config::settings::PlaylistView,
    },
    /// Drop saved per-playlist view-toggle entries for playlists that
    /// no longer exist in the given library. Sent after a successful
    /// `PlaylistsLoaded` so deleted playlists' settings don't
    /// accumulate forever in the config.
    PrunePlaylistViews {
        library_key: String,
        live_playlist_keys: std::collections::HashSet<String>,
    },
}

// ============================================================================
// From impls for ergonomic construction
// ============================================================================

impl From<SystemAction> for Action {
    fn from(a: SystemAction) -> Self { Action::System(a) }
}
impl From<NavigationAction> for Action {
    fn from(a: NavigationAction) -> Self { Action::Navigation(a) }
}
impl From<DataAction> for Action {
    fn from(a: DataAction) -> Self { Action::Data(a) }
}
impl From<MillerAction> for Action {
    fn from(a: MillerAction) -> Self { Action::Miller(a) }
}
impl From<PlaybackAction> for Action {
    fn from(a: PlaybackAction) -> Self { Action::Playback(a) }
}
impl From<QueueAction> for Action {
    fn from(a: QueueAction) -> Self { Action::Queue(a) }
}
impl From<SearchAction> for Action {
    fn from(a: SearchAction) -> Self { Action::Search(a) }
}
impl From<BrowseAction> for Action {
    fn from(a: BrowseAction) -> Self { Action::Browse(a) }
}
impl From<FolderAction> for Action {
    fn from(a: FolderAction) -> Self { Action::Folders(a) }
}
impl From<RadioAction> for Action {
    fn from(a: RadioAction) -> Self { Action::Radio(a) }
}
impl From<SettingsAction> for Action {
    fn from(a: SettingsAction) -> Self { Action::Settings(a) }
}
