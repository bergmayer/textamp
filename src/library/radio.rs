//! Radio from directory listings, without a tag/AI scan or downloading media.
//! A playable leaf directory is an album; parent names are not artist metadata.
use super::{FolderEntry, FolderLocation, FolderSource, cache::Store, track::Track};
use anyhow::{Context, Result, ensure};
use futures::{StreamExt, stream::FuturesUnordered};
use rand::seq::SliceRandom;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug)]
pub enum Recipe {
    RandomAlbum,
    Library,
}

// Bound traversal even when a remote tree is empty, huge, or changes mid-walk.
const MAX_DIRECTORIES: usize = 256;
const MAX_PENDING: usize = 100_000;
const MAX_DEPTH: usize = 128;

/// Randomized branch discovery. Library radio reads up to four directories at
/// once; random-album radio retains depth-first ordering. It is not a uniform draw
/// over every album: that would require indexing the entire remote hierarchy.
/// Listings share the browser's persistent, source-scoped cache. Local listings
/// are always refreshed; remote snapshots follow the weekly refresh policy.
pub async fn select(
    source: &FolderSource,
    store: &Store,
    recipe: Recipe,
    excluded: &HashSet<String>,
) -> Result<Vec<Track>> {
    let mut pending = vec![(String::new(), 0)];
    let mut visited = HashSet::new();
    let mut candidates = Vec::new();
    let mut albums = 0;
    let concurrency = match recipe {
        Recipe::RandomAlbum => 1,
        Recipe::Library => 4,
    };
    let mut requests = FuturesUnordered::new();
    loop {
        while requests.len() < concurrency {
            let Some((path, depth)) = pending.pop() else {
                break;
            };
            if !visited.insert(path.clone()) {
                continue;
            }
            ensure!(
                visited.len() <= MAX_DIRECTORIES,
                "Radio searched 256 folders without enough playable music; choose a smaller library root"
            );
            ensure!(depth <= MAX_DEPTH, "Radio folder hierarchy is too deep");
            requests.push(async move {
                listing(source, store, &path)
                    .await
                    .map(|entries| (depth, entries))
            });
        }
        let Some(result) = requests.next().await else {
            break;
        };
        let (depth, entries) = result?;
        let mut children: Vec<_> = entries.iter().filter(|e| e.directory).collect();
        let tracks: Vec<_> = entries
            .iter()
            .filter(|e| !e.directory)
            .map(|e| Track::from_folder(&source.id, &e.path))
            .collect();
        match recipe {
            Recipe::RandomAlbum
                if children.is_empty()
                    && !tracks.is_empty()
                    && tracks.iter().all(|t| !excluded.contains(&t.rating_key)) =>
            {
                return Ok(tracks);
            }
            Recipe::Library if !tracks.is_empty() => {
                albums += 1;
                candidates.extend(
                    tracks
                        .into_iter()
                        .filter(|t| !excluded.contains(&t.rating_key)),
                );
                if candidates.len() >= 100 || albums >= 8 && !candidates.is_empty() {
                    break;
                }
            }
            _ => {}
        }
        children.shuffle(&mut rand::rng());
        ensure!(
            pending.len() + children.len() <= MAX_PENDING,
            "Radio folder listing is too large"
        );
        pending.extend(children.into_iter().map(|e| (e.path.clone(), depth + 1)));
    }
    ensure!(
        !candidates.is_empty(),
        match recipe {
            Recipe::RandomAlbum =>
                "No unplayed albums remain in the leaf folders. Start the station again to replay them.",
            Recipe::Library =>
                "No unplayed audio files remain. Start the station again to replay them.",
        }
    );
    candidates.shuffle(&mut rand::rng());
    candidates.truncate(100);
    Ok(candidates)
}

async fn listing(source: &FolderSource, store: &Store, path: &str) -> Result<Vec<FolderEntry>> {
    let ticket = store.ticket(path);
    if matches!(source.location, FolderLocation::Webdav { .. }) {
        let read = ticket.clone();
        match tokio::task::spawn_blocking(move || {
            read.read::<Vec<FolderEntry>>(crate::library::cache::REFRESH_INTERVAL)
        })
        .await?
        {
            Ok(Some(hit)) if !hit.stale => return Ok(hit.value),
            Err(error) => tracing::warn!("Radio directory cache read: {error}"),
            _ => {}
        }
    }
    let entries = source.list(path).await.context("Read radio folder")?;
    let saved = entries.clone();
    // A failed cache write is reported, not silently treated as persistence.
    tokio::task::spawn_blocking(move || ticket.write(&saved))
        .await?
        .context("Save radio folder cache")?;
    Ok(entries)
}
