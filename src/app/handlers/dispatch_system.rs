//! System dispatch handlers: Quit, ShowError, ClearError, SetStatus, ClearStatus,
//! RefreshCategory, CycleTheme, LoadArtwork, LoadWaveform.

use crate::app::event::*;
use crate::app::{Action, AppState, Event};
use crate::app::action::SystemAction;
use crate::plex::PlexClient;
use crate::config::Config;

use anyhow::Result;
use tokio::sync::mpsc;

/// Download audio data from a stream URL for analysis (waveform/spectrogram generation).
async fn download_audio_for_analysis(stream_url: &str, token: Option<&str>) -> Result<Vec<u8>, String> {
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default();
    let mut request = http_client.get(stream_url);
    if let Some(token) = token {
        request = request.header("X-Plex-Token", token);
    }
    let response = request.send().await
        .map_err(|e| format!("Request failed: {}", e))?;
    let bytes = response.bytes().await
        .map_err(|e| format!("Download failed: {}", e))?;
    Ok(bytes.to_vec())
}

use super::helpers;

/// Dispatch system-level actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    config: &mut Config,
    action: SystemAction,
    state: &mut AppState,
    client: &mut PlexClient,
) -> Result<Vec<Action>> {
    match action {
        SystemAction::Quit => {
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
            if let crate::app::state::OutputTarget::Remote { ref player_id, ref player_uri, .. } = state.remote.output_target {
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

            // Build cache data to save after terminal is restored (deferred for fast quit).
            // Skip if nothing has changed since last save (cache_dirty is false).
            if state.cache_mgmt.dirty {
            if let Some(lib_key) = &state.active_library {
                use crate::plex::CacheData;

                let mut cache_data = CacheData::new(lib_key);
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

                // Core library data
                cache_data.artists = state.library.artists.clone();
                cache_data.albums = state.library.albums.clone();
                cache_data.playlists = state.library.playlists.clone();

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
                // Save cached subfolder contents (keep all if keep_subfolder_cache, else purge > 32 days)
                cache_data.folder_contents = if state.keep_subfolder_cache {
                    state.folder_contents_cache.clone()
                } else {
                    state.folder_contents_cache.iter()
                        .filter(|(_, cached)| !cached.is_older_than(crate::plex::constants::CACHE_VERY_STALE_THRESHOLD_SECS))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                };

                // Genre/mood/style data
                cache_data.genres = state.library.album_genres.clone();
                cache_data.artist_genres = state.library.artist_genres.clone();
                cache_data.album_genres = state.library.album_genres.clone();
                cache_data.moods = state.library.moods.clone();
                cache_data.styles = state.library.styles.clone();

                // Stations — save root column (not state.stations which may be drilled children)
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

                state.pending_cache_save = Some(cache_data);
            }
            } // cache_dirty

            state.should_quit = true;
        }
        SystemAction::ShowError(msg) => {
            state.set_error(msg);
        }
        SystemAction::ClearError => {
            state.clear_error();
        }
        SystemAction::SetStatus(msg) => {
            state.set_status(msg);
        }
        SystemAction::ClearStatus => {
            state.clear_status();
        }
        SystemAction::RefreshCategory(category) => {
            if let Some(lib_key) = &state.active_library {
                let lib_key = lib_key.clone();
                helpers::spawn_category_refresh(event_tx, category, &lib_key, state, client);
            }
        }
        SystemAction::CheckStaleness(tier1_category) => {
            helpers::check_staleness_on_view_load(event_tx, state, client, tier1_category);
        }
        SystemAction::LoadArtwork => {
            // Get thumb path from current track (clone to avoid borrow)
            let thumb_path = state.current_track()
                .and_then(|t| t.best_thumb().map(|s| s.to_string()));

            if let Some(thumb_path) = thumb_path {
                // Check if we need to load new artwork
                if state.artwork.current_thumb.as_deref() != Some(&thumb_path) {
                    state.artwork.loading = true;
                    match client.fetch_artwork(&thumb_path, 300).await {
                        Ok(data) => {
                            state.artwork.current_thumb = Some(thumb_path);
                            state.artwork.current_data = Some(data);
                        }
                        Err(e) => {
                            tracing::warn!("Failed to load artwork: {}", e);
                            state.artwork.current_thumb = None;
                            state.artwork.current_data = None;
                        }
                    }
                    state.artwork.loading = false;
                }
            } else {
                // No artwork available or no current track
                state.artwork.current_thumb = None;
                state.artwork.current_data = None;
            }
        }
        SystemAction::LoadWaveform => {
            // Only generate waveform if we have a track and don't already have data
            if let Some(track) = state.current_track().cloned() {
                // Self-correcting track_key sync. Without this the gate
                // below silently no-ops when the cached `track_key` is
                // stale (e.g. a previous track), and the only thing
                // that ever fixes it is the Tick safety-net on
                // `View::NowPlaying` — which never runs on the Queue
                // view's visualizer toggle. Resetting here makes
                // `LoadWaveform` work from any view that dispatches it.
                if state.waveform.track_key.as_ref() != Some(&track.rating_key) {
                    state.waveform = crate::app::state::WaveformState::default();
                    state.waveform.track_key = Some(track.rating_key.clone());
                    state.spectrogram = crate::app::state::SpectrogramState::default();
                    state.spectrogram.track_key = Some(track.rating_key.clone());
                }
                let needs_generation = state.waveform.data.is_none()
                    && !state.waveform.generating;

                if needs_generation {
                    state.waveform.generating = true;
                    // Also mark spectrogram as generating if it needs data
                    let also_generate_spectrogram = state.spectrogram.data.is_none()
                        && !state.spectrogram.generating;
                    if also_generate_spectrogram {
                        state.spectrogram.generating = true;
                    }
                    let track_key = track.rating_key.clone();
                    let duration_ms = track.duration_ms();
                    let event_tx = event_tx.clone();

                    // Get the stream URL synchronously — if it fails
                    // (no active server, missing token, etc.) we MUST
                    // emit a Failed event so `generating` clears.
                    // Without this, the panel is stuck on "Generating…"
                    // forever and the only fix is a track change.
                    let stream_url = match client.get_stream_url(&track) {
                        Ok(url) => url,
                        Err(e) => {
                            let err_msg = format!("stream URL unavailable: {}", e);
                            let track_key_err = track_key.clone();
                            let event_tx_err = event_tx.clone();
                            let also_sg = also_generate_spectrogram;
                            tokio::spawn(async move {
                                let _ = event_tx_err.send(VisualizerEvent::WaveformFailed {
                                    track_key: track_key_err.clone(),
                                    error: err_msg.clone(),
                                }.into()).await;
                                if also_sg {
                                    let _ = event_tx_err.send(VisualizerEvent::SpectrogramFailed {
                                        track_key: track_key_err,
                                        error: err_msg,
                                    }.into()).await;
                                }
                            });
                            return Ok(vec![]);
                        }
                    };
                    {
                        let token = client.token().map(|s| s.to_string());

                        tokio::spawn(async move {
                            // Check waveform cache first
                            let waveform_cache_dir = dirs::cache_dir()
                                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                                .join("textamp")
                                .join("waveforms");
                            let waveform_cache = crate::services::WaveformCache::new(waveform_cache_dir);

                            let spectrogram_cache_dir = dirs::cache_dir()
                                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                                .join("textamp")
                                .join("spectrograms");
                            let spectrogram_cache = crate::services::SpectrogramCache::new(spectrogram_cache_dir);

                            // Try waveform cache
                            let waveform_cached = waveform_cache.load(&track_key);
                            if let Some(data) = waveform_cached {
                                let _ = event_tx.send(VisualizerEvent::WaveformCacheHit {
                                    track_key: track_key.clone(),
                                    data,
                                }.into()).await;

                                // Check spectrogram cache; if miss, leave it for LoadSpectrogram
                                // (triggered by tick safety net) rather than downloading here.
                                if also_generate_spectrogram {
                                    if let Some(sg_data) = spectrogram_cache.load(&track_key) {
                                        let _ = event_tx.send(VisualizerEvent::SpectrogramCacheHit {
                                            track_key,
                                            data: sg_data,
                                        }.into()).await;
                                    } else {
                                        // Signal that spectrogram still needs work —
                                        // SpectrogramFailed clears generating so the tick
                                        // safety net triggers LoadSpectrogram independently.
                                        let _ = event_tx.send(VisualizerEvent::SpectrogramFailed {
                                            track_key,
                                            error: String::new(),
                                        }.into()).await;
                                    }
                                }
                                return;
                            }

                            // Check spectrogram cache too
                            let spectrogram_cached = if also_generate_spectrogram {
                                spectrogram_cache.load(&track_key)
                            } else {
                                None
                            };
                            if let Some(sg_data) = &spectrogram_cached {
                                let _ = event_tx.send(VisualizerEvent::SpectrogramCacheHit {
                                    track_key: track_key.clone(),
                                    data: sg_data.clone(),
                                }.into()).await;
                            }

                            // Download audio with timeout and generate waveform (+ spectrogram if not cached)
                            match download_audio_for_analysis(&stream_url, token.as_deref()).await {
                                Ok(audio_data) => {
                                    match crate::services::generate_waveform(
                                        track_key.clone(),
                                        duration_ms,
                                        audio_data.clone(),
                                    ) {
                                        Ok(data) => {
                                            waveform_cache.save(&data);
                                            let _ = event_tx.send(VisualizerEvent::WaveformGenerated {
                                                track_key: track_key.clone(),
                                                data,
                                            }.into()).await;
                                        }
                                        Err(e) => {
                                            let _ = event_tx.send(VisualizerEvent::WaveformFailed {
                                                track_key: track_key.clone(),
                                                error: e.to_string(),
                                            }.into()).await;
                                        }
                                    }

                                    // Co-compute spectrogram from same audio data if not cached
                                    if also_generate_spectrogram && spectrogram_cached.is_none() {
                                        match crate::services::generate_spectrogram(
                                            track_key.clone(), duration_ms, audio_data,
                                        ) {
                                            Ok(sg_data) => {
                                                spectrogram_cache.save(&sg_data);
                                                let _ = event_tx.send(VisualizerEvent::SpectrogramGenerated {
                                                    track_key, data: sg_data,
                                                }.into()).await;
                                            }
                                            Err(e) => {
                                                let _ = event_tx.send(VisualizerEvent::SpectrogramFailed {
                                                    track_key, error: e.to_string(),
                                                }.into()).await;
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = event_tx.send(VisualizerEvent::WaveformFailed {
                                        track_key: track_key.clone(),
                                        error: e.clone(),
                                    }.into()).await;
                                    if also_generate_spectrogram && spectrogram_cached.is_none() {
                                        let _ = event_tx.send(VisualizerEvent::SpectrogramFailed {
                                            track_key, error: e,
                                        }.into()).await;
                                    }
                                }
                            }
                        });
                    }
                }
            }
        }
        SystemAction::LoadSpectrogram => {
            // Load spectrogram data — check cache first, then generate if needed.
            // Generation is normally co-computed with waveform, but if waveform is
            // already loaded (e.g., re-entering NowPlaying), we download independently.
            if let Some(track) = state.current_track().cloned() {
                // Self-correcting track_key sync (same reasoning as
                // `LoadWaveform` above — gate must not silently no-op
                // on stale state when called from a non-NowPlaying view).
                if state.spectrogram.track_key.as_ref() != Some(&track.rating_key) {
                    state.spectrogram = crate::app::state::SpectrogramState::default();
                    state.spectrogram.track_key = Some(track.rating_key.clone());
                }
                let needs_generation = state.spectrogram.data.is_none()
                    && !state.spectrogram.generating;

                if needs_generation {
                    // Check cache first
                    let cache_dir = dirs::cache_dir()
                        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                        .join("textamp")
                        .join("spectrograms");
                    let cache = crate::services::SpectrogramCache::new(cache_dir);

                    if let Some(data) = cache.load(&track.rating_key) {
                        state.spectrogram.data = Some(data);
                        state.spectrogram.generating = false;
                        state.spectrogram.error = None;
                    } else if state.waveform.data.is_none() && !state.waveform.generating {
                        // Neither waveform nor spectrogram — trigger LoadWaveform to co-compute
                        return Ok(vec![SystemAction::LoadWaveform.into()]);
                    } else if state.waveform.generating {
                        // Waveform is being generated right now — it will co-compute spectrogram
                        state.spectrogram.generating = true;
                    } else {
                        // Waveform already loaded but no spectrogram — download independently
                        state.spectrogram.generating = true;
                        state.spectrogram.error = None;
                        let track_key = track.rating_key.clone();
                        let duration_ms = track.duration_ms();
                        let event_tx = event_tx.clone();

                        // Same defensive failure path as `LoadWaveform`:
                        // if get_stream_url fails synchronously we
                        // MUST emit `SpectrogramFailed`, otherwise
                        // `generating` stays true and the panel is
                        // stuck on "Generating spectrogram…".
                        let stream_url = match client.get_stream_url(&track) {
                            Ok(url) => url,
                            Err(e) => {
                                let err_msg = format!("stream URL unavailable: {}", e);
                                let track_key_err = track_key.clone();
                                let event_tx_err = event_tx.clone();
                                tokio::spawn(async move {
                                    let _ = event_tx_err.send(VisualizerEvent::SpectrogramFailed {
                                        track_key: track_key_err,
                                        error: err_msg,
                                    }.into()).await;
                                });
                                return Ok(vec![]);
                            }
                        };
                        {
                            let token = client.token().map(|s| s.to_string());

                            tokio::spawn(async move {
                                match download_audio_for_analysis(&stream_url, token.as_deref()).await {
                                    Ok(audio_data) => {
                                        match crate::services::generate_spectrogram(
                                            track_key.clone(), duration_ms, audio_data,
                                        ) {
                                            Ok(data) => {
                                                let sg_cache_dir = dirs::cache_dir()
                                                    .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                                                    .join("textamp")
                                                    .join("spectrograms");
                                                let sg_cache = crate::services::SpectrogramCache::new(sg_cache_dir);
                                                sg_cache.save(&data);
                                                let _ = event_tx.send(VisualizerEvent::SpectrogramGenerated {
                                                    track_key, data,
                                                }.into()).await;
                                            }
                                            Err(e) => {
                                                let _ = event_tx.send(VisualizerEvent::SpectrogramFailed {
                                                    track_key, error: e.to_string(),
                                                }.into()).await;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let _ = event_tx.send(VisualizerEvent::SpectrogramFailed {
                                            track_key, error: e,
                                        }.into()).await;
                                    }
                                }
                            });
                        }
                    }
                }
            }
        }
        SystemAction::LoadAlbumArt(batch) => {
            // Lazy-load gate: while the GUI flags rapid input motion,
            // skip the synchronous disk-cache reads + spawn so the UI
            // thread isn't stalled scrolling. The GUI re-fires this
            // action against the current viewport once motion settles.
            if state.artwork.suppress_loads {
                return Ok(vec![]);
            }
            let artwork_cache = crate::plex::ArtworkCache::default();
            let warm_threshold = crate::plex::constants::CACHE_VERY_STALE_THRESHOLD_SECS;

            for (key, thumb_path) in batch {
                if state.artwork.grid_pending.contains(&key) {
                    continue;
                }

                // Check disk cache with warm support (no TTL deletion, serve stale entries)
                if let Some((data, is_warm)) = artwork_cache.load_warm(&key, warm_threshold) {
                    state.artwork.grid_cache.insert(key.clone(), data);

                    // If warm (>= 32 days), re-fetch in background to update the cache file
                    if is_warm {
                        let event_tx = event_tx.clone();
                        let client = client.clone();
                        let bg_key = key;
                        let bg_thumb = thumb_path;
                        tokio::spawn(async move {
                            match client.fetch_artwork(&bg_thumb, 600).await {
                                Ok(data) => {
                                    let cache = crate::plex::ArtworkCache::default();
                                    cache.save(&bg_key, &data);
                                    // Send updated art to UI
                                    let _ = event_tx.send(ArtworkEvent::AlbumArtLoaded {
                                        key: bg_key,
                                        data,
                                    }.into()).await;
                                }
                                Err(e) => {
                                    tracing::debug!("Warm artwork re-fetch failed for {}: {}", bg_key, e);
                                }
                            }
                        });
                    }
                    continue;
                }

                state.artwork.grid_pending.insert(key.clone());

                let event_tx = event_tx.clone();
                let client = client.clone();

                tokio::spawn(async move {
                    match client.fetch_artwork(&thumb_path, 600).await {
                        Ok(data) => {
                            // Save to disk cache
                            let cache = crate::plex::ArtworkCache::default();
                            cache.save(&key, &data);

                            let _ = event_tx.send(ArtworkEvent::AlbumArtLoaded {
                                key,
                                data,
                            }.into()).await;
                        }
                        Err(e) => {
                            tracing::warn!("Failed to load album art for {}: {}", key, e);
                            let _ = event_tx.send(ArtworkEvent::AlbumArtFailed { key }.into()).await;
                        }
                    }
                });
            }
        }
        SystemAction::OpenExternalSearch { target, query } => {
            use crate::services::external_search::SearchTarget;
            let enabled = match target {
                SearchTarget::AppleMusic => config.ui.enable_apple_music_search,
                SearchTarget::Spotify    => config.ui.enable_spotify_search,
                SearchTarget::YouTube    => config.ui.enable_youtube_search,
            };
            if !enabled {
                let name = match target {
                    SearchTarget::AppleMusic => "Apple Music",
                    SearchTarget::Spotify    => "Spotify",
                    SearchTarget::YouTube    => "YouTube",
                };
                state.set_status(format!("{} search is disabled in Settings", name));
                return Ok(vec![]);
            }
            let q = query.unwrap_or_else(|| super::key_input::build_external_search_query(state));
            if q.is_empty() {
                state.set_status("Nothing selected to search".to_string());
                return Ok(vec![]);
            }
            let url = crate::services::external_search::generate_search_url(target, &q);
            let _ = open::that(&url);
        }
    }
    Ok(vec![])
}
