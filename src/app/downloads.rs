//! Download ownership and navigation shared by every presentation.
use super::{Action, AppState, Event, action::*, state::*, tasks::TaskLease};
use crate::{
    library::{models::Track, track::TrackOrigin},
    services::downloads::{Entry, Store},
};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::mpsc;

#[derive(Debug, Default)]
pub struct State {
    pub store: Store,
    pub jobs: BTreeMap<String, Job>,
    manual_offline: bool,
    network_offline: bool,
    pub group_by_album: bool,
    next: u64,
    collection_requests: BTreeMap<u64, TaskLease>,
    pub storage_error: Option<String>,
}
impl State {
    /// Manual Offline Mode and network reachability are independent.
    pub fn is_offline(&self) -> bool {
        self.manual_offline || self.network_offline
    }
}
#[derive(Debug)]
pub struct Job {
    pub entry: Entry,
    source: Source,
    pub phase: Phase,
}
#[derive(Debug)]
pub enum Phase {
    Queued,
    Transferring {
        operation: u64,
        received: u64,
        total: Option<u64>,
        _lease: TaskLease,
    },
    Failed(String),
}
#[derive(Clone, Debug)]
enum Source {
    Navidrome(Box<super::sources::navidrome::Session>),
    Folder(crate::library::FolderSource),
}

/// Owned completion data cleans its temporary file when cancelled or rejected.
#[derive(Clone, Debug)]
pub struct Ready {
    file: Arc<tempfile::TempPath>,
    bytes: u64,
    artwork: Option<Vec<u8>>,
}
/// Download intent and identity-bound asynchronous outcomes.
#[derive(Clone, Debug)]
pub enum Command {
    Open,
    Manage,
    Tracks(Vec<Track>),
    Album(String),
    Playlist(String),
    CollectionReady {
        operation: u64,
        generation: u64,
        result: Result<Vec<Track>, String>,
    },
    Progress {
        id: String,
        operation: u64,
        received: u64,
        total: Option<u64>,
    },
    Finished {
        id: String,
        operation: u64,
        result: Result<Ready, String>,
    },
    Remove(String),
    Cancel(String),
    CancelAll,
    Clear,
    Retry,
    Offline(bool),
    NetworkAvailable(bool),
}
impl From<Command> for Action {
    fn from(value: Command) -> Self {
        SystemAction::Downloads(value).into()
    }
}

/// Shared selection resolves track, album, and ordered collection downloads.
pub fn selection(state: &AppState) -> Vec<Action> {
    if let Some(BrowseItem::Playlist { key, .. }) =
        state.browse_nav().and_then(|nav| nav.selected_item())
    {
        return vec![Command::Playlist(key.clone()).into()];
    }
    if let Some(track) = state
        .palette_target_track()
        .or_else(|| state.focused_track().cloned())
    {
        return vec![Command::Tracks(vec![track]).into()];
    }
    if let Some((key, _)) = state.focused_album() {
        return vec![Command::Album(key).into()];
    }
    if let Some(column) = state.browse_nav().and_then(|nav| nav.focused())
        && !column.tracks.is_empty()
    {
        return vec![Command::Tracks(column.tracks.clone()).into()];
    }
    vec![]
}
#[derive(Clone, Copy)]
enum CollectionKind {
    Album,
    Playlist,
}

fn start_collection(
    state: &mut AppState,
    tx: &mpsc::Sender<Event>,
    key: String,
    kind: CollectionKind,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !state.downloads.is_offline(),
        "Downloads require a connection"
    );
    let session = state
        .sources
        .active
        .navidrome()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Open the collection's tracks to download them"))?;
    session.id(&key)?;
    let tx = tx.clone();
    let generation = state.library_generation;
    state.downloads.next = state.downloads.next.wrapping_add(1);
    let operation = state.downloads.next;
    let task = super::tasks::spawn(async move {
        let result = match kind {
            CollectionKind::Album => session.album_tracks(&key).await,
            CollectionKind::Playlist => session.playlist_tracks(&key).await,
        }
        .map_err(|error| error.to_string());
        let _ = tx
            .send(Event::Effect(
                Command::CollectionReady {
                    operation,
                    generation,
                    result,
                }
                .into(),
            ))
            .await;
    });
    state
        .downloads
        .collection_requests
        .insert(operation, TaskLease::new(&task));
    notify(state, "Preparing download…".into());
    Ok(())
}

