//! Owns player discovery, configuration, transfer lifetimes and observable progress.
use super::player_device_settings::{Profile, accounts};
use super::{Action, AppState, Event, tasks::TaskLease};
use crate::{
    config::Config,
    services::player_sync::{self as service, Player, Progress, Summary, Volume},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Discovering {
        previous: Box<Phase>,
        quiet: bool,
    },
    Preparing,
    Review {
        operation: u64,
        title: String,
        rows: std::sync::Arc<service::Report>,
    },
    LoadingInventory,
    Inventory {
        volume: String,
        rows: std::sync::Arc<service::Report>,
    },
    Copying(Progress),
    Cancelling,
    Complete(Summary),
    Cancelled,
    Failed(String),
    DiscoveryFailed(String),
}
impl Phase {
    pub fn display(&self) -> &Self {
        match self {
            Self::Discovering {
                previous,
                quiet: true,
            } => previous.display(),
            _ => self,
        }
    }
    pub fn busy(&self) -> bool {
        matches!(
            self,
            Self::Discovering { .. }
                | Self::LoadingInventory
                | Self::Review { .. }
                | Self::Preparing
                | Self::Copying(_)
                | Self::Cancelling
        )
    }
}
#[derive(Debug, Default)]
pub struct State {
    mtp_players: Vec<String>,
    pub(super) history: BTreeMap<String, crate::config::DeviceSync>,
    approval: Option<tokio::sync::oneshot::Sender<()>>,
    pub(crate) report_group: Option<String>,
    pub(crate) report_query: String,
    pub(crate) report_filter_focused: bool,
    profiles: BTreeMap<String, Profile>,
    synced: BTreeMap<String, SyncedFolders>,
    synced_tab: bool,
    pub volumes: Vec<Volume>,
    pub players: Vec<Player>,
    pub(crate) webdav: WebdavPlayers,
    browser: Option<WebdavBrowser>,
    pub(crate) webdav_filter_focused: bool,
    pub selected: Option<String>,
    pub phase: Phase,
    operation: u64,
    task: Option<TaskLease>,
    cancel: Option<watch::Sender<bool>>,
    last_scan: Option<Instant>,
    pub(crate) column_cursor: usize,
    pub(crate) choosing_playlist: bool,
    preview: Option<PlaylistPreview>,
    preview_serial: u64,
}
pub type SyncedFolders = BTreeMap<String, BTreeSet<String>>;
pub type WebdavPlayers = BTreeMap<String, BTreeMap<String, service::WebdavSelection>>;
#[derive(Debug)]
struct WebdavBrowser {
    generation: u64,
    path: String,
    query: String,
    tree: FolderLoad<std::sync::Arc<crate::library::tree::Tree>>,
    listing: FolderLoad<Vec<String>>,
    list_serial: u64,
}
#[derive(Debug)]
enum FolderLoad<T> {
    Idle,
    Loading { _task: TaskLease },
    Ready(T),
    Failed(String),
}
impl<T> FolderLoad<T> {
    fn ready(&self) -> Option<&T> {
        if let Self::Ready(value) = self {
            Some(value)
        } else {
            None
        }
    }
    fn error(&self) -> Option<&String> {
        if let Self::Failed(error) = self {
            Some(error)
        } else {
            None
        }
    }
    fn finish(&mut self, result: Result<T, String>) {
        if matches!(self, Self::Loading { .. }) {
            *self = match result {
                Ok(value) => Self::Ready(value),
                Err(error) => Self::Failed(error),
            };
        }
    }
}
type PlaylistKey = (String, Vec<String>);

#[derive(Debug)]
struct PlaylistPreview {
    key: PlaylistKey,
    serial: u64,
    started: Instant,
    status: PreviewStatus,
}

#[derive(Debug)]
enum PreviewStatus {
    Loading { _task: TaskLease },
    Ready(Option<u64>),
    Unavailable,
}

#[derive(Clone, Debug)]
pub enum Failure {
    Cancelled,
    Error(String),
}
#[derive(Clone, Debug)]
pub enum Command {
    ClearSyncHistory,
    ReviewReady {
        operation: u64,
        title: String,
        rows: std::sync::Arc<service::Report>,
    },
    ApplyReview(u64),
    Inventory,
    InventoryReady {
        operation: u64,
        volume: String,
        result: Result<std::sync::Arc<service::Report>, String>,
    },
    CloseInventory,
    ReportGroup(String),
    ReportFilter(String),
    TogglePlaylistPicker,
    ChooseWebdavFolder {
        volume_uuid: String,
        source_id: String,
        path: String,
    },
    SetWebdavNomedia {
        volume_uuid: String,
        source_id: String,
        enabled: bool,
    },
    BrowseWebdav(String),
    ShowSyncedFolders(bool),
    UpWebdav,
    FilterWebdav(String),
    WebdavTree {
        generation: u64,
        result: Result<std::sync::Arc<crate::library::tree::Tree>, String>,
    },
    WebdavListing {
        serial: u64,
        generation: u64,
        path: String,
        result: Result<Vec<String>, String>,
    },
    ResumeNavidrome,
    LoadPreview,
    PreviewLoaded {
        serial: u64,
        result: Result<Option<u64>, String>,
    },
    ChoosePlaylist {
        volume_uuid: String,
        source_id: String,
        title: String,
    },
    Discover {
        quiet: bool,
    },
    Select(String),
    Link {
        name: String,
        playlist: String,
        volume_uuid: String,
        source_id: String,
    },
    ToggleAuto,
    Start {
        create_playlist: bool,
    },
    Cancel,
    Discovered {
        mtp_players: Vec<String>,
        operation: u64,
        result: Result<Vec<(Volume, Profile, SyncedFolders)>, String>,
    },
    Progress {
        operation: u64,
        progress: Progress,
    },
    Finished {
        operation: u64,
        result: Result<Summary, Failure>,
    },
}
impl From<Command> for Action {
    fn from(command: Command) -> Self {
        Self::PlayerSync(command)
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Menu {
    Refresh,
    Sync,
    Cancel,
}
impl Menu {
    pub fn label(self) -> &'static str {
        match self {
            Self::Refresh => "Refresh Audio Player",
            Self::Sync => "Sync Audio Player",
            Self::Cancel => "Cancel Player Sync",
        }
    }
    pub fn command(self) -> Command {
        match self {
            Self::Refresh => Command::Discover { quiet: false },
            Self::Sync => Command::Start {
                create_playlist: false,
            },
            Self::Cancel => Command::Cancel,
        }
    }
}
pub fn selected(state: &State) -> Option<&Volume> {
    state
        .selected
        .as_ref()
        .and_then(|uuid| state.volumes.iter().find(|volume| &volume.uuid == uuid))
}
pub fn player(state: &State) -> Option<&Player> {
    state
        .players
        .iter()
        .find(|player| Some(&player.volume_uuid) == state.selected.as_ref())
}
fn configured(state: &State) -> Option<&Player> {
    if !state.volumes.is_empty() {
        return player(state);
    }
    player(state).or_else(|| state.players.first())
}

fn active_webdav(state: &AppState) -> Option<&crate::library::FolderSource> {
    let id = state.sources.active.folder()?;
    state.sources.folders.iter().find(|source| {
        &source.id == id
            && matches!(
                source.location,
                crate::library::FolderLocation::Webdav { .. }
            )
    })
}

fn webdav_folders(state: &AppState) -> BTreeSet<String> {
    let mut choices = BTreeSet::from([String::new()]);
    if let Some(source) = active_webdav(state)
        && let Some(nav) = &state.folder_state
        && nav.library_key == format!("folder:{}", source.id)
    {
        choices.extend(
            nav.columns
                .iter()
                .flat_map(|column| &column.items)
                .filter(|item| item.item_type == crate::library::folder::FolderItemType::Folder)
                .map(|item| item.key.clone()),
        );
    }
    choices
}

fn browser(state: &AppState) -> Option<&WebdavBrowser> {
    state
        .player_sync
        .browser
        .as_ref()
        .filter(|browser| browser.generation == state.library_generation)
}

fn browse_webdav(state: &mut AppState, events: &mpsc::Sender<Event>, path: String) {
    let Some(source) = active_webdav(state).cloned() else {
        return;
    };
    if !path.is_empty()
        && !webdav_folders(state).contains(&path)
        && !browser(state).is_some_and(|browser| {
            browser.path.starts_with(&format!("{path}/"))
                || browser.tree.ready().is_some_and(|tree| {
                    tree.directories.contains_key(&path)
                        || tree
                            .directories
                            .get(path.rsplit_once('/').map_or("", |(p, _)| p))
                            .is_some_and(|entries| entries.iter().any(|entry| entry.path == path))
                })
                || browser
                    .listing
                    .ready()
                    .is_some_and(|list| list.contains(&path))
        })
    {
        return;
    }
    let generation = state.library_generation;
    if browser(state).is_none() {
        state.player_sync.browser = Some(WebdavBrowser {
            generation,
            path: String::new(),
            query: String::new(),
            tree: FolderLoad::Idle,
            listing: FolderLoad::Idle,
            list_serial: 0,
        });
    }
    let browser = state
        .player_sync
        .browser
        .as_mut()
        .expect("browser was initialized");
    browser.path = path.clone();
    browser.query.clear();
    browser.listing = FolderLoad::Idle;
    browser.list_serial = browser.list_serial.wrapping_add(1);
    let serial = browser.list_serial;
    if matches!(browser.tree, FolderLoad::Idle | FolderLoad::Failed(_)) {
        let source = source.clone();
        let events = events.clone();
        let task = super::tasks::spawn(async move {
            let cache_source = source.clone();
            let cached = super::tasks::spawn_blocking(move || {
                use crate::library::{
                    cache::Store,
                    tree::{KEY, Tree},
                };
                Store::folder(&cache_source)?
                    .ticket(KEY)
                    .read::<Tree>(Duration::from_secs(604800))
            })
            .await
            .ok()
            .and_then(Result::ok)
            .flatten();
            let result = match cached {
                Some(hit) => Ok(hit.value),
                None => {
                    async {
                        use crate::library::{cache::Store, tree};
                        let ticket = Store::folder(&source)?.ticket(tree::KEY);
                        let tree = tree::scan(&source).await?;
                        super::tasks::spawn_blocking(move || {
                            ticket.write(&tree)?;
                            Ok::<_, anyhow::Error>(tree)
                        })
                        .await?
                    }
                    .await
                }
            }
            .map(std::sync::Arc::new)
            .map_err(|error| format!("{error:#}"));
            let _ = events
                .send(Event::Effect(
                    Command::WebdavTree { generation, result }.into(),
                ))
                .await;
        });
        browser.tree = FolderLoad::Loading {
            _task: TaskLease::new(&task),
        };
    }
    if browser
        .tree
        .ready()
        .is_some_and(|tree| tree.directories.contains_key(&path))
    {
        return;
    }
    let events = events.clone();
    let task = super::tasks::spawn(async move {
        let result = async {
            use crate::library::{
                cache::Store,
                tree::{KEY, Tree},
            };
            let store = Store::folder(&source)?;
            let list = store.ticket(&format!("sync-folders-{:x}", md5::compute(path.as_bytes())));
            let read = list.clone();
            let tree = store.ticket(KEY);
            let folder = path.clone();
            let cached =
                super::tasks::spawn_blocking(move || -> anyhow::Result<Option<Vec<String>>> {
                    if let Some(hit) = tree.read::<Tree>(Duration::from_secs(604800))?
                        && let Some(entries) = hit.value.directories.get(&folder)
                    {
                        return Ok(Some(
                            entries.iter().map(|entry| entry.path.clone()).collect(),
                        ));
                    }
                    Ok(read
                        .read::<Vec<String>>(Duration::from_secs(604800))?
                        .filter(|hit| !hit.stale)
                        .map(|hit| hit.value))
                })
                .await??;
            if let Some(rows) = cached {
                return Ok(rows);
            }
            let rows = source.list(&path).await.map(|entries| {
                entries
                    .into_iter()
                    .filter(|entry| entry.directory)
                    .map(|entry| entry.path)
                    .collect()
            })?;
            super::tasks::spawn_blocking(move || {
                list.write(&rows)?;
                Ok::<_, anyhow::Error>(rows)
            })
            .await?
        }
        .await
        .map_err(|error| format!("{error:#}"));
        let _ = events
            .send(Event::Effect(
                Command::WebdavListing {
                    serial,
                    generation,
                    path,
                    result,
                }
                .into(),
            ))
            .await;
    });
    browser.listing = FolderLoad::Loading {
        _task: TaskLease::new(&task),
    };
}

fn webdav_browser_rows(state: &AppState) -> Vec<String> {
    let path = browser(state).map_or("", |browser| browser.path.as_str());
    let query = browser(state)
        .map_or("", |browser| browser.query.as_str())
        .trim()
        .to_lowercase();
    let prefix = if path.is_empty() {
        String::new()
    } else {
        format!("{path}/")
    };
    if let Some(tree) = browser(state).and_then(|browser| browser.tree.ready()) {
        if query.is_empty() {
            if let Some(entries) = tree.directories.get(path) {
                return entries.iter().map(|entry| entry.path.clone()).collect();
            }
        } else if tree.directories.contains_key(path) {
            return tree
                .directories
                .values()
                .flatten()
                .map(|entry| &entry.path)
                .filter(|folder| folder.as_str() != path)
                .filter(|folder| {
                    folder
                        .strip_prefix(&prefix)
                        .is_some_and(|suffix| suffix.to_lowercase().contains(&query))
                })
                .cloned()
                .collect();
        }
    }
    let choices = browser(state)
        .and_then(|browser| browser.listing.ready().cloned())
        .unwrap_or_else(|| {
            webdav_folders(state)
                .into_iter()
                .filter(|folder| {
                    folder.rsplit_once('/').map_or("", |(parent, _)| parent) == path
                        && !folder.is_empty()
                })
                .collect()
        });
    choices
        .into_iter()
        .filter(|folder| {
            folder
                .strip_prefix(&prefix)
                .is_some_and(|suffix| suffix.to_lowercase().contains(&query))
        })
        .collect()
}

pub(crate) fn webdav_filter(state: &AppState) -> Option<&str> {
    active_webdav(state)?;
    Some(browser(state).map_or("", |browser| browser.query.as_str()))
}

pub(crate) fn webdav_parent(state: &AppState) -> Option<Command> {
    let path = &browser(state)?.path;
    (!path.is_empty()).then_some(Command::UpWebdav)
}

pub(crate) fn webdav_toggle(state: &AppState, path: String) -> Option<Command> {
    Some(Command::ChooseWebdavFolder {
        volume_uuid: selected(&state.player_sync)?.uuid.clone(),
        source_id: active_webdav(state)?.id.clone(),
        path,
    })
}

fn webdav_selection<'a>(
    state: &'a State,
    uuid: &str,
    source: &str,
) -> Option<&'a service::WebdavSelection> {
    state.webdav.get(uuid)?.get(source)
}

