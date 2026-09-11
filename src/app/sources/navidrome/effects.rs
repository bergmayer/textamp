use super::*;
use crate::app::action::*;
use crate::app::event::{ArtworkEvent, PlaylistEvent};
use crate::library::track::TrackOrigin;

pub(super) mod browse;
pub(super) mod discovery;
mod media;
pub use media::{play, scrobble};

pub fn event(event: impl Into<Event>) -> Action {
    NavAction::Event(Box::new(event.into())).into()
}
pub fn failure(error: anyhow::Error) -> AsyncError {
    AsyncError {
        message: format!("{error:#}"),
    }
}

/// A finite set of independently replaceable effects. Replacement aborts the old
/// request; IDs also reject an already queued completion from that request.
pub fn spawn(
    state: &mut AppState,
    tx: &mpsc::Sender<Event>,
    slot: &'static str,
    work: impl std::future::Future<Output = anyhow::Result<Vec<Action>>> + Send + 'static,
) {
    spawn_scoped(state, tx, slot, |_, _| work);
}

/// Progress and completion messages must carry the same operation identity.
pub fn spawn_scoped<F, W>(
    state: &mut AppState,
    tx: &mpsc::Sender<Event>,
    slot: &'static str,
    work: F,
) where
    F: FnOnce(u64, u64) -> W,
    W: std::future::Future<Output = anyhow::Result<Vec<Action>>> + Send + 'static,
{
    if slot.ends_with("-write") && state.sources.nav_tasks.contains_key(slot) {
        state.set_status("A save is already in progress".into());
        return;
    }
    state.sources.nav_tasks.remove(slot);
    state.sources.nav_request_id = state.sources.nav_request_id.wrapping_add(1);
    let request_id = state.sources.nav_request_id;
    let generation = state.library_generation;
    let work = work(generation, request_id);
    let tx = tx.clone();
    let task = crate::app::tasks::spawn(async move {
        let result = work.await.map_err(|e| format!("{e:#}"));
        let _ = tx
            .send(Event::Effect(
                NavAction::Completed {
                    generation,
                    request_id,
                    slot,
                    result,
                }
                .into(),
            ))
            .await;
    });
    state
        .sources
        .nav_tasks
        .insert(slot, (request_id, TaskLease::new(&task)));
}