/// Execute local reducers and start bounded source-owned background transfers.
/// # Errors
/// Returns storage failures; failed downloads remain visible and retryable.
pub async fn dispatch(
    command: Command,
    state: &mut AppState,
    tx: &mpsc::Sender<Event>,
) -> anyhow::Result<Vec<Action>> {
    if !matches!(&command, Command::Offline(_) | Command::NetworkAvailable(_))
        && let Some(error) = &state.downloads.storage_error
    {
        anyhow::bail!("Downloads are unavailable: {error}");
    }
    let mut actions = Vec::new();
    match command {
        Command::Open | Command::Manage => {
            state.popups.close_all();
            open(state);
        }
        Command::Tracks(tracks) => {
            anyhow::ensure!(
                !state.downloads.is_offline(),
                "Downloads require a connection"
            );
            let library = state
                .active_library
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Choose a library first"))?;
            let choice = super::sources::library_choices(state)
                .into_iter()
                .find(|choice| choice.active(state));
            let library_name = choice
                .as_ref()
                .map(|choice| choice.label())
                .unwrap_or_else(|| "Library".into());
            for track in tracks {
                let id = Store::id(&library, &track);
                if state.downloads.store.entries.contains_key(&id)
                    || state.downloads.jobs.contains_key(&id)
                {
                    continue;
                }
                let source = match &track.origin {
                    TrackOrigin::Navidrome { source_id, .. } => state
                        .sources
                        .active
                        .navidrome()
                        .filter(|session| &session.source.id == source_id)
                        .cloned()
                        .map(Box::new)
                        .map(Source::Navidrome),
                    TrackOrigin::Unavailable => None,
                    TrackOrigin::Folder { source_id, .. } => state
                        .sources
                        .folders
                        .iter()
                        .find(|source| &source.id == source_id)
                        .cloned()
                        .map(Source::Folder),
                }
                .ok_or_else(|| {
                    anyhow::anyhow!("This track does not belong to the active library")
                })?;
                state.downloads.jobs.insert(
                    id,
                    Job {
                        entry: Entry {
                            library: library.clone(),
                            library_name: library_name.clone(),
                            track,
                            bytes: 0,
                            artwork: false,
                        },
                        source,
                        phase: Phase::Queued,
                    },
                );
            }
            notify(state, "Downloading music…".into());
            actions.extend(refresh(state));
        }
        Command::Album(key) => start_collection(state, tx, key, CollectionKind::Album)?,
        Command::Playlist(key) => start_collection(state, tx, key, CollectionKind::Playlist)?,
        Command::CollectionReady {
            operation,
            generation,
            result,
        } => {
            if state
                .downloads
                .collection_requests
                .remove(&operation)
                .is_some()
                && generation == state.library_generation
            {
                return match result {
                    Ok(tracks) => Ok(vec![Command::Tracks(tracks).into()]),
                    Err(error) => Err(anyhow::anyhow!(error)),
                };
            }
        }
        Command::Progress {
            id,
            operation,
            received,
            total,
        } => {
            if let Some(Job {
                phase:
                    Phase::Transferring {
                        operation: current,
                        received: bytes,
                        total: size,
                        ..
                    },
                ..
            }) = state.downloads.jobs.get_mut(&id)
                && *current == operation
            {
                *bytes = received;
                *size = total;
            }
        }
        Command::Finished {
            id,
            operation,
            result,
        } => {
            if !state.downloads.jobs.get(&id).is_some_and(|job|matches!(job.phase,Phase::Transferring{operation:current,..} if current==operation)){return Ok(vec![]);}
            match result {
                Ok(ready) => {
                    let Some(job) = state.downloads.jobs.remove(&id) else {
                        return Ok(vec![]);
                    };
                    let mut entry = job.entry.clone();
                    entry.bytes = ready.bytes;
                    entry.artwork = ready.artwork.is_some();
                    let mut store = state.downloads.store.clone();
                    let commit_id = id.clone();
                    let saved = super::tasks::spawn_blocking(move || {
                        crate::services::downloads::install(
                            &mut store,
                            commit_id,
                            entry,
                            &ready.file,
                            ready.artwork.as_deref(),
                        )?;
                        Ok::<_, anyhow::Error>(store)
                    })
                    .await?;
                    match saved {
                        Ok(store) => {
                            state.downloads.store = store;
                            notify(state, format!("Downloaded {}", job.entry.track.title));
                        }
                        Err(error) => {
                            state.downloads.jobs.insert(
                                id,
                                Job {
                                    phase: Phase::Failed(error.to_string()),
                                    ..job
                                },
                            );
                            state.set_error(format!("Download: {error}"));
                        }
                    }
                    actions.extend(refresh(state));
                }
                Err(error) => {
                    if let Some(job) = state.downloads.jobs.get_mut(&id) {
                        job.phase = Phase::Failed(error);
                    }
                }
            }
        }
        Command::Cancel(id) => {
            if state.downloads.jobs.remove(&id).is_some() {
                notify(state, "Download cancelled".into());
                actions.extend(refresh(state));
            }
        }
        Command::Remove(id) => {
            state.downloads.jobs.remove(&id);
            let mut store = state.downloads.store.clone();
            let (store, result) = super::tasks::spawn_blocking(move || {
                let result = store.remove(&id);
                (store, result)
            })
            .await?;
            state.downloads.store = store;
            actions.extend(refresh(state));
            result?;
        }
        Command::CancelAll => {
            state.downloads.collection_requests.clear();
            state
                .downloads
                .jobs
                .retain(|_, job| matches!(job.phase, Phase::Failed(_)));
            actions.extend(refresh(state));
        }
        Command::Clear => {
            state.downloads.collection_requests.clear();
            state.downloads.jobs.clear();
            let mut store = state.downloads.store.clone();
            let (store, result) = super::tasks::spawn_blocking(move || {
                let result = store.clear();
                (store, result)
            })
            .await?;
            state.downloads.store = store;
            actions.extend(refresh(state));
            result?;
        }
        Command::Retry => {
            for job in state.downloads.jobs.values_mut() {
                if matches!(job.phase, Phase::Failed(_)) {
                    job.phase = Phase::Queued;
                }
            }
        }
        Command::Offline(_) | Command::NetworkAvailable(_) => {
            let was_offline = state.downloads.is_offline();
            match command {
                Command::Offline(offline) => state.downloads.manual_offline = offline,
                Command::NetworkAvailable(available) => {
                    state.downloads.network_offline = !available
                }
                _ => unreachable!(),
            }
            let offline = state.downloads.is_offline();
            if was_offline == offline {
                return Ok(vec![]);
            }
            if offline {
                state.downloads.collection_requests.clear();
                for job in state.downloads.jobs.values_mut() {
                    if matches!(job.phase, Phase::Transferring { .. }) {
                        job.phase = Phase::Queued;
                    }
                }
            }
            if !offline {
                state.set_status("Online".into());
            }
        }
    }
    pump(state, tx);
    Ok(actions)
}

fn pump(state: &mut AppState, tx: &mpsc::Sender<Event>) {
    if state.downloads.is_offline() {
        return;
    }
    let running = state
        .downloads
        .jobs
        .values()
        .filter(|job| matches!(job.phase, Phase::Transferring { .. }))
        .count();
    let ready = state
        .downloads
        .jobs
        .iter()
        .filter(|(_, job)| matches!(job.phase, Phase::Queued))
        .take(2usize.saturating_sub(running))
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    for id in ready {
        let Some(job) = state.downloads.jobs.get_mut(&id) else {
            continue;
        };
        state.downloads.next = state.downloads.next.wrapping_add(1);
        let operation = state.downloads.next;
        let source = job.source.clone();
        let entry = job.entry.clone();
        let root = state.downloads.store.root.clone();
        let output = tx.clone();
        let key = id.clone();
        let task = super::tasks::spawn(async move {
            let result = transfer(source, &entry, root, &output, &key, operation)
                .await
                .map_err(|error| error.to_string());
            let _ = output
                .send(Event::Effect(
                    Command::Finished {
                        id: key,
                        operation,
                        result,
                    }
                    .into(),
                ))
                .await;
        });
        job.phase = Phase::Transferring {
            operation,
            received: 0,
            total: None,
            _lease: TaskLease::new(&task),
        };
    }
}
async fn transfer(
    source: Source,
    entry: &Entry,
    root: std::path::PathBuf,
    tx: &mpsc::Sender<Event>,
    id: &str,
    operation: u64,
) -> anyhow::Result<Ready> {
    tokio::fs::create_dir_all(&root).await?;
    let temporary = tempfile::NamedTempFile::new_in(root)?.into_temp_path();
    let artwork_source = source.clone();
    let artwork = async {
        let library = entry.library.clone();
        let thumb = entry
            .track
            .best_thumb()
            .unwrap_or(&entry.track.rating_key)
            .to_owned();
        let result = match artwork_source {
            Source::Navidrome(session) => session.artwork(&thumb, &library).await.ok(),
            Source::Folder(_) => super::tasks::spawn_blocking(move || {
                crate::media::ArtworkCache::for_library(&library).load(&thumb, u64::MAX)
            })
            .await
            .ok()
            .flatten(),
        };
        Ok::<_, anyhow::Error>(result)
    };
    let audio = async {
        let bytes = match (source, &entry.track.origin) {
            (Source::Navidrome(session), TrackOrigin::Navidrome { song_id, .. }) => {
                session
                    .download(song_id, &temporary, |received, total| {
                        let _ = tx.try_send(Event::Effect(
                            Command::Progress {
                                id: id.into(),
                                operation,
                                received,
                                total,
                            }
                            .into(),
                        ));
                    })
                    .await?
            }
            (Source::Folder(source), TrackOrigin::Folder { path, .. }) => {
                let file = source.file(path).await?;
                tokio::fs::copy(&file.path, &temporary).await?
            }
            _ => anyhow::bail!("Download source mismatch"),
        };
        anyhow::ensure!(bytes > 0, "The server returned an empty download");
        Ok::<_, anyhow::Error>(bytes)
    };
    let (bytes, artwork) = tokio::try_join!(audio, artwork)?;
    Ok(Ready {
        file: Arc::new(temporary),
        bytes,
        artwork,
    })
}
fn notify(state: &mut AppState, message: String) {
    state.set_status(message);
    state.notifications.status_target = Some(NotificationTarget::Downloads);
}

