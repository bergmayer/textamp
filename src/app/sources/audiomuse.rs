//! AudioMuse enriches an existing library; it never owns playable tracks.
use super::*;
use crate::app::state::{BrowseColumn, BrowseItem, BrowseNavigationState, TextPopup};
use crate::audiomuse::{Client, Connection, Feature, Filter, Snapshot};
use crate::util::SecretString;
use navidrome::commands::CollectionKind;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct State {
    pub connections: HashMap<String, Connection>,
    pub snapshot: Option<Arc<Snapshot>>,
    /// A failed import requires F5 or reopening the library, not a retry loop.
    pub refresh_failed: bool,
}
pub const SYNC_SLOT: &str = "audiomuse-sync";
pub fn key(session: &navidrome::Session) -> String {
    super::sonic::nav_key(&session.source, &session.client.folder)
}
pub fn connection(state: &AppState) -> Option<&Connection> {
    state
        .sources
        .audiomuse
        .connections
        .get(&key(state.sources.active.navidrome()?))
}

#[derive(Debug, Clone)]
pub enum Command {
    Configure(LibraryChoice),
    Submit,
    Disconnect,
    Checked {
        instance: String,
        result: Result<(Connection, SecretString), String>,
    },
    Open {
        feature: Feature,
        refresh: bool,
    },
    Loaded {
        snapshot: Arc<Snapshot>,
        warning: Option<String>,
    },
    Progress {
        generation: u64,
        request_id: u64,
        progress: crate::audiomuse::SyncProgress,
    },
    Filter {
        filter: Filter,
        title: String,
        replace_child: bool,
    },
    Search {
        feature: Feature,
        query: String,
    },
    Results {
        request_id: u64,
        feature: Feature,
        title: String,
        ids: Vec<String>,
    },
}
impl Command {
    pub fn requires_analysis(&self) -> bool {
        !matches!(
            self,
            Self::Configure(_) | Self::Submit | Self::Disconnect | Self::Checked { .. }
        )
    }
}
impl From<Command> for Action {
    fn from(value: Command) -> Action {
        SourceAction::AudioMuse(Box::new(value)).into()
    }
}

#[derive(Debug, Clone)]
pub struct Form {
    pub instance: String,
    pub library_key: String,
    pub fields: [dialogs::Field; 4],
    pub focus: usize,
    pub error: Option<String>,
    pub task: Option<TaskLease>,
}
impl Form {
    pub const LABELS: [&'static str; 4] = [
        "AudioMuse URL",
        "Username",
        "Password (blank keeps saved)",
        "Music-server name in AudioMuse",
    ];
    pub fn draft(&self) -> anyhow::Result<Connection> {
        let connection = Connection {
            url: self.fields[0].value.trim().trim_end_matches('/').into(),
            username: self.fields[1].value.trim().into(),
            server: self.fields[3].value.trim().into(),
        };
        connection.validate()?;
        Ok(connection)
    }
}

pub(super) async fn client(connection: &Connection, library_key: &str) -> anyhow::Result<Client> {
    let key = library_key.to_owned();
    let password = crate::app::tasks::spawn_blocking(move || {
        crate::library::credentials::load_source("audiomuse", &key)
    })
    .await??
    .context("Set AudioMuse credentials in Settings → Libraries")?;
    Client::connect(connection, &password).await
}
use anyhow::Context;

fn begin(feature: Feature, state: &mut AppState) -> u64 {
    state.sources.nav_tasks.remove("collection");
    state.sources.nav_tasks.remove("audiomuse");
    state.set_view(View::Browse);
    state.browse_category = BrowseCategory::Library;
    state.sources.nav_collection = Some(CollectionKind::AudioMuse(feature));
    state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
    state.artist_nav = BrowseNavigationState::with_root(feature.label(), vec![]);
    state.alphabet_strip_focused = false;
    state.track_pane_focused = false;
    state.scroll.browse = None;
    state.select_mode = false;
    state.miller_scroll_manual = false;
    state.list_filter.deactivate();
    state.category_column_index = state.category_rows().iter().position(|row| matches!(row,
        crate::app::state::CategoryRow::NavidromeCollection(CollectionKind::AudioMuse(f)) if *f == feature)).unwrap_or(0);
    state.artist_nav_request_id
}
fn current(state: &AppState, request_id: u64, feature: Feature) -> bool {
    super::sonic::enabled(state)
        && state.artist_nav_request_id == request_id
        && state.sources.nav_collection == Some(CollectionKind::AudioMuse(feature))
        && state.browse_category == BrowseCategory::Library
}

