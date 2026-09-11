//! Generation-bound folder browsing and media preparation.
use super::*;

#[derive(Debug, Clone)]
pub enum FileAction {
    Listed {
        generation: u64,
        id: u64,
        path: String,
        column: usize,
        refreshing: bool,
        result: Result<Vec<FolderEntry>, String>,
    },
    Prepared {
        generation: u64,
        id: u64,
        track: Box<Track>,
        artwork: Option<Vec<u8>>,
        result: Result<MediaFile, String>,
    },
}

impl From<FileAction> for SourceAction {
    fn from(value: FileAction) -> Self {
        Self::Files(value)
    }
}
impl From<FileAction> for Action {
    fn from(value: FileAction) -> Self {
        Self::Source(value.into())
    }
}

pub(super) async fn dispatch(
    action: FileAction,
    state: &mut AppState,
    audio: &mut AudioPlayer,
) -> anyhow::Result<Vec<Action>> {
    match action {
        FileAction::Listed {
            generation,
            id,
            path,
            column,
            refreshing,
            result,
        } => {
            if generation != state.library_generation || id != state.sources.list_id {
                return Ok(vec![]);
            }
            if !refreshing {
                state.sources.listing = None;
            }
            state.library_loading = refreshing;
            if let Some(nav) = &mut state.folder_state {
                // Existing columns stay visible while refreshed in the background.
                nav.loading = false;
            }
            if column > 0
                && (state.category_column_focused
                    || state.folder_state.as_ref().is_none_or(|nav| {
                        nav.columns
                            .get(column)
                            .is_none_or(|c| c.key.as_deref() != Some(&path))
                            && nav
                                .columns
                                .get(column - 1)
                                .and_then(|c| c.selected_item())
                                .is_none_or(|i| i.key != path)
                    }))
            {
                return Ok(vec![]);
            }
            match result {
                Err(error) => {
                    state.cache_mgmt.failures.insert(
                        crate::app::state::RefreshCategory::Folders,
                        crate::app::state::RefreshFailure {
                            attempts: 1,
                            retry_at: None,
                        },
                    );
                    state.set_error(format!("Load folder: {error}"));
                }
                Ok(entries) => {
                    if !refreshing {
                        state
                            .cache_mgmt
                            .failures
                            .remove(&crate::app::state::RefreshCategory::Folders);
                    }
                    let Some(source_id) = state.sources.active.folder() else {
                        return Ok(vec![]);
                    };
                    let items = entries
                        .into_iter()
                        .map(|entry| {
                            let name = crate::util::sanitize_display_text(&entry.name).into_owned();
                            if entry.directory {
                                FolderItem::folder(entry.path, name)
                            } else {
                                let track = Track::from_folder(source_id, &entry.path);
                                FolderItem::track(
                                    entry.path,
                                    name,
                                    track.rating_key,
                                    None,
                                    None,
                                    None,
                                )
                            }
                        })
                        .collect();
                    let title = if path.is_empty() {
                        state
                            .sources
                            .folders
                            .iter()
                            .find(|s| &s.id == source_id)
                            .map(|s| s.name.clone())
                            .unwrap_or_default()
                    } else {
                        path.rsplit('/').next().unwrap_or(&path).into()
                    };
                    let mut next =
                        FolderColumn::new((!path.is_empty()).then_some(path), title, items);
                    let nav = state.folder_state.get_or_insert_with(|| {
                        FolderNavigationState::for_library(format!("folder:{source_id}"))
                    });
                    if let Some(old) = nav.columns.get(column).filter(|old| old.key == next.key) {
                        if let Some(selected) = old.selected_item() {
                            next.selected_index = next
                                .items
                                .iter()
                                .position(|i| i.key == selected.key)
                                .unwrap_or(0);
                        }
                        // Updating an already-open folder must not change focus
                        // or close a child the user opened during the request.
                        nav.columns[column] = next;
                    } else {
                        nav.columns.truncate(column);
                        nav.columns.push(next);
                        nav.focused_column = column;
                    }
                }
            }
        }
        FileAction::Prepared {
            generation,
            id,
            track,
            artwork,
            result,
        } => {
            if generation != state.library_generation
                || id != state.playback.preparation_id
                || state
                    .current_track()
                    .is_none_or(|t| t.rating_key != track.rating_key)
            {
                return Ok(vec![]);
            }
            state.sources.preparing = None;
            match result {
                Err(error) => {
                    state.playback.status = PlayStatus::Stopped;
                    state.set_error(format!("Play file: {error}"));
                }
                Ok(file) => {
                    let paused = state.playback.status == PlayStatus::Paused;
                    state.artwork.current_thumb = Some(track.rating_key.clone());
                    state.artwork.current_data = artwork;
                    state.playback.duration_ms = track.duration_ms();
                    if let Some(slot) = state.current_track_mut() {
                        *slot = *track;
                    }
                    if let Err(error) = audio.play_file(file.clone()) {
                        state.playback.status = PlayStatus::Stopped;
                        state.set_error(format!("Play file: {error}"));
                    } else {
                        state.sources.prepared = Some(file);
                        state.playback.request_id = audio.playback_id();
                        state.playback.status = if paused {
                            audio.pause();
                            PlayStatus::Paused
                        } else {
                            PlayStatus::Playing
                        };
                        state.playback.playback_started_at = Some(std::time::Instant::now());
                    }
                }
            }
        }
    }
    Ok(vec![])
}

