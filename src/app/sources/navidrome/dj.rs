//! Native Navidrome DJ and remix candidates, applied by the shared queue reducer.
use super::effects::{failure, spawn};
use super::recommendations::Recommendations;
use super::*;
use crate::app::action::*;
use crate::app::state::DjMode;
use futures::{stream, StreamExt};
use rand::prelude::IteratorRandom;
use std::collections::HashSet;

/// Missing IDs must not make two unknown artists look identical.
fn same_artist(a: &Track, b: &Track) -> bool {
    match (&a.grandparent_rating_key, &b.grandparent_rating_key) {
        (Some(a), Some(b)) => a == b,
        _ => {
            let a = a.track_artist().trim();
            let b = b.track_artist().trim();
            !a.is_empty() && a != "Unknown Artist" && a.eq_ignore_ascii_case(b)
        }
    }
}

fn local_candidates(
    state: &AppState,
    seed: &Track,
    mode: DjMode,
    excluded: &HashSet<String>,
) -> Vec<Track> {
    if !matches!(mode, DjMode::Twofer | DjMode::Contempo | DjMode::Groupie) {
        return vec![];
    }
    let mut rng = rand::rng();
    let year = seed.year.or(seed.parent_year);
    state
        .library
        .all_tracks
        .iter()
        .filter(|t| {
            t.rating_key != seed.rating_key
                && !excluded.contains(&t.rating_key)
                && match mode {
                    DjMode::Contempo => year.is_some_and(|y| {
                        y > 0 && t.year.or(t.parent_year).is_some_and(|v| v / 10 == y / 10)
                    }),
                    _ => same_artist(seed, t),
                }
        })
        .choose_multiple(&mut rng, 20)
        .into_iter()
        .cloned()
        .collect()
}

async fn candidates(
    api: &Recommendations,
    mode: DjMode,
    seed: &Track,
    next: Option<&Track>,
    local: Vec<Track>,
) -> anyhow::Result<Vec<Track>> {
    let session = &api.session;
    match mode {
        DjMode::Twofer => {
            if next.is_some_and(|n| same_artist(seed, n)) {
                Ok(vec![])
            } else {
                Ok(local)
            }
        }
        DjMode::Contempo => Ok(local),
        DjMode::Groupie => {
            let Some(artist) = &seed.grandparent_rating_key else {
                return Ok(local);
            };
            let related = api
                .artist(&session.id(artist)?, Some(&session.id(&seed.rating_key)?))
                .await;
            let mut tracks = match related {
                Ok(tracks) => tracks,
                Err(error) if !local.is_empty() => {
                    tracing::warn!("DJ Groupie: related discovery failed; using the artist's cached tracks: {error:#}");
                    vec![]
                }
                Err(error) => return Err(error),
            };
            tracks.extend(local);
            Ok(tracks)
        }
        DjMode::Stretch => {
            let Some(next) = next else {
                return Ok(vec![]);
            };
            let start_id = session.id(&seed.rating_key)?;
            let end_id = session.id(&next.rating_key)?;
            let tracks = api.path(&start_id, &end_id, 5).await?;
            Ok(tracks
                .into_iter()
                .filter(|t| t.rating_key != seed.rating_key && t.rating_key != next.rating_key)
                .collect())
        }
        DjMode::Gemini | DjMode::Freeze => api.similar(&session.id(&seed.rating_key)?, true).await,
    }
}

pub fn process(state: &mut AppState, tx: &mpsc::Sender<Event>) {
    let Some(mode) = state.dj.active_mode else {
        return;
    };
    if mode.is_interleaving() && state.dj.last_was_inserted {
        state.dj.last_was_inserted = false;
        return;
    }
    let Some(seed) = state.current_track().cloned() else {
        state.dj.inserting = false;
        return;
    };
    let next = state
        .queue
        .tracks
        .get(state.queue.index.unwrap_or(0) + 1)
        .cloned();
    let mut seen: HashSet<_> = state.dj.history.iter().cloned().collect();
    seen.insert(seed.rating_key.clone());
    if let Some(next) = &next {
        seen.insert(next.rating_key.clone());
    }
    let local = local_candidates(state, &seed, mode, &seen);
    let session = state.sources.active.navidrome().unwrap().clone();
    let api = Recommendations::new(state, &session);
    state.dj.inserting = true;
    let playback_id = state.playback.request_id;
    spawn(state, tx, "dj", async move {
        let result = candidates(&api, mode, &seed, next.as_ref(), local)
            .await
            .map(|tracks| {
                tracks
                    .into_iter()
                    .filter(|t| seen.insert(t.rating_key.clone()))
                    .take(mode.insert_count())
                    .collect()
            })
            .map_err(failure);
        Ok(vec![NavAction::DjReady {
            playback_id,
            track: seed.rating_key,
            mode,
            result,
        }
        .into()])
    });
}