pub fn open(feature: Feature, refresh: bool, state: &mut AppState, tx: &mpsc::Sender<Event>) {
    if !super::sonic::enabled(state) {
        return;
    }
    if connection(state).is_none() {
        state.set_status("Connect AudioMuse in Settings → Libraries".into());
        return;
    }
    if feature.is_search() {
        state.popups.input_dialog = Some(InputDialog {
            title: feature.label().into(),
            input: "".into(),
            action_type: InputDialogAction::AudioMuseSearch(feature),
        });
        return;
    }
    begin(feature, state);
    if state.sources.audiomuse.snapshot.is_some() {
        populate(feature, state);
    }
    // These are views of one library index, not separate downloads. Navigation
    // must not cancel a cold sync or start it again from page one.
    if state.sources.nav_tasks.contains_key(SYNC_SLOT) {
        state.artist_nav.loading = state.sources.audiomuse.snapshot.is_none();
        state.set_status("Loading AudioMuse analysis…".into());
        return;
    }
    start_sync(refresh, state, tx);
}

/// Part of the same weekly metadata maintenance as the Navidrome catalog.
/// This does not select a view, and never initiates server-side analysis.
pub fn check_staleness(state: &mut AppState, tx: &mpsc::Sender<Event>) {
    if super::sonic::enabled(state)
        && !state.library_loading
        && !super::cache::maintenance_paused(state)
    {
        start_sync(false, state, tx);
    }
}

