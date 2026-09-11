//! Source selection, folder effects, and the shared library-manager model.
//! Transport providers know nothing about application state or server.
use super::action::{FolderAction, PlaybackAction};
use super::handlers::{dispatch_playback, dispatch_settings, events, helpers};
use super::state::{BrowseCategory, InputDialog, InputDialogAction, PlayStatus, View};
use super::tasks::TaskLease;
use super::{Action, AppState, Event};
use crate::audio::AudioPlayer;
use crate::config::Config;
use crate::library::models::{FolderColumn, FolderItem, FolderNavigationState};
use crate::library::{track::Track, FolderEntry, FolderLocation, FolderSource, MediaFile};

use tokio::sync::mpsc;

pub mod audiomuse;
pub mod cache;
pub mod dialogs;
mod files;
pub mod manager;
pub mod navidrome;
pub mod options;
pub mod radio;
mod routing;
pub mod sonic;
pub use files::{folders, play, FileAction};

pub fn refresh_due_caches(state: &mut AppState, tx: &mpsc::Sender<Event>) {
    if cache::maintenance_paused(state) {
        return;
    }
    if state.sources.active.navidrome().is_some() {
        navidrome::check_staleness(state, tx);
        if !state.library_loading {
            audiomuse::check_staleness(state, tx);
        }
    } else if state.sources.active.folder().is_some()
        && state.sources.listing.is_none()
        && !state
            .cache_mgmt
            .failures
            .contains_key(&crate::app::state::RefreshCategory::Folders)
    {
        if let Some(source) = state
            .sources
            .folders
            .iter()
            .find(|s| Some(&s.id) == state.sources.active.folder())
            .cloned()
        {
            cache::start(LibraryChoice::Folder(source), false, state, tx);
        }
        // The directory tree and the visible album listing share the weekly
        // policy. Album file listings are never eagerly scanned into the tree.
        if let Some(nav) = &state.folder_state {
            if let Some(column) = nav.columns.len().checked_sub(1) {
                if nav.columns[column].is_shuffled() {
                    return;
                }
                let path = nav.columns[column].key.clone().unwrap_or_default();
                files::load(state, tx, path, column);
            }
        }
    }
}

