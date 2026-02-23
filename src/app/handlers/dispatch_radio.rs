//! Radio dispatch handlers: StopRadio, JumpToRadioTrack, StartPlexRadio,
//! PlayStation, DrillIntoStation, NavigateStationsBack, PlayCurrentRadioTrack,
//! ToggleDjMode, DjModeProcess, DjModeTracksReady, DjModeBatchReady.

use crate::app::{Action, AppState, Event};
use crate::app::state::{DjMode, PlaybackMode, RadioMode, View};
use crate::plex::PlexClient;
use crate::audio::AudioPlayer;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Timeout for station queue creation and radio startup (seconds).
const STATION_TIMEOUT_SECS: u64 = 30;

/// Number of tracks to fetch when looking for similar tracks.
const SIMILAR_TRACKS_FETCH_LIMIT: u32 = 50;

/// Number of DJ tracks to insert after the current track on each transition (continuous modes).
const DJ_INSERT_COUNT: usize = 2;

/// Number of nearby tracks (before and after current) to consider for diversity context.
const NEARBY_TRACKS_WINDOW: usize = 5;


/// Dispatch radio and station actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: Action,
    state: &mut AppState,
    client: &mut PlexClient,
    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    match action {
        Action::JumpToRadioTrack(idx) => {
            // Report stop for current track before jumping
            // continuing=true because we're jumping to another track
            if let Some(track) = state.current_track().cloned() {
                helpers::report_playback_stop_to_plex(&track, state.playback.position_ms, true, state.plex_session_id.clone(), client);
            }

            // Jump to track in radio queue without clearing radio state
            if idx < state.radio.tracks.len() {
                state.radio.track_index = Some(idx);
                state.list_state.queue_index = idx;
                helpers::play_current_track(event_tx, state, client, audio).await;
            }
        }
        Action::StartPlexRadio { key, title } => {
            // Start radio using Plex playQueue API (full heuristics)
            if let Some(current) = state.current_track().cloned() {
                helpers::report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
            }

            audio.stop();
            state.playback.status = crate::app::state::PlayStatus::Stopped;
            state.plex_session_id = Some(helpers::generate_plex_session_id());

            // Deactivate DJ mode when starting station/radio
            state.dj.active_mode = None;
            state.dj.history.clear();
            state.dj.inserting = false;
            state.dj.last_was_inserted = false;

            state.radio_state.mode = RadioMode::Active; // Plex decides the actual mix
            state.radio_state.seed_track_key = Some(key.clone());
            state.radio_state.seed_title = title.clone();
            state.radio_state.history.clear();
            state.radio_state.fetching = true;
            state.playback_mode = PlaybackMode::Radio;

            // Immediately update radio display state so the title shows the new
            // station name right away, not the previous station's name.
            state.radio.clear();
            state.radio.active_station = Some(crate::app::state::ActiveStation {
                key: format!("plex_radio:{}", key),
                title: title.clone(),
            });

            state.set_view(View::Queue);
            state.set_status(format!("Starting radio: {}...", title));

            // Pre-load artist artwork for the radio display
            let artist_thumb = state.artists.iter()
                .find(|a| a.rating_key == key)
                .and_then(|a| a.thumb.clone());
            if let Some(ref thumb) = artist_thumb {
                let art_tx = event_tx.clone();
                let art_client = client.clone();
                let thumb_path = thumb.clone();
                tokio::spawn(async move {
                    match art_client.fetch_artwork(&thumb_path, 300).await {
                        Ok(data) => {
                            let _ = art_tx.send(Event::ArtworkLoaded { thumb_path, data }).await;
                        }
                        Err(e) => {
                            tracing::debug!("Failed to load artist artwork for radio: {}", e);
                        }
                    }
                });
            }

            let tx = event_tx.clone();
            let mut client_clone = client.clone();
            let rk = key.clone();
            let rt = title.clone();
            tokio::spawn(async move {
                let timeout_duration = std::time::Duration::from_secs(STATION_TIMEOUT_SECS);
                match tokio::time::timeout(timeout_duration, client_clone.create_radio_from_metadata(&rk)).await {
                    Ok(Ok(tracks)) => {
                        let _ = tx.send(Event::StationTracksLoaded {
                            station_key: format!("plex_radio:{}", rk),
                            station_title: rt,
                            tracks,
                            time_travel_decades: vec![],
                        }).await;
                    }
                    Ok(Err(e)) => {
                        let _ = tx.send(Event::StationLoadFailed {
                            station_key: format!("plex_radio:{}", rk),
                            error: format!("Failed to start radio: {}", e),
                        }).await;
                    }
                    Err(_) => {
                        let _ = tx.send(Event::StationLoadFailed {
                            station_key: format!("plex_radio:{}", rk),
                            error: "Radio creation timed out".into(),
                        }).await;
                    }
                }
            });
        }
        Action::PlayStation(station_key) => {
            // Report stop for currently playing track before starting station
            // continuing=true because we're starting new content
            if let Some(current) = state.current_track().cloned() {
                helpers::report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
            }

            // Stop audio immediately to prevent stale TrackEnded events from the
            // old track being processed after the new station starts playing.
            audio.stop();
            state.playback.status = crate::app::state::PlayStatus::Stopped;

            // Generate new session ID for this playback context
            state.plex_session_id = Some(helpers::generate_plex_session_id());

            // Deactivate DJ mode when starting station
            state.dj.active_mode = None;
            state.dj.history.clear();
            state.dj.inserting = false;
            state.dj.last_was_inserted = false;

            // Find station title from station_nav (Miller columns) or fall back to legacy state.stations
            let station_title = state.station_nav.selected_station()
                .filter(|s| s.key == station_key)
                .map(|s| s.title.clone())
                .or_else(|| {
                    // Search all columns
                    state.station_nav.columns.iter()
                        .flat_map(|col| col.stations.iter())
                        .find(|s| s.key == station_key)
                        .map(|s| s.title.clone())
                })
                .or_else(|| {
                    // Fall back to legacy state.stations
                    state.stations.iter()
                        .find(|s| s.key == station_key)
                        .map(|s| s.title.clone())
                })
                .unwrap_or_else(|| "Radio".to_string());

            // Immediately update radio display state so the title is correct right away
            state.radio.clear();
            state.radio.active_station = Some(crate::app::state::ActiveStation {
                key: station_key.clone(),
                title: station_title.clone(),
            });
            state.playback_mode = PlaybackMode::Radio;
            state.set_status(format!("Loading {}...", station_title));

            // Spawn background task for station queue creation (non-blocking)
            let tx = event_tx.clone();
            let mut client_clone = client.clone();
            let sk = station_key.clone();
            let st = station_title.clone();
            let lib_key = state.active_library.clone();
            tokio::spawn(async move {
                let queue_future = client_clone.create_station_queue(&sk);
                let timeout_duration = std::time::Duration::from_secs(STATION_TIMEOUT_SECS);

                match tokio::time::timeout(timeout_duration, queue_future).await {
                    Ok(Ok(tracks)) => {
                        // For Time Travel Radio: fetch decades in same background task
                        let time_travel_decades = if sk.contains("timeTravel") {
                            if let Some(ref lk) = lib_key {
                                client_clone.get_time_travel_decades(lk).await.unwrap_or_default()
                            } else {
                                vec![]
                            }
                        } else {
                            vec![]
                        };

                        let _ = tx.send(Event::StationTracksLoaded {
                            station_key: sk,
                            station_title: st,
                            tracks,
                            time_travel_decades,
                        }).await;
                    }
                    Ok(Err(e)) => {
                        let _ = tx.send(Event::StationLoadFailed {
                            station_key: sk,
                            error: format!("Failed to start station: {}", e),
                        }).await;
                    }
                    Err(_) => {
                        tracing::warn!("Station queue creation timed out after {} seconds: {}", STATION_TIMEOUT_SECS, sk);
                        let _ = tx.send(Event::StationLoadFailed {
                            station_key: sk,
                            error: "Station timed out - try a different station".into(),
                        }).await;
                    }
                }
            });
        }
        Action::DrillIntoStation(station_key, station_title) => {
            // Check cache first for instant loading
            if let Some(cached_children) = state.station_children_cache.get(&station_key).cloned() {
                state.station_nav.push_column(crate::app::state::StationColumn::new(
                    Some(station_key),
                    station_title,
                    cached_children.clone(),
                ));
                state.stations = cached_children;
                state.clear_error();
                return Ok(vec![]);
            }

            // Drill into a station category (e.g., Mood Radio -> sub-moods)
            state.stations_loading = true;
            state.station_nav.loading = true;
            state.set_status(format!("Loading {}...", station_title));

            // Spawn background task for child loading (non-blocking)
            let tx = event_tx.clone();
            let mut client_clone = client.clone();
            let sk = station_key.clone();
            let st = station_title.clone();
            tokio::spawn(async move {
                match client_clone.get_station_children(&sk).await {
                    Ok(children) => {
                        if children.is_empty() {
                            // No children - treat as playable station
                            let queue_future = client_clone.create_station_queue(&sk);
                            let timeout_duration = std::time::Duration::from_secs(STATION_TIMEOUT_SECS);

                            match tokio::time::timeout(timeout_duration, queue_future).await {
                                Ok(Ok(tracks)) => {
                                    let _ = tx.send(Event::StationTracksLoaded {
                                        station_key: sk,
                                        station_title: st,
                                        tracks,
                                        time_travel_decades: vec![],
                                    }).await;
                                }
                                Ok(Err(e)) => {
                                    let _ = tx.send(Event::StationLoadFailed {
                                        station_key: sk,
                                        error: format!("Failed to start station: {}", e),
                                    }).await;
                                }
                                Err(_) => {
                                    tracing::warn!("Station queue creation timed out after {} seconds: {}", STATION_TIMEOUT_SECS, sk);
                                    let _ = tx.send(Event::StationLoadFailed {
                                        station_key: sk,
                                        error: "Station timed out - try a different station".into(),
                                    }).await;
                                }
                            }
                        } else {
                            let _ = tx.send(Event::StationChildrenLoaded {
                                station_key: sk,
                                station_title: st,
                                children,
                            }).await;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Event::StationLoadFailed {
                            station_key: sk,
                            error: format!("Failed to load station children: {}", e),
                        }).await;
                    }
                }
            });
        }
        Action::NavigateStationsBack => {
            state.scroll.station_back_highlighted = false;
            // Go back in Miller columns (just move focus left - data already in memory)
            if state.station_nav.can_go_left() {
                state.station_nav.focus_left();
                // Update legacy state to match focused column
                if let Some(col) = state.station_nav.focused() {
                    state.stations = col.stations.clone();
                }
            }
        }
        Action::PlayCurrentRadioTrack => {
            // Play the current track in radio mode (stays in Radio playback mode)
            state.consecutive_playback_errors = 0;
            helpers::play_current_track(event_tx, state, client, audio).await;
        }
        Action::ToggleDjMode(mode) => {
            tracing::info!("ToggleDjMode: {:?}, current_mode={:?}, playback_mode={:?}, queue_len={}, queue_index={:?}, current_track={}",
                mode, state.dj.active_mode, state.playback_mode,
                state.queue.len(), state.queue_index,
                state.current_track().map(|t| t.title.as_str()).unwrap_or("None"));

            if state.dj.active_mode == Some(mode) {
                // Same mode active → deactivate
                state.dj.active_mode = None;
                state.dj.history.clear();
                state.dj.inserting = false;
                state.dj.last_was_inserted = false;
                state.set_status(format!("{} off", mode.name()));
            } else {
                // DJ + Station mutual exclusivity: convert radio to queue if active
                if state.playback_mode == PlaybackMode::Radio {
                    tracing::info!("DJ mode: converting radio to queue (radio.tracks={}, radio.track_index={:?})",
                        state.radio.tracks.len(), state.radio.track_index);
                    state.queue = state.radio.tracks.clone();
                    state.queue_index = state.radio.track_index;
                    state.playback_mode = PlaybackMode::Queue;
                    state.radio.clear();
                }

                // Activate new mode (or switch from a different one)
                state.dj.active_mode = Some(mode);
                state.dj.history.clear();
                state.dj.inserting = false;
                state.dj.last_was_inserted = false;
                state.set_status(format!("{} on", mode.name()));

                // All DJ modes are continuous: insert DJ tracks after current position
                return Ok(vec![Action::DjModeProcess]);
            }
        }
        Action::DjModeProcess => {
            // Only for continuous modes (Freeze, Contempo, Groupie)
            dispatch_dj_continuous(event_tx, state, client).await;
        }
        Action::DjModeTracksReady(tracks, _insert_next, error) => {
            // Insert DJ-picked tracks right after the current track position.
            state.dj.inserting = false;

            if tracks.is_empty() {
                // Show specific error or generic hint
                if let Some(err_msg) = error {
                    state.set_error(err_msg);
                } else if let Some(mode) = state.dj.active_mode {
                    let hint = match mode {
                        DjMode::Freeze | DjMode::Gemini | DjMode::Stretch =>
                            format!("{}: no similar tracks found (requires Sonic Analysis)", mode.name()),
                        _ =>
                            format!("{}: no matching tracks found", mode.name()),
                    };
                    state.set_status(hint);
                }
                return Ok(vec![]);
            }

            for track in &tracks {
                state.dj.history.push(track.rating_key.clone());
            }

            // Mark that the next track to play is a DJ insertion.
            // Interleaving modes use this to alternate: original → DJ → original.
            state.dj.last_was_inserted = true;

            insert_tracks_after_current(state, tracks);

            // Pre-cache the newly inserted DJ tracks (and other upcoming tracks)
            let upcoming = helpers::get_upcoming_tracks(state);
            crate::audio::cache::trigger_prefetch(&audio.track_cache, &upcoming, client);
        }
        Action::DjModeBatchReady(inserts) => {
            // Inserter mode batch results: interleave into queue
            state.dj.inserting = false;

            if inserts.is_empty() {
                return Ok(vec![]);
            }

            // Collect inserts into a map: original_index -> tracks_to_insert_after
            let inserts_map: std::collections::HashMap<usize, Vec<crate::plex::models::Track>> =
                inserts.into_iter().collect();

            // Add all inserted track keys to history
            for tracks in inserts_map.values() {
                for track in tracks {
                    state.dj.history.push(track.rating_key.clone());
                }
            }

            // Process inserts in reverse index order so earlier splices don't shift later indices
            let mut positions: Vec<usize> = inserts_map.keys().copied().collect();
            positions.sort_unstable_by(|a, b| b.cmp(a));

            match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => {
                    for pos in positions {
                        if let Some(insert_tracks) = inserts_map.get(&pos) {
                            let insert_at = (pos + 1).min(state.queue.len());
                            state.queue.splice(insert_at..insert_at, insert_tracks.iter().cloned());
                        }
                    }
                }
                PlaybackMode::Radio => {
                    for pos in positions {
                        if let Some(insert_tracks) = inserts_map.get(&pos) {
                            let insert_at = (pos + 1).min(state.radio.tracks.len());
                            state.radio.tracks.splice(insert_at..insert_at, insert_tracks.iter().cloned());
                        }
                    }
                }
            }

            // Pre-cache upcoming tracks (including newly interleaved DJ tracks)
            let upcoming = helpers::get_upcoming_tracks(state);
            crate::audio::cache::trigger_prefetch(&audio.track_cache, &upcoming, client);
        }
        _ => unreachable!("dispatch_radio called with non-radio action: {:?}", action),
    }
    Ok(vec![])
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Insert tracks right after the current playback position.
/// Works for both Queue and Radio modes.
fn insert_tracks_after_current(state: &mut AppState, tracks: Vec<crate::plex::models::Track>) {
    match state.playback_mode {
        PlaybackMode::Queue | PlaybackMode::None => {
            let insert_at = state.queue_index.unwrap_or(0) + 1;
            let insert_at = insert_at.min(state.queue.len());
            state.queue.splice(insert_at..insert_at, tracks);
        }
        PlaybackMode::Radio => {
            let insert_at = state.radio.track_index.unwrap_or(0) + 1;
            let insert_at = insert_at.min(state.radio.tracks.len());
            state.radio.tracks.splice(insert_at..insert_at, tracks);
        }
    }
}