pub(super) fn load(state: &mut AppState, tx: &mpsc::Sender<Event>, path: String, column: usize) {
    load_folder(state, tx, path, column, false);
}

fn load_folder(
    state: &mut AppState,
    tx: &mpsc::Sender<Event>,
    path: String,
    column: usize,
    force: bool,
) {
    let Some(source) = state
        .sources
        .folders
        .iter()
        .find(|s| Some(&s.id) == state.sources.active.folder())
        .cloned()
    else {
        return;
    };
    if path.is_empty() {
        super::cache::start(LibraryChoice::Folder(source.clone()), false, state, tx);
    }
    state.sources.listing = None;
    state.sources.list_id = state.sources.list_id.wrapping_add(1);
    let id = state.sources.list_id;
    let generation = state.library_generation;
    state.library_loading = true;
    if let Some(nav) = &mut state.folder_state {
        nav.loading = nav.columns.is_empty();
    }
    let tx = tx.clone();
    let store = crate::library::cache::Store::folder(&source);
    let ticket = store
        .as_ref()
        .map(|store| store.ticket(&path))
        .map_err(|e| e.to_string());
    let task = crate::app::tasks::spawn(async move {
        let ticket = match ticket {
            Ok(ticket) => Some(ticket),
            Err(e) => {
                tracing::warn!("Folder cache: {e}");
                None
            }
        };
        if !force {
            if let Some(ticket) = ticket.clone() {
                let read_path = path.clone();
                match crate::app::tasks::spawn_blocking(move || {
                    let _ = ticket;
                    super::cache::read_folder(&store.map_err(anyhow::Error::msg)?, &read_path)
                })
                .await
                {
                    Ok(Ok(Some(hit))) => {
                        let refreshing = hit.stale;
                        if tx
                            .send(Event::Effect(
                                FileAction::Listed {
                                    generation,
                                    id,
                                    path: path.clone(),
                                    column,
                                    refreshing,
                                    result: Ok(hit.value),
                                }
                                .into(),
                            ))
                            .await
                            .is_err()
                            || !refreshing
                        {
                            return;
                        }
                    }
                    Ok(Ok(None)) => {}
                    other => tracing::warn!(
                        "Folder cache unavailable: {}",
                        match other {
                            Ok(Err(e)) => e.to_string(),
                            Err(e) => e.to_string(),
                            _ => unreachable!(),
                        }
                    ),
                }
            }
        }
        let result = source.list(&path).await.map_err(|e| format!("{e:#}"));
        if let (Some(ticket), Ok(entries)) = (ticket, &result) {
            let entries = entries.clone();
            let saved = crate::app::tasks::spawn_blocking(move || ticket.write(&entries)).await;
            if let Err(error) = saved
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| e.to_string()))
            {
                tracing::warn!("Folder cache write: {error}");
                let _ = tx
                    .send(Event::Effect(
                        super::super::action::SystemAction::ShowError(format!(
                            "Save folder cache: {error}"
                        ))
                        .into(),
                    ))
                    .await;
            }
        }
        let _ = tx
            .send(Event::Effect(Action::from(FileAction::Listed {
                generation,
                id,
                path,
                column,
                refreshing: false,
                result,
            })))
            .await;
    });
    state.sources.listing = Some(TaskLease::new(&task));
}

