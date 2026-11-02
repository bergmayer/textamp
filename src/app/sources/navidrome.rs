//! Navidrome source lifecycle. Catalog data has one owner: AppState.library.
use super::*;
use crate::app::action::QueueAction;
use crate::navidrome::{Client, Source};
use crate::util::SecretString;

pub(super) mod catalog;
pub use catalog::{check_staleness, load as reload, open, Catalog};
pub mod commands;
pub(crate) mod dj;
pub(super) mod effects;
mod mapping;
pub(crate) mod radio;
pub(crate) mod recommendations;
pub use effects::{intercept, play, scrobble};

#[derive(Debug, Clone)]
pub struct Session {
    pub source: Source,
    pub client: Client,
    pub extensions: std::collections::HashSet<String>,
}
impl Session {
    pub fn cache_store(&self) -> anyhow::Result<crate::library::cache::Store> {
        super::manager::navidrome_store(&self.source, self.client.folder.clone())
    }
    pub fn key(&self, id: &str) -> String {
        format!("navidrome:{}:{}", self.source.id, urlencoding::encode(id))
    }
    pub fn id(&self, key: &str) -> anyhow::Result<String> {
        let prefix = format!("navidrome:{}:", self.source.id);
        let id = key
            .strip_prefix(&prefix)
            .ok_or_else(|| anyhow::anyhow!("Item belongs to a different account"))?;
        Ok(urlencoding::decode(id)?.into_owned())
    }

    pub async fn album_tracks(&self, key: &str) -> anyhow::Result<Vec<Track>> {
        let mut album = self.client.album(&self.id(key)?).await?;
        album.song.sort_by_key(|song| {
            (
                song.disc_number.unwrap_or(1),
                song.track.unwrap_or(0),
                song.title.clone(),
            )
        });
        Ok(album
            .song
            .into_iter()
            .map(|song| self.track(song))
            .collect())
    }

    pub async fn artist_tracks(&self, key: &str) -> anyhow::Result<Vec<Track>> {
        use futures::{stream, StreamExt, TryStreamExt};
        let artist = self.client.artist(&self.id(key)?).await?;
        let albums: Vec<Vec<Track>> = stream::iter(artist.album)
            .map(|album| async move { self.album_tracks(&self.key(&album.id)).await })
            .buffered(4)
            .try_collect()
            .await?;
        Ok(albums.into_iter().flatten().collect())
    }
}

#[derive(Debug, Clone)]
pub enum NavAction {
    QueueEdit {
        expected: Vec<String>,
        index: Option<usize>,
        action: Box<Action>,
    },
    DjReady {
        playback_id: u64,
        track: String,
        mode: crate::app::state::DjMode,
        result: Result<Vec<Track>, crate::app::action::AsyncError>,
    },
    Command(commands::Command),
    List {
        request_id: u64,
        kind: commands::CollectionKind,
        items: commands::Collection,
    },
    Text {
        request_id: u64,
        result: Result<String, String>,
    },
    RestoreQueue {
        request_id: u64,
        tracks: Vec<Track>,
        index: usize,
        position: u64,
    },
    Password {
        source: Source,
        password: SecretString,
        analysis: Option<super::dialogs::AnalysisSetup>,
    },
    Connected {
        generation: u64,
        result: Result<Session, String>,
        password: Option<SecretString>,
        analysis: Option<super::dialogs::AnalysisSetup>,
    },
    Rename {
        id: String,
        name: String,
    },
    Remove(String),
    Catalog {
        generation: u64,
        request_id: u64,
        refreshing: bool,
        timestamp: u64,
        result: Result<catalog::PreparedCatalog, String>,
    },
    Completed {
        generation: u64,
        request_id: u64,
        slot: &'static str,
        result: Result<Vec<Action>, String>,
    },
    Event(Box<Event>),
}
impl From<NavAction> for Action {
    fn from(action: NavAction) -> Self {
        SourceAction::Navidrome(Box::new(action)).into()
    }
}
impl From<SourceAction> for Action {
    fn from(action: SourceAction) -> Self {
        Self::Source(action)
    }
}