fn visible_tracks(state: &AppState) -> Vec<Track> {
    let mut tracks: Vec<_> = state
        .downloads
        .store
        .entries
        .values()
        .map(|entry| entry.track.clone())
        .chain(
            state
                .downloads
                .jobs
                .values()
                .map(|job| job.entry.track.clone()),
        )
        .collect();
    tracks.sort_by(|a, b| {
        a.album_name()
            .cmp(b.album_name())
            .then(a.index.cmp(&b.index))
            .then(a.title.cmp(&b.title))
    });
    tracks
}

fn track_identity(track: &Track) -> (&str, &str) {
    let source = match &track.origin {
        TrackOrigin::Navidrome { source_id, .. } | TrackOrigin::Folder { source_id, .. } => {
            source_id.as_str()
        }
        TrackOrigin::Unavailable => "",
    };
    (source, track.rating_key.as_str())
}

pub fn is_root_column(state: &AppState, index: usize) -> bool {
    state.browse_category == BrowseCategory::Library
        && state.desktop_library_root == LibraryRoot::Downloads
        && index == 0
}

/// Provider album keys are source-qualified. Folder albums use their directory and tags.
pub(super) fn album_key(track: &Track) -> std::borrow::Cow<'_, str> {
    if let Some(key) = track
        .parent_rating_key
        .as_deref()
        .filter(|key| !key.is_empty())
    {
        return key.into();
    }
    if let TrackOrigin::Folder { source_id, path } = &track.origin
        && let Some(directory) = std::path::Path::new(path).parent()
    {
        return format!(
            "download-album:{}:{}:{}:{}",
            urlencoding::encode(source_id),
            urlencoding::encode(&directory.to_string_lossy()),
            urlencoding::encode(track.artist_name()),
            urlencoding::encode(track.album_name()),
        )
        .into();
    }
    track.rating_key.as_str().into()
}

fn download_column(state: &AppState, tracks: Vec<Track>) -> BrowseColumn {
    let ids: std::collections::HashMap<_, _> = state
        .downloads
        .jobs
        .iter()
        .map(|(id, job)| (track_identity(&job.entry.track), id.as_str()))
        .collect();
    let mut items = BrowseItem::from_tracks(&tracks);
    for (item, track) in items.iter_mut().zip(&tracks) {
        if let BrowseItem::Track { download_id, .. } = item {
            *download_id = ids.get(&track_identity(track)).map(|id| (*id).to_owned());
        }
    }
    BrowseColumn::new_with_tracks("Downloads", items, tracks)
}
/// Context commands target an individual local copy or live transfer, never source music.
pub fn item_command(
    state: &AppState,
    column: &BrowseColumn,
    index: usize,
) -> Option<(&'static str, Command)> {
    if state.view != View::Browse {
        return None;
    }
    let item = column.items.get(index)?;
    if let Some(id) = item.pending_download() {
        return Some((
            Menu::CancelSelection.label(),
            Command::Cancel(id.to_owned()),
        ));
    }
    if state.desktop_library_root != LibraryRoot::Downloads {
        return None;
    }
    let track = column
        .tracks
        .get(index)
        .filter(|track| track.rating_key == item.key())?;
    let (id, _) = state.downloads.store.find(track)?;
    Some((Menu::Remove.label(), Command::Remove(id.to_owned())))
}

pub fn selected_item_command(state: &AppState) -> Option<(&'static str, Command)> {
    if state.category_column_focused || state.track_pane_focused || state.palette.player_context {
        return None;
    }
    let column = state.browse_nav()?.focused()?;
    item_command(state, column, column.selected_index)
}

/// Status text and cancellation share the same live job identity.
pub fn pending_status(state: &AppState, id: &str) -> Option<String> {
    Some(match &state.downloads.jobs.get(id)?.phase {
        Phase::Queued => {
            if state.downloads.is_offline() {
                "Waiting for connection".into()
            } else {
                "Queued".into()
            }
        }
        Phase::Transferring {
            received, total, ..
        } => total
            .filter(|total| *total > 0)
            .map(|total| format!("Downloading · {}%", received.saturating_mul(100) / total))
            .unwrap_or_else(|| format!("Downloading · {}", crate::util::format_bytes(*received))),
        Phase::Failed(error) => format!("Failed · {error}"),
    })
}
/// Filter only the active Downloads list; library and queued playback stay independent.
pub fn ready_selection(state: &AppState, tracks: &[Track]) -> Vec<Track> {
    if state.view != View::Browse || state.desktop_library_root != LibraryRoot::Downloads {
        return tracks.to_vec();
    }
    let pending: std::collections::HashSet<_> = state
        .downloads
        .jobs
        .values()
        .map(|job| track_identity(&job.entry.track))
        .collect();
    tracks
        .iter()
        .filter(|track| !pending.contains(&track_identity(track)))
        .cloned()
        .collect()
}

pub(super) fn toggle_grouping(state: &mut AppState) -> Vec<Action> {
    let nav = &mut state.artist_nav;
    let Some(column) = nav.columns.first_mut() else {
        return vec![];
    };
    let selected = column.selected_item().map(|item| item.key().to_owned());
    let selected_track = if column.grouped_by_album {
        column
            .album_groups
            .as_ref()
            .and_then(|groups| groups.get(column.selected_index))
            .and_then(|group| column.tracks.get(group[0]))
    } else {
        column.tracks.get(column.selected_index)
    };
    state.downloads.group_by_album = !state.downloads.group_by_album;
    let key = selected_track
        .map(|track| {
            if state.downloads.group_by_album {
                album_key(track)
            } else {
                track.rating_key.as_str().into()
            }
            .into_owned()
        })
        .or(selected);
    if state.downloads.group_by_album {
        column.clear_originals();
        column.group_by_album();
    } else {
        column.ungroup_by_album();
    }
    column.selected_index = key
        .and_then(|key| column.items.iter().position(|item| item.key() == key))
        .unwrap_or(0);
    column.selected_set.clear();
    column.selection_anchor = None;
    nav.columns.truncate(1);
    nav.focused_column = 0;
    state.track_pane_open = false;
    state.track_pane_focused = false;
    crate::app::filtering::refresh_column(state, 0)
        .into_iter()
        .collect()
}

