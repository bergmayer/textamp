//! System dispatch handlers: Quit, ShowError, ClearError, SetStatus, ClearStatus,
//! RefreshCategory, CycleTheme, LoadArtwork, LoadWaveform.

use crate::app::action::SystemAction;
use crate::app::event::*;
use crate::app::{Action, AppState, Event};
use crate::config::Config;

use anyhow::Result;
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Download audio data from a stream URL for analysis (waveform/spectrogram generation).
const MAX_ANALYSIS_AUDIO_BYTES: usize = 128 * 1024 * 1024;

async fn download_audio_for_analysis(
    prepared_file: Option<crate::library::MediaFile>,
    stream_url: &str,
    headers: reqwest::header::HeaderMap,
    http_client: reqwest::Client,
) -> Result<Arc<[u8]>, String> {
    if let Some(file) = prepared_file {
        use tokio::io::AsyncReadExt;
        let input = tokio::fs::File::open(&file.path)
            .await
            .map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        input
            .take(MAX_ANALYSIS_AUDIO_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| e.to_string())?;
        if bytes.len() > MAX_ANALYSIS_AUDIO_BYTES {
            return Err("Audio exceeds analysis limit of 128 MiB".into());
        }
        return Ok(Arc::from(bytes));
    }
    let response = http_client
        .get(stream_url)
        .headers(headers)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    if !response.status().is_success() {
        return Err(format!("Server returned HTTP {}", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ANALYSIS_AUDIO_BYTES as u64)
    {
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

/// Dispatch system-level actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    config: &mut Config,
    action: SystemAction,
    state: &mut AppState,
) -> Result<Vec<Action>> {
    match action {
        SystemAction::Quit => {
            let navidrome_track = state.sources.active.navidrome().and_then(|session| {
                let track = state.current_track()?;
                match &track.origin {
                    crate::library::track::TrackOrigin::Navidrome { source_id, song_id }
                        if source_id == &session.source.id
                            && state.playback.status != crate::app::state::PlayStatus::Stopped =>
                    {
                        Some((session.client.clone(), song_id.clone()))
                    }
                    _ => None,
                }
            });
            let should_scrobble = navidrome_track.is_some()
                && !state.playback.scrobble_reported
                && state.playback.duration_ms > 0
                && state.playback.position_ms.saturating_mul(10)
                    >= state.playback.duration_ms.saturating_mul(9);
            if should_scrobble {
                state.playback.scrobble_reported = true;
            }
            let flush = async move {
                let navidrome = async move {
                    if let Some((client, id)) = navidrome_track.filter(|_| should_scrobble) {
                        if let Err(error) = client
                            .call("scrobble", &[("id", id), ("submission", "true".into())])
                            .await
                        {
                            tracing::warn!("Final Navidrome scrobble failed: {error}");
                        }
                    }
                };
                navidrome.await;
            };
            if tokio::time::timeout(std::time::Duration::from_secs(1), flush)
                .await
                .is_err()
            {
                tracing::warn!("Timed out flushing playback state during shutdown");
            }

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

        SystemAction::LoadWaveform => {
            // Folder playback prepares/downloads the file first. A request
            // during preparation must not fall through to the server client.
            if state.current_track().is_some_and(|t| {
                matches!(t.origin, crate::library::track::TrackOrigin::Folder { .. })
            }) && state.sources.prepared.is_none()
            {
                return Ok(vec![]);
            }
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
                let needs_generation = state.waveform.data.is_none() && !state.waveform.generating;

                if needs_generation {
                    state.waveform.generating = true;
                    // Also mark spectrogram as generating if it needs data
                    let also_generate_spectrogram =
                        state.spectrogram.data.is_none() && !state.spectrogram.generating;
                    if also_generate_spectrogram {
                        state.spectrogram.generating = true;
                    }
                    let track_key = track.rating_key.clone();
                    let duration_ms = track.duration_ms();
                    let event_tx =
                        LibraryEventSender::new(event_tx.clone(), state.library_generation);
                    let cache_scope = format!(
                        "{}\0{}",
                        state
                            .connected_server_url
                            .as_deref()
                            .unwrap_or("unknown-server"),
                        state.active_library.as_deref().unwrap_or("unknown-library"),
                    );

                    // Get the stream URL synchronously — if it fails
                    // (no active server, missing token, etc.) we MUST
                    // emit a Failed event so `generating` clears.
                    // Without this, the panel is stuck on "Generating…"
                    // forever and the only fix is a track change.
                    let prepared_file = state.sources.prepared.clone().filter(|_| true);
                    let navidrome = state.sources.active.navidrome();
                    let stream_url = match if prepared_file.is_some() {
                        Ok(String::new())
                    } else if let Some(session) = navidrome {
                        session
                            .id(&track.rating_key)
                            .and_then(|id| {
                                session
                                    .client
                                    .url("stream", &[("id", id), ("format", "raw".into())])
                            })
                            .map(|u| u.to_string())
                            .map_err(|e| e.to_string())
                    } else {
                        Err("No playable library selected".to_string())
                    } {
                        Ok(url) => url,
                        Err(e) => {
                            let err_msg = format!("stream URL unavailable: {}", e);
                            let track_key_err = track_key.clone();
                            let event_tx_err = event_tx.clone();
                            let also_sg = also_generate_spectrogram;
                            crate::app::tasks::spawn(async move {
                                let _ = event_tx_err
                                    .send(
                                        VisualizerEvent::WaveformFailed {
                                            track_key: track_key_err.clone(),
                                            error: err_msg.clone(),
                                        }
                                        .into(),
                                    )
                                    .await;
                                if also_sg {
                                    let _ = event_tx_err
                                        .send(
                                            VisualizerEvent::SpectrogramFailed {
                                                track_key: track_key_err,
                                                error: err_msg,
                                            }
                                            .into(),
                                        )
                                        .await;
                                }
                            });
                            return Ok(vec![]);
                        }
                    };
                    let stream_headers = Default::default();
                    let http_client = navidrome.map(|s| s.client.http()).unwrap_or_default();
                    crate::app::tasks::spawn(async move {
                        let cache_root = crate::config::XdgPaths::new("textamp").cache_dir;
                        let waveform_cache_dir = cache_root.join("waveforms");
                        let spectrogram_cache_dir = cache_root.join("spectrograms");
                        let cache_track_key = track_key.clone();
                        let cache_read_scope = cache_scope.clone();
                        let waveform_read_dir = waveform_cache_dir.clone();
                        let spectrogram_read_dir = spectrogram_cache_dir.clone();

                        let cached = crate::app::tasks::spawn_blocking(move || {
                            let waveform = crate::services::WaveformCache::new(waveform_read_dir)
                                .load_scoped(&cache_read_scope, &cache_track_key);
                            let spectrogram = also_generate_spectrogram
                                .then(|| {
                                    crate::services::SpectrogramCache::new(spectrogram_read_dir)
                                        .load_scoped(&cache_read_scope, &cache_track_key)
                                })
                                .flatten();
                            (waveform, spectrogram)
                        })
                        .await;
                        let (waveform_cached, spectrogram_cached) = match cached {
                            Ok(cached) => cached,
                            Err(error) => {
                                let message = format!("cache worker failed: {}", error);
                                let _ = event_tx
                                    .send(
                                        VisualizerEvent::WaveformFailed {
                                            track_key: track_key.clone(),
                                            error: message.clone(),
                                        }
                                        .into(),
                                    )
                                    .await;
                                if also_generate_spectrogram {
                                    let _ = event_tx
                                        .send(
                                            VisualizerEvent::SpectrogramFailed {
                                                track_key,
                                                error: message,
                                            }
                                            .into(),
                                        )
                                        .await;
                                }
                                return;
                            }
                        };

                        if let Some(data) = waveform_cached {
                            let _ = event_tx
                                .send(
                                    VisualizerEvent::WaveformCacheHit {
                                        track_key: track_key.clone(),
                                        data,
                                    }
                                    .into(),
                                )
                                .await;
                            if also_generate_spectrogram {
                                let event = match spectrogram_cached {
                                    Some(data) => {
                                        VisualizerEvent::SpectrogramCacheHit { track_key, data }
                                    }
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
                            let _ = event_tx
                                .send(
                                    VisualizerEvent::SpectrogramCacheHit {
                                        track_key: track_key.clone(),
                                        data: data.clone(),
                                    }
                                    .into(),
                                )
                                .await;
                        }
                        let generate_spectrogram =
                            also_generate_spectrogram && spectrogram_cached.is_none();

                        let audio_data = match download_audio_for_analysis(
                            prepared_file,
                            &stream_url,
                            stream_headers,
                            http_client,
                        )
                        .await
                        {
                            Ok(data) => data,
                            Err(error) => {
                                let _ = event_tx
                                    .send(
                                        VisualizerEvent::WaveformFailed {
                                            track_key: track_key.clone(),
                                            error: error.clone(),
                                        }
                                        .into(),
                                    )
                                    .await;
                                if generate_spectrogram {
                                    let _ = event_tx
                                        .send(
                                            VisualizerEvent::SpectrogramFailed { track_key, error }
                                                .into(),
                                        )
                                        .await;
                                }
                                return;
                            }
                        };

                        let computation_key = track_key.clone();
                        let computation = crate::app::tasks::spawn_blocking(move || {
                            match crate::media::analysis::analyze_audio(
                                computation_key,
                                duration_ms,
                                audio_data,
                                generate_spectrogram,
                            ) {
                                Ok((waveform, spectrogram)) => {
                                    let waveform = Ok(waveform);
                                    if let Ok(data) = &waveform {
                                        crate::services::WaveformCache::new(waveform_cache_dir)
                                            .save_scoped(&cache_scope, data);
                                    }

                                    let spectrogram = spectrogram.map(|data| {
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
                                        generate_spectrogram.then_some(Err(error)),
                                    )
                                }
                            }
                        })
                        .await;

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
                                        Err(error) => {
                                            VisualizerEvent::SpectrogramFailed { track_key, error }
                                        }
                                    };
                                    let _ = event_tx.send(event.into()).await;
                                }
                            }
                            Err(error) => {
                                let message = format!("audio analysis worker failed: {}", error);
                                let _ = event_tx
                                    .send(
                                        VisualizerEvent::WaveformFailed {
                                            track_key: track_key.clone(),
                                            error: message.clone(),
                                        }
                                        .into(),
                                    )
                                    .await;
                                if generate_spectrogram {
                                    let _ = event_tx
                                        .send(
                                            VisualizerEvent::SpectrogramFailed {
                                                track_key,
                                                error: message,
                                            }
                                            .into(),
                                        )
                                        .await;
                                }
                            }
                        }
                    });
                }
            }
        }
        SystemAction::LoadSpectrogram => {
            if state.current_track().is_some_and(|t| {
                matches!(t.origin, crate::library::track::TrackOrigin::Folder { .. })
            }) && state.sources.prepared.is_none()
            {
                return Ok(vec![]);
            }
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
                let needs_generation =
                    state.spectrogram.data.is_none() && !state.spectrogram.generating;

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
                        let event_tx =
                            LibraryEventSender::new(event_tx.clone(), state.library_generation);
                        let cache_scope = format!(
                            "{}\0{}",
                            state
                                .connected_server_url
                                .as_deref()
                                .unwrap_or("unknown-server"),
                            state.active_library.as_deref().unwrap_or("unknown-library"),
                        );

                        // Same defensive failure path as `LoadWaveform`:
                        // if get_stream_url fails synchronously we
                        // MUST emit `SpectrogramFailed`, otherwise
                        // `generating` stays true and the panel is
                        // stuck on "Generating spectrogram…".
                        let prepared_file = state.sources.prepared.clone().filter(|_| true);
                        let navidrome = state.sources.active.navidrome();
                        let stream_url = match if prepared_file.is_some() {
                            Ok(String::new())
                        } else if let Some(session) = navidrome {
                            session
                                .id(&track.rating_key)
                                .and_then(|id| {
                                    session
                                        .client
                                        .url("stream", &[("id", id), ("format", "raw".into())])
                                })
                                .map(|u| u.to_string())
                                .map_err(|e| e.to_string())
                        } else {
                            Err("No playable library selected".to_string())
                        } {
                            Ok(url) => url,
                            Err(e) => {
                                let err_msg = format!("stream URL unavailable: {}", e);
                                let track_key_err = track_key.clone();
                                let event_tx_err = event_tx.clone();
                                crate::app::tasks::spawn(async move {
                                    let _ = event_tx_err
                                        .send(
                                            VisualizerEvent::SpectrogramFailed {
                                                track_key: track_key_err,
                                                error: err_msg,
                                            }
                                            .into(),
                                        )
                                        .await;
                                });
                                return Ok(vec![]);
                            }
                        };
                        let stream_headers = Default::default();
                        let http_client = navidrome.map(|s| s.client.http()).unwrap_or_default();
                        crate::app::tasks::spawn(async move {
                            let cache_dir = crate::config::XdgPaths::new("textamp")
                                .cache_dir
                                .join("spectrograms");
                            let read_dir = cache_dir.clone();
                            let read_key = track_key.clone();
                            let read_scope = cache_scope.clone();
                            match crate::app::tasks::spawn_blocking(move || {
                                crate::services::SpectrogramCache::new(read_dir)
                                    .load_scoped(&read_scope, &read_key)
                            })
                            .await
                            {
                                Ok(Some(data)) => {
                                    let _ = event_tx
                                        .send(
                                            VisualizerEvent::SpectrogramCacheHit {
                                                track_key,
                                                data,
                                            }
                                            .into(),
                                        )
                                        .await;
                                    return;
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    let _ = event_tx
                                        .send(
                                            VisualizerEvent::SpectrogramFailed {
                                                track_key,
                                                error: format!("cache worker failed: {}", error),
                                            }
                                            .into(),
                                        )
                                        .await;
                                    return;
                                }
                            }

                            let audio_data = match download_audio_for_analysis(
                                prepared_file,
                                &stream_url,
                                stream_headers,
                                http_client,
                            )
                            .await
                            {
                                Ok(data) => data,
                                Err(error) => {
                                    let _ = event_tx
                                        .send(
                                            VisualizerEvent::SpectrogramFailed { track_key, error }
                                                .into(),
                                        )
                                        .await;
                                    return;
                                }
                            };
                            let computation_key = track_key.clone();
                            let result = crate::app::tasks::spawn_blocking(move || {
                                let result = crate::services::generate_spectrogram(
                                    computation_key,
                                    duration_ms,
                                    audio_data,
                                )
                                .map_err(|error| error.to_string());
                                if let Ok(data) = &result {
                                    crate::services::SpectrogramCache::new(cache_dir)
                                        .save_scoped(&cache_scope, data);
                                }
                                result
                            })
                            .await;
                            let event = match result {
                                Ok(Ok(data)) => {
                                    VisualizerEvent::SpectrogramGenerated { track_key, data }
                                }
                                Ok(Err(error)) => {
                                    VisualizerEvent::SpectrogramFailed { track_key, error }
                                }
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

        SystemAction::OpenExternalSearch { target, query } => {
            use crate::services::external_search::SearchTarget;
            let enabled = match target {
                SearchTarget::AppleMusic => config.ui.enable_apple_music_search,
                SearchTarget::Spotify => config.ui.enable_spotify_search,
                SearchTarget::YouTube => config.ui.enable_youtube_search,
            };
            if !enabled {
                let name = match target {
                    SearchTarget::AppleMusic => "Apple Music",
                    SearchTarget::Spotify => "Spotify",
                    SearchTarget::YouTube => "YouTube",
                };
                state.set_status(format!("{} search is disabled in Settings", name));
                return Ok(vec![]);
            }
            let q = query.unwrap_or_else(|| super::key_input::build_external_search_query(state));
            if q.trim().is_empty() {
                state.set_status("Nothing selected to search".to_string());
                return Ok(vec![]);
            }
            let tx = event_tx.clone();
            crate::app::tasks::spawn(async move {
                let action = match crate::services::external_search::open_search(target, &q).await {
                    Ok(None) => return,
                    Ok(Some(notice)) => SystemAction::SetStatus(notice.into()),
                    Err(error) => {
                        tracing::warn!(?target, "Could not open external search: {error:#}");
                        SystemAction::ShowError(format!("Could not open search: {error}"))
                    }
                };
                let _ = tx.send(Event::Effect(action.into())).await;
            });
        }
        _ => anyhow::bail!("Unsupported library operation reached shared handler"),
    }
    Ok(vec![])
}
