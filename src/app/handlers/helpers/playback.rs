//! Playback helpers: track playing, Plex reporting, radio fetching.

use crate::app::event::*;
use crate::app::event::LibraryEventSender;
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
            if let Some(idx) = state.queue.index {
                let start = idx + 1;
                let end = (start + 10).min(state.queue.tracks.len());
                if start < state.queue.tracks.len() {
                    return state.queue.tracks[start..end].to_vec();
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
    let (audio_tx, mut audio_rx) = mpsc::channel::<AudioEvent>(32);
    let event_tx = event_tx.clone();
    tokio::spawn(async move {
        while let Some(ev) = audio_rx.recv().await {
            let app_event = match ev {
                AudioEvent::BufferingReady { playback_id } => {
                    PlaybackEvent::BufferingEnd { playback_id }
                }
                AudioEvent::Error { playback_id, message } => {
                    PlaybackEvent::PlaybackError {
                        playback_id: Some(playback_id),
                        message,
                    }
                }
            };
            let _ = event_tx.send(app_event.into()).await;
        }
    });
    audio_tx
}

/// Play a track, prepending it to the queue and preserving upcoming tracks.
pub fn play_track(
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
        state.queue.tracks = state.radio.tracks.clone();
        state.queue.index = state.radio.track_index;
        state.radio.clear();
    }

    // Prepend new track at front of queue
    state.queue.tracks.insert(0, track);
    state.queue.index = Some(0);
    state.queue.selected.clear();
    state.queue.original.clear();
    state.queue.sort_mode = crate::app::state::QueueSortMode::QueueOrder;
    state.playback_mode = PlaybackMode::Queue;

    // Scroll queue view to top
    state.list_state.queue_index = 0;

    audio.track_cache.flush();
    play_current_track(event_tx, state, client, audio);
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
pub fn queue_and_play(
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
    state.queue.tracks = tracks;
    state.queue.index = Some(play_idx);
    state.queue.selected.clear();
    state.queue.original.clear();
    state.queue.sort_mode = crate::app::state::QueueSortMode::QueueOrder;
    state.playback_mode = PlaybackMode::Queue;
    state.list_state.queue_index = play_idx;
    state.set_view(View::Queue);
    play_current_track(event_tx, state, client, audio);
}

/// Insert tracks into the queue immediately after the currently playing track.
/// If no track is playing, inserts at the beginning of the queue.
/// Does NOT start playback — just modifies the queue.
pub fn insert_tracks_next(state: &mut AppState, tracks: Vec<Track>) -> usize {
    // Convert radio to queue if needed
    if state.playback_mode == PlaybackMode::Radio {
        state.queue.tracks = state.radio.tracks.clone();
        state.queue.index = state.radio.track_index;
        state.playback_mode = PlaybackMode::Queue;
        state.radio.clear();
        if let Some(idx) = state.queue.index {
            state.list_state.queue_index = idx;
        }
    }

    state.queue.original.clear();
    state.queue.sort_mode = crate::app::state::QueueSortMode::QueueOrder;

    let insert_pos = state.queue.index.map(|idx| idx + 1).unwrap_or(0);
    let added = tracks.len();
    state.queue.tracks.splice(insert_pos..insert_pos, tracks);
    added
}

/// Play the current track from the queue.
pub fn play_current_track(
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,
    client: &PlexClient,
    audio: &mut AudioPlayer,
) {
    // Remote playback guard: when output is Remote, use remote player instead of local audio
    if let crate::app::state::OutputTarget::Remote { ref player_id, ref player_uri, .. } = state.remote.output_target {
        play_current_track_remote(event_tx, state, client, player_id.clone(), player_uri.clone());
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
        state.playback.scrobble_reported = false;
        state.playback.preparation_id = state.playback.preparation_id.wrapping_add(1);
        let preparation_id = state.playback.preparation_id;

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
                if client.server_url().is_some() {
                    state.artwork.loading = true;
                    state.artwork.pending_thumb = Some(thumb_path.to_string());
                    let thumb_path_owned = thumb_path.to_string();
                    let event_tx = event_tx.clone();
                    let client = client.clone();
                    let generation = state.artwork.grid_generation;

                    tokio::spawn(async move {
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            client.fetch_artwork(&thumb_path_owned, 600)
                        ).await {
                            Ok(Ok(data)) => {
                                let _ = event_tx.send(ArtworkEvent::ArtworkLoaded {
                                    generation,
                                    thumb_path: thumb_path_owned,
                                    data,
                                }.into()).await;
                            }
                            Ok(Err(e)) => {
                                tracing::warn!("Failed to load artwork: {}", e);
                                let _ = event_tx.send(ArtworkEvent::ArtworkFailed {
                                    generation,
                                    thumb_path: thumb_path_owned,
                                }.into()).await;
                            }
                            Err(_) => {
                                tracing::warn!("Artwork loading timed out");
                                let _ = event_tx.send(ArtworkEvent::ArtworkFailed {
                                    generation,
                                    thumb_path: thumb_path_owned,
                                }.into()).await;
                            }
                        }
                    });
                } else {
                    state.artwork.loading = false;
                    state.artwork.pending_thumb = None;
                    state.artwork.current_data = None;
                }
            } else {
                state.artwork.loading = false;
                state.artwork.pending_thumb = None;
            }
        } else if let Some(artist_thumb) = find_artist_thumb(&track, &state.library.artists) {
            if state.artwork.current_thumb.as_deref() != Some(&artist_thumb) {
                if client.server_url().is_some() {
                    state.artwork.loading = true;
                    state.artwork.pending_thumb = Some(artist_thumb.clone());
                    let thumb_path_owned = artist_thumb.clone();
                    let event_tx = event_tx.clone();
                    let client = client.clone();
                    let generation = state.artwork.grid_generation;

                    tokio::spawn(async move {
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            client.fetch_artwork(&thumb_path_owned, 600)
                        ).await {
                            Ok(Ok(data)) => {
                                let _ = event_tx.send(ArtworkEvent::ArtworkLoaded {
                                    generation,
                                    thumb_path: thumb_path_owned,
                                    data,
                                }.into()).await;
                            }
                            Ok(Err(e)) => {
                                tracing::warn!("Failed to load artist artwork: {}", e);
                                let _ = event_tx.send(ArtworkEvent::ArtworkFailed {
                                    generation,
                                    thumb_path: thumb_path_owned,
                                }.into()).await;
                            }
                            Err(_) => {
                                tracing::warn!("Artist artwork loading timed out");
                                let _ = event_tx.send(ArtworkEvent::ArtworkFailed {
                                    generation,
                                    thumb_path: thumb_path_owned,
                                }.into()).await;
                            }
                        }
                    });
                } else {
                    state.artwork.loading = false;
                    state.artwork.pending_thumb = None;
                    state.artwork.current_data = None;
                }
            } else {
                state.artwork.loading = false;
                state.artwork.pending_thumb = None;
            }
        } else {
            state.artwork.current_thumb = None;
            state.artwork.current_data = None;
            state.artwork.loading = false;
            state.artwork.pending_thumb = None;
        }

        // Check track cache first (pre-fetched audio data)
        if let Some(cached_data) = audio.track_cache.get(&track.rating_key) {
            tracing::info!("Cache hit for: {} - {}", track.artist_name(), track.title);
            match audio.play_data(cached_data) {
                Ok(()) => {
                    state.playback.request_id = audio.playback_id();
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

        if state.transcode_kbps > 0 {
            // Universal-transcode negotiation is a network round trip. Stop the
            // old stream now, then resolve it as a versioned background effect.
            audio.stop();
            state.playback.request_id = audio.playback_id();
            let bitrate = state.transcode_kbps;
            let track_key = track.rating_key.clone();
            let request_track = track.clone();
            let request_client = client.clone();
            let tx = event_tx.clone();
            tokio::spawn(async move {
                let result = request_client
                    .get_transcoded_stream_url(&request_track, bitrate)
                    .await
                    .map_err(|error| {
                        crate::app::action::AsyncError::from_api(
                            "Failed to prepare transcoded stream",
                            &error,
                        )
                    });
                let _ = tx
                    .send(
                        PlaybackEvent::TranscodeUrlReady {
                            preparation_id,
                            track_key,
                            result,
                        }
                        .into(),
                    )
                    .await;
            });
            return;
        }

        match client.get_stream_url(&track) {
            Ok(url) => start_resolved_stream(
                event_tx,
                state,
                client,
                audio,
                &track,
                &url,
                false,
            ),
            Err(error) => {
                tracing::error!("Cannot build direct stream URL: {}", error);
                state.set_error("Failed to get stream URL".to_string());
                state.playback.status = PlayStatus::Stopped;
            }
        }
    }
}

/// Attach a resolved URL to the bounded network → decoder pipeline. This is
/// synchronous from the reducer's perspective: it only spawns the HTTP
/// producer and sends an actor command.
pub fn start_resolved_stream(
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,
    client: &PlexClient,
    audio: &mut AudioPlayer,
    track: &Track,
    url: &str,
    transcoded: bool,
) {
    let audio_tx = audio_event_adapter(event_tx);
    let headers = if transcoded {
        reqwest::header::HeaderMap::new()
    } else {
        client.stream_headers()
    };
    let mode = if transcoded {
        format!("transcode {}kbps", state.transcode_kbps)
    } else {
        "direct".to_string()
    };
    tracing::debug!("Starting {} stream", mode);
    if let Err(error) = audio.play_url_with_headers(
        url,
        headers,
        None,
        audio_tx,
        client.http_client().clone(),
    ) {
        state.set_error(format!("Playback failed: {error}"));
        state.playback.status = PlayStatus::Stopped;
        return;
    }
    state.playback.request_id = audio.playback_id();
    report_playback_to_plex(event_tx, track, state.plex_session_id.clone(), client);
    state.last_progress_report = Some(std::time::Instant::now());
}

/// Report playback start to Plex server in background.
pub fn report_playback_to_plex(_event_tx: &mpsc::Sender<Event>, track: &Track, session_id: Option<String>, client: &PlexClient) {
    if client.server_url().is_some() {
        let track_clone = track.clone();
        let client = client.clone();

        tokio::spawn(async move {
            if let Err(e) = client.report_playback_start(&track_clone, 0, session_id.as_deref()).await {
                tracing::debug!("Failed to report playback start: {}", e);
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
    if client.server_url().is_some() {
        let track_clone = track.clone();
        let client = client.clone();

        tokio::spawn(async move {
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
    if client.server_url().is_some() {
        let track_clone = track.clone();
        let client = client.clone();

        tokio::spawn(async move {
            if let Err(e) = client.report_playback_progress(&track_clone, position_ms, session_id.as_deref()).await {
                tracing::debug!("Failed to report playback progress: {}", e);
            }
        });
    }
}

/// Mark a completed (or at least 90%-played) track as played on Plex.
/// The caller flips `PlaybackState::scrobble_reported` before invoking this
/// helper so repeated UI ticks cannot enqueue duplicate reports.
pub fn report_scrobble_to_plex(rating_key: String, client: &PlexClient) {
    if client.server_url().is_none() {
        return;
    }
    let client = client.clone();
    tokio::spawn(async move {
        if let Err(error) = client.scrobble(&rating_key).await {
            tracing::warn!("Failed to scrobble track {}: {}", rating_key, error);
        }
    });
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

        let event_tx =
            LibraryEventSender::new(event_tx.clone(), state.library_generation);
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
                            let _ = event_tx.send(RadioEvent::RadioTracksLoaded {
                                tracks,
                                time_travel_index: Some(current_index + 3),
                            }.into()).await;
                        }
                        Err(e) => {
                            tracing::warn!("Time Travel Radio: failed to fetch more tracks: {}", e);
                            // Send empty result to clear fetching flag
                            let _ = event_tx.send(RadioEvent::RadioTracksLoaded {
                                tracks: vec![],
                                time_travel_index: None,
                            }.into()).await;
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
                    let _ = event_tx.send(RadioEvent::RadioTracksLoaded {
                        tracks,
                        time_travel_index: None,
                    }.into()).await;
                }
                Err(e) => {
                    tracing::warn!("Failed to fetch more radio tracks: {}", e);
                    let _ = event_tx.send(RadioEvent::RadioTracksLoaded {
                        tracks: vec![],
                        time_travel_index: None,
                    }.into()).await;
                }
            }
        });
    } else {
        state.radio.fetching = false;
    }
}

/// Play the current track on a remote Plex player.
fn play_current_track_remote(
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
        state.playback.preparation_id = state.playback.preparation_id.wrapping_add(1);
        state.playback.request_id = state.playback.request_id.wrapping_add(1);
        let playback_id = state.playback.request_id;
        state.playback.duration_ms = track.duration_ms();
        state.playback.position_ms = 0;
        state.playback.scrobble_reported = false;

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
                if client.server_url().is_some() {
                    state.artwork.loading = true;
                    state.artwork.pending_thumb = Some(thumb_path.to_string());
                    let thumb_path_owned = thumb_path.to_string();
                    let event_tx_clone = event_tx.clone();
                    let client = client.clone();
                    let generation = state.artwork.grid_generation;

                    tokio::spawn(async move {
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            client.fetch_artwork(&thumb_path_owned, 600)
                        ).await {
                            Ok(Ok(data)) => {
                                let _ = event_tx_clone.send(ArtworkEvent::ArtworkLoaded {
                                    generation,
                                    thumb_path: thumb_path_owned,
                                    data,
                                }.into()).await;
                            }
                            _ => {
                                let _ = event_tx_clone.send(ArtworkEvent::ArtworkFailed {
                                    generation,
                                    thumb_path: thumb_path_owned,
                                }.into()).await;
                            }
                        }
                    });
                } else {
                    state.artwork.loading = false;
                    state.artwork.pending_thumb = None;
                }
            } else {
                state.artwork.loading = false;
                state.artwork.pending_thumb = None;
            }
        } else if let Some(artist_thumb) = find_artist_thumb(&track, &state.library.artists) {
            if state.artwork.current_thumb.as_deref() != Some(&artist_thumb) {
                if client.server_url().is_some() {
                    state.artwork.loading = true;
                    state.artwork.pending_thumb = Some(artist_thumb.clone());
                    let thumb_path_owned = artist_thumb.clone();
                    let event_tx_clone = event_tx.clone();
                    let client = client.clone();
                    let generation = state.artwork.grid_generation;

                    tokio::spawn(async move {
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            client.fetch_artwork(&thumb_path_owned, 600)
                        ).await {
                            Ok(Ok(data)) => {
                                let _ = event_tx_clone.send(ArtworkEvent::ArtworkLoaded {
                                    generation,
                                    thumb_path: thumb_path_owned,
                                    data,
                                }.into()).await;
                            }
                            _ => {
                                let _ = event_tx_clone.send(ArtworkEvent::ArtworkFailed {
                                    generation,
                                    thumb_path: thumb_path_owned,
                                }.into()).await;
                            }
                        }
                    });
                } else {
                    state.artwork.loading = false;
                    state.artwork.pending_thumb = None;
                }
            } else {
                state.artwork.loading = false;
                state.artwork.pending_thumb = None;
            }
        } else {
            state.artwork.current_thumb = None;
            state.artwork.current_data = None;
            state.artwork.loading = false;
            state.artwork.pending_thumb = None;
        }

        // Send playMedia to remote player via server
        let token = client.shared_token_or_empty();
        let client_id = client.client_identifier().to_string();
        let server_url = client.server_url().unwrap_or("").to_string();
        let machine_id = state.active_server_id.clone()
            .or_else(|| state.available_servers.first()
                .map(|server| server.client_identifier.clone()))
            .unwrap_or_default();
        let lib_key = state.active_library.clone().unwrap_or_default();
        let event_tx_clone = event_tx.clone();
        let result_player_id = target_player_id.clone();
        let result_track_key = track.rating_key.clone();
        let library_generation = state.library_generation;

        tokio::spawn(async move {
            let result = match crate::plex::RemotePlayerClient::new(
                token, client_id, target_player_id, server_url, machine_id, player_uri,
            ) {
                Ok(client) => client.play_media(&track, &lib_key).await,
                Err(error) => Err(error),
            };
            let event = RemoteEvent::RemotePlayResult {
                player_id: result_player_id,
                playback_id,
                track_key: result_track_key,
                error: result.err().map(|error| error.to_string()),
            };
            let _ = event_tx_clone
                .send(Event::for_library(library_generation, event))
                .await;
        });
    }
}