fn start_sync(refresh: bool, state: &mut AppState, tx: &mpsc::Sender<Event>) {
    if state.sources.nav_tasks.contains_key(SYNC_SLOT) {
        return;
    }
    if !refresh
        && (state.sources.audiomuse.refresh_failed
            || state.sources.audiomuse.snapshot.as_ref().is_some_and(|s| {
                !crate::library::cache::refresh_due(s.updated, crate::library::cache::now())
            }))
    {
        return;
    }
    let Some(connection) = connection(state).cloned() else {
        return;
    };
    state.sources.audiomuse.refresh_failed = false;
    let session = state.sources.active.navidrome().unwrap().clone();
    let library_key = key(&session);
    // Exact IDs from the selected, account-scoped Navidrome catalog. Never match titles.
    let allowed: HashSet<_> = state
        .library
        .all_tracks
        .iter()
        .filter_map(|t| session.id(&t.rating_key).ok())
        .collect();
    if allowed.is_empty() {
        if viewing_analysis(state) {
            state.artist_nav.loading = state.library_loading;
            state.set_status(
                if state.library_loading {
                    "Waiting for the Navidrome library…"
                } else {
                    "Load the Navidrome library first (Library → F5)"
                }
                .into(),
            );
        }
        return;
    }
    if viewing_analysis(state) {
        state.artist_nav.loading = state.sources.audiomuse.snapshot.is_none();
        state.set_status(
            if refresh {
                "Refreshing AudioMuse analysis…"
            } else {
                "Loading AudioMuse analysis…"
            }
            .into(),
        );
    }
    // A preview must not turn a retry into an incremental fetch of every
    // remaining song. Only completed snapshots have an update timestamp.
    let previous = state
        .sources
        .audiomuse
        .snapshot
        .clone()
        .filter(|s| s.updated != 0);
    let progress_tx = tx.clone();
    navidrome::effects::spawn_scoped(
        state,
        tx,
        SYNC_SLOT,
        move |generation, request_id| async move {
            let ticket = session.cache_store()?.ticket(&format!(
                "audiomuse:{}",
                serde_json::to_string(&connection)?
            ));
            let read = ticket.clone();
            let cached_ids = allowed.clone();
            let cached = crate::app::tasks::spawn_blocking(move || {
                let mut hit = read.read::<Snapshot>(crate::library::cache::REFRESH_INTERVAL)?;
                if let Some(hit) = &mut hit {
                    hit.value.tracks.retain(|id, _| cached_ids.contains(id));
                }
                Ok::<_, anyhow::Error>(hit)
            })
            .await?;
            let mut warning = None;
            let previous = match cached {
                Ok(Some(hit))
                    if !refresh
                        && !hit.stale
                        && !crate::library::cache::refresh_due(
                            hit.value.updated,
                            crate::library::cache::now(),
                        ) =>
                {
                    return Ok(vec![Command::Loaded {
                        snapshot: Arc::new(hit.value),
                        warning: None,
                    }
                    .into()]);
                }
                Ok(Some(hit)) => {
                    let snapshot = Arc::new(hit.value);
                    // Show stale disk data before network work; a failed refresh
                    // must not make an otherwise usable cache disappear.
                    progress_tx
                        .send(Event::Effect(
                            Command::Progress {
                                generation,
                                request_id,
                                progress: crate::audiomuse::SyncProgress::Preview(snapshot.clone()),
                            }
                            .into(),
                        ))
                        .await?;
                    snapshot
                }
                Ok(None) => previous.unwrap_or_default(),
                Err(error) => {
                    warning = Some(format!("Read AudioMuse cache: {error}"));
                    previous.unwrap_or_default()
                }
            };
            let api = client(&connection, &library_key).await?;
            api.verify().await?;
            let snapshot = tokio::time::timeout(
                Duration::from_secs(600),
                api.refresh_with_progress(&previous, &allowed, |progress| {
                    // Progress is replaceable; never let it block completion or input.
                    let _ = progress_tx.try_send(Event::Effect(
                        Command::Progress {
                            generation,
                            request_id,
                            progress,
                        }
                        .into(),
                    ));
                }),
            )
            .await
            .context("AudioMuse sync timed out; cached data retained")??;
            let snapshot = Arc::new(snapshot);
            let save = snapshot.clone();
            if let Err(error) =
                crate::app::tasks::spawn_blocking(move || ticket.write(save.as_ref())).await?
            {
                warning = Some(format!("Save AudioMuse cache: {error}"));
            }
            Ok(vec![Command::Loaded { snapshot, warning }.into()])
        },
    );
}

fn viewing_analysis(state: &AppState) -> bool {
    matches!(state.sources.nav_collection, Some(CollectionKind::AudioMuse(f)) if !f.is_search())
}

fn populate(feature: Feature, state: &mut AppState) {
    let Some(snapshot) = state.sources.audiomuse.snapshot.clone() else {
        return;
    };
    let focus = state.artist_nav.focused_column;
    let selected_item = state
        .artist_nav
        .focused()
        .and_then(|c| c.selected_item())
        .map(|item| item.key().to_owned());
    if feature == Feature::Analyzed {
        show_tracks(
            state,
            snapshot.tracks.keys().cloned().collect(),
            feature.label(),
            false,
        );
    } else {
        let items = snapshot
            .groups(feature)
            .into_iter()
            .map(|(title, filter)| BrowseItem::Genre {
                key: serde_json::to_string(&filter).expect("analysis filter serialization"),
                title,
            })
            .collect();
        // A completed import can add groups while a preview's child is open.
        // Keep that child and the selected group's identity, not its old index.
        let selected = state
            .artist_nav
            .columns
            .first()
            .and_then(|c| c.selected_item())
            .map(|item| item.key().to_owned());
        state.artist_nav.update_root_items(feature.label(), items);
        if let (Some(selected), Some(root)) =
            (selected.as_ref(), state.artist_nav.columns.first_mut())
        {
            if let Some(index) = root.items.iter().position(|item| item.key() == selected) {
                root.selected_index = index;
            } else {
                // The server removed this group; do not leave its old tracks
                // underneath an unrelated newly-selected heading.
                state.artist_nav.columns.truncate(1);
                state.artist_nav.focused_column = 0;
            }
        }
        // Refresh the open group's tracks as well as the group headings.
        // Otherwise an early preview would remain in the child after completion.
        if let Some(child) = state.artist_nav.columns.get(1) {
            if let Some(filter) = selected.and_then(|key| serde_json::from_str::<Filter>(&key).ok())
            {
                let title = child.title.clone();
                state.artist_nav.focused_column = 0;
                drill(state, filter, title, true);
                state.artist_nav.focused_column = focus;
            }
        }
    }
    if let (Some(selected), Some(column)) = (selected_item, state.artist_nav.columns.get_mut(focus))
    {
        if let Some(index) = column.items.iter().position(|item| item.key() == selected) {
            column.selected_index = index;
        }
    }
    state.artist_nav.loading = false;
    state.set_status(format!(
        "{} analyzed tracks{}{} · F5 updates",
        snapshot.tracks.len(),
        if snapshot.partial {
            " · partial index"
        } else {
            ""
        },
        if snapshot.unresolved > 0 {
            format!(" · {} unmatched IDs", snapshot.unresolved)
        } else {
            String::new()
        }
    ));
}