/// Populate the ordinary track column; filtering, details and playback retain shared behavior.
pub fn open(state: &mut AppState) {
    state.library_navigation.open_columns();
    state.sources.nav_collection = None;
    state.category_column_focused = false;
    state.desktop_library_root = LibraryRoot::Downloads;
    state.view = View::Browse;
    state.browse_category = BrowseCategory::Library;
    state.track_pane_open = false;
    state.track_pane_focused = false;
    state.list_filter.deactivate();
    let tracks = visible_tracks(state);
    let mut column = download_column(state, tracks);
    column.play_all_row = (!column.ready_tracks().is_empty()).then(|| PlayAllRow::AllTracks {
        label: "Downloads".into(),
    });
    if state.downloads.group_by_album {
        column.group_by_album();
    }
    state.artist_nav.columns = vec![column];
    state.artist_nav.focused_column = 0;
    state.artist_nav.loading = false;
}
pub(super) fn refresh(state: &mut AppState) -> Option<Action> {
    if !is_root_column(state, 0) || state.artist_nav.columns.is_empty() {
        return None;
    }
    let tracks = visible_tracks(state);
    let child = state
        .artist_nav
        .columns
        .get(1)
        .and_then(|column| match &column.play_all_row {
            Some(PlayAllRow::Album { rating_key, .. }) => {
                let tracks = tracks
                    .iter()
                    .filter(|track| album_key(track) == rating_key.as_str())
                    .cloned()
                    .collect();
                Some(download_column(state, tracks))
            }
            _ => None,
        });
    let mut updated = download_column(state, tracks);
    let column = &mut state.artist_nav.columns[0];
    let selected = column.selected_item().map(|item| item.key().to_owned());
    updated.play_all_row = (!updated.ready_tracks().is_empty()).then(|| PlayAllRow::AllTracks {
        label: "Downloads".into(),
    });
    updated.artwork_visible = column.artwork_visible;
    updated.apply_sort(column.sort_mode);
    if !column.sort_ascending {
        updated.items.reverse();
        updated.tracks.reverse();
        updated.sort_ascending = false;
    }
    if state.downloads.group_by_album {
        updated.clear_originals();
        updated.group_by_album();
    }
    updated.selected_index = selected
        .and_then(|key| updated.items.iter().position(|item| item.key() == key))
        .unwrap_or(column.selected_index)
        .min(updated.items.len().saturating_sub(1));
    *column = updated;
    if let Some(child) = child {
        state.artist_nav.columns[1].replace_content(child.items, child.tracks);
    }
    crate::app::filtering::refresh_column(state, state.list_filter.column)
}
/// Visible download count and committed audio size, shared by frontend summaries.
pub fn summary(state: &AppState, count: usize) -> String {
    format!(
        "{} · {}",
        super::presentation::item_count_label(count),
        crate::util::format_bytes(state.downloads.store.size())
    )
}

/// Use committed audio before asking a provider to connect. Never starts a second backend.
pub fn play(state: &mut AppState, audio: &mut crate::audio::AudioPlayer) -> bool {
    let Some(track) = state.current_track().cloned() else {
        return false;
    };
    let Some((id, entry)) = state.downloads.store.find(&track) else {
        return false;
    };
    let path = state.downloads.store.root.join(id).join("audio");
    if !path.is_file() {
        return false;
    }
    state.artwork.current_data = entry
        .artwork
        .then(|| std::fs::read(state.downloads.store.root.join(id).join("artwork")).ok())
        .flatten();
    state.sources.nav_tasks.remove("now-playing");
    state.playback.preparation_id = state.playback.preparation_id.wrapping_add(1);
    state.playback.scrobble = ScrobbleState::Pending;
    state.waveform = WaveformState {
        track_key: Some(track.rating_key.clone()),
        ..Default::default()
    };
    state.spectrogram = SpectrogramState {
        track_key: Some(track.rating_key.clone()),
        ..Default::default()
    };
    state.artwork.fetch = ArtworkFetch::Idle;
    state.artwork.current_thumb = track.best_thumb().map(str::to_owned);
    let file = crate::library::MediaFile::local(path);
    state.sources.media = super::sources::MediaPreparation::Ready(file.clone());
    match audio.play_file(file) {
        Ok(()) => {
            state.playback.request_id = audio.playback_id();
            state.playback.position_ms = 0;
            state.playback.duration_ms = track.duration_ms();
            state.playback.status = PlayStatus::Playing;
            state.playback.playback_started_at = Some(std::time::Instant::now());
        }
        Err(error) => {
            state.playback.status = PlayStatus::Stopped;
            state.set_error(format!("Playback: {error}"));
        }
    }
    true
}

