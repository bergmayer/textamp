use crate::app::{self, Action, AppState, action::*, library_details::Destination, state::*};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Deserialize)]
pub(super) struct PlaylistMark {
    index: usize,
    key: String,
    selected: bool,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Command {
    Shutdown,
    LyricsLanguage {
        index: usize,
    },
    LyricsFollow,
    LyricsSearch,
    LyricsRetry,
    HomeAction {
        action: HomeAction,
    },
    ReturnToPreviousLocation,
    Invoke {
        token: u64,
    },
    Select {
        owner: String,
        index: usize,
        key: String,
    },
    Pop {
        depth: usize,
    },
    Tab {
        tab: Tab,
    },
    Sheet {
        sheet: Option<Sheet>,
    },
    Dismiss {
        sheet: Sheet,
    },
    EditInput {
        text: String,
    },
    ArtistRadioQuery {
        query: String,
    },
    RefreshCacheSizes,
    DismissNotice {
        message: String,
    },
    MoreRows {
        owner: String,
    },
    ToggleTrackSelection {
        owner: String,
        index: usize,
        key: String,
    },
    ClearSelection {
        owner: String,
    },
    SelectAll {
        owner: String,
    },
    PlaylistMark {
        owner: String,
        index: usize,
        key: String,
        selected: bool,
    },
    PlaylistGesture {
        owner: String,
        marks: Vec<PlaylistMark>,
    },
    Viewport {
        owner: String,
        start: usize,
        end: usize,
    },
    Filter {
        owner: String,
        query: String,
    },
    Search {
        query: String,
        #[serde(default)]
        submit: bool,
    },
    Palette {
        query: String,
    },
    Transport {
        operation: Transport,
    },
    Seek {
        milliseconds: u64,
    },
    Volume {
        value: f32,
    },
    QueueMove {
        from: usize,
        to: usize,
        keys: Vec<String>,
    },
    QueueRemove {
        index: usize,
        key: String,
    },
    QueuePlay {
        index: usize,
        key: String,
    },
    Metadata {
        target: Target,
    },
    Artwork {
        owner: String,
        keys: Vec<String>,
    },
    Connect {
        provider: Provider,
        fields: Vec<String>,
        #[serde(default)]
        owner: Option<String>,
    },
    CancelConnection {
        #[serde(default)]
        owner: Option<String>,
    },
    Connectivity {
        available: bool,
    },
    Describe {
        query: String,
    },
    JourneyEndpoint {
        endpoint: Endpoint,
    },
    JourneyBack,
    JourneyQuery {
        endpoint: Endpoint,
        query: String,
    },
    JourneyCount {
        count: String,
    },
    SubmitInput {
        text: String,
    },
    Confirm {
        accept: bool,
    },
    SetSingles {
        count: u32,
    },
}
#[derive(Clone, Copy, Debug, Default, Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Tab {
    #[default]
    Library,
    NowPlaying,
    Queue,
}
#[derive(Clone, Copy, Debug, Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Sheet {
    Settings,
    Search,
    Palette,
    Lyrics,
    SavePlaylist,
    Downloads,
    Journey,
    Input,
    Confirm,
    ViewOptions,
    LibraryPicker,
    ArtistRadio,
    Biography,
    Text,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Endpoint {
    Start,
    End,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Transport {
    OutputInterrupted,
    PlayPause,
    Play,
    Pause,
    Previous,
    Next,
    Stop,
    Lyrics,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Target {
    Track,
    Album,
    Artist,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Provider {
    Navidrome,
    Webdav,
    Audiomuse,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum HomeAction {
    RandomAlbumRadio,
}

#[derive(Clone)]
pub(super) enum Binding {
    Actions(Vec<Action>),
    LibraryOption {
        choice: app::sources::LibraryChoice,
        action: app::sources::options::OptionAction,
    },
    Sidebar(app::sidebar::SidebarRow),
    SidebarGroup(app::sidebar::SidebarGroup),
    Palette(PaletteCommandKind),
    PaletteSelection(PaletteCommandKind),
    Tool(app::presentation::NpSidebarButton),
    Search(SearchTab, usize, String),
    ViewOption {
        column: usize,
        option: SortPopupOption,
    },
    Details(usize),
    Station {
        owner: String,
        key: String,
        open: bool,
    },
    ToggleTrackSelection {
        owner: String,
        index: usize,
        key: String,
    },
    PlaylistEntry {
        owner: String,
        index: usize,
        key: String,
        edit: app::sources::navidrome::playlist_edit::Edit,
    },
    TrackContext {
        owner: String,
        index: usize,
        key: String,
        kind: app::track_context::ContextKind,
    },
    JourneyChoice {
        index: usize,
        key: String,
    },
    ArtistRadio(String),
}

pub(super) use crate::app::navigation_history::LibraryNavigation;

#[derive(Default)]
pub(super) struct Adapter {
    pending_home_action: Option<(HomeAction, u64)>,
    pub tab: Tab,
    pub sheet: Option<Sheet>,
    next_token: u64,
    pub windows: HashMap<String, std::ops::Range<usize>>,
    pub group_filters: HashMap<String, String>,
    bindings: HashMap<u64, (u64, Binding)>,
    control_tokens: HashMap<(u64, String), u64>,
    visible_tokens: HashSet<u64>,
}
fn apply_marks(selected: &mut std::collections::BTreeSet<usize>, marks: &[PlaylistMark]) {
    for mark in marks {
        if mark.selected {
            selected.insert(mark.index);
        } else {
            selected.remove(&mark.index);
        }
    }
}

impl Adapter {
    pub fn window(&mut self, owner: &str, count: usize, selected: usize) -> std::ops::Range<usize> {
        let window = self.windows.entry(owner.into()).or_insert_with(|| {
            let start = selected.saturating_sub(100);
            start..(start + 300).min(count)
        });
        window.start = window.start.min(count.saturating_sub(1));
        window.end = window.end.max(window.start + 300).min(count);
        window.clone()
    }
    pub fn begin_projection(&mut self) {
        self.visible_tokens.clear();
    }
    pub fn bind(&mut self, state: &AppState, identity: &str, binding: Binding) -> u64 {
        let token = *self
            .control_tokens
            .entry((state.library_generation, identity.into()))
            .or_insert_with(|| {
                self.next_token += 1;
                self.next_token
            });
        self.bindings
            .insert(token, (state.library_generation, binding));
        self.visible_tokens.insert(token);
        token
    }
    fn mark_rows(&self, state: &mut AppState, owner: &str, marks: &[PlaylistMark], enter: bool) {
        use app::sources::navidrome::playlist_edit;
        let Some(ci) = column_index(state, owner) else {
            return;
        };
        if marks.is_empty() {
            return;
        }
        if state.browse_category == BrowseCategory::Folders {
            let Some(nav) = state.folder_state.as_mut() else {
                return;
            };
            let col = &mut nav.columns[ci];
            if !marks
                .iter()
                .all(|m| col.items.get(m.index).is_some_and(|item| item.key == m.key))
            {
                return;
            }
            apply_marks(&mut col.selected_set, marks);
            nav.focused_column = ci;
            state.category_column_focused = false;
            return;
        }
        let Some(col) = state.browse_nav().and_then(|nav| nav.columns.get(ci)) else {
            return;
        };
        if !marks.iter().all(|m| {
            col.items
                .get(m.index)
                .is_some_and(|item| item.key() == m.key)
        }) {
            return;
        }
        let Some(playlist) = playlist_edit::editable(state, col) else {
            if let Some(nav) = state.browse_nav_mut() {
                apply_marks(&mut nav.columns[ci].selected_set, marks);
                nav.focused_column = ci;
            }
            state.category_column_focused = false;
            state.track_pane_focused = false;
            return;
        };
        if !col.playlist_editing {
            if !enter {
                return;
            }
            let playlist = playlist.to_owned();
            playlist_edit::set_editing(state, &playlist, true);
        }
        let columns = &mut state.playlist_nav.columns;
        if !playlist_edit::mark_group_rows(
            columns,
            ci,
            marks.iter().map(|mark| (mark.index, Some(mark.selected))),
        ) {
            let col = &mut columns[ci];
            apply_marks(&mut col.selected_set, marks);
            if let Some(mark) = marks.last() {
                col.selected_index = mark.index;
            }
            col.header_selection = ColumnHeaderSelection::Items;
        }
        state.playlist_nav.focused_column = ci;
        state.category_column_focused = false;
        state.track_pane_focused = false;
    }
    pub fn finish_projection(&mut self) {
        self.bindings
            .retain(|token, _| self.visible_tokens.contains(token));
        self.control_tokens
            .retain(|_, token| self.visible_tokens.contains(token));
    }
    pub fn observe(&mut self, action: &Action, state: &mut AppState) {
        match action {
            Action::System(SystemAction::Downloads(
                app::downloads::Command::Open | app::downloads::Command::Manage,
            ))
            | Action::Search(SearchAction::ActivateListFilter)
            | Action::Browse(
                BrowseAction::OpenDetailsInLibrary { .. }
                | BrowseAction::OpenInLibrary { .. }
                | BrowseAction::OpenSingles,
            )
            | Action::Navigation(
                NavigationAction::SetCategory { .. } | NavigationAction::OpenLibraryRoot(_),
            ) => {
                if !matches!(
                    action,
                    Action::Browse(
                        BrowseAction::OpenDetailsInLibrary { .. }
                            | BrowseAction::OpenInLibrary { .. }
                    )
                ) {
                    state.library_navigation.open_columns();
                }
                self.tab = Tab::Library;
                state.popups.close_all();
                state.palette.close();
            }
            Action::Navigation(NavigationAction::SetView(view)) => {
                if matches!(view, View::Browse | View::Queue | View::NowPlaying) {
                    state.popups.close_all();
                    state.palette.close();
                }
                self.tab = match view {
                    View::Queue => Tab::Queue,
                    View::NowPlaying => Tab::NowPlaying,
                    View::Browse => Tab::Library,
                    _ => self.tab,
                }
            }
            Action::Data(
                DataAction::LoadSimilarTracks { .. }
                | DataAction::LoadSimilarAlbums { .. }
                | DataAction::LoadSimilarArtists { .. }
                | DataAction::LoadRelated { .. },
            ) => {
                state.popups.close_all();
                state.palette.close();
            }
            _ => {}
        }
    }
    pub fn take_home_action(&mut self, state: &mut AppState) -> Vec<Action> {
        let Some((action, generation)) = self.pending_home_action else {
            return vec![];
        };
        if generation != state.library_generation {
            self.pending_home_action = None;
            return vec![];
        }
        use app::sources::ActiveSource;
        let station = match &state.sources.active {
            ActiveSource::None if state.library_loading => return vec![],
            ActiveSource::None => {
                self.pending_home_action = None;
                state.set_status("Choose a library to start Play Random Albums".into());
                return vec![SettingsAction::OpenSettings.into()];
            }
            ActiveSource::Navidrome(_) if state.downloads.is_offline() => {
                self.pending_home_action = None;
                state.set_status("Connect to your library to start Play Random Albums".into());
                return vec![];
            }
            ActiveSource::Navidrome(_) => "nav-radio/randomAlbum",
            ActiveSource::Folder(_) if state.library_loading => return vec![],
            ActiveSource::Folder(_) => "folder-radio/randomAlbum",
        };
        self.pending_home_action = None;
        match action {
            HomeAction::RandomAlbumRadio => {
                self.tab = Tab::NowPlaying;
                state.popups.close_all();
                state.palette.close();
                vec![
                    NavigationAction::SetView(View::NowPlaying).into(),
                    RadioAction::PlayStation(station.into()).into(),
                ]
            }
        }
    }

    pub fn sync(&mut self, state: &AppState) {
        self.sheet = if state.popups.confirm_dialog().is_some() {
            Some(Sheet::Confirm)
        } else if state.palette.open {
            Some(Sheet::Palette)
        } else if app::radio_navigation::inline_input(state) {
            None
        } else if state.popups.adventure_launcher().is_some() {
            Some(Sheet::Journey)
        } else if let Some(input) = state.popups.input_dialog() {
            Some(
                if matches!(input.action_type, InputDialogAction::SavePlaylist) {
                    Sheet::SavePlaylist
                } else {
                    Sheet::Input
                },
            )
        } else if state.popups.search_active() {
            Some(Sheet::Search)
        } else if state.popups.sort().is_some() {
            Some(Sheet::ViewOptions)
        } else if state.popups.library_picker_active() {
            Some(Sheet::LibraryPicker)
        } else if state.popups.artist_radio_picker().is_some() {
            Some(Sheet::ArtistRadio)
        } else if state.popups.artist_bio().is_some() {
            Some(Sheet::Biography)
        } else if state.popups.lyrics().is_some() {
            Some(Sheet::Lyrics)
        } else if let Some(text) = state.popups.text() {
            Some(match text.title.as_str() {
                "Downloads" => Sheet::Downloads,
                title if title.starts_with("Lyrics") => Sheet::Lyrics,
                _ => Sheet::Text,
            })
        } else {
            match state.view {
                View::Settings => Some(Sheet::Settings),
                _ => None,
            }
        };
        self.tab = match state.view {
            View::NowPlaying => Tab::NowPlaying,
            View::Queue => Tab::Queue,
            View::Browse => Tab::Library,
            _ => self.tab,
        };
    }
    pub fn handle(
        &mut self,
        command: Command,
        state: &mut AppState,
        config: &crate::config::Config,
    ) -> Vec<Action> {
        match command {
            Command::Shutdown => vec![],
            Command::RefreshCacheSizes => vec![SettingsAction::RefreshCacheStats.into()],
            Command::DismissNotice { message } => {
                if state.notifications.last_error.as_deref() == Some(message.as_str()) {
                    state.clear_error();
                }
                if state.notifications.toast_message.as_deref() == Some(message.as_str()) {
                    state.notifications.toast_message = None;
                    state.notifications.toast_show_time = None;
                }
                if state.notifications.status_message.as_deref() == Some(message.as_str()) {
                    state.clear_status();
                }
                vec![]
            }
            Command::Dismiss { sheet } => {
                self.sync(state);
                if self.sheet != Some(sheet) {
                    return vec![];
                }
                let actions = if sheet == Sheet::Palette {
                    state.palette.close();
                    vec![]
                } else if sheet == Sheet::Confirm {
                    return self.handle(Command::Confirm { accept: false }, state, config);
                } else {
                    state.popups.close_all();
                    state.sources.nav_tasks.remove("lyrics");
                    let mut actions = vec![SearchAction::CloseSearchPopup.into()];
                    if sheet == Sheet::Settings {
                        let view = state.previous_view.take().unwrap_or(View::Browse);
                        actions.push(NavigationAction::SetView(view).into());
                    }
                    actions
                };
                self.sync(state);
                actions
            }
            Command::EditInput { text } => {
                if let Some(input) = state.popups.input_dialog_mut() {
                    input.input = text.into();
                }
                vec![]
            }
            Command::ArtistRadioQuery { query } => {
                if let Some(picker) = state.popups.artist_radio_picker_mut() {
                    picker.query = query;
                    vec![SearchAction::ArtistRadioPickerSearch.into()]
                } else {
                    vec![]
                }
            }
            Command::Connectivity { available } => {
                vec![app::downloads::Command::NetworkAvailable(available).into()]
            }
            Command::Invoke { token } => {
                let binding = self.bindings.get(&token).cloned();
                if let Some((generation, binding)) = binding
                    && generation == state.library_generation
                {
                    self.activate(binding, state)
                } else {
                    vec![]
                }
            }
            Command::ReturnToPreviousLocation => {
                let actions = app::navigation_history::back(state);
                self.tab = Tab::Library;
                actions
            }
            Command::Tab { tab } => {
                self.tab = tab;
                let mut actions = vec![];
                if tab == Tab::NowPlaying {
                    actions.push(SystemAction::LoadWaveform.into());
                }
                actions.push(
                    NavigationAction::SetView(match tab {
                        Tab::Library => View::Browse,
                        Tab::NowPlaying => View::NowPlaying,
                        Tab::Queue => View::Queue,
                    })
                    .into(),
                );
                actions
            }
            Command::LyricsLanguage { index } => {
                vec![PlaybackAction::SelectLyricsLanguage(index).into()]
            }
            Command::LyricsFollow => vec![PlaybackAction::ToggleLyricsSync.into()],
            Command::LyricsSearch => app::lyrics::Control::Search.actions(state),
            Command::LyricsRetry => vec![PlaybackAction::RetryLyrics.into()],
            Command::HomeAction { action } => {
                self.pending_home_action = Some((action, state.library_generation));
                self.take_home_action(state)
            }
            Command::Sheet { sheet } => match sheet {
                Some(Sheet::Downloads) => vec![app::downloads::Command::Manage.into()],
                Some(Sheet::Search) => {
                    state.palette.open = false;
                    state.search.tab = SearchTab::Global;
                    vec![SearchAction::OpenSearchPopup.into()]
                }
                Some(Sheet::Palette) => app::command_palette::open(state),
                Some(Sheet::Settings) => {
                    state.palette.open = false;
                    vec![SettingsAction::OpenSettings.into()]
                }
                Some(Sheet::Lyrics) => vec![PlaybackAction::ShowLyrics.into()],
                _ => {
                    self.sync(state);
                    self.sheet
                        .map(|sheet| self.handle(Command::Dismiss { sheet }, state, config))
                        .unwrap_or_default()
                }
            },
            Command::Describe { query } => vec![
                app::sources::audiomuse::Command::Search {
                    feature: crate::audiomuse::Feature::Describe,
                    query,
                }
                .into(),
            ],
            Command::JourneyEndpoint { endpoint } => vec![
                SearchAction::AdventureLauncherSetStep(match endpoint {
                    Endpoint::Start => AdventureStep::FindStartTrack,
                    Endpoint::End => AdventureStep::FindEndTrack,
                })
                .into(),
            ],
            Command::JourneyBack => vec![SearchAction::AdventureLauncherBack.into()],
            Command::JourneyQuery { endpoint, query } => vec![
                SearchAction::AdventureLauncherSetEndpointQuery {
                    step: match endpoint {
                        Endpoint::Start => AdventureStep::FindStartTrack,
                        Endpoint::End => AdventureStep::FindEndTrack,
                    },
                    query,
                }
                .into(),
            ],
            Command::JourneyCount { count } => {
                vec![SearchAction::AdventureLauncherSetTrackCount(count).into()]
            }
            Command::SubmitInput { text } => {
                if let Some(dialog) = state.popups.input_dialog_mut() {
                    dialog.input = text.into();
                } else {
                    return vec![];
                }
                app::handlers::key_input::handle_key(
                    crossterm::event::KeyEvent::new(
                        crossterm::event::KeyCode::Enter,
                        crossterm::event::KeyModifiers::NONE,
                    ),
                    state,
                    config,
                )
            }
            Command::Confirm { accept } => {
                if let Some(dialog) = state.popups.confirm_dialog_mut() {
                    dialog.selected_yes = accept;
                } else {
                    return vec![];
                }
                app::handlers::key_input::handle_key(
                    crossterm::event::KeyEvent::new(
                        crossterm::event::KeyCode::Enter,
                        crossterm::event::KeyModifiers::NONE,
                    ),
                    state,
                    config,
                )
            }
            Command::Select { owner, index, key } => {
                let Some(column) = column_index(state, &owner) else {
                    return vec![];
                };
                if state.browse_category == BrowseCategory::Folders {
                    let Some(nav) = state.folder_state.as_mut() else {
                        return vec![];
                    };
                    let Some(item) = nav
                        .columns
                        .get(column)
                        .and_then(|column| column.items.get(index))
                        .filter(|item| item.key == key)
                        .cloned()
                    else {
                        return vec![];
                    };
                    nav.focused_column = column;
                    nav.columns[column].selected_index = index;
                    nav.columns.truncate(column + 1);
                    state.category_column_focused = false;
                    state.track_pane_open = false;
                    return vec![if item.is_track() {
                        BrowseAction::OpenTrackDetails.into()
                    } else {
                        FolderAction::NavigateIntoFolder {
                            folder_key: key,
                            replace_child: true,
                        }
                        .into()
                    }];
                }
                if state
                    .browse_nav()
                    .and_then(|nav| nav.columns.get(column))
                    .and_then(|col| col.items.get(index))
                    .is_none_or(|item| item.key() != key)
                {
                    return vec![];
                }
                if state
                    .browse_nav()
                    .and_then(|nav| nav.columns.get(column))
                    .is_some_and(|c| {
                        !c.selected_set.is_empty()
                            && matches!(c.items.get(index), Some(BrowseItem::Track { .. }))
                    })
                {
                    return self
                        .activate(Binding::ToggleTrackSelection { owner, index, key }, state);
                }
                // A visible parent row replaces its existing child on a single tap.
                let has_child = state.track_pane_open
                    || state
                        .browse_nav()
                        .is_some_and(|nav| column + 1 < nav.columns.len());
                app::browse_drill::plan_drill(
                    state,
                    app::browse_drill::ClickContext {
                        column_index: column,
                        item_index: index,
                        activate: !has_child,
                    },
                )
                .actions
            }
            Command::Pop { depth } => {
                let group = usize::from(state.library_navigation.group().is_some());
                let hidden = hidden_root_columns(state);
                let count = state
                    .browse_nav()
                    .map_or(0, |nav| nav.columns.len())
                    .saturating_sub(hidden)
                    + group;
                if state.track_pane_open && depth >= count {
                    state.track_pane_open = false;
                    state.track_pane_focused = false;
                    return vec![];
                }
                let depth = state.library_navigation.pop(depth) + hidden;
                state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
                state.playlist_nav_request_id = state.playlist_nav_request_id.wrapping_add(1);
                state.playlist_nav.loading = false;
                for slot in [
                    "artist-browse",
                    "album-browse",
                    "collection",
                    "playlist-browse",
                ] {
                    state.sources.nav_tasks.remove(slot);
                }
                state.artist_nav.loading = false;
                if state.list_filter.active && state.list_filter.column >= depth {
                    state.list_filter.deactivate();
                }
                if state.desktop_library_root == LibraryRoot::Radio {
                    state.station_detail = None;
                    state.station_navigation_generation =
                        state.station_navigation_generation.wrapping_add(1);
                    state.sources.nav_tasks.remove("station-children");
                    state.station_nav.loading = false;
                    state.station_nav.columns.truncate(depth.max(1));
                    state.station_nav.focused_column = depth.saturating_sub(1);
                    return vec![];
                }
                state.track_pane_open = false;
                if state.browse_category == BrowseCategory::Folders
                    && let Some(nav) = state.folder_state.as_mut()
                {
                    nav.columns.truncate(depth);
                    nav.focused_column = depth.saturating_sub(1);
                }
                if let Some(nav) = state.browse_nav_mut() {
                    nav.columns.truncate(depth);
                    nav.focused_column = depth.saturating_sub(1);
                }
                vec![]
            }
            Command::ToggleTrackSelection { owner, index, key } => {
                self.activate(Binding::ToggleTrackSelection { owner, index, key }, state)
            }
            Command::ClearSelection { owner } => {
                if let Some(ci) = column_index(state, &owner) {
                    if state.browse_category == BrowseCategory::Folders {
                        if let Some(nav) = state.folder_state.as_mut() {
                            nav.columns[ci].selected_set.clear();
                        }
                    } else if let Some(nav) = state.browse_nav_mut() {
                        nav.columns[ci].selected_set.clear();
                    }
                }
                vec![]
            }
            Command::SelectAll { owner } => {
                if let Some(ci) = column_index(state, &owner)
                    && let Some(col) = state.browse_nav().and_then(|nav| nav.columns.get(ci))
                {
                    if let Some(playlist) =
                        app::sources::navidrome::playlist_edit::editable(state, col)
                            .map(str::to_owned)
                    {
                        app::sources::navidrome::playlist_edit::set_editing(state, &playlist, true);
                    }
                    if let Some(nav) = state.browse_nav_mut() {
                        nav.focused_column = ci;
                    }
                    app::column_controls::select_all(state, ci);
                }
                vec![]
            }
            Command::PlaylistMark {
                owner,
                index,
                key,
                selected,
            } => {
                self.mark_rows(
                    state,
                    &owner,
                    &[PlaylistMark {
                        index,
                        key,
                        selected,
                    }],
                    false,
                );
                vec![]
            }
            Command::PlaylistGesture { owner, marks } => {
                self.mark_rows(state, &owner, &marks, true);
                vec![]
            }
            Command::Viewport { owner, start, end } => {
                if column_index(state, &owner).is_some() && start < end {
                    let start = start.saturating_sub(75);
                    self.windows.insert(owner, start..start.saturating_add(300));
                }
                vec![]
            }
            Command::MoreRows { owner } => {
                let Some(index) = column_index(state, &owner) else {
                    return vec![];
                };
                if let Some(column) = state.browse_nav().and_then(|nav| nav.columns.get(index))
                    && let Some(lazy) = &column.lazy
                    && !lazy.loading
                    && lazy
                        .total
                        .is_none_or(|total| column.tracks.len() < total as usize)
                {
                    return vec![
                        MillerAction::LoadMorePlaylistTracks {
                            playlist_key: lazy.key.clone(),
                            offset: column.tracks.len() as u32,
                        }
                        .into(),
                    ];
                }
                vec![]
            }
            Command::Filter { owner, query } => {
                self.windows.remove(&owner);
                if state
                    .library_navigation
                    .group()
                    .is_some_and(|group| group_owner(state, group) == owner)
                {
                    self.group_filters.insert(owner, query);
                    return vec![];
                }
                let Some(column) = column_index(state, &owner) else {
                    return vec![];
                };
                if let Some(nav) = state.browse_nav_mut() {
                    nav.focused_column = column;
                }
                if state.browse_category == BrowseCategory::Folders
                    && let Some(nav) = state.folder_state.as_mut()
                {
                    nav.focused_column = column;
                }
                if state.list_filter.column != column
                    || state.list_filter.category != state.browse_category
                {
                    state.list_filter.deactivate();
                }
                vec![SearchAction::SetListFilterQuery(query).into()]
            }
            Command::Search { query, submit } => {
                let mut actions = vec![SearchAction::SetSearchQuery(query).into()];
                if submit {
                    actions.push(SearchAction::SubmitSearch.into());
                }
                actions
            }
            Command::Palette { query } if state.palette.open && !state.palette.path.is_empty() => {
                state.palette.query = query;
                state.palette.selected = 0;
                app::command_palette::refresh_matches(state);
                vec![]
            }
            Command::Palette { query } => app::command_palette::open_with_query(state, &query),
            Command::Transport { operation } => vec![
                match operation {
                    Transport::Play if state.playback.status == PlayStatus::Playing => {
                        return vec![];
                    }
                    Transport::Pause if state.playback.status != PlayStatus::Playing => {
                        return vec![];
                    }
                    Transport::PlayPause | Transport::Play | Transport::Pause => {
                        PlaybackAction::TogglePlayPause
                    }
                    Transport::OutputInterrupted => PlaybackAction::OutputInterrupted,
                    Transport::Previous => PlaybackAction::Previous,
                    Transport::Next => PlaybackAction::Next,
                    Transport::Stop => PlaybackAction::Stop,
                    Transport::Lyrics => PlaybackAction::ShowLyrics,
                }
                .into(),
            ],
            Command::Seek { milliseconds } => vec![PlaybackAction::Seek(milliseconds).into()],
            Command::Volume { value } => {
                vec![PlaybackAction::SetVolume(value.clamp(0., 1.)).into()]
            }
            Command::QueueMove { from, to, keys } => {
                if !state
                    .playback_tracks()
                    .iter()
                    .map(|track| &track.rating_key)
                    .eq(keys.iter())
                    || from >= keys.len()
                    || to > keys.len()
                {
                    return vec![];
                }
                state.queue.selected.clear();
                vec![QueueAction::ReorderSelection { from, to }.into()]
            }
            Command::QueueRemove { index, key } => {
                if queue_matches(state, index, &key) {
                    vec![QueueAction::RemoveFromQueue(index).into()]
                } else {
                    vec![]
                }
            }
            Command::QueuePlay { index, key } => {
                if queue_matches(state, index, &key) {
                    app::queue_edit::play_at(state, index)
                } else {
                    vec![]
                }
            }
            Command::Metadata { target } => state
                .current_track()
                .cloned()
                .map(|track| {
                    self.tab = Tab::Library;
                    vec![
                        BrowseAction::OpenDetailsInLibrary {
                            track: Box::new(track),
                            destination: match target {
                                Target::Track => Destination::Track,
                                Target::Album => Destination::Album,
                                Target::Artist => Destination::Artist,
                            },
                        }
                        .into(),
                    ]
                })
                .unwrap_or_default(),
            Command::Artwork { owner, keys } => {
                let Some(column) = column_index(state, &owner) else {
                    return vec![];
                };
                let Some(column) = state.browse_nav().and_then(|nav| nav.columns.get(column))
                else {
                    return vec![];
                };
                let mut images = column
                    .items
                    .iter()
                    .filter(|item| keys.iter().any(|key| key == item.key()))
                    .filter_map(|item| match item {
                        BrowseItem::Album {
                            key,
                            thumb: Some(thumb),
                            ..
                        }
                        | BrowseItem::Artist {
                            key,
                            thumb: Some(thumb),
                            ..
                        } => Some((key.clone(), thumb.clone())),
                        BrowseItem::Track { .. } => column
                            .tracks
                            .iter()
                            .find(|track| track.rating_key == item.key())
                            .and_then(|track| {
                                track
                                    .parent_rating_key
                                    .clone()
                                    .zip(track.parent_thumb.clone().or(track.thumb.clone()))
                            }),
                        _ => None,
                    })
                    .take(30)
                    .collect::<Vec<_>>();
                if let Some(PlayAllRow::Album { rating_key, .. }) = &column.play_all_row
                    && keys.contains(rating_key)
                    && let Some(thumb) = state
                        .library
                        .albums
                        .iter()
                        .find(|album| &album.rating_key == rating_key)
                        .and_then(|album| album.thumb.clone())
                        .or_else(|| {
                            column.tracks.first().and_then(|track| {
                                track.parent_thumb.clone().or(track.thumb.clone())
                            })
                        })
                {
                    images.push((rating_key.clone(), thumb));
                }
                images.sort();
                images.dedup();
                if images.is_empty() {
                    vec![]
                } else {
                    vec![SystemAction::LoadAlbumArt(images).into()]
                }
            }
            Command::Connect {
                provider,
                fields,
                owner,
            } => {
                if owner.as_deref().is_some_and(|owner| {
                    state.popups.library_dialog().and_then(connection_owner) != Some(owner)
                }) {
                    return vec![];
                }
                use app::sources::dialogs::{Dialog, Field, ServerForm, WebdavForm};
                let dialog = match provider {
                    Provider::Navidrome => {
                        let mut form = match (owner.as_ref(), state.popups.library_dialog()) {
                            (Some(_), Some(Dialog::Navidrome(form))) => form.clone(),
                            _ => ServerForm::new(None),
                        };
                        for (field, value) in form.fields.iter_mut().zip(fields) {
                            *field = Field {
                                cursor: value.len(),
                                value: value.into(),
                                selected: false,
                            };
                        }
                        form.focus = 8;
                        Dialog::Navidrome(form)
                    }
                    Provider::Webdav => {
                        let mut form = match (owner.as_ref(), state.popups.library_dialog()) {
                            (Some(_), Some(Dialog::Webdav(form))) => form.clone(),
                            _ => WebdavForm::new(None),
                        };
                        for (field, value) in form.fields.iter_mut().zip(fields) {
                            *field = Field {
                                cursor: value.len(),
                                value: value.into(),
                                selected: false,
                            };
                        }
                        form.focus = 4;
                        Dialog::Webdav(form)
                    }
                    Provider::Audiomuse => {
                        let Some(Dialog::AudioMuse(mut form)) =
                            state.popups.library_dialog().cloned()
                        else {
                            return vec![];
                        };
                        for (field, value) in form.fields.iter_mut().zip(fields) {
                            *field = Field {
                                cursor: value.len(),
                                value: value.into(),
                                selected: false,
                            };
                        }
                        form.focus = 4;
                        Dialog::AudioMuse(form)
                    }
                };
                state.popups.set_library_dialog(Some(dialog));
                app::sources::dialogs::key(
                    crossterm::event::KeyEvent::new(
                        crossterm::event::KeyCode::Enter,
                        crossterm::event::KeyModifiers::NONE,
                    ),
                    state,
                )
            }
            Command::CancelConnection { owner } => {
                if owner.as_deref().is_some_and(|owner| {
                    state.popups.library_dialog().and_then(connection_owner) != Some(owner)
                }) {
                    return vec![];
                }
                app::sources::dialogs::key(
                    crossterm::event::KeyEvent::new(
                        crossterm::event::KeyCode::Esc,
                        crossterm::event::KeyModifiers::NONE,
                    ),
                    state,
                )
            }
            Command::SetSingles { count } => {
                vec![SettingsAction::SetSinglesTrackLimit(count).into()]
            }
        }
    }
    fn activate(&mut self, binding: Binding, state: &mut AppState) -> Vec<Action> {
        match binding {
            Binding::PaletteSelection(command) => {
                let mut actions = app::command_palette::remember(&command, state);
                actions.extend(self.activate(Binding::Palette(command), state));
                actions
            }
            Binding::ArtistRadio(key) => {
                let Some(picker) = state.popups.artist_radio_picker_mut() else {
                    return vec![];
                };
                let Some(index) = picker
                    .filtered_artists
                    .iter()
                    .position(|artist| artist.rating_key == key)
                else {
                    return vec![];
                };
                picker.item_index = index;
                vec![SearchAction::ArtistRadioPickerToggleArtist.into()]
            }
            Binding::JourneyChoice { index, key } => {
                if let Some(launcher) = state.popups.adventure_launcher_mut()
                    && app::journey::choices(launcher)
                        .get(index)
                        .is_some_and(|choice| choice.key() == key)
                {
                    launcher.item_index = index;
                    return app::journey::select(launcher);
                }
                vec![]
            }
            Binding::Actions(actions) => actions,
            Binding::LibraryOption { choice, action } => {
                app::sources::options::apply(choice, action, state)
            }
            Binding::Station { owner, key, open } => {
                let Some(column) =
                    state
                        .station_nav
                        .columns
                        .iter()
                        .enumerate()
                        .find_map(|(index, column)| {
                            (station_owner(state.library_generation, column) == owner)
                                .then_some(index)
                        })
                else {
                    return vec![];
                };
                let Some((index, station)) = state.station_nav.columns[column]
                    .stations
                    .iter()
                    .enumerate()
                    .find(|(_, station)| station.key == key)
                    .map(|(index, station)| (index, station.clone()))
                else {
                    return vec![];
                };
                if !open && !station.requires_input() {
                    state.station_nav.focused_column = column;
                    state.station_nav.columns[column].selected_index = index;
                    state.station_detail = None;
                    return vec![RadioAction::PlayStation(station.key).into()];
                }
                app::radio_navigation::select(state, column, &station.key)
            }

            Binding::ToggleTrackSelection { owner, index, key } => {
                let Some(ci) = column_index(state, &owner) else {
                    return vec![];
                };
                let Some(nav) = state.browse_nav_mut() else {
                    return vec![];
                };
                let col = &mut nav.columns[ci];
                if !matches!(col.items.get(index),Some(BrowseItem::Track{key:k,..}) if *k==key) {
                    return vec![];
                }
                if !col.selected_set.remove(&index) {
                    col.selected_set.insert(index);
                }
                col.selected_index = if col.selected_set.contains(&index) {
                    index
                } else {
                    col.selected_set.last().copied().unwrap_or(index)
                };
                col.header_selection = ColumnHeaderSelection::Items;
                nav.focused_column = ci;
                state.category_column_focused = false;
                state.track_pane_focused = false;
                vec![]
            }
            Binding::PlaylistEntry {
                owner,
                index,
                key,
                edit,
            } => {
                let command = column_index(state, &owner)
                    .and_then(|ci| state.browse_nav()?.columns.get(ci))
                    .filter(|c| c.items.get(index).is_some_and(|i| i.key() == key))
                    .and_then(|c| {
                        app::sources::navidrome::playlist_edit::row_command(state, c, index, edit)
                    });
                command
                    .map(|c| vec![app::sources::navidrome::NavAction::Command(c).into()])
                    .unwrap_or_default()
            }
            Binding::TrackContext {
                owner,
                index,
                key,
                kind,
            } => {
                let Some(column_index) = column_index(state, &owner) else {
                    return vec![];
                };
                let Some(nav) = state.browse_nav_mut() else {
                    return vec![];
                };
                let column = &mut nav.columns[column_index];
                if column.items.get(index).is_none_or(|item| item.key() != key) {
                    return vec![];
                }
                let Some(track) = column
                    .tracks
                    .iter()
                    .find(|track| track.rating_key == key)
                    .cloned()
                else {
                    return vec![];
                };
                if matches!(kind, app::track_context::ContextKind::AddToPlaylist)
                    && column.selected_set.contains(&index)
                    && column.selected_set.len() > 1
                {
                    return app::sources::navidrome::playlist_edit::addition(
                        &column.tracks,
                        &column.selected_set,
                        index,
                    )
                    .map(|a| {
                        vec![
                            app::sources::navidrome::NavAction::Command(
                                app::sources::navidrome::commands::Command::ChoosePlaylist(a),
                            )
                            .into(),
                        ]
                    })
                    .unwrap_or_default();
                }
                column.selected_index = index;
                nav.focused_column = column_index;
                state.category_column_focused = false;
                self.activate(
                    Binding::Palette(PaletteCommandKind::FromTrackContext {
                        kind,
                        track: Box::new(track),
                    }),
                    state,
                )
            }
            Binding::SidebarGroup(app::sidebar::SidebarGroup::Radio) => self.open_radio(state),
            Binding::SidebarGroup(group) => {
                state.library_navigation = LibraryNavigation::Group(group);
                self.tab = Tab::Library;
                state.view = View::Browse;
                state.track_pane_open = false;
                state.popups.close_all();
                state.palette.close();
                vec![]
            }
            Binding::Sidebar(row) => {
                state.library_navigation = LibraryNavigation::Columns(row.parent);
                state.track_pane_open = false;
                state.track_pane_focused = false;
                self.tab = Tab::Library;
                state.view = View::Browse;
                if let [Action::Navigation(NavigationAction::OpenRadioStation(key))] =
                    row.actions.as_slice()
                {
                    self.open_radio(state);
                    if let Some(root) = state.station_nav.columns.first()
                        && let Some(station) =
                            root.stations.iter().find(|station| station.key == *key)
                    {
                        return self.activate(
                            Binding::Station {
                                owner: station_owner(state.library_generation, root),
                                key: key.clone(),
                                open: station.requires_input(),
                            },
                            state,
                        );
                    }
                    return vec![];
                }
                if let Some(root) = row.library_root {
                    if root == LibraryRoot::Radio {
                        return self.open_radio(state);
                    }
                    app::sidebar::open_root(root, state)
                } else if let Some(key) = row.playlist_key {
                    state.set_browse_category(BrowseCategory::Playlists, false);
                    state.category_column_focused = false;
                    if !state.select_playlist(&key) {
                        return vec![];
                    }
                    state.playlist_nav.focused_column = 0;
                    state.playlist_nav.truncate_right();
                    vec![
                        MillerAction::LoadPlaylistTracksForMiller {
                            playlist_key: key,
                            replace_child: true,
                        }
                        .into(),
                    ]
                } else {
                    row.actions
                }
            }
            Binding::Tool(tool) => app::queue_controls::activate(state, tool),
            Binding::Palette(command) => {
                if app::command_catalog::disabled_reason(state, &command).is_some() {
                    return vec![];
                }
                let group = matches!(command, PaletteCommandKind::OpenCommandGroup(_));
                if !group {
                    state.palette.open = false;
                }
                match command {
                    PaletteCommandKind::GotoQueue => {
                        state.popups.close_all();
                        vec![NavigationAction::SetView(View::Queue).into()]
                    }
                    PaletteCommandKind::OpenAiBrowser => self.activate(Binding::SidebarGroup(app::sidebar::SidebarGroup::Ai), state),
                    PaletteCommandKind::OpenRadioBrowser => self.open_radio(state),
                    PaletteCommandKind::ConfigureStation(key) => {
                        self.tab = Tab::Library;
                        app::radio_navigation::configure(state, &key)
                    }
                    PaletteCommandKind::BrowseStations { key, title } => {
                        self.open_radio(state);
                        vec![RadioAction::DrillIntoStation(key, title).into()]
                    }
                    PaletteCommandKind::StationsBack => {
                        state.library_navigation.open_columns();
                        self.tab = Tab::Library;
                        state.set_view(View::Browse);
                        vec![RadioAction::NavigateStationsBack.into()]
                    }
                    PaletteCommandKind::Navidrome(
                        app::sources::navidrome::commands::Command::Collection(_),
                    ) => {
                        state.library_navigation.open_columns();
                        app::command_palette::run(command, state)
                    }
                    _ => app::command_palette::run(command, state),
                }
            }
            Binding::Search(tab, index, query) => {
                if state.search.query != query {
                    return vec![];
                }
                state.search.tab = tab;
                state.list_state.search_item_index = index;
                self.tab = Tab::Library;
                if matches!(tab, SearchTab::Playlists | SearchTab::Genres) {
                    return vec![SearchAction::SelectSearchResult.into()];
                }
                let mut actions = vec![SearchAction::CloseSearchPopup.into()];
                actions.extend(app::library_details::search_result_actions(state));
                actions
            }
            Binding::Details(column) => {
                state.track_pane_open = false;
                app::column_controls::activate(
                    state,
                    column,
                    app::column_controls::ColumnControl::Details,
                    app::playback_controls::QueuePlacement::Now,
                )
            }
            Binding::ViewOption { column, option } => {
                if let Some(nav) = state.browse_nav_mut() {
                    nav.focused_column = column;
                }
                let mut actions = Vec::new();
                match option {
                    SortPopupOption::SortMode(mode) => actions.extend(
                        app::handlers::key_input::sort_popup::apply_sort_mode(state, column, mode),
                    ),
                    SortPopupOption::Direction => actions.extend(
                        app::handlers::key_input::sort_popup::toggle_sort_direction(state, column),
                    ),
                    SortPopupOption::Artwork => actions.extend(
                        app::handlers::key_input::sort_popup::toggle_artwork(state, column),
                    ),
                    SortPopupOption::LargeArtwork => {
                        actions.push(SettingsAction::SetLargeArtwork(!state.large_artwork).into())
                    }
                    SortPopupOption::CompilationOnlyArtists => actions.push(
                        SettingsAction::SetShowCompilationOnlyArtists(
                            !state.show_compilation_only_artists,
                        )
                        .into(),
                    ),
                    SortPopupOption::GroupByAlbum => actions.extend(
                        app::handlers::key_input::sort_popup::toggle_group_by_album(state, column),
                    ),
                }
                actions
            }
        }
    }
    fn open_radio(&mut self, state: &mut AppState) -> Vec<Action> {
        state.library_navigation = LibraryNavigation::Columns(None);
        state.station_detail = None;
        self.tab = Tab::Library;
        state.palette.close();
        state.popups.close_all();
        state.view = View::Browse;
        state.browse_category = BrowseCategory::Library;
        state.desktop_library_root = LibraryRoot::Radio;
        state.sources.nav_collection = None;
        state.station_nav.columns.truncate(1);
        state.station_nav.focused_column = 0;
        if state.station_nav.columns.is_empty() && !state.station_nav.loading {
            vec![BrowseAction::LoadStations.into()]
        } else {
            vec![]
        }
    }
}
pub(super) fn station_owner(generation: u64, column: &StationColumn) -> String {
    format!(
        "radio:{generation}:{}",
        column.key.as_deref().unwrap_or("root")
    )
}
fn queue_matches(state: &AppState, index: usize, key: &str) -> bool {
    state
        .playback_tracks()
        .get(index)
        .is_some_and(|track| track.rating_key == key)
}

pub(super) fn group_owner(state: &AppState, group: app::sidebar::SidebarGroup) -> String {
    format!(
        "sidebar-group:{}:{}",
        state.library_generation,
        group.label()
    )
}

/// The shared Miller chain retains a catalog root that is not a phone hierarchy level.
fn connection_owner(dialog: &app::sources::dialogs::Dialog) -> Option<&str> {
    use app::sources::dialogs::Dialog;
    match dialog {
        Dialog::Navidrome(form) => Some(&form.instance),
        Dialog::Webdav(form) => Some(&form.instance),
        Dialog::AudioMuse(form) => Some(&form.instance),
        _ => None,
    }
}

pub(super) fn hidden_root_columns(state: &AppState) -> usize {
    if state.browse_category == BrowseCategory::Playlists
        && state.library_navigation.group() == Some(app::sidebar::SidebarGroup::Playlists)
        && !state.playlist_nav.columns.is_empty()
    {
        return 1;
    }
    usize::from(
        state.browse_category == BrowseCategory::Library
            && state.sources.nav_collection.is_none()
            && matches!(
                state.desktop_library_root,
                LibraryRoot::Albums | LibraryRoot::Compilations | LibraryRoot::Tracks
            )
            && state
                .artist_nav
                .columns
                .first()
                .is_some_and(|column| column.title.eq_ignore_ascii_case("artists")),
    )
}

pub(super) fn owner(state: &AppState, index: usize) -> String {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    state.library_generation.hash(&mut hash);
    format!("{:?}", state.browse_category).hash(&mut hash);
    format!("{:?}", state.desktop_library_root).hash(&mut hash);
    index.hash(&mut hash);
    if let Some(nav) = state.browse_nav() {
        for column in nav.columns.iter().take(index) {
            column.selected_item().map(BrowseItem::key).hash(&mut hash);
        }
    }
    if let Some(column) = state.browse_nav().and_then(|nav| nav.columns.get(index)) {
        column.title.hash(&mut hash);
    }
    if state.browse_category == BrowseCategory::Folders
        && let Some(column) = state
            .folder_state
            .as_ref()
            .and_then(|nav| nav.columns.get(index))
    {
        column.key.hash(&mut hash);
    }
    format!("{:x}", hash.finish())
}
fn column_index(state: &AppState, id: &str) -> Option<usize> {
    let count = if state.browse_category == BrowseCategory::Folders {
        state.folder_state.as_ref()?.columns.len()
    } else {
        state.browse_nav()?.columns.len()
    };
    (0..count).find(|index| owner(state, *index) == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn with_catalog_source(state: &mut AppState) {
        let source = crate::navidrome::Source {
            id: "test".into(),
            name: "Fixture".into(),
            url: "http://127.0.0.1:1".into(),
            username: "fixture".into(),
            libraries: vec![],
        };
        state.sources.active =
            app::sources::ActiveSource::Navidrome(Box::new(app::sources::navidrome::Session {
                client: crate::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
                source,
                extensions: Default::default(),
            }));
        state.active_library = Some("fixture".into());
    }

    #[test]
    fn ordinary_controls_do_not_record_palette_history() {
        let mut state = AppState::new();
        let mut adapter = Adapter::default();
        adapter.activate(Binding::Palette(PaletteCommandKind::ToggleMute), &mut state);
        assert!(state.palette.history.is_empty());
        adapter.activate(
            Binding::PaletteSelection(PaletteCommandKind::ToggleMute),
            &mut state,
        );
        assert_eq!(state.palette.history.rank("ToggleMute"), Some(0));
        app::command_palette::open(&mut state);
        let opened = state.palette.history.clone();
        app::command_palette::open_with_query(&mut state, "mute");
        assert_eq!(
            state.palette.history, opened,
            "Searching an open palette must not age recency"
        );
    }

    #[test]
    fn home_radio_waits_for_source_and_runs_once_without_palette_history() {
        let mut state = AppState::new();
        let mut adapter = Adapter::default();
        state.library_loading = true;
        let actions = adapter.handle(
            Command::HomeAction {
                action: HomeAction::RandomAlbumRadio,
            },
            &mut state,
            &Default::default(),
        );
        assert!(actions.is_empty());
        with_catalog_source(&mut state);
        state.library_loading = false;
        let before = serde_json::to_string(&state.palette.history).unwrap();
        let actions = adapter.take_home_action(&mut state);
        assert!(
            matches!(actions.as_slice(),[Action::Navigation(NavigationAction::SetView(View::NowPlaying)),Action::Radio(RadioAction::PlayStation(station))] if station=="nav-radio/randomAlbum")
        );
        assert!(adapter.take_home_action(&mut state).is_empty());
        assert_eq!(
            before,
            serde_json::to_string(&state.palette.history).unwrap()
        );
        adapter.pending_home_action =
            Some((HomeAction::RandomAlbumRadio, state.library_generation));
        state.library_generation += 1;
        assert!(adapter.take_home_action(&mut state).is_empty());
        assert!(adapter.pending_home_action.is_none());
    }

    #[test]
    fn remote_play_and_pause_are_idempotent() {
        let mut state = AppState::new();
        let mut adapter = Adapter::default();
        for (status, operation, changes) in [
            (PlayStatus::Playing, "play", false),
            (PlayStatus::Paused, "pause", false),
            (PlayStatus::Stopped, "pause", false),
            (PlayStatus::Paused, "play", true),
            (PlayStatus::Playing, "pause", true),
        ] {
            state.playback.status = status;
            let command = serde_json::from_value(serde_json::json!({
                "type": "transport", "operation": operation
            }))
            .unwrap();
            let actions = adapter.handle(command, &mut state, &Default::default());
            assert_eq!(actions.len(), usize::from(changes));
            if changes {
                assert!(matches!(
                    actions.as_slice(),
                    [Action::Playback(PlaybackAction::TogglePlayPause)]
                ));
            }
        }
    }

    #[test]
    fn playlist_navigation_uses_keys_and_rejects_completion_after_back() {
        let mut state = AppState::new();
        with_catalog_source(&mut state);
        state.library.playlists = ["first", "second"]
            .into_iter()
            .map(|key| {
                serde_json::from_value(serde_json::json!({
                    "ratingKey": key, "key": key, "title": "Same name", "type": "audio"
                }))
                .unwrap()
            })
            .collect::<Vec<_>>()
            .into();
        let config = crate::config::Config::default();
        let mut adapter = Adapter::default();
        let mut assets = super::super::projection::Assets::default();
        let snapshot = super::super::projection::model(&state, &mut adapter, &mut assets);
        let roots = snapshot["roots"].as_array().unwrap();
        let playlists: Vec<_> = roots
            .iter()
            .filter(|row| row["label"] == "Same name")
            .collect();
        assert_eq!(playlists.len(), 2);
        assert_ne!(playlists[0]["token"], playlists[1]["token"]);
        let actions = adapter.handle(
            Command::Invoke {
                token: playlists[1]["token"].as_u64().unwrap(),
            },
            &mut state,
            &config,
        );
        assert!(
            matches!(actions.as_slice(), [Action::Miller(MillerAction::LoadPlaylistTracksForMiller { playlist_key, .. })] if playlist_key == "second")
        );
        assert_eq!(
            state.playlist_nav.columns[0].selected_item().unwrap().key(),
            "second"
        );
        state.playlist_nav_request_id = 10;
        let completion = app::event::PlaylistEvent::PlaylistFirstPageLoaded {
            library_key: state.active_library.clone().unwrap(),
            request_id: 10,
            playlist_key: "second".into(),
            tracks: vec![],
            total: Some(0),
        };
        let (tx, _) = tokio::sync::mpsc::channel(32);
        app::dispatch::handle_core_event(app::Event::Playlist(completion), &mut state, &tx);
        let snapshot = super::super::projection::model(&state, &mut adapter, &mut assets);
        assert_eq!(snapshot["columns"].as_array().unwrap().len(), 2);
        assert_eq!(snapshot["columns"][0]["title"], "Playlists");
        adapter.handle(Command::Pop { depth: 1 }, &mut state, &config);
        assert!(matches!(
            state.library_navigation,
            LibraryNavigation::Group(app::sidebar::SidebarGroup::Playlists)
        ));
        app::dispatch::handle_core_event(
            app::Event::Playlist(app::event::PlaylistEvent::PlaylistFirstPageLoaded {
                library_key: state.active_library.clone().unwrap(),
                request_id: 10,
                playlist_key: "second".into(),
                tracks: vec![],
                total: Some(0),
            }),
            &mut state,
            &tx,
        );
        assert_eq!(state.playlist_nav.columns.len(), 1);
    }

    #[test]
    fn leaving_track_details_preserves_its_album_track_column() {
        let mut state = AppState::new();
        state.artist_nav.columns = vec![BrowseColumn::new("Tracks", vec![])];
        state.track_pane_open = true;
        state.library_navigation = LibraryNavigation::Columns(None);
        let mut adapter = Adapter::default();
        adapter.handle(
            Command::Pop { depth: 1 },
            &mut state,
            &crate::config::Config::default(),
        );
        assert!(!state.track_pane_open);
        assert_eq!(state.artist_nav.columns.len(), 1);
        adapter.handle(
            Command::Pop { depth: 0 },
            &mut state,
            &crate::config::Config::default(),
        );
        assert!(matches!(state.library_navigation, LibraryNavigation::Home));
        assert!(state.artist_nav.columns.is_empty());
    }

    async fn dispatch(
        adapter: &mut Adapter,
        session: &mut app::runtime::Session,
        command: Command,
    ) {
        for action in adapter.handle(command, &mut session.state, &session.config) {
            adapter.observe(&action, &mut session.state);
            session.dispatch(action).await.unwrap();
        }
        adapter.sync(&session.state);
    }

    async fn palette(
        adapter: &mut Adapter,
        session: &mut app::runtime::Session,
        command: PaletteCommandKind,
    ) {
        app::command_palette::open(&mut session.state);
        adapter.begin_projection();
        let token = adapter.bind(&session.state, "test-command", Binding::Palette(command));
        adapter.finish_projection();
        dispatch(adapter, session, Command::Invoke { token }).await;
    }

    #[tokio::test]
    async fn palette_navigation_changes_the_phone_destination_and_closes_overlays() {
        let mut session = app::runtime::Session::open(
            crate::config::Config::default(),
            app::runtime::AudioMode::Silent,
        )
        .await
        .unwrap();
        let mut adapter = Adapter::default();
        with_catalog_source(&mut session.state);
        session.state.artist_nav.reset(
            "Artists",
            vec![BrowseItem::Artist {
                key: "artist".into(),
                title: "Artist".into(),
                thumb: None,
                is_placeholder: false,
            }],
        );
        session
            .task_context()
            .run(async {
                palette(&mut adapter, &mut session, PaletteCommandKind::GotoQueue).await;
                assert_eq!(session.state.view, View::Queue);
                assert_eq!(adapter.tab, Tab::Queue);
                assert_eq!(adapter.sheet, None);
                palette(
                    &mut adapter,
                    &mut session,
                    PaletteCommandKind::GotoNowPlaying,
                )
                .await;
                assert_eq!(adapter.tab, Tab::NowPlaying);
                palette(&mut adapter, &mut session, PaletteCommandKind::OpenSettings).await;
                assert_eq!(adapter.sheet, Some(Sheet::Settings));
                assert_eq!(adapter.tab, Tab::NowPlaying);
                palette(&mut adapter, &mut session, PaletteCommandKind::GotoLibrary).await;
                assert!(session.state.library_navigation.has_columns());
                assert_eq!(adapter.tab, Tab::Library);
                assert_eq!(adapter.sheet, None);
                palette(&mut adapter, &mut session, PaletteCommandKind::OpenSort).await;
                assert_eq!(adapter.sheet, Some(Sheet::ViewOptions));
                palette(&mut adapter, &mut session, PaletteCommandKind::ToggleFilter).await;
                assert!(session.state.list_filter.active);
                assert_eq!(adapter.sheet, None);
                palette(
                    &mut adapter,
                    &mut session,
                    PaletteCommandKind::SwitchLibrary,
                )
                .await;
                assert_eq!(adapter.sheet, Some(Sheet::LibraryPicker));
                dispatch(
                    &mut adapter,
                    &mut session,
                    Command::Dismiss {
                        sheet: Sheet::LibraryPicker,
                    },
                )
                .await;
                assert_eq!(adapter.sheet, None);
                palette(&mut adapter, &mut session, PaletteCommandKind::ArtistRadio).await;
                assert_eq!(adapter.sheet, Some(Sheet::ArtistRadio));
                palette(&mut adapter, &mut session, PaletteCommandKind::GotoQueue).await;
                assert_eq!(adapter.sheet, None);
                assert!(session.state.popups.artist_radio_picker().is_none());
            })
            .await;
    }

    #[test]
    fn dismissing_palette_preserves_its_form_and_rejects_a_stale_second_dismissal() {
        let mut state = AppState::new();
        let config = crate::config::Config::default();
        let mut adapter = Adapter::default();
        state.popups.set_input_dialog(Some(InputDialog {
            title: "Save Queue as Playlist".into(),
            input: "Evening".into(),
            action_type: InputDialogAction::SavePlaylist,
        }));
        adapter.handle(
            Command::Sheet {
                sheet: Some(Sheet::Palette),
            },
            &mut state,
            &config,
        );
        adapter.sync(&state);
        assert_eq!(adapter.sheet, Some(Sheet::Palette));
        for _ in 0..2 {
            assert!(
                adapter
                    .handle(
                        Command::Dismiss {
                            sheet: Sheet::Palette
                        },
                        &mut state,
                        &config
                    )
                    .is_empty()
            );
            assert_eq!(adapter.sheet, Some(Sheet::SavePlaylist));
            assert_eq!(
                state.popups.input_dialog().unwrap().input.to_string(),
                "Evening"
            );
        }
        state.popups.close_all();
        adapter.sync(&state);
        assert_eq!(adapter.sheet, None);
    }

    #[test]
    fn open_menu_commands_survive_updates_but_not_removal_or_source_switch() {
        let mut state = AppState::new();
        state.playback_mode = PlaybackMode::Queue;
        state.queue.tracks.push(crate::library::models::Track {
            rating_key: "fixture-track".into(),
            ..Default::default()
        });
        let config = crate::config::Config::default();
        let mut adapter = Adapter::default();
        let mut tokens = vec![];
        for _ in 0..20 {
            adapter.begin_projection();
            tokens.push(adapter.bind(
                &state,
                "queue:repeat",
                Binding::Tool(app::presentation::NpSidebarButton::Repeat),
            ));
            adapter.finish_projection();
        }
        let token = tokens[0];
        assert!(tokens.iter().all(|current| *current == token));
        assert!(matches!(
            adapter
                .handle(Command::Invoke { token }, &mut state, &config)
                .as_slice(),
            [Action::Queue(QueueAction::SetRepeat(RepeatMode::Queue))]
        ));
        assert_eq!(adapter.bindings.len(), 1);

        adapter.begin_projection();
        adapter.finish_projection();
        assert!(
            adapter
                .handle(Command::Invoke { token }, &mut state, &config)
                .is_empty()
        );
        assert!(adapter.bindings.is_empty());

        adapter.begin_projection();
        let token = adapter.bind(
            &state,
            "queue:repeat",
            Binding::Tool(app::presentation::NpSidebarButton::Repeat),
        );
        adapter.finish_projection();
        state.library_generation += 1;
        assert!(
            adapter
                .handle(Command::Invoke { token }, &mut state, &config)
                .is_empty()
        );
        adapter.begin_projection();
        adapter.finish_projection();
        assert!(adapter.bindings.is_empty());
        assert!(adapter.control_tokens.is_empty());
    }
}

#[cfg(test)]
mod connection_edit_tests {
    use super::*;
    #[test]
    fn saved_connection_edits_keep_source_identity_and_reject_stale_form_submissions() {
        let mut state = AppState::new();
        let source = crate::navidrome::Source {
            id: "saved".into(),
            name: "Library".into(),
            url: "http://example.test".into(),
            username: "user".into(),
            libraries: vec![],
        };
        let form = app::sources::dialogs::ServerForm::new(Some(&source));
        let owner = form.instance.clone();
        state.active_library = Some("different".into());
        state
            .popups
            .set_library_dialog(Some(app::sources::dialogs::Dialog::Navidrome(form)));
        let mut adapter = Adapter::default();
        let fields = vec![
            "http://example.test".into(),
            "user".into(),
            String::new(),
            "Edited".into(),
        ];
        assert!(
            adapter
                .handle(
                    Command::Connect {
                        provider: Provider::Navidrome,
                        fields: fields.clone(),
                        owner: Some("expired".into())
                    },
                    &mut state,
                    &Default::default()
                )
                .is_empty()
        );
        let actions = adapter.handle(
            Command::Connect {
                provider: Provider::Navidrome,
                fields,
                owner: Some(owner),
            },
            &mut state,
            &Default::default(),
        );
        assert!(
            matches!(actions.as_slice(), [Action::Source(app::sources::SourceAction::Navidrome(action))] if matches!(action.as_ref(), app::sources::navidrome::NavAction::Password { source, password, .. } if source.id == "saved" && source.name == "Edited" && password.is_empty()))
        );
        assert_eq!(state.active_library.as_deref(), Some("different"));
    }
}