/// Resolve exclusively against the current Navidrome catalog. Preserve search ranking.
fn show_tracks(state: &mut AppState, ids: Vec<String>, title: &str, child: bool) {
    let Some(session) = state.sources.active.navidrome() else {
        return;
    };
    let by_id: HashMap<_, _> = state
        .library
        .all_tracks
        .iter()
        .filter_map(|t| session.id(&t.rating_key).ok().map(|id| (id, t)))
        .collect();
    let mut seen = HashSet::new();
    let tracks: Vec<_> = ids
        .iter()
        .filter(|id| seen.insert(*id))
        .filter_map(|id| by_id.get(id).map(|t| (*t).clone()))
        .collect();
    let excluded = ids.len().saturating_sub(tracks.len());
    let column = BrowseColumn::new_with_tracks(title, BrowseItem::from_tracks(&tracks), tracks);
    if child {
        state.artist_nav.drill_column(column, true);
    } else {
        state.artist_nav.columns = vec![column];
        state.artist_nav.focused_column = 0;
    }
    state.artist_nav.loading = false;
    if excluded > 0 {
        state.set_status(format!(
            "{excluded} results outside this library omitted · F5 in Library refreshes tracks"
        ));
    }
}

pub fn drill(state: &mut AppState, filter: Filter, title: String, replace_child: bool) {
    if !super::sonic::enabled(state)
        || !matches!(
            state.sources.nav_collection,
            Some(CollectionKind::AudioMuse(_))
        )
    {
        return;
    }
    let Some(snapshot) = &state.sources.audiomuse.snapshot else {
        return;
    };
    let ids = snapshot
        .tracks
        .values()
        .filter(|t| filter.matches(t))
        .map(|t| t.id.clone())
        .collect();
    let focus = state.artist_nav.focused_column;
    show_tracks(state, ids, &title, true);
    if replace_child {
        state.artist_nav.focused_column = focus;
    } else {
        state.artist_nav.focused_column =
            (focus + 1).min(state.artist_nav.columns.len().saturating_sub(1));
    }
}

pub fn track_info(state: &mut AppState, tx: &mpsc::Sender<Event>) {
    if !super::sonic::enabled(state) {
        return;
    }
    let (Some(track), Some(connection), Some(session)) = (
        state.palette_target_track(),
        connection(state).cloned(),
        state.sources.active.navidrome().cloned(),
    ) else {
        return;
    };
    let Ok(id) = session.id(&track.rating_key) else {
        return;
    };
    state.sources.nav_request_id = state.sources.nav_request_id.wrapping_add(1);
    let request_id = state.sources.nav_request_id;
    state.popups.text = Some(TextPopup {
        title: format!("AudioMuse · {}", track.title),
        text: "Loading…".into(),
        scroll: 0,
        request_id,
    });
    if let Some(analysis) = state
        .sources
        .audiomuse
        .snapshot
        .as_ref()
        .and_then(|s| s.tracks.get(&id))
    {
        state.popups.text.as_mut().unwrap().text = analysis.summary();
        return;
    }
    navidrome::effects::spawn(state, tx, "audiomuse-info", async move {
        let result = async {
            client(&connection, &key(&session))
                .await?
                .analysis(&id)
                .await
                .map(|a| a.map_or_else(|| "Not analyzed yet".into(), |a| a.summary()))
        }
        .await;
        Ok(vec![navidrome::NavAction::Text {
            request_id,
            result: result.map_err(|e: anyhow::Error| e.to_string()),
        }
        .into()])
    });
}