/// Download operations exposed by native menus and both command palettes.
#[derive(Debug, Clone, Copy)]
pub enum Menu {
    Open,
    Selection,
    Queue,
    Manage,
    Remove,
    CancelSelection,
    Cancel,
    Clear,
    Retry,
    Offline,
}
impl Menu {
    /// Availability is shared by palettes, menus and download management.
    pub fn disabled_reason(self, state: &AppState) -> Option<&'static str> {
        if state.downloads.storage_error.is_some() {
            return Some("Download storage is unavailable");
        }
        match self {
            Self::Selection | Self::Queue | Self::Retry if state.downloads.is_offline() => {
                Some("Unavailable offline")
            }
            Self::Selection if state.active_library.is_none() => Some("Choose a library first"),
            Self::Selection
                if state.palette_target_track().is_none()
                    && state.focused_album().is_none()
                    && !state
                        .browse_nav()
                        .and_then(|nav| nav.focused())
                        .is_some_and(|column| {
                            !column.tracks.is_empty()
                                || matches!(
                                    column.selected_item(),
                                    Some(BrowseItem::Playlist { .. })
                                )
                        }) =>
            {
                Some("Select a track, album or playlist")
            }
            Self::Queue if state.playback_tracks().is_empty() => Some("The queue is empty"),
            Self::Remove
                if state
                    .palette_target_track()
                    .or_else(|| state.focused_track().cloned())
                    .is_none_or(|track| state.downloads.store.find(&track).is_none()) =>
            {
                Some("Select a downloaded track")
            }
            Self::CancelSelection
                if !matches!(selected_item_command(state), Some((_, Command::Cancel(_)))) =>
            {
                Some("Select a pending download")
            }
            Self::Cancel
                if state.downloads.jobs.is_empty()
                    && state.downloads.collection_requests.is_empty() =>
            {
                Some("No downloads are running")
            }
            Self::Clear
                if state.downloads.store.entries.is_empty() && state.downloads.jobs.is_empty() =>
            {
                Some("No downloaded music")
            }
            Self::Retry
                if !state
                    .downloads
                    .jobs
                    .values()
                    .any(|job| matches!(job.phase, Phase::Failed(_))) =>
            {
                Some("No failed downloads")
            }
            _ => None,
        }
    }
    /// Controls for the download-management surface.
    pub const MANAGEMENT: [Self; 5] = [
        Self::Open,
        Self::Retry,
        Self::Cancel,
        Self::Offline,
        Self::Clear,
    ];
    pub const ALL: [Self; 10] = [
        Self::Open,
        Self::Selection,
        Self::Queue,
        Self::Manage,
        Self::Remove,
        Self::CancelSelection,
        Self::Cancel,
        Self::Clear,
        Self::Retry,
        Self::Offline,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Downloads",
            Self::Selection => "Download Selection",
            Self::Queue => "Download Play Queue",
            Self::Manage => "Manage Downloads",
            Self::Remove => "Remove Download",
            Self::CancelSelection => "Cancel Download",
            Self::Cancel => "Cancel Downloads",
            Self::Clear => "Remove All Downloads",
            Self::Retry => "Retry Failed Downloads",
            Self::Offline => "Toggle Offline Mode",
        }
    }
    pub fn actions(self, state: &mut AppState) -> Vec<Action> {
        match self {
            Self::Selection => selection(state),
            Self::Queue => vec![Command::Tracks(state.playback_tracks().to_vec()).into()],
            Self::CancelSelection => selected_item_command(state)
                .filter(|(_, command)| matches!(command, Command::Cancel(_)))
                .map(|(_, command)| vec![command.into()])
                .unwrap_or_default(),
            Self::Remove => state
                .palette_target_track()
                .or_else(|| state.focused_track().cloned())
                .and_then(|track| {
                    state
                        .downloads
                        .store
                        .find(&track)
                        .map(|(id, _)| id.to_owned())
                })
                .map(|id| vec![Command::Remove(id).into()])
                .unwrap_or_default(),
            _ => vec![
                match self {
                    Self::Open => Command::Open,
                    Self::Manage => Command::Manage,
                    Self::Cancel => Command::CancelAll,
                    Self::Clear => Command::Clear,
                    Self::Retry => Command::Retry,
                    Self::Offline => Command::Offline(!state.downloads.manual_offline),
                    _ => return vec![],
                }
                .into(),
            ],
        }
    }
}
/// Suppress network-only discovery while retaining completed metadata and local playback.
pub fn suppress_offline(action: &Action, state: &mut AppState) -> bool {
    if !state.downloads.is_offline() {
        return false;
    }
    match action {
        Action::Data(DataAction::LoadTrackPaneSimilar { rating_key, .. }) => {
            state.track_pane_similar_loading.remove(rating_key);
            state
                .track_pane_similar
                .insert(rating_key.clone(), Err("Unavailable offline".into()));
            true
        }
        Action::Data(
            DataAction::LoadSimilarTracks { .. }
            | DataAction::LoadSimilarAlbums { .. }
            | DataAction::LoadSimilarArtists { .. }
            | DataAction::LoadRelated { .. },
        ) => {
            state.set_status("Unavailable offline".into());
            true
        }
        _ => false,
    }
}

/// Prefer saved media and metadata when a request belongs to the offline collection.
pub fn route(action: &Action, state: &mut AppState) -> Option<Vec<Action>> {
    if let Action::System(SystemAction::LoadAlbumArt(requests)) = action {
        let mut remaining = vec![];
        for (key, thumb) in requests {
            let saved = state.downloads.store.entries.iter().find(|(_, entry)| {
                entry.artwork
                    && (entry.track.rating_key == *key
                        || entry.track.parent_rating_key.as_ref() == Some(key)
                        || (key.starts_with("download-album:")
                            && album_key(&entry.track) == key.as_str())
                        || entry.track.best_thumb() == Some(thumb.as_str()))
            });
            if let Some((id, _)) = saved
                && let Ok(bytes) =
                    std::fs::read(state.downloads.store.root.join(id).join("artwork"))
            {
                state.artwork.insert_grid_art(key.clone(), bytes);
                state.artwork.grid_pending.remove(key);
            } else if thumb.is_empty() {
                state.artwork.grid_pending.remove(key);
            } else {
                remaining.push((key.clone(), thumb.clone()));
            }
        }
        if state.downloads.is_offline() {
            for (key, _) in requests {
                state.artwork.grid_pending.remove(key);
            }
            return Some(vec![]);
        }
        if remaining.len() != requests.len() {
            return Some(if remaining.is_empty() {
                vec![]
            } else {
                vec![SystemAction::LoadAlbumArt(remaining).into()]
            });
        }
    }
    let in_downloads = state.view == View::Browse && is_root_column(state, 0);
    if state.downloads.is_offline()
        && let Action::Browse(BrowseAction::OpenDetailsInLibrary { track, destination }) = action
        && let Some(actions) = open_saved_details(state, track, *destination)
    {
        return Some(actions);
    }
    if let Action::Miller(MillerAction::LoadAlbumTracksForMiller {
        album_key,
        replace_child,
    }) = action
        && let Some(actions) = drill_saved_album(state, album_key, *replace_child, in_downloads)
    {
        return Some(actions);
    }
    if !state.downloads.is_offline() && !in_downloads {
        return None;
    }
    let (key, placement) = match action {
        Action::Queue(
            QueueAction::PlayAlbum { rating_key } | QueueAction::PlayAlbumNow { rating_key, .. },
        ) => (rating_key, super::playback_controls::QueuePlacement::Now),
        Action::Queue(QueueAction::EnqueueAlbum { rating_key, .. }) => {
            (rating_key, super::playback_controls::QueuePlacement::End)
        }
        Action::Queue(QueueAction::EnqueueAlbumNext { rating_key, .. }) => {
            (rating_key, super::playback_controls::QueuePlacement::Next)
        }
        _ => return None,
    };
    let tracks = saved_album(state, key);
    if tracks.is_empty() {
        if !state.downloads.is_offline()
            && !state
                .downloads
                .jobs
                .values()
                .any(|job| album_key(&job.entry.track) == key.as_str())
        {
            return None;
        }
        let message = if state.downloads.is_offline() {
            "Not downloaded · Unavailable offline"
        } else {
            "No tracks in this album have finished downloading"
        };
        state.set_status(message.into());
        return Some(vec![]);
    }
    Some(vec![
        match placement {
            super::playback_controls::QueuePlacement::Now => QueueAction::PlayTracksNow(tracks),
            super::playback_controls::QueuePlacement::Next => {
                QueueAction::EnqueueTracksNext(tracks)
            }
            super::playback_controls::QueuePlacement::End => {
                super::playback_controls::append_tracks(&tracks, "Downloads")
            }
        }
        .into(),
    ])
}
fn drill_saved_album(
    state: &mut AppState,
    key: &str,
    replace_child: bool,
    in_downloads: bool,
) -> Option<Vec<Action>> {
    let tracks = if in_downloads {
        let root = state.artist_nav.columns.first()?;
        let indices = root.album_groups.as_ref()?.get(root.selected_index)?;
        if root.selected_item()?.key() != key {
            return None;
        }
        indices
            .iter()
            .map(|&i| root.tracks[i].clone())
            .collect::<Vec<_>>()
    } else if state.downloads.is_offline() {
        saved_album(state, key)
    } else {
        return None;
    };
    let title = tracks.first()?.album_name().to_owned();
    let mut column = download_column(state, tracks);
    column.title = title.clone();
    column.play_all_row = Some(PlayAllRow::Album {
        rating_key: key.to_owned(),
        title,
    });
    state.browse_nav_mut()?.drill_column(column, replace_child);
    state.sources.nav_tasks.remove("album-browse");
    state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
    state.miller_reveal_right = true;
    Some(vec![])
}

