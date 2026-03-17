//! Playback helpers: track playing, Plex reporting, radio fetching.

use crate::app::{AppState, Event};
use crate::app::state::{PlayStatus, PlaybackMode, View};
use crate::plex::PlexClient;
use crate::plex::models::{Artist, Track};
use crate::audio::{AudioEvent, AudioPlayer};
use crate::audio::cache;
use tokio::sync::mpsc;

/// Look up artist artwork as a fallback when a track has no thumb.
fn find_artist_thumb(track: &Track, artists: &[Artist]) -> Option<String> {
    let artist_key = track.grandparent_rating_key.as_ref()?;
    artists.iter()
        .find(|a| a.rating_key == *artist_key)
        .and_then(|a| a.thumb.clone())
}

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

/// Play a track, prepending it to the queue and preserving upcoming tracks.
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

    // Migrate radio tracks to queue before clearing radio mode
    if state.playback_mode == PlaybackMode::Radio {
        state.queue = state.radio.tracks.clone();
        state.queue_index = state.radio.track_index;
        state.radio.clear();
    }

    // Prepend new track at front of queue
    state.queue.insert(0, track);
    state.queue_index = Some(0);
    state.queue_selected.clear();
    state.queue_original.clear();
    state.queue_sort_mode = crate::app::state::QueueSortMode::QueueOrder;
    state.playback_mode = PlaybackMode::Queue;

    // Scroll queue view to top
    state.list_state.queue_index = 0;

    audio.track_cache.flush();
    play_current_track(event_tx, state, client, audio).await;
}

/// Replace the active queue with `tracks`, start playback at `play_idx`,
/// and switch to the Queue view.
///
/// This consolidates the common queue-management sequence shared by all
/// "play tracks" handlers (Miller columns, folders, album groups, etc.):
///   1. Clear radio mode if active
///   2. Drain played tracks to history
///   3. Flush the audio pre-fetch cache
///   4. Splice new tracks into the queue
///   5. Set queue index, playback mode, list state
///   6. Switch to Now Playing
///   7. Start playback
pub async fn queue_and_play(
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,
    client: &PlexClient,
    audio: &mut AudioPlayer,
    tracks: Vec<Track>,
    play_idx: usize,
) {
    if state.playback_mode == PlaybackMode::Radio {
        state.radio.clear();
    }
    audio.track_cache.flush();
    state.queue = tracks;
    state.queue_index = Some(play_idx);
    state.queue_selected.clear();
    state.queue_original.clear();
    state.queue_sort_mode = crate::app::state::QueueSortMode::QueueOrder;
    state.playback_mode = PlaybackMode::Queue;
    state.list_state.queue_index = play_idx;
    state.set_view(View::Queue);
    play_current_track(event_tx, state, client, audio).await;
}

/// Insert tracks into the queue immediately after the currently playing track.
/// If no track is playing, inserts at the beginning of the queue.
/// Does NOT start playback — just modifies the queue.
pub fn insert_tracks_next(state: &mut AppState, tracks: Vec<Track>) -> usize {
    // Convert radio to queue if needed
    if state.playback_mode == PlaybackMode::Radio {
        state.queue = state.radio.tracks.clone();
        state.queue_index = state.radio.track_index;
        state.playback_mode = PlaybackMode::Queue;
        state.radio.clear();
        if let Some(idx) = state.queue_index {
            state.list_state.queue_index = idx;
        }
    }

    state.queue_original.clear();
    state.queue_sort_mode = crate::app::state::QueueSortMode::QueueOrder;

    let insert_pos = state.queue_index.map(|idx| idx + 1).unwrap_or(0);
    let added = tracks.len();
    state.queue.splice(insert_pos..insert_pos, tracks);
    added
}