fn project_profiles(sync: &mut State, keys: &BTreeMap<String, String>) {
    sync.players.clear();
    sync.webdav.clear();
    for profile in sync.profiles.values() {
        profile.project(keys, &mut sync.players, &mut sync.webdav);
    }
}

fn save_settings(sync: &mut State, config: &Config) -> anyhow::Result<()> {
    let keys = accounts(config);
    let result = (|| {
        let volume = selected(sync).context("Connect the player to change its sync settings")?;
        let previous = sync
            .profiles
            .get(&volume.uuid)
            .context("Refresh the player before changing sync settings")?;
        let mut profile = previous.clone();
        profile.update(sync, &keys);
        profile.write(volume, previous)?;
        sync.profiles.insert(volume.uuid.clone(), profile);
        Ok(())
    })();
    if result.is_err() {
        project_profiles(sync, &keys);
    }
    result
}

fn save_webdav(
    state: &mut AppState,
    config: &mut Config,
    volume_uuid: &str,
    source_id: &str,
    update: impl FnOnce(&mut service::WebdavSelection),
) -> anyhow::Result<Vec<Action>> {
    anyhow::ensure!(
        !state.player_sync.phase.busy(),
        "A player operation is running"
    );
    anyhow::ensure!(
        selected(&state.player_sync).is_some_and(|volume| volume.uuid == volume_uuid)
            && active_webdav(state).is_some_and(|source| source.id == source_id),
        "The selected player or WebDAV library changed"
    );
    if let Some(previous) = webdav_selection(&state.player_sync, volume_uuid, source_id)
        && let Some(profile) = state.player_sync.profiles.get(volume_uuid)
        && let Some(volume) = selected(&state.player_sync)
    {
        service::synced_folders(
            volume,
            &profile.device_id(source_id, &accounts(config)),
            &previous.folders,
            true,
        )?;
    }
    let selection = state
        .player_sync
        .webdav
        .entry(volume_uuid.into())
        .or_default()
        .entry(source_id.into())
        .or_default();
    update(selection);
    save_settings(&mut state.player_sync, config)?;
    Ok(vec![])
}

fn review_request(
    sync: &mut State,
    events: &mpsc::Sender<Event>,
    title: String,
    manual: bool,
) -> impl FnOnce(service::Report) -> std::pin::Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>
+ use<> {
    let (send, receive) = tokio::sync::oneshot::channel();
    sync.approval = manual.then_some(send);
    let operation = sync.operation;
    let events = events.clone();
    move |rows| {
        Box::pin(async move {
            if manual {
                events
                    .send(Event::Effect(
                        Command::ReviewReady {
                            operation,
                            title,
                            rows: std::sync::Arc::new(rows),
                        }
                        .into(),
                    ))
                    .await
                    .map_err(|_| anyhow::anyhow!("Player window closed"))?;
                receive.await.context("Sync review closed")?;
            }
            Ok(())
        })
    }
}

fn start_webdav(
    state: &mut AppState,
    config: &Config,
    events: &mpsc::Sender<Event>,
) -> anyhow::Result<Vec<Action>> {
    anyhow::ensure!(
        !state.player_sync.phase.busy(),
        "A player operation is already running"
    );
    anyhow::ensure!(
        !state.downloads.is_offline(),
        "Go online to sync the player"
    );
    let source = active_webdav(state)
        .context("Select a WebDAV library")?
        .clone();
    let volume = selected(&state.player_sync)
        .context("Choose a connected player")?
        .clone();
    let selection = webdav_selection(&state.player_sync, &volume.uuid, &source.id)
        .context("Choose WebDAV folders to sync")?
        .clone();
    let device_id = state.player_sync.profiles.get(&volume.uuid).map_or_else(
        || source.id.clone(),
        |p| p.device_id(&source.id, &accounts(config)),
    );
    let downloads = state.downloads.store.clone();
    let sync = &mut state.player_sync;
    sync.operation = sync.operation.wrapping_add(1);
    let operation = sync.operation;
    sync.phase = Phase::Preparing;
    let review = review_request(
        sync,
        events,
        format!("{} · {}", volume.name, source.name),
        true,
    );
    let (cancel, mut stopped) = watch::channel(false);
    sync.cancel = Some(cancel);
    let events = events.clone();
    let task = super::tasks::spawn(async move {
        let run = async {
            service::volumes::verify(&volume).await?;
            let target = service::volumes::Target::for_volume(&volume, Some(&device_id)).await?;
            service::review_webdav(
                &source,
                &device_id,
                &target,
                &volume,
                &selection,
                &downloads,
                report_progress(&events, operation),
                review,
            )
            .await
        };
        let result = tokio::select! {
            biased;
            _ = stopped.changed() => Err(Failure::Cancelled),
            result = run => result.map_err(|error| Failure::Error(format!("{error:#}"))),
        };
        let _ = events
            .send(Event::Effect(
                Command::Finished { operation, result }.into(),
            ))
            .await;
    });
    sync.task = Some(TaskLease::new(&task));
    Ok(vec![])
}

fn playlist_key(state: &State) -> Option<PlaylistKey> {
    configured(state).map(|player| (player.source_id.clone(), player.playlists.clone()))
}

pub(crate) fn playlist_size_label(state: &AppState) -> Option<String> {
    if active_webdav(state).is_some() {
        return None;
    }
    let key = playlist_key(&state.player_sync)?;
    let preview = state
        .player_sync
        .preview
        .as_ref()
        .filter(|preview| preview.key == key);
    Some(match preview.map(|preview| &preview.status) {
        Some(PreviewStatus::Ready(Some(bytes))) => {
            format!("Total file size: {}", crate::util::format_bytes(*bytes))
        }
        Some(PreviewStatus::Ready(None)) => {
            "Total file size unavailable · server did not report all file sizes".into()
        }
        Some(PreviewStatus::Unavailable) => "Total file size unavailable".into(),
        _ if state.downloads.is_offline() => "Go online to check total file size".into(),
        _ => "Checking total file size…".into(),
    })
}

pub fn visible(state: &AppState) -> bool {
    cfg!(target_os = "macos")
        && state.view == super::View::Browse
        && state.browse_category == super::BrowseCategory::Library
        && state.desktop_library_root == super::state::LibraryRoot::Mp3Players
        && state.sources.nav_collection.is_none()
}

fn playlist_choices(state: &AppState) -> Vec<String> {
    let Some(session) = state.sources.active.navidrome() else {
        return vec![];
    };
    if configured(&state.player_sync).is_some_and(|player| player.source_id != session.source.id) {
        return vec![];
    }
    let titles = state
        .library
        .playlists
        .iter()
        .filter(|playlist| session.id(&playlist.rating_key).is_ok())
        .map(|playlist| &playlist.title);
    let mut counts = std::collections::HashMap::new();
    for title in titles.clone() {
        *counts.entry(title.to_ascii_lowercase()).or_insert(0) += 1;
    }
    let mut choices: Vec<_> = titles
        .filter(|title| counts.get(&title.to_ascii_lowercase()) == Some(&1))
        .cloned()
        .collect();
    if let Some(player) = configured(&state.player_sync) {
        for title in player.playlists.clone() {
            if !choices
                .iter()
                .any(|choice| choice.eq_ignore_ascii_case(&title))
            {
                choices.push(title);
            }
        }
    }
    choices.sort_unstable_by_key(|title| title.to_lowercase());
    choices
}

