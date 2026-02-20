//! Background data preloading for faster access.

use crate::app::Event;
use crate::app::event_loop::PreloadType;
use crate::api::PlexClient;
use tokio::sync::mpsc;

/// Preload data in background for faster access.
pub fn preload_data(event_tx: &mpsc::Sender<Event>, preload_type: PreloadType, lib_key: &str, client: &PlexClient) {
    use crate::services::{FolderColumn, FolderNavigationState, FolderService};

    let Some(server_url) = client.server_url() else { return };
    let server_url = server_url.to_string();
    let token = client.token().map(|s| s.to_string());
    let client_id = client.client_identifier().to_string();
    let lib_key = lib_key.to_string();
    let event_tx = event_tx.clone();

    tokio::spawn(async move {
        let client = crate::api::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);
        let lib_key_ref = lib_key.as_str();

        match preload_type {
            PreloadType::Artists => {
                tracing::debug!("Preloading artists for library: {}", lib_key);
                match client.get_artists(lib_key_ref).await {
                    Ok(data) => {
                        tracing::debug!("Artists preloaded: {} items", data.len());
                        let _ = event_tx.send(Event::ArtistsPreloaded { library_key: lib_key, artists: data }).await;
                    }
                    Err(e) => {
                        tracing::error!("Failed to preload artists: {}", e);
                        let _ = event_tx.send(Event::PreloadFailed { category: "Artists".to_string() }).await;
                    }
                }
            }
            PreloadType::Albums => {
                tracing::debug!("Preloading albums for library: {}", lib_key);
                match client.get_albums(lib_key_ref).await {
                    Ok(data) => {
                        tracing::debug!("Albums preloaded: {} items", data.len());
                        let _ = event_tx.send(Event::AlbumsPreloaded { library_key: lib_key, albums: data }).await;
                    }
                    Err(e) => {
                        tracing::error!("Failed to preload albums: {}", e);
                        let _ = event_tx.send(Event::PreloadFailed { category: "Albums".to_string() }).await;
                    }
                }
            }
            PreloadType::Playlists => {
                tracing::debug!("Preloading playlists for library: {}", lib_key);
                match client.get_playlists(Some(&lib_key)).await {
                    Ok(data) => {
                        tracing::debug!("Playlists preloaded: {} items", data.len());
                        let _ = event_tx.send(Event::PlaylistsPreloaded { library_key: lib_key, playlists: data }).await;
                    }
                    Err(e) => {
                        tracing::error!("Failed to preload playlists: {}", e);
                        let _ = event_tx.send(Event::PreloadFailed { category: "Playlists".to_string() }).await;
                    }
                }
            }
            PreloadType::Genres => {
                tracing::debug!("Preloading genres for library: {}", lib_key);
                match client.get_genres(lib_key_ref).await {
                    Ok(data) => {
                        tracing::debug!("Genres preloaded: {} items", data.len());
                        let _ = event_tx.send(Event::GenresPreloaded { library_key: lib_key, genres: data }).await;
                    }
                    Err(e) => {
                        tracing::error!("Failed to preload genres: {}", e);
                        let _ = event_tx.send(Event::PreloadFailed { category: "Genres".to_string() }).await;
                    }
                }
            }
            PreloadType::Moods => {
                tracing::debug!("Preloading moods for library: {}", lib_key);
                match client.get_moods(lib_key_ref).await {
                    Ok(data) => {
                        tracing::debug!("Moods preloaded: {} items", data.len());
                        let _ = event_tx.send(Event::MoodsPreloaded { library_key: lib_key, moods: data }).await;
                    }
                    Err(e) => {
                        tracing::error!("Failed to preload moods: {}", e);
                        let _ = event_tx.send(Event::PreloadFailed { category: "Moods".to_string() }).await;
                    }
                }
            }
            PreloadType::ArtistGenres => {
                tracing::debug!("Preloading artist genres for library: {}", lib_key);
                match client.get_artist_genres(lib_key_ref).await {
                    Ok(data) => {
                        tracing::debug!("Artist genres preloaded: {} items", data.len());
                        let _ = event_tx.send(Event::ArtistGenresPreloaded { library_key: lib_key, genres: data }).await;
                    }
                    Err(e) => {
                        tracing::error!("Failed to preload artist genres: {}", e);
                        let _ = event_tx.send(Event::PreloadFailed { category: "Artist Genres".to_string() }).await;
                    }
                }
            }
            PreloadType::AlbumGenres => {
                tracing::debug!("Preloading album genres for library: {}", lib_key);
                match client.get_album_genres(lib_key_ref).await {
                    Ok(data) => {
                        tracing::debug!("Album genres preloaded: {} items", data.len());
                        let _ = event_tx.send(Event::AlbumGenresPreloaded { library_key: lib_key, genres: data }).await;
                    }
                    Err(e) => {
                        tracing::error!("Failed to preload album genres: {}", e);
                        let _ = event_tx.send(Event::PreloadFailed { category: "Album Genres".to_string() }).await;
                    }
                }
            }
            PreloadType::Styles => {
                tracing::debug!("Preloading styles for library: {}", lib_key);
                match client.get_styles(lib_key_ref).await {
                    Ok(data) => {
                        tracing::debug!("Styles preloaded: {} items", data.len());
                        let _ = event_tx.send(Event::StylesPreloaded { library_key: lib_key, styles: data }).await;
                    }
                    Err(e) => {
                        tracing::error!("Failed to preload styles: {}", e);
                        let _ = event_tx.send(Event::PreloadFailed { category: "Styles".to_string() }).await;
                    }
                }
            }
            PreloadType::Stations => {
                tracing::debug!("Preloading stations for library: {}", lib_key);
                match client.get_stations(lib_key_ref).await {
                    Ok(data) => {
                        tracing::debug!("Stations preloaded: {} items", data.len());
                        let _ = event_tx.send(Event::StationsPreloaded { library_key: lib_key, stations: data }).await;
                    }
                    Err(e) => {
                        tracing::error!("Failed to preload stations: {}", e);
                        let _ = event_tx.send(Event::PreloadFailed { category: "Stations".to_string() }).await;
                    }
                }
            }
            PreloadType::AllTracks => {
                // Use a much longer timeout for AllTracks — can be hundreds of MB for large libraries
                let long_client = crate::api::PlexClient::new_with_url_and_timeout(
                    &server_url, token.as_deref(), &client_id, 600,
                );
                tracing::debug!("Preloading all tracks for library: {}", lib_key);
                match long_client.get_tracks(lib_key_ref).await {
                    Ok(data) => {
                        tracing::debug!("All tracks preloaded: {} items", data.len());
                        let _ = event_tx.send(Event::AllTracksPreloaded { library_key: lib_key, tracks: data }).await;
                    }
                    Err(e) => {
                        tracing::error!("Failed to preload all tracks: {}", e);
                        let _ = event_tx.send(Event::PreloadFailed { category: "Tracks".to_string() }).await;
                    }
                }
            }
            PreloadType::Folders { lib_title } => {
                tracing::debug!("Preloading folders for library: {}", lib_key);
                match client.get_library_folders(lib_key_ref).await {
                    Ok(response) => {
                        let items = FolderService::from_response(&response);
                        let root_column = FolderColumn::new(None, lib_title, items);
                        let folder_state = FolderNavigationState::with_root(lib_key.clone(), root_column);
                        tracing::debug!("Folders preloaded successfully");
                        let _ = event_tx.send(Event::FoldersPreloaded { library_key: lib_key, folder_state }).await;
                    }
                    Err(e) => {
                        tracing::error!("Failed to preload folders: {}", e);
                        let _ = event_tx.send(Event::PreloadFailed { category: "Folders".to_string() }).await;
                    }
                }
            }
        }
    });
}

