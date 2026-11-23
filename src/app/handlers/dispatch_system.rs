//! System dispatch handlers: Quit, ShowError, ClearError, SetStatus, ClearStatus,
//! RefreshCategory, CycleTheme, LoadArtwork, LoadWaveform.

use crate::app::action::SystemAction;
use crate::app::event::*;
use crate::app::{Action, AppState, Event};

use anyhow::Result;
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Download audio data from a stream URL for analysis (waveform/spectrogram generation).
const MAX_ANALYSIS_AUDIO_BYTES: usize = 128 * 1024 * 1024;
const ANALYSIS_DOWNLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

async fn download_audio_for_analysis(
    prepared_file: Option<crate::library::MediaFile>,
    stream_url: &str,
    headers: reqwest::header::HeaderMap,
    http_client: reqwest::Client,
    timeout: std::time::Duration,
    progress: Option<(&LibraryEventSender, &str)>,
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
        report_analysis_progress(progress, crate::app::state::AnalysisProgress::Decoding).await;
        return Ok(Arc::from(bytes));
    }
    let response = http_client
        .get(stream_url)
        .timeout(timeout)
        .headers(headers)
        .send()
        .await
        .map_err(|e| format!("Analysis request failed: {}", e.without_url()))?;
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

    let total = response.content_length();
    report_analysis_progress(
        progress,
        crate::app::state::AnalysisProgress::Downloading { received: 0, total },
    )
    .await;
    let mut reported = std::time::Instant::now();
    let initial_capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or(256 * 1024)
        .min(MAX_ANALYSIS_AUDIO_BYTES);
    let mut bytes = Vec::with_capacity(initial_capacity);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = tokio::time::timeout(std::time::Duration::from_secs(20), stream.next())
        .await
        .map_err(|_| "Analysis download stalled for 20 seconds".to_string())?
    {
        let chunk =
            chunk.map_err(|error| format!("Analysis download failed: {}", error.without_url()))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_ANALYSIS_AUDIO_BYTES {
            return Err(format!(
                "Audio exceeds analysis limit of {} MiB",
                MAX_ANALYSIS_AUDIO_BYTES / (1024 * 1024)
            ));
        }
        bytes.extend_from_slice(&chunk);
        if reported.elapsed() >= std::time::Duration::from_millis(500) {
            report_analysis_progress(
                progress,
                crate::app::state::AnalysisProgress::Downloading {
                    received: bytes.len(),
                    total,
                },
            )
            .await;
            reported = std::time::Instant::now();
        }
    }
    report_analysis_progress(progress, crate::app::state::AnalysisProgress::Decoding).await;
    Ok(Arc::from(bytes))
}

async fn report_analysis_progress(
    target: Option<(&LibraryEventSender, &str)>,
    progress: crate::app::state::AnalysisProgress,
) {
    if let Some((sender, key)) = target {
        let _ = sender
            .send(
                VisualizerEvent::AnalysisProgress {
                    track_key: key.to_owned(),
                    progress,
                }
                .into(),
            )
            .await;
    }
}

/// Publish independent visualizer jobs as each finishes, sharing one audio download.
async fn publish_analysis(
    waveform: impl std::future::Future<Output = Result<crate::media::WaveformData, String>>,
    spectral: impl std::future::Future<Output = Option<Result<crate::media::SpectrogramData, String>>>,
    sender: &LibraryEventSender,
    track_key: &str,
) {
    let wave = async {
        let event = match waveform.await {
            Ok(data) => VisualizerEvent::WaveformGenerated {
                track_key: track_key.to_owned(),
                data,
            },
            Err(error) => VisualizerEvent::WaveformFailed {
                track_key: track_key.to_owned(),
                error,
            },
        };
        let _ = sender.send(event.into()).await;
    };
    let spectrum = async {
        if let Some(result) = spectral.await {
            let event = match result {
                Ok(data) => VisualizerEvent::SpectrogramGenerated {
                    track_key: track_key.to_owned(),
                    data,
                },
                Err(error) => VisualizerEvent::SpectrogramFailed {
                    track_key: track_key.to_owned(),
                    error,
                },
            };
            let _ = sender.send(event.into()).await;
        }
    };
    tokio::join!(wave, spectrum);
}