/// Keep mounted player state fresh while the column is visible or a player is linked.
pub fn poll(state: &mut AppState, _config: &Config) -> Vec<Action> {
    if !cfg!(target_os = "macos") {
        return vec![];
    }
    let showing = visible(state);
    let key = (showing && active_webdav(state).is_none())
        .then(|| playlist_key(&state.player_sync))
        .flatten();
    let offline = state.downloads.is_offline();
    let needs_browser = showing && active_webdav(state).is_some() && browser(state).is_none();
    let sync = &mut state.player_sync;
    if sync
        .browser
        .as_ref()
        .is_some_and(|browser| browser.generation != state.library_generation)
    {
        sync.browser = None;
        sync.webdav_filter_focused = false;
    }
    if sync.preview.as_ref().is_some_and(|preview| {
        matches!(&preview.status, PreviewStatus::Loading { .. })
            && (offline || key.as_ref() != Some(&preview.key))
    }) {
        sync.preview = None;
    }
    let mut actions = Vec::new();
    if needs_browser {
        actions.push(Command::BrowseWebdav(String::new()).into());
    }
    let scan_interval = if showing || !sync.players.is_empty() {
        10
    } else {
        60
    };
    if !sync.phase.busy()
        && sync
            .last_scan
            .is_none_or(|last| last.elapsed() >= Duration::from_secs(scan_interval))
    {
        actions.push(Command::Discover { quiet: true }.into());
    }
    if !offline
        && let Some(key) = key
        && sync.preview.as_ref().is_none_or(|preview| {
            preview.key != key
                || (!matches!(&preview.status, PreviewStatus::Loading { .. })
                    && preview.started.elapsed() >= Duration::from_secs(60))
        })
    {
        actions.push(Command::LoadPreview.into());
    }
    actions
}

// Keep network chunk updates from flooding the event queue during navigation.
fn report_progress(events: &mpsc::Sender<Event>, operation: u64) -> impl FnMut(Progress) + '_ {
    let mut last = None::<Instant>;
    move |progress| {
        if last.is_none_or(|at| at.elapsed() >= Duration::from_millis(150))
            || progress.completed == progress.total
        {
            last = Some(Instant::now());
            emit(
                events,
                Command::Progress {
                    operation,
                    progress,
                },
            );
        }
    }
}