/// Build sets of artist/album keys from tracks near the current playback position.
/// Used to seed diversity filtering so DJ picks avoid recently-heard artists/albums.
fn build_diversity_context(state: &AppState) -> (std::collections::HashSet<String>, std::collections::HashSet<String>) {
    let mut used_artists = std::collections::HashSet::new();
    let mut used_albums = std::collections::HashSet::new();

    // Add current track
    if let Some(t) = state.current_track() {
        if let Some(a) = t.grandparent_rating_key.as_deref() {
            used_artists.insert(a.to_string());
        }
        if let Some(a) = t.parent_rating_key.as_deref() {
            used_albums.insert(a.to_string());
        }
    }

    // Gather from nearby queue tracks
    let nearby: &[crate::plex::models::Track] = match state.playback_mode {
        PlaybackMode::Queue | PlaybackMode::None => {
            let idx = state.queue_index.unwrap_or(0);
            let start = idx.saturating_sub(NEARBY_TRACKS_WINDOW);
            let end = (idx + NEARBY_TRACKS_WINDOW).min(state.queue.len());
            &state.queue[start..end]
        }
        PlaybackMode::Radio => {
            let idx = state.radio.track_index.unwrap_or(0);
            let start = idx.saturating_sub(NEARBY_TRACKS_WINDOW);
            let end = (idx + NEARBY_TRACKS_WINDOW).min(state.radio.tracks.len());
            &state.radio.tracks[start..end]
        }
    };
    for t in nearby {
        if let Some(a) = t.grandparent_rating_key.as_deref() {
            used_artists.insert(a.to_string());
        }
        if let Some(a) = t.parent_rating_key.as_deref() {
            used_albums.insert(a.to_string());
        }
    }

    (used_artists, used_albums)
}