fn open_saved_details(
    state: &mut AppState,
    track: &Track,
    destination: super::library_details::Destination,
) -> Option<Vec<Action>> {
    use super::library_details::Destination;
    let key = album_key(track);
    let saved = match destination {
        Destination::Track => state.downloads.store.find(track)?.1.track.clone(),
        Destination::Album => saved_album(state, &key).into_iter().next()?,
        Destination::Artist => {
            let artist_key = if track.track_artist() == track.artist_name() {
                track.grandparent_rating_key.as_deref().unwrap_or("")
            } else {
                ""
            };
            let name = track.track_artist();
            let document = offline_artist(state, artist_key, name, Some(track))?;
            let key = if artist_key.is_empty() {
                format!(
                    "download-artist:{}:{}",
                    urlencoding::encode(track_identity(track).0),
                    urlencoding::encode(name)
                )
            } else {
                artist_key.to_owned()
            };
            state.popups.set_artist_bio(Some(ArtistBioPopup {
                artist_key: key,
                artist_name: name.to_owned(),
                document,
                scroll: 0,
                google_focused: false,
                image_index: 0,
                task: None,
            }));
            return Some(vec![]);
        }
    };
    reopen_for_details(state, &saved);
    if destination == Destination::Track && !state.downloads.group_by_album {
        return Some(vec![BrowseAction::OpenTrackDetails.into()]);
    }
    let key = album_key(&saved);
    let tracks = saved_album(state, &key);
    let mut column = download_column(state, tracks);
    column.title = saved.album_name().into();
    column.play_all_row = Some(PlayAllRow::Album {
        rating_key: key.into_owned(),
        title: saved.album_name().into(),
    });
    column.selected_index = column
        .tracks
        .iter()
        .position(|track| track_identity(track) == track_identity(&saved))
        .unwrap_or(0);
    state.artist_nav.push_column(column);
    state.miller_reveal_right = true;
    Some(vec![if destination == Destination::Album {
        MillerAction::OpenAlbumDetails.into()
    } else {
        BrowseAction::OpenTrackDetails.into()
    }])
}

fn reopen_for_details(state: &mut AppState, track: &Track) {
    super::navigation_history::record(state);
    state.popups.close_all();
    state.pending_library_details = None;
    state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
    state.sources.nav_tasks.remove("album-browse");
    open(state);
    let key = if state.downloads.group_by_album {
        album_key(track)
    } else {
        track.rating_key.as_str().into()
    };
    let column = &mut state.artist_nav.columns[0];
    column.selected_index = column
        .items
        .iter()
        .position(|item| item.key() == key)
        .unwrap_or(0);
}

pub(super) fn offline_artist(
    state: &AppState,
    key: &str,
    name: &str,
    seed: Option<&Track>,
) -> Option<crate::services::biography::Biography> {
    let mut albums = BTreeMap::new();
    for entry in state.downloads.store.entries.values() {
        let track = &entry.track;
        let same_source = seed
            .is_some_and(|seed| track_identity(seed).0 == track_identity(track).0)
            || (!key.is_empty()
                && key.starts_with(&format!("navidrome:{}:", track_identity(track).0)));
        if !(track.grandparent_rating_key.as_deref() == Some(key)
            || (same_source && track.track_artist().eq_ignore_ascii_case(name)))
        {
            continue;
        }
        *albums
            .entry((track.album_name(), album_key(track)))
            .or_insert(0usize) += 1;
    }
    if albums.is_empty() {
        return None;
    }
    let mut text = String::from("Downloaded albums");
    for ((title, _), count) in albums {
        use std::fmt::Write;
        let _ = write!(
            text,
            "\n{title} · {count} {}",
            if count == 1 { "track" } else { "tracks" }
        );
    }
    Some(crate::services::biography::Biography {
        unavailable: true,
        text,
        ..Default::default()
    })
}

