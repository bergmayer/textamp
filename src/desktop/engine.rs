//! Serialized application owner. GPUI never borrows mutable domain state.
use super::catalog::ControlTooltip as _;
use crate::app::{self, Action, AppState, action::SystemAction, state::PaletteCommandKind};
use std::sync::mpsc;
use tokio::sync::{mpsc as channel, watch};

/// The catalog cache is rebuilt only when its content or selection changes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ProjectionChange {
    Content,
    Selection,
    Playback,
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub activity_history: Option<std::sync::Arc<str>>,
    pub player_sync: app::player_sync::View,
    pub presentation: app::windowed_presentation::Presentation,
    pub presentation_anchors: Option<app::view_continuity::BrowseAnchors>,
    pub browse_category: Option<app::BrowseCategory>,
    #[cfg(target_os = "macos")]
    pub text_frame: Option<std::sync::Arc<crate::text_mode::PresentedFrame>>,
    pub separate_player: bool,
    pub singles_track_limit: u32,
    pub view_options: super::commands::ViewOptions,
    pub disabled_queue_menu_items: Vec<&'static str>,
    pub queue_tools: Vec<app::queue_controls::Definition>,
    pub queue_mode: String,
    pub queue_modes: Vec<app::queue_modes::Choice>,
    pub recent_commands: app::command_history::History,
    pub sidebar_cursor: Option<usize>,
    pub collapsed_sidebar_sections: Vec<String>,
    pub filter: app::state::ListFilterState,
    pub sidebar_options: Vec<super::preferences::SidebarOption>,
    pub caches: Vec<app::sources::cache::CacheLibrary>,
    pub status: String,
    pub download_notification: bool,
    pub error: Option<String>,
    pub loading: Option<String>,
    pub radio_commands: Vec<String>,
    pub menu_commands: Vec<super::menu_catalog::Command>,
    pub settings_request: u64,
    pub player_window_request: u64,
    pub queue_open: bool,
    pub sidebar_visible: Option<bool>,
    pub queue_request: u64,
    pub library_window_request: u64,
    pub surface_request: u64,
    pub connection: Option<super::connection::Status>,
    pub settings_progress: Option<app::settings_transfer::Progress>,
    pub columns: Vec<super::browser::Column>,
    pub playback_queue: Option<super::browser::Column>,
    pub preview: Option<super::catalog::Preview>,
    pub details_focused: bool,
    pub sidebar: Vec<super::catalog::SidebarRow>,
    pub surface: Option<super::surface::Surface>,
    pub radio_browser: Option<super::surface::Surface>,
    pub workspace: Option<super::surface::Surface>,
    pub workspace_title: Option<String>,
    pub surfaces: std::sync::Arc<Vec<super::surface::Surface>>,
    pub player: super::player_model::Player,
    pub preferences: Vec<super::preferences::Page>,
    pub theme: crate::app::theme::ThemeName,
    pub artwork_mode: app::state::ArtworkMode,
    pub ghost_artist: Option<String>,
    pub split_player: bool,
    pub context_policy: crate::app::track_context::ContextPolicy,
    pub now_playing: String,
    pub generation: u64,
    pub libraries: Vec<app::sources::LibraryChoice>,
    pub analysis_connections: std::collections::HashMap<String, crate::audiomuse::Connection>,
    pub section: String,
    pub navigation_serial: u64,
    pub jump_head: Option<u64>,
    pub jump_restore: Option<u64>,
    pub player_navigation_serial: u64,
    pub navigation_origin: super::browser::Origin,
    pub playback_id: u64,
    pub volume: f32,
    pub muted: bool,
    pub seekable: bool,
    pub artwork: Option<std::sync::Arc<Vec<u8>>>,
    pub images: super::artwork::Images,
    pub art_generation: u64,
    pub focused_column: usize,
    pub item_summary: Option<String>,
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub artist: String,
}
impl Snapshot {
    /// Refresh clock, meters and activity without rebuilding catalog or Settings models.
    fn refresh_playback(
        &mut self,
        state: &AppState,
        player: &mut super::player_model::Projection,
        connection: Option<&super::connection::Status>,
    ) -> bool {
        let previous = (
            self.playback_id,
            self.playing,
            self.position_ms,
            self.duration_ms,
            self.volume,
            self.muted,
            self.seekable,
            self.error.clone(),
            self.status.clone(),
            self.loading.clone(),
        );
        self.player = super::player_model::project(state, player);
        self.playback_id = state.playback.request_id;
        self.playing = state.playback.status == app::PlayStatus::Playing;
        self.position_ms = state.playback.position_ms;
        self.duration_ms = state.playback.duration_ms;
        self.volume = if state.playback.muted {
            0.
        } else {
            state.playback.volume
        };
        self.muted = state.playback.muted;
        self.seekable = self.duration_ms > 0 && state.playback.status != app::PlayStatus::Stopped;
        self.error = state.notifications.last_error.clone();
        self.status = connection
            .and_then(|status| match &status.phase {
                app::settings_transfer::Progress::Failed(error) => Some(error.clone()),
                _ => None,
            })
            .or_else(|| self.error.clone())
            .or_else(|| state.current_notification().map(|n| n.message))
            .unwrap_or_default()
            .replace("Settings (F2)", "Settings (⌘,)");
        self.loading = app::activity::message(state).map(str::to_owned);
        self.download_notification = self.error.is_none()
            && if self.loading.is_some() {
                app::activity::target(state).is_some()
            } else {
                state
                    .current_notification()
                    .is_some_and(|n| n.target.is_some())
            };
        self.playing
            || self.player.active
            || state.desktop_split_view
            || state.desktop_player_visible
            || state.gui_multiple_windows
            || previous
                != (
                    self.playback_id,
                    self.playing,
                    self.position_ms,
                    self.duration_ms,
                    self.volume,
                    self.muted,
                    self.seekable,
                    self.error.clone(),
                    self.status.clone(),
                    self.loading.clone(),
                )
    }
}
pub enum Command {
    SettingsClosed,
    WindowedReady {
        identity: app::windowed_presentation::Identity,
        mode: app::launch::WindowedMode,
    },
    StageSettings(app::settings_transfer::Draft),
    ReadSettings(tokio::sync::oneshot::Sender<Option<app::settings_transfer::Draft>>),
    PresentationFeedback(app::view_continuity::BrowseAnchors),
    DisplayedWorkspace(app::windowed_presentation::Workspace),
    PresentationInput {
        identity: app::windowed_presentation::Identity,
        command: Box<Command>,
    },
    #[cfg(target_os = "macos")]
    TextViewport {
        identity: app::windowed_presentation::Identity,
        viewport: crate::text_mode::backend::Viewport,
    },
    #[cfg(target_os = "macos")]
    TextInput {
        identity: app::windowed_presentation::Identity,
        viewport_revision: Option<u64>,
        input: crate::text_mode::Input,
    },
    PresentationReady(app::windowed_presentation::Preparation),
    PresentationDeferred(app::windowed_presentation::Preparation),
    AlbumInfo {
        generation: u64,
        column: usize,
        identity: String,
    },
    SidebarOpen {
        generation: u64,
        index: usize,
    },
    CloseSurface(String),
    ExecutePalette {
        id: String,
        revision: u64,
        index: usize,
    },
    PlayerWindowVisible(bool),
    ToggleQueue,
    ToggleLargeAlbumCovers,
    QueueWindowClosed,
    PlayerKey(crossterm::event::KeyEvent),
    SidebarGroup {
        generation: u64,
        title: String,
    },
    ToggleSidebarSection {
        generation: u64,
        title: String,
    },
    BrowseGenre {
        generation: u64,
        genre: String,
    },
    ToggleColumnFilter {
        generation: u64,
        column: usize,
        identity: String,
    },
    ColumnFilter {
        generation: u64,
        column: usize,
        identity: String,
        query: String,
    },
    QueueTool {
        generation: u64,
        tool: app::presentation::NpSidebarButton,
    },
    ColumnControl {
        generation: u64,
        column: usize,
        identity: String,
        revision: u64,
        control: usize,
        option: bool,
        shift: bool,
    },
    ArtistContext {
        target: super::browser::Target,
        biography: bool,
    },
    SidebarVisibility {
        generation: u64,
        section: app::state::SidebarSection,
        visible: bool,
    },
    OpenLibraryRoot {
        generation: u64,
        root: app::state::LibraryRoot,
    },
    LibraryRootVisibility {
        generation: u64,
        root: app::state::LibraryRoot,
        visible: bool,
    },
    Cache {
        generation: u64,
        choice: app::sources::LibraryChoice,
        action: app::sources::options::OptionAction,
    },
    ColumnOptions {
        generation: u64,
        column: usize,
        identity: String,
    },
    RadioByName(String),
    OpenPlaylist {
        generation: u64,
        key: String,
        focus: super::sidebar::PlaylistFocus,
    },
    Viewport {
        generation: u64,
        column: usize,
        identity: String,
        range: std::ops::Range<usize>,
    },
    QueueDrop {
        from: super::browser::Target,
        to: super::browser::Target,
        after: bool,
    },
    TrackContext {
        target: super::browser::Target,
        label: String,
    },
    PlayerMode(usize),
    ExitPlayer,
    Preference {
        generation: u64,
        setting: app::state::TextampSetting,
    },
    CoreKey(crossterm::event::KeyEvent),
    Catalog(String),
    RememberPalette(String),
    OpenPalette {
        query: String,
        player: bool,
    },
    Surface {
        id: String,
        revision: u64,
        operation: super::surface::Operation,
    },
    SurfaceEdit {
        id: String,
        field: usize,
        value: String,
    },
    GuardedActions {
        generation: u64,
        target: Option<super::browser::Target>,
        actions: Vec<Action>,
    },
    JumpLetter {
        generation: u64,
        column: usize,
        identity: String,
        letter: char,
        second: bool,
    },
    Action(Action),
    SaveConnection {
        owner: uuid::Uuid,
        draft: super::connection::Draft,
    },
    CancelConnection(uuid::Uuid),
    Seek {
        playback_id: u64,
        fraction: f32,
    },
    FocusDetails {
        generation: u64,
        navigation_serial: u64,
    },
    ClosePreview {
        generation: u64,
        navigation_serial: u64,
    },
    CloseColumn {
        generation: u64,
        column: usize,
        identity: String,
    },
    FocusColumn {
        generation: u64,
        column: usize,
        identity: String,
    },
    Palette(PaletteCommandKind),
    SeekRelative {
        playback_id: u64,
        delta_ms: i64,
    },
    Activate {
        following: bool,
    },
    Navigate {
        column_delta: i8,
        row_delta: i8,
    },
    Select {
        target: super::browser::Target,
        mode: super::browser::Selection,
        origin: super::browser::Origin,
        intent: super::browser::Intent,
    },
}
/// Frontend endpoint stamped with its observed presentation identity.
#[derive(Clone)]
pub struct CommandPort {
    sender: channel::Sender<Command>,
    identity: std::sync::Arc<std::sync::atomic::AtomicU64>,
}
impl CommandPort {
    fn new(sender: channel::Sender<Command>, identity: u64) -> Self {
        Self {
            sender,
            identity: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(identity)),
        }
    }
    pub fn observe(&self, identity: app::windowed_presentation::Identity) {
        self.identity
            .store(identity.0, std::sync::atomic::Ordering::Release);
    }
    fn stamp(&self, command: Command) -> Command {
        let identity = self.identity.load(std::sync::atomic::Ordering::Acquire);
        if identity == 0 || matches!(&command, Command::PresentationReady(_)) {
            return command;
        }
        #[cfg(target_os = "macos")]
        if matches!(
            &command,
            Command::TextViewport { .. } | Command::TextInput { .. }
        ) {
            return command;
        }
        Command::PresentationInput {
            identity: app::windowed_presentation::Identity(identity),
            command: Box::new(command),
        }
    }
    pub fn try_send(
        &self,
        command: Command,
    ) -> Result<(), Box<channel::error::TrySendError<Command>>> {
        self.sender.try_send(self.stamp(command)).map_err(Box::new)
    }
    pub async fn send(
        &self,
        command: Command,
    ) -> Result<(), Box<channel::error::SendError<Command>>> {
        self.sender
            .send(self.stamp(command))
            .await
            .map_err(Box::new)
    }
    fn blocking_send(
        &self,
        command: Command,
    ) -> Result<(), Box<channel::error::SendError<Command>>> {
        self.sender.blocking_send(command).map_err(Box::new)
    }
}

