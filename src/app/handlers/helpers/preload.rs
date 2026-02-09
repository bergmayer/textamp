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
                if let Ok(data) = client.get_artists(lib_key_ref).await {
                    tracing::debug!("Artists preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::ArtistsPreloaded { library_key: lib_key, artists: data }).await;
                }
            }
            PreloadType::Albums => {
                tracing::debug!("Preloading albums for library: {}", lib_key);
                if let Ok(data) = client.get_albums(lib_key_ref).await {
                    tracing::debug!("Albums preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::AlbumsPreloaded { library_key: lib_key, albums: data }).await;
                }
            }
            PreloadType::Playlists => {
                tracing::debug!("Preloading playlists for library: {}", lib_key);
                if let Ok(data) = client.get_playlists(Some(&lib_key)).await {
                    tracing::debug!("Playlists preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::PlaylistsPreloaded { library_key: lib_key, playlists: data }).await;
                }
            }
            PreloadType::Genres => {
                tracing::debug!("Preloading genres for library: {}", lib_key);
                if let Ok(data) = client.get_genres(lib_key_ref).await {
                    tracing::debug!("Genres preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::GenresPreloaded { library_key: lib_key, genres: data }).await;
                }
            }
            PreloadType::Moods => {
                tracing::debug!("Preloading moods for library: {}", lib_key);
                if let Ok(data) = client.get_moods(lib_key_ref).await {
                    tracing::debug!("Moods preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::MoodsPreloaded { library_key: lib_key, moods: data }).await;
                }
            }
            PreloadType::ArtistGenres => {
                tracing::debug!("Preloading artist genres for library: {}", lib_key);
                if let Ok(data) = client.get_artist_genres(lib_key_ref).await {
                    tracing::debug!("Artist genres preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::ArtistGenresPreloaded { library_key: lib_key, genres: data }).await;
                }
            }
            PreloadType::AlbumGenres => {
                tracing::debug!("Preloading album genres for library: {}", lib_key);
                if let Ok(data) = client.get_album_genres(lib_key_ref).await {
                    tracing::debug!("Album genres preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::AlbumGenresPreloaded { library_key: lib_key, genres: data }).await;
                }
            }
            PreloadType::Styles => {
                tracing::debug!("Preloading styles for library: {}", lib_key);
                if let Ok(data) = client.get_styles(lib_key_ref).await {
                    tracing::debug!("Styles preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::StylesPreloaded { library_key: lib_key, styles: data }).await;
                }
            }
            PreloadType::Stations => {
                tracing::debug!("Preloading stations for library: {}", lib_key);
                if let Ok(data) = client.get_stations(lib_key_ref).await {
                    tracing::debug!("Stations preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::StationsPreloaded { library_key: lib_key, stations: data }).await;
                }
            }
            PreloadType::RecentlyAdded => {
                tracing::debug!("Preloading recently added albums for library: {}", lib_key);
                if let Ok(data) = client.get_recently_added_albums(lib_key_ref, 50).await {
                    tracing::debug!("Recently added albums preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::RecentlyAddedPreloaded { library_key: lib_key, albums: data }).await;
                }
            }
            PreloadType::RecentlyPlayed => {
                tracing::debug!("Preloading recently played albums for library: {}", lib_key);
                if let Ok(data) = client.get_recently_played_albums(lib_key_ref, 50).await {
                    tracing::debug!("Recently played albums preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::RecentlyPlayedPreloaded { library_key: lib_key, albums: data }).await;
                }
            }
            PreloadType::Folders { lib_title } => {
                tracing::debug!("Preloading folders for library: {}", lib_key);
                if let Ok(response) = client.get_library_folders(lib_key_ref).await {
                    let items = FolderService::from_response(&response);
                    let root_column = FolderColumn::new(None, lib_title, items);
                    let folder_state = FolderNavigationState {
                        library_key: lib_key.clone(),
                        columns: vec![root_column],
                        focused_column: 0,
                        loading: false,
                    };
                    tracing::debug!("Folders preloaded successfully");
                    let _ = event_tx.send(Event::FoldersPreloaded { library_key: lib_key, folder_state }).await;
                }
            }
        }
    });
}

/// Start background subfolder pre-caching if root folders are available.
///
/// Crawls root-level folder keys and fetches their immediate contents from the Plex API,
/// sending results back in batches. Skips folders that are already cached and fresh
/// (< 32 days old). Stale entries are re-fetched incrementally — the old data stays
/// available as a warm cache until overwritten by fresh results.
/// Rate-limited to ~50ms between requests to avoid overloading the server.
pub fn maybe_start_subfolder_preload(
    event_tx: &mpsc::Sender<Event>,
    state: &mut crate::app::AppState,
    client: &PlexClient,
) {
    use crate::plex::constants::CACHE_VERY_STALE_THRESHOLD_SECS;

    // Guard: already active
    if state.subfolder_preload_active {
        return;
    }

    // Guard: no active library
    let Some(lib_key) = state.active_library.clone() else { return };

    // Guard: no root folders loaded yet
    let Some(ref folder_state) = state.folder_state else { return };
    if folder_state.columns.is_empty() {
        return;
    }

    // Extract root folder keys (only folders, not tracks)
    let root_folder_keys: Vec<String> = folder_state.columns[0]
        .items
        .iter()
        .filter(|item| item.is_folder())
        .map(|item| item.key.clone())
        .collect();

    if root_folder_keys.is_empty() {
        return;
    }

    // Determine which keys need fetching (missing or stale > 32 days)
    let keys_to_fetch = crate::services::CacheService::keys_needing_refresh(
        &root_folder_keys,
        &state.folder_contents_cache,
        CACHE_VERY_STALE_THRESHOLD_SECS,
    );

    if keys_to_fetch.is_empty() {
        tracing::debug!("All {} root subfolders cached and fresh, skipping preload", root_folder_keys.len());
        return;
    }

    let uncached_count = keys_to_fetch.iter()
        .filter(|k| !state.folder_contents_cache.contains_key(k.as_str()))
        .count();
    let stale_count = keys_to_fetch.len() - uncached_count;
    tracing::info!(
        "Starting subfolder preload: {} uncached, {} stale, {} fresh (of {} total root folders)",
        uncached_count, stale_count, root_folder_keys.len() - keys_to_fetch.len(), root_folder_keys.len()
    );

    // Set active and reset cancel flag
    state.subfolder_preload_active = true;
    state.subfolder_preload_cancel.store(false, std::sync::atomic::Ordering::Relaxed);

    let cancel = state.subfolder_preload_cancel.clone();
    let Some(server_url) = client.server_url() else { return };
    let server_url = server_url.to_string();
    let token = client.token().map(|s| s.to_string());
    let client_id = client.client_identifier().to_string();
    let event_tx = event_tx.clone();

    let total_to_fetch = keys_to_fetch.len();

    tokio::spawn(async move {
        use crate::plex::CachedFolder;
        use crate::services::FolderService;

        // Let other preloads finish first
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;

        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            tracing::debug!("Subfolder preload cancelled before starting");
            return;
        }

        let client = crate::api::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);
        let mut batch: Vec<(String, CachedFolder)> = Vec::new();
        let mut fetched = 0u64;

        for folder_key in &keys_to_fetch {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                tracing::info!("Subfolder preload cancelled after {} fetches", fetched);
                return;
            }

            // Rate limit
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;

            match client.get_folder_contents(folder_key).await {
                Ok(response) => {
                    let items = FolderService::from_response(&response);
                    batch.push((folder_key.clone(), CachedFolder::new(items)));
                    fetched += 1;
                }
                Err(e) => {
                    tracing::warn!("Subfolder preload failed for {}: {}", folder_key, e);
                }
            }

            // Send batch every 10 entries (small batches for responsive UI updates)
            if batch.len() >= 10 {
                tracing::debug!("Subfolder preload batch: {} fetched so far (of {} to fetch)", fetched, total_to_fetch);
                let _ = event_tx.send(Event::SubfoldersPreloaded {
                    library_key: lib_key.clone(),
                    entries: std::mem::take(&mut batch),
                    done: false,
                }).await;
            }
        }

        // Send final batch (even if empty, to signal done)
        tracing::info!("Subfolder preload complete: {} fetched of {} needed", fetched, total_to_fetch);
        let _ = event_tx.send(Event::SubfoldersPreloaded {
            library_key: lib_key,
            entries: batch,
            done: true,
        }).await;
    });
}

/// Preload all library data in background.
pub fn preload_all_library_data(event_tx: &mpsc::Sender<Event>, lib_key: &str, lib_title: &str, client: &PlexClient) {
    preload_data(event_tx, PreloadType::Artists, lib_key, client);
    preload_data(event_tx, PreloadType::Folders { lib_title: lib_title.to_string() }, lib_key, client);
    preload_data(event_tx, PreloadType::Albums, lib_key, client);
    preload_data(event_tx, PreloadType::Genres, lib_key, client);
    preload_data(event_tx, PreloadType::ArtistGenres, lib_key, client);
    preload_data(event_tx, PreloadType::AlbumGenres, lib_key, client);
    preload_data(event_tx, PreloadType::Moods, lib_key, client);
    preload_data(event_tx, PreloadType::Styles, lib_key, client);
    preload_data(event_tx, PreloadType::Stations, lib_key, client);
    preload_data(event_tx, PreloadType::Playlists, lib_key, client);
    preload_data(event_tx, PreloadType::RecentlyAdded, lib_key, client);
    preload_data(event_tx, PreloadType::RecentlyPlayed, lib_key, client);
}