pub async fn choose(
    choice: LibraryChoice,
    state: &mut AppState,

    audio: &mut AudioPlayer,
    config: &mut Config,
    tx: &mpsc::Sender<Event>,
) -> anyhow::Result<Vec<Action>> {
    match choice {
        LibraryChoice::AddNavidrome => {
            state
                .popups
                .set_library_dialog(Some(super::dialogs::Dialog::Navidrome(
                    super::dialogs::ServerForm::new(None),
                )))
        }
        LibraryChoice::Navidrome { source, folder, .. } => {
            let reconnect = state.sources.reconnect_sources.remove(&source.id);
            if !reconnect
                && state
                    .sources
                    .active
                    .navidrome()
                    .is_some_and(|s| s.source.id == source.id && s.client.folder == folder)
            {
                state.popups.set_library_picker_active(false);
                return Ok(vec![]);
            }
            super::reset_source(state, audio, tx).await?;
            state.view = View::Browse;
            state.library_loading = true;
            state.popups.set_library_picker_active(false);
            let generation = state.connection_generation;
            let tx = tx.clone();
            let task = crate::app::tasks::spawn(async move {
                let result = async {
                    let id = source.id.clone();
                    let password = crate::app::tasks::spawn_blocking(move || {
                        crate::library::credentials::load_source("navidrome", &id)
                    })
                    .await??
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Password missing; open Libraries and use C to set credentials"
                        )
                    })?;
                    // Saved accounts can browse their cache without waiting for
                    // a server handshake. A new account still verifies sign-in.
                    Ok(Session {
                        client: Client::new(
                            &source,
                            SecretString::from(password.as_str()),
                            folder,
                        )?,
                        source,
                        extensions: Default::default(),
                    })
                }
                .await
                .map_err(|e: anyhow::Error| format!("{e:#}"));
                let _ = tx
                    .send(Event::Effect(
                        NavAction::Connected {
                            generation,
                            result,
                            password: None,
                            analysis: None,
                        }
                        .into(),
                    ))
                    .await;
            });
            state.sources.nav_connection_task = Some(TaskLease::new(&task));
        }
        _ => unreachable!(),
    }
    let _ = config;
    Ok(vec![])
}

async fn connect(
    mut source: Source,
    password: SecretString,
    folder: Option<String>,
) -> anyhow::Result<Session> {
    source.url = crate::util::server_url::resolve(&source.url, "rest/ping").await?;
    let mut session = Session {
        client: Client::new(&source, password, folder)?,
        source,
        extensions: Default::default(),
    };
    discover(&mut session).await?;
    Ok(session)
}

