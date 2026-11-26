//! One-time Navidrome remix candidates, applied by the shared queue reducer.
use super::effects::{failure, spawn};
use super::recommendations::Recommendations;
use super::*;
use crate::app::action::*;
use crate::audiomuse::ranking::{self, Mode, Query, Rankings};
use anyhow::{Result, ensure};
#[derive(Clone, Copy, PartialEq, Eq)]
enum RemixKind {
    Twins,
    Twofer,
    Stretch,
}
use futures::{StreamExt, stream};
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

// Keep both IDs and normalized metadata: older caches may omit IDs, and
// multiple editions of an album must not bypass the album exclusion.
fn album_keys(track: &Track) -> impl Iterator<Item = String> {
    let id = track
        .parent_rating_key
        .as_deref()
        .filter(|id| !id.is_empty())
        .map(|id| format!("id:{id}"));
    let title = track.album_name().trim();
    let name = (!title.is_empty() && title != "Unknown Album").then(|| {
        format!(
            "name:{}\0{}",
            title.to_lowercase(),
            track.artist_name().trim().to_lowercase()
        )
    });
    [id, name].into_iter().flatten()
}
fn artist_keys(track: &Track) -> impl Iterator<Item = String> {
    let id = track
        .grandparent_rating_key
        .as_deref()
        .filter(|id| !id.is_empty())
        .map(|id| format!("id:{id}"));
    let artist = track.track_artist().trim();
    let name = (!artist.is_empty() && artist != "Unknown Artist")
        .then(|| format!("name:{}", artist.to_lowercase()));
    [id, name].into_iter().flatten()
}

fn local_candidates(
    state: &AppState,
    seed: &Track,
    mode: RemixKind,
    excluded: &HashSet<String>,
) -> Vec<Track> {
    if mode != RemixKind::Twofer {
        return vec![];
    }
    fn sample<'a>(
        tracks: impl Iterator<Item = &'a Track>,
        seed: &Track,
        excluded: &HashSet<String>,
    ) -> Vec<Track> {
        tracks
            .filter(|track| {
                track.rating_key != seed.rating_key
                    && !excluded.contains(&track.rating_key)
                    && same_artist(seed, track)
            })
            .choose_multiple(&mut rand::rng(), 20)
            .into_iter()
            .cloned()
            .collect()
    }
    let indexed = (state.library.track_lookup.len == state.library.all_tracks.len())
        .then(|| {
            seed.grandparent_rating_key
                .as_ref()
                .and_then(|key| state.library.track_lookup.artists.get(key))
        })
        .flatten();
    match indexed {
        Some(indices) => sample(
            indices
                .iter()
                .filter_map(|index| state.library.all_tracks.get(*index)),
            seed,
            excluded,
        ),
        None => sample(state.library.all_tracks.iter(), seed, excluded),
    }
}