pub fn intercept(
    action: &Action,
    state: &mut AppState,
    tx: &mpsc::Sender<Event>,
) -> Option<Vec<Action>> {
    let session = state.sources.active.navidrome()?.clone();
    if let Action::Miller(MillerAction::LoadGenreAlbumsForMiller {
        genre_key,
        replace_child,
    }) = action
    {
        if matches!(
            state.sources.nav_collection,
            Some(commands::CollectionKind::AudioMuse(_))
        ) {
            return Some(match serde_json::from_str(genre_key) {
                Ok(filter) => vec![super::super::audiomuse::Command::Filter {
                    filter,
                    title: state
                        .artist_nav
                        .selected_item()
                        .map_or("Analysis", |i| i.title())
                        .into(),
                    replace_child: *replace_child,
                }
                .into()],
                Err(_) => vec![SystemAction::ShowError("Invalid AudioMuse filter".into()).into()],
            });
        }
    }
    if let Action::Navigation(NavigationAction::SetCategory {
        category,
        preserve_sections_focus,
    }) = action
    {
        if state.sources.nav_collection.is_some() {
            state.set_browse_category(*category, *preserve_sections_focus);
        }
    }
    if let Some(actions) = browse::intercept(action, state, tx, &session) {
        return Some(actions);
    }
    if let Some(actions) = discovery::intercept(action, state, tx, &session) {
        return Some(actions);
    }
    match action.clone() {
        Action::System(SystemAction::RefreshCategory(_)) => catalog::load(state, tx),
        Action::Data(
            DataAction::LoadInitialData | DataAction::LoadArtists | DataAction::LoadPlaylists,
        ) => catalog::open(state, tx),
        Action::System(SystemAction::CheckStaleness(_)) => catalog::check_staleness(state, tx),

        Action::System(SystemAction::LoadArtwork) => media::artwork(state, tx, &session),
        Action::Queue(
            action @ (QueueAction::RemixGemini
            | QueueAction::RemixTwofer
            | QueueAction::RemixStretch
            | QueueAction::RemixDoppelganger),
        ) => {
            super::dj::remix(state, tx, action.clone());
        }
        Action::System(SystemAction::LoadAlbumArt(batch)) => {
            media::art_batch(state, tx, &session, batch)
        }
        Action::Queue(
            QueueAction::PlayAlbum { rating_key } | QueueAction::PlayAlbumNow { rating_key, .. },
        ) => {
            state.queue_play_request_id = state.queue_play_request_id.wrapping_add(1);
            let intent = QueueLoadIntent::ReplaceAndPlay {
                request_id: state.queue_play_request_id,
                label: None,
            };
            load_album(state, tx, session, rating_key, intent);
        }
        Action::Queue(QueueAction::PlayArtistTracks { artist_key }) => {
            state.queue_play_request_id = state.queue_play_request_id.wrapping_add(1);
            let intent = QueueLoadIntent::ReplaceAndPlay {
                request_id: state.queue_play_request_id,
                label: None,
            };
            load_artist(state, tx, session, artist_key, intent);
        }
        Action::Queue(QueueAction::EnqueueAlbum { rating_key, title }) => {
            load_album(
                state,
                tx,
                session,
                rating_key,
                QueueLoadIntent::Append { label: title },
            );
        }
        Action::Queue(QueueAction::EnqueueArtistTracks {
            artist_key,
            artist_name,
        }) => {
            load_artist(
                state,
                tx,
                session,
                artist_key,
                QueueLoadIntent::Append { label: artist_name },
            );
        }
        Action::Queue(QueueAction::EnqueueAlbumNext { rating_key, title }) => {
            load_album(
                state,
                tx,
                session,
                rating_key,
                QueueLoadIntent::InsertNext { label: title },
            );
        }
        Action::Queue(QueueAction::EnqueueArtistTracksNext {
            artist_key,
            artist_name,
        }) => {
            load_artist(
                state,
                tx,
                session,
                artist_key,
                QueueLoadIntent::InsertNext { label: artist_name },
            );
        }
        Action::Queue(QueueAction::PlayPlaylistNow {
            playlist_key,
            title,
        }) => {
            state.queue_play_request_id = state.queue_play_request_id.wrapping_add(1);
            let request_id = state.queue_play_request_id;
            spawn(state, tx, "playlist-play", async move {
                let result = async {
                    let playlist = session.client.playlist(&session.id(&playlist_key)?).await?;
                    Ok(playlist
                        .entry
                        .into_iter()
                        .map(|t| session.track(t))
                        .collect())
                }
                .await
                .map_err(failure);
                Ok(vec![QueueAction::TracksLoaded {
                    intent: QueueLoadIntent::ReplaceAndPlay {
                        request_id,
                        label: Some(title),
                    },
                    result,
                }
                .into()])
            });
        }
        Action::Queue(QueueAction::SaveQueueAsPlaylist(name)) => {
            let tracks = state.playback_tracks().to_vec();
            spawn(state, tx, "playlist-write", async move {
                let result = async {
                    anyhow::ensure!(!tracks.is_empty(), "No tracks to save");
                    anyhow::ensure!(!name.trim().is_empty(), "Playlist name cannot be empty");
                    let mut params = vec![("name", name.clone())];
                    for track in &tracks {
                        params.push(("songId", session.id(&track.rating_key)?));
                    }
                    session.client.call("createPlaylist", &params).await?;
                    Ok(())
                }
                .await
                .map_err(failure);
                Ok(vec![QueueAction::QueuePlaylistSaved {
                    name,
                    track_count: tracks.len(),
                    result,
                }
                .into()])
            });
        }

        _ => return super::super::routing::navidrome_fallback(action),
    }
    Some(vec![])
}
fn load_album(
    state: &mut AppState,
    tx: &mpsc::Sender<Event>,
    session: Session,
    key: String,
    intent: QueueLoadIntent,
) {
    // Queue requests use authoritative album contents, including albums reached
    // through an account-wide playlist outside the selected music folder.
    load_queue(state, tx, intent, async move {
        session.album_tracks(&key).await
    });
}

fn load_artist(
    state: &mut AppState,
    tx: &mpsc::Sender<Event>,
    session: Session,
    key: String,
    intent: QueueLoadIntent,
) {
    let tracks = browse::artist_tracks(state, &key);
    load_queue(state, tx, intent, async move {
        if tracks.is_empty() {
            session.artist_tracks(&key).await
        } else {
            Ok(tracks)
        }
    });
}

fn load_queue(
    state: &mut AppState,
    tx: &mpsc::Sender<Event>,
    intent: QueueLoadIntent,
    work: impl std::future::Future<Output = anyhow::Result<Vec<Track>>> + Send + 'static,
) {
    let slot = if matches!(intent, QueueLoadIntent::ReplaceAndPlay { .. }) {
        "queue-play"
    } else {
        "queue-write"
    };
    spawn(state, tx, slot, async move {
        Ok(vec![QueueAction::TracksLoaded {
            intent,
            result: work.await.map_err(failure),
        }
        .into()])
    });
}