// ---------------------------------------------------------------------------
// DJ continuous processing (all modes are continuous)
// ---------------------------------------------------------------------------

/// Pick up to `count` diverse tracks from candidates, avoiding already-used tracks,
/// and preferring different artists and albums from those recently seen.
///
/// Three-phase selection:
/// 1. Different artist AND different album from all `used_artists`/`used_albums`
/// 2. Relax: just different track (not in history)
/// 3. Last resort: any candidate not already picked in this batch
fn pick_diverse(
    candidates: Vec<crate::plex::models::Track>,
    count: usize,
    history: &[String],
    used_artists: &std::collections::HashSet<String>,
    used_albums: &std::collections::HashSet<String>,
) -> Vec<crate::plex::models::Track> {
    let mut result = Vec::with_capacity(count);
    let mut picked_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut picked_artists: std::collections::HashSet<String> = used_artists.clone();
    let mut picked_albums: std::collections::HashSet<String> = used_albums.clone();

    // Phase 1: Diverse — different track, different artist, different album
    for t in &candidates {
        if result.len() >= count { break; }
        if history.contains(&t.rating_key) || picked_keys.contains(&t.rating_key) { continue; }
        let artist = t.grandparent_rating_key.as_deref().unwrap_or("");
        let album = t.parent_rating_key.as_deref().unwrap_or("");
        if !artist.is_empty() && picked_artists.contains(artist) { continue; }
        if !album.is_empty() && picked_albums.contains(album) { continue; }
        picked_keys.insert(t.rating_key.clone());
        if !artist.is_empty() { picked_artists.insert(artist.to_string()); }
        if !album.is_empty() { picked_albums.insert(album.to_string()); }
        result.push(t.clone());
    }

    // Phase 2: Relax artist/album — just avoid history and already-picked
    if result.len() < count {
        for t in &candidates {
            if result.len() >= count { break; }
            if history.contains(&t.rating_key) || picked_keys.contains(&t.rating_key) { continue; }
            picked_keys.insert(t.rating_key.clone());
            result.push(t.clone());
        }
    }

    // Phase 3: Last resort — any candidate not yet picked
    if result.len() < count {
        for t in &candidates {
            if result.len() >= count { break; }
            if picked_keys.contains(&t.rating_key) { continue; }
            picked_keys.insert(t.rating_key.clone());
            result.push(t.clone());
        }
    }

    result
}