fn emit(events: &mpsc::Sender<Event>, command: Command) {
    let _ = events.try_send(Event::Effect(command.into()));
}
pub fn dispatch(
    command: Command,
    state: &mut AppState,
    config: &mut Config,
    events: &mpsc::Sender<Event>,
) -> anyhow::Result<Vec<Action>> {
    match dispatch_command(command, state, config, events) {
        Ok(actions) => Ok(actions),
        Err(error) if !state.player_sync.phase.busy() => {
            state.player_sync.phase = Phase::Failed(format!("{error:#}"));
            Ok(vec![])
        }
        Err(error) => Err(error),
    }
}
fn dispatch_command(
    command: Command,
    state: &mut AppState,
    config: &mut Config,
    events: &mpsc::Sender<Event>,
) -> anyhow::Result<Vec<Action>> {
    match command {
        Command::ClearSyncHistory => {
            let previous = std::mem::take(&mut config.player_sync_history);
            if let Err(error) = crate::config::save_config(config) {
                config.player_sync_history = previous;
                return Err(error);
            }
            state.player_sync.history.clear();
        }
        Command::ReviewReady {
            operation,
            title,
            rows,
        } => {
            let sync = &mut state.player_sync;
            if sync.operation == operation && matches!(sync.phase, Phase::Preparing) {
                sync.phase = Phase::Review {
                    operation,
                    title,
                    rows,
                };
                sync.report_filter_focused = false;
                sync.report_group = None;
                sync.report_query.clear();
                sync.column_cursor = 0;
            }
        }
        Command::ApplyReview(expected) => {
            let sync = &mut state.player_sync;
            if matches!(sync.phase,Phase::Review {operation,..} if operation==expected)
                && let Some(approval) = sync.approval.take()
            {
                sync.phase = Phase::Preparing;
                let _ = approval.send(());
            }
        }
        Command::Inventory => {
            let sync = &mut state.player_sync;
            anyhow::ensure!(!sync.phase.busy(), "A player operation is running");
            let volume = selected(sync).context("Choose a connected player")?.clone();
            let keys = accounts(config);
            let mut names = BTreeMap::new();
            for (id, name) in config
                .navidrome_sources
                .iter()
                .map(|s| (&s.id, format!("{} (Navidrome)", s.name)))
                .chain(
                    config
                        .folder_sources
                        .iter()
                        .map(|s| (&s.id, format!("{} (WebDAV)", s.name))),
                )
            {
                let device_id = sync
                    .profiles
                    .get(&volume.uuid)
                    .map_or_else(|| id.clone(), |p| p.device_id(id, &keys));
                names.insert(device_id, name);
            }
            sync.operation = sync.operation.wrapping_add(1);
            let operation = sync.operation;
            sync.phase = Phase::LoadingInventory;
            let events = events.clone();
            let task = super::tasks::spawn(async move {
                let result = async {
                    service::volumes::verify(&volume).await?;
                    service::inventory(&volume, &names).await
                }
                .await
                .map(std::sync::Arc::new)
                .map_err(|e| format!("{e:#}"));
                let _ = events
                    .send(Event::Effect(
                        Command::InventoryReady {
                            operation,
                            volume: volume.uuid,
                            result,
                        }
                        .into(),
                    ))
                    .await;
            });
            sync.task = Some(TaskLease::new(&task));
        }
        Command::InventoryReady {
            operation,
            volume,
            result,
        } => {
            let sync = &mut state.player_sync;
            if sync.operation == operation && matches!(sync.phase, Phase::LoadingInventory) {
                sync.task = None;
                sync.report_filter_focused = false;
                sync.report_group = None;
                sync.report_query.clear();
                sync.column_cursor = 0;
                sync.phase = match result {
                    Ok(rows) => Phase::Inventory { volume, rows },
                    Err(e) => Phase::Failed(e),
                };
            }
        }
        Command::CloseInventory => {
            if matches!(state.player_sync.phase.display(), Phase::Inventory { .. }) {
                state.player_sync.phase = Phase::Idle;
                state.player_sync.column_cursor = 0;
            }
        }
        Command::ReportGroup(group) => {
            let sync = &mut state.player_sync;
            sync.report_group = (sync.report_group.as_ref() != Some(&group)).then_some(group);
        }
        Command::ReportFilter(query) => state.player_sync.report_query = query,
        Command::ShowSyncedFolders(synced) => state.player_sync.synced_tab = synced,
        Command::BrowseWebdav(path) => {
            state.player_sync.synced_tab = false;
            browse_webdav(state, events, path);
        }
        Command::UpWebdav => {
            if let Some(browser) = browser(state).filter(|browser| !browser.path.is_empty()) {
                let parent = browser
                    .path
                    .rsplit_once('/')
                    .map_or("", |(parent, _)| parent)
                    .to_owned();
                browse_webdav(state, events, parent);
            }
        }
        Command::FilterWebdav(query) => {
            if let Some(browser) = state
                .player_sync
                .browser
                .as_mut()
                .filter(|browser| browser.generation == state.library_generation)
            {
                browser.query = query.chars().take(256).collect();
            }
        }
        Command::WebdavTree { generation, result } => {
            if let Some(browser) = state.player_sync.browser.as_mut().filter(|browser| {
                generation == state.library_generation && browser.generation == generation
            }) {
                browser.tree.finish(result);
            }
        }
        Command::WebdavListing {
            serial,
            generation,
            path,
            result,
        } => {
            if let Some(browser) = state.player_sync.browser.as_mut().filter(|browser| {
                generation == state.library_generation
                    && browser.generation == generation
                    && browser.path == path
                    && browser.list_serial == serial
            }) {
                browser.listing.finish(result);
            }
        }
        Command::ChooseWebdavFolder {
            volume_uuid,
            source_id,
            path,
        } => {
            anyhow::ensure!(
                state
                    .player_sync
                    .synced
                    .get(&volume_uuid)
                    .and_then(|m| m.get(&source_id))
                    .is_some_and(|roots| roots.contains(&path))
                    || webdav_folders(state).contains(&path)
                    || webdav_browser_rows(state).contains(&path)
                    || browser(state).is_some_and(|browser| browser.path == path)
                    || webdav_selection(&state.player_sync, &volume_uuid, &source_id)
                        .is_some_and(|selection| selection.folders.contains(&path)),
                "Choose a folder from the active WebDAV library"
            );
            return save_webdav(state, config, &volume_uuid, &source_id, |selection| {
                if !selection.folders.remove(&path) {
                    selection.folders.insert(path);
                }
            });
        }
        Command::SetWebdavNomedia {
            volume_uuid,
            source_id,
            enabled,
        } => {
            return save_webdav(state, config, &volume_uuid, &source_id, |selection| {
                selection.nomedia = enabled;
            });
        }
        Command::LoadPreview => {
            let Some(key) = playlist_key(&state.player_sync) else {
                return Ok(vec![]);
            };
            let (source_id, titles) = key.clone();
            let active = state
                .sources
                .active
                .navidrome()
                .filter(|session| session.source.id == source_id)
                .cloned();
            let source = config
                .navidrome_sources
                .iter()
                .find(|source| source.id == source_id)
                .cloned();
            let sync = &mut state.player_sync;
            sync.preview_serial = sync.preview_serial.wrapping_add(1);
            let serial = sync.preview_serial;
            let events = events.clone();
            let task = super::tasks::spawn(async move {
                let result = async {
                    let session = sync_session(active, source).await?;
                    let playlists = load_playlists(&session.client, &titles, false).await?;
                    Ok::<_, anyhow::Error>(playlist_total(playlists.iter().flat_map(|p| &p.entry)))
                }
                .await
                .map_err(|error| format!("{error:#}"));
                let _ = events
                    .send(Event::Effect(
                        Command::PreviewLoaded { serial, result }.into(),
                    ))
                    .await;
            });
            sync.preview = Some(PlaylistPreview {
                key,
                serial,
                started: Instant::now(),
                status: PreviewStatus::Loading {
                    _task: TaskLease::new(&task),
                },
            });
        }
        Command::PreviewLoaded { serial, result } => {
            let key = playlist_key(&state.player_sync);
            if let Some(preview) = &mut state.player_sync.preview
                && preview.serial == serial
                && key.as_ref() == Some(&preview.key)
            {
                preview.status = match result {
                    Ok(total) => PreviewStatus::Ready(total),
                    Err(error) => {
                        tracing::debug!(%error, "Could not check audio player playlist size");
                        PreviewStatus::Unavailable
                    }
                };
            }
        }
        Command::TogglePlaylistPicker => {
            if !state.player_sync.phase.busy() {
                state.player_sync.choosing_playlist = !state.player_sync.choosing_playlist;
            }
        }
        Command::ChoosePlaylist {
            volume_uuid,
            source_id,
            title,
        } => {
            anyhow::ensure!(
                !state.player_sync.phase.busy(),
                "A player operation is running"
            );
            anyhow::ensure!(
                selected(&state.player_sync).is_some_and(|volume| volume.uuid == volume_uuid),
                "The selected player changed"
            );
            anyhow::ensure!(
                state
                    .sources
                    .active
                    .navidrome()
                    .is_some_and(|session| session.source.id == source_id),
                "The active library changed"
            );
            anyhow::ensure!(
                playlist_choices(state).contains(&title),
                "Choose an available playlist"
            );
            if let Some(player) = state
                .player_sync
                .players
                .iter_mut()
                .find(|p| p.volume_uuid == volume_uuid)
            {
                let mut names = player.playlists.clone();
                if let Some(index) = names
                    .iter()
                    .position(|name| name.eq_ignore_ascii_case(&title))
                {
                    names.remove(index);
                } else {
                    names.push(title);
                }
                player.playlists = names;
            } else {
                let name = selected(&state.player_sync)
                    .expect("checked volume")
                    .name
                    .clone();
                state.player_sync.players.push(Player {
                    volume_uuid,
                    source_id,
                    name,
                    playlists: vec![title],
                    auto_sync: true,
                });
            }
            state.player_sync.phase = Phase::Idle;
            save_settings(&mut state.player_sync, config)?;
        }
        Command::Discover { quiet } => {
            if state.player_sync.phase.busy() {
                return Ok(vec![]);
            }
            let sync = &mut state.player_sync;
            sync.operation = sync.operation.wrapping_add(1);
            let operation = sync.operation;
            sync.last_scan = Some(Instant::now());
            if !quiet {
                sync.preview = None;
            }
            let previous = Box::new(std::mem::take(&mut sync.phase));
            sync.phase = Phase::Discovering { previous, quiet };
            let events = events.clone();
            let task = super::tasks::spawn(async move {
                let mut mtp_players = Vec::new();
                let result = async {
                    let (players, volumes) = service::mtp::volumes().await?;
                    mtp_players = players;
                    super::tasks::spawn_blocking(move || {
                        volumes
                            .into_iter()
                            .map(|volume| {
                                let profile = Profile::read(&volume)?;
                                let folders = profile.synced(&volume)?;
                                Ok((volume, profile, folders))
                            })
                            .collect::<anyhow::Result<Vec<_>>>()
                    })
                    .await?
                }
                .await
                .map_err(|error| format!("{error:#}"));
                let _ = events
                    .send(Event::Effect(
                        Command::Discovered {
                            operation,
                            result,
                            mtp_players,
                        }
                        .into(),
                    ))
                    .await;
            });
            sync.task = Some(TaskLease::new(&task));
        }
        Command::Select(uuid) => {
            if !state.player_sync.phase.busy()
                && state
                    .player_sync
                    .volumes
                    .iter()
                    .any(|volume| volume.uuid == uuid)
            {
                state.player_sync.selected = Some(uuid);
                state.player_sync.choosing_playlist = false;
                state.player_sync.phase = Phase::Idle;
            }
        }
        Command::Link {
            name,
            playlist,
            volume_uuid,
            source_id,
        } => {
            anyhow::ensure!(
                !state.player_sync.phase.busy(),
                "Wait for the current player operation to finish"
            );
            let name = name.trim();
            let playlist = playlist.trim();
            anyhow::ensure!(
                playlist.chars().count() <= 200 && !playlist.chars().any(char::is_control),
                "Enter a playlist name without line breaks"
            );
            anyhow::ensure!(
                !name.is_empty()
                    && name.chars().count() <= 80
                    && !name.chars().any(char::is_control),
                "Enter a player name"
            );
            anyhow::ensure!(
                player(&state.player_sync).is_none(),
                "This player is already set up"
            );
            let volume = selected(&state.player_sync).context("Choose a connected player")?;
            anyhow::ensure!(
                volume.uuid == volume_uuid,
                "The selected player changed; choose it again"
            );
            let source = state
                .sources
                .active
                .navidrome()
                .context("Select a Navidrome library before setting up a player")?;
            anyhow::ensure!(
                source.source.id == source_id,
                "The selected library changed; choose it again"
            );
            let mapping = Player {
                volume_uuid: volume.uuid.clone(),
                name: name.into(),
                source_id: source.source.id.clone(),
                playlists: vec![if playlist.is_empty() {
                    format!("Sync: {name}")
                } else {
                    playlist.into()
                }],
                auto_sync: true,
            };
            state
                .player_sync
                .players
                .retain(|player| player.volume_uuid != mapping.volume_uuid);
            state.player_sync.players.push(mapping);
            save_settings(&mut state.player_sync, config)?;
            return Ok(vec![
                Command::Start {
                    create_playlist: true,
                }
                .into(),
            ]);
        }
        Command::ToggleAuto => {
            if !state.player_sync.phase.busy()
                && let Some(player) =
                    state.player_sync.players.iter_mut().find(|player| {
                        Some(&player.volume_uuid) == state.player_sync.selected.as_ref()
                    })
            {
                player.auto_sync = !player.auto_sync;
                save_settings(&mut state.player_sync, config)?;
            }
        }
        Command::Start {
            create_playlist: false,
        } if active_webdav(state).is_some() => {
            return start_webdav(state, config, events);
        }
        command @ (Command::Start { .. } | Command::ResumeNavidrome) => {
            let create_playlist = matches!(
                command,
                Command::Start {
                    create_playlist: true
                }
            );
            anyhow::ensure!(
                !state.player_sync.phase.busy(),
                "A player operation is already running"
            );
            anyhow::ensure!(
                !state.downloads.is_offline(),
                "Go online to sync the player"
            );
            let volume = selected(&state.player_sync)
                .context("Choose a connected player")?
                .clone();
            let player = player(&state.player_sync)
                .context("Set up this player first")?
                .clone();
            let active = state
                .sources
                .active
                .navidrome()
                .filter(|source| source.source.id == player.source_id)
                .cloned();
            let source = config
                .navidrome_sources
                .iter()
                .find(|source| source.id == player.source_id)
                .cloned()
                .context("The player's library account is no longer configured")?;
            let mut player = player;
            let local_source_id = player.source_id.clone();
            if let Some(profile) = state.player_sync.profiles.get(&volume.uuid) {
                player.source_id = profile.device_id(&local_source_id, &accounts(config));
            }
            let downloads = state.downloads.store.clone();
            let sync = &mut state.player_sync;
            sync.operation = sync.operation.wrapping_add(1);
            let operation = sync.operation;
            sync.phase = Phase::Preparing;
            let review = review_request(
                sync,
                events,
                format!("{} · {}", volume.name, source.name),
                matches!(command, Command::Start { .. }),
            );
            let (cancel, mut stopped) = watch::channel(false);
            sync.cancel = Some(cancel);
            let events = events.clone();
            let task = super::tasks::spawn(async move {
                let run = async {
                    service::volumes::verify(&volume).await?;
                    let target =
                        service::volumes::Target::for_volume(&volume, Some(&player.source_id))
                            .await?;
                    let session = sync_session(active, Some(source)).await?;
                    let mut playlists = Vec::new();
                    for playlist in
                        load_playlists(&session.client, &player.playlists, create_playlist).await?
                    {
                        let songs = playlist
                            .entry
                            .into_iter()
                            .map(|song| service::Song {
                                id: song.id.clone(),
                                track: {
                                    let mut track = session.track(song);
                                    if local_source_id != player.source_id
                                        && let Some(thumb) = track.thumb.as_mut()
                                    {
                                        *thumb = thumb.replacen(
                                            &format!("navidrome:{local_source_id}:"),
                                            &format!("navidrome:{}:", player.source_id),
                                            1,
                                        );
                                    }
                                    track
                                },
                            })
                            .collect();
                        playlists.push(service::Playlist {
                            title: playlist.name,
                            songs,
                        });
                    }
                    service::volumes::verify(&volume).await?;
                    service::review_playlists(
                        &session.client,
                        &target,
                        &volume,
                        &player,
                        &playlists,
                        &downloads,
                        report_progress(&events, operation),
                        review,
                    )
                    .await
                };
                let result = tokio::select! {biased;_=stopped.changed()=>Err(Failure::Cancelled),result=run=>result.map_err(|error|Failure::Error(format!("{error:#}")))};
                let _ = events
                    .send(Event::Effect(
                        Command::Finished { operation, result }.into(),
                    ))
                    .await;
            });
            sync.task = Some(TaskLease::new(&task));
        }
        Command::Cancel => {
            if let Some(cancel) = &state.player_sync.cancel {
                let _ = cancel.send(true);
                state.player_sync.phase = Phase::Cancelling;
            }
        }
        Command::Discovered {
            operation,
            result,
            mtp_players,
        } => {
            let webdav_id = active_webdav(state).map(|s| s.id.clone());
            let sync = &mut state.player_sync;
            if sync.operation != operation || !matches!(sync.phase, Phase::Discovering { .. }) {
                return Ok(vec![]);
            }
            sync.task = None;
            sync.mtp_players = mtp_players;
            let previous = match std::mem::take(&mut sync.phase) {
                Phase::Discovering { previous, .. } => *previous,
                phase => phase,
            };
            match result {
                Ok(records) => {
                    let keys = accounts(config);
                    let mut migrated = false;
                    let mut profiles = BTreeMap::new();
                    let mut volumes = Vec::new();
                    sync.synced.clear();
                    for (volume, mut profile, synced) in records {
                        sync.synced.insert(
                            volume.uuid.clone(),
                            synced
                                .into_iter()
                                .map(|(id, roots)| (profile.local_id(&id, &keys), roots))
                                .collect(),
                        );
                        let legacy = config.players.iter().find(|p| p.volume_uuid == volume.uuid);
                        let folders = config.webdav_player_sync.get(&volume.uuid);
                        if legacy.is_some() || folders.is_some() {
                            if profile.empty() {
                                let mut old = State::default();
                                old.players.extend(legacy.cloned());
                                if let Some(folders) = folders {
                                    old.webdav.insert(volume.uuid.clone(), folders.clone());
                                }
                                let previous = profile.clone();
                                profile.update(&old, &keys);
                                profile.write(&volume, &previous)?;
                            }
                            config.players.retain(|p| p.volume_uuid != volume.uuid);
                            config.webdav_player_sync.remove(&volume.uuid);
                            migrated = true;
                        }
                        profiles.insert(volume.uuid.clone(), profile);
                        volumes.push(volume);
                    }
                    if migrated {
                        super::handlers::dispatch_settings::save_config_in_background(
                            events,
                            config,
                            "finish moving sync settings to the player",
                        );
                    }
                    sync.profiles = profiles;
                    project_profiles(sync, &keys);
                    let changed = volumes.len() != sync.volumes.len()
                        || volumes
                            .iter()
                            .any(|volume| !sync.volumes.iter().any(|old| old.uuid == volume.uuid));
                    let reconnect = match volumes.as_slice() {
                        [volume] => {
                            !sync.volumes.iter().any(|old| old.uuid == volume.uuid)
                                && sync.players.iter().any(|player| {
                                    player.volume_uuid == volume.uuid && player.auto_sync
                                })
                        }
                        _ => false,
                    };
                    match volumes.as_slice() {
                        [volume] => sync.selected = Some(volume.uuid.clone()),
                        [] => sync.selected = None,
                        _ if !volumes
                            .iter()
                            .any(|v| Some(&v.uuid) == sync.selected.as_ref()) =>
                        {
                            let mut linked = volumes.iter().filter(|v| {
                                webdav_id.as_ref().is_some_and(|id| {
                                    sync.webdav
                                        .get(&v.uuid)
                                        .is_some_and(|sources| sources.contains_key(id))
                                })
                            });
                            sync.selected = linked
                                .next()
                                .filter(|_| linked.next().is_none())
                                .map(|v| v.uuid.clone());
                        }
                        _ if changed && webdav_id.is_none() => sync.selected = None,
                        _ => {}
                    }
                    if changed {
                        sync.choosing_playlist = false;
                    }
                    sync.volumes = volumes;
                    sync.phase = match previous {
                        Phase::Inventory { ref volume, .. }
                            if sync.selected.as_ref() != Some(volume) =>
                        {
                            Phase::Idle
                        }
                        Phase::DiscoveryFailed(_) => Phase::Idle,
                        phase => phase,
                    };
                    if reconnect && !state.downloads.is_offline() {
                        return Ok(vec![Command::ResumeNavidrome.into()]);
                    }
                }
                Err(error) => sync.phase = Phase::DiscoveryFailed(error),
            }
        }
        Command::Progress {
            operation,
            progress,
        } => {
            if state.player_sync.operation == operation
                && matches!(
                    state.player_sync.phase,
                    Phase::Preparing | Phase::Copying(_)
                )
            {
                state.player_sync.phase = Phase::Copying(progress);
            }
        }
        Command::Finished { operation, result } => {
            if result.as_ref().is_ok_and(|s| s.listens > 0) {
                state.listening_history = crate::app::listening_history::load()?;
            }
            let sync = &mut state.player_sync;
            if sync.operation != operation
                || !matches!(
                    sync.phase,
                    Phase::Preparing | Phase::Review { .. } | Phase::Copying(_) | Phase::Cancelling
                )
            {
                return Ok(vec![]);
            }
            sync.approval = None;
            sync.cancel = None;
            sync.task = None;
            sync.phase = match result {
                Ok(summary) => Phase::Complete(summary),
                Err(Failure::Cancelled) => Phase::Cancelled,
                Err(Failure::Error(error)) => Phase::Failed(error),
            };
            let mut actions = vec![];
            if matches!(sync.phase, Phase::Complete(_))
                && let Some(volume) = selected(sync)
            {
                let record = crate::config::DeviceSync {
                    name: player(sync)
                        .filter(|p| p.volume_uuid == volume.uuid)
                        .map_or_else(|| volume.name.clone(), |p| p.name.clone()),
                    completed_at: time::OffsetDateTime::now_utc().unix_timestamp(),
                };
                config
                    .player_sync_history
                    .insert(volume.uuid.clone(), record);
                sync.history.clone_from(&config.player_sync_history);
                if let Err(error) = crate::config::save_config(config) {
                    tracing::warn!("Could not save player sync history: {error}");
                    actions.push(
                        super::action::SystemAction::SetStatus(format!(
                            "Sync completed, but its history could not be saved: {error}"
                        ))
                        .into(),
                    );
                }
            }
            if let Some(player) = player(sync)
                && state
                    .sources
                    .active
                    .navidrome()
                    .is_some_and(|session| session.source.id == player.source_id)
                && player.playlists.iter().any(|title| {
                    !state
                        .library
                        .playlists
                        .iter()
                        .any(|playlist| playlist.title.eq_ignore_ascii_case(title))
                })
            {
                actions.push(super::action::DataAction::LoadPlaylists.into());
            }
            return Ok(actions);
        }
    }
    Ok(vec![])
}
async fn sync_session(
    active: Option<super::sources::navidrome::Session>,
    source: Option<crate::navidrome::Source>,
) -> anyhow::Result<super::sources::navidrome::Session> {
    if let Some(active) = active {
        return Ok(active);
    }
    let source = source.context("The player's library account is no longer configured")?;
    let id = source.id.clone();
    let secret = super::tasks::spawn_blocking(move || {
        crate::library::credentials::load_source("navidrome", &id)
    })
    .await??
    .context("The player's library password is unavailable")?;
    Ok(super::sources::navidrome::Session {
        client: crate::navidrome::Client::new(&source, secret.as_str().into(), None)?,
        source,
        extensions: Default::default(),
    })
}