async fn discover(session: &mut Session) -> anyhow::Result<()> {
    let client = &session.client;
    // These account reads are independent. Keep extension discovery optional,
    // while authentication and folder failures still fail the connection.
    let (_, libraries, extensions) =
        tokio::try_join!(client.call("ping", &[]), client.folders(), async {
            Ok::<_, anyhow::Error>(client.call("getOpenSubsonicExtensions", &[]).await)
        },)?;
    session.source.libraries = libraries;
    for library in &mut session.source.libraries {
        library.name = crate::util::sanitize_display_text(&library.name).into_owned();
    }
    if client
        .folder
        .as_ref()
        .is_some_and(|id| !session.source.libraries.iter().any(|f| &f.id == id))
    {
        anyhow::bail!(
            "This account no longer has access to that library; choose another in Settings → Libraries"
        );
    }
    session.extensions = match extensions {
        Ok(response) => response["openSubsonicExtensions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v["name"].as_str().map(str::to_owned))
            .collect(),
        Err(error) => {
            tracing::warn!("Navidrome extension discovery: {error}");
            Default::default()
        }
    };
    session.client.folder = session
        .source
        .canonical_folder(session.client.folder.clone());
    Ok(())
}

pub async fn dispatch(
    action: NavAction,
    state: &mut AppState,

    audio: &mut AudioPlayer,
    config: &mut Config,
    tx: &mpsc::Sender<Event>,
) -> anyhow::Result<Vec<Action>> {
    match action {
        NavAction::Command(command) => {
            commands::dispatch(command, state, tx);
        }
        NavAction::List {
            request_id,
            kind,
            items,
        } => {
            if request_id != state.artist_nav_request_id
                || state.sources.nav_collection != Some(kind)
                || state.browse_category != BrowseCategory::Library
            {
                return Ok(vec![]);
            }
            state.artist_nav.focused_column = 0;
            let (items, tracks) = match items {
                commands::Collection::Albums(albums) => (
                    crate::app::state::BrowseItem::from_albums(
                        &albums,
                        &state.library.album_display_artist,
                    ),
                    vec![],
                ),
                commands::Collection::Artists(artists) => (
                    crate::app::state::BrowseItem::from_artists(&artists),
                    vec![],
                ),
                commands::Collection::Tracks(tracks) => {
                    (crate::app::state::BrowseItem::from_tracks(&tracks), tracks)
                }
            };
            state.artist_nav.columns = vec![crate::app::state::BrowseColumn::new_with_tracks(
                kind.label(),
                items,
                tracks,
            )];
            state.artist_nav.loading = false;
        }
        NavAction::Text { request_id, result } => {
            if let Some(popup) = state
                .popups
                .text_mut()
                .filter(|p| p.request_id == request_id)
            {
                popup.text = result.unwrap_or_else(|e| e);
            }
        }
        NavAction::RestoreQueue {
            request_id,
            tracks,
            index,
            position,
        } => {
            if request_id != state.queue_play_request_id {
                return Ok(vec![]);
            }
            audio.stop();
            state.sources.media = crate::app::sources::MediaPreparation::Idle;
            state.queue.tracks = tracks;
            state.queue.index = (!state.queue.tracks.is_empty())
                .then_some(index.min(state.queue.tracks.len().saturating_sub(1)));
            state.queue.original.clear();
            state.queue_play_request_id = state.queue_play_request_id.wrapping_add(1);
            state.radio.clear();
            state.set_playback_mode(super::super::state::PlaybackMode::Queue);
            state.playback.status = PlayStatus::Stopped;
            state.playback.position_ms = position;
            state.playback.duration_ms = state.current_track().map_or(0, Track::duration_ms);
            state.set_view(View::Queue);
            helpers::play_current_track(tx, state, audio);
            state.set_status("Queue restored".into());
            return Ok(vec![PlaybackAction::Seek(position).into()]);
        }
        NavAction::Event(event) => {
            return Ok(crate::app::dispatch::handle_core_event(*event, state, tx))
        }
        NavAction::QueueEdit {
            expected,
            index,
            mut action,
        } => {
            if state.playback_mode == crate::app::state::PlaybackMode::Queue
                && state
                    .queue
                    .tracks
                    .iter()
                    .map(|t| &t.rating_key)
                    .eq(expected.iter())
            {
                // Playback advancing is not a queue edit. Keep completed work
                // for the remaining queue without changing the new current song.
                if state.queue.index != index {
                    let current = state.queue.index.unwrap_or(0);
                    match action.as_mut() {
                        Action::Queue(QueueAction::RemixBatchReady(outcome)) => {
                            outcome.items.retain(|(position, _)| *position >= current);
                        }
                        Action::Queue(QueueAction::RemixDoppelgangerReady(outcome)) => {
                            outcome.items.retain(|(position, _)| *position > current);
                        }
                        _ => return Ok(vec![]),
                    }
                }
                return Ok(vec![*action]);
            }
            state.set_status("Queue changed; remix discarded".into());
        }
        NavAction::DjReady {
            playback_id,
            track,
            mode,
            result,
        } => {
            if state.dj.active_mode == Some(mode)
                && state.playback.request_id == playback_id
                && state.current_track().is_some_and(|t| t.rating_key == track)
            {
                return Ok(vec![crate::app::action::RadioAction::DjModeTracksReady(
                    result, true,
                )
                .into()]);
            }
        }
        NavAction::Password {
            source,
            password,
            analysis,
        } => {
            if config
                .navidrome_sources
                .iter()
                .any(|saved| saved.id != source.id && saved.same_account(&source))
            {
                state.set_error("That Navidrome account is already added".into());
                return Ok(vec![]);
            }
            state.advance_connection_generation();
            // Credentials are validated before replacing the saved account. Current playback
            // remains usable while this request runs.
            let generation = state.connection_generation;
            let tx = tx.clone();
            let saved_id = config
                .navidrome_sources
                .iter()
                .find(|s| s.id == source.id && s.same_account(&source))
                .map(|s| s.id.clone());
            let task = crate::app::tasks::spawn(async move {
                let result: anyhow::Result<_> = async {
                    let password = if password.is_empty() {
                        if let Some(id) = saved_id {
                            crate::app::tasks::spawn_blocking(move || {
                                crate::library::credentials::load_source("navidrome", &id)
                            })
                            .await??
                            .map(|value| SecretString::from(value.as_str()))
                            .unwrap_or_default()
                        } else {
                            password
                        }
                    } else {
                        password
                    };
                    let music = connect(source, password.clone(), None);
                    let analysis = async {
                        let Some(mut setup) = analysis else {
                            return Ok::<_, anyhow::Error>(None);
                        };
                        setup.connection.url =
                            crate::util::server_url::resolve(&setup.connection.url, "api/servers")
                                .await?;
                        if setup.password.is_empty() {
                            let key = setup
                                .library_key
                                .clone()
                                .ok_or_else(|| anyhow::anyhow!("Enter an AudioMuse password"))?;
                            setup.password = crate::app::tasks::spawn_blocking(move || {
                                crate::library::credentials::load_source("audiomuse", &key)
                            })
                            .await??
                            .ok_or_else(|| anyhow::anyhow!("Enter an AudioMuse password"))?
                            .as_str()
                            .into();
                        }
                        crate::audiomuse::Client::connect(&setup.connection, &setup.password)
                            .await?
                            .verify()
                            .await?;
                        Ok(Some(setup))
                    };
                    let (session, analysis) = tokio::try_join!(music, analysis)?;
                    Ok((session, password, analysis))
                }
                .await;
                let (result, password, analysis) = match result {
                    Ok((session, password, analysis)) => (Ok(session), Some(password), analysis),
                    Err(e) => (Err(format!("{e:#}")), None, None),
                };
                let _ = tx
                    .send(Event::Effect(
                        NavAction::Connected {
                            generation,
                            result,
                            password,
                            analysis,
                        }
                        .into(),
                    ))
                    .await;
            });
            state.sources.nav_connection_task = Some(TaskLease::new(&task));
            if let Some(super::dialogs::Dialog::Navidrome(form)) = state.popups.library_dialog_mut()
            {
                form.busy = true;
                form.error = None;
            }
            state.set_status("Connecting to Navidrome…".into());
        }
        NavAction::Connected {
            generation,
            result,
            password,
            analysis,
        } => {
            if generation != state.connection_generation {
                return Ok(vec![]);
            }
            state.sources.nav_connection_task = None;
            state.library_loading = false;
            let session = match result {
                Ok(s) => s,
                Err(e) => {
                    if let Some(super::dialogs::Dialog::Navidrome(form)) =
                        state.popups.library_dialog_mut()
                    {
                        form.busy = false;
                        form.error = Some(e);
                        return Ok(vec![]);
                    }
                    state.set_error(e);
                    state
                        .popups
                        .set_library_picker_active(!super::manager::settings_active(state));
                    return Ok(vec![]);
                }
            };
            if config
                .navidrome_sources
                .iter()
                .any(|saved| saved.id != session.source.id && saved.same_account(&session.source))
            {
                state.set_error("That Navidrome account is already added".into());
                return Ok(vec![]);
            }
            if password.is_some() || analysis.is_some() {
                let id = session.source.id.clone();
                let analysis = analysis.map(|setup| {
                    let folder = setup
                        .library_key
                        .as_deref()
                        .and_then(|key| serde_json::from_str::<serde_json::Value>(key).ok())
                        .and_then(|value| value[4].as_str().map(str::to_owned));
                    let folder = session.source.canonical_folder(folder);
                    (super::sonic::nav_key(&session.source, &folder), setup)
                });
                // Save both checked connections before changing config. If the
                // second credential write fails, restore the first account's secret.
                let saved = crate::app::tasks::spawn_blocking(move || -> anyhow::Result<_> {
                    let changed_music = password.is_some();
                    let previous = if changed_music {
                        crate::library::credentials::load_source("navidrome", &id)?
                    } else { None };
                    if let Some(password) = password {
                        crate::library::credentials::save_source("navidrome", &id, password)?;
                    }
                    if let Some((key, setup)) = analysis {
                        if let Err(error) = crate::library::credentials::save_source("audiomuse", &key, setup.password) {
                            if changed_music {
                                let old = previous.as_deref().map(|value| SecretString::from(value.as_str())).unwrap_or_default();
                                crate::library::credentials::save_source("navidrome", &id, old)
                                    .map_err(|restore| anyhow::anyhow!("Save AudioMuse credentials: {error}; restore music-server credentials: {restore}"))?;
                            }
                            return Err(error);
                        }
                        return Ok(Some((key, setup.connection)));
                    }
                    Ok(None)
                }).await?;
                match saved {
                    Ok(Some((key, connection))) => {
                        config
                            .audiomuse_connections
                            .insert(key.clone(), connection.clone());
                        state.sources.audiomuse.connections.insert(key, connection);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        if let Some(super::dialogs::Dialog::Navidrome(form)) =
                            state.popups.library_dialog_mut()
                        {
                            form.busy = false;
                            form.error = Some(format!("Save credentials: {error}"));
                        } else {
                            state.set_error(format!("Save credentials: {error}"));
                        }
                        return Ok(vec![]);
                    }
                }
            }
            // Checking a setup form saves the account. Choosing a library is a
            // separate action, so another source and its playback stay active.
            if matches!(
                state.popups.library_dialog(),
                Some(super::dialogs::Dialog::Navidrome(_))
            ) {
                let id = session.source.id.clone();
                config.navidrome_sources.retain(|s| s.id != id);
                let choice = LibraryChoice::Navidrome {
                    source: session.source.clone(),
                    folder: session.client.folder.clone(),
                    name: String::new(),
                };
                config.navidrome_sources.push(session.source);
                state.sources.navidrome = config.navidrome_sources.clone();
                state.sources.reconnect_sources.insert(id.clone());
                state.popups.set_library_dialog(None);
                super::manager::reset_selection(state);
                state.popups.library_picker_index = super::choices(state)
                    .iter()
                    .position(
                        |c| matches!(c, LibraryChoice::Navidrome { source, .. } if source.id == id),
                    )
                    .unwrap_or(0);
                dispatch_settings::save_config_in_background(
                    tx,
                    config,
                    "save Navidrome connection",
                );
                super::cache::start(choice, false, state, tx);
                state.set_status("Connection saved. Scanning library in the background.".into());
                return Ok(vec![]);
            }
            state.popups.set_library_dialog(None);
            super::reset_source(state, audio, tx).await?;
            config
                .navidrome_sources
                .retain(|s| s.id != session.source.id);
            config.navidrome_sources.push(session.source.clone());
            state.sources.navidrome = config.navidrome_sources.clone();
            config.default_folder_source = None;
            config.default_navidrome = Some(crate::navidrome::Selection {
                source_id: session.source.id.clone(),
                folder: session.client.folder.clone(),
            });
            state.active_library = Some(format!(
                "navidrome:{}:{}",
                session.source.id,
                session.client.folder.as_deref().unwrap_or("all")
            ));
            state.sources.active = ActiveSource::Navidrome(Box::new(session));
            super::restore_library_artwork(state, config);
            crate::config::canonicalize_navidrome(config);
            state.sources.sonic_disabled_libraries = config.sonic_disabled_libraries.clone(); // Request slots own server health and retries.
            state.view = View::Browse;
            state.browse_category = BrowseCategory::Library;
            state.category_column_index = state.row_index_for_category(BrowseCategory::Library);
            state.category_column_focused = false;
            dispatch_settings::save_config_in_background(tx, config, "save Navidrome account");
            catalog::open(state, tx);
        }
        NavAction::Rename { id, name } => {
            if !name.trim().is_empty() {
                if let Some(source) = config.navidrome_sources.iter_mut().find(|s| s.id == id) {
                    source.name = name.trim().into();
                }
                state.sources.navidrome = config.navidrome_sources.clone();
                dispatch_settings::save_config_in_background(
                    tx,
                    config,
                    "rename Navidrome account",
                );
            }
        }
        NavAction::Remove(id) => {
            for choice in super::library_choices(state)
                .into_iter()
                .filter(|c| matches!(c, LibraryChoice::Navidrome {source,..} if source.id == id))
            {
                super::cache::cancel(&choice, state);
            }
            let managing = super::manager::settings_active(state);
            let credential_id = id.clone();
            let analysis_keys: Vec<_> = config
                .audiomuse_connections
                .keys()
                .filter(|key| {
                    serde_json::from_str::<serde_json::Value>(key)
                        .ok()
                        .is_some_and(|value| value[1].as_str() == Some(id.as_str()))
                })
                .cloned()
                .collect();
            let credentials_to_remove = analysis_keys.clone();
            match crate::app::tasks::spawn_blocking(move || {
                for key in credentials_to_remove {
                    crate::library::credentials::save_source(
                        "audiomuse",
                        &key,
                        SecretString::default(),
                    )?;
                }
                crate::library::credentials::save_source(
                    "navidrome",
                    &credential_id,
                    SecretString::default(),
                )
            })
            .await?
            {
                Ok(()) => {}
                Err(e) => {
                    state.set_error(format!("Remove account: {e}"));
                    return Ok(vec![]);
                }
            }
            if state
                .sources
                .active
                .navidrome()
                .is_some_and(|s| s.source.id == id)
            {
                super::reset_source(state, audio, tx).await?;
                state.view = if managing {
                    View::Settings
                } else {
                    View::Browse
                };
                state.popups.set_library_picker_active(!managing);
            }
            config.navidrome_sources.retain(|s| s.id != id);
            config
                .ui
                .library_view_settings
                .retain(|key, _| !key.starts_with(&format!("navidrome:{id}:")));
            config.description_history.retain(|key, _| {
                serde_json::from_str::<serde_json::Value>(key)
                    .ok()
                    .is_none_or(|value| value[1].as_str() != Some(id.as_str()))
            });
            state.sources.audiomuse.description_history = config.description_history.clone();
            for key in analysis_keys {
                config.audiomuse_connections.remove(&key);
                state.sources.audiomuse.connections.remove(&key);
            }
            if config
                .default_navidrome
                .as_ref()
                .is_some_and(|selection| selection.source_id == id)
            {
                config.default_navidrome = None;
            }
            state.sources.navidrome = config.navidrome_sources.clone();
            dispatch_settings::save_config_in_background(tx, config, "remove Navidrome account");
        }
        NavAction::Catalog {
            generation,
            request_id,
            refreshing,
            timestamp,
            result,
        } => {
            if generation != state.library_generation || request_id != state.sources.list_id {
                return Ok(vec![]);
            }
            if !refreshing {
                state.sources.listing = None;
            }
            state.library_loading = refreshing;
            state.library.artists_loading = false;
            state.library.albums_loading = false;
            state.library.playlists_loading = false;
            match result {
                Ok(catalog) => {
                    state
                        .cache_mgmt
                        .category_timestamps
                        .insert(super::super::state::RefreshCategory::Artists, timestamp);
                    if !refreshing {
                        state
                            .cache_mgmt
                            .failures
                            .remove(&super::super::state::RefreshCategory::Artists);
                    }
                    catalog::install(state, catalog);
                    let mut changed = false;
                    if let Some(session) = state.sources.active.navidrome() {
                        if let Some(source) = config
                            .navidrome_sources
                            .iter_mut()
                            .find(|s| s.id == session.source.id)
                        {
                            changed = source.libraries != session.source.libraries;
                            source.libraries = session.source.libraries.clone();
                        }
                        state.sources.navidrome = config.navidrome_sources.clone();
                    }
                    changed |= crate::config::canonicalize_navidrome(config);
                    state.sources.sonic_disabled_libraries =
                        config.sonic_disabled_libraries.clone();
                    super::sonic::reconcile(state);
                    if changed {
                        dispatch_settings::save_config_in_background(
                            tx,
                            config,
                            "save Navidrome libraries",
                        );
                    }
                    // A cold start (or cache clear) may finish the catalog after
                    // the user has already opened an analysis view. Resume that
                    // selection instead of leaving an empty, inert pane.
                    if state.artist_nav.loading
                        && state.sources.audiomuse.snapshot.is_none()
                        && !state
                            .sources
                            .nav_tasks
                            .contains_key(super::audiomuse::SYNC_SLOT)
                    {
                        if let Some(commands::CollectionKind::AudioMuse(feature)) = state.sources.nav_collection.filter(|kind| matches!(kind, commands::CollectionKind::AudioMuse(f) if !f.is_search())) {
                            return Ok(vec![super::audiomuse::Command::Open { feature, refresh: false }.into()]);
                        }
                    }
                    if !refreshing {
                        super::audiomuse::check_staleness(state, tx);
                    }
                }
                Err(error) => {
                    if matches!(
                        state.sources.nav_collection,
                        Some(commands::CollectionKind::AudioMuse(_))
                    ) && !state
                        .sources
                        .nav_tasks
                        .contains_key(super::audiomuse::SYNC_SLOT)
                    {
                        state.artist_nav.loading = false;
                    }
                    state.cache_mgmt.failures.insert(
                        super::super::state::RefreshCategory::Artists,
                        super::super::state::RefreshFailure {
                            attempts: 1,
                            retry_at: None,
                        },
                    );
                    state.set_error(format!(
                        "Load Navidrome: {error}. F5 retries; existing data is unchanged."
                    ));
                }
            }
        }
        NavAction::Completed {
            generation,
            request_id,
            slot,
            result,
        } => {
            if generation != state.library_generation
                || state
                    .sources
                    .nav_tasks
                    .get(slot)
                    .is_none_or(|(id, _)| *id != request_id)
            {
                return Ok(vec![]);
            }
            state.sources.nav_tasks.remove(slot);
            if slot == super::audiomuse::SYNC_SLOT {
                state.sources.audiomuse.refresh_failed = result.is_err();
            }
            if matches!(slot, "collection" | "audiomuse") {
                state.artist_nav.loading = false;
            }
            if slot == super::audiomuse::SYNC_SLOT
                && matches!(state.sources.nav_collection, Some(commands::CollectionKind::AudioMuse(f)) if !f.is_search())
            {
                state.artist_nav.loading = false;
                if result.is_err() {
                    state.set_status("AudioMuse unavailable · F5 retries".into());
                }
            }
            match result {
                Ok(actions) => return Ok(actions),
                // Index maintenance must never interrupt browsing or playback,
                // including the first import before a complete cache exists.
                Err(error) if slot == super::audiomuse::SYNC_SLOT => {
                    tracing::warn!("AudioMuse refresh: {error}");
                    let retained = state
                        .sources
                        .audiomuse
                        .snapshot
                        .as_ref()
                        .filter(|s| !s.tracks.is_empty())
                        .map(|s| {
                            if s.updated == 0 {
                                "partial results kept"
                            } else {
                                "cached data kept"
                            }
                        });
                    state.set_status(match retained {
                        Some(retained) => format!("AudioMuse sync failed · {retained} · retry in an analysis category (F5)"),
                        None => "AudioMuse unavailable · retry in an analysis category (F5)".into(),
                    });
                }
                Err(error) => state.set_error(error),
            }
        }
    }
    Ok(vec![])
}

#[cfg(test)]
mod async_discovery_tests {
    use super::*;
    use serde_json::json;
    use std::{sync::Arc, time::Duration};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn account_reads_overlap_and_optional_extension_errors_still_allow_browsing() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let source = Source {
            id: "concurrency-fixture".into(),
            name: "Fixture".into(),
            url: format!("http://{}", listener.local_addr().unwrap()),
            username: "fixture".into(),
            libraries: vec![],
        };
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let server = tokio::spawn(async move {
            let mut replies = tokio::task::JoinSet::new();
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let barrier = barrier.clone();
                replies.spawn(async move {
                    let mut request = Vec::new();
                    let mut chunk = [0; 4096];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        let n = socket.read(&mut chunk).await.unwrap();
                        assert!(n > 0);
                        request.extend_from_slice(&chunk[..n]);
                    }
                    let request = String::from_utf8(request).unwrap();
                    let payload = if request.contains("/getMusicFolders?") {
                        json!({"status":"ok", "musicFolders":{"musicFolder":[{"id":"one","name":"Music"}]}})
                    } else if request.contains("/getOpenSubsonicExtensions?") {
                        json!({"status":"failed", "error":{"code":70,"message":"Unsupported"}})
                    } else {
                        assert!(request.contains("/ping?"));
                        json!({"status":"ok"})
                    };
                    // No response is available until all account reads arrive.
                    barrier.wait().await;
                    let body = json!({"subsonic-response":payload}).to_string();
                    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                });
            }
            while let Some(reply) = replies.join_next().await {
                reply.unwrap();
            }
        });
        let mut session = Session {
            client: Client::new(&source, SecretString::from("fixture-password"), None).unwrap(),
            source,
            extensions: Default::default(),
        };
        let result = tokio::time::timeout(Duration::from_secs(5), discover(&mut session)).await;
        if result.is_err() {
            server.abort();
        }
        result
            .expect("account reads must run concurrently")
            .unwrap();
        assert_eq!(session.source.libraries.len(), 1);
        assert_eq!(session.client.folder.as_deref(), Some("one"));
        assert!(session.extensions.is_empty());
        server.await.unwrap();
    }
}