/// Play the current track from the queue.
pub async fn play_current_track(
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,
    client: &PlexClient,
    audio: &mut AudioPlayer,
) {
    // Remote playback guard: when output is Remote, use remote player instead of local audio
    if let crate::app::state::OutputTarget::Remote { ref player_id, ref player_uri, .. } = state.remote.output_target {
        play_current_track_remote(event_tx, state, client, player_id.clone(), player_uri.clone()).await;
        return;
    }

    if let Some(track) = state.current_track().cloned() {
        tracing::info!("Playing: {} - {}", track.artist_name(), track.title);
        tracing::info!("PlayCurrentTrack: client_identifier={}", client.client_identifier());
        tracing::info!("PlayCurrentTrack: server_url={:?}", client.server_url());
        tracing::info!("PlayCurrentTrack: has_token={}", client.token().is_some());
        tracing::info!("PlayCurrentTrack: track.media.len()={}", track.media.len());

        state.playback.status = PlayStatus::Buffering;
        state.playback.duration_ms = track.duration_ms();
        state.playback.position_ms = 0;

        // Reset waveform and spectrogram state for new track.
        // The tick handler auto-triggers generation when on NowPlaying view.
        if state.waveform.track_key.as_ref() != Some(&track.rating_key) {
            state.waveform = crate::app::state::WaveformState::default();
            state.waveform.track_key = Some(track.rating_key.clone());
            state.spectrogram = crate::app::state::SpectrogramState::default();
            state.spectrogram.track_key = Some(track.rating_key.clone());
        }

        // Load artwork for the new track (non-blocking)
        if let Some(thumb_path) = track.best_thumb() {
            if state.artwork.current_thumb.as_deref() != Some(thumb_path) {
                if let Some(server_url) = client.server_url() {
                    state.artwork.loading = true;
                    let thumb_path_owned = thumb_path.to_string();
                    let event_tx = event_tx.clone();
                    let server_url = server_url.to_string();
                    let token = client.token().map(|s| s.to_string());
                    let client_id = client.client_identifier().to_string();

                    tokio::spawn(async move {
                        let client = crate::plex::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            client.fetch_artwork(&thumb_path_owned, 600)
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
                    state.artwork.loading = false;
                    state.artwork.current_data = None;
                }
            } else {
                state.artwork.loading = false;
            }
        } else if let Some(artist_thumb) = find_artist_thumb(&track, &state.artists) {
            if state.artwork.current_thumb.as_deref() != Some(&artist_thumb) {
                if let Some(server_url) = client.server_url() {
                    state.artwork.loading = true;
                    let thumb_path_owned = artist_thumb.clone();
                    let event_tx = event_tx.clone();
                    let server_url = server_url.to_string();
                    let token = client.token().map(|s| s.to_string());
                    let client_id = client.client_identifier().to_string();

                    tokio::spawn(async move {
                        let client = crate::plex::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            client.fetch_artwork(&thumb_path_owned, 600)
                        ).await {
                            Ok(Ok(data)) => {
                                let _ = event_tx.send(Event::ArtworkLoaded {
                                    thumb_path: thumb_path_owned,
                                    data,
                                }).await;
                            }
                            Ok(Err(e)) => {
                                tracing::warn!("Failed to load artist artwork: {}", e);
                                let _ = event_tx.send(Event::ArtworkFailed {
                                    thumb_path: thumb_path_owned,
                                }).await;
                            }
                            Err(_) => {
                                tracing::warn!("Artist artwork loading timed out");
                                let _ = event_tx.send(Event::ArtworkFailed {
                                    thumb_path: thumb_path_owned,
                                }).await;
                            }
                        }
                    });
                } else {
                    state.artwork.loading = false;
                    state.artwork.current_data = None;
                }
            } else {
                state.artwork.loading = false;
            }
        } else {
            state.artwork.current_thumb = None;
            state.artwork.current_data = None;
            state.artwork.loading = false;
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
                    cache::trigger_prefetch(&audio.track_cache, &upcoming, client, state.transcode_kbps);
                    return;
                }
                Err(e) => {
                    tracing::warn!("Cached playback failed, falling back to stream: {}", e);
                    audio.track_cache.remove(&track.rating_key);
                    // Fall through to normal streaming path
                }
            }
        }

        // Build stream URL: transcode if configured, otherwise direct play. No fallback.
        let stream_url = if state.transcode_kbps > 0 {
            client.get_transcoded_stream_url(&track, state.transcode_kbps).await.ok()
        } else {
            client.get_stream_url(&track).ok()
        };

        // Create adapter to bridge AudioEvent → app Event
        let audio_tx = audio_event_adapter(event_tx);

        // Use the PlexClient's HTTP client — shares connection pool and settings with working API calls.
        // Transcode URLs have all auth params in the query string already — adding them
        // as headers too causes 400. Direct play URLs need stream_headers for auth.
        let stream_headers = if state.transcode_kbps > 0 {
            reqwest::header::HeaderMap::new()
        } else {
            client.stream_headers()
        };
        let http_client = client.http_client().clone();

        if let Some(url) = stream_url {
            let mode = if state.transcode_kbps > 0 { format!("transcode {}kbps", state.transcode_kbps) } else { "direct".to_string() };
            tracing::debug!("{} stream URL: {}", mode, url);
            if let Err(e) = audio.play_url_with_headers(&url, stream_headers, None, audio_tx, http_client).await {
                state.set_error(format!("Playback failed: {}", e));
                state.playback.status = PlayStatus::Stopped;
                return;
            }
            report_playback_to_plex(event_tx, &track, state.plex_session_id.clone(), client);
            state.last_progress_report = Some(std::time::Instant::now());
            // Trigger pre-fetch for next tracks
            let upcoming = get_upcoming_tracks(state);
            cache::trigger_prefetch(&audio.track_cache, &upcoming, client, state.transcode_kbps);
        } else {
            tracing::error!("Cannot get stream URL (track has {} media items, transcode_kbps={})", track.media.len(), state.transcode_kbps);
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
            let client = crate::plex::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);

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
            let client = crate::plex::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);

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
            let client = crate::plex::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);

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