async fn candidates(
    api: &Recommendations,
    mode: RemixKind,
    seed: &Track,
    next: Option<&Track>,
    local: Vec<Track>,
) -> anyhow::Result<Vec<Track>> {
    let session = &api.session;
    match mode {
        RemixKind::Twofer => {
            if next.is_some_and(|n| same_artist(seed, n)) {
                Ok(vec![])
            } else {
                Ok(local)
            }
        }
        RemixKind::Stretch => {
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
        RemixKind::Twins => api.similar(&session.id(&seed.rating_key)?, true).await,
    }
}

const SONIC_QUEUE_LIMIT: usize = 64;

pub fn remix(state: &mut AppState, tx: &mpsc::Sender<Event>, action: QueueAction) {
    let Some(session) = state.sources.active.navidrome().cloned() else {
        state.set_status("Choose a server library to edit this queue".into());
        return;
    };
    if let Some(mode) = match action {
        QueueAction::RemixOppositeDay => Some(Mode::WholeQueue),
        QueueAction::RemixEnemies => Some(Mode::PerTrack),
        _ => None,
    } {
        ranked_remix(state, tx, &session, mode);
        return;
    }
    let mode = match action {
        QueueAction::RemixTwofer => RemixKind::Twofer,
        QueueAction::RemixStretch => RemixKind::Stretch,
        _ => RemixKind::Twins,
    };
    if mode != RemixKind::Twofer && !super::recommendations::available(state) {
        state.set_error(
            "This remix needs Navidrome sonic analysis; enable AudioMuse and choose Refresh Library"
                .into(),
        );
        return;
    }
    if state.playback_tracks().len() < if mode == RemixKind::Stretch { 2 } else { 1 } {
        state.set_error("Add tracks to the queue; Stretch needs at least two".into());
        return;
    }
    let name = match action {
        QueueAction::RemixTwofer => "Twofer",
        QueueAction::RemixStretch => "Stretch",
        QueueAction::RemixDoppelganger => "Doppelganger",
        _ => "Twins",
    };
    state.set_status(format!("Remix: {name} processing..."));
    let api = Recommendations::new(state, &session);
    let replacing = matches!(action, QueueAction::RemixDoppelganger);
    let expected: Vec<_> = state
        .playback_tracks()
        .iter()
        .map(|t| t.rating_key.clone())
        .collect();
    let queue = state.playback_tracks();
    let tracks = if mode == RemixKind::Twofer {
        queue
    } else {
        &queue[..queue.len().min(SONIC_QUEUE_LIMIT)]
    }
    .to_vec();
    let excluded = expected.iter().cloned().collect();
    let allow_same_artist = mode == RemixKind::Twofer || state.remix_allow_same_artist;
    let mut excluded_albums: HashSet<_> = tracks.iter().flat_map(album_keys).collect();
    let mut excluded_artists: HashSet<_> = tracks.iter().flat_map(artist_keys).collect();
    let locals: Vec<_> = tracks
        .iter()
        .map(|t| local_candidates(state, t, mode, &excluded))
        .collect();
    let work = async move {
        let mut seen: HashSet<_> = expected.iter().cloned().collect();
        let mut outcome = AsyncBatchOutcome::new(Vec::new(), tracks.len());
        // Bounded concurrency avoids serial 30-second waits for every seed.
        // Buffered ordering preserves queue order even when requests finish out of order.
        let jobs: Vec<_> = tracks
            .iter()
            .cloned()
            .zip(locals)
            .enumerate()
            .filter(|(i, _)| mode != RemixKind::Stretch || i + 1 < tracks.len())
            .map(|(i, (seed, local))| {
                let next = if mode == RemixKind::Twofer {
                    None
                } else {
                    tracks.get(i + 1).cloned()
                };
                (i, seed, next, local)
            })
            .collect();
        outcome.attempted = jobs.len();
        let api = std::sync::Arc::new(api);
        for key in &expected {
            api.session.id(key)?;
        }
        let mut requests = stream::iter(jobs)
            .map(move |(i, seed, next, local)| {
                let api = api.clone();
                async move {
                    let result = candidates(&api, mode, &seed, next.as_ref(), local).await;
                    (i, result)
                }
            })
            .buffered(4);
        while let Some((i, result)) = requests.next().await {
            match result {
                Ok(candidates) => {
                    let mut picked = Vec::new();
                    for candidate in candidates {
                        let albums: Vec<_> = album_keys(&candidate).collect();
                        let artists: Vec<_> = artist_keys(&candidate).collect();
                        if albums.is_empty()
                            || albums.iter().any(|key| excluded_albums.contains(key))
                            || (!allow_same_artist
                                && artists.iter().any(|key| excluded_artists.contains(key)))
                            || !seen.insert(candidate.rating_key.clone())
                        {
                            continue;
                        }
                        excluded_albums.extend(albums);
                        excluded_artists.extend(artists);
                        picked.push(candidate);
                        if picked.len() == if mode == RemixKind::Stretch { 3 } else { 1 } {
                            break;
                        }
                    }
                    if !picked.is_empty() {
                        outcome.items.push((i, picked));
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
            let outcome = AsyncBatchOutcome {
                items: outcome
                    .items
                    .into_iter()
                    .filter_map(|(i, ts)| ts.into_iter().next().map(|t| (i, t)))
                    .collect(),
                attempted: outcome.attempted,
                failed: outcome.failed,
                first_error: outcome.first_error,
            };
            QueueAction::RemixDoppelgangerReady(outcome)
        } else {
            QueueAction::RemixBatchReady(outcome)
        };
        Ok(vec![
            NavAction::QueueEdit {
                expected,
                action: Box::new(result.into()),
            }
            .into(),
        ])
    };
    spawn(state, tx, "remix", async move {
        tokio::time::timeout(std::time::Duration::from_secs(90), work)
            .await
            .map_err(|_| anyhow::anyhow!("Queue edit timed out; the queue was not changed. Try again or choose another edit."))?
    });
}

fn ranked_remix(state: &mut AppState, tx: &mpsc::Sender<Event>, session: &Session, mode: Mode) {
    if !crate::app::sources::sonic::enabled(state)
        || crate::app::sources::audiomuse::connection(state).is_none()
    {
        state.set_error("Connect AudioMuse to rank distant tracks".into());
        return;
    }
    let expected: Vec<_> = state
        .playback_tracks()
        .iter()
        .map(|track| track.rating_key.clone())
        .collect();
    if expected.is_empty() {
        state.set_error("Add tracks to the queue first".into());
        return;
    }
    let api = Recommendations::new(state, session);
    spawn(state, tx, "remix", async move {
        let result = tokio::time::timeout(std::time::Duration::from_secs(120), async {
            let ids = expected
                .iter()
                .take(SONIC_QUEUE_LIMIT)
                .map(|key| api.session.id(key))
                .collect::<Result<Vec<_>>>()?;
            let outcome = rank_queue(&api, mode, &ids).await?;
            Ok::<_, anyhow::Error>(match mode {
                Mode::WholeQueue => QueueAction::RemixOppositeDayReady(outcome),
                Mode::PerTrack => QueueAction::RemixEnemiesReady(AsyncBatchOutcome {
                    items: outcome
                        .items
                        .into_iter()
                        .map(|(i, track)| (i, vec![track]))
                        .collect(),
                    attempted: outcome.attempted,
                    failed: outcome.failed,
                    first_error: outcome.first_error,
                }),
            })
        })
        .await
        .map_err(|_| {
            anyhow::anyhow!("AudioMuse ranking timed out. Try again after the index is ready.")
        })??;
        Ok(vec![
            NavAction::QueueEdit {
                expected,
                action: Box::new(result.into()),
            }
            .into(),
        ])
    });
}

pub(super) async fn rank_queue(
    api: &Recommendations,
    mode: Mode,
    ids: &[String],
) -> Result<AsyncBatchOutcome<Vec<(usize, Track)>>> {
    match rank_snapshot(api, mode, ids).await {
        Err(error) if error.is::<ranking::IndexChanged>() => rank_snapshot(api, mode, ids).await,
        result => result,
    }
}

async fn rank_snapshot(
    api: &Recommendations,
    mode: Mode,
    ids: &[String],
) -> Result<AsyncBatchOutcome<Vec<(usize, Track)>>> {
    ensure!(
        mode != Mode::WholeQueue || ids.len() <= ranking::WHOLE_QUEUE_LIMIT,
        "Opposite Day supports up to {} queued tracks",
        ranking::WHOLE_QUEUE_LIMIT
    );
    let limit = match mode {
        Mode::WholeQueue => ranking::WHOLE_QUEUE_LIMIT,
        Mode::PerTrack => ranking::PER_TRACK_LIMIT,
    };
    let mut seen: HashSet<_> = ids.iter().cloned().collect();
    let exclude: Vec<_> = if ids.len() > limit {
        let mut exclude: Vec<_> = seen.iter().cloned().collect();
        exclude.sort_unstable();
        ensure!(
            exclude.len() <= 10_000,
            "Enemies supports up to 10,000 distinct queued tracks"
        );
        exclude
    } else {
        Vec::new()
    };
    let client = api.direct().await?;
    let mut generation = None;
    let mut outcome = AsyncBatchOutcome::new(Vec::new(), ids.len());
    let mut select = |candidate: ranking::Candidate| {
        let track = api.ranked_candidate(&candidate.id)?;
        seen.insert(candidate.id).then(|| track.clone())
    };
    for (batch, seeds) in ids.chunks(limit).enumerate() {
        let query = Query {
            mode,
            queue: seeds,
            exclude: &exclude,
            music_folder_id: api.session.client.folder.as_deref(),
            k: match mode {
                Mode::WholeQueue => seeds.len().max(50),
                // At most 16,384 candidates per page, below the plugin's response limit.
                Mode::PerTrack => 64,
            },
        };
        let mut picked = vec![false; seeds.len()];
        let mut offset = 0;
        loop {
            let page = client.rank(&query, offset, generation.as_deref()).await?;
            generation = Some(page.generation);
            let next_offset = match page.rankings {
                Rankings::WholeQueue {
                    candidates,
                    next_offset,
                } => {
                    let start = outcome.items.len();
                    outcome.items.extend(
                        candidates
                            .into_iter()
                            .filter_map(&mut select)
                            .take(seeds.len() - start)
                            .enumerate()
                            .map(|(i, track)| (start + i, track)),
                    );
                    next_offset.filter(|_| outcome.items.len() < seeds.len())
                }
                Rankings::PerTrack { positions } => {
                    let mut next_offset = None;
                    for (i, position) in positions.into_iter().enumerate() {
                        if picked[i] {
                            continue;
                        }
                        if position.analysis_unavailable() {
                            outcome.failed += 1;
                            outcome.first_error.get_or_insert_with(|| AsyncError {
                                message: "AudioMuse has no compatible analysis for one or more queued tracks.".into(),
                            });
                        } else if let Some(track) =
                            position.candidates.into_iter().find_map(&mut select)
                        {
                            outcome.items.push((batch * limit + i, track));
                        } else if let Some(next) = position.next_offset {
                            ensure!(
                                next_offset.is_none_or(|offset| offset == next),
                                "AudioMuse returned inconsistent ranking offsets"
                            );
                            next_offset = Some(next);
                            continue;
                        }
                        picked[i] = true;
                    }
                    next_offset
                }
            };
            let Some(next) = next_offset else { break };
            offset = next;
        }
    }
    outcome
        .items
        .sort_unstable_by_key(|(position, _)| *position);
    Ok(outcome)
}

#[cfg(test)]
mod performance_tests {
    use super::*;
    #[test]
    fn indexed_twofer_candidates_preserve_artist_identity_and_legacy_names() {
        let mut state = AppState::new();
        let make = |key: &str, artist: Option<&str>, name: &str| Track {
            rating_key: key.into(),
            grandparent_rating_key: artist.map(str::to_owned),
            original_title: Some(name.into()),
            ..Default::default()
        };
        state.library.all_tracks = std::sync::Arc::new(vec![
            make("seed", Some("a"), "Shared"),
            make("same-id", Some("a"), "Other"),
            make("legacy", None, "Shared"),
            make("different-id", Some("b"), "Shared"),
        ]);
        state.library.track_lookup = crate::services::catalog_lookup::TrackLookup::build(
            &[crate::library::models::Artist {
                rating_key: "a".into(),
                title: "Shared".into(),
                ..Default::default()
            }],
            &[],
            &[],
            &state.library.all_tracks,
        );
        let picked = local_candidates(
            &state,
            &state.library.all_tracks[0],
            RemixKind::Twofer,
            &HashSet::new(),
        );
        assert_eq!(
            picked
                .into_iter()
                .map(|t| t.rating_key)
                .collect::<HashSet<_>>(),
            HashSet::from(["same-id".into(), "legacy".into()])
        );
    }

    #[test]
    #[ignore = "generated queue-edit measurement"]
    fn twofer_candidate_selection_cost() {
        let mut state = AppState::new();
        state.library.all_tracks = std::sync::Arc::new(
            (0..100_000)
                .map(|i| Track {
                    rating_key: format!("track-{i}"),
                    grandparent_rating_key: Some(format!("artist-{}", i / 100)),
                    ..Default::default()
                })
                .collect(),
        );
        state.library.track_lookup = crate::services::catalog_lookup::TrackLookup::build(
            &[],
            &[],
            &[],
            &state.library.all_tracks,
        );
        let seeds: Vec<_> = state
            .library
            .all_tracks
            .iter()
            .step_by(100)
            .take(200)
            .collect();
        let excluded = seeds.iter().map(|track| track.rating_key.clone()).collect();
        let started = std::time::Instant::now();
        for seed in seeds {
            let matches = local_candidates(&state, seed, RemixKind::Twofer, &excluded);
            assert_eq!(matches.len(), 20);
            assert!(
                matches
                    .iter()
                    .all(|track| same_artist(seed, track) && !excluded.contains(&track.rating_key))
            );
        }
        eprintln!(
            "twofer_200_seeds_100000_tracks: {:.3} ms",
            started.elapsed().as_secs_f64() * 1000.
        );
    }
}