async fn load_playlists(
    client: &crate::navidrome::Client,
    titles: &[String],
    create: bool,
) -> anyhow::Result<Vec<crate::navidrome::Playlist>> {
    use futures::{StreamExt, TryStreamExt, stream};
    if titles.is_empty() {
        return Ok(Vec::new());
    }
    let mut by_name = std::collections::HashMap::<String, Vec<String>>::new();
    for playlist in client.playlists().await? {
        by_name
            .entry(playlist.name.to_ascii_lowercase())
            .or_default()
            .push(playlist.id);
    }
    let mut ids = Vec::with_capacity(titles.len());
    for title in titles {
        let matches = by_name.entry(title.to_ascii_lowercase()).or_default();
        let id = match matches.as_slice() {
            [id] => id.clone(),
            [] if create => {
                let response = client
                    .call("createPlaylist", &[("name", title.clone())])
                    .await?;
                let id = response["playlist"]["id"]
                    .as_str()
                    .context("The server did not return the created playlist")?
                    .to_owned();
                matches.push(id.clone());
                id
            }
            [] => anyhow::bail!("Playlist {title} was not found"),
            _ => anyhow::bail!("More than one playlist is named {title}; rename the duplicate"),
        };
        ids.push(id);
    }
    stream::iter(ids.into_iter().map(|id| async move {
        let playlist = client.playlist(&id).await?;
        anyhow::ensure!(
            playlist.id == id
                && playlist
                    .song_count
                    .is_none_or(|n| n as usize == playlist.entry.len()),
            "Server returned an incomplete or different playlist"
        );
        Ok(playlist)
    }))
    .buffered(4)
    .try_collect()
    .await
}

fn playlist_total<'a>(songs: impl IntoIterator<Item = &'a crate::navidrome::Song>) -> Option<u64> {
    let mut seen = std::collections::HashSet::new();
    songs
        .into_iter()
        .filter(|song| seen.insert(song.id.as_str()))
        .try_fold(0u64, |total, song| total.checked_add(song.size?))
}

use anyhow::Context;

#[derive(Clone, Debug, Default)]
pub struct View {
    pub connection: String,
    pub history: Vec<String>,
    pub busy: bool,
    pub report_query: String,
    pub report_group: Option<String>,
    pub volumes: Vec<Volume>,
    pub players: Vec<Player>,
    pub selected: Option<String>,
    pub phase: Phase,
    pub can_cancel: bool,
    pub can_link: bool,
    pub library: String,
    pub active_source: Option<String>,
    pub sync_disabled: Option<&'static str>,
    pub playlists: Vec<String>,
    pub visible: bool,
    pub configured_player: Option<Player>,
    pub size_label: Option<String>,
    pub webdav: Option<WebdavView>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebdavView {
    pub source_id: String,
    pub name: String,
    pub folders: Vec<(String, bool)>,
    pub nomedia: bool,
    pub path: String,
    pub query: String,
    pub current_checked: bool,
    pub mixed: BTreeSet<String>,
    pub inherited: BTreeSet<String>,
    pub synced_tab: bool,
    pub selected_count: usize,
    pub loading: bool,
    pub error: Option<String>,
}

fn webdav_view(state: &AppState) -> Option<WebdavView> {
    let source = active_webdav(state)?;
    let folders = webdav_browser_rows(state);
    let path = browser(state)
        .map_or("", |browser| browser.path.as_str())
        .to_owned();
    let selection = selected(&state.player_sync)
        .and_then(|volume| webdav_selection(&state.player_sync, &volume.uuid, &source.id))
        .cloned()
        .unwrap_or_default();

    let synced_tab = state.player_sync.synced_tab;
    let recorded = selected(&state.player_sync)
        .and_then(|v| state.player_sync.synced.get(&v.uuid))
        .and_then(|records| records.get(&source.id));
    let folders = if synced_tab {
        recorded.into_iter().flatten().cloned().collect()
    } else {
        folders
    };
    let checked = |folder: &str| {
        selection
            .folders
            .iter()
            .any(|p| service::contains_folder(p, folder))
    };
    let inherited = folders
        .iter()
        .chain([&path])
        .filter(|p| checked(p) && !selection.folders.contains(*p))
        .cloned()
        .collect();
    let mixed = folders
        .iter()
        .chain([&path])
        .filter(|p| {
            !checked(p)
                && selection
                    .folders
                    .iter()
                    .any(|child| service::contains_folder(p, child))
        })
        .cloned()
        .collect();
    Some(WebdavView {
        source_id: source.id.clone(),
        name: source.name.clone(),
        synced_tab,
        mixed,
        inherited,
        folders: folders
            .into_iter()
            .map(|path| {
                let selected = checked(&path);
                (path, selected)
            })
            .collect(),
        nomedia: selection.nomedia,
        current_checked: checked(&path),
        selected_count: selection.folders.len(),
        path,
        query: webdav_filter(state).unwrap_or_default().to_owned(),
        loading: browser(state).is_some_and(|browser| {
            matches!(browser.listing, FolderLoad::Loading { .. })
                || (!browser.query.is_empty() && matches!(browser.tree, FolderLoad::Loading { .. }))
        }),
        error: browser(state).and_then(|browser| {
            browser
                .listing
                .error()
                .or_else(|| browser.tree.error())
                .cloned()
        }),
    })
}
fn history_rows(sync: &State) -> Vec<String> {
    let mut records: Vec<_> = sync.history.values().collect();
    records.sort_by_key(|record| std::cmp::Reverse(record.completed_at));
    records
        .into_iter()
        .filter_map(|record| {
            let utc = time::OffsetDateTime::from_unix_timestamp(record.completed_at).ok()?;
            let offset = time::UtcOffset::local_offset_at(utc).unwrap_or(time::UtcOffset::UTC);
            let local = utc.to_offset(offset);
            Some(format!(
                "{} · {} {:02}:{:02} {}",
                record.name,
                local.date(),
                local.hour(),
                local.minute(),
                offset
            ))
        })
        .collect()
}

pub fn project(state: &AppState) -> View {
    View {
        connection: connection(&state.player_sync),
        history: history_rows(&state.player_sync),
        busy: state.player_sync.phase.busy(),
        report_query: state.player_sync.report_query.clone(),
        report_group: state.player_sync.report_group.clone(),
        sync_disabled: disabled(state, Menu::Sync),
        webdav: webdav_view(state),
        playlists: if visible(state) {
            playlist_choices(state)
        } else {
            vec![]
        },
        visible: visible(state),
        configured_player: configured(&state.player_sync).cloned(),
        size_label: playlist_size_label(state),
        volumes: state.player_sync.volumes.clone(),
        players: state.player_sync.players.clone(),
        selected: state.player_sync.selected.clone(),
        phase: state.player_sync.phase.display().clone(),
        can_cancel: state.player_sync.cancel.is_some()
            && !matches!(state.player_sync.phase, Phase::Cancelling),
        can_link: state.sources.active.navidrome().is_some(),
        active_source: state
            .sources
            .active
            .navidrome()
            .map(|source| source.source.id.clone()),
        library: state
            .sources
            .active
            .navidrome()
            .map(|source| source.source.name.clone())
            .unwrap_or_default(),
    }
}
pub fn disabled(state: &AppState, menu: Menu) -> Option<&'static str> {
    let sync = &state.player_sync;
    if !cfg!(target_os = "macos") {
        return Some("Available in the Mac app");
    }
    match menu {
        Menu::Cancel if sync.cancel.is_none() || matches!(sync.phase, Phase::Cancelling) => {
            Some("No player sync is running")
        }
        Menu::Cancel => None,
        _ if sync.phase.busy() => Some("A player operation is running"),
        Menu::Refresh => None,
        _ if selected(sync).is_none() => Some("Choose a connected player"),
        Menu::Sync
            if let Some(source) = active_webdav(state)
                && selected(sync)
                    .and_then(|volume| webdav_selection(sync, &volume.uuid, &source.id))
                    .is_none() =>
        {
            Some("Choose WebDAV folders to sync")
        }
        Menu::Sync if active_webdav(state).is_none() && player(sync).is_none() => {
            Some("Set up the selected player first")
        }
        Menu::Sync if state.downloads.is_offline() => Some("Go online to sync the player"),
        _ => None,
    }
}