/// Process DJ mode logic for all modes.
/// Called on every track transition.
///
/// - **Interleaving modes** (Gemini, Twofer, Stretch): only insert after original
///   queue tracks, not after DJ-inserted tracks. This alternates: original → DJ → original → DJ.
/// - **Continuous modes** (Freeze, Contempo, Groupie): always insert after the current
///   track, pushing original queue tracks further down so you only hear DJ picks.
async fn dispatch_dj_continuous(
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,
    client: &mut PlexClient,
) {
    use crate::app::state::DjMode;

    let mode = match state.dj.active_mode {
        Some(m) => m,
        None => {
            tracing::warn!("dispatch_dj_continuous: no active DJ mode");
            return;
        }
    };

    tracing::info!("dispatch_dj_continuous: mode={:?}, playback_mode={:?}, queue_len={}, queue_index={:?}",
        mode, state.playback_mode, state.queue.len(), state.queue_index);

    // Interleaving modes: skip insertion when the last track was DJ-inserted
    // (let the next original queue track play through)
    if mode.is_interleaving() && state.dj.last_was_inserted {
        state.dj.last_was_inserted = false;
        tracing::info!("{}: skipping (last was DJ-inserted, letting original play)", mode.name());
        return;
    }

    let current_track = match state.current_track().cloned() {
        Some(t) => {
            tracing::info!("{}: seed track = {} (key={})", mode.name(), t.title, t.rating_key);
            t
        }
        None => {
            tracing::warn!("{}: no current track, cannot process DJ mode", mode.name());
            state.set_status(format!("{}: no track playing", mode.name()));
            state.dj.inserting = false;
            return;
        }
    };

    // Get the next track in queue (needed for Twofer and Stretch)
    let next_track = match state.playback_mode {
        PlaybackMode::Queue | PlaybackMode::None => {
            let idx = state.queue_index.unwrap_or(0);
            state.queue.get(idx + 1).cloned()
        }
        PlaybackMode::Radio => {
            let idx = state.radio.track_index.unwrap_or(0);
            state.radio.tracks.get(idx + 1).cloned()
        }
    };

    state.dj.inserting = true;

    let (used_artists, used_albums) = build_diversity_context(state);

    let tx = event_tx.clone();
    let client_clone = client.clone();
    let history = state.dj.history.clone();
    let lib_key = state.active_library.clone();

    tokio::spawn(async move {
        tracing::info!("{}: spawning async task for track key={}", mode.name(), current_track.rating_key);
        let result: Vec<crate::plex::models::Track> = match mode {
            DjMode::Gemini => {
                dispatch_dj_gemini(&client_clone, &current_track, &history, &used_artists, &used_albums).await
            }
            DjMode::Twofer => {
                dispatch_dj_twofer(&client_clone, &current_track, next_track.as_ref(), &history).await
            }
            DjMode::Stretch => {
                dispatch_dj_stretch(&client_clone, &current_track, next_track.as_ref(), &history, &used_artists, &used_albums).await
            }
            DjMode::Freeze => {
                dispatch_dj_freeze(&client_clone, &current_track, &history, &used_artists, &used_albums).await
            }
            DjMode::Contempo => {
                dispatch_dj_contempo(&client_clone, &current_track, &history, &used_artists, &used_albums, lib_key.as_deref()).await
            }
            DjMode::Groupie => {
                dispatch_dj_groupie(&client_clone, &current_track, &history, &used_albums).await
            }
        };

        tracing::info!("{}: async task completed, {} tracks found", mode.name(), result.len());
        // Always send the event, even if empty — this ensures dj_inserting gets cleared
        let _ = tx.send(Event::DjTracksReady { tracks: result, insert_next: true, error: None }).await;
    });
}

