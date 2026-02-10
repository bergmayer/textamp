//! Playback helpers: track playing, Plex reporting, radio fetching.

use crate::app::{AppState, Event};
use crate::app::state::{PlayStatus, PlaybackMode, View};
use crate::api::PlexClient;
use crate::api::models::Track;
use crate::audio::{AudioEvent, AudioPlayer};
use crate::audio::cache;
use tokio::sync::mpsc;

/// Compute the list of upcoming tracks to pre-fetch from current state.
pub fn get_upcoming_tracks(state: &AppState) -> Vec<Track> {
    match state.playback_mode {
        PlaybackMode::Queue | PlaybackMode::None => {
            if let Some(idx) = state.queue_index {
                let start = idx + 1;
                let end = (start + 10).min(state.queue.len());
                if start < state.queue.len() {
                    return state.queue[start..end].to_vec();
                }
            }
            vec![]
        }
        PlaybackMode::Radio => {
            if let Some(idx) = state.radio.track_index {
                let start = idx + 1;
                let end = (start + 10).min(state.radio.tracks.len());
                if start < state.radio.tracks.len() {
                    return state.radio.tracks[start..end].to_vec();
                }
            }
            vec![]
        }
    }
}

/// Create an adapter channel that converts `AudioEvent` to app `Event`.
///
/// Returns a sender that the audio player can use. The spawned task
/// forwards events to the app event loop.
fn audio_event_adapter(event_tx: &mpsc::Sender<Event>) -> mpsc::Sender<AudioEvent> {
    let (audio_tx, mut audio_rx) = mpsc::channel::<AudioEvent>(4);
    let event_tx = event_tx.clone();
    tokio::spawn(async move {
        while let Some(ev) = audio_rx.recv().await {
            let app_event = match ev {
                AudioEvent::BufferingReady => Event::BufferingEnd,
                AudioEvent::Error(msg) => Event::PlaybackError(msg),
            };
            let _ = event_tx.send(app_event).await;
        }
    });
    audio_tx
}

/// Play a track, setting up queue context.
pub async fn play_track(
    event_tx: &mpsc::Sender<Event>,
    track: Track,
    state: &mut AppState,
    client: &PlexClient,
    audio: &mut AudioPlayer,
) {
    // Report stop for currently playing track before switching
    if let Some(current) = state.current_track().cloned() {
        report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
    }

    // Generate new session ID for this playback context
    state.plex_session_id = Some(generate_plex_session_id());

    if state.view == View::NowPlaying || state.view == View::Similar {
        if state.playback_mode == PlaybackMode::Radio {
            state.radio.clear();
        }
        state.queue_original.clear();
        state.queue_sort_mode = crate::app::state::QueueSortMode::QueueOrder;
        state.playback_mode = PlaybackMode::Queue;
        play_current_track(event_tx, state, client, audio).await;
    } else {
        if state.playback_mode == PlaybackMode::Radio {
            state.radio.clear();
        }
        state.queue = vec![track];
        state.queue_index = Some(0);
        state.queue_original.clear();
        state.queue_sort_mode = crate::app::state::QueueSortMode::QueueOrder;
        state.playback_mode = PlaybackMode::Queue;
        play_current_track(event_tx, state, client, audio).await;
    }
}

/// Helper to collect tracks from a Miller column for playback.
pub fn collect_tracks_from_column(col: &crate::app::state::BrowseColumn) -> Vec<Track> {
    if !col.tracks.is_empty() {
        return col.tracks.clone();
    }

    let track_count = col.items.iter().filter(|item| matches!(item, crate::app::state::BrowseItem::Track { .. })).count();
    if track_count > 0 {
        tracing::warn!(
            "collect_tracks_from_column fallback: creating {} track stubs without media info for column '{}'. Direct playback may fail.",
            track_count,
            col.title
        );
    }

    col.items.iter()
        .filter_map(|item| {
            if let crate::app::state::BrowseItem::Track { key, title, duration_ms, track_number, .. } = item {
                Some(Track {
                    rating_key: key.clone(),
                    title: title.clone(),
                    duration: Some(*duration_ms),
                    index: *track_number,
                    year: None,
                    parent_year: None,
                    parent_title: None,
                    grandparent_title: None,
                    parent_rating_key: None,
                    grandparent_rating_key: None,
                    media: vec![],
                    thumb: None,
                    key: String::new(),
                    parent_thumb: None,
                    grandparent_thumb: None,
                })
            } else {
                None
            }
        })
        .collect()
}

