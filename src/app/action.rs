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
    OpenSearchPopup,
    CloseSearchPopup,
    CloseRadioLauncher,
    RadioLauncherSearch,
    RadioLauncherSelectResult,
    OpenAdventureLauncher,
    CloseAdventureLauncher,
    AdventureLauncherSearch,
    AdventureLauncherDrillArtist { key: String, name: String },
    AdventureLauncherDrillAlbum { key: String, title: String, artist_name: String },
    AdventureLauncherSelectTrack,
    AdventureLauncherBack,
    OpenLibraryPicker,
    CloseLibraryPicker,
    OpenSortPopup,
    CloseSortPopup,
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
    LoadGenres,
    LoadArtistGenres,
    LoadAlbumGenres,
    LoadMoods,
    LoadStyles,
    LoadGenreAlbums,
    RefreshGenreView,
    CycleGenreTab,
    SetGenreTab(crate::app::state::GenreTab),
    DrillGenreCategory { category_key: String },
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
