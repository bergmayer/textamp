//! Cache saving and background data refresh.

use crate::app::{AppState, Event};
use crate::cache::LibraryCache;
use tokio::sync::mpsc;

/// Check if we should save the cache and spawn async save if conditions are met.
pub fn maybe_save_cache_async(event_tx: &mpsc::Sender<Event>, state: &mut AppState) {
    if !state.cache_dirty || state.cache_save_in_progress {
        return;
    }

    let lib_key = match &state.active_library {
        Some(k) => k.clone(),
        None => return,
    };

    let idle_threshold = std::time::Duration::from_secs(30);
    if state.last_input_time.elapsed() < idle_threshold {
        return;
    }

    let save_interval = std::time::Duration::from_secs(120);
    if state.last_cache_save.elapsed() < save_interval {
        return;
    }

    state.cache_save_in_progress = true;
    state.cache_dirty = false;
    state.last_cache_save = std::time::Instant::now();

    use crate::cache::CacheData;
    let mut cache_data = CacheData::new(&lib_key);
    // Write per-category timestamps
    cache_data.category_timestamps = state.category_timestamps.iter()
        .map(|(cat, &ts)| (cat.cache_key().to_string(), ts))
        .collect();
    // Write legacy timestamps for backward compat
    if let Some(&ts) = state.category_timestamps.get(&crate::app::state::RefreshCategory::Artists) {
        cache_data.timestamp = ts;
    }
    if let Some(&ts) = state.category_timestamps.get(&crate::app::state::RefreshCategory::Playlists) {
        cache_data.playlist_timestamp = ts;
    }
    cache_data.artists = state.artists.clone();
    cache_data.albums = state.albums.clone();
    cache_data.playlists = state.playlists.clone();
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
    // Keep all subfolder entries if keep_folder_cache, else purge > 32 days
    cache_data.folder_contents = if state.keep_folder_cache {
        state.folder_contents_cache.clone()
    } else {
        state.folder_contents_cache.iter()
            .filter(|(_, cached)| !cached.is_older_than(crate::plex::constants::CACHE_VERY_STALE_THRESHOLD_SECS))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    };
    cache_data.genres = state.genres.clone();
    cache_data.artist_genres = state.artist_genres.clone();
    cache_data.album_genres = state.album_genres.clone();
    cache_data.moods = state.moods.clone();
    cache_data.styles = state.styles.clone();
    cache_data.stations = state.stations.clone();

    // Save non-smart playlist tracks to disk cache
    for (key, cached) in &state.playlist_tracks_cache {
        let is_smart = state.playlists.iter().any(|p| p.rating_key == *key && p.smart);
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

        let _ = event_tx.send(Event::CacheSaved).await;
    });
}