pub async fn dispatch(
    command: Command,
    state: &mut AppState,
    config: &mut Config,
    tx: &mpsc::Sender<Event>,
) -> anyhow::Result<Vec<Action>> {
    if command.requires_analysis() && !super::sonic::enabled(state) {
        return Ok(vec![]);
    }
    match command {
        Command::Open { feature, refresh } => open(feature, refresh, state, tx),
        Command::Loaded { snapshot, warning } => {
            // The scoped completion has already verified library and task IDs.
            // Keep the index even when the user browsed away while it loaded.
            state.sources.audiomuse.snapshot = Some(snapshot);
            state.sources.audiomuse.refresh_failed = false;
            if let Some(CollectionKind::AudioMuse(feature)) = state
                .sources
                .nav_collection
                .filter(|kind| matches!(kind, CollectionKind::AudioMuse(f) if !f.is_search()))
            {
                populate(feature, state);
            }
            if let Some(warning) = warning {
                state.set_status(warning);
            }
        }
        Command::Progress {
            generation,
            request_id,
            progress,
        } => {
            if generation == state.library_generation
                && state
                    .sources
                    .nav_tasks
                    .get(SYNC_SLOT)
                    .is_some_and(|(id, _)| *id == request_id)
            {
                if let crate::audiomuse::SyncProgress::Preview(snapshot) = &progress {
                    state.sources.audiomuse.snapshot = Some(snapshot.clone());
                    if let Some(CollectionKind::AudioMuse(feature)) =
                        state.sources.nav_collection.filter(
                            |kind| matches!(kind, CollectionKind::AudioMuse(f) if !f.is_search()),
                        )
                    {
                        populate(feature, state);
                    }
                }
                if matches!(state.sources.nav_collection, Some(CollectionKind::AudioMuse(feature)) if !feature.is_search())
                {
                    state.set_status(progress.to_string());
                }
            }
        }
        Command::Filter {
            filter,
            title,
            replace_child,
        } => drill(state, filter, title, replace_child),
        Command::Search { feature, query } => {
            let Some(connection) = connection(state).cloned() else {
                return Ok(vec![]);
            };
            let Some(session) = state.sources.active.navidrome().cloned() else {
                return Ok(vec![]);
            };
            let request_id = begin(feature, state);
            state.category_column_focused = false;
            state.artist_nav.loading = true;
            navidrome::effects::spawn(state, tx, "audiomuse", async move {
                let api = client(&connection, &key(&session)).await?;
                let ids = api.search(feature, &query).await?;
                Ok(vec![Command::Results {
                    request_id,
                    feature,
                    title: query,
                    ids,
                }
                .into()])
            });
        }
        Command::Results {
            request_id,
            feature,
            title,
            ids,
        } => {
            if current(state, request_id, feature) {
                show_tracks(state, ids, &title, false);
            }
        }
        Command::Configure(choice) => {
            let LibraryChoice::Navidrome { source, folder, .. } = choice else {
                return Ok(vec![]);
            };
            let library_key = super::sonic::nav_key(&source, &folder);
            let saved = state.sources.audiomuse.connections.get(&library_key);
            state.popups.library_dialog = Some(dialogs::Dialog::AudioMuse(Form {
                instance: uuid::Uuid::new_v4().to_string(),
                library_key,
                fields: [
                    dialogs::Field::new(saved.map_or("", |c| c.url.as_str())),
                    dialogs::Field::new(saved.map_or("", |c| c.username.as_str())),
                    dialogs::Field::default(),
                    dialogs::Field::new(saved.map_or("Navidrome", |c| c.server.as_str())),
                ],
                focus: 0,
                error: None,
                task: None,
            }));
        }
        Command::Submit => {
            let Some(dialogs::Dialog::AudioMuse(form)) = &mut state.popups.library_dialog else {
                return Ok(vec![]);
            };
            if form.task.is_some() {
                return Ok(vec![]);
            }
            let connection = match form.draft() {
                Ok(c) => c,
                Err(e) => {
                    form.error = Some(e.to_string());
                    return Ok(vec![]);
                }
            };
            let instance = form.instance.clone();
            let library_key = form.library_key.clone();
            let password = form.fields[2].value.clone();
            let tx = tx.clone();
            let task = crate::app::tasks::spawn(async move {
                let result = async {
                    let password = if password.is_empty() {
                        crate::app::tasks::spawn_blocking(move || {
                            crate::library::credentials::load_source("audiomuse", &library_key)
                        })
                        .await??
                        .context("Enter an AudioMuse password")?
                        .as_str()
                        .into()
                    } else {
                        password
                    };
                    Client::connect(&connection, &password)
                        .await?
                        .verify()
                        .await?;
                    Ok((connection, password))
                }
                .await
                .map_err(|e: anyhow::Error| format!("{e:#}"));
                let _ = tx
                    .send(Event::Effect(Command::Checked { instance, result }.into()))
                    .await;
            });
            form.task = Some(TaskLease::new(&task));
        }
        Command::Checked { instance, result } => {
            let Some(dialogs::Dialog::AudioMuse(form)) = &mut state.popups.library_dialog else {
                return Ok(vec![]);
            };
            if form.instance != instance {
                return Ok(vec![]);
            }
            form.task = None;
            let (connection, password) = match result {
                Ok(v) => v,
                Err(e) => {
                    form.error = Some(e);
                    return Ok(vec![]);
                }
            };
            let library_key = form.library_key.clone();
            let credential_key = library_key.clone();
            if let Err(error) = crate::app::tasks::spawn_blocking(move || {
                crate::library::credentials::save_source("audiomuse", &credential_key, password)
            })
            .await?
            {
                form.error = Some(error.to_string());
                return Ok(vec![]);
            }
            state
                .sources
                .audiomuse
                .connections
                .insert(library_key, connection);
            save(state, config, tx);
        }
        Command::Disconnect => {
            let Some(dialogs::Dialog::AudioMuse(form)) = &state.popups.library_dialog else {
                return Ok(vec![]);
            };
            let library_key = form.library_key.clone();
            let credential_key = library_key.clone();
            crate::app::tasks::spawn_blocking(move || {
                crate::library::credentials::save_source(
                    "audiomuse",
                    &credential_key,
                    SecretString::default(),
                )
            })
            .await??;
            state.sources.audiomuse.connections.remove(&library_key);
            save(state, config, tx);
        }
    }
    Ok(vec![])
}
fn save(state: &mut AppState, config: &mut Config, tx: &mpsc::Sender<Event>) {
    let changes_active = match (
        &state.popups.library_dialog,
        state.sources.active.navidrome(),
    ) {
        (Some(dialogs::Dialog::AudioMuse(form)), Some(session)) => form.library_key == key(session),
        _ => false,
    };
    if changes_active {
        super::sonic::cancel_pending(state);
    }
    config.audiomuse_connections = state.sources.audiomuse.connections.clone();
    dispatch_settings::save_config_in_background(tx, config, "save AudioMuse connection");
    state.sources.audiomuse.snapshot = None;
    state.sources.audiomuse.refresh_failed = false;
    state.sources.nav_tasks.remove("audiomuse");
    state.sources.nav_tasks.remove("audiomuse-info");
    state.sources.nav_tasks.remove(SYNC_SLOT);
    state.popups.library_dialog = None;
    if matches!(
        state.sources.nav_collection,
        Some(CollectionKind::AudioMuse(_))
    ) {
        state.set_browse_category(BrowseCategory::Library, true);
    }
}