/// DJ Gemini (continuous): insert the most sonically similar track after current.
async fn dispatch_dj_gemini(
    client: &PlexClient,
    current_track: &crate::plex::models::Track,
    history: &[String],
    used_artists: &std::collections::HashSet<String>,
    used_albums: &std::collections::HashSet<String>,
) -> Vec<crate::plex::models::Track> {
    match client.get_similar_tracks(&current_track.rating_key, SIMILAR_TRACKS_FETCH_LIMIT).await {
        Ok(tracks) => pick_diverse(tracks, 1, history, used_artists, used_albums),
        Err(e) => {
            tracing::warn!("DJ Gemini: similar tracks failed: {}", e);
            vec![]
        }
    }
}

/// DJ Twofer (continuous): insert a same-artist track when the next track differs.
/// Skips insertion if the next track is already by the same artist.
async fn dispatch_dj_twofer(
    client: &PlexClient,
    current_track: &crate::plex::models::Track,
    next_track: Option<&crate::plex::models::Track>,
    history: &[String],
) -> Vec<crate::plex::models::Track> {
    let current_artist = current_track.grandparent_rating_key.as_deref().unwrap_or("");
    if current_artist.is_empty() {
        return vec![];
    }

    // Skip if next track is already by the same artist
    if let Some(next) = next_track {
        let next_artist = next.grandparent_rating_key.as_deref().unwrap_or("");
        if next_artist == current_artist {
            tracing::debug!("DJ Twofer: next track is same artist, skipping");
            return vec![];
        }
    }

    match client.get_artist_all_tracks(current_artist).await {
        Ok(tracks) => {
            let candidates: Vec<_> = tracks.into_iter()
                .filter(|t| {
                    t.rating_key != current_track.rating_key
                        && !history.contains(&t.rating_key)
                })
                .collect();
            if candidates.is_empty() {
                return vec![];
            }
            use rand::prelude::IndexedRandom;
            let mut rng = rand::rng();
            match candidates.choose(&mut rng) {
                Some(pick) => vec![pick.clone()],
                None => vec![],
            }
        }
        Err(e) => {
            tracing::warn!("DJ Twofer: artist tracks failed: {}", e);
            vec![]
        }
    }
}