/// Single backend boundary for library requests. Shared radio resolves its own
/// provider request; reducers and account management are independent of source.
pub fn route(
    action: &Action,
    state: &mut AppState,

    audio: &mut AudioPlayer,
    tx: &mpsc::Sender<Event>,
) -> Option<Vec<Action>> {
    if matches!(action, Action::Radio(_)) {
        return None;
    }
    match state.sources.active {
        ActiveSource::None => routing::navidrome_fallback(action),
        ActiveSource::Navidrome(_) => navidrome::intercept(action, state, tx),
        ActiveSource::Folder(_) => {
            if let Action::Folders(action) = action {
                folders(action.clone(), state, audio, tx);
                Some(vec![])
            } else {
                reject_unsupported_action(action, state).then(Vec::new)
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SourceState {
    pub nav_connection_task: Option<TaskLease>,
    pub audiomuse: audiomuse::State,
    pub sonic_tasks: std::collections::HashMap<&'static str, TaskLease>,
    pub sonic_disabled_libraries: std::collections::HashSet<String>,
    pub folders: Vec<FolderSource>,
    pub active: ActiveSource,
    pub navidrome: Vec<crate::navidrome::Source>,
    pub nav_tasks: std::collections::HashMap<&'static str, (u64, TaskLease)>,
    pub nav_request_id: u64,
    pub nav_collection: Option<navidrome::commands::CollectionKind>,
    pub list_id: u64,
    pub listing: Option<TaskLease>,
    pub preparing: Option<TaskLease>,
    pub prepared: Option<MediaFile>,
    pub picker_scroll_pin: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub enum ActiveSource {
    #[default]
    None,
    Folder(String),
    Navidrome(Box<navidrome::Session>),
}
impl ActiveSource {
    pub fn capabilities(&self) -> crate::library::capabilities::Capabilities {
        use crate::library::capabilities;
        match self {
            Self::None => capabilities::Capabilities(&[]),
            Self::Navidrome(_) => capabilities::NAVIDROME,
            Self::Folder(_) => capabilities::FOLDERS,
        }
    }
    pub fn folder(&self) -> Option<&String> {
        match self {
            Self::Folder(id) => Some(id),
            _ => None,
        }
    }
    pub fn navidrome(&self) -> Option<&navidrome::Session> {
        match self {
            Self::Navidrome(session) => Some(session),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum LibraryChoice {
    AddWebdav,
    Navidrome {
        source: crate::navidrome::Source,
        folder: Option<String>,
        name: String,
    },
    AddNavidrome,
    Add,
    Folder(FolderSource),

    AddFolder,
}
impl LibraryChoice {
    pub fn label(&self) -> String {
        match self {
            Self::AddWebdav => "WebDAV share".into(),
            Self::Navidrome { source, name, .. } if name != &source.name => {
                format!("{name} / {}", source.name)
            }
            Self::Navidrome { name, .. } => name.clone(),
            Self::AddNavidrome => "Navidrome library…".into(),
            Self::Add => "[ Add library… ]".into(),
            Self::Folder(s) => s.name.clone(),

            Self::AddFolder => "Local folder".into(),
        }
    }
    pub fn active(&self, state: &AppState) -> bool {
        match self {
            Self::Navidrome { source, folder, .. } => {
                state.sources.active.navidrome().is_some_and(|s| {
                    s.source.id == source.id
                        && source.canonical_folder(s.client.folder.clone())
                            == source.canonical_folder(folder.clone())
                })
            }
            Self::Folder(s) => state.sources.active.folder().map(String::as_str) == Some(&s.id),

            _ => false,
        }
    }
    pub fn action(&self) -> Action {
        Action::Source(SourceAction::Choose(self.clone()))
    }
}

pub fn picker_offset(state: &AppState, height: usize) -> usize {
    let max = choices(state).len().saturating_sub(height);
    state
        .sources
        .picker_scroll_pin
        .unwrap_or_else(|| {
            state
                .popups
                .library_picker_index
                .saturating_sub(height.saturating_sub(1))
        })
        .min(max)
}

pub fn choices(state: &AppState) -> Vec<LibraryChoice> {
    let mut entries = library_choices(state);
    if !state.popups.library_picker_active {
        entries.push(LibraryChoice::Add);
    }
    entries
}

pub fn library_choices(state: &AppState) -> Vec<LibraryChoice> {
    let mut result: Vec<_> = state
        .sources
        .folders
        .iter()
        .cloned()
        .map(LibraryChoice::Folder)
        .collect();
    for source in &state.sources.navidrome {
        if source.libraries.len() != 1 {
            result.push(LibraryChoice::Navidrome {
                source: source.clone(),
                folder: None,
                name: "All music".into(),
            });
        }
        result.extend(
            source
                .libraries
                .iter()
                .map(|folder| LibraryChoice::Navidrome {
                    source: source.clone(),
                    folder: Some(folder.id.clone()),
                    name: folder.name.clone(),
                }),
        );
    }
    result.sort_by_key(|choice| (choice.group(), choice.label().to_lowercase()));
    result
}

async fn reset_source(
    state: &mut AppState,
    audio: &mut AudioPlayer,
    tx: &mpsc::Sender<Event>,
) -> anyhow::Result<()> {
    dispatch_playback::dispatch(tx, PlaybackAction::Stop, state, audio).await?;
    dispatch_playback::dispatch(tx, PlaybackAction::ResetForAccountChange, state, audio).await?;
    state.advance_connection_generation();
    state.advance_library_generation();
    events::reset_account_scoped_state(state);
    state.sources.nav_tasks.clear();
    state.sources.nav_collection = None;
    state.sources.listing = None;
    state.sources.preparing = None;
    state.sources.prepared = None;
    state.sources.active = ActiveSource::None;
    Ok(())
}

pub fn cache_store(state: &AppState) -> Option<anyhow::Result<crate::library::cache::Store>> {
    if let Some(session) = state.sources.active.navidrome() {
        return Some(session.cache_store());
    }
    state
        .sources
        .folders
        .iter()
        .find(|source| Some(&source.id) == state.sources.active.folder())
        .map(crate::library::cache::Store::folder)
}

#[derive(Debug, Clone)]
pub enum SourceAction {
    AudioMuse(Box<audiomuse::Command>),

    CheckWebdav(dialogs::WebdavDraft),
    WebdavChecked {
        instance: String,
        result: Result<dialogs::WebdavDraft, String>,
    },
    ToggleSonic(LibraryChoice),
    Navidrome(Box<navidrome::NavAction>),
    Choose(LibraryChoice),
    AddLocation(String),
    Rename {
        id: String,
        name: String,
    },
    Remove(String),

    Files(FileAction),
}

pub async fn dispatch(
    action: SourceAction,
    state: &mut AppState,

    audio: &mut AudioPlayer,
    config: &mut Config,
    tx: &mpsc::Sender<Event>,
) -> anyhow::Result<Vec<Action>> {
    if let SourceAction::Choose(choice) = &action {
        if choice.active(state) {
            if manager::settings_active(state) {
                state.set_view(View::Browse);
            }
            state.popups.library_picker_active = false;
            return Ok(vec![]);
        }
    }
    match action {
        SourceAction::AudioMuse(command) => {
            return audiomuse::dispatch(*command, state, config, tx).await
        }
        action @ (SourceAction::CheckWebdav(_) | SourceAction::WebdavChecked { .. }) => {
            return dialogs::dispatch(action, state, audio, config, tx).await;
        }
        SourceAction::Choose(LibraryChoice::Add) => {
            state.popups.library_dialog = Some(dialogs::Dialog::Add { selected: 0 });
        }
        SourceAction::Choose(LibraryChoice::AddWebdav) => {
            state.popups.library_dialog =
                Some(dialogs::Dialog::Webdav(dialogs::WebdavForm::new(None)));
        }
        SourceAction::ToggleSonic(choice) => {
            if let Some(key) = sonic::choice_key(&choice, state) {
                if !state.sources.sonic_disabled_libraries.remove(&key) {
                    state.sources.sonic_disabled_libraries.insert(key);
                }
                if sonic::choice_key(&choice, state)
                    .is_some_and(|key| state.sources.sonic_disabled_libraries.contains(&key))
                {
                    cache::cancel(&choice, state);
                }
                config.sonic_disabled_libraries = state.sources.sonic_disabled_libraries.clone();
                dispatch_settings::save_config_in_background(tx, config, "save sonic preference");
                if choice.active(state) {
                    if !sonic::enabled(state) {
                        sonic::cancel_pending(state);
                    } else {
                        return Ok(vec![super::action::BrowseAction::LoadStations.into()]);
                    }
                }
            }
        }

        SourceAction::Navidrome(action) => {
            return navidrome::dispatch(*action, state, audio, config, tx).await
        }
        SourceAction::Choose(
            choice @ (LibraryChoice::Navidrome { .. } | LibraryChoice::AddNavidrome),
        ) => {
            cache::resume(&choice, state);
            return navidrome::choose(choice, state, audio, config, tx).await;
        }
        SourceAction::Files(action) => return files::dispatch(action, state, audio).await,
        SourceAction::Choose(LibraryChoice::Folder(source)) => {
            cache::resume(&LibraryChoice::Folder(source.clone()), state);
            config.default_navidrome = None;
            if state.sources.active.folder() == Some(&source.id) {
                state.popups.library_picker_active = false;
                return Ok(vec![]);
            }
            reset_source(state, audio, tx).await?;
            state.sources.active = crate::app::sources::ActiveSource::Folder(source.id.clone());
            state.active_library = Some(format!("folder:{}", source.id));
            radio::load_folder_stations(state);
            state.view = View::Browse;
            state.browse_category = BrowseCategory::Folders;
            state.category_column_index = state.row_index_for_category(BrowseCategory::Folders);
            state.category_column_focused = false;
            state.popups.library_picker_active = false;
            config.default_folder_source = Some(source.id);
            dispatch_settings::save_config_in_background(tx, config, "save library selection");
            files::load(state, tx, String::new(), 0);
        }

        SourceAction::Choose(LibraryChoice::AddFolder) => {
            state.popups.input_dialog = Some(InputDialog {
                title: "Local folder path".into(),
                input: Default::default(),
                action_type: InputDialogAction::FolderLocation,
            });
        }
        SourceAction::AddLocation(location) => match source_from_location(&location) {
            Ok(source) => {
                if matches!(source.location, FolderLocation::Webdav { .. }) {
                    state.popups.library_dialog = Some(dialogs::Dialog::Webdav(
                        dialogs::WebdavForm::from_location(source),
                    ));
                    return Ok(vec![]);
                }
                let candidate = source.clone();
                let saved = config.folder_sources.clone();
                let duplicate = super::tasks::spawn_blocking(move || {
                    saved.iter().any(|s| s.same_library(&candidate))
                })
                .await?;
                if duplicate {
                    state.set_error("That folder library is already added".into());
                } else {
                    config.folder_sources.push(source.clone());
                    state.sources.folders = config.folder_sources.clone();
                    manager::reset_selection(state);
                    state.popups.library_picker_index = choices(state).iter().position(|choice| matches!(choice, LibraryChoice::Folder(s) if s.id == source.id)).unwrap_or(0);
                    dispatch_settings::save_config_in_background(tx, config, "add folder library");
                    state.popups.input_dialog = Some(InputDialog {
                        title: "Library name".into(),
                        input: source.name.into(),
                        action_type: InputDialogAction::FolderName(source.id),
                    });
                }
            }
            Err(error) => state.set_error(error.to_string()),
        },
        SourceAction::Rename { id, name } => {
            if name.trim().is_empty() {
                state.set_error("Library name is required".into());
            } else if let Some(source) = config.folder_sources.iter_mut().find(|s| s.id == id) {
                source.name = name.trim().into();
                state.sources.folders = config.folder_sources.clone();
                state.popups.library_picker_index = choices(state)
                    .iter()
                    .position(|choice| matches!(choice, LibraryChoice::Folder(s) if s.id == id))
                    .unwrap_or(0);
                dispatch_settings::save_config_in_background(tx, config, "rename library");
            }
        }
        SourceAction::Remove(id) => {
            for choice in library_choices(state)
                .into_iter()
                .filter(|c| matches!(c, LibraryChoice::Folder(s) if s.id == id))
            {
                cache::cancel(&choice, state);
            }
            let managing = manager::settings_active(state);
            if !config.folder_sources.iter().any(|s| s.id == id) {
                return Ok(vec![]);
            }
            if state.sources.active.folder() == Some(&id) {
                reset_source(state, audio, tx).await?;
                state.view = if managing {
                    View::Settings
                } else {
                    View::Browse
                };
            }
            config.folder_sources.retain(|s| s.id != id);
            state.sources.folders = config.folder_sources.clone();
            if config.default_folder_source.as_ref() == Some(&id) {
                config.default_folder_source = None;
            }
            dispatch_settings::save_config_in_background(tx, config, "remove library");
            state.popups.library_picker_active = !managing;
            state.popups.library_picker_index = 0;
            let tx = tx.clone();
            super::tasks::spawn_blocking(move || {
                if let Err(error) = crate::library::credentials::save(&id, Default::default()) {
                    let _ = tx.blocking_send(Event::Effect(
                        super::action::SystemAction::ShowError(format!(
                            "Remove credentials: {error}"
                        ))
                        .into(),
                    ));
                }
            });
        }
    }
    Ok(vec![])
}

pub fn source_from_location(location: &str) -> anyhow::Result<FolderSource> {
    use anyhow::{bail, Context};
    let location = location.trim();
    let (name, location) = if location.starts_with('/') || location.starts_with("~/") {
        let path = if let Some(rest) = location.strip_prefix("~/") {
            dirs::home_dir()
                .context("Home directory unavailable")?
                .join(rest)
        } else {
            location.into()
        };
        (
            path.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Music")
                .to_string(),
            FolderLocation::Local { path },
        )
    } else {
        let mut url = reqwest::Url::parse(location)
            .context("Enter an absolute folder path or a WebDAV URL")?;
        if url.password().is_some() {
            bail!("Use the separate username and password fields, not passwords in URLs");
        }
        let username = (!url.username().is_empty())
            .then(|| urlencoding::decode(url.username()).map(|s| s.into_owned()))
            .transpose()?;
        let path = urlencoding::decode(url.path())?.into_owned();
        let name = path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or(url.host_str().unwrap_or("Music"))
            .to_owned();
        let location = match url.scheme() {
            "https" | "http" => {
                let _ = url.set_username("");
                FolderLocation::Webdav {
                    url: url.to_string(),
                    username,
                    password_env: None,
                }
            }
            _ => bail!("Supported sources: local folders and https:// or http:// WebDAV"),
        };
        (name, location)
    };
    let source = FolderSource {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        location,
    };
    source.validate()?;
    Ok(source)
}

/// Feature requirements are independent of transport. The per-library sonic
/// preference is applied after backend capability checks.
pub fn command_visible(state: &AppState, command: &super::state::PaletteCommandKind) -> bool {
    use super::state::PaletteCommandKind::*;
    use crate::library::capabilities::Feature;
    let requirement = match command {
        GotoLibrary | OpenInLibrary | PlayFocusedAlbum => Some(Feature::Catalog),
        GotoGenres => Some(Feature::Genres),
        OpenRelated => Some(Feature::RelatedArtists),
        SaveQueue => Some(Feature::SavePlaylist),
        ArtistRadio | RemixTwofer => Some(Feature::ArtistRadio),
        RandomAlbum => Some(Feature::AlbumRadio),
        ToggleDj(mode) => Some(sonic::dj_feature(*mode)),
        OpenSimilar | RemixGemini | RemixDoppelganger => Some(Feature::SonicSimilarity),
        SonicAdventure | SonicAdventureFromFocusedTrack | RemixStretch => Some(Feature::SonicPaths),
        _ => None,
    };
    requirement.is_none_or(|f| state.sources.active.capabilities().supports(f))
        && sonic::command_visible(state, command)
}

/// Server-only commands are rejected at the orchestration boundary as well as
/// disabled in the UI. Folder identities must never reach a server endpoint.
pub fn reject_unsupported_action(action: &Action, state: &mut AppState) -> bool {
    use super::action::{NavigationAction, QueueAction, SearchAction, SystemAction};
    let unsupported = match action {
        Action::Browse(super::action::BrowseAction::LoadStations) => {
            radio::load_folder_stations(state);
            return true;
        }
        Action::Data(super::action::DataAction::LoadTrackPaneSimilar { .. }) => return true,
        Action::Data(_) | Action::Miller(_) => true,
        Action::Browse(_) => true,
        Action::Navigation(NavigationAction::SetCategory { category, .. }) => {
            *category != BrowseCategory::Folders
        }
        Action::Search(
            SearchAction::OpenAdventureLauncher
            | SearchAction::OpenAdventureLauncherWithStart { .. }
            | SearchAction::OpenArtistRadioPicker,
        ) => true,
        Action::Queue(QueueAction::PromptSavePlaylist) => true,
        Action::Queue(_) => routing::requires_provider(action),

        Action::System(
            SystemAction::CheckStaleness(_)
            | SystemAction::LoadArtwork
            | SystemAction::LoadAlbumArt(_),
        ) => return true,
        _ => false,
    };
    if unsupported {
        state.set_status("Not available for folder libraries".into());
    }
    unsupported
}
