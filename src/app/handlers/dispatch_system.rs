//! System dispatch handlers: Quit, ShowError, ClearError, SetStatus, ClearStatus,
//! RefreshCategory, CycleTheme, LoadArtwork, LoadWaveform.

use crate::app::{Action, AppState, Event};
use crate::cache::LibraryCache;
use crate::api::PlexClient;
use crate::config::Config;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Dispatch system-level actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    config: &mut Config,
    action: Action,
    state: &mut AppState,
    client: &mut PlexClient,
) -> Result<Vec<Action>> {
    match action {
        Action::Quit => {
            // Report playback stop to Plex before quitting
            if state.playback.status != crate::app::state::PlayStatus::Stopped {
                if let Some(track) = state.current_track().cloned() {
                    helpers::report_playback_stop_to_plex(
                        &track, state.playback.position_ms, false,
                        state.plex_session_id.clone(), client,
                    );
                }
            }

            // Stop remote player if active
            if let crate::app::state::OutputTarget::Remote { ref player_id, ref player_uri, .. } = state.output_target {
                let target_id = player_id.clone();
                let p_uri = player_uri.clone();
                let token = client.token().map(|s| s.to_string()).unwrap_or_default();
                let client_id = client.client_identifier().to_string();
                let server_url = client.server_url().unwrap_or("").to_string();
                let machine_id = state.available_servers.first()
                    .map(|s| s.client_identifier.clone()).unwrap_or_default();
                // Use blocking wait to ensure stop is sent before app exits
                let rt = tokio::runtime::Handle::current();
                rt.spawn(async move {
                    let rc = crate::plex::RemotePlayerClient::new(
                        token, client_id, target_id, server_url, machine_id, p_uri,
                    );
                    let _ = rc.stop().await;
                });
                // Brief pause to let the stop command send
                std::thread::sleep(std::time::Duration::from_millis(200));
            }

            // Save cache to disk before quitting
            if let Some(lib_key) = &state.active_library {
                use crate::cache::CacheData;

                let mut cache_data = CacheData::new(lib_key);
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

                // Core library data
                cache_data.artists = state.artists.clone();
                cache_data.albums = state.albums.clone();
                cache_data.playlists = state.playlists.clone();

                // Folder data - extract root folder items only if they belong to this library
                if let Some(ref folder_state) = state.folder_state {
                    if folder_state.library_key == *lib_key {
                        if let Some(root_col) = folder_state.columns.first() {
                            cache_data.root_folders = root_col.unshuffled_items().to_vec();
                        }
                    } else {
                        tracing::debug!("Not saving folder_state on quit - belongs to different library (expected {}, got {})",
                            lib_key, folder_state.library_key);
                    }
                }
                // Save cached subfolder contents (keep all if keep_folder_cache, else purge > 32 days)
                cache_data.folder_contents = if state.keep_folder_cache {
                    state.folder_contents_cache.clone()
                } else {
                    state.folder_contents_cache.iter()
                        .filter(|(_, cached)| !cached.is_older_than(crate::plex::constants::CACHE_VERY_STALE_THRESHOLD_SECS))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                };

                // Genre/mood/style data
                cache_data.genres = state.genres.clone();
                cache_data.artist_genres = state.artist_genres.clone();
                cache_data.album_genres = state.album_genres.clone();
                cache_data.moods = state.moods.clone();
                cache_data.styles = state.styles.clone();

                // Stations
                cache_data.stations = state.stations.clone();

                // Recent content
                cache_data.recently_added_albums = state.recently_added_albums.clone();

                // Save non-smart playlist tracks to disk cache
                for (key, cached) in &state.playlist_tracks_cache {
                    let is_smart = state.playlists.iter().any(|p| p.rating_key == *key && p.smart);
                    if !is_smart {
                        cache_data.playlist_tracks.insert(key.clone(), cached.clone());
                    }
                }

                if let Some(cache) = LibraryCache::new() {
                    if cache.save(&cache_data) {
                        tracing::info!("Cache saved on quit");
                    }
                }
            }

            state.should_quit = true;
        }
        Action::ShowError(msg) => {
            state.set_error(msg);
        }
        Action::ClearError => {
            state.clear_error();
        }
        Action::SetStatus(msg) => {
            state.set_status(msg);
        }
        Action::ClearStatus => {
            state.clear_status();
        }
        Action::RefreshCategory(category) => {
            if let Some(lib_key) = &state.active_library {
                let lib_key = lib_key.clone();
                helpers::spawn_category_refresh(event_tx, category, &lib_key, state, client);
            }
        }
        Action::CheckStaleness(tier1_category) => {
            helpers::check_staleness_on_view_load(event_tx, state, client, tier1_category);
        }
        Action::CycleTheme => {
            state.theme = state.theme.next();
            crate::ui::theme::set_theme(state.theme);
            state.set_status(format!("Theme: {}", state.theme.display_name()));

            // Persist theme to config
            config.ui.theme = state.theme.config_name().to_string();
            if let Err(e) = crate::config::save_config(config) {
                tracing::warn!("Failed to save theme preference: {}", e);
            }
        }
        Action::LoadArtwork => {
            // Get thumb path from current track (clone to avoid borrow)
            let thumb_path = state.current_track()
                .and_then(|t| t.best_thumb().map(|s| s.to_string()));

            if let Some(thumb_path) = thumb_path {
                // Check if we need to load new artwork
                if state.artwork_thumb.as_deref() != Some(&thumb_path) {
                    state.artwork_loading = true;
                    match client.fetch_artwork(&thumb_path, 300).await {
                        Ok(data) => {
                            state.artwork_thumb = Some(thumb_path);
                            state.artwork_data = Some(data);
                        }
                        Err(e) => {
                            tracing::warn!("Failed to load artwork: {}", e);
                            state.artwork_thumb = None;
                            state.artwork_data = None;
                        }
                    }
                    state.artwork_loading = false;
                }
            } else {
                // No artwork available or no current track
                state.artwork_thumb = None;
                state.artwork_data = None;
            }
        }
        Action::LoadWaveform => {
            // Only generate waveform if we have a track and don't already have data
            if let Some(track) = state.current_track().cloned() {
                let needs_generation = state.waveform.data.is_none()
                    && !state.waveform.generating
                    && state.waveform.track_key.as_ref() == Some(&track.rating_key);

                if needs_generation {
                    state.waveform.generating = true;
                    let track_key = track.rating_key.clone();
                    let duration_ms = track.duration_ms();
                    let event_tx = event_tx.clone();

                    if let Ok(stream_url) = client.get_stream_url(&track) {
                        let token = client.token().map(|s| s.to_string());

                        tokio::spawn(async move {
                            // Check cache first
                            let cache_dir = dirs::cache_dir()
                                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                                .join("textamp")
                                .join("waveforms");
                            let cache = crate::services::WaveformCache::new(cache_dir);

                            if let Some(data) = cache.load(&track_key) {
                                // Cache hit
                                let _ = event_tx.send(Event::WaveformCacheHit {
                                    track_key,
                                    data,
                                }).await;
                                return;
                            }

                            // Cache miss - download and generate
                            let http_client = reqwest::Client::new();
                            let mut request = http_client.get(&stream_url);
                            if let Some(ref token) = token {
                                request = request.header("X-Plex-Token", token);
                            }

                            match request.send().await {
                                Ok(response) => {
                                    match response.bytes().await {
                                        Ok(audio_data) => {
                                            // Generate waveform
                                            match crate::services::generate_waveform(
                                                track_key.clone(),
                                                duration_ms,
                                                audio_data.to_vec(),
                                            ) {
                                                Ok(data) => {
                                                    // Save to cache
                                                    cache.save(&data);
                                                    let _ = event_tx.send(Event::WaveformGenerated {
                                                        track_key,
                                                        data,
                                                    }).await;
                                                }
                                                Err(e) => {
                                                    let _ = event_tx.send(Event::WaveformFailed {
                                                        track_key,
                                                        error: e.to_string(),
                                                    }).await;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            let _ = event_tx.send(Event::WaveformFailed {
                                                track_key,
                                                error: format!("Download failed: {}", e),
                                            }).await;
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = event_tx.send(Event::WaveformFailed {
                                        track_key,
                                        error: format!("Request failed: {}", e),
                                    }).await;
                                }
                            }
                        });
                    }
                }
            }
        }
        Action::ToggleAlbumArtView => {
            state.album_art_view = !state.album_art_view;

            // Persist cover art view preference
            config.ui.cover_art_view = state.album_art_view;
            if let Err(e) = crate::config::save_config(config) {
                tracing::warn!("Failed to save cover art view preference: {}", e);
            }

            if state.album_art_view {
                // Load art only for visible items in the focused column
                let nav = match state.browse_category {
                    crate::app::state::BrowseCategory::Artists => &state.artist_nav,
                    crate::app::state::BrowseCategory::Genres => &state.genre_nav,
                    crate::app::state::BrowseCategory::Playlists => &state.playlist_nav,
                    _ => return Ok(vec![]),
                };

                if let Some(col) = nav.focused() {
                    let total_items = col.items.len();
                    if total_items > 0 {
                        let inner_height = state.terminal_height.saturating_sub(4) as usize;
                        let target_visible = 3usize.max(total_items.min(5));
                        let row_height = if target_visible > 0 { (inner_height / target_visible).max(3) } else { 3 };
                        let visible_rows = if row_height > 0 { (inner_height / row_height).max(1) } else { 1 };
                        let scroll_offset = crate::services::NavigationService::calc_scroll_offset(
                            col.selected_index, visible_rows, total_items,
                        );
                        let end = (scroll_offset + visible_rows).min(total_items);

                        let mut to_load: Vec<(String, String)> = Vec::new();
                        for item in &col.items[scroll_offset..end] {
                            if to_load.len() >= 4 { break; }
                            match item {
                                crate::app::state::BrowseItem::Album { key, thumb: Some(thumb), .. } => {
                                    if !state.album_art_cache.contains_key(key)
                                        && !state.album_art_pending.contains(key)
                                    {
                                        to_load.push((key.clone(), thumb.clone()));
                                    }
                                }
                                crate::app::state::BrowseItem::AllTracks { artist_key, thumb: Some(thumb), .. } => {
                                    if !state.album_art_cache.contains_key(artist_key)
                                        && !state.album_art_pending.contains(artist_key)
                                    {
                                        to_load.push((artist_key.clone(), thumb.clone()));
                                    }
                                }
                                _ => {}
                            }
                        }

                        if !to_load.is_empty() {
                            return Ok(vec![Action::LoadAlbumArt(to_load)]);
                        }
                    }
                }
            }
        }
        Action::LoadAlbumArt(batch) => {
            let artwork_cache = crate::plex::ArtworkCache::default();
            let warm_threshold = crate::plex::constants::CACHE_VERY_STALE_THRESHOLD_SECS;

            for (key, thumb_path) in batch {
                if state.album_art_pending.contains(&key) {
                    continue;
                }

                // Check disk cache with warm support (no TTL deletion, serve stale entries)
                if let Some((data, is_warm)) = artwork_cache.load_warm(&key, warm_threshold) {
                    state.album_art_cache.insert(key.clone(), data);

                    // If warm (>= 32 days), re-fetch in background to update the cache file
                    if is_warm {
                        let event_tx = event_tx.clone();
                        let client = client.clone();
                        let bg_key = key;
                        let bg_thumb = thumb_path;
                        tokio::spawn(async move {
                            match client.fetch_artwork(&bg_thumb, 300).await {
                                Ok(data) => {
                                    let cache = crate::plex::ArtworkCache::default();
                                    cache.save(&bg_key, &data);
                                    // Send updated art to UI
                                    let _ = event_tx.send(Event::AlbumArtLoaded {
                                        key: bg_key,
                                        data,
                                    }).await;
                                }
                                Err(e) => {
                                    tracing::debug!("Warm artwork re-fetch failed for {}: {}", bg_key, e);
                                }
                            }
                        });
                    }
                    continue;
                }

                state.album_art_pending.insert(key.clone());

                let event_tx = event_tx.clone();
                let client = client.clone();

                tokio::spawn(async move {
                    match client.fetch_artwork(&thumb_path, 300).await {
                        Ok(data) => {
                            // Save to disk cache
                            let cache = crate::plex::ArtworkCache::default();
                            cache.save(&key, &data);

                            let _ = event_tx.send(Event::AlbumArtLoaded {
                                key,
                                data,
                            }).await;
                        }
                        Err(e) => {
                            tracing::warn!("Failed to load album art for {}: {}", key, e);
                            let _ = event_tx.send(Event::AlbumArtFailed { key }).await;
                        }
                    }
                });
            }
        }
        _ => unreachable!("dispatch_system called with non-system action: {:?}", action),
    }
    Ok(vec![])
}