/// DJ Stretch (continuous): insert a bridge track between current and next.
/// Uses looser sonic distance (0.5) to find a midpoint bridge.
async fn dispatch_dj_stretch(
    client: &PlexClient,
    current_track: &crate::plex::models::Track,
    next_track: Option<&crate::plex::models::Track>,
    history: &[String],
    used_artists: &std::collections::HashSet<String>,
    used_albums: &std::collections::HashSet<String>,
) -> Vec<crate::plex::models::Track> {
    let next = match next_track {
        Some(t) => t,
        None => {
            tracing::debug!("DJ Stretch: no next track, skipping");
            return vec![];
        }
    };

    // Get similar tracks for both current and next with looser distance
    let current_similar = client.get_similar_tracks_with_distance(
        &current_track.rating_key, SIMILAR_TRACKS_FETCH_LIMIT, 0.5
    ).await;
    let next_similar = client.get_similar_tracks_with_distance(
        &next.rating_key, SIMILAR_TRACKS_FETCH_LIMIT, 0.5
    ).await;

    let current_tracks = match current_similar {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("DJ Stretch: current similar failed: {}", e);
            return vec![];
        }
    };
    let next_tracks = match next_similar {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("DJ Stretch: next similar failed: {}", e);
            return vec![];
        }
    };

    // Find tracks in the intersection of both sets (bridge candidates)
    let next_keys: std::collections::HashSet<&str> = next_tracks.iter()
        .map(|t| t.rating_key.as_str())
        .collect();

    let bridge_candidates: Vec<_> = current_tracks.into_iter()
        .filter(|t| {
            next_keys.contains(t.rating_key.as_str())
                && t.rating_key != current_track.rating_key
                && t.rating_key != next.rating_key
                && !history.contains(&t.rating_key)
        })
        .collect();

    if !bridge_candidates.is_empty() {
        return pick_diverse(bridge_candidates, 1, history, used_artists, used_albums);
    }

    // Fallback: pick from current's similar tracks (not in intersection)
    let fallback: Vec<_> = next_tracks.into_iter()
        .filter(|t| {
            t.rating_key != current_track.rating_key
                && t.rating_key != next.rating_key
                && !history.contains(&t.rating_key)
        })
        .collect();

    pick_diverse(fallback, 1, history, used_artists, used_albums)
}