pub fn folders(
    action: FolderAction,
    state: &mut AppState,

    audio: &mut AudioPlayer,
    tx: &mpsc::Sender<Event>,
) {
    match action {
        FolderAction::LoadFolderRoot => load(state, tx, String::new(), 0),
        FolderAction::NavigateIntoFolder { folder_key, .. } => {
            let column = state
                .folder_state
                .as_ref()
                .map_or(0, |nav| nav.focused_column + 1);
            load(state, tx, folder_key, column);
        }
        FolderAction::RefreshSubfolder(path) => {
            let column = state
                .folder_state
                .as_ref()
                .and_then(|nav| {
                    nav.columns
                        .iter()
                        .position(|c| c.key.as_deref().unwrap_or("") == path)
                })
                .unwrap_or(0);
            load_folder(state, tx, path, column, true);
        }
        FolderAction::PlayFolderTracks | FolderAction::PlayFolderTrack { .. } => {
            if let Some(column) = state.folder_state.as_ref().and_then(|nav| nav.focused()) {
                let source = state
                    .sources
                    .active
                    .folder()
                    .map(String::as_str)
                    .unwrap_or_default();
                let tracks: Vec<_> = column
                    .items
                    .iter()
                    .filter(|i| i.is_track())
                    .map(|i| Track::from_folder(source, &i.key))
                    .collect();
                let selected = match action {
                    FolderAction::PlayFolderTrack { track_index } => column.items.get(track_index),
                    _ => column.selected_item(),
                };
                let index = selected
                    .and_then(|i| {
                        tracks
                            .iter()
                            .position(|t| Some(&t.rating_key) == i.rating_key.as_ref())
                    })
                    .unwrap_or(0);
                if tracks.is_empty() {
                    state.set_status("No audio files in this folder".into());
                } else {
                    helpers::queue_and_play(tx, state, audio, tracks, index);
                }
            }
        }
        _ => {} // Catalog completion events cannot apply to file sources.
    }
}

pub fn play(
    state: &mut AppState,
    audio: &mut AudioPlayer,
    tx: &mpsc::Sender<Event>,
    mut track: Track,
) {
    let crate::library::track::TrackOrigin::Folder { source_id, path } = track.origin.clone()
    else {
        return;
    };
    let Some(source) = state
        .sources
        .folders
        .iter()
        .find(|s| s.id == source_id)
        .cloned()
    else {
        state.set_error("Library has been removed".into());
        return;
    };
    state.sources.preparing = None;
    state.sources.prepared = None;
    audio.stop();
    state.playback.request_id = audio.playback_id();
    state.playback.preparation_id = state.playback.preparation_id.wrapping_add(1);
    state.playback.status = PlayStatus::Buffering;
    state.playback.position_ms = 0;
    state.playback.duration_ms = track.duration_ms();
    state.waveform = Default::default();
    state.spectrogram = Default::default();
    state.artwork.current_data = None;
    state.artwork.current_thumb = None;
    let id = state.playback.preparation_id;
    let generation = state.library_generation;
    let tx = tx.clone();
    let task = crate::app::tasks::spawn(async move {
        let result = source.file(&path).await;
        let failed_track = track.clone();
        let (track, artwork, result) = match result {
            Ok(file) => crate::app::tasks::spawn_blocking(move || {
                let artwork = match track.read_file_metadata(&file.path) {
                    Ok(artwork) => artwork,
                    Err(error) => {
                        tracing::debug!("Audio tags unavailable: {error}");
                        None
                    }
                };
                (track, artwork, Ok(file))
            })
            .await
            .unwrap_or_else(|error| (failed_track, None, Err(error.to_string()))),
            Err(error) => (track, None, Err(format!("{error:#}"))),
        };
        let _ = tx
            .send(Event::Effect(Action::from(FileAction::Prepared {
                generation,
                id,
                track: Box::new(track),
                artwork,
                result,
            })))
            .await;
    });
    state.sources.preparing = Some(TaskLease::new(&task));
}