/// Play the current track on a remote Plex player.
async fn play_current_track_remote(
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,
    client: &PlexClient,
    target_player_id: String,
    player_uri: Option<String>,
) {
    use crate::app::state::PlayStatus;

    if let Some(track) = state.current_track().cloned() {
        tracing::info!("Remote: playing {} - {}", track.artist_name(), track.title);

        state.playback.status = PlayStatus::Buffering;
        state.playback.duration_ms = track.duration_ms();
        state.playback.position_ms = 0;

        // Reset waveform and spectrogram state for new track.
        // The tick handler auto-triggers generation when on NowPlaying view.
        if state.waveform.track_key.as_ref() != Some(&track.rating_key) {
            state.waveform = crate::app::state::WaveformState::default();
            state.waveform.track_key = Some(track.rating_key.clone());
            state.spectrogram = crate::app::state::SpectrogramState::default();
            state.spectrogram.track_key = Some(track.rating_key.clone());
        }

        // Load artwork for the new track (same as local)
        if let Some(thumb_path) = track.best_thumb() {
            if state.artwork.current_thumb.as_deref() != Some(thumb_path) {
                if let Some(server_url) = client.server_url() {
                    state.artwork.loading = true;
                    let thumb_path_owned = thumb_path.to_string();
                    let event_tx_clone = event_tx.clone();
                    let server_url = server_url.to_string();
                    let token = client.token().map(|s| s.to_string());
                    let client_id = client.client_identifier().to_string();

                    tokio::spawn(async move {
                        let client = crate::plex::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            client.fetch_artwork(&thumb_path_owned, 600)
                        ).await {
                            Ok(Ok(data)) => {
                                let _ = event_tx_clone.send(Event::ArtworkLoaded {
                                    thumb_path: thumb_path_owned,
                                    data,
                                }).await;
                            }
                            _ => {
                                let _ = event_tx_clone.send(Event::ArtworkFailed {
                                    thumb_path: thumb_path_owned,
                                }).await;
                            }
                        }
                    });
                }
            }
        } else if let Some(artist_thumb) = find_artist_thumb(&track, &state.artists) {
            if state.artwork.current_thumb.as_deref() != Some(&artist_thumb) {
                if let Some(server_url) = client.server_url() {
                    state.artwork.loading = true;
                    let thumb_path_owned = artist_thumb.clone();
                    let event_tx_clone = event_tx.clone();
                    let server_url = server_url.to_string();
                    let token = client.token().map(|s| s.to_string());
                    let client_id = client.client_identifier().to_string();

                    tokio::spawn(async move {
                        let client = crate::plex::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            client.fetch_artwork(&thumb_path_owned, 600)
                        ).await {
                            Ok(Ok(data)) => {
                                let _ = event_tx_clone.send(Event::ArtworkLoaded {
                                    thumb_path: thumb_path_owned,
                                    data,
                                }).await;
                            }
                            _ => {
                                let _ = event_tx_clone.send(Event::ArtworkFailed {
                                    thumb_path: thumb_path_owned,
                                }).await;
                            }
                        }
                    });
                }
            }
        } else {
            state.artwork.current_thumb = None;
            state.artwork.current_data = None;
            state.artwork.loading = false;
        }

        // Send playMedia to remote player via server
        let token = client.token().map(|s| s.to_string()).unwrap_or_default();
        let client_id = client.client_identifier().to_string();
        let server_url = client.server_url().unwrap_or("").to_string();
        let machine_id = state.available_servers.first()
            .map(|s| s.client_identifier.clone()).unwrap_or_default();
        let lib_key = state.active_library.clone().unwrap_or_default();
        let event_tx_clone = event_tx.clone();

        tokio::spawn(async move {
            let rc = crate::plex::RemotePlayerClient::new(
                token, client_id, target_player_id, server_url, machine_id, player_uri,
            );
            match rc.play_media(&track, &lib_key).await {
                Ok(()) => {
                    // Signal that playback started on remote device
                }
                Err(e) => {
                    let _ = event_tx_clone.send(Event::RemotePlayerError(e.to_string())).await;
                }
            }
        });

        // Optimistically set playing status
        state.playback.status = PlayStatus::Playing;
        state.playback.playback_started_at = Some(std::time::Instant::now());

        // Report to Plex server
        if let Some(track) = state.current_track().cloned() {
            report_playback_to_plex(event_tx, &track, state.plex_session_id.clone(), client);
            state.last_progress_report = Some(std::time::Instant::now());
        }
    }
}