/// DJ Freeze: sonically similar tracks to the current one.
/// Tries increasing maxDistance (0.25 → 0.5 → 0.75) if initial results are empty.
/// Used when the probe call was not made (non-Freeze modes that fall back here won't use this).
async fn dispatch_dj_freeze(
    client: &PlexClient,
    current_track: &crate::plex::models::Track,
    history: &[String],
    used_artists: &std::collections::HashSet<String>,
    used_albums: &std::collections::HashSet<String>,
) -> Vec<crate::plex::models::Track> {
    let distances = [0.25f32, 0.5, 0.75];
    for &distance in &distances {
        let result = client.get_similar_tracks_with_distance(
            &current_track.rating_key, SIMILAR_TRACKS_FETCH_LIMIT, distance
        ).await;
        match result {
            Ok(tracks) => {
                let picked = pick_diverse(tracks, DJ_INSERT_COUNT, history, used_artists, used_albums);
                if !picked.is_empty() {
                    if distance > 0.25 {
                        tracing::info!("DJ Freeze: found tracks at maxDistance={}", distance);
                    }
                    return picked;
                }
                tracing::debug!("DJ Freeze: no diverse tracks at maxDistance={}, widening", distance);
            }
            Err(e) => {
                tracing::warn!("DJ Freeze: similar tracks failed at maxDistance={}: {}", distance, e);
                return vec![];
            }
        }
    }
    tracing::warn!("DJ Freeze: no tracks found at any distance");
    vec![]
}

