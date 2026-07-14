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
    let mut cache_data = CacheData::new_scoped(&lib_key, state.active_server_id.as_deref());
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
    cache_data.decades = state.library.decades.clone();
    cache_data.years = state.library.years.clone();
    cache_data.collections = state.library.collections.clone();
    cache_data.countries = state.library.countries.clone();
    cache_data.labels = state.library.labels.clone();
    cache_data.formats = state.library.formats.clone();
    cache_data.studios = state.library.studios.clone();
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
    let smart_playlist_keys: std::collections::HashSet<&str> = state
        .library
        .playlists
        .iter()
        .filter(|playlist| playlist.smart)
        .map(|playlist| playlist.rating_key.as_str())
        .collect();
    for (key, cached) in &state.playlist_tracks_cache {
        if !smart_playlist_keys.contains(key.as_str()) {
            cache_data.playlist_tracks.insert(key.clone(), cached.clone());
        }
    }

    let event_tx = event_tx.clone();
    tokio::task::spawn_blocking(move || {
        if let Some(cache) = LibraryCache::new() {
            if cache.save_preserving_unloaded(cache_data) {
                tracing::debug!("Cache saved (periodic) for library {}", lib_key);
            }
        }

        let _ = event_tx.blocking_send(CacheEvent::CacheSaved.into());
    });
}