/// Start background subfolder pre-caching if root folders are available.
///
/// Crawls root-level folder keys and fetches their immediate contents from the Plex API,
/// sending results back in batches. Skips folders that are already cached and fresh
/// (< 72h old). Stale entries are re-fetched incrementally — the old data stays
/// available as a warm cache until overwritten by fresh results.
/// Rate-limited to ~50ms between requests to avoid overloading the server.
/// Result of attempting to start a subfolder preload.
pub enum SubfolderPreloadResult {
    /// Crawl started successfully
    Started,
    /// Already running
    AlreadyActive,
    /// No active library selected
    NoLibrary,
    /// Root folders not loaded yet
    NoRootFolders,
    /// Root has no subfolders (only tracks)
    NoSubfolders,
    /// All subfolders already cached and fresh
    AllCached { count: usize },
}

pub fn maybe_start_subfolder_preload(
    event_tx: &mpsc::Sender<Event>,
    state: &mut crate::app::AppState,
    client: &PlexClient,
) -> SubfolderPreloadResult {
    use crate::plex::constants::CACHE_STALE_THRESHOLD_SECS;
    use std::collections::HashSet;

    // Guard: already active
    if state.subfolder_preload_active {
        return SubfolderPreloadResult::AlreadyActive;
    }

    // Guard: no active library
    let Some(lib_key) = state.active_library.clone() else {
        return SubfolderPreloadResult::NoLibrary;
    };

    // Guard: no root folders loaded yet
    let Some(ref folder_state) = state.folder_state else {
        return SubfolderPreloadResult::NoRootFolders;
    };
    if folder_state.columns.is_empty() {
        return SubfolderPreloadResult::NoRootFolders;
    }

    // Extract root folder keys (only folders, not tracks)
    let root_folder_keys: Vec<String> = folder_state.columns[0]
        .items
        .iter()
        .filter(|item| item.is_folder())
        .map(|item| item.key.clone())
        .collect();

    if root_folder_keys.is_empty() {
        return SubfolderPreloadResult::NoSubfolders;
    }

    // BFS through cache to collect ALL known child folder keys at any depth
    let all_cached_child_keys = collect_cached_child_keys(&root_folder_keys, &state.folder_contents_cache);

    // Determine which keys need fetching (missing or stale > 72h)
    let root_keys_to_fetch = crate::services::CacheService::keys_needing_refresh(
        &root_folder_keys,
        &state.folder_contents_cache,
        CACHE_STALE_THRESHOLD_SECS,
    );

    let child_keys_to_fetch = if all_cached_child_keys.is_empty() {
        Vec::new()
    } else {
        crate::services::CacheService::keys_needing_refresh(
            &all_cached_child_keys,
            &state.folder_contents_cache,
            CACHE_STALE_THRESHOLD_SECS,
        )
    };

    if root_keys_to_fetch.is_empty() && child_keys_to_fetch.is_empty() {
        let total = root_folder_keys.len() + all_cached_child_keys.len();
        tracing::debug!("All {} folder listings cached and fresh ({} root, {} descendant), skipping preload",
            total, root_folder_keys.len(), all_cached_child_keys.len());
        return SubfolderPreloadResult::AllCached { count: total };
    }

    // Combine into initial fetch list
    let mut keys_to_fetch = root_keys_to_fetch;
    keys_to_fetch.extend(child_keys_to_fetch);

    let uncached_count = keys_to_fetch.iter()
        .filter(|k| !state.folder_contents_cache.contains_key(k.as_str()))
        .count();
    let stale_count = keys_to_fetch.len() - uncached_count;
    tracing::info!(
        "Starting subfolder preload: {} to fetch ({} uncached, {} stale), {} already fresh",
        keys_to_fetch.len(), uncached_count, stale_count,
        root_folder_keys.len() + all_cached_child_keys.len() - keys_to_fetch.len()
    );

    // Set active and reset cancel flag
    state.subfolder_preload_active = true;
    state.subfolder_preload_cancel.store(false, std::sync::atomic::Ordering::Relaxed);

    let cancel = state.subfolder_preload_cancel.clone();
    let Some(server_url) = client.server_url() else {
        return SubfolderPreloadResult::NoLibrary;
    };
    let server_url = server_url.to_string();
    let token = client.token().map(|s| s.to_string());
    let client_id = client.client_identifier().to_string();
    let event_tx = event_tx.clone();

    tokio::spawn(async move {
        use crate::plex::CachedFolder;
        use crate::services::FolderService;
        use tokio::sync::Semaphore;
        use std::sync::Arc;

        // Let other preloads finish first
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;

        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            tracing::debug!("Subfolder preload cancelled before starting");
            return;
        }

        let semaphore = Arc::new(Semaphore::new(4));
        let client = Arc::new(crate::api::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id));
        let cancel = Arc::new(cancel);

        // Track all keys we've ever queued for fetching (cycle prevention)
        let mut all_fetched_keys: HashSet<String> = keys_to_fetch.iter().cloned().collect();
        let mut current_keys = keys_to_fetch;
        let mut batch: Vec<(String, CachedFolder)> = Vec::new();
        let mut total_fetched = 0u64;
        let mut depth = 0u32;

        // Iterative crawl: fetch current level, discover children, repeat
        loop {
            if current_keys.is_empty() || cancel.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }

            tracing::info!("Subfolder crawl depth {}: {} keys to fetch", depth, current_keys.len());

            // Spawn concurrent fetches for this level
            let mut handles = Vec::with_capacity(current_keys.len());
            for folder_key in current_keys {
                let sem = semaphore.clone();
                let client = client.clone();
                let cancel = cancel.clone();

                handles.push(tokio::spawn(async move {
                    let _permit = sem.acquire().await.unwrap();
                    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        return None;
                    }
                    match client.get_folder_contents(&folder_key).await {
                        Ok(response) => {
                            let items = FolderService::from_response(&response);
                            let folder_path = FolderService::folder_path(&response);
                            Some((folder_key, CachedFolder::with_path(items, folder_path)))
                        }
                        Err(e) => {
                            tracing::warn!("Subfolder preload failed for {}: {}", folder_key, e);
                            None
                        }
                    }
                }));
            }

            // Collect results and discover child folder keys
            let mut next_keys: Vec<String> = Vec::new();

            for handle in handles {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    tracing::info!("Subfolder preload cancelled at depth {} after {} total fetches", depth, total_fetched);
                    return;
                }
                if let Ok(Some(entry)) = handle.await {
                    // Discover child folder keys from this result
                    for item in &entry.1.items {
                        if item.is_folder() && all_fetched_keys.insert(item.key.clone()) {
                            next_keys.push(item.key.clone());
                        }
                    }

                    batch.push(entry);
                    total_fetched += 1;

                    // Send batch every 10 entries for responsive UI updates
                    if batch.len() >= 10 {
                        tracing::debug!("Subfolder preload batch: {} fetched so far", total_fetched);
                        let _ = event_tx.send(Event::SubfoldersPreloaded {
                            library_key: lib_key.clone(),
                            entries: std::mem::take(&mut batch),
                            done: false,
                        }).await;
                    }
                }
            }

            // Send remaining batch from this level
            if !batch.is_empty() {
                let _ = event_tx.send(Event::SubfoldersPreloaded {
                    library_key: lib_key.clone(),
                    entries: std::mem::take(&mut batch),
                    done: false,
                }).await;
            }

            if next_keys.is_empty() {
                break;
            }

            current_keys = next_keys;
            depth += 1;
        }

        // Send final event to signal done
        tracing::info!("Subfolder preload complete: {} total fetched across {} depth levels", total_fetched, depth + 1);
        let _ = event_tx.send(Event::SubfoldersPreloaded {
            library_key: lib_key,
            entries: batch,
            done: true,
        }).await;
    });

    SubfolderPreloadResult::Started
}