pub fn remix(state: &mut AppState, tx: &mpsc::Sender<Event>, action: QueueAction) {
    let session = state.sources.active.navidrome().unwrap().clone();
    let mode = match action {
        QueueAction::RemixTwofer => DjMode::Twofer,
        QueueAction::RemixStretch => DjMode::Stretch,
        _ => DjMode::Gemini,
    };
    if mode != DjMode::Twofer && !super::recommendations::available(state) {
        state.set_error(
            "This remix needs Navidrome sonic analysis; enable AudioMuse and refresh with F5"
                .into(),
        );
        return;
    }
    if state.playback_tracks().len() < if mode == DjMode::Stretch { 2 } else { 1 } {
        state.set_error("Add tracks to the queue; Stretch needs at least two".into());
        return;
    }
    let name = match action {
        QueueAction::RemixTwofer => "Twofer",
        QueueAction::RemixStretch => "Stretch",
        QueueAction::RemixDoppelganger => "Doppelganger",
        _ => "Gemini",
    };
    if state.playback_mode == crate::app::state::PlaybackMode::Radio {
        let snapshot = state.convert_radio_to_queue(&format!("Remix: {name} (from radio)"));
        state.queue.undo_snapshot = Some(snapshot);
    } else if state.queue.undo_snapshot.is_none() {
        state.queue.undo_snapshot = Some(crate::app::state::QueueSnapshot {
            contents: crate::app::state::QueueContents::Queue {
                tracks: state.queue.tracks.clone(),
                index: state.queue.index,
            },
            description: format!("Remix: {name}"),
        });
    }
    state.set_status(format!("Remix: {name} processing..."));
    let api = Recommendations::new(state, &session);
    let replacing = matches!(action, QueueAction::RemixDoppelganger);
    let expected: Vec<_> = state
        .queue
        .tracks
        .iter()
        .map(|t| t.rating_key.clone())
        .collect();
    let base = state.queue.index.unwrap_or(0).min(state.queue.tracks.len());
    let index = state.queue.index;
    let tracks = state.queue.tracks[base..].to_vec();
    let excluded = expected.iter().cloned().collect();
    let locals: Vec<_> = tracks
        .iter()
        .map(|t| local_candidates(state, t, mode, &excluded))
        .collect();
    spawn(state, tx, "remix", async move {
        let mut seen: HashSet<_> = expected.iter().cloned().collect();
        let mut outcome = AsyncBatchOutcome::new(Vec::new(), tracks.len());
        // Bounded concurrency avoids serial 30-second waits for every seed.
        // Buffered ordering preserves queue order even when requests finish out of order.
        let jobs: Vec<_> = tracks
            .iter()
            .cloned()
            .zip(locals)
            .enumerate()
            .filter(|(i, _)| mode != DjMode::Stretch || i + 1 < tracks.len())
            .map(|(i, (seed, local))| {
                let next = if mode == DjMode::Twofer {
                    None
                } else {
                    tracks.get(i + 1).cloned()
                };
                (i, seed, next, local)
            })
            .collect();
        outcome.attempted = jobs.len();
        let api = std::sync::Arc::new(api);
        let mut requests = stream::iter(jobs)
            .map(move |(i, seed, next, local)| {
                let api = api.clone();
                async move {
                    let result = candidates(&api, mode, &seed, next.as_ref(), local).await;
                    (i, seed, result)
                }
            })
            .buffered(4);
        while let Some((i, seed, result)) = requests.next().await {
            match result {
                Ok(candidates) => {
                    let picked: Vec<_> = candidates
                        .into_iter()
                        .filter(|t| {
                            (!replacing || !same_artist(t, &seed))
                                && seen.insert(t.rating_key.clone())
                        })
                        .take(if mode == DjMode::Stretch { 3 } else { 1 })
                        .collect();
                    if !picked.is_empty() {
                        outcome.items.push((base + i, picked));
                    }
                }
                Err(e) => {
                    outcome.failed += 1;
                    if outcome.first_error.is_none() {
                        outcome.first_error = Some(failure(e));
                    }
                }
            }
        }
        let result = if replacing {
            QueueAction::RemixDoppelgangerReady(AsyncBatchOutcome {
                items: outcome
                    .items
                    .into_iter()
                    .filter_map(|(i, ts)| ts.into_iter().next().map(|t| (i, t)))
                    .collect(),
                attempted: outcome.attempted,
                failed: outcome.failed,
                first_error: outcome.first_error,
            })
        } else {
            QueueAction::RemixBatchReady(outcome)
        };
        Ok(vec![NavAction::QueueEdit {
            index,
            expected,
            action: Box::new(result.into()),
        }
        .into()])
    });
}
