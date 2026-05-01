//! Cache saving and background data refresh.

use crate::app::event::*;
use crate::app::{AppState, Event};
use crate::plex::LibraryCache;
use tokio::sync::mpsc;

/// Check if we should save the cache and spawn async save if conditions are met.
pub fn maybe_save_cache_async(event_tx: &mpsc::Sender<Event>, state: &mut AppState) {
    if !state.cache_mgmt.dirty || state.cache_mgmt.save_in_progress {
        return;
    }

    let lib_key = match &state.active_library {
        Some(k) => k.clone(),
        None => return,
    };

    let idle_threshold = std::time::Duration::from_secs(30);
    if state.cache_mgmt.last_input_time.elapsed() < idle_threshold {
        return;
    }

    let save_interval = std::time::Duration::from_secs(120);
    if state.cache_mgmt.last_save.elapsed() < save_interval {
        return;
    }

    state.cache_mgmt.save_in_progress = true;
    state.cache_mgmt.dirty = false;
    state.cache_mgmt.last_save = std::time::Instant::now();

    use crate::plex::CacheData;
    let mut cache_data = CacheData::new(&lib_key);
    // Write per-category timestamps
    cache_data.category_timestamps = state.cache_mgmt.category_timestamps.iter()
        .map(|(cat, &ts)| (cat.cache_key().to_string(), ts))
        .collect();
    // Write legacy timestamps for backward compat
    if let Some(&ts) = state.cache_mgmt.category_timestamps.get(&crate::app::state::RefreshCategory::Artists) {
        cache_data.timestamp = ts;
    }
    if let Some(&ts) = state.cache_mgmt.category_timestamps.get(&crate::app::state::RefreshCategory::Playlists) {
        cache_data.playlist_timestamp = ts;
    }
    cache_data.artists = state.library.artists.clone();
    cache_data.albums = state.library.albums.clone();
    cache_data.playlists = state.library.playlists.clone();
    if let Some(ref folder_state) = state.folder_state {
        if folder_state.library_key == lib_key {
            if let Some(root_col) = folder_state.columns.first() {
                cache_data.root_folders = root_col.unshuffled_items().to_vec();
            }
        } else {
            tracing::debug!("Not saving folder_state (periodic) - belongs to different library (expected {}, got {})",
                lib_key, folder_state.library_key);
        }
    }
    // Keep all subfolder entries if keep_subfolder_cache, else purge > 32 days
    cache_data.folder_contents = if state.keep_subfolder_cache {
        state.folder_contents_cache.clone()
    } else {
        state.folder_contents_cache.iter()
            .filter(|(_, cached)| !cached.is_older_than(crate::plex::constants::CACHE_VERY_STALE_THRESHOLD_SECS))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    };
    cache_data.genres = state.library.album_genres.clone();
    cache_data.artist_genres = state.library.artist_genres.clone();
    cache_data.album_genres = state.library.album_genres.clone();
    cache_data.moods = state.library.moods.clone();
    cache_data.styles = state.library.styles.clone();
    // Save root column stations (not state.stations which may be drilled children)
    cache_data.stations = state.station_nav.columns.first()
        .map(|c| c.stations.clone())
        .unwrap_or_default();
    cache_data.station_children = state.station_children_cache.clone();

    // All tracks + track-level artists + aliases
    // Only save if non-empty to avoid overwriting cached data when preload is in-flight
    if !state.library.all_tracks.is_empty() {
        cache_data.all_tracks = state.library.all_tracks.clone();
        cache_data.track_artists = state.library.track_artists.clone();
    }
    cache_data.artist_aliases = state.library.artist_aliases.clone();
    cache_data.album_display_artist = state.library.album_display_artist.clone();

    // Compilation detection results
    cache_data.compilation_albums = state.library.compilations.albums.clone();
    cache_data.compilation_artist_keys = state.library.compilations.artist_keys.clone();
    cache_data.compilation_track_artist_keys = state.library.compilations.track_artist_keys.clone();
    cache_data.artist_compilation_map = state.library.compilations.artist_map.clone();
    cache_data.single_artist_compilations = state.library.compilations.single_artist.clone();

    // Save non-smart playlist tracks to disk cache
    for (key, cached) in &state.playlist_tracks_cache {
        let is_smart = state.library.playlists.iter().any(|p| p.rating_key == *key && p.smart);
        if !is_smart {
            cache_data.playlist_tracks.insert(key.clone(), cached.clone());
        }
    }

    let event_tx = event_tx.clone();
    tokio::spawn(async move {
        if let Some(cache) = LibraryCache::new() {
            match serde_json::to_string(&cache_data) {
                Ok(contents) => {
                    let path = cache.cache_path(&lib_key);
                    let temp_path = path.with_extension("json.tmp");

                    match tokio::fs::write(&temp_path, &contents).await {
                        Ok(_) => {
                            if let Err(e) = tokio::fs::rename(&temp_path, &path).await {
                                tracing::warn!("Failed to rename cache file: {}", e);
                                let _ = tokio::fs::remove_file(&temp_path).await;
                            } else {
                                tracing::debug!("Cache saved (periodic): {:?}", path);
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to write cache temp file: {}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to serialize cache: {}", e);
                }
            }
        }

        let _ = event_tx.send(CacheEvent::CacheSaved.into()).await;
    });
}