/// Play the current track from the queue.
pub async fn play_current_track(
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,
    client: &PlexClient,
    audio: &mut AudioPlayer,
) {
    if let Some(track) = state.current_track().cloned() {
        tracing::info!("Playing: {} - {}", track.artist_name(), track.title);
        tracing::info!("PlayCurrentTrack: client_identifier={}", client.client_identifier());
        tracing::info!("PlayCurrentTrack: server_url={:?}", client.server_url());
        tracing::info!("PlayCurrentTrack: has_token={}", client.token().is_some());
        tracing::info!("PlayCurrentTrack: track.media.len()={}", track.media.len());

        state.playback.status = PlayStatus::Buffering;
        state.playback.duration_ms = track.duration_ms();
        state.playback.position_ms = 0;

        // Reset waveform state for new track
        if state.waveform.track_key.as_ref() != Some(&track.rating_key) {
            state.waveform = crate::app::state::WaveformState::default();
            state.waveform.track_key = Some(track.rating_key.clone());

            // Auto-generate waveform if currently in visualizer mode
            if state.view == View::NowPlaying
                && state.now_playing_mode == crate::app::state::NowPlayingMode::NowPlaying
            {
                if let Ok(stream_url) = client.get_stream_url(&track) {
                    state.waveform.generating = true;
                    let track_key = track.rating_key.clone();
                    let duration_ms = track.duration_ms();
                    let event_tx = event_tx.clone();
                    let token = client.token().map(|s| s.to_string());

                    tokio::spawn(async move {
                        let cache_dir = dirs::cache_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                            .join("textamp")
                            .join("waveforms");
                        let cache = crate::services::WaveformCache::new(cache_dir);

                        if let Some(data) = cache.load(&track_key) {
                            let _ = event_tx.send(Event::WaveformCacheHit {
                                track_key,
                                data,
                            }).await;
                            return;
                        }

                        let http_client = reqwest::Client::new();
                        let mut request = http_client.get(&stream_url);
                        if let Some(ref token) = token {
                            request = request.header("X-Plex-Token", token);
                        }

                        match request.send().await {
                            Ok(response) => {
                                match response.bytes().await {
                                    Ok(audio_data) => {
                                        match crate::services::generate_waveform(
                                            track_key.clone(),
                                            duration_ms,
                                            audio_data.to_vec(),
                                        ) {
                                            Ok(data) => {
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

        // Load artwork for the new track (non-blocking)
        if let Some(thumb_path) = track.best_thumb() {
            if state.artwork_thumb.as_deref() != Some(thumb_path) {
                if let Some(server_url) = client.server_url() {
                    state.artwork_loading = true;
                    let thumb_path_owned = thumb_path.to_string();
                    let event_tx = event_tx.clone();
                    let server_url = server_url.to_string();
                    let token = client.token().map(|s| s.to_string());
                    let client_id = client.client_identifier().to_string();

                    tokio::spawn(async move {
                        let client = crate::api::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            client.fetch_artwork(&thumb_path_owned, 300)
                        ).await {
                            Ok(Ok(data)) => {
                                let _ = event_tx.send(Event::ArtworkLoaded {
                                    thumb_path: thumb_path_owned,
                                    data,
                                }).await;
                            }
                            Ok(Err(e)) => {
                                tracing::warn!("Failed to load artwork: {}", e);
                                let _ = event_tx.send(Event::ArtworkFailed {
                                    thumb_path: thumb_path_owned,
                                }).await;
                            }
                            Err(_) => {
                                tracing::warn!("Artwork loading timed out");
                                let _ = event_tx.send(Event::ArtworkFailed {
                                    thumb_path: thumb_path_owned,
                                }).await;
                            }
                        }
                    });
                } else {
                    state.artwork_loading = false;
                    state.artwork_data = None;
                }
            } else {
                state.artwork_loading = false;
            }
        } else {
            state.artwork_thumb = None;
            state.artwork_data = None;
            state.artwork_loading = false;
        }

        // Check track cache first (pre-fetched audio data)
        if let Some(cached_data) = audio.track_cache.get(&track.rating_key) {
            tracing::info!("Cache hit for: {} - {}", track.artist_name(), track.title);
            // Stop current playback before starting cached playback
            audio.stop();
            match audio.play_data(cached_data) {
                Ok(()) => {
                    state.playback.status = PlayStatus::Playing;
                    state.playback.playback_started_at = Some(std::time::Instant::now());
                    report_playback_to_plex(event_tx, &track, state.plex_session_id.clone(), client);
                    state.last_progress_report = Some(std::time::Instant::now());
                    // Trigger pre-fetch for next tracks
                    let upcoming = get_upcoming_tracks(state);
                    cache::trigger_prefetch(&audio.track_cache, &upcoming, client);
                    return;
                }
                Err(e) => {
                    tracing::warn!("Cached playback failed, falling back to stream: {}", e);
                    audio.track_cache.remove(&track.rating_key);
                    // Fall through to normal streaming path
                }
            }
        }

        // Build stream URLs: primary (direct) + fallback (transcode)
        let primary_url = client.get_stream_url(&track).ok();
        let fallback_url = client.get_transcoded_stream_url(&track).ok();

        // Create adapter to bridge AudioEvent → app Event
        let audio_tx = audio_event_adapter(event_tx);

        if let Some(url) = primary_url {
            tracing::debug!("Direct stream URL: {}", url);
            // play_url_with_headers spawns HTTP fetch in background — returns immediately
            if let Err(e) = audio.play_url_with_headers(&url, reqwest::header::HeaderMap::new(), fallback_url, audio_tx).await {
                state.set_error(format!("Playback failed: {}", e));
                state.playback.status = PlayStatus::Stopped;
                return;
            }
            report_playback_to_plex(event_tx, &track, state.plex_session_id.clone(), client);
            state.last_progress_report = Some(std::time::Instant::now());
            // Trigger pre-fetch for next tracks
            let upcoming = get_upcoming_tracks(state);
            cache::trigger_prefetch(&audio.track_cache, &upcoming, client);
        } else if let Some(url) = fallback_url {
            let redacted = url.split("X-Plex-Token=").next().unwrap_or(&url);
            tracing::info!("Using transcoded stream for: {} - URL: {}...", track.title, redacted);
            if let Err(e) = audio.play_url_with_headers(&url, reqwest::header::HeaderMap::new(), None, audio_event_adapter(event_tx)).await {
                state.set_error(format!("Playback failed: {}", e));
                state.playback.status = PlayStatus::Stopped;
                return;
            }
            report_playback_to_plex(event_tx, &track, state.plex_session_id.clone(), client);
            state.last_progress_report = Some(std::time::Instant::now());
            // Trigger pre-fetch for next tracks
            let upcoming = get_upcoming_tracks(state);
            cache::trigger_prefetch(&audio.track_cache, &upcoming, client);
        } else {
            tracing::error!("Cannot get any stream URL (track has {} media items)", track.media.len());
            state.set_error("Failed to get stream URL".to_string());
            state.playback.status = PlayStatus::Stopped;
        }
    }
}

/// Report playback start to Plex server in background.
pub fn report_playback_to_plex(_event_tx: &mpsc::Sender<Event>, track: &Track, session_id: Option<String>, client: &PlexClient) {
    if let Some(server_url) = client.server_url() {
        let rating_key = track.rating_key.clone();
        let track_clone = track.clone();
        let server_url = server_url.to_string();
        let token = client.token().map(|s| s.to_string());
        let client_id = client.client_identifier().to_string();

        tokio::spawn(async move {
            let client = crate::api::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);

            if let Err(e) = client.report_playback_start(&track_clone, 0, session_id.as_deref()).await {
                tracing::debug!("Failed to report playback start: {}", e);
            }

            if let Err(e) = client.scrobble(&rating_key).await {
                tracing::debug!("Failed to scrobble: {}", e);
            } else {
                tracing::debug!("Scrobbled track: {}", rating_key);
            }
        });
    }
}

/// Report playback stop to Plex server in background.
pub fn report_playback_stop_to_plex(
    track: &Track,
    position_ms: u64,
    continuing: bool,
    session_id: Option<String>,
    client: &PlexClient,
) {
    if let Some(server_url) = client.server_url() {
        let track_clone = track.clone();
        let server_url = server_url.to_string();
        let token = client.token().map(|s| s.to_string());
        let client_id = client.client_identifier().to_string();

        tokio::spawn(async move {
            let client = crate::api::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);

            if let Err(e) = client.report_playback_stop(&track_clone, position_ms, continuing, session_id.as_deref()).await {
                tracing::debug!("Failed to report playback stop: {}", e);
            } else {
                tracing::debug!("Reported playback stop for: {} (continuing={}, session={:?})", track_clone.title, continuing, session_id);
            }
        });
    }
}

/// Report playback progress to Plex server in background.
pub fn report_playback_progress_to_plex(
    track: &Track,
    position_ms: u64,
    session_id: Option<String>,
    client: &PlexClient,
) {
    if let Some(server_url) = client.server_url() {
        let track_clone = track.clone();
        let server_url = server_url.to_string();
        let token = client.token().map(|s| s.to_string());
        let client_id = client.client_identifier().to_string();

        tokio::spawn(async move {
            let client = crate::api::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);

            if let Err(e) = client.report_playback_progress(&track_clone, position_ms, session_id.as_deref()).await {
                tracing::debug!("Failed to report playback progress: {}", e);
            }
        });
    }
}

/// Generate a new Plex session ID for timeline reporting.
pub fn generate_plex_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Fetch more tracks for the current radio station (non-blocking).
pub fn fetch_more_radio_tracks(event_tx: &mpsc::Sender<Event>, state: &mut AppState, client: &PlexClient) {
    if state.radio.fetching {
        return;
    }

    if let Some(ref station) = state.radio.active_station {
        state.radio.fetching = true;

        let event_tx = event_tx.clone();
        let client = client.clone();

        // Special handling for Time Travel Radio
        if station.key.contains("timeTravel") && !state.radio.time_travel_decades.is_empty() {
            if let Some(lib_key) = state.active_library.clone() {
                let decades = state.radio.time_travel_decades.clone();
                let current_index = state.radio.time_travel_index;

                tracing::info!("Time Travel Radio: fetching more tracks starting from decade index {} ({})",
                    current_index % decades.len(),
                    decades.get(current_index % decades.len()).unwrap_or(&"?".to_string()));

                tokio::spawn(async move {
                    match client.fetch_time_travel_tracks_from_index(&lib_key, &decades, current_index).await {
                        Ok(tracks) => {
                            let _ = event_tx.send(Event::RadioTracksLoaded {
                                tracks,
                                time_travel_index: Some(current_index + 3),
                            }).await;
                        }
                        Err(e) => {
                            tracing::warn!("Time Travel Radio: failed to fetch more tracks: {}", e);
                            // Send empty result to clear fetching flag
                            let _ = event_tx.send(Event::RadioTracksLoaded {
                                tracks: vec![],
                                time_travel_index: None,
                            }).await;
                        }
                    }
                });
                return;
            }
        }

        // Standard station fetch
        let station_key = station.key.clone();
        let station_title = station.title.clone();
        tracing::info!("Fetching more tracks for station: {}", station_title);

        let mut client = client;
        tokio::spawn(async move {
            match client.create_station_queue(&station_key).await {
                Ok(tracks) => {
                    let _ = event_tx.send(Event::RadioTracksLoaded {
                        tracks,
                        time_travel_index: None,
                    }).await;
                }
                Err(e) => {
                    tracing::warn!("Failed to fetch more radio tracks: {}", e);
                    let _ = event_tx.send(Event::RadioTracksLoaded {
                        tracks: vec![],
                        time_travel_index: None,
                    }).await;
                }
            }
        });
    } else {
        state.radio.fetching = false;
    }
}