/// BFS through cached folder contents to collect all known child folder keys.
fn collect_cached_child_keys(
    parent_keys: &[String],
    cache: &std::collections::HashMap<String, crate::plex::CachedFolder>,
) -> Vec<String> {
    use std::collections::HashSet;

    let mut all_keys = Vec::new();
    let mut frontier = parent_keys.to_vec();
    let mut visited: HashSet<String> = parent_keys.iter().cloned().collect();

    while !frontier.is_empty() {
        let mut next = Vec::new();
        for key in &frontier {
            if let Some(cached) = cache.get(key.as_str()) {
                for item in &cached.items {
                    if item.is_folder() && visited.insert(item.key.clone()) {
                        all_keys.push(item.key.clone());
                        next.push(item.key.clone());
                    }
                }
            }
        }
        frontier = next;
    }

    all_keys
}

/// Preload all library data in background.
pub fn preload_all_library_data(event_tx: &mpsc::Sender<Event>, lib_key: &str, lib_title: &str, client: &PlexClient, state: &mut crate::app::AppState) {
    let categories = [
        "Artists", "Folders", "Albums", "Tracks", "Genres",
        "Artist Genres", "Album Genres", "Moods", "Styles", "Stations", "Playlists",
    ];
    state.cache_mgmt.preloads_in_progress = categories.iter().map(|s| s.to_string()).collect();
    state.cache_mgmt.preloads_total = categories.len();

    preload_data(event_tx, PreloadType::Artists, lib_key, client);
    preload_data(event_tx, PreloadType::Folders { lib_title: lib_title.to_string() }, lib_key, client);
    preload_data(event_tx, PreloadType::Albums, lib_key, client);
    preload_data(event_tx, PreloadType::AllTracks, lib_key, client);
    preload_data(event_tx, PreloadType::Genres, lib_key, client);
    preload_data(event_tx, PreloadType::ArtistGenres, lib_key, client);
    preload_data(event_tx, PreloadType::AlbumGenres, lib_key, client);
    preload_data(event_tx, PreloadType::Moods, lib_key, client);
    preload_data(event_tx, PreloadType::Styles, lib_key, client);
    preload_data(event_tx, PreloadType::Stations, lib_key, client);
    preload_data(event_tx, PreloadType::Playlists, lib_key, client);
}