/// DJ Contempo: tracks from the same era/decade.
async fn dispatch_dj_contempo(
    client: &PlexClient,
    current_track: &crate::plex::models::Track,
    history: &[String],
    used_artists: &std::collections::HashSet<String>,
    used_albums: &std::collections::HashSet<String>,
    lib_key: Option<&str>,
) -> Vec<crate::plex::models::Track> {
    let year = current_track.year.or(current_track.parent_year).unwrap_or(0);
    let decade = (year / 10) * 10;
    if decade == 0 {
        return vec![];
    }
    let Some(lk) = lib_key else {
        return vec![];
    };
    match client.create_album_filter_radio_tracks(lk, "decade", &decade.to_string()).await {
        Ok(tracks) => pick_diverse(tracks, DJ_INSERT_COUNT, history, used_artists, used_albums),
        Err(e) => {
            tracing::warn!("DJ Contempo: decade tracks failed: {}", e);
            vec![]
        }
    }
}

/// DJ Groupie: tracks from current artist AND related artists.
/// Falls back to same-artist-only if related artists endpoint fails.
async fn dispatch_dj_groupie(
    client: &PlexClient,
    current_track: &crate::plex::models::Track,
    history: &[String],
    used_albums: &std::collections::HashSet<String>,
) -> Vec<crate::plex::models::Track> {
    let artist_key = current_track.grandparent_rating_key.as_deref().unwrap_or("");
    if artist_key.is_empty() {
        return vec![];
    }

    // Build cluster of current artist + related artist keys
    let mut cluster_keys = vec![artist_key.to_string()];

    // Try to get related artists
    match client.get_related_artists(artist_key).await {
        Ok(related) => {
            for artist in related.iter().take(5) {
                cluster_keys.push(artist.rating_key.clone());
            }
            tracing::debug!("DJ Groupie: cluster of {} artists (1 current + {} related)",
                cluster_keys.len(), related.len().min(5));
        }
        Err(e) => {
            tracing::debug!("DJ Groupie: related artists failed ({}), using same artist only", e);
        }
    }

    // Fetch tracks from the cluster (sample from each artist)
    let mut all_candidates = Vec::new();
    for key in &cluster_keys {
        match client.get_artist_all_tracks(key).await {
            Ok(tracks) => {
                let filtered: Vec<_> = tracks.into_iter()
                    .filter(|t| {
                        t.rating_key != current_track.rating_key
                            && !history.contains(&t.rating_key)
                    })
                    .collect();
                all_candidates.extend(filtered);
            }
            Err(e) => {
                tracing::warn!("DJ Groupie: tracks for artist {} failed: {}", key, e);
            }
        }
    }

    use rand::seq::SliceRandom;
    let mut rng = rand::rng();
    all_candidates.shuffle(&mut rng);

    // Groupie stays with the artist cluster, so only enforce album diversity
    pick_diverse(all_candidates, DJ_INSERT_COUNT, history, &std::collections::HashSet::new(), used_albums)
}