pub struct Engine {
    pub commands: CommandPort,
    pub snapshot: watch::Receiver<Snapshot>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Engine {
    #[cfg(all(test, feature = "desktop-tests"))]
    pub(super) fn fixture(
        snapshot: Snapshot,
    ) -> (Self, watch::Sender<Snapshot>, channel::Receiver<Command>) {
        let (commands, receiver) = channel::channel(256);
        let (updates, snapshot) = watch::channel(snapshot);
        (
            Self {
                commands: CommandPort::new(commands, 0),
                snapshot,
                worker: None,
            },
            updates,
            receiver,
        )
    }

    #[cfg(test)]
    pub fn start(no_audio: bool) -> anyhow::Result<Self> {
        Self::start_mode(no_audio, app::launch::Mode::Restore)
    }
    pub fn start_mode(no_audio: bool, requested: app::launch::Mode) -> anyhow::Result<Self> {
        if !cfg!(target_os = "macos")
            && requested == app::launch::Mode::Windowed(app::launch::WindowedMode::Text)
        {
            anyhow::bail!("Embedded Text Mode is available in the macOS application build");
        }
        let (commands, mut inbox) = channel::channel(256);
        let actor_commands = commands.clone();
        let config = crate::config::load_config()?;
        let requested = match requested {
            app::launch::Mode::Restore if !cfg!(target_os = "macos") => {
                app::launch::WindowedMode::Gui
            }
            app::launch::Mode::Restore => config.ui.last_windowed_mode,
            app::launch::Mode::Windowed(mode) => mode,
            app::launch::Mode::Terminal => {
                anyhow::bail!("External terminal mode does not initialize a desktop engine")
            }
        };
        let (updates, snapshot) = watch::channel(Snapshot {
            theme: crate::app::theme::ThemeName::from_config(&config.ui.theme),
            separate_player: config.ui.desktop_separate_player,
            ..Default::default()
        });
        let (ready, initialized) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new().name("textamp-engine".into()).spawn(move || {
            let result = (|| -> anyhow::Result<()> {
                let runtime = tokio::runtime::Runtime::new()?;
                let result = runtime.block_on(async {
                    let mut live = app::runtime::Session::open(config, if no_audio {app::runtime::AudioMode::Silent} else {app::runtime::AudioMode::Native}).await?;
                    let tasks = live.task_context();
                    let result = tasks.run(async {
                        live.initialize().await?;
                        live.state.windowed_presentation=app::windowed_presentation::Presentation::windowed();
                        if requested==app::launch::WindowedMode::Text {live.state.windowed_presentation.request(requested);}
                        let state = &mut live.state;
                        let audio = &mut live.audio;
                        let config = &mut live.config;
                        let events = live.events.clone();
                        let completions = &mut live.completions;
                        let clock = &mut live.clock;
                        let _ = ready.send(Ok(()));
                        let mut art_cache=super::artwork::Cache::default();

                        let mut player_projection=super::player_model::Projection::default();
                        let mut inspection=super::navigation::Inspection::default();
                        let mut cached_columns=vec![];
                        let mut cached_preview=None;
                        let mut cached_queue=None;
                        let mut changes=ProjectionChange::Content;
                        state.desktop_frontend=true;
                        let mut connection = None;
                        let mut main_view=state.view;let mut settings_request=0u64;
                        let mut player_window_request=0u64;let mut library_window_request=0u64;
                        let mut queue_open=false;let mut queue_request=0u64;
                        let mut sessions=super::workflows::Sessions::default();let mut cached_surfaces=std::sync::Arc::new(Vec::new());
                        let mut surface_request=0u64;let mut surface_epoch=0u64;let mut previous_surface=None;

                        let mut sidebar_cursor=None;
                        let mut sidebar_inspection=super::sidebar::PlaylistInspection::default();
                        let mut sidebar_group:Option<(u64,String,usize)>=None;
                        let mut navigation_serial=0;let mut player_navigation_serial=0u64;
                        let mut navigation_origin=super::browser::Origin::Pointer;
                        let mut source_generation=state.library_generation;
                        let mut radio_was_visible=false;
                        let mut desktop_focus: Option<(u64, app::BrowseCategory, usize)> = None;
                        #[cfg(target_os="macos")]
                        let mut text_session=super::text_session::TextSession::default();
                        let mut text_timer=tokio::time::interval(std::time::Duration::from_millis(16));
                        text_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                        let mut timer = tokio::time::interval(std::time::Duration::from_millis(100));
                        loop {
                            let graphical=state.windowed_presentation.active()!=Some(app::launch::WindowedMode::Text);
                            #[cfg(target_os="macos")]
                            let text_continuous=text_session.continuous(state);
                            #[cfg(not(target_os="macos"))]
                            let text_continuous=false;
                            if graphical && matches!(state.view,app::View::Queue|app::View::NowPlaying) {state.view=app::View::Browse;}
                            if graphical && queue_open!=state.tui_queue_visible {queue_open=state.tui_queue_visible;queue_request=queue_request.wrapping_add(1);}
                            if graphical && state.desktop_split_view && !state.desktop_player_fullscreen.is_open() {state.desktop_player_visible=false;}
                            if graphical && changes!=ProjectionChange::Playback {let mosaics=crate::app::artwork_mosaic::requests(state);if !mosaics.is_empty() {app::dispatch::dispatch_action(SystemAction::LoadAlbumArt(mosaics).into(),state,audio,config,&events).await?;}}
                            if source_generation!=state.library_generation {
                                state.desktop_library_root=app::state::LibraryRoot::Artists;source_generation=state.library_generation;sidebar_cursor=None;sidebar_group=None;
                            }
                            if state.active_library.is_some() && state.station_nav.columns.is_empty() && !state.station_nav.loading {
                                app::dispatch::dispatch_action(app::action::BrowseAction::LoadStations.into(), state, audio, config, &events).await?;
                            }
                            let radio_visible=super::radio::visible(state);
                            if radio_was_visible && !radio_visible {
                                state.sources.nav_tasks.remove("station-children");
                                state.station_navigation_generation=state.station_navigation_generation.wrapping_add(1);
                                state.station_nav.loading=false;
                            }
                            radio_was_visible=radio_visible;
                            let mut active_surface=updates.borrow().surface.clone();
                            let mut presentation=super::navigation::Presentation {columns:cached_columns.clone(),preview:cached_preview.clone(),artwork:None};
                            if graphical && changes!=ProjectionChange::Playback {
                            if state.view==app::View::Settings {settings_request=settings_request.wrapping_add(1);state.view=main_view;}else {main_view=state.view;}
                            if let Some((generation, category, column)) = desktop_focus {
                                if generation == state.library_generation && category == state.browse_category {
                                    if category == app::BrowseCategory::Folders {
                                        if let Some(nav) = state.folder_state.as_mut() { nav.focused_column = column.min(nav.columns.len().saturating_sub(1)); }
                                    } else if let Some(nav) = state.browse_nav_mut() { nav.focused_column = column.min(nav.columns.len().saturating_sub(1)); }
                                } else { desktop_focus = None; }
                            }
                            if !matches!(state.desktop_library_root,app::state::LibraryRoot::Mp3Players|app::state::LibraryRoot::Downloads|app::state::LibraryRoot::Artists|app::state::LibraryRoot::Singles|app::state::LibraryRoot::Radio|app::state::LibraryRoot::Journey|app::state::LibraryRoot::DescribeMusic) && state.artist_nav.columns.first().and_then(|c|c.selected_item()).is_some_and(|item|!matches!(item,app::state::BrowseItem::AllArtists)) {state.desktop_library_root=app::state::LibraryRoot::Artists;}
                            if let Some(nav)=state.browse_nav_mut() {
                                for column in &mut nav.columns {if column.has_album_details() && column.header_selection==app::state::ColumnHeaderSelection::Play {column.header_selection=app::state::ColumnHeaderSelection::AlbumDetails;}}
                            }
                            if super::browser::first_content_column(state)==1
                                && let Some(nav)=state.browse_nav_mut()&& nav.columns.len()>1 && nav.focused_column==0 {nav.focused_column=1;}
                            if inspection.pending_column().is_none() && !matches!(state.artwork.navigation,app::state::ArtworkNavigation::Moving{..}) {
                                let request=super::artwork::request(state).map(|(key,thumb)|(state.library_generation,state.artwork.grid_generation,key,thumb));
                                if let Some((_,_,key,thumb))=&request
                                    && !state.artwork.grid_cache.contains_key(key) && !state.artwork.grid_pending.contains(key) && !state.sources.nav_tasks.contains_key("art-grid") {
                                        app::dispatch::dispatch_action(SystemAction::LoadAlbumArt(vec![(key.clone(),thumb.clone())]).into(),state,audio,config,&events).await?;
                                    }
                            }
                            if inspection.pending_column().is_none() && state.view==app::View::Browse && app::sources::sonic::enabled(state)
                                && let Some((seed,_))=super::catalog::similar_seed(state)
                                    && !state.track_pane_similar.contains_key(&seed.rating_key) && !state.track_pane_similar_loading.contains(&seed.rating_key) {
                                        app::dispatch::dispatch_action(app::action::DataAction::LoadTrackPaneSimilar{rating_key:seed.rating_key}.into(),state,audio,config,&events).await?;
                                    }
                            super::connection::refresh(state, &mut connection);
                            app::settings_transfer::refresh_parked(state);
                            active_surface=if (state.staged_settings.is_some() || connection.as_ref().is_some_and(|s:&super::connection::Status|!matches!(s.phase,app::settings_transfer::Progress::Saved))) && matches!(state.popups.library_dialog(),Some(app::sources::dialogs::Dialog::Navidrome(_) | app::sources::dialogs::Dialog::Webdav(_) | app::sources::dialogs::Dialog::Local(_))) {None}else{super::surface::project(state).filter(|model|model.palette || !super::workspace::owns(state))};
                            let base=active_surface.as_ref().map(|s|s.id.clone());
                            if base!=previous_surface {surface_epoch=surface_epoch.wrapping_add(1);previous_surface=base;}
                            if !config.ui.desktop_separate_player&& let Some(model)=active_surface.as_mut(){model.id.push_str(&format!(":{surface_epoch}"));}
                            if changes!=ProjectionChange::Playback {sessions.sync(active_surface.as_ref(), state, config.ui.desktop_separate_player);if config.ui.desktop_separate_player {sessions.refresh_palettes(state);}cached_surfaces=std::sync::Arc::new(sessions.models());}
                            if config.ui.desktop_separate_player&& let Some(model)=active_surface.as_mut()&& let Some(id)=sessions.resolve(&model.id){model.id=id;}
                            if changes!=ProjectionChange::Playback {
                                if changes!=ProjectionChange::Selection || !super::browser::refresh_selection(state,&mut cached_columns) {cached_columns=super::browser::columns(state);}
                                for column in &mut cached_columns {column.large_artwork=config.ui.desktop_large_album_covers && column.rows.iter().any(|r|r.kind==super::browser::Kind::Album);}
                                cached_queue=Some(super::browser::queue(state));
                            }
                            inspection.settle(state);
                            if changes!=ProjectionChange::Playback && inspection.pending_column().is_none() {
                                cached_preview=super::catalog::preview(state);
                            }
                            let fresh = super::navigation::Presentation {
                                columns: cached_columns.clone(),
                                preview: if inspection.pending_column().is_none() {cached_preview.clone()} else {None},
                                artwork: if inspection.pending_column().is_none() {super::artwork::preview(state)} else {None},
                            };
                            presentation=inspection.project(fresh);
                            let mut next = Snapshot {
                                activity_history:state.settings_state.activity_history.clone(),
                                player_sync:app::player_sync::project(state),
                                presentation:state.windowed_presentation.clone(),presentation_anchors:state.presentation_anchors.clone(),browse_category:Some(state.browse_category),
                                #[cfg(target_os="macos")]
                                text_frame:text_session.frame(),
                                workspace:sidebar_group.as_ref().filter(|(generation,_,_)|*generation==state.library_generation).and_then(|(_,title,selected)|super::surface::sidebar_group(state,title,*selected)).or_else(||super::workspace::project(state)),
                                workspace_title:sidebar_group.as_ref().filter(|(generation,_,_)|*generation==state.library_generation).map(|(_,title,_)|title.clone()).or_else(||super::workspace::title(state).map(str::to_owned)),
                                radio_browser: (sidebar_group.is_none() && super::radio::visible(state)).then(||super::surface::radio_surface(state)),
                                details_focused:state.track_pane_focused,
                                separate_player:config.ui.desktop_separate_player,
                                singles_track_limit:state.singles_track_limit,
                                recent_commands:state.palette.history.clone(),
                                menu_commands:super::menu_catalog::catalog(Some(state)),
                                sidebar_cursor,
                                collapsed_sidebar_sections:state.desktop_collapsed_sections.clone(),
                                filter: if state.list_filter.category == state.browse_category {state.list_filter.clone()} else {Default::default()},
                                connection: connection.clone(),settings_progress:state.staged_settings.as_ref().map(|draft|draft.progress.clone()),
                                sidebar_options: super::preferences::sidebar_options(state),
                                caches: app::sources::cache::summaries(state),
                                settings_request, player_window_request, library_window_request, queue_open, queue_request,sidebar_visible:Some(state.tui_sidebar_visible),
                                surface_request,
                                radio_commands: app::command_palette::materialize_entries(state).into_iter().filter(|e|matches!(e.command,PaletteCommandKind::StopRadio|PaletteCommandKind::PlayStation(_)|PaletteCommandKind::BrowseStations{..}|PaletteCommandKind::StationsBack)).map(|e|e.label).collect(),
                                images:art_cache.project(state),art_generation:state.artwork.grid_generation,
                                playback_queue:cached_queue.clone(),
                                preferences:super::preferences::project(state),
                                theme:state.theme,artwork_mode:state.artwork.graphical_mode,
                                view_options:super::commands::view_options(state),queue_tools:app::queue_controls::definitions(state),disabled_queue_menu_items:super::commands::disabled_queue_menu_items(state),queue_mode:app::queue_modes::label(state),queue_modes:app::queue_modes::choices(state),
                                split_player:state.desktop_split_view,
                                context_policy:crate::app::track_context::ContextPolicy::for_state(state),
                                preview:presentation.preview.clone(),
                                sidebar:super::catalog::sidebar(state),
                                surface:active_surface.clone(),
                                surfaces:cached_surfaces.clone(),
                                artwork:presentation.artwork.clone(),
                                navigation_serial,
                                ghost_artist:state.revealed_compilation_artist.clone().filter(|key|state.is_ghost_artist(key)),
                                jump_head:state.jump_history.head(),
                                jump_restore:state.jump_history.restored,
                                player_navigation_serial,
                                navigation_origin,
                                playback_id: state.playback.request_id,
                                section: match state.view { app::View::Queue => "Queue".into(), app::View::NowPlaying => "Now Playing".into(), _ => state.browse_category.display_label().into() },
                                focused_column: if matches!(state.view,app::View::Queue|app::View::NowPlaying) {0}else{state.browse_nav().map(|n| n.focused_column).or_else(|| state.folder_state.as_ref().map(|n| n.focused_column)).unwrap_or(0)},

                                item_summary: state.browse_item_summary(),
                                artist: state.current_track().and_then(|t| t.grandparent_title.clone()).unwrap_or_default(),
                                generation: state.library_generation,
                                libraries: app::sources::library_choices(state),
                                analysis_connections: app::sources::library_choices(state).iter().filter_map(|choice| {
                                    let key = app::sources::sonic::choice_key(choice, state)?;
                                    Some((choice.artwork_key()?, state.sources.audiomuse.connections.get(&key)?.clone()))
                                }).collect(),
                                columns: presentation.columns.clone(),

                                now_playing: state.current_track().map(|t| t.title.clone()).unwrap_or_else(|| "Nothing playing".into()),
                                ..Default::default()
                            };
                            next.refresh_playback(state, &mut player_projection, connection.as_ref());
                            updates.send_replace(next);
                            } else if graphical {
                                let mut next = updates.borrow().clone();
                                if next.refresh_playback(state, &mut player_projection, connection.as_ref()) {
                                    updates.send_replace(next);
                                }
                            } else {
                                let status=state.current_notification().map(|notification|notification.message).unwrap_or_default();
                                let loading=app::activity::message(state).map(str::to_owned);
                                let current=updates.borrow();
                                let render=changes!=ProjectionChange::Playback || text_continuous || loading.is_some() || status!=current.status || state.notifications.last_error!=current.error;
                                drop(current);
                                if render {
                                #[cfg(target_os="macos")]
                                if let Err(error)=text_session.render(state) {state.set_status(format!("Text Mode renderer: {error}"));}
                                let mut next=updates.borrow().clone();
                                next.presentation=state.windowed_presentation.clone();next.presentation_anchors=state.presentation_anchors.clone();
                                #[cfg(target_os="macos")]
                                {next.text_frame=text_session.frame();}
                                next.theme=state.theme;
                                next.generation=state.library_generation;
                                next.view_options=super::commands::view_options(state);
                                if changes!=ProjectionChange::Playback {next.preferences=super::preferences::project(state);next.libraries=app::sources::library_choices(state);next.queue_tools=app::queue_controls::definitions(state);next.disabled_queue_menu_items=super::commands::disabled_queue_menu_items(state);}
                                next.status=status;
                                next.loading=loading;
                                next.error=state.notifications.last_error.clone();
                                next.playing=state.playback.status==app::PlayStatus::Playing;
                                next.position_ms=state.playback.position_ms;
                                updates.send_replace(next);
                                }
                            }
                            // Commands and completions may change lists or their selection.
                            // A passive playback tick only changes transport, meters and status.
                            changes=ProjectionChange::Content;let mut defer_inspection=false;
                            let mut actions = tokio::select! {
                                command = async {
                                    'receive: loop {
                                        let Some(mut command) = inbox.recv().await else {
                                            break None;
                                        };
                                        loop {
                                            match command {
                                                Command::Catalog(key) => {
                                                    let Some(command) = super::menu_catalog::resolve(
                                                        &key,
                                                        &super::menu_catalog::catalog(Some(state)),
                                                    ) else {
                                                        continue 'receive;
                                                    };
                                                    break 'receive Some(Command::Palette(command));
                                                }
                                                Command::PresentationInput { identity, command: input } => {
                                                    if !state.windowed_presentation.accepts(identity) {
                                                        continue 'receive;
                                                    }
                                                    command = *input;
                                                }
                                                other => break 'receive Some(other),
                                            }
                                        }
                                    }
                                } => {
                                    let command = if let Some(Command::SidebarOpen { generation, index }) = command {
                                        if generation != state.library_generation { continue; }
                                        let Some(destination) = super::sidebar::open_command(state,index) else { continue; };
                                        sidebar_group = None;
                                        if super::workspace::owns(state) {
                                            for action in super::surface::cancel(state,config) {
                                                app::dispatch::dispatch_action(action,state,audio,config,&events).await?;
                                            }
                                        }
                                        if config.ui.desktop_separate_player { sessions.leave_for_browse(state); }
                                        if !matches!(destination, Command::ToggleSidebarSection { .. }) {
                                            if app::player_sync::visible(state)
                                                && !matches!(destination, Command::OpenLibraryRoot { root: app::state::LibraryRoot::Mp3Players, .. })
                                            {
                                                state.desktop_library_root = app::state::LibraryRoot::Artists;
                                            }
                                            sidebar_cursor = None;
                                            desktop_focus = None;
                                            state.category_column_focused = false;
                                            state.focus = app::state::Focus::Right;
                                        }
                                        Some(destination)
                                    } else { command };
                                    match command {
                                    Some(Command::PresentationInput {..})=>vec![],
                                    Some(Command::SettingsClosed)=>{state.staged_settings=None;vec![]},
                                    Some(Command::WindowedReady {identity,mode})=>{
                                        if state.windowed_presentation.accepts(identity) && state.windowed_presentation.active()==Some(mode) && state.windowed_presentation.pending().is_none() && config.ui.last_windowed_mode!=mode {
                                            config.ui.last_windowed_mode=mode;
                                            vec![app::action::SettingsAction::SaveSettings.into()]
                                        }else{vec![]}
                                    },
                                    Some(Command::StageSettings(mut draft))=>{draft.retain_pending_request(state);state.staged_settings=Some(draft);vec![]},
                                    Some(Command::ReadSettings(reply))=>{let _=reply.send(state.staged_settings.clone());vec![]},
                                    Some(Command::DisplayedWorkspace(workspace))=>{
                                        if state.windowed_presentation.active()==Some(app::launch::WindowedMode::Gui) {state.windowed_presentation.set_workspace(workspace);}
                                        vec![]
                                    },
                                    Some(Command::PresentationFeedback(anchors))=>{
                                        if anchors.generation==state.library_generation && anchors.category==state.browse_category {state.browse_viewport=Some(anchors.clone());state.presentation_anchors=Some(anchors);}
                                        vec![]
                                    },
                                    #[cfg(target_os="macos")]
                                    Some(Command::TextViewport {identity,viewport})=>text_session.viewport(identity,viewport,state,config,queue_open),
                                    #[cfg(target_os="macos")]
                                    Some(Command::TextInput {identity,viewport_revision,input})=>text_session.input(identity,viewport_revision,input,state,config,&events),
                                    Some(Command::PresentationDeferred(preparation))=>{
                                        if state.windowed_presentation.defer(preparation) {state.set_status("Close the native dialog or finish text composition to switch modes".into());}
                                        vec![]
                                    },
                                    Some(Command::PresentationReady(preparation))=>{
                                        if preparation.destination==app::launch::WindowedMode::Gui && state.windowed_presentation.commit(preparation) {
                                            state.staged_settings=app::settings_transfer::Draft::capture(state);
                                            if app::settings_transfer::Progress::from_dialog(state.popups.library_dialog()) != app::settings_transfer::Progress::Checking {state.popups.set_library_dialog(None);}
                                            let anchors=app::view_continuity::capture_terminal(state);
                                            if !anchors.columns.is_empty() {state.browse_viewport=Some(anchors);}
                                            state.presentation_anchors=state.browse_viewport.clone();
                                            state.desktop_frontend=true;
                                            connection=None;
                                            desktop_focus=None;
                                            sidebar_cursor=state.category_column_focused.then(||super::catalog::sidebar(state).iter().position(|row|row.active)).flatten();
                                            queue_open=state.tui_queue_visible;
                                            state.desktop_player_visible=state.windowed_presentation.workspace()==Some(app::windowed_presentation::Workspace::NowPlaying);
                                            if state.desktop_player_visible {player_window_request=player_window_request.wrapping_add(1);}else {library_window_request=library_window_request.wrapping_add(1);}
                                            if state.view==app::View::Settings {settings_request=settings_request.wrapping_add(1);}
                                            let content_view=if state.view==app::View::Settings {state.previous_view.unwrap_or(app::View::Browse)}else{state.view};
                                            state.view=state.windowed_presentation.project_view(app::launch::WindowedMode::Gui,content_view);
                                            config.ui.last_windowed_mode=app::launch::WindowedMode::Gui;
                                            #[cfg(target_os="macos")]
                                            text_session.suspend();
                                            vec![app::action::SettingsAction::SaveSettings.into()]
                                        }else{vec![]}
                                    },
                                    Some(Command::AlbumInfo {generation,column,identity})=>{
                                        if generation!=state.library_generation || super::browser::columns(state).get(column).is_none_or(|c|c.identity!=identity || !c.album_info) {continue;}
                                        sidebar_cursor=None;desktop_focus=Some((generation,state.browse_category,column));
                                        select_album_info(state,column);vec![]
                                    },
                                    Some(Command::SidebarGroup{generation,title}) if title == "Radio" => {
                                        if generation == state.library_generation {
                                            sidebar_group=None;
                                            let _ = actor_commands.try_send(Command::OpenLibraryRoot {generation, root: app::state::LibraryRoot::Radio});
                                        }
                                        vec![]
                                    },
                                    Some(Command::SidebarGroup{generation,title})=>{
                                        if generation==state.library_generation && super::surface::sidebar_group(state,&title,0).is_some() {
                                            sidebar_cursor=super::catalog::sidebar(state).iter().position(|row|row.header && row.label==title);
                                            state.playlist_group_filter=None;
                                            sidebar_group=Some((generation,title,0));
                                            if app::player_sync::visible(state) { state.desktop_library_root=app::state::LibraryRoot::Artists; }
                                            state.desktop_player_visible=false;state.desktop_player_fullscreen=app::state::DesktopFullscreenView::Closed;
                                            library_window_request=library_window_request.wrapping_add(1);
                                        }
                                        vec![]
                                    },
                                    Some(Command::SidebarOpen{..})=>unreachable!("sidebar destinations are resolved on receipt"),
                                    Some(Command::CloseSurface(id))=>{
                                        if sidebar_group.as_ref().and_then(|(_,title,selected)|super::surface::sidebar_group(state,title,*selected)).is_some_and(|model|model.id==id) {sidebar_group=None;sidebar_cursor=None;continue;}
                                        let actions=if super::workspace::project(state).is_some_and(|model|model.id==id) {super::surface::cancel(state,config)}else if super::radio::visible(state) && super::surface::radio_surface(state).id==id {
                                            super::surface::navigate(PaletteCommandKind::GotoLibrary,state,config)
                                        }else if config.ui.desktop_separate_player {sessions.close(&id,state,config)}
                                            else if active_surface.as_ref().is_some_and(|model| model.id==id) {super::surface::cancel(state,config)}else{vec![]};
                                        for action in actions {app::dispatch::dispatch_action(action,state,audio,config,&events).await?;}
                                        vec![]
                                    },
                                    Some(Command::OpenLibraryRoot{generation,root})=>{
                                        sidebar_cursor=None;
                                        if generation!=state.library_generation {continue;}
                                        if matches!(root,app::state::LibraryRoot::Journey|app::state::LibraryRoot::DescribeMusic) {
                                            library_window_request=library_window_request.wrapping_add(1);
                                            let actions=super::workspace::open(root,state,config);
                                            for action in actions {app::dispatch::dispatch_action(action,state,audio,config,&events).await?;}
                                            continue;
                                        }
                                        if root==app::state::LibraryRoot::Radio {
                                            library_window_request=library_window_request.wrapping_add(1);
                                            let _=actor_commands.try_send(Command::Palette(PaletteCommandKind::OpenRadioBrowser));
                                            continue;
                                        }
                                        if root != app::state::LibraryRoot::Mp3Players || state.sources.active.folder().is_none() {
                                            let actions=super::surface::navigate(PaletteCommandKind::GotoLibrary,state,config);
                                            for action in actions {app::dispatch::dispatch_action(action,state,audio,config,&events).await?;}
                                        }
                                        if root == app::state::LibraryRoot::Mp3Players {
                                            library_window_request=library_window_request.wrapping_add(1);
                                        }
                                        let actions=app::sidebar::open_root(root,state);
                                        let column=usize::from(!matches!(root,app::state::LibraryRoot::Downloads|app::state::LibraryRoot::Artists|app::state::LibraryRoot::Singles));desktop_focus=Some((generation,app::BrowseCategory::Library,column));
                                        navigation_serial+=1;navigation_origin=super::browser::Origin::Keyboard;
                                        actions
                                    },
                                    Some(Command::ToggleSidebarSection{generation,title})=>{
                                        if generation!=state.library_generation {continue;}
                                        app::dispatch::dispatch_action(app::action::SettingsAction::ToggleSidebarGroup(title.clone()).into(),state,audio,config,&events).await?;
                                        let rows=super::catalog::sidebar(state);
                                        if sidebar_cursor.is_some_and(|i|!super::sidebar::visible_rows(&rows,&state.desktop_collapsed_sections).contains(&i)) {sidebar_cursor=rows.iter().position(|r|r.header && r.label==title);}
                                        vec![]
                                    },
                                    Some(Command::SidebarVisibility{generation,section,visible})=>{
                                        if generation!=state.library_generation || !super::preferences::sidebar_options(state).iter().any(|option| option.target==super::preferences::SidebarTarget::Section(section)) {continue;}
                                        if section.hidden(state)==visible {vec![app::action::SettingsAction::ToggleSectionVisibility(section).into()]}else{vec![]}
                                    },
                                    Some(Command::LibraryRootVisibility{generation,root,visible})=>{
                                        if generation!=state.library_generation {continue;}
                                        vec![app::action::SettingsAction::SetLibraryRootVisible { root, visible }.into()]
                                    },
                                    Some(Command::Cache{generation,choice,action})=>{
                                        if generation!=state.library_generation || !app::sources::library_choices(state).iter().any(|c|c.artwork_key()==choice.artwork_key()) {continue;}
                                        app::sources::options::apply(choice,action,state)
                                    },
                                    Some(Command::ColumnOptions{generation,column,identity})=>{
                                        if generation!=state.library_generation || super::browser::columns(state).get(column).is_none_or(|c|c.identity!=identity){continue;}
                                        surface_request=surface_request.wrapping_add(1);
                                        surface_epoch=surface_epoch.wrapping_add(1);
                                        desktop_focus=Some((generation,state.browse_category,column));
                                        if let Some(nav)=state.browse_nav_mut(){nav.focused_column=column;}
                                        state.category_column_focused=false;state.focus=app::state::Focus::Right;
                                        vec![app::action::SearchAction::OpenSortPopup.into()]
                                    },
                                    Some(Command::OpenPlaylist {generation,key,focus}) => {
                                        if generation!=state.library_generation {continue;}
                                        let keep_sidebar=focus==super::sidebar::PlaylistFocus::Sidebar;
                                        if keep_sidebar && !super::sidebar::playlist_has_focus(state,sidebar_cursor,&key) {continue;}
                                        if !keep_sidebar {sidebar_cursor=None;}
                                        inspection.cancel(state);
                                        let already_open=super::sidebar::playlist_is_open(state,&key);
                                        if keep_sidebar && already_open {continue;}
                                        if !already_open {
                                            app::dispatch::dispatch_action(app::action::NavigationAction::set_category(app::BrowseCategory::Playlists).into(),state,audio,config,&events).await?;
                                        }
                                        if state.select_playlist(&key) {
                                            if !already_open {state.list_filter.deactivate();}
                                            state.focus=app::state::Focus::Right;state.category_column_focused=keep_sidebar;
                                            desktop_focus=if keep_sidebar {None}else{Some((generation,app::BrowseCategory::Playlists,1))};
                                            navigation_serial+=1;navigation_origin=super::browser::Origin::Keyboard;
                                            if already_open {vec![]}else{vec![app::action::MillerAction::LoadPlaylistTracksForMiller{playlist_key:key,replace_child:false}.into()]}
                                        }else{vec![]}
                                    },
                                    Some(Command::RadioByName(name)) => {
                                        app::command_palette::materialize_entries(state).into_iter().find(|e|e.label==name && (matches!(e.command,PaletteCommandKind::StopRadio|PaletteCommandKind::PlayStation(_)|PaletteCommandKind::BrowseStations{..}|PaletteCommandKind::StationsBack))).map(|e|super::surface::navigate(e.command,state,config)).unwrap_or_default()
                                    },
                                    Some(Command::Action(action)) => {
                                        if matches!(action,Action::Search(app::action::SearchAction::SelectFilteredItem)) && state.list_filter.active {
                                            desktop_focus=Some((state.library_generation,state.browse_category,state.list_filter.column));
                                        }
                                        vec![action]
                                    },
                                    Some(Command::Viewport {generation,column,identity,range})=>{
                                        let mut actions=vec![];
                                        if generation==state.library_generation && super::browser::columns(state).get(column).is_some_and(|c|c.identity==identity)
                                            && let Some(c)=state.browse_nav().and_then(|n|n.columns.get(column)) {
                                                if c.artwork_visible {
                                                    let mut images=c.items.get(range.start.min(c.items.len())..range.end.min(c.items.len())).unwrap_or_default().iter().filter_map(|item|match item {
                                                        app::state::BrowseItem::Album {key,thumb:Some(thumb),..} if !state.artwork.grid_cache.contains_key(key) && !state.artwork.grid_pending.contains(key)=>Some((key.clone(),thumb.clone())),
                                                        app::state::BrowseItem::Track {key,..} if !state.artwork.grid_cache.contains_key(key) && !state.artwork.grid_pending.contains(key)=>c.tracks.iter().find(|t|&t.rating_key==key).and_then(|t|t.best_thumb().map(|thumb|(key.clone(),thumb.to_owned()))),_=>None
                                                    }).collect::<Vec<_>>();
                                                    if let Some(pair)=super::artwork::request(state).filter(|(key,_)|!state.artwork.grid_cache.contains_key(key)) {images.insert(0,pair);}
                                                    if !images.is_empty(){actions.push(SystemAction::LoadAlbumArt(images).into());}
                                                }
                                                if let Some(lazy)=c.lazy.as_ref().filter(|l|!l.loading && l.total.is_none_or(|total|c.tracks.len()<total as usize))
                                                    && range.end.saturating_add(20)>=c.items.len(){actions.push(app::action::MillerAction::LoadMorePlaylistTracks {playlist_key:lazy.key.clone(),offset:c.tracks.len().min(u32::MAX as usize) as u32}.into());}
                                            }actions
                                    },
                                    Some(Command::QueueDrop {from,to,after})=>{
                                        if (from.column==super::browser::PLAYER_QUEUE || matches!(state.view,app::View::Queue|app::View::NowPlaying)) && super::browser::valid(state,&from) && super::browser::valid(state,&to) {
                                            vec![app::action::QueueAction::ReorderSelection {from:from.row,to:to.row+usize::from(after)}.into()]
                                        }else{vec![]}
                                    },
                                    Some(Command::BrowseGenre {generation,genre})=>{
                                        if generation!=state.library_generation {continue;}
                                        app::navigation_history::record(state);
                                        navigation_serial+=1;navigation_origin=super::browser::Origin::LibraryJump;
                                        library_window_request=library_window_request.wrapping_add(1);if !config.ui.desktop_separate_player {state.desktop_player_visible=false;state.desktop_player_fullscreen=app::state::DesktopFullscreenView::Closed;}
                                        if generation!=state.library_generation {continue;}
                                        let key=state.library.album_genres.iter().find(|g|g.title==genre).map(|g|g.key.clone());
                                        if let Some(genre_key)=key {
                                            app::dispatch::dispatch_action(app::action::NavigationAction::set_category(app::BrowseCategory::AlbumGenres).into(),state,audio,config,&events).await?;
                                            if let Some(col)=state.tag_nav.columns.first_mut()&& let Some(index)=col.items.iter().position(|i|i.key()==genre_key){col.selected_index=index;}
                                            desktop_focus=None;
                                            vec![app::action::MillerAction::LoadGenreAlbumsForMiller{genre_key,replace_child:false}.into()]
                                        }else{state.set_status(format!("Genre {genre} is not in this library"));vec![]}
                                    },
                                    Some(Command::ToggleColumnFilter {generation,column,identity}) => {
                                        if generation!=state.library_generation || super::browser::columns(state).get(column).is_none_or(|c|c.identity!=identity){continue;}
                                        desktop_focus=Some((generation,state.browse_category,column));
                                        if let Some(nav)=state.browse_nav_mut(){nav.focused_column=column;}
                                        if let Some(nav)=state.folder_state.as_mut().filter(|_|state.browse_category==app::BrowseCategory::Folders){nav.focused_column=column;}
                                        state.category_column_focused=false;state.focus=app::state::Focus::Right;
                                        if !state.list_filter.active || state.list_filter.column!=column {
                                            vec![app::action::SearchAction::ActivateListFilter.into()]
                                        } else {vec![app::action::SearchAction::DeactivateListFilter.into()]}
                                    },
                                    Some(Command::ColumnFilter{generation,column,identity,query}) => {
                                        if generation!=state.library_generation || super::browser::columns(state).get(column).is_none_or(|c|c.identity!=identity){continue;}
                                        desktop_focus=Some((generation,state.browse_category,column));
                                        if let Some(nav)=state.browse_nav_mut(){nav.focused_column=column;}
                                        if let Some(nav)=state.folder_state.as_mut().filter(|_|state.browse_category==app::BrowseCategory::Folders){nav.focused_column=column;}
                                        state.category_column_focused=false;state.focus=app::state::Focus::Right;
                                        if state.list_filter.column!=column {state.list_filter.deactivate();}
                                        vec![app::action::SearchAction::SetListFilterQuery(query).into()]
                                    },
                                    Some(Command::QueueTool {generation,tool}) => {
                                        if generation != state.library_generation {continue;}
                                        app::queue_controls::activate(state,tool)
                                    },
                                    Some(Command::ColumnControl {generation,column,identity,revision,control,option,shift}) => {
                                        if generation != state.library_generation {vec![]} else {
                                            super::browser::columns(state).get(column)
                                                .filter(|c| c.identity==identity && c.revision==revision)
                                                .and_then(|c| c.header_controls.get(control)).map(|c| c.actions_for(option,shift)).unwrap_or_default()
                                        }
                                    },
                                    Some(Command::ArtistContext {target,biography}) => {
                                        if !super::browser::valid(state,&target) {vec![]} else {
                                            if let Some(nav)=state.browse_nav_mut() {
                                                nav.focused_column=target.column;
                                                let column=&mut nav.columns[target.column];
                                                column.selected_index=target.row;column.selected_set.clear();
                                            }
                                            state.track_pane_focused=false;state.category_column_focused=false;state.focus=app::state::Focus::Right;
                                            desktop_focus=Some((target.generation,state.browse_category,target.column));
                                            app::command_palette::run(if biography {PaletteCommandKind::ShowArtistBio} else {PaletteCommandKind::ArtistRadio},state)
                                        }
                                    },
                                    Some(Command::TrackContext {target,label})=>{
                                        if !super::browser::valid(state,&target){vec![]}else {
                                            let track=super::browser::target_column(state,target.column).and_then(|c|c.tracks.get(target.row).cloned());
                                            if let Some(track)=track {
                                                select_target(state,&target,super::browser::Selection::Context,super::browser::Intent::Focus);
                                                if target.column!=super::browser::PLAYER_QUEUE {desktop_focus=Some((target.generation,state.browse_category,target.column));}
                                                if let Some(entry)=crate::app::track_context::track_context_entries(state,&track,false).into_iter().chain(crate::app::track_context::now_playing_context(crate::app::track_context::ContextPolicy::for_state(state),&track)).find(|e|e.label==label) {
                                                    if matches!(entry.kind,crate::app::track_context::ContextKind::AddToPlaylist) {select_target(state,&target,super::browser::Selection::Context,super::browser::Intent::AddTracksToPlaylist)} else if target.column==super::browser::PLAYER_QUEUE && matches!(entry.kind,crate::app::track_context::ContextKind::PlayTrack|crate::app::track_context::ContextKind::PlayTrackAndFollowing) {super::queue::play_at(state,target.row)}else{app::command_palette::run(PaletteCommandKind::FromTrackContext {kind:entry.kind,track:Box::new(track)},state)}
                                                }else{vec![]}
                                            }else{vec![]}
                                        }
                                    },
                                    Some(Command::ToggleLargeAlbumCovers)=>vec![app::action::SettingsAction::SetLargeArtwork(!state.large_artwork).into()],
                                    Some(Command::ToggleQueue)=>vec![app::action::NavigationAction::ToggleQueuePanel.into()],
                                    Some(Command::QueueWindowClosed)=>{state.tui_queue_visible=false;vec![]},
                                    Some(Command::PlayerWindowVisible(visible))=>{state.desktop_player_visible=visible;if !visible {state.desktop_player_fullscreen=app::state::DesktopFullscreenView::Closed;}vec![]},
                                    Some(Command::PlayerKey(key))=>{player_navigation_serial=player_navigation_serial.wrapping_add(1);queue_key(state,key,config)},
                                    Some(Command::CoreKey(key))=>{
                                        desktop_focus=None;navigation_serial+=1;navigation_origin=super::browser::Origin::Keyboard;
                                        if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) && key.code==crossterm::event::KeyCode::Char('l') {super::surface::navigate(PaletteCommandKind::GotoLibrary,state,config)}else if state.view==app::View::NowPlaying && matches!(key.code,crossterm::event::KeyCode::Up|crossterm::event::KeyCode::Down|crossterm::event::KeyCode::Home|crossterm::event::KeyCode::End|crossterm::event::KeyCode::PageUp|crossterm::event::KeyCode::PageDown) {queue_key(state,key,config)}else{app::handlers::key_input::handle_key(key,state,config)}
                                    },
                                    Some(Command::PlayerMode(mode))=>{select_player_mode(state,mode);vec![]},
                                    Some(Command::ExitPlayer)=>{
                                        if state.desktop_player_fullscreen.is_open() {state.desktop_player_fullscreen=app::state::DesktopFullscreenView::Closed;vec![]}
                                        else {state.windowed_presentation.set_workspace(app::windowed_presentation::Workspace::Library);library_window_request=library_window_request.wrapping_add(1);if !config.ui.desktop_separate_player {state.desktop_player_visible=false;state.desktop_player_fullscreen=app::state::DesktopFullscreenView::Closed;}vec![]}
                                    },
                                    Some(Command::Preference {generation,setting})=>{
                                        if generation!=state.library_generation {continue;}
                                        let mut actions = if matches!(setting, app::state::TextampSetting::Presentation(_)) {
                                            inspection.take(state,std::time::Instant::now(),true).unwrap_or_default()
                                        } else { vec![] };
                                        actions.push(app::action::SettingsAction::ActivatePreference(setting).into());
                                        actions
                                    },
                                    Some(Command::Catalog(_)) => unreachable!("catalog commands are normalized at receipt"),
                                    Some(Command::RememberPalette(key)) => {
                                        if state.palette.history.remember(key) {vec![app::action::SettingsAction::PersistCommandHistory.into()]} else {vec![]}
                                    },
                                    Some(Command::OpenPalette{query,player}) => {
                                        if state.palette.open && !config.ui.desktop_separate_player {surface_epoch=surface_epoch.wrapping_add(1);}
                                        surface_request=surface_request.wrapping_add(1);
                                        {
                                            state.palette.player_context=player;
                                            let mut actions = app::command_palette::open_with_query(state,&query);
                                            if state.station_nav.columns.is_empty() && !state.station_nav.loading {actions.push(app::action::BrowseAction::LoadStations.into());} actions
                                        }
                                    },
                                    Some(Command::ExecutePalette{id,revision,index})=>{
                                        let item=if config.ui.desktop_separate_player {sessions.palette_item(&id,revision,index)}else{active_surface.as_ref().filter(|model|model.id==id && model.revision==revision && model.palette).and_then(|model|model.rows.get(index)).cloned()};
                                        if let Some(item)=item {
                                            if config.ui.desktop_separate_player && !sessions.activate(&id,Some(revision),state) {continue;}
                                            let mut remembered = if let super::surface::Operation::Palette(command) = &item.operation {app::command_palette::remember(command,state)} else {vec![]};
                                            let actions=if matches!(item.operation,super::surface::Operation::Palette(PaletteCommandKind::CloseColumn)) {
                                                state.palette.close();
                                                let _=actor_commands.try_send(Command::Palette(PaletteCommandKind::CloseColumn));
                                                vec![]
                                            } else {apply_surface_operation(item.operation,state,config)};
                                            if config.ui.desktop_separate_player && !state.palette.open {
                                                sessions.dismiss_playlist_picker(&id);
                                            }
                                            remembered.extend(actions); remembered
                                        }else{vec![]}
                                    },
                                    Some(Command::Surface {id,revision,operation}) => {
                                        if let Some((_,title,selected))=sidebar_group.as_mut().filter(|(generation,_,_)|*generation==state.library_generation)
                                            && let Some(model)=super::surface::sidebar_group(state,title,*selected).filter(|model|model.id==id && model.revision==revision) {
                                                let index=match operation {
                                                    super::surface::Operation::Row(index)=>Some(index),
                                                    super::surface::Operation::Key(crossterm::event::KeyCode::Enter|crossterm::event::KeyCode::Right)=>model.rows.get(*selected).and_then(|item|if let super::surface::Operation::Row(index)=item.operation {Some(index)} else {None}),
                                                    super::surface::Operation::Key(crossterm::event::KeyCode::Down)=>{*selected=(*selected+1).min(model.rows.len().saturating_sub(1));None},
                                                    super::surface::Operation::Key(crossterm::event::KeyCode::Up)=>{*selected=selected.saturating_sub(1);None},
                                                    super::surface::Operation::Key(crossterm::event::KeyCode::Esc|crossterm::event::KeyCode::Left)=>{sidebar_group=None;sidebar_cursor=None;None},
                                                    _=>None,
                                                };
                                                if let Some(index)=index.filter(|index|model.rows.iter().any(|row|matches!(row.operation,super::surface::Operation::Row(target) if target==*index))) {let _=actor_commands.try_send(Command::SidebarOpen{generation:state.library_generation,index});}
                                                continue;
                                            }
                                        if matches!(&operation,super::surface::Operation::Palette(PaletteCommandKind::GotoQueue)) && (active_surface.as_ref().is_some_and(|model|model.id==id && model.revision==revision) || cached_surfaces.iter().any(|model|model.id==id && model.revision==revision)) {
                                            state.palette.close();for action in app::command_palette::run(PaletteCommandKind::GotoQueue,state) {app::dispatch::dispatch_action(action,state,audio,config,&events).await?;}continue;
                                        }
                                        if super::workspace::project(state).is_some_and(|model|model.id==id && (model.revision==revision || matches!(operation,super::surface::Operation::Key(_)))) {
                                            let actions=apply_surface_operation(operation,state,config);
                                            for action in actions {app::dispatch::dispatch_action(action,state,audio,config,&events).await?;}
                                            continue;
                                        }
                                        if super::radio::visible(state) {
                                            let model=super::surface::radio_surface(state);
                                            if model.id==id && (model.revision==revision || matches!(operation,super::surface::Operation::Key(_))) {
                                                let actions=apply_surface_operation(operation,state,config);
                                                for action in actions {app::dispatch::dispatch_action(action,state,audio,config,&events).await?;}
                                                continue;
                                            }
                                        }
                                        if (config.ui.desktop_separate_player && sessions.activate(&id, if matches!(operation,super::surface::Operation::Key(_)){None}else{Some(revision)}, state)) || (!config.ui.desktop_separate_player && active_surface.as_ref().is_some_and(|s|s.id==id && (s.revision==revision || matches!(operation,super::surface::Operation::Key(_))))) {
                                            apply_surface_operation(operation,state,config)}else{vec![]}
                                    },
                                    Some(Command::SurfaceEdit {id,field,value}) => {
                                        if let Some((generation,title,selected))=sidebar_group.as_mut()
                                            && *generation==state.library_generation && title=="Playlists" && field==0 && id==format!("{generation}:sidebar-group:{title}") {
                                            state.playlist_group_filter=Some(value);
                                            *selected=0;
                                            continue;
                                        }
                                        if super::workspace::project(state).is_some_and(|model|model.id==id) {
                                            for action in super::surface::edit(state,field,value) {app::dispatch::dispatch_action(action,state,audio,config,&events).await?;}
                                            continue;
                                        }
                                        if (config.ui.desktop_separate_player && sessions.activate(&id,None,state)) || (!config.ui.desktop_separate_player && active_surface.as_ref().is_some_and(|s|s.id==id)) {super::surface::edit(state,field,value)}else{vec![]}
                                    },
                                    Some(Command::GuardedActions {generation,target,actions}) => {
                                        if generation==state.library_generation && target.as_ref().is_none_or(|t|super::browser::valid(state,t)) {actions} else {vec![]}
                                    },
                                    Some(Command::JumpLetter {generation,column,identity,letter,second}) => {
                                        defer_inspection=true;changes=ProjectionChange::Selection;

                                        let columns=super::browser::columns(state);
                                        let target=columns.get(column).filter(|c| generation==state.library_generation && c.identity==identity)
                                            .and_then(|c|super::browser::jump(c,letter,second).and_then(|row|c.target(generation,column,row)));
                                        if let Some(target)=target {
                                            desktop_focus=Some((generation,state.browse_category,column));
                                            navigation_serial+=1;navigation_origin=super::browser::Origin::AlphabetJump;
                                            select_target(state,&target,super::browser::Selection::Single,super::browser::Intent::Inspect)
                                        } else {vec![]}
                                    },
                                    Some(Command::SaveConnection {owner, draft}) => super::connection::begin(owner, draft, state, &mut connection),
                                    Some(Command::CancelConnection(owner)) => {super::connection::cancel(owner, state, &mut connection); vec![]},
                                    Some(Command::Seek {playback_id,fraction}) => seek_action(state,playback_id,fraction).into_iter().collect(),
                                    Some(Command::FocusDetails {generation,navigation_serial:serial}) => {
                                        if generation==state.library_generation && serial==navigation_serial && super::catalog::preview(state).is_some() {
                                            state.track_pane_focused=true;
                                            state.category_column_focused=false;
                                            state.focus=app::state::Focus::Right;
                                        }
                                        vec![]
                                    },
                                    Some(Command::ClosePreview {generation,navigation_serial: serial}) => {
                                        if generation==state.library_generation && serial==navigation_serial {
                                            inspection.cancel(state);
                                            close_details(state);
                                            desktop_focus=None;
                                        }
                                        vec![]
                                    },
                                    Some(Command::CloseColumn {generation,column,identity}) => {
                                        if close_column(state,&mut inspection,generation,column,&identity) {
                                            desktop_focus=None;
                                            navigation_serial+=1;navigation_origin=super::browser::Origin::Pointer;
                                        }
                                        vec![]
                                    },
                                    Some(Command::FocusColumn {generation,column,identity}) => {
                                        if generation==state.library_generation && super::browser::columns(state).get(column).is_some_and(|c|c.identity==identity) {
                                            if config.ui.desktop_separate_player {sessions.leave_for_browse(state);}
                                            app::handlers::key_input::close_browse_column_at(state,column+1);
                                            if let Some(nav)=state.browse_nav_mut() {nav.focused_column=column;}
                                            if let Some(nav)=state.folder_state.as_mut() {nav.focused_column=column;}
                                            desktop_focus=Some((generation,state.browse_category,column));
                                            navigation_serial+=1;navigation_origin=super::browser::Origin::Keyboard;
                                        }
                                        vec![]
                                    },
                                    Some(Command::Palette(command)) => {
                                        if matches!(command,PaletteCommandKind::SetApplicationMode(_)|PaletteCommandKind::CycleApplicationMode)
                                            && let Some(actions)=inspection.take(state,std::time::Instant::now(),true) {
                                                for action in actions {app::dispatch::dispatch_action(action,state,audio,config,&events).await?;}
                                            }
                                        if state.windowed_presentation.active()==Some(app::launch::WindowedMode::Text) {
                                            for action in app::command_palette::run(command,state) {
                                                app::dispatch::dispatch_action(action,state,audio,config,&events).await?;
                                            }
                                            continue;
                                        }
                                        if matches!(command,PaletteCommandKind::OpenAiBrowser) {
                                            let _ = actor_commands.try_send(Command::SidebarGroup{generation:state.library_generation,title:"AI".into()});
                                            continue;
                                        }
                                        if matches!(command,PaletteCommandKind::CloseColumn) && sidebar_group.is_some() {sidebar_group=None;sidebar_cursor=None;continue;}
                                        if matches!(command,PaletteCommandKind::GotoQueue) {for action in app::command_palette::run(command,state) {app::dispatch::dispatch_action(action,state,audio,config,&events).await?;}continue;}
                                        if matches!(command,PaletteCommandKind::OpenSearch) {
                                            surface_request=surface_request.wrapping_add(1);
                                            if super::workspace::owns(state) {
                                                for action in super::surface::cancel(state,config) {app::dispatch::dispatch_action(action,state,audio,config,&events).await?;}
                                            }
                                        }
                                        if matches!(command,PaletteCommandKind::OpenRadioBrowser|PaletteCommandKind::ConfigureStation(_)|PaletteCommandKind::Downloads(app::downloads::Menu::Open|app::downloads::Menu::Manage)) {
                                            sessions.leave_for_browse(state);
                                            library_window_request=library_window_request.wrapping_add(1);
                                        }
                                        if matches!(command,PaletteCommandKind::CloseColumn) && state.track_pane_focused && super::catalog::preview(state).is_some() {
                                            inspection.cancel(state);
                                            close_details(state);
                                            desktop_focus=None;
                                            vec![]
                                        } else if matches!(command,PaletteCommandKind::CloseColumn) && inspection.pending_column().is_some() {
                                            inspection.cancel(state);
                                            navigation_serial+=1;navigation_origin=super::browser::Origin::Keyboard;
                                            vec![]
                                        } else {
                                            if matches!(command,PaletteCommandKind::JumpToArtist(_)|PaletteCommandKind::JumpToAlbum(_)|PaletteCommandKind::GoToRandomAlbum) {inspection.cancel(state);sidebar_cursor=None;desktop_focus=None;}
                                            super::surface::navigate(command, state,config)
                                        }
                                    },
                                    Some(Command::SeekRelative {playback_id,delta_ms}) => if playback_id==state.playback.request_id {vec![crate::app::action::PlaybackAction::SeekRelative(delta_ms).into()]} else {vec![]},
                                    Some(Command::Activate {following}) => {
                                        if super::radio::visible(state) && sidebar_cursor.is_none() && super::surface::project(state).is_none() {
                                            for action in super::radio::key(state,crossterm::event::KeyCode::Enter) {app::dispatch::dispatch_action(action,state,audio,config,&events).await?;}
                                            continue;
                                        }
                                        inspection.cancel(state);

                                        if config.ui.desktop_separate_player {sessions.leave_for_browse(state);}
                                        if let Some(index)=sidebar_cursor {let _=actor_commands.try_send(Command::SidebarOpen{generation:state.library_generation,index});continue;}

                                        navigation_serial+=1;navigation_origin=super::browser::Origin::Keyboard;
                                        activate(state, following)
                                    },
                                    Some(Command::Navigate {column_delta, row_delta}) => {
                                        if super::radio::visible(state) && sidebar_cursor.is_none() && super::surface::project(state).is_none() {
                                            use crossterm::event::KeyCode;
                                            let key=if column_delta<0 {KeyCode::Left}else if column_delta>0 {KeyCode::Right}else if row_delta<0 {KeyCode::Up}else{KeyCode::Down};
                                            for action in super::radio::key(state,key) {app::dispatch::dispatch_action(action,state,audio,config,&events).await?;}
                                            continue;
                                        }
                                        if column_delta>0 {if let Some(actions)=inspection.take(state,std::time::Instant::now(),true){for action in actions{sessions.dispatch(action,state,audio,config,&events).await?;}}}
                                        else if column_delta<0 {inspection.cancel(state);}
                                        if row_delta!=0 && sidebar_cursor.is_none() && !state.list_filter.active {defer_inspection=true;changes=ProjectionChange::Selection;}

                                        if config.ui.desktop_separate_player {sessions.leave_for_browse(state);}
                                        if sidebar_cursor.is_some() {
                                            if let Some(command)=super::sidebar::navigate(state,&mut sidebar_cursor,column_delta,row_delta) {let _=actor_commands.try_send(command);}
                                            sidebar_inspection.schedule(state,sidebar_cursor,std::time::Instant::now());
                                            continue;
                                        }
                                        if state.track_pane_focused && column_delta<0 {
                                            state.track_pane_focused=false;
                                            continue;
                                        }
                                        if state.track_pane_focused && row_delta!=0 {continue;}
                                        let first=super::browser::first_content_column(state);
                                        let focused=state.browse_nav().map(|n|n.focused_column).or_else(||state.folder_state.as_ref().map(|n|n.focused_column)).unwrap_or(first);
                                        if column_delta<0 && state.view==app::View::Browse && focused<=first {
                                            let rows=super::catalog::sidebar(state);
                                            let visible=super::sidebar::visible_rows(&rows,&state.desktop_collapsed_sections);
                                            sidebar_cursor=visible.iter().find(|i|rows[**i].active).copied().or_else(||visible.first().copied());
                                            desktop_focus=None;
                                            sidebar_inspection.schedule(state,sidebar_cursor,std::time::Instant::now());
                                            continue;
                                        }

                                        if column_delta>0 && super::catalog::preview(state).is_some() && (album_info_selected(state) || state.browse_nav().is_some_and(|nav| nav.focused_column+1>=nav.columns.len() || nav.columns.get(nav.focused_column+1).is_some_and(|c|c.details.is_some()))) {
                                            state.track_pane_focused=true;
                                            navigation_serial+=1;navigation_origin=super::browser::Origin::Keyboard;
                                            continue;
                                        }
                                        navigation_serial+=1;navigation_origin=super::browser::Origin::Keyboard;
                                        let position = if matches!(state.view,app::View::Queue|app::View::NowPlaying) {Some((0,state.list_state.queue_index,state.playback_tracks().len(),1))} else if state.browse_category == app::BrowseCategory::Folders {
                                            state.folder_state.as_ref().and_then(|n| n.focused().map(|c| (n.focused_column, c.selected_index, c.items.len(), n.columns.len())))
                                        } else { state.browse_nav().and_then(|n| n.focused().map(|c| (n.focused_column, c.selected_index, c.items.len(), n.columns.len()))) };
                                        if let Some((column, row, _count, columns)) = position {
                                            let target = column.saturating_add_signed(column_delta as isize).min(columns.saturating_sub(1)).max(super::browser::first_content_column(state).min(columns.saturating_sub(1)));
                                            desktop_focus = Some((state.library_generation, state.browse_category, target));
                                            focus_browse_column(state,target);
                                            if let Some(nav) = state.folder_state.as_mut() { nav.focused_column = target; }
                                            if row_delta != 0 {
                                                state.focus=app::state::Focus::Right;state.category_column_focused=false;
                                                let key=if row_delta>0 {crossterm::event::KeyCode::Down}else{crossterm::event::KeyCode::Up};
                                                let key=crossterm::event::KeyEvent::new(key,crossterm::event::KeyModifiers::NONE);
                                                if state.view==app::View::NowPlaying {queue_key(state,key,config)}else if state.view==app::View::Browse && (state.browse_category==app::BrowseCategory::Playlists || state.list_filter.active) {navigate_browser_rows(state,row_delta)}else{
                                                    let mut actions=vec![];
                                                    for _ in 0..row_delta.unsigned_abs() {actions=app::handlers::key_input::handle_column_navigation(key,state);}
                                                    actions
                                                }
                                            }
                                            else if column_delta > 0 && target == column {
                                                select(state,column,row,false)
                                            } else { vec![] }
                                        } else {vec![]}
                                    },
                                    Some(Command::Select {target, mode, origin, intent}) => {
                                        inspection.cancel(state);

                                        if config.ui.desktop_separate_player && target.column!=super::browser::PLAYER_QUEUE && super::browser::valid(state,&target) {sessions.leave_for_browse(state);}
                                        sidebar_cursor=None;
                                        if !super::browser::valid(state,&target) {vec![]}
                                        else {
                                            navigation_serial+=1;navigation_origin=origin;
                                            if target.column!=super::browser::PLAYER_QUEUE {desktop_focus=Some((target.generation,state.browse_category,target.column));}
                                            select_target(state,&target,mode,intent)
                                        }
                                    },
                                    None => vec![SystemAction::Quit.into()],
                                }},
                                Some(event) = completions.recv() => sessions.event(event, state, &events),
                                _ = text_timer.tick(), if state.windowed_presentation.active()==Some(app::launch::WindowedMode::Text) && text_continuous => {
                                    changes=ProjectionChange::Playback;vec![]
                                },
                                _ = timer.tick() => {
                                    if graphical&& let Some(command)=sidebar_inspection.take(state,sidebar_cursor,std::time::Instant::now()) {let _=actor_commands.try_send(command);}
                                    let polled=clock.poll(state,audio);
                                    let passive=polled.iter().all(|event| matches!(event,app::Event::Tick));
                                    let mut actions:Vec<_>=polled.into_iter().flat_map(|event|app::dispatch::handle_core_event(event,state,&events)).collect();
                                    actions.extend(app::player_sync::poll(state, config));
                                    if let Some(ready)=inspection.take(state,std::time::Instant::now(),false){actions.extend(ready);changes=ProjectionChange::Content;}else{changes=if !passive || !actions.is_empty() {ProjectionChange::Content} else {ProjectionChange::Playback};}
                                    actions
                                },
                            };
                            if defer_inspection {
                                let focused=state.browse_nav().map(|nav|nav.focused_column).or_else(||state.folder_state.as_ref().map(|nav|nav.focused_column));
                                if actions.is_empty()
                                    && let Some(column)=focused.filter(|column| presentation.columns.len()>column+1) {
                                        let row=state.browse_nav().and_then(|nav|nav.columns.get(column).map(|c|c.selected_index)).or_else(||state.folder_state.as_ref().and_then(|nav|nav.columns.get(column).map(|c|c.selected_index)));
                                        if let Some(row)=row {actions=select(state,column,row,false);}
                                    }
                                inspection.schedule(state,std::mem::take(&mut actions),std::time::Instant::now(),presentation.clone());
                            }
                            for action in actions {
                                if state.windowed_presentation.active()!=Some(app::launch::WindowedMode::Text) {
                                match &action {
                                    Action::Search(app::action::SearchAction::OpenArtistRadioPicker) => {state.desktop_library_root=app::state::LibraryRoot::Radio;state.view=app::View::Browse;state.browse_category=app::BrowseCategory::Library;state.sources.nav_collection=None;state.desktop_player_visible=false;library_window_request=library_window_request.wrapping_add(1);},
                                    Action::Search(app::action::SearchAction::OpenAdventureLauncher|app::action::SearchAction::OpenAdventureLauncherWithStart{..}) => {state.desktop_library_root=app::state::LibraryRoot::Journey;state.view=app::View::Browse;state.browse_category=app::BrowseCategory::Library;state.sources.nav_collection=None;state.desktop_player_visible=false;library_window_request=library_window_request.wrapping_add(1);},
                                    Action::Navigation(app::action::NavigationAction::SetView(app::View::Queue)) => {
                                        queue_open = true;
                                        state.tui_queue_visible = true;
                                        queue_request = queue_request.wrapping_add(1);
                                        state.palette.close();
                                        continue;
                                    },
                                    Action::Navigation(app::action::NavigationAction::SetView(app::View::NowPlaying))=>{
                                        state.windowed_presentation.set_workspace(app::windowed_presentation::Workspace::NowPlaying);
                                        if state.desktop_split_view {library_window_request=library_window_request.wrapping_add(1);state.desktop_player_visible=false;state.desktop_player_fullscreen=app::state::DesktopFullscreenView::Closed;state.palette.close();}
                                        else {player_window_request=player_window_request.wrapping_add(1);state.desktop_player_visible=true;}
                                        continue;
                                    },
                                    Action::Navigation(app::action::NavigationAction::ToggleFullscreen)=>{
                                        state.desktop_player_fullscreen=match state.desktop_player_fullscreen {app::state::DesktopFullscreenView::Closed=>app::state::DesktopFullscreenView::Visualizer,_=>app::state::DesktopFullscreenView::Closed};
                                        if state.desktop_player_fullscreen.is_open(){player_window_request=player_window_request.wrapping_add(1);state.desktop_player_visible=true;}state.palette.close();continue;
                                    },
                                    Action::Navigation(app::action::NavigationAction::DesktopFullscreen(mode))=>{state.desktop_player_fullscreen = *mode;player_window_request=player_window_request.wrapping_add(1);state.desktop_player_visible=true;state.palette.close();continue;},
                                    Action::Navigation(app::action::NavigationAction::FullscreenArtwork)=>{state.desktop_player_fullscreen=app::state::DesktopFullscreenView::Artwork;player_window_request=player_window_request.wrapping_add(1);state.desktop_player_visible=true;state.palette.close();continue;},
                                    Action::Navigation(app::action::NavigationAction::SetView(app::View::Browse)) | Action::Navigation(app::action::NavigationAction::SetCategory{..})=>{state.windowed_presentation.set_workspace(app::windowed_presentation::Workspace::Library);library_window_request=library_window_request.wrapping_add(1);if !config.ui.desktop_separate_player {state.desktop_player_visible=false;state.desktop_player_fullscreen=app::state::DesktopFullscreenView::Closed;}},
                                    _=>{},
                                }
                                }
                                if matches!(&action,Action::Browse(app::action::BrowseAction::OpenInLibrary{..})) {state.windowed_presentation.set_workspace(app::windowed_presentation::Workspace::Library);sidebar_cursor=None;desktop_focus=None;inspection.cancel(state);navigation_serial+=1;navigation_origin=super::browser::Origin::LibraryJump;}
                                let jump = matches!(&action,Action::Browse(app::action::BrowseAction::OpenInLibrary{..}));
                                let undo = matches!(&action,Action::Navigation(app::action::NavigationAction::Undo|app::action::NavigationAction::Back));
                                let previous_restore = state.jump_history.restored;
                                let previous_jump = state.jump_history.head();
                                let playlist_revision = state.playlist_revision;
                                let cursor_playlist = sidebar_cursor.and_then(|index| updates.borrow().sidebar.get(index).and_then(|row| row.playlist_key.clone()));
                                let dispatched=if state.windowed_presentation.active()==Some(app::launch::WindowedMode::Text) {
                                    app::dispatch::dispatch_action(action,state,audio,config,&events).await
                                }else{sessions.dispatch(action,state,audio,config,&events).await};
                                if let Err(error)=dispatched {state.set_status(error.to_string());}
                                if state.playlist_revision != playlist_revision
                                    && let Some(key) = cursor_playlist { sidebar_cursor = super::catalog::sidebar(state).iter().position(|row| row.playlist_key.as_ref() == Some(&key)); }
                                let nested_jump = !jump && state.jump_history.head()!=previous_jump && state.jump_history.restored==previous_restore;
                                if nested_jump {sidebar_cursor=None;desktop_focus=None;inspection=Default::default();navigation_serial+=1;navigation_origin=super::browser::Origin::LibraryJump;}
                                if jump || nested_jump {sidebar_group=None;library_window_request=library_window_request.wrapping_add(1);if !config.ui.desktop_separate_player {state.desktop_player_visible=false;state.desktop_player_fullscreen=app::state::DesktopFullscreenView::Closed;}}
                                if undo && previous_restore!=state.jump_history.restored {navigation_serial+=1;navigation_origin=super::browser::Origin::LibraryJump;if state.desktop_player_visible && !state.desktop_split_view {player_window_request=player_window_request.wrapping_add(1);}else {library_window_request=library_window_request.wrapping_add(1);}}
                            }
                            if state.should_quit { break; }
                        }
                        Ok::<_, anyhow::Error>(())
                    }).await;
                    drop(tasks);
                    if !live.shutdown().await { anyhow::bail!("Background work timed out at shutdown"); }
                    result
                });
                runtime.shutdown_timeout(std::time::Duration::from_secs(3));
                result
            })();
            if let Err(error) = result {
                let message = error.to_string();
                let _ = ready.send(Err(message.clone()));
                let previous = updates.borrow().clone();
                updates.send_replace(Snapshot { status: message, ..previous });
            }
        })?;
        initialized.recv()?.map_err(anyhow::Error::msg)?;
        Ok(Self {
            commands: CommandPort::new(commands, 1),
            snapshot,
            worker: Some(worker),
        })
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self
            .commands
            .blocking_send(Command::Action(SystemAction::Quit.into()));
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn select_player_mode(state: &mut AppState, mode: usize) {
    use app::state::DesktopFullscreenView as FullscreenView;
    if mode == 0 {
        state.desktop_player_fullscreen = if state.desktop_player_fullscreen.has_transport() {
            FullscreenView::ArtworkTransport
        } else {
            FullscreenView::Artwork
        };
    } else {
        state.visualizer_tab = app::state::VisualizerTab::ALL
            [(mode - 1).min(app::state::VisualizerTab::ALL.len() - 1)];
        if state.desktop_player_fullscreen.is_open() {
            state.desktop_player_fullscreen = if state.desktop_player_fullscreen.has_transport() {
                FullscreenView::VisualizerTransport
            } else {
                FullscreenView::Visualizer
            };
        }
    }
}

fn queue_key(
    state: &mut AppState,
    key: crossterm::event::KeyEvent,
    config: &crate::config::Config,
) -> Vec<Action> {
    if key.code == crossterm::event::KeyCode::Enter {
        return super::queue::play_at(state, state.list_state.queue_index);
    }
    let view = state.view;
    let focus = state.focus;
    let category_focus = state.category_column_focused;
    state.view = app::View::Queue;
    state.now_playing_focus = app::state::NowPlayingFocus::Tracks;
    let actions = app::handlers::key_input::handle_key(key, state, config);
    state.view = view;
    if view == app::View::Browse {
        state.focus = focus;
        state.category_column_focused = category_focus;
    }
    actions
}

fn focus_browse_column(state: &mut AppState, target: usize) {
    if let Some(nav) = state.browse_nav_mut() {
        let entering = nav.focused_column != target;
        if let Some(column) = nav.columns.get_mut(target) {
            if column.has_album_details() {
                if entering {
                    column.header_selection = app::state::ColumnHeaderSelection::AlbumDetails;
                }
            } else {
                column.header_selection = app::state::ColumnHeaderSelection::Items;
            }
            nav.focused_column = target;
        }
    }
}

fn album_info_selected(state: &AppState) -> bool {
    state
        .browse_nav()
        .and_then(|n| n.focused())
        .is_some_and(|c| {
            c.has_album_details()
                && c.header_selection == app::state::ColumnHeaderSelection::AlbumDetails
        })
}
fn select_album_info(state: &mut AppState, column: usize) {
    if let Some(nav) = state.browse_nav_mut()
        && nav
            .columns
            .get(column)
            .is_some_and(|c| c.has_album_details())
    {
        nav.focused_column = column;
        nav.columns.truncate(column + 1);
        nav.columns[column].header_selection = app::state::ColumnHeaderSelection::AlbumDetails;
        state.category_column_focused = false;
        state.focus = app::state::Focus::Right;
    }
}

// Resolve keyboard activation in the actor, after earlier pointer/navigation commands.
// A UI snapshot can still contain the previous selection during a rapid click + Enter.
fn activate(state: &mut AppState, following: bool) -> Vec<Action> {
    if album_info_selected(state) {
        return vec![];
    }
    let column = if matches!(state.view, app::View::Queue | app::View::NowPlaying) {
        0
    } else if state.browse_category == app::BrowseCategory::Folders {
        state
            .folder_state
            .as_ref()
            .map_or(0, |nav| nav.focused_column)
    } else {
        state.browse_nav().map_or(0, |nav| nav.focused_column)
    };
    let Some(projected) = super::browser::columns(state).get(column).cloned() else {
        return vec![];
    };
    let Some(target) = projected.target(state.library_generation, column, projected.selected)
    else {
        return vec![];
    };
    select_target(
        state,
        &target,
        super::browser::Selection::Single,
        if following {
            super::browser::Intent::PlayFollowing
        } else {
            super::browser::Intent::Play
        },
    )
}

fn select(state: &mut AppState, column: usize, row: usize, play: bool) -> Vec<Action> {
    use app::action::{FolderAction, QueueAction};
    if column == super::browser::PLAYER_QUEUE
        || matches!(state.view, app::View::Queue | app::View::NowPlaying)
    {
        return if play && row < state.playback_tracks().len() {
            if state.playback_mode == app::state::PlaybackMode::Radio {
                vec![app::action::RadioAction::JumpToRadioTrack(row).into()]
            } else {
                vec![QueueAction::JumpToQueueIndex(row).into()]
            }
        } else {
            vec![]
        };
    }
    if state.browse_category == app::BrowseCategory::Folders {
        let Some(nav) = state.folder_state.as_mut() else {
            return vec![];
        };
        let Some(item) = nav
            .columns
            .get(column)
            .and_then(|c| c.items.get(row))
            .cloned()
        else {
            return vec![];
        };
        nav.focused_column = column;
        nav.columns[column].selected_index = row;
        nav.columns.truncate(column + 1);
        return if item.item_type == crate::library::models::FolderItemType::Folder {
            vec![
                FolderAction::NavigateIntoFolder {
                    folder_key: item.key,
                    replace_child: true,
                }
                .into(),
            ]
        } else if play {
            vec![FolderAction::PlayFolderTrack { track_index: row }.into()]
        } else {
            vec![]
        };
    }
    let Some(nav) = state.browse_nav_mut() else {
        return vec![];
    };
    if nav
        .columns
        .get(column)
        .and_then(|c| c.items.get(row))
        .is_none()
    {
        return vec![];
    }
    nav.focused_column = column;
    nav.columns[column].selected_index = row;
    nav.columns.truncate(column + 1);
    if play {
        match nav.columns[column].items[row].clone() {
            app::state::BrowseItem::Album { key, title, .. } => {
                return vec![
                    QueueAction::PlayAlbumNow {
                        rating_key: key,
                        title,
                    }
                    .into(),
                ];
            }
            app::state::BrowseItem::Playlist { key, title, .. } => {
                return vec![
                    QueueAction::PlayPlaylistNow {
                        playlist_key: key,
                        title,
                    }
                    .into(),
                ];
            }
            app::state::BrowseItem::ArtistInfo {
                artist_key,
                artist_name,
                ..
            } => {
                return vec![
                    app::action::SearchAction::ShowArtistInfo {
                        artist_key,
                        artist_name,
                    }
                    .into(),
                ];
            }
            _ => {}
        }
        if let Some(track) = nav.columns[column].tracks.get(row).cloned() {
            return vec![QueueAction::PlayTrack(Box::new(track)).into()];
        }
    }
    state.list_filter.category = state.browse_category;
    app::handlers::key_input::get_filter_drilldown_actions(state)
}

fn navigate_browser_rows(state: &mut AppState, delta: i8) -> Vec<Action> {
    let column = state
        .browse_nav()
        .map(|nav| nav.focused_column)
        .or_else(|| state.folder_state.as_ref().map(|nav| nav.focused_column))
        .unwrap_or(0);
    let Some(data) = super::browser::columns(state).get(column).cloned() else {
        return vec![];
    };
    let filtered = (state.list_filter.active
        && state.list_filter.column == column
        && !state.list_filter.query.trim().is_empty())
    .then_some(state.list_filter.results.as_ref())
    .flatten();
    let row = if let Some(filtered) = filtered {
        let position = filtered
            .matched_indices
            .iter()
            .position(|row| *row == data.selected)
            .unwrap_or(0);
        filtered
            .matched_indices
            .get(
                position
                    .saturating_add_signed(delta as isize)
                    .min(filtered.matched_indices.len().saturating_sub(1)),
            )
            .copied()
    } else {
        (!data.rows.is_empty()).then(|| {
            data.selected
                .saturating_add_signed(delta as isize)
                .min(data.rows.len() - 1)
        })
    };
    let Some(target) = row.and_then(|row| data.target(state.library_generation, column, row))
    else {
        return vec![];
    };
    let inspect = data.rows[target.row].kind != super::browser::Kind::Track;
    select_target(
        state,
        &target,
        super::browser::Selection::Single,
        if inspect {
            super::browser::Intent::Inspect
        } else {
            super::browser::Intent::Focus
        },
    )
}

fn seek_action(state: &AppState, playback_id: u64, fraction: f32) -> Option<Action> {
    (playback_id == state.playback.request_id
        && fraction.is_finite()
        && state.playback.duration_ms > 0)
        .then(|| {
            crate::app::action::PlaybackAction::Seek(
                (fraction.clamp(0., 1.) as f64 * state.playback.duration_ms as f64).round() as u64,
            )
            .into()
        })
}
fn select_target(
    state: &mut AppState,
    target: &super::browser::Target,
    mode: super::browser::Selection,
    intent: super::browser::Intent,
) -> Vec<Action> {
    use super::browser::selection;
    if !super::browser::valid(state, target) {
        return vec![];
    }
    use app::sources::navidrome::{NavAction, commands::Command as Server, playlist_edit};
    if let super::browser::Intent::EditPlaylist(edit) = intent {
        return state
            .browse_nav()
            .and_then(|n| n.columns.get(target.column))
            .and_then(|c| playlist_edit::row_command(state, c, target.row, edit))
            .map(|c| vec![NavAction::Command(c).into()])
            .unwrap_or_default();
    }
    if intent == super::browser::Intent::AddTracksToPlaylist {
        return super::browser::target_column(state, target.column)
            .and_then(|c| playlist_edit::addition(&c.tracks, &c.selected_set, target.row))
            .map(|a| vec![NavAction::Command(Server::ChoosePlaylist(a)).into()])
            .unwrap_or_default();
    }
    if intent == super::browser::Intent::OpenAlbumDetails {
        return state
            .browse_nav()
            .and_then(|nav| nav.columns.get(target.column))
            .and_then(|column| column.items.get(target.row))
            .and_then(app::library_details::browse_destination)
            .map(|(track, destination)| {
                vec![app::action::BrowseAction::OpenDetailsInLibrary {
                    track: Box::new(track),
                    destination,
                }.into()]
            })
            .unwrap_or_default();
    }
    if intent == super::browser::Intent::AddAlbumToPlaylist {
        return vec![
            NavAction::Command(Server::ChoosePlaylist(playlist_edit::Addition::Album(
                target.key.clone(),
            )))
            .into(),
        ];
    }
    if intent == super::browser::Intent::ManageDownload {
        return state
            .browse_nav()
            .and_then(|nav| nav.columns.get(target.column))
            .and_then(|column| app::downloads::item_command(state, column, target.row))
            .map(|(_, command)| vec![command.into()])
            .unwrap_or_default();
    }
    let projected = super::browser::target_column(state, target.column).expect("validated target");
    if projected.rows[target.row].download_id.is_some() {
        return vec![];
    }
    if intent == super::browser::Intent::Download {
        let Some(column) = state
            .browse_nav()
            .and_then(|nav| nav.columns.get(target.column))
        else {
            return vec![];
        };
        return match column.items.get(target.row) {
            Some(app::state::BrowseItem::Album { key, .. }) => {
                vec![app::downloads::Command::Album(key.clone()).into()]
            }
            Some(app::state::BrowseItem::Playlist { key, .. }) => {
                vec![app::downloads::Command::Playlist(key.clone()).into()]
            }
            _ => vec![],
        };
    }
    if target.column != super::browser::PLAYER_QUEUE {
        state.track_pane_focused = false;
        state.focus = app::state::Focus::Right;
        state.category_column_focused = false;
    }
    let multi = !matches!(
        projected.rows[target.row].kind,
        super::browser::Kind::Info | super::browser::Kind::Description
    );
    if target.column == super::browser::PLAYER_QUEUE
        || matches!(state.view, app::View::Queue | app::View::NowPlaying)
    {
        let mut selected = state.list_state.queue_index;
        selection(
            &mut selected,
            &mut state.queue.selected,
            &mut state.queue.selection_anchor,
            target.row,
            mode,
            multi,
        );
        state.list_state.queue_index = selected;
    } else if state.browse_category == app::BrowseCategory::Folders {
        if let Some(nav) = state.folder_state.as_mut() {
            nav.focused_column = target.column;
            let c = &mut nav.columns[target.column];
            selection(
                &mut c.selected_index,
                &mut c.selected_set,
                &mut c.selection_anchor,
                target.row,
                mode,
                multi,
            );
        }
    } else if let Some(nav) = state.browse_nav_mut() {
        nav.focused_column = target.column;
        if projected.playlist_editing
            && mode == super::browser::Selection::Extend
            && (nav.columns[target.column].grouped_by_album
                || !nav.columns[target.column].playlist_positions.is_empty())
        {
            let col = &nav.columns[target.column];
            let start = col.selection_anchor.unwrap_or(col.selected_index);
            playlist_edit::mark_group_rows(
                &mut nav.columns,
                target.column,
                (start.min(target.row)..=start.max(target.row)).map(|row| (row, Some(true))),
            );
            nav.columns[target.column].selected_index = target.row;
            return vec![];
        }
        if projected.playlist_editing
            && matches!(
                mode,
                super::browser::Selection::Check | super::browser::Selection::Toggle
            )
            && playlist_edit::mark_group_row(&mut nav.columns, target.column, target.row, None)
        {
            nav.columns[target.column].selection_anchor = Some(target.row);
            return vec![];
        }
        let c = &mut nav.columns[target.column];
        c.header_selection = app::state::ColumnHeaderSelection::Items;
        if projected.playlist_editing && (c.grouped_by_album || !c.playlist_positions.is_empty()) {
            c.selected_index = target.row;
        } else {
            selection(
                &mut c.selected_index,
                &mut c.selected_set,
                &mut c.selection_anchor,
                target.row,
                mode,
                multi,
            );
        }
    }
    if state.list_filter.active
        && state.list_filter.column == target.column
        && let Some(position) = state.list_filter.results.as_ref().and_then(|results| {
            results
                .matched_indices
                .iter()
                .position(|row| *row == target.row)
        })
    {
        state.list_filter.selected = position;
    }
    if intent == super::browser::Intent::Focus {
        return vec![];
    }
    if intent == super::browser::Intent::PlayFollowing {
        if state.browse_category == app::BrowseCategory::Folders {
            return vec![app::action::FolderAction::PlayFolderTracks.into()];
        }
        if !matches!(state.view, app::View::Queue | app::View::NowPlaying)
            && let Some(column) = state
                .browse_nav()
                .and_then(|nav| nav.columns.get(target.column))
        {
            let tracks = column.ready_tracks_from(target.row);
            if !tracks.is_empty() {
                return vec![app::action::QueueAction::PlayTracksNow(tracks).into()];
            }
        }
    }
    let mut actions = select(
        state,
        target.column,
        target.row,
        matches!(
            intent,
            super::browser::Intent::Play | super::browser::Intent::PlayFollowing
        ),
    );
    if multi
        && intent == super::browser::Intent::Inspect
        && target.column != super::browser::PLAYER_QUEUE
    {
        actions.push(app::action::BrowseAction::OpenTrackDetails.into());
    }
    actions
}

fn apply_surface_operation(
    operation: super::surface::Operation,
    state: &mut AppState,
    config: &mut crate::config::Config,
) -> Vec<Action> {
    let previous = state.view;
    let queue_context = state.palette.open && state.palette.player_context;
    if queue_context {
        state.view = app::View::Queue;
    }
    let actions = super::surface::apply(operation, state, config);
    if queue_context && state.view == app::View::Queue {
        state.view = previous;
    }
    actions
}

fn close_details(state: &mut AppState) {
    if let Some(nav) = state.browse_nav_mut()
        && let Some(index) = nav
            .columns
            .iter()
            .position(|column| column.details.is_some())
    {
        nav.columns.truncate(index);
        nav.focused_column = nav.focused_column.min(index.saturating_sub(1));
    }
    if let Some(column) = state.browse_nav_mut().and_then(|nav| nav.focused_mut()) {
        column.header_selection = app::state::ColumnHeaderSelection::Items;
    }
    state.track_pane_open = false;
    state.track_pane_focused = false;
    state.track_pane_index = 0;
    state.sonic_details = None;
}

fn close_column(
    state: &mut AppState,
    inspection: &mut super::navigation::Inspection,
    generation: u64,
    column: usize,
    identity: &str,
) -> bool {
    if generation != state.library_generation {
        return false;
    }
    let visible = inspection.project(super::navigation::Presentation {
        columns: super::browser::columns(state),
        ..Default::default()
    });
    if !visible
        .columns
        .get(column)
        .is_some_and(|current| !current.hidden && current.identity == identity)
    {
        return false;
    }
    inspection.cancel(state);
    app::handlers::key_input::close_browse_column_at(state, column);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::models::{FolderColumn, FolderItem, FolderNavigationState};

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires the isolated loopback catalog profile"]
    fn live_windowed_switch_preserves_columns_and_accepts_text_navigation() {
        assert!(
            crate::config::load_config().unwrap().navidrome_sources[0]
                .url
                .starts_with("http://127.0.0.1:")
        );
        let engine = Engine::start_mode(
            true,
            app::launch::Mode::Windowed(app::launch::WindowedMode::Gui),
        )
        .unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            async fn wait(
                updates: &mut watch::Receiver<Snapshot>,
                predicate: impl Fn(&Snapshot) -> bool,
            ) -> Snapshot {
                tokio::time::timeout(std::time::Duration::from_secs(12), async {
                    loop {
                        let snapshot = updates.borrow().clone();
                        if predicate(&snapshot) {
                            return snapshot;
                        }
                        updates.changed().await.unwrap();
                    }
                })
                .await
                .expect("live presentation transition")
            }
            let mut updates = engine.snapshot.clone();
            let initial = wait(&mut updates, |snapshot| {
                !snapshot.columns.is_empty() && snapshot.columns[0].rows.len() > 5
            })
            .await;
            engine
                .commands
                .send(Command::Palette(PaletteCommandKind::SetApplicationMode(
                    app::launch::ApplicationMode::Text,
                )))
                .await
                .unwrap();
            let preparing = wait(&mut updates, |snapshot| {
                snapshot.presentation.pending().is_some()
            })
            .await;
            let request = preparing.presentation.pending().unwrap();
            engine
                .commands
                .send(Command::TextViewport {
                    identity: request.identity,
                    viewport: crate::text_mode::backend::Viewport {
                        metrics: crate::text_mode::backend::Metrics {
                            columns: 130,
                            rows: 38,
                            cell_width: 18,
                            cell_height: 40,
                        },
                        scale_factor: 2.,
                        revision: 1,
                        geometry: Default::default(),
                        appearance: crate::text_mode::backend::Appearance {
                            system: None,
                            foreground: [255; 3],
                            background: [0; 3],
                        },
                    },
                })
                .await
                .unwrap();
            let text = wait(&mut updates, |snapshot| {
                snapshot.presentation.active() == Some(app::launch::WindowedMode::Text)
            })
            .await;
            assert_eq!(initial.generation, text.generation);
            assert!(text.text_frame.is_some());
            for code in [
                crossterm::event::KeyCode::Down,
                crossterm::event::KeyCode::Enter,
            ] {
                engine
                    .commands
                    .send(Command::TextInput {
                        identity: request.identity,
                        viewport_revision: None,
                        input: crate::text_mode::Input::Key(crossterm::event::KeyEvent::new(
                            code,
                            crossterm::event::KeyModifiers::NONE,
                        )),
                    })
                    .await
                    .unwrap();
            }
            let _navigated = wait(&mut updates, |snapshot| {
                snapshot.text_frame.as_ref().is_some_and(|frame| {
                    frame.frame.cells.iter().any(|row| {
                        row.iter()
                            .map(|cell| cell.text.as_str())
                            .collect::<String>()
                            .contains("albums")
                    })
                })
            })
            .await;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let navigated = updates.borrow().clone();
            let frame = navigated.text_frame.as_ref().unwrap();
            let text = frame
                .frame
                .cells
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|cell| cell.text.as_str())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains("C Artist"), "{text}");
            engine.commands.observe(request.identity);
            engine
                .commands
                .send(Command::Palette(PaletteCommandKind::SetApplicationMode(
                    app::launch::ApplicationMode::GuiSingle,
                )))
                .await
                .unwrap();
            let preparing = wait(&mut updates, |snapshot| {
                snapshot
                    .presentation
                    .pending()
                    .is_some_and(|request| request.destination == app::launch::WindowedMode::Gui)
            })
            .await;
            engine
                .commands
                .send(Command::PresentationReady(
                    preparing.presentation.pending().unwrap(),
                ))
                .await
                .unwrap();
            let graphical = wait(&mut updates, |snapshot| {
                snapshot.presentation.active() == Some(app::launch::WindowedMode::Gui)
                    && snapshot.columns.len() > 1
            })
            .await;
            assert_eq!(graphical.generation, initial.generation);
            assert_eq!(graphical.columns[0].selected, 1);
            assert!(graphical.columns[1].title.contains("A Artist"));
            assert_eq!(graphical.playback_id, initial.playback_id);
        });
    }

    #[test]
    fn header_close_cancels_the_retained_child_during_keyboard_inspection() {
        use app::state::{BrowseColumn, BrowseItem};
        let mut state = AppState::new();
        state.category_column_focused = false;
        state.artist_nav.columns = vec![
            BrowseColumn::new(
                "artists",
                ["A", "B"]
                    .into_iter()
                    .map(|name| BrowseItem::Artist {
                        key: name.into(),
                        title: name.into(),
                        thumb: None,
                        is_placeholder: false,
                    })
                    .collect(),
            ),
            BrowseColumn::new("albums A", vec![]),
        ];
        let retained = super::super::navigation::Presentation {
            columns: super::super::browser::columns(&state),
            ..Default::default()
        };
        let identity = retained.columns[1].identity.clone();
        state.artist_nav.columns[0].selected_index = 1;
        let mut inspection = super::super::navigation::Inspection::default();
        inspection.schedule(&mut state, vec![], std::time::Instant::now(), retained);
        state.artist_nav.columns.truncate(1);
        let generation = state.library_generation;
        assert!(inspection.pending_column().is_some());
        assert!(close_column(
            &mut state,
            &mut inspection,
            generation,
            1,
            &identity
        ));
        assert!(inspection.pending_column().is_none());
        assert_eq!(state.artist_nav.columns.len(), 1);
    }

    #[test]
    fn close_header_targets_its_column_and_rejects_stale_identity() {
        use app::state::BrowseColumn;
        let mut state = AppState::new();
        state.category_column_focused = false;
        state.artist_nav.columns = vec![
            BrowseColumn::new("artists", vec![]),
            BrowseColumn::new("albums", vec![]),
            BrowseColumn::new("tracks", vec![]),
        ];
        state.artist_nav.focused_column = 2;
        let generation = state.library_generation;
        let identity = super::super::browser::columns(&state)[1].identity.clone();
        let mut inspection = super::super::navigation::Inspection::default();
        assert!(!close_column(
            &mut state,
            &mut inspection,
            generation + 1,
            1,
            &identity
        ));
        assert!(!close_column(
            &mut state,
            &mut inspection,
            generation,
            1,
            "other-column"
        ));
        assert_eq!(state.artist_nav.columns.len(), 3);
        assert!(close_column(
            &mut state,
            &mut inspection,
            generation,
            1,
            &identity
        ));
        assert_eq!(state.artist_nav.columns.len(), 1);
        assert_eq!(state.artist_nav.focused_column, 0);
    }

    async fn wait_for_sidebar(
        updates: &mut watch::Receiver<Snapshot>,
        predicate: impl Fn(&Snapshot) -> bool,
    ) -> Snapshot {
        let result = tokio::time::timeout(std::time::Duration::from_secs(12), async {
            loop {
                let snapshot = updates.borrow().clone();
                if predicate(&snapshot) {
                    return snapshot;
                }
                updates.changed().await.unwrap();
            }
        })
        .await;
        result.unwrap_or_else(|_| {
            let snapshot = updates.borrow();
            panic!(
                "Sidebar did not load: section={}, status={}, loading={:?}, columns={:?}",
                snapshot.section,
                snapshot.status,
                snapshot.loading,
                snapshot
                    .columns
                    .iter()
                    .map(|column| (&column.title, column.rows.len(), column.hidden))
                    .collect::<Vec<_>>()
            );
        })
    }

    #[test]
    #[ignore = "requires an isolated XDG profile and the loopback desktop catalog fixture"]
    fn actor_leaves_audio_player_for_sidebar_destinations() {
        use app::state::LibraryRoot;
        assert!(
            std::env::var("TEXTAMP_ACTOR_FIXTURE_URL")
                .unwrap()
                .starts_with("http://127.0.0.1:")
        );
        let engine = Engine::start(true).unwrap();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let mut updates = engine.snapshot.clone();
            let catalog = wait_for_sidebar(&mut updates, |s| {
                s.sidebar.iter().any(|r| r.playlist_key.is_some())
            })
            .await;
            let targets: Vec<_> = catalog
                .sidebar
                .iter()
                .filter(|r| r.library_root != Some(LibraryRoot::Mp3Players) && !r.header)
                .map(|r| r.label.clone())
                .collect();
            for label in targets {
                for (destination, showing_player) in [
                    (LibraryRoot::Mp3Players.label(), true),
                    (label.as_str(), false),
                ] {
                    eprintln!("Opening {destination}");
                    let current = updates.borrow().clone();
                    let index = current
                        .sidebar
                        .iter()
                        .position(|r| !r.header && r.label == destination)
                        .unwrap();
                    engine
                        .commands
                        .send(Command::SidebarOpen {
                            generation: current.generation,
                            index,
                        })
                        .await
                        .unwrap();
                    wait_for_sidebar(&mut updates, |s| {
                        s.player_sync.visible == showing_player
                            && if destination == crate::audiomuse::Feature::Lyrics.label() {
                                s.surface.is_some() || !s.surfaces.is_empty()
                            } else {
                                s.sidebar.iter().any(|r| r.label == destination && r.active)
                            }
                    })
                    .await;
                }
            }
        });
    }
    #[test]
    #[ignore = "requires an isolated XDG profile and the loopback desktop catalog fixture"]
    fn actor_sidebar_destinations_load_during_slow_similarity_requests() {
        assert!(
            std::env::var("TEXTAMP_ACTOR_FIXTURE_URL")
                .unwrap()
                .starts_with("http://127.0.0.1:")
        );
        let engine = Engine::start(true).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut updates = engine.snapshot.clone();
            wait_for_sidebar(&mut updates, |snapshot| {
                snapshot
                    .sidebar
                    .iter()
                    .any(|row| row.playlist_key.is_some())
            })
            .await;
            for root in [
                app::state::LibraryRoot::Albums,
                app::state::LibraryRoot::Tracks,
                app::state::LibraryRoot::Artists,
            ] {
                let current = updates.borrow().clone();
                let index = current
                    .sidebar
                    .iter()
                    .position(|row| row.library_root == Some(root))
                    .unwrap();
                engine
                    .commands
                    .send(Command::SidebarOpen {
                        generation: current.generation,
                        index,
                    })
                    .await
                    .unwrap();
                let kind = match root {
                    app::state::LibraryRoot::Albums => super::super::browser::Kind::Album,
                    app::state::LibraryRoot::Tracks => super::super::browser::Kind::Track,
                    _ => super::super::browser::Kind::Artist,
                };
                wait_for_sidebar(&mut updates, |snapshot| {
                    !snapshot.player_sync.visible
                        && snapshot
                            .columns
                            .iter()
                            .filter(|column| !column.hidden)
                            .any(|column| column.rows.iter().any(|row| row.kind == kind))
                })
                .await;
            }
            let current = updates.borrow().clone();
            let index = current
                .sidebar
                .iter()
                .position(|row| row.playlist_key.is_some())
                .unwrap();
            engine
                .commands
                .send(Command::SidebarOpen {
                    generation: current.generation,
                    index,
                })
                .await
                .unwrap();
            wait_for_sidebar(&mut updates, |snapshot| {
                snapshot.section == "Playlists"
                    && snapshot
                        .columns
                        .iter()
                        .filter(|column| !column.hidden)
                        .any(|column| {
                            column
                                .rows
                                .iter()
                                .any(|row| row.kind == super::super::browser::Kind::Track)
                        })
            })
            .await;
            let current = updates.borrow().clone();
            let index = current
                .sidebar
                .iter()
                .position(|row| row.library_root == Some(app::state::LibraryRoot::Artists))
                .unwrap();
            let track = current
                .columns
                .iter()
                .enumerate()
                .find_map(|(column_index, column)| {
                    if column.hidden {
                        return None;
                    }
                    let row = column
                        .rows
                        .iter()
                        .position(|row| row.kind == super::super::browser::Kind::Track)?;
                    column.target(current.generation, column_index, row)
                })
                .unwrap();
            engine
                .commands
                .send(Command::Select {
                    target: track,
                    mode: super::super::browser::Selection::Single,
                    origin: super::super::browser::Origin::Pointer,
                    intent: super::super::browser::Intent::Focus,
                })
                .await
                .unwrap();
            engine
                .commands
                .send(Command::Action(
                    app::action::BrowseAction::OpenTrackDetails.into(),
                ))
                .await
                .unwrap();
            wait_for_sidebar(&mut updates, |snapshot| {
                snapshot.loading.as_deref() == Some("Loading similar tracks…")
            })
            .await;
            let started = std::time::Instant::now();
            engine
                .commands
                .send(Command::SidebarOpen {
                    generation: current.generation,
                    index,
                })
                .await
                .unwrap();
            wait_for_sidebar(&mut updates, |snapshot| {
                snapshot.columns.first().is_some_and(|column| {
                    column
                        .rows
                        .iter()
                        .any(|row| row.kind == super::super::browser::Kind::Artist)
                })
            })
            .await;
            assert!(started.elapsed() < std::time::Duration::from_secs(2));
        });
    }

    #[test]
    fn album_arrows_ignore_unrelated_workflow_and_terminal_detail_focus() {
        use app::state::{BrowseColumn, BrowseItem};
        let mut state = AppState::new();
        state.view = app::View::Browse;
        state.browse_category = app::BrowseCategory::Library;
        state.artist_nav.columns = vec![BrowseColumn::new(
            "Albums",
            (0..3)
                .map(|i| BrowseItem::Album {
                    key: i.to_string(),
                    title: format!("Album {i}"),
                    year: None,
                    artist: String::new(),
                    thumb: None,
                    is_placeholder: false,
                })
                .collect(),
        )];
        state.track_pane_focused = true;
        state.category_column_focused = true;
        let key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        app::handlers::key_input::handle_column_navigation(key, &mut state);
        assert_eq!(state.artist_nav.columns[0].selected_index, 1);
        assert!(!state.track_pane_focused);
        assert!(!state.category_column_focused);
        assert!(state.queue.tracks.is_empty());
    }

    #[test]
    fn album_info_keyboard_entry_keeps_tracks_playback_and_raw_targets_distinct() {
        use app::state::{BrowseColumn, BrowseItem, ColumnHeaderSelection, PlayAllRow};
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut state = AppState::new();
        state.desktop_frontend = true;
        state.track_pane_open = true;
        state.view = app::View::Browse;
        state.category_column_focused = false;
        state.focus = app::state::Focus::Right;
        let tracks: Vec<_> = (0..2)
            .map(|i| crate::library::models::Track {
                rating_key: format!("track-{i}"),
                title: format!("Track {i}"),
                parent_rating_key: Some("album".into()),
                parent_title: Some("Album".into()),
                ..Default::default()
            })
            .collect();
        let mut column =
            BrowseColumn::new_with_tracks("Album", BrowseItem::from_tracks(&tracks), tracks);
        column.play_all_row = Some(PlayAllRow::Album {
            rating_key: "album".into(),
            title: "Album".into(),
        });
        state.artist_nav.columns = vec![
            BrowseColumn::new(
                "Albums",
                vec![BrowseItem::Album {
                    key: "album".into(),
                    title: "Album".into(),
                    artist: "Artist".into(),
                    year: None,
                    thumb: None,
                    is_placeholder: false,
                }],
            ),
            column,
        ];
        assert!(super::super::catalog::preview(&state).is_none());
        focus_browse_column(&mut state, 1);
        assert!(album_info_selected(&state));
        assert_eq!(
            super::super::catalog::preview(&state).unwrap().title,
            "Album"
        );
        assert!(activate(&mut state, false).is_empty());
        for expected in [0, 1] {
            focus_browse_column(&mut state, 1);
            let actions = app::handlers::key_input::handle_key(
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                &mut state,
                &Default::default(),
            );
            assert!(actions.is_empty());
            let c = state.artist_nav.focused().unwrap();
            assert_eq!(c.selected_index, expected);
            assert_eq!(c.header_selection, ColumnHeaderSelection::Items);
            assert_eq!(
                super::super::catalog::preview(&state).unwrap().title,
                format!("Track {expected}")
            );
        }
        select_album_info(&mut state, 1);
        let snapshot = Snapshot {
            columns: super::super::browser::columns(&state),
            focused_column: 1,
            ..Default::default()
        };
        assert_eq!(
            super::super::filter::album_info_index(&snapshot, 1),
            Some(0)
        );
        assert_eq!(super::super::filter::row(&snapshot, 1, 1), Some(0));
        assert!(state.queue.tracks.is_empty());
        assert!(album_info_selected(&state));
    }
    #[tokio::test]
    async fn dismissing_a_temporary_reader_restores_the_library_view() {
        let mut state = AppState::new();
        state.view = app::View::Help;
        state.previous_view = Some(app::View::Browse);
        state.palette.player_context = true;
        let mut config = crate::config::Config::default();
        let actions = apply_surface_operation(
            super::super::surface::Operation::Key(crossterm::event::KeyCode::Esc),
            &mut state,
            &mut config,
        );
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        let (events, _) = tokio::sync::mpsc::channel(8);
        for action in actions {
            app::dispatch::dispatch_action(action, &mut state, &mut audio, &mut config, &events)
                .await
                .unwrap();
        }
        assert_eq!(state.view, app::View::Browse);
    }
    #[test]
    fn player_tabs_and_expansion_leave_library_navigation_independent() {
        use app::state::{DesktopFullscreenView as FullscreenView, VisualizerTab};
        let mut state = AppState::new();
        state.view = app::View::Browse;
        state.desktop_player_visible = true;
        select_player_mode(&mut state, 2);
        assert_eq!(state.visualizer_tab, VisualizerTab::Spectrum);
        assert_eq!(state.desktop_player_fullscreen, FullscreenView::Closed);
        state.desktop_player_fullscreen = FullscreenView::Artwork;
        state.set_view(app::View::Search);
        let model = super::super::player_model::project(&state, &mut Default::default());
        assert!(model.fullscreen);
        assert_eq!(model.mode, 0);
        select_player_mode(&mut state, 6);
        assert_eq!(state.view, app::View::Search);
        assert_eq!(state.desktop_player_fullscreen, FullscreenView::Visualizer);
        assert_eq!(state.fullscreen, app::state::FullscreenView::Closed);
        assert_eq!(state.visualizer_tab, VisualizerTab::Meters);
        select_player_mode(&mut state, 7);
        assert_eq!(
            super::super::player_model::project(&state, &mut Default::default()).mode,
            6
        );
        assert_eq!(
            VisualizerTab::ALL.len(),
            6,
            "terminal and desktop share the measured visualizers"
        );
        select_player_mode(&mut state, 1);
        assert_eq!(
            super::super::player_model::project(&state, &mut Default::default()).mode,
            1
        );
    }
    #[test]
    fn player_window_keyboard_and_palette_do_not_replace_library_selection() {
        use crate::app::state::{BrowseColumn, BrowseItem};
        let mut state = AppState::new();
        state.view = app::View::Browse;
        state.category_column_focused = true;
        state.focus = app::state::Focus::Left;
        let track = |key: &str| crate::library::models::Track {
            rating_key: key.into(),
            title: key.into(),
            ..Default::default()
        };
        let library = vec![track("library")];
        state.artist_nav.columns = vec![BrowseColumn::new_with_tracks(
            "Tracks",
            BrowseItem::from_tracks(&library),
            library,
        )];
        state.queue.tracks = vec![track("queue-1"), track("queue-2")];
        queue_key(
            &mut state,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Down,
                crossterm::event::KeyModifiers::NONE,
            ),
            &Default::default(),
        );
        assert_eq!(state.view, app::View::Browse);
        assert_eq!(state.artist_nav.columns[0].selected_index, 0);
        assert_eq!(state.list_state.queue_index, 1);
        assert!(state.category_column_focused);
        assert_eq!(state.focus, app::state::Focus::Left);
        state.category_column_focused = false;
        state.palette.open = true;
        state.palette.player_context = true;
        assert_eq!(state.palette_target_track().unwrap().rating_key, "queue-2");
        state.palette.close();
        assert_eq!(state.palette_target_track().unwrap().rating_key, "library");
    }
    #[test]
    fn clicked_playlist_tracks_keep_keyboard_selection_in_the_clicked_column() {
        use super::super::browser::{Intent, Selection};
        use app::state::{BrowseCategory, BrowseColumn, BrowseItem};
        let mut state = AppState::new();
        state.view = app::View::Browse;
        state.browse_category = BrowseCategory::Playlists;
        let tracks: Vec<_> = (0..4)
            .map(|index| crate::library::models::Track {
                rating_key: format!("track-{index}"),
                title: format!("Track {index}"),
                ..Default::default()
            })
            .collect();
        state.playlist_nav.columns = vec![
            BrowseColumn::new("Playlists", vec![]),
            BrowseColumn::new_with_tracks("Playlist", BrowseItem::from_tracks(&tracks), tracks),
        ];
        state.playlist_nav.columns[1].header_selection = app::state::ColumnHeaderSelection::Play;
        let target = super::super::browser::columns(&state)[1]
            .target(state.library_generation, 1, 1)
            .unwrap();
        select_target(&mut state, &target, Selection::Single, Intent::Focus);
        let inspection = select_target(&mut state, &target, Selection::Single, Intent::Inspect);
        assert!(inspection.iter().any(|action| matches!(
            action,
            Action::Browse(app::action::BrowseAction::OpenTrackDetails)
        )));
        assert_eq!(state.playlist_nav.focused_column, 1);
        assert!(navigate_browser_rows(&mut state, 1).is_empty());
        assert_eq!(state.playlist_nav.columns[1].selected_index, 2);
        assert!(state.queue.tracks.is_empty());
        navigate_browser_rows(&mut state, -1);
        assert_eq!(state.playlist_nav.columns[1].selected_index, 1);
        assert!(
            matches!(activate(&mut state,false).as_slice(),[Action::Queue(app::action::QueueAction::PlayTrack(track))] if track.rating_key=="track-1")
        );
        assert!(
            matches!(activate(&mut state,true).as_slice(),[Action::Queue(app::action::QueueAction::PlayTracksNow(tracks))] if tracks.len()==3)
        );
        let extended = super::super::browser::columns(&state)[1]
            .target(state.library_generation, 1, 3)
            .unwrap();
        select_target(&mut state, &extended, Selection::Extend, Intent::Focus);
        select_target(&mut state, &target, Selection::Context, Intent::Focus);
        let actions = select_target(
            &mut state,
            &target,
            Selection::Context,
            Intent::AddTracksToPlaylist,
        );
        assert!(
            matches!(&actions[..],[Action::Source(app::sources::SourceAction::Navidrome(command))] if matches!(command.as_ref(),app::sources::navidrome::NavAction::Command(app::sources::navidrome::commands::Command::ChoosePlaylist(app::sources::navidrome::playlist_edit::Addition::Tracks(ids))) if ids.as_ref()==["track-1","track-2","track-3"]))
        );
    }
    #[test]
    fn enter_and_command_enter_play_distinct_ranges_and_reject_stale_targets() {
        use super::super::browser::{Intent, Selection};
        use app::state::{BrowseColumn, BrowseItem};
        let mut state = AppState::new();
        state.view = app::View::Browse;
        let tracks: Vec<crate::library::models::Track> = (0..3)
            .map(|index| crate::library::models::Track {
                rating_key: format!("track-{index}"),
                title: format!("Track {index}"),
                ..Default::default()
            })
            .collect();
        state.artist_nav.columns = vec![BrowseColumn::new_with_tracks(
            "Tracks",
            BrowseItem::from_tracks(&tracks),
            tracks,
        )];
        let target = super::super::browser::columns(&state)[0]
            .target(state.library_generation, 0, 1)
            .unwrap();
        assert!(
            matches!(select_target(&mut state, &target, Selection::Single, Intent::Play).as_slice(),
            [Action::Queue(app::action::QueueAction::PlayTrack(track))] if track.rating_key == "track-1")
        );
        assert!(matches!(activate(&mut state, true).as_slice(),
            [Action::Queue(app::action::QueueAction::PlayTracksNow(tracks))] if tracks.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>() == ["track-1", "track-2"]));
        state.library_generation += 1;
        assert!(
            select_target(
                &mut state,
                &target,
                Selection::Single,
                Intent::PlayFollowing
            )
            .is_empty()
        );
    }

    #[test]
    fn download_context_removes_clicked_copy_and_rejects_stale_targets() {
        use super::super::browser::{Intent, Selection};
        use crate::services::downloads::{Entry, Store};
        use app::state::{BrowseColumn, BrowseItem, LibraryRoot};
        let mut state = AppState::new();
        state.view = app::View::Browse;
        state.desktop_library_root = LibraryRoot::Downloads;
        let tracks: Vec<_> = ["A.wav", "B.wav"]
            .into_iter()
            .map(|name| crate::library::models::Track::from_folder("fixture", name))
            .collect();
        for track in &tracks {
            state.downloads.store.entries.insert(
                Store::id("fixture", track),
                Entry {
                    library: "fixture".into(),
                    library_name: "Fixture".into(),
                    track: track.clone(),
                    bytes: 12,
                    artwork: false,
                },
            );
        }
        state.artist_nav.columns = vec![BrowseColumn::new_with_tracks(
            "Downloads",
            BrowseItem::from_tracks(&tracks),
            tracks.clone(),
        )];
        let column = super::super::browser::columns(&state).remove(0);
        assert_eq!(column.rows[1].download_action, Some("Remove Download"));
        let target = column.target(state.library_generation, 0, 1).unwrap();
        let second = Store::id("fixture", &tracks[1]);
        assert!(
            matches!(select_target(&mut state,&target,Selection::Context,Intent::ManageDownload).as_slice(),
            [Action::System(app::action::SystemAction::Downloads(app::downloads::Command::Remove(id)))] if id == &second)
        );
        state.downloads.store.entries.remove(&second);
        assert!(
            select_target(
                &mut state,
                &target,
                Selection::Context,
                Intent::ManageDownload
            )
            .is_empty()
        );
        assert!(
            state
                .downloads
                .store
                .entries
                .contains_key(&Store::id("fixture", &tracks[0]))
        );
    }

    #[test]
    fn pending_download_target_cancels_without_playing_and_rejects_stale_rows() {
        use super::super::browser::{Intent, Selection};
        use app::state::{BrowseColumn, BrowseItem};
        let mut state = AppState::new();
        state.view = app::View::Browse;
        let track = crate::library::models::Track::from_folder("fixture", "song.wav");
        let mut items = BrowseItem::from_tracks(std::slice::from_ref(&track));
        if let BrowseItem::Track { download_id, .. } = &mut items[0] {
            *download_id = Some("transfer".into());
        }
        state.artist_nav.columns = vec![BrowseColumn::new_with_tracks(
            "Downloads",
            items,
            vec![track],
        )];
        let target = super::super::browser::columns(&state)[0]
            .target(state.library_generation, 0, 0)
            .unwrap();
        assert!(select_target(&mut state, &target, Selection::Single, Intent::Play).is_empty());
        assert!(
            matches!(select_target(&mut state, &target, Selection::Single, Intent::ManageDownload).as_slice(),
            [Action::System(app::action::SystemAction::Downloads(app::downloads::Command::Cancel(id)))] if id == "transfer")
        );
        if let BrowseItem::Track { download_id, .. } = &mut state.artist_nav.columns[0].items[0] {
            *download_id = None;
        }
        assert!(
            select_target(
                &mut state,
                &target,
                Selection::Single,
                Intent::ManageDownload
            )
            .is_empty()
        );
        state.view = app::View::Queue;
        state.queue.tracks = vec![crate::library::models::Track::from_folder(
            "fixture",
            "queued.wav",
        )];
        if let BrowseItem::Track { download_id, .. } = &mut state.artist_nav.columns[0].items[0] {
            *download_id = Some("transfer".into());
        }
        let target = super::super::browser::columns(&state)[0]
            .target(state.library_generation, 0, 0)
            .unwrap();
        assert!(!select_target(&mut state, &target, Selection::Single, Intent::Play).is_empty());
    }

    #[test]
    fn album_context_play_targets_clicked_row_and_rejects_stale_menu() {
        use super::super::browser::{Intent, Selection};
        use app::state::{BrowseColumn, BrowseItem};
        let mut state = AppState::new();
        state.view = app::View::Browse;
        state.artist_nav.columns = vec![BrowseColumn::new(
            "Albums",
            (0..2)
                .map(|index| BrowseItem::Album {
                    key: format!("album-{index}"),
                    title: format!("Album {index}"),
                    artist: "Artist".into(),
                    year: None,
                    thumb: None,
                    is_placeholder: false,
                })
                .collect(),
        )];
        let target = super::super::browser::columns(&state)[0]
            .target(state.library_generation, 0, 1)
            .unwrap();
        assert!(matches!(
            select_target(&mut state, &target, Selection::Context, Intent::Download).as_slice(),
            [Action::System(app::action::SystemAction::Downloads(app::downloads::Command::Album(key)))]
                if key == "album-1"
        ));
        assert_eq!(state.artist_nav.columns[0].selected_index, 0);
        assert!(select_target(&mut state, &target, Selection::Context, Intent::Focus).is_empty());
        assert!(state.queue.tracks.is_empty());
        assert_eq!(state.artist_nav.columns[0].selected_index, 1);
        assert!(
            matches!(select_target(&mut state, &target, Selection::Context, Intent::Play).as_slice(),
            [Action::Queue(app::action::QueueAction::PlayAlbumNow { rating_key, .. })] if rating_key == "album-1")
        );
        state.library_generation += 1;
        assert!(select_target(&mut state, &target, Selection::Context, Intent::Play).is_empty());
        assert!(
            select_target(&mut state, &target, Selection::Context, Intent::Download).is_empty()
        );
    }

    #[test]
    fn album_track_column_exposes_play_and_append_with_next_as_a_modifier() {
        use app::state::{BrowseColumn, PlayAllRow};
        let mut state = AppState::new();
        let mut tracks = BrowseColumn::new("tracks — Album", vec![]);
        tracks.play_all_row = Some(PlayAllRow::Album {
            rating_key: "album".into(),
            title: "Album".into(),
        });
        state.artist_nav.columns = vec![tracks];
        let columns = super::super::browser::columns(&state);
        assert_eq!(
            columns[0]
                .header_controls
                .iter()
                .map(|control| control.label.as_str())
                .collect::<Vec<_>>(),
            ["Play Album", "Add Album to Queue"]
        );
        assert!(state.artist_nav.columns[0].play_all_row.is_some());
    }

    #[test]
    fn now_playing_queue_keys_reorder_and_navigate_without_changing_workspace() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut state = AppState::new();
        state.view = app::View::NowPlaying;
        state.queue.tracks = vec![Default::default(), Default::default(), Default::default()];
        state.list_state.queue_index = 1;
        let actions = queue_key(
            &mut state,
            KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT),
            &Default::default(),
        );
        assert!(matches!(
            actions.as_slice(),
            [Action::Queue(app::action::QueueAction::MoveQueueTrackDown)]
        ));
        assert_eq!(state.view, app::View::NowPlaying);
        queue_key(
            &mut state,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            &Default::default(),
        );
        assert_eq!(state.list_state.queue_index, 2);
        assert_eq!(state.view, app::View::NowPlaying);
    }

    #[test]
    fn seek_rejects_stale_playback_and_bounds_valid_positions() {
        let mut state = AppState::new();
        state.playback.request_id = 42;
        state.playback.duration_ms = 100_000;
        assert!(seek_action(&state, 41, 0.5).is_none());
        assert!(seek_action(&state, 42, f32::NAN).is_none());
        assert!(matches!(
            seek_action(&state, 42, 0.5),
            Some(Action::Playback(crate::app::action::PlaybackAction::Seek(
                50_000
            )))
        ));
        assert!(matches!(
            seek_action(&state, 42, 2.),
            Some(Action::Playback(crate::app::action::PlaybackAction::Seek(
                100_000
            )))
        ));
        assert!(matches!(
            seek_action(&state, 42, -1.),
            Some(Action::Playback(crate::app::action::PlaybackAction::Seek(
                0
            )))
        ));
        state.playback.duration_ms = 0;
        assert!(seek_action(&state, 42, 0.5).is_none());
    }

    #[test]
    fn selecting_folder_navigates_without_playing() {
        let mut state = AppState::new();
        state.browse_category = app::BrowseCategory::Folders;
        let mut nav = FolderNavigationState::for_library("fixture".into());
        nav.columns.push(FolderColumn::new(
            None,
            "Music".into(),
            vec![FolderItem::folder("album".into(), "Album".into())],
        ));
        state.folder_state = Some(nav);
        let actions = select(&mut state, 0, 0, false);
        assert!(
            matches!(actions.as_slice(), [Action::Folders(app::action::FolderAction::NavigateIntoFolder { folder_key, .. })] if folder_key == "album")
        );
        assert!(state.queue.tracks.is_empty());
        assert_eq!(state.playback.status, app::PlayStatus::Stopped);
    }

    #[test]
    fn stale_row_does_not_change_selection() {
        let mut state = AppState::new();
        state.browse_category = app::BrowseCategory::Folders;
        state.folder_state = Some(FolderNavigationState::for_library("fixture".into()));
        assert!(select(&mut state, 99, 99, true).is_empty());
        assert!(state.queue.tracks.is_empty());
    }
    #[test]
    fn filtered_parent_does_not_block_child_selection_and_drilling() {
        use app::state::{BrowseColumn, BrowseItem, ListFilterResults};
        let mut state = AppState::new();
        state.view = app::View::Browse;
        state.artist_nav.columns = vec![
            BrowseColumn::new(
                "artists",
                vec![BrowseItem::Artist {
                    key: "artist".into(),
                    title: "Artist".into(),
                    thumb: None,
                    is_placeholder: false,
                }],
            ),
            BrowseColumn::new(
                "albums",
                ["first", "second"]
                    .into_iter()
                    .map(|key| BrowseItem::Album {
                        key: key.into(),
                        title: key.into(),
                        artist: "Artist".into(),
                        year: None,
                        thumb: None,
                        is_placeholder: false,
                    })
                    .collect(),
            ),
        ];
        state.artist_nav.focused_column = 1;
        state.list_filter.active = true;
        state.list_filter.column = 0;
        state.list_filter.category = app::BrowseCategory::Library;
        state.list_filter.query = "Artist".into();
        state.list_filter.results = Some(ListFilterResults {
            matched_indices: vec![0],
            total_matches: 1,
            has_more: false,
        });
        let actions = navigate_browser_rows(&mut state, 1);
        assert!(
            matches!(actions.as_slice(),[Action::Miller(app::action::MillerAction::LoadAlbumTracksForMiller {album_key,..})] if album_key == "second")
        );
        assert_eq!(state.artist_nav.focused_column, 1);
        assert_eq!(state.list_filter.query, "Artist");
        assert!(state.list_filter.active);
    }
}

#[cfg(all(test, feature = "desktop-tests"))]
mod playback_projection_tests {
    use super::*;
    #[test]
    fn passive_frames_reuse_catalog_and_artwork_and_sleep_when_idle() {
        let mut state = AppState::new();
        state.artwork.current_data = Some(vec![1, 2, 3]);
        let mut projection = super::super::player_model::Projection::default();
        let mut frame = Snapshot::default();
        assert!(frame.refresh_playback(&state, &mut projection, None));
        let artwork = frame.player.artwork.clone().unwrap();
        assert!(!frame.refresh_playback(&state, &mut projection, None));
        assert!(std::sync::Arc::ptr_eq(
            &artwork,
            frame.player.artwork.as_ref().unwrap()
        ));
        state.playback.status = app::PlayStatus::Playing;
        state.playback.position_ms = 12_345;
        assert!(frame.refresh_playback(&state, &mut projection, None));
        assert_eq!(frame.position_ms, 12_345);
        state.playback.status = app::PlayStatus::Paused;
        assert!(frame.refresh_playback(&state, &mut projection, None));
        assert!(!frame.playing);
        state.set_status("Ready".into());
        assert!(frame.refresh_playback(&state, &mut projection, None));
        assert_eq!(frame.status, "Ready");
    }
}