async fn fail_analysis(
    sender: &LibraryEventSender,
    track_key: &str,
    spectral: bool,
    error: String,
) {
    let waveform = std::future::ready(Err(error.clone()));
    publish_analysis(
        waveform,
        std::future::ready(spectral.then_some(Err(error))),
        sender,
        track_key,
    )
    .await;
}

/// Dispatch system-level actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: SystemAction,
    state: &mut AppState,
    playback_input: Option<crate::audio::player::AnalysisInput>,
) -> Result<Vec<Action>> {
    match action {
        SystemAction::Downloads(command) => {
            return crate::app::downloads::dispatch(command, state, event_tx).await;
        }
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
            let should_scrobble = !state.downloads.is_offline()
                && navidrome_track.is_some()
                && state.playback.scrobble == crate::app::state::ScrobbleState::Pending
                && state.playback.duration_ms > 0
                && state.playback.position_ms.saturating_mul(10)
                    >= state.playback.duration_ms.saturating_mul(9);
            if should_scrobble {
                state.playback.scrobble = crate::app::state::ScrobbleState::Queued;
            }
            let flush = async move {
                if let Some((client, id)) = navidrome_track.filter(|_| should_scrobble)
                    && let Err(error) = client
                        .call("scrobble", &[("id", id), ("submission", "true".into())])
                        .await
                {
                    tracing::warn!("Final Navidrome scrobble failed: {error}");
                }
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
        SystemAction::OpenBrowser(url) => {
            let tx = event_tx.clone();
            crate::app::tasks::spawn(async move {
                if let Err(error) = crate::services::external_search::open_browser(&url).await {
                    let _ = tx
                        .send(Event::Effect(
                            SystemAction::ShowError(format!("Open browser: {error:#}")).into(),
                        ))
                        .await;
                }
            });
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

        SystemAction::PrefetchNextWaveform => {
            crate::app::sources::navidrome::prefetch_waveform(state, event_tx)
        }
        SystemAction::LoadWaveform => {
            // Folder playback prepares/downloads the file first. A request
            // during preparation must not fall through to the server client.
            if state.current_track().is_some_and(|t| {
                matches!(t.origin, crate::library::track::TrackOrigin::Folder { .. })
            }) && state.sources.media.file().is_none()
            {
                return Ok(vec![]);
            }
            // Only generate waveform if we have a track and don't already have data
            if let Some(track) = state.current_track().cloned() {
                // A new track replaces its analysis state and cancels the previous job.
                if state.waveform.track_key.as_ref() != Some(&track.rating_key) {
                    state.waveform = crate::app::state::WaveformState::default();
                    state.waveform.track_key = Some(track.rating_key.clone());
                    state.spectrogram = crate::app::state::SpectrogramState::default();
                    state.spectrogram.track_key = Some(track.rating_key.clone());
                }
                let needs_generation =
                    state.waveform.data().is_none() && !state.waveform.generating();

                if needs_generation {
                    state.waveform.start();
                    let also_generate_spectrogram = state.visualizers_available
                        && state.spectrogram.data().is_none()
                        && !state.spectrogram.generating();
                    if also_generate_spectrogram {
                        state.spectrogram.start();
                    }
                    let track_key = track.rating_key.clone();
                    let duration_ms = track.duration_ms();
                    let event_tx =
                        LibraryEventSender::new(event_tx.clone(), state.library_generation);
                    let cache_scope = state.media_cache_scope();

                    // Source resolution failures complete the analysis request.
                    let prepared_file = state.sources.media.file().cloned();

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
                                fail_analysis(&event_tx_err, &track_key_err, also_sg, err_msg)
                                    .await;
                            });
                            return Ok(vec![]);
                        }
                    };
                    let stream_headers = Default::default();
                    let http_client = navidrome.map(|s| s.client.http()).unwrap_or_default();
                    let task = crate::app::tasks::spawn(async move {
                        let cache_root = crate::config::XdgPaths::new("textamp").cache_dir;
                        let waveform_cache_dir = cache_root.join("waveforms");
                        let spectrogram_cache_dir = cache_root.join("spectrograms");
                        let cache_track_key = track_key.clone();
                        let cache_read_scope = cache_scope.clone();
                        let waveform_read_dir = waveform_cache_dir.clone();
                        let spectrogram_read_dir = spectrogram_cache_dir.clone();

                        let cached = crate::app::tasks::spawn_blocking(move || {
                            let waveform = crate::media::WaveformCache::new(waveform_read_dir)
                                .load_scoped(&cache_read_scope, &cache_track_key);
                            let spectrogram = also_generate_spectrogram
                                .then(|| {
                                    crate::media::SpectrogramCache::new(spectrogram_read_dir)
                                        .load_scoped(&cache_read_scope, &cache_track_key)
                                })
                                .flatten();
                            (waveform, spectrogram)
                        })
                        .await;
                        let (waveform_cached, spectrogram_cached) = match cached {
                            Ok(cached) => cached,
                            Err(error) => {
                                fail_analysis(
                                    &event_tx,
                                    &track_key,
                                    also_generate_spectrogram,
                                    format!("cache worker failed: {error}"),
                                )
                                .await;
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
                                    None => VisualizerEvent::SpectrogramCacheMiss { track_key },
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

                        let prepared_file = if prepared_file.is_some() {
                            prepared_file
                        } else if let Some(input) = playback_input {
                            match input.wait().await {
                                Ok(file) => Some(file),
                                Err(error) => {
                                    fail_analysis(
                                        &event_tx,
                                        &track_key,
                                        generate_spectrogram,
                                        error,
                                    )
                                    .await;
                                    return;
                                }
                            }
                        } else {
                            None
                        };
                        let audio_data = match download_audio_for_analysis(
                            prepared_file,
                            &stream_url,
                            stream_headers,
                            http_client,
                            ANALYSIS_DOWNLOAD_TIMEOUT,
                            Some((&event_tx, &track_key)),
                        )
                        .await
                        {
                            Ok(data) => data,
                            Err(error) => {
                                fail_analysis(&event_tx, &track_key, generate_spectrogram, error)
                                    .await;
                                return;
                            }
                        };

                        let waveform_key = track_key.clone();
                        let waveform_audio = audio_data.clone();
                        let waveform_scope = cache_scope.clone();
                        let waveform = async move {
                            crate::app::tasks::analyze(move || {
                                let data = crate::media::generate_waveform(
                                    waveform_key,
                                    duration_ms,
                                    waveform_audio,
                                )
                                .map_err(|error| error.to_string())?;
                                crate::media::WaveformCache::new(waveform_cache_dir)
                                    .save_scoped(&waveform_scope, &data);
                                Ok(data)
                            })
                            .await
                            .map_err(|error| format!("audio analysis worker failed: {error}"))?
                        };
                        let spectral_key = track_key.clone();
                        let spectral = async move {
                            if !generate_spectrogram {
                                return None;
                            }
                            Some(
                                crate::app::tasks::analyze(move || {
                                    let data = crate::media::generate_spectrogram(
                                        spectral_key,
                                        duration_ms,
                                        audio_data,
                                    )
                                    .map_err(|error| error.to_string())?;
                                    crate::media::SpectrogramCache::new(spectrogram_cache_dir)
                                        .save_scoped(&cache_scope, &data);
                                    Ok(data)
                                })
                                .await
                                .map_err(|error| format!("audio analysis worker failed: {error}"))
                                .and_then(|result| result),
                            )
                        };
                        publish_analysis(waveform, spectral, &event_tx, &track_key).await;
                    });
                    let lease = crate::app::tasks::TaskLease::new(&task);
                    state.waveform.task = Some(lease.clone());
                    if also_generate_spectrogram {
                        state.spectrogram.task = Some(lease);
                    }
                }
            }
        }
        SystemAction::LoadSpectrogram => {
            if state.current_track().is_some_and(|t| {
                matches!(t.origin, crate::library::track::TrackOrigin::Folder { .. })
            }) && state.sources.media.file().is_none()
            {
                return Ok(vec![]);
            }
            // Load spectrogram data — check cache first, then generate if needed.
            // Generation is normally co-computed with waveform, but if waveform is
            // already loaded (e.g., re-entering NowPlaying), we download independently.
            if let Some(track) = state.current_track().cloned() {
                if state.spectrogram.track_key.as_ref() != Some(&track.rating_key) {
                    state.spectrogram = crate::app::state::SpectrogramState::default();
                    state.spectrogram.track_key = Some(track.rating_key.clone());
                }
                let needs_generation =
                    state.spectrogram.data().is_none() && !state.spectrogram.generating();

                if needs_generation {
                    if state.waveform.data().is_none() && !state.waveform.generating() {
                        // Neither waveform nor spectrogram — trigger LoadWaveform to co-compute
                        return Ok(vec![SystemAction::LoadWaveform.into()]);
                    } else if state.waveform.generating() {
                        // Waveform is being generated right now — it will co-compute spectrogram
                        state.spectrogram.start();
                    } else {
                        // Waveform already loaded but no spectrogram — download independently
                        state.spectrogram.start();
                        let track_key = track.rating_key.clone();
                        let duration_ms = track.duration_ms();
                        let event_tx =
                            LibraryEventSender::new(event_tx.clone(), state.library_generation);
                        let cache_scope = state.media_cache_scope();

                        // Same defensive failure path as `LoadWaveform`:
                        // if get_stream_url fails synchronously we
                        // MUST emit `SpectrogramFailed`, otherwise
                        // `generating` stays true and the panel is
                        // stuck on "Generating spectrogram…".
                        let prepared_file = state.sources.media.file().cloned();
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
                        let task = crate::app::tasks::spawn(async move {
                            let cache_dir = crate::config::XdgPaths::new("textamp")
                                .cache_dir
                                .join("spectrograms");
                            let read_dir = cache_dir.clone();
                            let read_key = track_key.clone();
                            let read_scope = cache_scope.clone();
                            match crate::app::tasks::spawn_blocking(move || {
                                crate::media::SpectrogramCache::new(read_dir)
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
                                ANALYSIS_DOWNLOAD_TIMEOUT,
                                Some((&event_tx, &track_key)),
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
                                let result = crate::media::generate_spectrogram(
                                    computation_key,
                                    duration_ms,
                                    audio_data,
                                )
                                .map_err(|error| error.to_string());
                                if let Ok(data) = &result {
                                    crate::media::SpectrogramCache::new(cache_dir)
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
                        state.spectrogram.task = Some(crate::app::tasks::TaskLease::new(&task));
                    }
                }
            }
        }

        SystemAction::OpenExternalSearch { target, query } => {
            let request = match crate::app::external_search::request(state, target, query) {
                Ok(request) => request,
                Err(error) => {
                    state.set_status(error.to_string());
                    return Ok(vec![]);
                }
            };
            let tx = event_tx.clone();
            crate::app::tasks::spawn(async move {
                let action = match crate::services::external_search::open_search(
                    request.target,
                    &request.query,
                )
                .await
                {
                    Ok(None) => return,
                    Ok(Some(notice)) => SystemAction::SetStatus(notice.into()),
                    Err(error) => {
                        tracing::warn!(?target, "Could not open external search: {error:#}");
                        SystemAction::ShowError(format!("Could not open search: {error:#}"))
                    }
                };
                let _ = tx.send(Event::Effect(action.into())).await;
            });
        }
        _ => anyhow::bail!("Unsupported library operation reached shared handler"),
    }
    Ok(vec![])
}

#[cfg(test)]
mod analysis_download_tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn cancelled_playback_input_completes_both_analysis_requests() {
        let mut state = AppState::new();
        state.visualizers_available = true;
        let source = crate::navidrome::Source {
            id: "analysis-fixture".into(),
            name: "Fixture".into(),
            username: "fixture".into(),
            url: "http://127.0.0.1:1".into(),
            libraries: vec![],
        };
        let session = crate::app::sources::navidrome::Session {
            client: crate::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
            source,
            extensions: Default::default(),
        };
        state.queue.tracks.push(crate::library::models::Track {
            rating_key: session.key("song"),
            ..Default::default()
        });
        state.playback_mode = crate::app::state::PlaybackMode::Queue;
        state.queue.index = Some(0);
        state.sources.active = crate::app::sources::ActiveSource::Navidrome(Box::new(session));
        let mut audio = crate::audio::AudioPlayer::with_test_samples(&[]);
        let (backend_tx, _backend_rx) = mpsc::channel(4);
        audio
            .play_url(
                "http://127.0.0.1:1/stream",
                backend_tx,
                reqwest::Client::new(),
            )
            .unwrap();
        let input = audio.analysis_input();
        audio.stop();

        let (tx, mut rx) = mpsc::channel(8);
        dispatch(&tx, SystemAction::LoadWaveform, &mut state, input)
            .await
            .unwrap();
        assert!(state.waveform.generating());
        assert!(state.spectrogram.generating());
        let mut failures = (false, false);
        for _ in 0..2 {
            let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
                .unwrap();
            let Event::LibraryResult { event, .. } = event else {
                panic!("expected library-scoped analysis failure");
            };
            match *event {
                Event::Visualizer(VisualizerEvent::WaveformFailed { error, .. }) => {
                    assert_eq!(error, "Track changed");
                    failures.0 = true;
                }
                Event::Visualizer(VisualizerEvent::SpectrogramFailed { error, .. }) => {
                    assert_eq!(error, "Track changed");
                    failures.1 = true;
                }
                event => panic!("unexpected analysis completion: {event:?}"),
            }
        }
        assert_eq!(failures, (true, true));
    }

    #[tokio::test]
    async fn waveform_is_published_while_spectral_analysis_is_pending() {
        let (tx, mut rx) = mpsc::channel(8);
        let sender = LibraryEventSender::new(tx, 42);
        let (finish, wait) = tokio::sync::oneshot::channel::<()>();
        let wave = crate::media::generate_waveform_from_pcm("track".into(), 1000, &[0.2; 1000]);
        let job = tokio::spawn(async move {
            publish_analysis(
                std::future::ready(Ok(wave)),
                async move {
                    wait.await.unwrap();
                    Some(Err("spectral analysis failed".into()))
                },
                &sender,
                "track",
            )
            .await;
        });
        let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(event, Event::LibraryResult { generation: 42, event }
            if matches!(*event, Event::Visualizer(VisualizerEvent::WaveformGenerated { ref track_key, .. }) if track_key == "track"))
        );
        assert!(!job.is_finished());
        finish.send(()).unwrap();
        let event = rx.recv().await.unwrap();
        assert!(
            matches!(event, Event::LibraryResult { generation: 42, event }
            if matches!(*event, Event::Visualizer(VisualizerEvent::SpectrogramFailed { ref track_key, .. }) if track_key == "track"))
        );
        job.await.unwrap();
    }

    #[tokio::test]
    async fn analysis_download_deadline_covers_the_body_and_redacts_credentials() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut connection, _) = listener.accept().await.unwrap();
            let mut request = [0; 1];
            connection.read_exact(&mut request).await.unwrap();
            connection
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\na")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        let result = download_audio_for_analysis(
            None,
            &format!("http://{address}/stream?token=private-value"),
            Default::default(),
            reqwest::Client::new(),
            Duration::from_millis(100),
            None,
        )
        .await;
        server.abort();
        let error = result.unwrap_err();
        assert!(error.contains("Analysis download failed"), "{error}");
        assert!(!error.contains("private-value"));
        assert!(!error.contains("http://"));
    }

    #[tokio::test]
    async fn replacing_track_analysis_cancels_its_download() {
        let task = tokio::spawn(std::future::pending::<()>());
        let waveform = crate::app::state::WaveformState {
            task: Some(crate::app::tasks::TaskLease::new(&task)),
            ..Default::default()
        };
        drop(waveform);
        assert!(task.await.unwrap_err().is_cancelled());
    }
}
