//! System dispatch handlers: Quit, ShowError, ClearError, SetStatus, ClearStatus,
//! RefreshCategory, CycleTheme, LoadArtwork, LoadWaveform.

use crate::app::event::*;
use crate::app::{Action, AppState, Event};
use crate::app::action::SystemAction;
use crate::plex::PlexClient;
use crate::config::Config;

use anyhow::Result;
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Download audio data from a stream URL for analysis (waveform/spectrogram generation).
const MAX_ANALYSIS_AUDIO_BYTES: usize = 128 * 1024 * 1024;

async fn download_audio_for_analysis(
    stream_url: &str,
    headers: reqwest::header::HeaderMap,
    http_client: reqwest::Client,
) -> Result<Arc<[u8]>, String> {
    let response = http_client
        .get(stream_url)
        .headers(headers)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    if !response.status().is_success() {
        return Err(format!("Server returned HTTP {}", response.status()));
    }
    if response.content_length().is_some_and(|length| {
        length > MAX_ANALYSIS_AUDIO_BYTES as u64
    }) {
        return Err(format!(
            "Audio exceeds analysis limit of {} MiB",
            MAX_ANALYSIS_AUDIO_BYTES / (1024 * 1024)
        ));
    }

    let initial_capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or(256 * 1024)
        .min(MAX_ANALYSIS_AUDIO_BYTES);
    let mut bytes = Vec::with_capacity(initial_capacity);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("Download failed: {}", error))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_ANALYSIS_AUDIO_BYTES {
            return Err(format!(
                "Audio exceeds analysis limit of {} MiB",
                MAX_ANALYSIS_AUDIO_BYTES / (1024 * 1024)
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Arc::from(bytes))
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
            // Flush local timeline state and remote stop as structured work.
            // Both operations share one hard deadline, so shutdown is reliable
            // without sleeping the UI/runtime thread or abandoning spawned tasks.
            let local_track = (state.playback.status
                != crate::app::state::PlayStatus::Stopped)
                .then(|| state.current_track().cloned())
                .flatten();
            let local_position = state.playback.position_ms;
            let local_session = state.plex_session_id.clone();
            let should_scrobble = local_track.is_some()
                && !state.playback.scrobble_reported
                && state.playback.duration_ms > 0
                && state.playback.position_ms.saturating_mul(10)
                    >= state.playback.duration_ms.saturating_mul(9);
            if should_scrobble {
                state.playback.scrobble_reported = true;
            }
            let local_client = client.clone();

            let remote_target = match &state.remote.output_target {
                crate::app::state::OutputTarget::Remote { player_id, player_uri, .. } => {
                    Some((player_id.clone(), player_uri.clone()))
                }
                crate::app::state::OutputTarget::Local => None,
            };
            let remote_token = client.shared_token_or_empty();
            let remote_client_id = client.client_identifier().to_string();
            let remote_server_url = client.server_url().unwrap_or_default().to_string();
            let remote_machine_id = state.active_server_id.clone()
                .or_else(|| state.available_servers.first()
                    .map(|server| server.client_identifier.clone()))
                .unwrap_or_default();

            let flush = async move {
                let local = async move {
                    if let Some(track) = local_track {
                        if should_scrobble {
                            if let Err(error) = local_client.scrobble(&track.rating_key).await {
                                tracing::warn!("Final scrobble report failed: {}", error);
                            }
                        }
                        if let Err(error) = local_client.report_playback_stop(
                            &track,
                            local_position,
                            false,
                            local_session.as_deref(),
                        ).await {
                            tracing::warn!("Final playback stop report failed: {}", error);
                        }
                    }
                };
                let remote = async move {
                    if let Some((player_id, player_uri)) = remote_target {
                        match crate::plex::RemotePlayerClient::new(
                            remote_token,
                            remote_client_id,
                            player_id,
                            remote_server_url,
                            remote_machine_id,
                            player_uri,
                        ) {
                            Ok(remote_client) => {
                                if let Err(error) = remote_client.stop().await {
                                    tracing::warn!("Final remote stop failed: {}", error);
                                }
                            }
                            Err(error) => tracing::warn!(
                                "Cannot initialize remote client during shutdown: {}",
                                error
                            ),
                        }
                    }
                };
                tokio::join!(local, remote);
            };
            if tokio::time::timeout(std::time::Duration::from_secs(1), flush)
                .await
                .is_err()
            {
                tracing::warn!("Timed out flushing playback state during shutdown");
            }

            // Build cache data to save after terminal is restored (deferred for fast quit).
            // Skip if nothing has changed since last save (cache_dirty is false).
            if state.cache_mgmt.dirty {
            if let Some(lib_key) = state.active_library.clone() {
                use crate::plex::CacheData;

                let mut cache_data =
                    CacheData::new_scoped(&lib_key, state.active_server_id.as_deref());
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
                let smart_playlist_keys: std::collections::HashSet<String> = state
                    .library
                    .playlists
                    .iter()
                    .filter(|playlist| playlist.smart)
                    .map(|playlist| playlist.rating_key.clone())
                    .collect();
                cache_data.artists = std::mem::take(&mut state.library.artists);
                cache_data.albums = std::mem::take(&mut state.library.albums);
                cache_data.playlists = std::mem::take(&mut state.library.playlists);

                // Folder data - extract root folder items only if they belong to this library
                if let Some(ref folder_state) = state.folder_state {
                    if folder_state.library_key == lib_key {
                        if let Some(root_col) = folder_state.columns.first() {
                            cache_data.root_folders = root_col.unshuffled_items().to_vec();
                        }
                    } else {
                        tracing::debug!("Not saving folder_state on quit - belongs to different library (expected {}, got {})",
                            lib_key, folder_state.library_key);
                    }
                }
                // Save cached subfolder contents (keep all if keep_subfolder_cache, else purge > 32 days)
                cache_data.folder_contents = std::mem::take(&mut state.folder_contents_cache);
                if !state.keep_subfolder_cache {
                    cache_data.folder_contents.retain(|_, cached| {
                        !cached.is_older_than(
                            crate::plex::constants::CACHE_VERY_STALE_THRESHOLD_SECS,
                        )
                    });
                }

                // Genre/mood/style data
                cache_data.album_genres = std::mem::take(&mut state.library.album_genres);
                cache_data.genres = cache_data.album_genres.clone();
                cache_data.artist_genres = std::mem::take(&mut state.library.artist_genres);
                cache_data.moods = std::mem::take(&mut state.library.moods);
                cache_data.styles = std::mem::take(&mut state.library.styles);
                cache_data.decades = std::mem::take(&mut state.library.decades);
                cache_data.years = std::mem::take(&mut state.library.years);
                cache_data.collections = std::mem::take(&mut state.library.collections);
                cache_data.countries = std::mem::take(&mut state.library.countries);
                cache_data.labels = std::mem::take(&mut state.library.labels);
                cache_data.formats = std::mem::take(&mut state.library.formats);
                cache_data.studios = std::mem::take(&mut state.library.studios);

                // Stations — save root column (not state.stations which may be drilled children)
                if let Some(root) = state.station_nav.columns.first_mut() {
                    root.unshuffle();
                    cache_data.stations = std::mem::take(&mut root.stations);
                }
                cache_data.station_children = std::mem::take(&mut state.station_children_cache);

                // All tracks + track-level artists + aliases
                // Only save if non-empty to avoid overwriting cached data when preload is in-flight
                if !state.library.all_tracks.is_empty() {
                    cache_data.all_tracks = std::mem::take(&mut state.library.all_tracks);
                    cache_data.track_artists = std::mem::take(&mut state.library.track_artists);
                }
                cache_data.artist_aliases = std::mem::take(&mut state.library.artist_aliases);
                cache_data.album_display_artist = std::mem::take(&mut state.library.album_display_artist);

                // Compilation detection results
                cache_data.compilation_albums = std::mem::take(&mut state.library.compilations.albums);
                cache_data.compilation_artist_keys = std::mem::take(&mut state.library.compilations.artist_keys);
                cache_data.compilation_track_artist_keys = std::mem::take(&mut state.library.compilations.track_artist_keys);
                cache_data.artist_compilation_map = std::mem::take(&mut state.library.compilations.artist_map);
                cache_data.single_artist_compilations = std::mem::take(&mut state.library.compilations.single_artist);

                // Save non-smart playlist tracks to disk cache
                cache_data.playlist_tracks = std::mem::take(&mut state.playlist_tracks_cache);
                cache_data
                    .playlist_tracks
                    .retain(|key, _| !smart_playlist_keys.contains(key));

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
                    state.artwork.pending_thumb = Some(thumb_path.clone());
                    let event_tx = event_tx.clone();
                    let client = client.clone();
                    let generation = state.artwork.grid_generation;
                    tokio::spawn(async move {
                        let event: Event = match client.fetch_artwork(&thumb_path, 300).await {
                            Ok(data) => ArtworkEvent::ArtworkLoaded {
                                generation,
                                thumb_path,
                                data,
                            }.into(),
                            Err(error) => {
                                tracing::warn!("Failed to load artwork: {}", error);
                                ArtworkEvent::ArtworkFailed {
                                    generation,
                                    thumb_path,
                                }.into()
                            }
                        };
                        let _ = event_tx.send(event).await;
                    });
                }
            } else {
                // No artwork available or no current track
                state.artwork.current_thumb = None;
                state.artwork.current_data = None;
                state.artwork.pending_thumb = None;
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
                    let event_tx = LibraryEventSender::new(
                        event_tx.clone(),
                        state.library_generation,
                    );
                    let cache_scope = format!(
                        "{}\0{}",
                        state.active_server_id.as_deref()
                            .or_else(|| client.server_url())
                            .unwrap_or("unknown-server"),
                        state.active_library.as_deref().unwrap_or("unknown-library"),
                    );

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
                    let stream_headers = client.stream_headers();
                    let http_client = client.http_client().clone();
                    tokio::spawn(async move {
                        let cache_root = crate::config::XdgPaths::new("textamp").cache_dir;
                        let waveform_cache_dir = cache_root.join("waveforms");
                        let spectrogram_cache_dir = cache_root.join("spectrograms");
                        let cache_track_key = track_key.clone();
                        let cache_read_scope = cache_scope.clone();
                        let waveform_read_dir = waveform_cache_dir.clone();
                        let spectrogram_read_dir = spectrogram_cache_dir.clone();

                        let cached = tokio::task::spawn_blocking(move || {
                            let waveform = crate::services::WaveformCache::new(waveform_read_dir)
                                .load_scoped(&cache_read_scope, &cache_track_key);
                            let spectrogram = also_generate_spectrogram.then(|| {
                                crate::services::SpectrogramCache::new(spectrogram_read_dir)
                                    .load_scoped(&cache_read_scope, &cache_track_key)
                            }).flatten();
                            (waveform, spectrogram)
                        }).await;
                        let (waveform_cached, spectrogram_cached) = match cached {
                            Ok(cached) => cached,
                            Err(error) => {
                                let message = format!("cache worker failed: {}", error);
                                let _ = event_tx.send(VisualizerEvent::WaveformFailed {
                                    track_key: track_key.clone(),
                                    error: message.clone(),
                                }.into()).await;
                                if also_generate_spectrogram {
                                    let _ = event_tx.send(VisualizerEvent::SpectrogramFailed {
                                        track_key,
                                        error: message,
                                    }.into()).await;
                                }
                                return;
                            }
                        };

                        if let Some(data) = waveform_cached {
                            let _ = event_tx.send(VisualizerEvent::WaveformCacheHit {
                                track_key: track_key.clone(),
                                data,
                            }.into()).await;
                            if also_generate_spectrogram {
                                let event = match spectrogram_cached {
                                    Some(data) => VisualizerEvent::SpectrogramCacheHit {
                                        track_key,
                                        data,
                                    },
                                    None => VisualizerEvent::SpectrogramFailed {
                                        track_key,
                                        error: String::new(),
                                    },
                                };
                                let _ = event_tx.send(event.into()).await;
                            }
                            return;
                        }

                        if let Some(data) = spectrogram_cached.as_ref() {
                            let _ = event_tx.send(VisualizerEvent::SpectrogramCacheHit {
                                track_key: track_key.clone(),
                                data: data.clone(),
                            }.into()).await;
                        }
                        let generate_spectrogram = also_generate_spectrogram
                            && spectrogram_cached.is_none();

                        let audio_data = match download_audio_for_analysis(
                            &stream_url,
                            stream_headers,
                            http_client,
                        ).await {
                            Ok(data) => data,
                            Err(error) => {
                                let _ = event_tx.send(VisualizerEvent::WaveformFailed {
                                    track_key: track_key.clone(),
                                    error: error.clone(),
                                }.into()).await;
                                if generate_spectrogram {
                                    let _ = event_tx.send(VisualizerEvent::SpectrogramFailed {
                                        track_key,
                                        error,
                                    }.into()).await;
                                }
                                return;
                            }
                        };

                        let computation_key = track_key.clone();
                        let computation = tokio::task::spawn_blocking(move || {
                            match crate::services::decode_to_pcm(audio_data) {
                                Ok((samples, sample_rate)) => {
                                    let waveform = Ok(crate::services::generate_waveform_from_pcm(
                                        computation_key.clone(),
                                        duration_ms,
                                        &samples,
                                    ));
                                    if let Ok(data) = &waveform {
                                        crate::services::WaveformCache::new(waveform_cache_dir)
                                            .save_scoped(&cache_scope, data);
                                    }

                                    let spectrogram = generate_spectrogram.then(|| {
                                        let data = crate::services::generate_spectrogram_from_pcm(
                                            computation_key,
                                            duration_ms,
                                            &samples,
                                            sample_rate,
                                        );
                                        crate::services::SpectrogramCache::new(
                                            spectrogram_cache_dir,
                                        )
                                        .save_scoped(&cache_scope, &data);
                                        Ok(data)
                                    });
                                    (waveform, spectrogram)
                                }
                                Err(error) => {
                                    let error = error.to_string();
                                    (
                                        Err(error.clone()),
                                        generate_spectrogram.then(|| Err(error)),
                                    )
                                }
                            }
                        }).await;

                        match computation {
                            Ok((waveform, spectrogram)) => {
                                let waveform_event = match waveform {
                                    Ok(data) => VisualizerEvent::WaveformGenerated {
                                        track_key: track_key.clone(),
                                        data,
                                    },
                                    Err(error) => VisualizerEvent::WaveformFailed {
                                        track_key: track_key.clone(),
                                        error,
                                    },
                                };
                                let _ = event_tx.send(waveform_event.into()).await;
                                if let Some(spectrogram) = spectrogram {
                                    let event = match spectrogram {
                                        Ok(data) => VisualizerEvent::SpectrogramGenerated {
                                            track_key,
                                            data,
                                        },
                                        Err(error) => VisualizerEvent::SpectrogramFailed {
                                            track_key,
                                            error,
                                        },
                                    };
                                    let _ = event_tx.send(event.into()).await;
                                }
                            }
                            Err(error) => {
                                let message = format!("audio analysis worker failed: {}", error);
                                let _ = event_tx.send(VisualizerEvent::WaveformFailed {
                                    track_key: track_key.clone(),
                                    error: message.clone(),
                                }.into()).await;
                                if generate_spectrogram {
                                    let _ = event_tx.send(VisualizerEvent::SpectrogramFailed {
                                        track_key,
                                        error: message,
                                    }.into()).await;
                                }
                            }
                        }
                    });
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
                    if state.waveform.data.is_none() && !state.waveform.generating {
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
                        let event_tx = LibraryEventSender::new(
                            event_tx.clone(),
                            state.library_generation,
                        );
                        let cache_scope = format!(
                            "{}\0{}",
                            state.active_server_id.as_deref()
                                .or_else(|| client.server_url())
                                .unwrap_or("unknown-server"),
                            state.active_library.as_deref().unwrap_or("unknown-library"),
                        );

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
                        let stream_headers = client.stream_headers();
                        let http_client = client.http_client().clone();
                        tokio::spawn(async move {
                            let cache_dir = crate::config::XdgPaths::new("textamp")
                                .cache_dir
                                .join("spectrograms");
                            let read_dir = cache_dir.clone();
                            let read_key = track_key.clone();
                            let read_scope = cache_scope.clone();
                            match tokio::task::spawn_blocking(move || {
                                crate::services::SpectrogramCache::new(read_dir)
                                    .load_scoped(&read_scope, &read_key)
                            }).await {
                                Ok(Some(data)) => {
                                    let _ = event_tx.send(VisualizerEvent::SpectrogramCacheHit {
                                        track_key,
                                        data,
                                    }.into()).await;
                                    return;
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    let _ = event_tx.send(VisualizerEvent::SpectrogramFailed {
                                        track_key,
                                        error: format!("cache worker failed: {}", error),
                                    }.into()).await;
                                    return;
                                }
                            }

                            let audio_data = match download_audio_for_analysis(
                                &stream_url,
                                stream_headers,
                                http_client,
                            ).await {
                                Ok(data) => data,
                                Err(error) => {
                                    let _ = event_tx.send(VisualizerEvent::SpectrogramFailed {
                                        track_key,
                                        error,
                                    }.into()).await;
                                    return;
                                }
                            };
                            let computation_key = track_key.clone();
                            let result = tokio::task::spawn_blocking(move || {
                                let result = crate::services::generate_spectrogram(
                                    computation_key,
                                    duration_ms,
                                    audio_data,
                                ).map_err(|error| error.to_string());
                                if let Ok(data) = &result {
                                    crate::services::SpectrogramCache::new(cache_dir)
                                        .save_scoped(&cache_scope, data);
                                }
                                result
                            }).await;
                            let event = match result {
                                Ok(Ok(data)) => VisualizerEvent::SpectrogramGenerated {
                                    track_key,
                                    data,
                                },
                                Ok(Err(error)) => VisualizerEvent::SpectrogramFailed {
                                    track_key,
                                    error,
                                },
                                Err(error) => VisualizerEvent::SpectrogramFailed {
                                    track_key,
                                    error: format!("audio analysis worker failed: {}", error),
                                },
                            };
                            let _ = event_tx.send(event.into()).await;
                        });
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
            let warm_threshold = crate::plex::constants::CACHE_VERY_STALE_THRESHOLD_SECS;
            let generation = state.artwork.grid_generation;
            let cache_scope = format!(
                "{}:{}",
                state.active_server_id.as_deref().unwrap_or("unknown-server"),
                state.active_library.as_deref().unwrap_or("unknown-library")
            );

            for (key, thumb_path) in batch {
                if state.artwork.grid_pending.contains(&key) {
                    continue;
                }
                state.artwork.grid_pending.insert(key.clone());
                let event_tx = event_tx.clone();
                let client = client.clone();
                let disk_key = format!("{}:{}", cache_scope, key);

                tokio::spawn(async move {
                    let read_key = disk_key.clone();
                    let cached = tokio::task::spawn_blocking(move || {
                        crate::plex::ArtworkCache::default()
                            .load_warm(&read_key, warm_threshold)
                    }).await;
                    match cached {
                        Ok(Some((data, is_warm))) => {
                            let _ = event_tx.send(ArtworkEvent::AlbumArtLoaded {
                                generation,
                                key: key.clone(),
                                data,
                            }.into()).await;
                            if !is_warm {
                                return;
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            tracing::warn!("Artwork cache worker failed for {}: {}", key, error);
                        }
                    }

                    match client.fetch_artwork(&thumb_path, 600).await {
                        Ok(data) => {
                            let save_key = disk_key;
                            let save_data = data.clone();
                            let _ = tokio::task::spawn_blocking(move || {
                                crate::plex::ArtworkCache::default().save(&save_key, &save_data)
                            }).await;
                            let _ = event_tx.send(ArtworkEvent::AlbumArtLoaded {
                                generation,
                                key,
                                data,
                            }.into()).await;
                        }
                        Err(error) => {
                            tracing::warn!("Failed to load album art for {}: {}", key, error);
                            let _ = event_tx.send(ArtworkEvent::AlbumArtFailed {
                                generation,
                                key,
                            }.into()).await;
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