fn saved_album(state: &AppState, key: &str) -> Vec<Track> {
    let mut tracks = state
        .downloads
        .store
        .entries
        .values()
        .filter(|entry| album_key(&entry.track) == key)
        .map(|entry| entry.track.clone())
        .collect::<Vec<_>>();
    tracks.sort_by_key(|track| (track.parent_index, track.index));
    tracks
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn pending_job(state: &mut AppState, name: &str) -> (String, Track) {
        let track = Track::from_folder("fixture", name);
        let id = Store::id("fixture", &track);
        state.downloads.jobs.insert(
            id.clone(),
            Job {
                entry: Entry {
                    library: "fixture".into(),
                    library_name: "Fixture".into(),
                    track: track.clone(),
                    bytes: 0,
                    artwork: false,
                },
                source: Source::Folder(crate::library::FolderSource {
                    id: "fixture".into(),
                    name: "Fixture".into(),
                    location: crate::library::FolderLocation::Local {
                        path: "/unused".into(),
                    },
                }),
                phase: Phase::Queued,
            },
        );
        (id, track)
    }

    #[tokio::test]
    async fn failed_audio_does_not_wait_for_optional_cover() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = server.local_addr().unwrap();
        let fixture = tokio::spawn(async move {
            let mut audio = None;
            let mut cover = None;
            for _ in 0..2 {
                let (socket, _) = server.accept().await.unwrap();
                let mut socket = BufReader::new(socket);
                let mut request = String::new();
                socket.read_line(&mut request).await.unwrap();
                if request.contains("/rest/download") {
                    audio = Some(socket);
                } else if request.contains("/rest/getCoverArt") {
                    cover = Some(socket);
                } else {
                    panic!("unexpected fixture request");
                }
            }
            audio
                .as_mut()
                .unwrap()
                .write_all(
                    b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            // The audio failure must complete even while the server holds the cover request.
            std::future::pending::<()>().await;
            drop(cover);
        });

        let id = "download-failure-fixture";
        let source = crate::navidrome::Source {
            id: id.into(),
            name: "Fixture".into(),
            url: format!("http://{address}"),
            username: "fixture".into(),
            libraries: vec![],
        };
        let session = super::super::sources::navidrome::Session {
            client: crate::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
            source,
            extensions: Default::default(),
        };
        let entry = Entry {
            library: id.into(),
            library_name: "Fixture".into(),
            artwork: false,
            bytes: 0,
            track: Track {
                origin: TrackOrigin::Navidrome {
                    source_id: id.into(),
                    song_id: "song".into(),
                },
                rating_key: session.key("song"),
                ..Default::default()
            },
        };
        let root = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::channel(4);
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            transfer(
                Source::Navidrome(Box::new(session)),
                &entry,
                root.path().into(),
                &tx,
                "fixture",
                1,
            ),
        )
        .await;
        fixture.abort();
        assert!(
            result
                .expect("audio failure was blocked by artwork")
                .is_err()
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn pending_rows_are_visible_unplayable_and_individually_cancellable() {
        let mut state = AppState::new();
        state.downloads.manual_offline = true;
        let (first, _) = pending_job(&mut state, "A.wav");
        let (second, _) = pending_job(&mut state, "B.wav");
        open(&mut state);
        let column = &state.artist_nav.columns[0];
        assert_eq!(column.items.len(), 2);
        assert!(column.ready_tracks().is_empty());
        assert!(column.play_all_row.is_none());
        assert_eq!(column.items[0].pending_download(), Some(first.as_str()));
        assert!(state.palette_track_list().is_none());
        assert_eq!(
            super::super::activity::target(&state),
            Some(NotificationTarget::Downloads)
        );
        let (tx, _) = mpsc::channel(1);
        let entries = super::super::command_palette::materialize_entries(&state);
        assert_eq!(entries[0].label, "Cancel Download");
        assert!(super::super::command_catalog::contextual(
            &entries[0].command
        ));
        assert!(
            matches!(item_command(&state, &state.artist_nav.columns[0], 0), Some(("Cancel Download", Command::Cancel(id))) if id == first)
        );
        dispatch(Command::Cancel(first), &mut state, &tx)
            .await
            .unwrap();
        assert_eq!(state.downloads.jobs.len(), 1);
        assert!(state.downloads.jobs.contains_key(&second));
        assert_eq!(state.artist_nav.columns[0].items.len(), 1);
    }

    #[tokio::test]
    async fn cancelling_transfer_aborts_lease_and_rejects_late_completion() {
        let mut state = AppState::new();
        let (id, _) = pending_job(&mut state, "A.wav");
        let task = tokio::spawn(std::future::pending::<()>());
        state.downloads.jobs.get_mut(&id).unwrap().phase = Phase::Transferring {
            operation: 42,
            received: 25,
            total: Some(100),
            _lease: TaskLease::new(&task),
        };
        assert_eq!(
            pending_status(&state, &id).as_deref(),
            Some("Downloading · 25%")
        );
        let (tx, _) = mpsc::channel(1);
        dispatch(Command::Cancel(id.clone()), &mut state, &tx)
            .await
            .unwrap();
        assert!(task.await.unwrap_err().is_cancelled());
        dispatch(
            Command::Finished {
                id,
                operation: 42,
                result: Err("late error".into()),
            },
            &mut state,
            &tx,
        )
        .await
        .unwrap();
        assert!(state.downloads.jobs.is_empty());
        assert!(state.notifications.last_error.is_none());
    }

    #[test]
    fn unrelated_status_does_not_keep_download_navigation() {
        let mut state = AppState::new();
        notify(&mut state, "Downloaded song".into());
        assert_eq!(
            state.current_notification().unwrap().target,
            Some(NotificationTarget::Downloads)
        );
        state.set_status("Saved settings".into());
        assert_eq!(state.current_notification().unwrap().target, None);
    }

    #[test]
    fn refreshing_downloads_keeps_filter_and_selected_track() {
        let mut state = AppState::new();
        let add = |state: &mut AppState, title: &str| {
            let mut track = Track::from_folder("fixture", title);
            track.title = title.into();
            state.downloads.store.entries.insert(
                Store::id("fixture", &track),
                Entry {
                    library: "fixture".into(),
                    library_name: "Fixture".into(),
                    track,
                    bytes: 10,
                    artwork: false,
                },
            );
        };
        add(&mut state, "Song B");
        add(&mut state, "Song C");
        open(&mut state);
        state.artist_nav.columns[0].selected_index = 1;
        state.list_filter.active = true;
        state.list_filter.query = "Song".into();
        state.list_filter.advance_version();
        let old_version = state.list_filter.version;
        add(&mut state, "Song A");
        let action = refresh(&mut state);
        assert_eq!(
            state.browse_item_summary().as_deref(),
            Some("3 items · 30 B")
        );
        assert!(matches!(
            action,
            Some(Action::Search(SearchAction::RunListFilter { .. }))
        ));
        assert_eq!(state.list_filter.query, "Song");
        assert_eq!(state.artist_nav.columns[0].selected_index, 2);

        let (tx, _) = mpsc::channel(1);
        for version in [old_version, state.list_filter.version] {
            super::super::handlers::events::handle_app_event(
                Event::Ui(super::super::event::UiEvent::ListFilterCompleted {
                    version,
                    generation: state.library_generation,
                    results: ListFilterResults {
                        matched_indices: vec![0, 1, 2],
                        total_matches: 3,
                        has_more: false,
                    },
                }),
                &mut state,
                &tx,
            );
            if version == old_version {
                assert!(state.list_filter.loading);
            }
        }
        assert!(!state.list_filter.loading);
        assert_eq!(state.list_filter.selected, 2);
        assert_eq!(state.artist_nav.columns[0].tracks[2].title, "Song C");
        assert_eq!(state.artist_nav.columns[0].selected_index, 2);
    }

    #[tokio::test]
    async fn cancelling_album_resolution_rejects_its_late_tracks() {
        let mut state = AppState::new();
        let task = tokio::spawn(std::future::pending::<()>());
        state
            .downloads
            .collection_requests
            .insert(7, TaskLease::new(&task));
        let (tx, _) = mpsc::channel(1);
        dispatch(Command::CancelAll, &mut state, &tx).await.unwrap();
        let generation = state.library_generation;
        let actions = dispatch(
            Command::CollectionReady {
                operation: 7,
                generation,
                result: Ok(vec![Track::from_folder("fixture", "song.wav")]),
            },
            &mut state,
            &tx,
        )
        .await
        .unwrap();
        assert!(actions.is_empty());
        assert!(task.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn local_download_is_durable_and_cancelled_completion_cannot_restore_it() {
        let temporary = tempfile::tempdir().unwrap();
        let source = crate::library::FolderSource {
            id: "fixture".into(),
            name: "Fixture".into(),
            location: crate::library::FolderLocation::Local {
                path: temporary.path().into(),
            },
        };
        std::fs::write(temporary.path().join("song.wav"), b"local music").unwrap();
        let mut state = AppState::new();
        state.active_library = Some("fixture-library".into());
        state.sources.folders.push(source);
        state.downloads.store = Store::load(temporary.path().join("downloads")).unwrap();
        let mut track = Track::from_folder("fixture", "song.wav");
        track.title = "Downloaded Song".into();
        let id = Store::id("fixture-library", &track);
        let (tx, mut rx) = mpsc::channel(32);
        dispatch(Command::Tracks(vec![track.clone()]), &mut state, &tx)
            .await
            .unwrap();
        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let Event::Effect(Action::System(SystemAction::Downloads(completion))) = event else {
            panic!("download completion expected")
        };
        let stale = completion.clone();
        dispatch(completion, &mut state, &tx).await.unwrap();
        assert!(state.downloads.store.find(&track).is_some());
        dispatch(Command::Cancel(id.clone()), &mut state, &tx)
            .await
            .unwrap();
        assert!(state.downloads.store.find(&track).is_some());
        state.queue.tracks = vec![track.clone()];
        state.queue.index = Some(0);
        state.playback_mode = PlaybackMode::Queue;
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        assert!(play(&mut state, &mut audio));
        let super::super::sources::MediaPreparation::Ready(file) = &state.sources.media else {
            panic!("downloaded audio must be routed locally before a provider request");
        };
        assert_eq!(
            file.path,
            state.downloads.store.root.join(&id).join("audio")
        );
        assert_eq!(state.current_track().unwrap().origin, track.origin);
        assert_eq!(
            Store::load(state.downloads.store.root.clone())
                .unwrap()
                .entries
                .len(),
            1
        );
        open(&mut state);
        let (label, removal) = item_command(&state, &state.artist_nav.columns[0], 0).unwrap();
        assert_eq!(label, "Remove Download");
        assert!(matches!(&removal, Command::Remove(key) if key == &id));
        dispatch(removal, &mut state, &tx).await.unwrap();
        dispatch(stale, &mut state, &tx).await.unwrap();
        assert!(state.downloads.store.entries.is_empty());
        assert!(temporary.path().join("song.wav").exists());
    }
    fn save_album_track(state: &mut AppState, source: &str, song: &str) -> Track {
        let track = Track {
            origin: TrackOrigin::Navidrome {
                source_id: source.into(),
                song_id: song.into(),
            },
            rating_key: format!("navidrome:{source}:{song}"),
            parent_rating_key: Some(format!("navidrome:{source}:album")),
            grandparent_rating_key: Some(format!("navidrome:{source}:artist")),
            title: song.into(),
            parent_title: Some("Saved Album".into()),
            grandparent_title: Some("Saved Artist".into()),
            parent_year: Some(2001),
            ..Default::default()
        };
        state.downloads.store.entries.insert(
            Store::id(source, &track),
            Entry {
                library: source.into(),
                library_name: source.into(),
                track: track.clone(),
                bytes: 10,
                artwork: false,
            },
        );
        track
    }

    #[tokio::test]
    async fn downloads_grouping_preserves_local_tracks_and_offline_details() {
        use crate::app::{library_details::Destination, runtime::Session};
        let mut session = Session::with_audio(
            Default::default(),
            crate::audio::AudioPlayer::new_without_audio(),
        );
        let saved = save_album_track(&mut session.state, "fixture", "Song A");
        save_album_track(&mut session.state, "fixture", "Song B");
        save_album_track(&mut session.state, "other", "Song C");
        session.dispatch(Command::Open.into()).await.unwrap();
        let popup = super::super::view_options::for_column(&session.state, 0).unwrap();
        assert!(popup.options.contains(&SortPopupOption::GroupByAlbum));
        let (pending_id, _) = pending_job(&mut session.state, "Pending");
        session
            .state
            .downloads
            .jobs
            .get_mut(&pending_id)
            .unwrap()
            .entry
            .track = Track {
            rating_key: "navidrome:fixture:pending".into(),
            title: "Pending".into(),
            ..saved.clone()
        };
        refresh(&mut session.state);
        toggle_grouping(&mut session.state);
        assert_eq!(session.state.artist_nav.columns[0].items.len(), 2);
        assert_eq!(session.state.artist_nav.columns[0].ready_tracks().len(), 3);
        session
            .dispatch(Command::NetworkAvailable(false).into())
            .await
            .unwrap();
        session
            .dispatch(
                MillerAction::LoadAlbumTracksForMiller {
                    album_key: album_key(&saved).into_owned(),
                    replace_child: false,
                }
                .into(),
            )
            .await
            .unwrap();
        let album = session.state.artist_nav.focused().unwrap();
        assert_eq!(album.tracks.len(), 3);
        assert!(
            album
                .items
                .iter()
                .any(|item| item.pending_download() == Some(pending_id.as_str()))
        );
        assert!(album.has_album_details());

        // Search results may carry an album key without a track origin.
        session
            .dispatch(
                BrowseAction::OpenDetailsInLibrary {
                    track: Box::new(Track {
                        parent_rating_key: saved.parent_rating_key.clone(),
                        ..Default::default()
                    }),
                    destination: Destination::Album,
                }
                .into(),
            )
            .await
            .unwrap();
        let document = crate::app::details::project(&session.state).unwrap();
        assert_eq!(document.title, "Saved Album");
        assert!(document.rows.contains(&("Year".into(), "2001".into())));
        assert!(document.rows.contains(&("Tracks".into(), "2".into())));

        session
            .dispatch(
                BrowseAction::OpenDetailsInLibrary {
                    track: Box::new(saved),
                    destination: Destination::Artist,
                }
                .into(),
            )
            .await
            .unwrap();
        let document = crate::app::details::project(&session.state).unwrap();
        assert_eq!(document.title, "Saved Artist");
        assert!(document.text.contains("Saved Album · 2 tracks"));
        assert!(!document.loading);
        assert!(document.similar.is_none());
    }

    #[tokio::test]
    async fn downloads_offline_mode_survives_network_changes() {
        let mut state = AppState::new();
        let (tx, _) = mpsc::channel(1);
        dispatch(Command::NetworkAvailable(false), &mut state, &tx)
            .await
            .unwrap();
        assert!(state.downloads.is_offline());
        dispatch(Command::Offline(true), &mut state, &tx)
            .await
            .unwrap();
        dispatch(Command::NetworkAvailable(true), &mut state, &tx)
            .await
            .unwrap();
        assert!(state.downloads.is_offline());
        dispatch(Command::Offline(false), &mut state, &tx)
            .await
            .unwrap();
        assert!(!state.downloads.is_offline());
    }

    #[test]
    fn offline_details_show_metadata_without_starting_discovery() {
        let mut state = AppState::new();
        state.downloads.manual_offline = true;
        let track = Track {
            rating_key: "song".into(),
            title: "Song".into(),
            parent_title: Some("Album".into()),
            ..Default::default()
        };
        state.artist_nav.columns = vec![BrowseColumn::new_with_tracks(
            "Tracks",
            BrowseItem::from_tracks(std::slice::from_ref(&track)),
            vec![track],
        )];
        state.track_pane_open = true;
        state.category_column_focused = false;
        let document = super::super::details::project(&state).unwrap();
        assert_eq!(document.title, "Song");
        assert_eq!(document.similar.unwrap().message, "Unavailable offline");
        assert!(suppress_offline(
            &DataAction::LoadTrackPaneSimilar {
                rating_key: "song".into()
            }
            .into(),
            &mut state
        ));
        assert!(state.track_pane_similar_loading.is_empty());
    }
}