pub(crate) fn report_groups(phase: &Phase) -> Vec<&str> {
    match phase {
        Phase::Review { rows, .. } => service::CHANGE_GROUPS
            .into_iter()
            .chain(
                ["Playlists", "Media scanner"]
                    .into_iter()
                    .filter(|key| rows.contains_key(*key)),
            )
            .collect(),
        Phase::Inventory { rows, .. } => rows.keys().map(String::as_str).collect(),
        _ => vec![],
    }
}
pub(crate) fn column_rows(state: &AppState) -> Vec<(String, Option<Command>)> {
    let sync = &state.player_sync;
    let linked = configured(sync);
    let mounted = selected(sync);
    if let Phase::Review { rows: report, .. } | Phase::Inventory { rows: report, .. } =
        sync.phase.display()
    {
        let mut rows = if let Phase::Review { operation, .. } = sync.phase.display() {
            vec![
                ("Apply Sync".into(), Some(Command::ApplyReview(*operation))),
                ("Cancel".into(), Some(Command::Cancel)),
            ]
        } else {
            vec![("Done".into(), Some(Command::CloseInventory))]
        };
        rows.push((format!("Filter (/): {}", sync.report_query), None));
        for group in report_groups(sync.phase.display()) {
            let entries = &report[group];
            rows.push((
                format!("{} ({})", group, entries.len()),
                Some(Command::ReportGroup(group.into())),
            ));
            if sync.report_group.as_deref() == Some(group) || !sync.report_query.is_empty() {
                let query = sync.report_query.to_lowercase();
                rows.extend(
                    entries
                        .iter()
                        .filter(|row| row.to_lowercase().contains(&query))
                        .map(|row| (row.clone(), None)),
                );
            }
        }
        return rows;
    }
    let choices = playlist_choices(state);
    let mut rows = vec![
        (
            "Review Sync…".into(),
            disabled(state, Menu::Sync)
                .is_none()
                .then(|| Menu::Sync.command()),
        ),
        (
            "Refresh players".into(),
            disabled(state, Menu::Refresh)
                .is_none()
                .then(|| Menu::Refresh.command()),
        ),
    ];
    if let (Some(volume), Some(webdav)) = (mounted, webdav_view(state)) {
        let volume_uuid = &volume.uuid;
        rows.push((
            format!(
                "Folders: {} · {} selected",
                if webdav.path.is_empty() {
                    &webdav.name
                } else {
                    &webdav.path
                },
                webdav.selected_count
            ),
            webdav_parent(state),
        ));
        rows.push((
            if webdav.synced_tab {
                "Browse folders"
            } else {
                "Synced folders"
            }
            .into(),
            Some(Command::ShowSyncedFolders(!webdav.synced_tab)),
        ));
        if !webdav.synced_tab {
            rows.push((format!("Filter (/): {}", webdav.query), None));
            rows.push((
                format!(
                    "[{}] {}",
                    if webdav.current_checked {
                        "x"
                    } else if webdav.mixed.contains(&webdav.path) {
                        "−"
                    } else {
                        " "
                    },
                    if webdav.path.is_empty() {
                        "Entire library"
                    } else {
                        "This folder"
                    }
                ),
                (!sync.phase.busy())
                    .then(|| webdav_toggle(state, webdav.path.clone()))
                    .flatten(),
            ));
        }
        let prefix = if webdav.synced_tab || webdav.path.is_empty() {
            String::new()
        } else {
            format!("{}/", webdav.path)
        };
        rows.extend(webdav.folders.iter().map(|(path, checked)| {
            (
                format!(
                    "  [{}] {}",
                    if *checked {
                        "x"
                    } else if webdav.mixed.contains(path) {
                        "−"
                    } else {
                        " "
                    },
                    path.strip_prefix(&prefix).unwrap_or(path)
                ),
                if webdav.synced_tab {
                    webdav_toggle(state, path.clone())
                } else {
                    Some(Command::BrowseWebdav(path.clone()))
                },
            )
        }));
        if webdav.loading {
            rows.push(("Loading folders…".into(), None));
        }
        if let Some(error) = &webdav.error {
            rows.push((error.clone(), None));
        }
        rows.push((
            format!(
                "[{}] Hide from media scanner (.nomedia)",
                if webdav.nomedia { "x" } else { " " }
            ),
            (!sync.phase.busy()).then(|| Command::SetWebdavNomedia {
                volume_uuid: volume_uuid.clone(),
                source_id: webdav.source_id,
                enabled: !webdav.nomedia,
            }),
        ));
    } else if let (Some(volume_uuid), Some(session)) = (
        mounted
            .map(|v| &v.uuid)
            .or_else(|| linked.map(|p| &p.volume_uuid)),
        state.sources.active.navidrome(),
    ) {
        let current = linked
            .map(|player| player.playlists.clone())
            .unwrap_or_default();
        let can_choose = !sync.phase.busy() && !choices.is_empty();
        rows.push((
            format!("Playlists: {} selected", current.len()),
            can_choose.then_some(Command::TogglePlaylistPicker),
        ));
        if sync.choosing_playlist && can_choose {
            rows.extend(choices.into_iter().map(|title| {
                let check = if current.iter().any(|name| name.eq_ignore_ascii_case(&title)) {
                    "✓"
                } else {
                    " "
                };
                (
                    format!("  {check} {title}"),
                    Some(Command::ChoosePlaylist {
                        volume_uuid: volume_uuid.clone(),
                        source_id: session.source.id.clone(),
                        title,
                    }),
                )
            }));
        }
    } else if let Some(player) = linked {
        rows.push((
            format!("Playlists: {}", player.playlists.clone().join(", ")),
            None,
        ));
    }
    if active_webdav(state).is_none()
        && let Some(player) = linked
    {
        let check = if player.auto_sync { "x" } else { " " };
        rows.push((
            format!("[{check}] Sync when connected"),
            (mounted.is_some() && !sync.phase.busy()).then_some(Command::ToggleAuto),
        ));
    }
    if mounted.is_some() {
        rows.push((
            "On this player".into(),
            (!sync.phase.busy()).then_some(Command::Inventory),
        ));
    }
    if disabled(state, Menu::Cancel).is_none() {
        rows.push(("Cancel Sync".into(), Some(Command::Cancel)));
    }
    rows.extend(sync.volumes.iter().map(|volume| {
        let check = if Some(&volume.uuid) == sync.selected.as_ref() {
            "●"
        } else {
            "○"
        };
        (
            format!(
                "{check} {} ({})",
                volume.name,
                crate::util::format_bytes(volume.available)
            ),
            (!sync.phase.busy()).then(|| Command::Select(volume.uuid.clone())),
        )
    }));
    if !sync.history.is_empty() {
        rows.push(("Last successful sync".into(), None));
        rows.extend(history_rows(sync).into_iter().map(|row| (row, None)));
        rows.push(("Clear Sync History".into(), Some(Command::ClearSyncHistory)));
    }
    rows
}

pub(crate) fn connection(sync: &State) -> String {
    if !sync.mtp_players.is_empty() {
        format!(
            "{} connected via MTP. No Textamp player profile found.",
            sync.mtp_players.join(", ")
        )
    } else if sync.volumes.is_empty() {
        "Connect a player in MTP (file transfer) mode.".into()
    } else {
        String::new()
    }
}

pub(crate) fn status(state: &AppState) -> String {
    match state.player_sync.phase.display() {
        Phase::Idle
            if state.player_sync.volumes.len() > 1 && selected(&state.player_sync).is_none() =>
        {
            "Choose a storage volume before syncing.".into()
        }
        Phase::Idle if state.player_sync.volumes.is_empty() => String::new(),
        Phase::Idle
            if state.sources.active.navidrome().is_none()
                && player(&state.player_sync).is_none() =>
        {
            "Select a Navidrome library to choose a playlist.".into()
        }
        Phase::Idle => String::new(),
        Phase::Discovering { .. } => "Checking connected players…".into(),
        Phase::LoadingInventory => "Reading player inventory…".into(),
        Phase::Inventory { .. } => "On this player".into(),
        Phase::Review { .. } => "Review sync changes".into(),
        Phase::Preparing => "Preparing sync…".into(),
        Phase::Copying(progress) => format!(
            "{} of {} · {}",
            progress.completed, progress.total, progress.title
        ),
        Phase::Cancelling => "Stopping the transfer…".into(),
        Phase::Complete(summary) => {
            let mut text = format!(
                "Sync complete · {} copied · {} unchanged · {} removed",
                summary.copied, summary.reused, summary.removed
            );
            if summary.listens > 0 {
                text.push_str(&format!(" · {} listens imported", summary.listens));
            }
            if summary.unconfirmed_listens > 0 {
                text.push_str(&format!(
                    " · {} unconfirmed submissions",
                    summary.unconfirmed_listens
                ));
            }
            text
        }
        Phase::Cancelled => "Sync stopped.".into(),
        Phase::Failed(error) | Phase::DiscoveryFailed(error) => error.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn volume(uuid: &str) -> Volume {
        Volume {
            uuid: uuid.into(),
            name: uuid.into(),
            mount: format!("/Volumes/{uuid}").into(),
            identifier: uuid.into(),
            available: 100,
            capacity: 200,
        }
    }
    fn events() -> mpsc::Sender<Event> {
        mpsc::channel(8).0
    }
    #[tokio::test]
    async fn sync_review_applies_only_the_visible_operation() {
        let mut app = AppState::new();
        let mut config = Config::default();
        let (tx, mut rx) = mpsc::channel(4);
        app.player_sync.operation = 21;
        app.player_sync.phase = Phase::Preparing;
        let request = review_request(&mut app.player_sync, &tx, "Library".into(), true);
        let job = tokio::spawn(request(service::Report::new()));
        let Some(Event::Effect(Action::PlayerSync(ready))) = rx.recv().await else {
            panic!("missing review")
        };
        dispatch(ready, &mut app, &mut config, &tx).unwrap();
        dispatch(Command::ApplyReview(20), &mut app, &mut config, &tx).unwrap();
        assert!(!job.is_finished());
        assert!(matches!(
            app.player_sync.phase,
            Phase::Review { operation: 21, .. }
        ));
        dispatch(Command::ApplyReview(21), &mut app, &mut config, &tx).unwrap();
        job.await.unwrap().unwrap();
    }
    #[tokio::test]
    async fn playlist_reads_share_the_catalog_and_keep_selection_order() {
        use serde_json::{Value, json};
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::{TcpListener, TcpStream};

        async fn request(listener: &TcpListener) -> (TcpStream, reqwest::Url) {
            let (socket, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(socket);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let path = line.split_whitespace().nth(1).unwrap();
            let url = reqwest::Url::parse(&format!("http://localhost{path}")).unwrap();
            loop {
                line.clear();
                reader.read_line(&mut line).await.unwrap();
                if line.trim().is_empty() {
                    break;
                }
            }
            (reader.into_inner(), url)
        }

        async fn reply(mut socket: TcpStream, payload: Value) {
            let body = json!({"subsonic-response": payload}).to_string();
            let header = format!(
                "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let titles = ["F", "B", "D", "A", "E", "C"].map(str::to_owned);
        let catalog: Vec<_> = titles
            .iter()
            .map(|id| json!({"id": id,"name": id}))
            .collect();
        let server = async {
            let (socket, url) = request(&listener).await;
            assert_eq!(url.path(), "/rest/getPlaylists");
            reply(
                socket,
                json!({"status":"ok","playlists":{"playlist":catalog}}),
            )
            .await;
            for count in [4, 2] {
                let mut pending = Vec::new();
                for _ in 0..count {
                    let (socket, url) = request(&listener).await;
                    assert_eq!(url.path(), "/rest/getPlaylist");
                    let id = url
                        .query_pairs()
                        .find(|(key, _)| key == "id")
                        .unwrap()
                        .1
                        .into_owned();
                    pending.push((socket, id));
                }
                for (socket, id) in pending.into_iter().rev() {
                    reply(
                        socket,
                        json!({"status":"ok","playlist":{"id":id,"name":id,"songCount":0}}),
                    )
                    .await;
                }
            }
        };
        let source = crate::navidrome::Source {
            id: "fixture".into(),
            name: "Fixture".into(),
            username: "fixture".into(),
            url,
            libraries: vec![],
        };
        let client = crate::navidrome::Client::new(&source, "fixture".into(), None).unwrap();
        let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(load_playlists(&client, &titles, false), server)
        })
        .await
        .expect("independent reads must proceed before earlier responses arrive");
        assert_eq!(
            result
                .unwrap()
                .into_iter()
                .map(|p| p.id)
                .collect::<Vec<_>>(),
            titles
        );
    }

    #[test]
    fn playlist_size_counts_each_file_once_and_leaves_unknown_totals_unknown() {
        let songs: Vec<crate::navidrome::Song> = serde_json::from_value(serde_json::json!([
            {"id":"one","title":"One","size":120},
            {"id":"one","title":"One","size":120},
            {"id":"two","title":"Two","size":340},
            {"id":"three","title":"Three"}
        ]))
        .unwrap();
        assert_eq!(playlist_total(&songs[..3]), Some(460));
        assert_eq!(playlist_total(&songs), None);
        assert_eq!(playlist_total(&[]), Some(0));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn terminal_sidebar_and_column_choose_a_playlist_without_starting_sync() {
        use super::super::state::{CategoryRow, LibraryRoot};
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut state = AppState::new();
        let mut config = Config::default();
        let source = crate::navidrome::Source {
            id: "fixture".into(),
            name: "Music".into(),
            url: "http://127.0.0.1:1".into(),
            username: "fixture".into(),
            libraries: vec![],
        };
        let session = super::super::sources::navidrome::Session {
            client: crate::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
            source,
            extensions: Default::default(),
        };
        let raw: crate::navidrome::Playlist =
            serde_json::from_value(serde_json::json!({"id":"playlist","name":"Road Trip"}))
                .unwrap();
        state.library.playlists = vec![session.playlist(raw)].into();
        state.sources.active = super::super::sources::ActiveSource::Navidrome(Box::new(session));
        let mounted = tempfile::tempdir().unwrap();
        let mut disk = volume("test-player");
        disk.mount = mounted.path().to_owned();
        state
            .player_sync
            .profiles
            .insert(disk.uuid.clone(), Profile::read(&disk).unwrap());
        state.player_sync.volumes = vec![disk];
        state.player_sync.selected = Some("test-player".into());
        state.view = super::super::View::Browse;
        state.category_column_index = state
            .category_rows()
            .iter()
            .position(|row| *row == CategoryRow::Root(LibraryRoot::Mp3Players))
            .unwrap();
        state.category_column_focused = true;
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        let handle = super::super::handlers::key_input::handle_key;
        assert!(matches!(
            handle(key(KeyCode::Enter), &mut state, &config).as_slice(),
            [Action::Navigation(
                super::super::action::NavigationAction::OpenLibraryRoot(LibraryRoot::Mp3Players)
            )]
        ));
        super::super::sidebar::open_root(LibraryRoot::Mp3Players, &mut state);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 24)).unwrap();
        terminal
            .draw(|frame| {
                crate::ui::render(frame, &state);
            })
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            text.contains("Audio Player")
                && text.contains("Review Sync")
                && text.contains("Playlists: 0 selected")
        );
        for _ in 0..2 {
            handle(key(KeyCode::Down), &mut state, &config);
        }
        let actions = handle(key(KeyCode::Enter), &mut state, &config);
        let [Action::PlayerSync(open)] = actions.as_slice() else {
            panic!("expected playlist picker")
        };
        dispatch(open.clone(), &mut state, &mut config, &events()).unwrap();
        handle(key(KeyCode::Down), &mut state, &config);
        let actions = handle(key(KeyCode::Enter), &mut state, &config);
        let [Action::PlayerSync(choose)] = actions.as_slice() else {
            panic!("expected playlist choice")
        };
        assert!(
            dispatch(choose.clone(), &mut state, &mut config, &events())
                .unwrap()
                .is_empty()
        );
        assert_eq!(state.player_sync.players[0].playlists, ["Road Trip"]);
        assert!(state.player_sync.choosing_playlist);
        dispatch(choose.clone(), &mut state, &mut config, &events()).unwrap();
        assert!(state.player_sync.players[0].playlists.is_empty());
        assert!(!state.player_sync.phase.busy());
    }

    #[tokio::test]
    async fn webdav_browser_filters_descendants_and_keeps_selection_across_levels() {
        use crate::library::{FolderEntry, FolderLocation, FolderSource, tree::Tree};
        let mut state = AppState::new();
        let mut config = Config::default();
        let mounted = tempfile::tempdir().unwrap();
        let mut disk = volume("player");
        disk.mount = mounted.path().to_owned();
        state
            .player_sync
            .profiles
            .insert(disk.uuid.clone(), Profile::read(&disk).unwrap());
        state.player_sync.volumes = vec![disk];
        state.player_sync.selected = Some("player".into());
        state.sources.folders.push(FolderSource {
            id: "archive".into(),
            name: "Archive".into(),
            location: FolderLocation::Webdav {
                url: "http://127.0.0.1:1/archive/".into(),
                username: None,
                password_env: None,
            },
        });
        state.sources.active = crate::app::sources::ActiveSource::Folder("archive".into());
        let folder = |path: &str| FolderEntry {
            path: path.into(),
            name: path.rsplit('/').next().unwrap().into(),
            directory: true,
        };
        state.player_sync.browser = Some(WebdavBrowser {
            generation: state.library_generation,
            path: String::new(),
            query: String::new(),
            tree: FolderLoad::Ready(std::sync::Arc::new(Tree {
                directories: BTreeMap::from([
                    (String::new(), vec![folder("Concerts"), folder("Studio")]),
                    ("Concerts".into(), vec![folder("Concerts/Paris")]),
                    ("Concerts/Paris".into(), vec![folder("Concerts/Paris/1970")]),
                    ("Concerts/Paris/1970".into(), vec![]),
                    ("Studio".into(), vec![]),
                ]),
                ..Default::default()
            })),
            listing: FolderLoad::Idle,
            list_serial: 0,
        });
        let run = |command, state: &mut AppState, config: &mut Config| {
            dispatch(command, state, config, &events()).unwrap()
        };
        assert_eq!(webdav_browser_rows(&state), ["Concerts", "Studio"]);
        run(
            Command::FilterWebdav("1970".into()),
            &mut state,
            &mut config,
        );
        assert_eq!(webdav_browser_rows(&state), ["Concerts/Paris/1970"]);
        run(
            Command::BrowseWebdav("Concerts/Paris/1970".into()),
            &mut state,
            &mut config,
        );
        assert!(webdav_browser_rows(&state).is_empty());
        run(
            webdav_toggle(&state, "Concerts/Paris/1970".into()).unwrap(),
            &mut state,
            &mut config,
        );
        run(Command::UpWebdav, &mut state, &mut config);
        let view = webdav_view(&state).unwrap();
        assert_eq!(view.path, "Concerts/Paris");
        assert!(view.query.is_empty());
        assert_eq!(view.folders, [("Concerts/Paris/1970".into(), true)]);
        assert_eq!(view.selected_count, 1);
        assert!(view.mixed.contains("Concerts/Paris"));
        state.player_sync.synced.insert(
            "player".into(),
            BTreeMap::from([(
                "archive".into(),
                BTreeSet::from(["Concerts/Paris/1970".into()]),
            )]),
        );
        run(Command::ShowSyncedFolders(true), &mut state, &mut config);
        assert_eq!(
            webdav_view(&state).unwrap().folders,
            [("Concerts/Paris/1970".into(), true)]
        );
        run(
            webdav_toggle(&state, "Concerts/Paris/1970".into()).unwrap(),
            &mut state,
            &mut config,
        );
        assert_eq!(
            webdav_view(&state).unwrap().folders,
            [("Concerts/Paris/1970".into(), false)]
        );
    }

    #[tokio::test]
    async fn webdav_settings_are_separate_from_navidrome_and_from_other_webdav_libraries() {
        use crate::library::{
            FolderLocation, FolderSource,
            folder::{FolderColumn, FolderItem, FolderNavigationState},
        };
        let mut state = AppState::new();
        let mut config = Config::default();
        let mounted = tempfile::tempdir().unwrap();
        let mut disk = volume("player");
        disk.mount = mounted.path().to_owned();
        state
            .player_sync
            .profiles
            .insert(disk.uuid.clone(), Profile::read(&disk).unwrap());
        state.player_sync.volumes = vec![disk];
        state.player_sync.selected = Some("player".into());
        state.player_sync.players.push(Player {
            volume_uuid: "player".into(),
            name: "Player".into(),
            source_id: "nav".into(),
            playlists: vec!["Road Trip".into()],
            auto_sync: true,
        });
        state.sources.folders = ["first", "second"]
            .into_iter()
            .map(|id| FolderSource {
                id: id.into(),
                name: id.into(),
                location: FolderLocation::Webdav {
                    url: format!("http://127.0.0.1:1/{id}/"),
                    username: None,
                    password_env: None,
                },
            })
            .collect();
        state.sources.active = crate::app::sources::ActiveSource::Folder("first".into());
        let mut nav = FolderNavigationState::for_library("folder:first".into());
        nav.columns.push(FolderColumn::new(
            None,
            "first".into(),
            vec![FolderItem::folder("Concerts".into(), "Concerts".into())],
        ));
        state.folder_state = Some(nav);
        dispatch(
            Command::ChooseWebdavFolder {
                volume_uuid: "player".into(),
                source_id: "first".into(),
                path: "Concerts".into(),
            },
            &mut state,
            &mut config,
            &events(),
        )
        .unwrap();
        dispatch(
            Command::SetWebdavNomedia {
                volume_uuid: "player".into(),
                source_id: "first".into(),
                enabled: false,
            },
            &mut state,
            &mut config,
            &events(),
        )
        .unwrap();
        assert!(!webdav_view(&state).unwrap().nomedia);
        assert_eq!(state.player_sync.players[0].playlists, ["Road Trip"]);
        state.sources.active = crate::app::sources::ActiveSource::Folder("second".into());
        let second = webdav_view(&state).unwrap();
        assert!(second.nomedia);
        assert!(second.folders.is_empty());
        assert_eq!(
            state.player_sync.webdav["player"]["first"].folders,
            BTreeSet::from(["Concerts".into()])
        );
    }

    #[test]
    fn device_history_survives_disconnect_and_clears_independently() {
        let mut state = AppState::new();
        let mut config = Config::default();
        state.player_sync.volumes = vec![volume("portable")];
        state.player_sync.selected = Some("portable".into());
        for result in [
            Err(Failure::Cancelled),
            Ok(Summary::default()),
            Ok(Summary::default()),
        ] {
            state.player_sync.phase = Phase::Preparing;
            let cancelled = result.is_err();
            dispatch(
                Command::Finished {
                    operation: 0,
                    result,
                },
                &mut state,
                &mut config,
                &events(),
            )
            .unwrap();
            assert_eq!(config.player_sync_history.len(), usize::from(!cancelled));
        }
        state.player_sync.volumes.clear();
        state.player_sync.selected = None;
        assert_eq!(
            crate::config::load_config().unwrap().player_sync_history,
            config.player_sync_history
        );
        let history = project(&state).history;
        assert_eq!(history.len(), 1);
        assert!(history[0].contains("portable"));
        assert!(
            column_rows(&state)
                .iter()
                .any(|(row, _)| row == &history[0])
        );
        dispatch(
            Command::ClearSyncHistory,
            &mut state,
            &mut config,
            &events(),
        )
        .unwrap();
        assert!(project(&state).history.is_empty());
        assert!(
            crate::config::load_config()
                .unwrap()
                .player_sync_history
                .is_empty()
        );
    }
    #[test]
    fn stale_results_cannot_replace_a_newer_transfer_or_reopen_a_finished_job() {
        let mut state = AppState::new();
        let mut config = Config::default();
        state.player_sync.operation = 2;
        state.player_sync.phase = Phase::Preparing;
        dispatch(
            Command::Finished {
                operation: 1,
                result: Err(Failure::Cancelled),
            },
            &mut state,
            &mut config,
            &events(),
        )
        .unwrap();
        dispatch(
            Command::Progress {
                operation: 1,
                progress: Progress {
                    title: "Old track".into(),
                    ..Default::default()
                },
            },
            &mut state,
            &mut config,
            &events(),
        )
        .unwrap();
        assert!(matches!(state.player_sync.phase, Phase::Preparing));
        dispatch(
            Command::Finished {
                operation: 2,
                result: Ok(Summary {
                    copied: 2,
                    ..Default::default()
                }),
            },
            &mut state,
            &mut config,
            &events(),
        )
        .unwrap();
        dispatch(
            Command::Progress {
                operation: 2,
                progress: Progress::default(),
            },
            &mut state,
            &mut config,
            &events(),
        )
        .unwrap();
        assert!(matches!(
            state.player_sync.phase,
            Phase::Complete(Summary { copied: 2, .. })
        ));
    }
    #[test]
    fn disconnect_clears_a_previous_discovery_error() {
        let mut state = AppState::new();
        let mut config = Config::default();
        state.player_sync.volumes = vec![volume("player")];
        state.player_sync.selected = Some("player".into());
        state.player_sync.phase = Phase::Discovering {
            quiet: true,
            previous: Box::new(Phase::DiscoveryFailed("MTP discovery timed out".into())),
        };
        let actions = dispatch(
            Command::Discovered {
                mtp_players: vec![],
                operation: 0,
                result: Ok(vec![]),
            },
            &mut state,
            &mut config,
            &events(),
        )
        .unwrap();
        assert!(actions.is_empty());
        assert!(state.player_sync.volumes.is_empty());
        assert_eq!(state.player_sync.phase, Phase::Idle);
    }

    #[test]
    fn multiple_volumes_require_an_explicit_choice_before_syncing() {
        let mut state = AppState::new();
        let mut config = Config::default();
        state.player_sync.volumes = vec![volume("known")];
        state.player_sync.selected = Some("known".into());
        state.player_sync.players = vec![Player {
            volume_uuid: "new".into(),
            name: "new".into(),
            source_id: "fixture".into(),
            playlists: vec!["Road Trip".into()],
            auto_sync: true,
        }];
        let tmp = tempfile::tempdir().unwrap();
        let mut disk = volume("new");
        disk.mount = tmp.path().into();
        let mut profile = Profile::read(&disk).unwrap();
        profile.update(&state.player_sync, &BTreeMap::new());
        let scan = |state: &mut AppState, config: &mut Config, volumes: Vec<Volume>| {
            state.player_sync.phase = Phase::Discovering {
                quiet: true,
                previous: Box::new(Phase::Idle),
            };
            dispatch(
                Command::Discovered {
                    mtp_players: vec![],
                    operation: 0,
                    result: Ok(volumes
                        .into_iter()
                        .map(|v| {
                            let p = if v.uuid == "new" {
                                profile.clone()
                            } else {
                                Profile::default()
                            };
                            (v, p, BTreeMap::new())
                        })
                        .collect()),
                },
                state,
                config,
                &events(),
            )
            .unwrap()
        };
        assert!(
            scan(
                &mut state,
                &mut config,
                vec![volume("known"), volume("new")]
            )
            .is_empty()
        );
        assert!(state.player_sync.selected.is_none());
        assert!(configured(&state.player_sync).is_none());
        assert!(disabled(&state, Menu::Sync).is_some());
        dispatch(
            Command::Select("new".into()),
            &mut state,
            &mut config,
            &events(),
        )
        .unwrap();
        assert!(
            scan(
                &mut state,
                &mut config,
                vec![volume("new"), volume("known")]
            )
            .is_empty()
        );
        assert_eq!(state.player_sync.selected.as_deref(), Some("new"));
        assert!(disabled(&state, Menu::Sync).is_none());
        scan(&mut state, &mut config, vec![]);
        assert!(matches!(
            scan(&mut state, &mut config, vec![volume("new")]).as_slice(),
            [Action::PlayerSync(Command::ResumeNavidrome)]
        ));
    }
    #[test]
    fn background_discovery_finds_portable_players_and_throttles_repeated_scans() {
        let mut state = AppState::new();
        let config = Config::default();
        assert!(!poll(&mut state, &config).is_empty());
        super::super::sidebar::open_root(super::super::state::LibraryRoot::Mp3Players, &mut state);
        assert!(!poll(&mut state, &config).is_empty());
        super::super::sidebar::open_root(super::super::state::LibraryRoot::Artists, &mut state);
        state.player_sync.last_scan = Some(Instant::now());
        assert!(poll(&mut state, &config).is_empty());
    }
    #[test]
    fn newly_created_playlist_refreshes_only_its_active_account() {
        let mut state = AppState::new();
        let mut config = Config::default();
        let source = crate::navidrome::Source {
            id: "fixture".into(),
            name: "Fixture".into(),
            url: "http://127.0.0.1:1".into(),
            username: "fixture".into(),
            libraries: vec![],
        };
        state.sources.active = super::super::sources::ActiveSource::Navidrome(Box::new(
            super::super::sources::navidrome::Session {
                client: crate::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
                source,
                extensions: Default::default(),
            },
        ));
        state.player_sync.selected = Some("player".into());
        state.player_sync.players = vec![Player {
            volume_uuid: "player".into(),
            name: "Y2".into(),
            source_id: "fixture".into(),
            playlists: vec!["Sync: Y2".into()],
            auto_sync: false,
        }];
        for (account, listed, refresh) in [
            ("fixture", false, true),
            ("fixture", true, false),
            ("other", false, false),
        ] {
            state.player_sync.players[0].source_id = account.into();
            state.library.playlists = if listed {
                vec![crate::library::catalog::Playlist {
                    rating_key: "playlist".into(),
                    key: "playlist".into(),
                    title: "Sync: Y2".into(),
                    playlist_type: "audio".into(),
                    composite: None,
                    duration: None,
                    leaf_count: Some(0),
                    added_at: None,
                    updated_at: None,
                    smart: false,
                }]
                .into()
            } else {
                vec![].into()
            };
            state.player_sync.phase = Phase::Preparing;
            let actions = dispatch(
                Command::Finished {
                    operation: 0,
                    result: Ok(Summary::default()),
                },
                &mut state,
                &mut config,
                &events(),
            )
            .unwrap();
            assert_eq!(
                actions.iter().any(|action| matches!(
                    action,
                    Action::Data(super::super::action::DataAction::LoadPlaylists)
                )),
                refresh
            );
        }
    }
    #[test]
    fn stale_setup_selection_is_rejected_before_changing_config() {
        let mut state = AppState::new();
        let mut config = Config::default();
        state.player_sync.volumes = vec![volume("new")];
        state.player_sync.selected = Some("new".into());
        dispatch(
            Command::Link {
                playlist: String::new(),
                name: "Y2".into(),
                volume_uuid: "old".into(),
                source_id: "fixture".into(),
            },
            &mut state,
            &mut config,
            &events(),
        )
        .unwrap();
        assert!(config.players.is_empty());
        assert!(state.player_sync.players.is_empty());
        assert!(
            matches!(&state.player_sync.phase,Phase::Failed(error) if error.contains("selected player changed"))
        );
    }
}
