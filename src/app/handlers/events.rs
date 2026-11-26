//! Event result handlers.
//!
//! Processes async event results (auth, data loading, playback, cache, etc.)

use super::helpers;
use crate::app::action::*;
use crate::app::event::*;
use crate::app::state::{BrowseItem, PlayStatus, View};
use crate::app::{Action, AppState, Event};

use std::time::Duration;
use tokio::sync::mpsc;

/// Remove every object whose identity is scoped to a server account/server.
/// Large collections are dropped on a blocking worker so an account switch
/// cannot freeze either the TUI reducer or the async runtime worker running it.
pub(crate) fn reset_account_scoped_state(state: &mut AppState) {
    let library_sub_mode = state.library.library_sub_mode;
    let artwork_generation = state.artwork.grid_generation.wrapping_add(1);
    let old_library = std::mem::take(&mut state.library);
    state.library.library_sub_mode = library_sub_mode;

    let old_folder_state = std::mem::take(&mut state.folder_state);
    let old_artist_nav = std::mem::replace(
        &mut state.artist_nav,
        crate::app::state::BrowseNavigationState::new(),
    );
    let old_tag_nav = std::mem::replace(
        &mut state.tag_nav,
        crate::app::state::BrowseNavigationState::new(),
    );
    let old_playlist_nav = std::mem::replace(
        &mut state.playlist_nav,
        crate::app::state::BrowseNavigationState::new(),
    );
    let old_station_nav = std::mem::take(&mut state.station_nav);
    let old_station_children = std::mem::take(&mut state.station_children_cache);
    let old_similar = std::mem::take(&mut state.similar);
    let old_related = std::mem::take(&mut state.related);
    let old_search = std::mem::take(&mut state.search);
    let old_track_pane_similar = std::mem::take(&mut state.track_pane_similar);
    let old_track_pane_loading = std::mem::take(&mut state.track_pane_similar_loading);
    let old_image_loaded = std::mem::take(&mut state.image_loaded);
    let old_waveform = std::mem::take(&mut state.waveform);
    let old_spectrogram = std::mem::take(&mut state.spectrogram);

    let artwork_mode = state.artwork.mode;
    let old_artwork = std::mem::take(&mut state.artwork);
    state.artwork.mode = artwork_mode;
    state.artwork.grid_generation = artwork_generation;

    state.active_library = None;
    state.connected_server_url = None;

    state.station_nav.loading = false;
    state.library_loading = false;
    state.cache_mgmt = crate::app::state::CacheManagement::default();

    state.list_filter.deactivate();
    state.list_state.reset();
    state.scroll = crate::app::state::ScrollPins::default();
    state.popups.close_all();
    state.sonic_details = None;
    state.track_pane_open = false;
    state.track_pane_focused = false;
    state.track_pane_index = 0;
    state.select_mode = false;
    state.vectorscope_buffer.clear();
    state.studio_meters = Default::default();

    crate::app::tasks::spawn_blocking(move || {
        drop(old_library);
        drop(old_folder_state);
        drop(old_artist_nav);
        drop(old_tag_nav);
        drop(old_playlist_nav);
        drop(old_station_nav);
        drop(old_station_children);
        drop(old_similar);
        drop(old_related);
        drop(old_search);
        drop(old_track_pane_similar);
        drop(old_track_pane_loading);
        drop(old_image_loaded);
        drop(old_waveform);
        drop(old_spectrogram);
        drop(old_artwork);
    });
}

/// Handle non-input events (async results, timers, etc.) and return actions to dispatch.
pub fn handle_app_event(
    event: Event,
    state: &mut AppState,

    event_tx: &mpsc::Sender<Event>,
) -> Vec<Action> {
    let event = match event {
        Event::Effect(action) => return vec![action],
        event => event,
    };

    match event {
        Event::Data(DataEvent::ScopedLoadError {
            request_key,
            message,
        }) => {
            if state.related.source_key == request_key {
                state.related.loading = false;
                state.set_error(message);
            } else {
                tracing::debug!("Ignoring stale scoped load error: {}", request_key);
            }
            vec![]
        }

        Event::Data(DataEvent::SimilarLoadFailed {
            request_key,
            message,
        }) => {
            if state.similar.request.loading_key() == Some(&request_key) {
                state.similar.request = crate::app::state::SimilarRequest::Failed { message };
            }
            vec![]
        }
        Event::Data(DataEvent::SimilarAlbumsLoaded {
            request_key,
            albums,
        }) => {
            if state.similar.request.loading_key() != Some(&request_key) {
                tracing::debug!("Ignoring stale similar-albums completion: {}", request_key);
                return vec![];
            }
            state.similar.request = crate::app::state::SimilarRequest::Ready;
            state.similar.albums = albums;
            state.similar.mode = crate::app::state::SimilarMode::Albums;
            state.list_state.similar_index = 0;
            vec![]
        }
        Event::Data(DataEvent::SimilarTracksLoaded {
            request_key,
            tracks,
        }) => {
            if state.similar.request.loading_key() != Some(&request_key) {
                tracing::debug!("Ignoring stale similar-tracks completion: {}", request_key);
                return vec![];
            }
            state.similar.request = crate::app::state::SimilarRequest::Ready;
            state.similar.tracks = tracks;
            state.similar.mode = crate::app::state::SimilarMode::Tracks;
            state.list_state.similar_index = 0;
            vec![]
        }
        Event::Data(DataEvent::TrackPaneSimilarLoaded {
            server_url,
            rating_key,
            result,
        }) => {
            if state.connected_server_url.as_deref() != server_url.as_deref() {
                return vec![];
            }
            state.track_pane_similar_loading.remove(&rating_key);
            match result {
                Ok(tracks) => {
                    state.track_pane_similar.insert(rating_key, Ok(tracks));
                }
                Err(error) => {
                    tracing::debug!("{}", error.message);
                    state
                        .track_pane_similar
                        .insert(rating_key, Err(error.message));
                }
            }
            vec![]
        }
        Event::Data(DataEvent::SimilarArtistsLoaded {
            request_key,
            artists,
        }) => {
            if state.similar.request.loading_key() != Some(&request_key) {
                tracing::debug!("Ignoring stale similar-artists completion: {}", request_key);
                return vec![];
            }
            state.similar.request = crate::app::state::SimilarRequest::Ready;
            state.similar.artists = artists;
            state.similar.mode = crate::app::state::SimilarMode::Artists;
            state.list_state.similar_index = 0;
            vec![]
        }
        Event::Data(DataEvent::RelatedDataLoaded {
            request_key,
            groups,
        }) => {
            if state.related.source_key != request_key {
                tracing::debug!("Ignoring stale related-data completion: {}", request_key);
                return vec![];
            }
            state.related.groups = groups;
            state.related.loading = false;
            state.list_state.related_index = 0;
            state.scroll.related = None;
            vec![]
        }

        Event::Playback(PlaybackEvent::TrackEnded) => {
            // Ignore stale TrackEnded events. The tick loop only sends TrackEnded
            // when status is Playing. If status has since changed (e.g., a new station
            // started and set Buffering/Stopped), this event is from the old track.
            if state.playback.status != PlayStatus::Playing {
                return vec![];
            }

            // Additional guard: ignore if track was just started. The tick loop has
            // a 1-second grace period before sending TrackEnded, so any event arriving
            // within 1 second of playback start is stale (from a previous track).
            let playing_long_enough = state
                .playback
                .playback_started_at
                .map(|t| t.elapsed() >= Duration::from_secs(1))
                .unwrap_or(false);
            if !playing_long_enough {
                tracing::debug!("Ignoring stale TrackEnded (track just started)");
                return vec![];
            }

            // Immediately set Stopped to prevent any duplicate TrackEnded events
            // (still in the channel) from triggering another Next action.
            state.playback.status = PlayStatus::Stopped;

            // Report stop to server when track ends naturally
            // continuing=true because we're about to play the next track
            if let Some(track) = state.current_track().cloned() {
                if state.playback.scrobble == crate::app::state::ScrobbleState::Pending {
                    state.playback.scrobble = crate::app::state::ScrobbleState::Queued;
                    crate::app::sources::navidrome::scrobble(state, event_tx, &track);
                }
            }
            if state
                .station_starting
                .as_ref()
                .is_some_and(|start| start.continue_playback == Some(state.playback.request_id))
            {
                // Natural completion waits for Sonic Mix, rather than playing
                // the old queue's next song. Explicit transport input remains free
                // to cancel or supersede the pending station.
                state.set_status("Waiting for Sonic Mix…".into());
                vec![]
            } else if state.queue.repeat == crate::app::state::RepeatMode::Track {
                vec![PlaybackAction::RetryCurrentTrack.into()]
            } else {
                vec![PlaybackAction::Next.into()]
            }
        }

        Event::Playback(PlaybackEvent::SeekFailed {
            playback_id,
            message,
        }) => {
            if playback_id == state.playback.request_id {
                tracing::warn!("{message}");
                state.set_status(message);
            }
            vec![]
        }
        Event::Playback(PlaybackEvent::PlaybackError {
            playback_id,
            message: msg,
        }) => {
            if playback_id.is_some_and(|id| id != state.playback.request_id) {
                tracing::debug!(
                    "Ignoring stale playback error for request {:?}; current request is {}",
                    playback_id,
                    state.playback.request_id
                );
                return vec![];
            }
            state.playback.status = PlayStatus::Stopped;
            state.consecutive_playback_errors += 1;

            let track_info = state
                .current_track()
                .map(|t| format!("{} - {}", t.artist_name(), t.title))
                .unwrap_or_else(|| "unknown".to_string());
            let qi = state.queue.index.unwrap_or(9999);

            // First 5 errors: retry the SAME track with increasing delays
            // (handles cold-start / remote relay warm-up)
            if state.consecutive_playback_errors <= 5 {
                let delays_ms = [500, 1000, 1500, 2000, 2500];
                let delay = delays_ms
                    [(state.consecutive_playback_errors as usize - 1).min(delays_ms.len() - 1)];
                let playback_id = state.playback.request_id;
                tracing::warn!(
                    "Playback error (retry {}/5 for queue[{}] '{}', delay {}ms): {}",
                    state.consecutive_playback_errors,
                    qi,
                    track_info,
                    delay,
                    msg
                );
                let tx = event_tx.clone();
                crate::app::tasks::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    let _ = tx
                        .send(PlaybackEvent::RetryAfterDelay { playback_id }.into())
                        .await;
                });
                return vec![];
            }

            // Errors 6-8: auto-skip to next track
            if state.consecutive_playback_errors <= 8 {
                tracing::warn!(
                    "Playback error (skipping queue[{}] '{}', attempt {}/8): {}",
                    qi,
                    track_info,
                    state.consecutive_playback_errors,
                    msg
                );
                return vec![PlaybackAction::Next.into()];
            }

            // After 8 consecutive failures, show error to user
            state.consecutive_playback_errors = 0;
            if msg.contains("404") || msg.to_lowercase().contains("not found") {
                state.popups.close_all();
                state
                    .popups
                    .set_confirm_dialog(Some(crate::app::state::ConfirmDialog {
                        title: "Track Not Found".to_string(),
                        message: "This track may have been removed. Refresh cache?".to_string(),
                        on_confirm: crate::app::state::ConfirmAction::RefreshCache,
                        selected_yes: true,
                    }));
            } else {
                state.set_error("Playback stopped after multiple consecutive errors".to_string());
            }
            vec![]
        }
        Event::Playback(PlaybackEvent::RetryAfterDelay { playback_id }) => {
            if playback_id == state.playback.request_id
                && state.playback.status == PlayStatus::Stopped
            {
                vec![PlaybackAction::RetryCurrentTrack.into()]
            } else {
                tracing::debug!("Ignoring stale delayed playback retry");
                vec![]
            }
        }

        Event::Playback(PlaybackEvent::BufferingEnd { playback_id }) => {
            if playback_id != state.playback.request_id
                || state.playback.status != PlayStatus::Buffering
            {
                tracing::debug!("Ignoring stale buffering completion");
                return vec![];
            }
            // The decoder actor emits this only after it has decoded a PCM
            // runway and attached the source to the device mixer.
            state.playback.status = PlayStatus::Playing;
            state.playback.playback_started_at = Some(std::time::Instant::now());
            vec![]
        }

        Event::Artwork(ArtworkEvent::ArtworkLoaded {
            generation,
            thumb_path,
            data,
        }) => {
            if generation != state.artwork.grid_generation
                || state.artwork.fetch.pending_thumb() != Some(&thumb_path)
            {
                tracing::debug!("Ignoring stale artwork completion: {}", thumb_path);
                return vec![];
            }
            state.artwork.current_thumb = Some(thumb_path);
            state.artwork.current_data = Some(data);
            state.artwork.fetch = crate::app::state::ArtworkFetch::Idle;
            vec![]
        }
        Event::Artwork(ArtworkEvent::ArtworkFailed {
            generation,
            thumb_path,
        }) => {
            if generation != state.artwork.grid_generation
                || state.artwork.fetch.pending_thumb() != Some(&thumb_path)
            {
                tracing::debug!("Ignoring stale artwork failure: {}", thumb_path);
                return vec![];
            }
            state.artwork.current_thumb = None;
            state.artwork.current_data = None;
            state.artwork.fetch = crate::app::state::ArtworkFetch::Idle;
            vec![]
        }
        Event::Artwork(ArtworkEvent::AlbumArtLoaded {
            generation,
            key,
            data,
        }) => {
            if generation != state.artwork.grid_generation {
                tracing::debug!("Ignoring stale grid artwork completion for {}", key);
                return vec![];
            }
            state.artwork.grid_pending.remove(&key);
            state.artwork.insert_grid_art(key, data);
            vec![]
        }
        Event::Artwork(ArtworkEvent::AlbumArtFailed { generation, key }) => {
            if generation != state.artwork.grid_generation {
                return vec![];
            }
            state.artwork.grid_pending.remove(&key);
            vec![]
        }

        Event::Tick => {
            // Clear expired modifier bars
            if let Some(deadline) = state.alt_bar_until
                && std::time::Instant::now() >= deadline
            {
                state.alt_bar_until = None;
            }

            // Clear expired toasts (5 second display)
            if let Some(show_time) = state.notifications.toast_show_time
                && show_time.elapsed() > Duration::from_secs(5)
            {
                state.notifications.toast_message = None;
                state.notifications.toast_show_time = None;
            }

            // Clear expired status messages (5 second display)
            if let Some(show_time) = state.notifications.status_show_time
                && show_time.elapsed() > Duration::from_secs(5)
            {
                state.clear_status();
            }

            // Periodic playback progress report to server (~10 seconds)
            if state.playback.status == PlayStatus::Playing {
                if !state.play_recorded
                    && state.playback.position_ms > 0
                    && let Some(play) = state
                        .current_track()
                        .map(crate::app::listening_history::Play::from_track)
                {
                    state.play_recorded = true;
                    match crate::app::listening_history::save(Some(play)) {
                        Ok(rows) => state.listening_history = rows,
                        Err(e) => state.set_error(format!("Could not save listening history: {e}")),
                    }
                }
                let reached_scrobble_threshold = state.playback.duration_ms > 0
                    && state.playback.position_ms.saturating_mul(10)
                        >= state.playback.duration_ms.saturating_mul(9);
                if reached_scrobble_threshold
                    && state.playback.scrobble == crate::app::state::ScrobbleState::Pending
                    && let Some(track) = state.current_track().cloned()
                {
                    state.playback.scrobble = crate::app::state::ScrobbleState::Queued;
                    crate::app::sources::navidrome::scrobble(state, event_tx, &track);
                }
            }

            // Marquee scroll animation (title + subtitle)
            state.marquee.tick();
            state.marquee_subtitle.tick();

            // Per-tick counter for animated "Loading..." text in
            // miller column placeholders. Wraps; consumers do `% 4`.
            state.loading_tick = state.loading_tick.wrapping_add(1);

            // Drain the audio backend's sample tap into the
            // vectorscope buffer. Done unconditionally (cheap) so
            // the visualizer is "warm" the moment the user opens
            // its tab. Buffer is capped at VECTORSCOPE_BUFFER_LEN
            // — older samples roll off the front of the deque.
            if let Some(tap) = state.vectorscope_tap.clone() {
                let mut samples = Vec::with_capacity(4_096);
                tap.drain_into(&mut samples, 4_096);
                if state.playback.status == PlayStatus::Playing {
                    state.studio_meters.update(
                        &samples,
                        std::time::Instant::now(),
                        state.playback.request_id,
                    );
                } else if state.playback.status != PlayStatus::Paused {
                    state.studio_meters = Default::default();
                }
                for sample in samples {
                    if state.vectorscope_buffer.len() >= crate::app::state::VECTORSCOPE_BUFFER_LEN {
                        state.vectorscope_buffer.pop_front();
                    }
                    state.vectorscope_buffer.push_back(sample);
                }
            }

            // Lazy-art settle: if navigation entered Moving after a
            // recent rapid-nav gesture and the user has been still for
            // `ART_LOAD_PAUSE_MS`, clear the gate and dispatch one
            // viewport-wide `LoadAlbumArt` batch.
            if let Some(actions) = super::lazy_art::settle(state)
                && !actions.is_empty()
            {
                return actions;
            }

            if let Some((track, _)) = crate::app::details::similar_seed(state) {
                let key = track.rating_key;
                if crate::app::sources::sonic::enabled(state)
                    && !key.is_empty()
                    && !state.track_pane_similar.contains_key(&key)
                    && !state.track_pane_similar_loading.contains(&key)
                {
                    return vec![
                        crate::app::action::DataAction::LoadTrackPaneSimilar { rating_key: key }
                            .into(),
                    ];
                }
            }

            // Album art loading: lazy-load for visible items across
            // EVERY column with `artwork_visible`, not just the
            // focused one. The user expects album artwork to start
            // populating the moment they pause on an artist row —
            // before they've drilled into the album column itself —
            // so the artist's albums column (focused +1) and any
            // already-open art columns to its right all get a turn.
            // Cap concurrent in-flight requests to avoid overwhelming
            // the server transcoder.
            if (state.view == crate::app::state::View::Browse
                || (state.view == crate::app::state::View::Queue
                    && state.tui_queue_workspace == crate::app::state::TuiWorkspace::Library))
                && state.artwork.grid_pending.len() < 4
            {
                let nav = match state.browse_category {
                    crate::app::state::BrowseCategory::Library => &state.artist_nav,
                    crate::app::state::BrowseCategory::Playlists => &state.playlist_nav,
                    cat if cat.is_tag_section() => &state.tag_nav,
                    _ => &state.artist_nav,
                };
                let max_batch = 4usize.saturating_sub(state.artwork.grid_pending.len());
                let mut to_load: Vec<(String, String)> = Vec::new();
                if let Some(track) =
                    crate::app::sonic_details::selected_track(state).or_else(|| state.pane_track())
                    && let Some(cover) = crate::app::details::track_cover(state, track)
                    && !state.artwork.grid_cache.contains_key(&track.rating_key)
                    && !state.artwork.grid_pending.contains(&track.rating_key)
                {
                    to_load.push((track.rating_key.clone(), cover.to_owned()));
                }
                // Iterate art-visible columns starting from the
                // focused one and walking outward, so the column the
                // user is staring at gets first claim on the budget.
                let focused_idx = nav.focused_column;
                let mut col_order: Vec<usize> = Vec::with_capacity(nav.columns.len());
                col_order.push(focused_idx);
                for d in 1..=nav.columns.len() {
                    if focused_idx + d < nav.columns.len() {
                        col_order.push(focused_idx + d);
                    }
                    if focused_idx >= d {
                        col_order.push(focused_idx - d);
                    }
                }
                'cols: for ci in col_order {
                    if to_load.len() >= max_batch {
                        break;
                    }
                    let col = match nav.columns.get(ci) {
                        Some(c) => c,
                        None => continue,
                    };
                    if let Some(cover) = crate::app::details::album_cover(state, col)
                        && !state.artwork.grid_cache.contains_key(&cover.0)
                        && !state.artwork.grid_pending.contains(&cover.0)
                    {
                        to_load.push(cover);
                        if to_load.len() >= max_batch {
                            break;
                        }
                    }
                    let album_key = match &col.play_all_row {
                        Some(crate::app::state::PlayAllRow::Album { rating_key, .. }) => {
                            Some(rating_key)
                        }
                        _ => None,
                    };
                    let albums = album_key
                        .and_then(|_| col.tracks.first())
                        .into_iter()
                        .flat_map(|seed| {
                            let tracks = state
                                .track_pane_similar
                                .get(&seed.rating_key)
                                .and_then(|r| r.as_ref().ok())
                                .map(Vec::as_slice)
                                .unwrap_or_default();
                            crate::app::details::recommendation_tracks(seed, tracks, true)
                        })
                        .filter_map(|t| {
                            let key = t.parent_rating_key.as_ref()?;
                            if state.artwork.grid_cache.contains_key(key)
                                || state.artwork.grid_pending.contains(key)
                            {
                                return None;
                            }
                            let thumb = t
                                .parent_thumb
                                .as_ref()
                                .or_else(|| {
                                    state
                                        .library
                                        .albums
                                        .iter()
                                        .find(|a| a.rating_key == *key)
                                        .and_then(|a| a.thumb.as_ref())
                                })
                                .or(t.thumb.as_ref())?;
                            Some((key, thumb))
                        });
                    let related = col
                        .artist_header
                        .as_ref()
                        .map(|info| info.document.related_artists.as_slice())
                        .or_else(|| {
                            let rating_key = album_key?;
                            state
                                .sources
                                .album_artists
                                .as_ref()
                                .filter(|r| {
                                    r.generation == state.library_generation
                                        && r.album == *rating_key
                                })?
                                .document
                                .as_ref()
                                .map(|doc| doc.related_artists.as_slice())
                        });
                    let artists = related.into_iter().flatten().filter_map(|a| {
                        let thumb = a.thumb.as_ref().or_else(|| {
                            state
                                .library
                                .artists
                                .iter()
                                .find(|known| known.rating_key == a.rating_key)?
                                .thumb
                                .as_ref()
                        })?;
                        Some((&a.rating_key, thumb))
                    });
                    for (key, thumb) in albums.chain(artists) {
                        if !state.artwork.grid_cache.contains_key(key)
                            && !state.artwork.grid_pending.contains(key)
                            && !to_load.iter().any(|(queued, _)| queued == key)
                        {
                            to_load.push((key.clone(), thumb.clone()));
                            if to_load.len() >= max_batch {
                                break 'cols;
                            }
                        }
                    }
                    if !col.artwork_visible && !crate::app::view_options::album_grid(state, ci) {
                        continue;
                    }
                    let total_items = col.items.len();
                    if total_items == 0 {
                        continue;
                    }
                    let rendered = state
                        .hit_regions
                        .miller_item_rows
                        .iter()
                        .filter(|(column, _, _)| *column == ci)
                        .filter_map(|(_, _, row)| col.items.get(*row));
                    // Desktop does not produce terminal viewport feedback.
                    let rows = if state.desktop_frontend {
                        let count = 5.min(total_items);
                        let start = crate::services::NavigationService::calc_scroll_offset(
                            col.selected_index,
                            count,
                            total_items,
                        );
                        &col.items[start..(start + count).min(total_items)]
                    } else {
                        &[]
                    };
                    for item in rendered.chain(rows.iter()) {
                        if to_load.len() >= max_batch {
                            break 'cols;
                        }
                        match item {
                            BrowseItem::Album {
                                key,
                                thumb: Some(thumb),
                                ..
                            } if !state.artwork.grid_cache.contains_key(key)
                                && !state.artwork.grid_pending.contains(key) =>
                            {
                                to_load.push((key.clone(), thumb.clone()));
                            }
                            BrowseItem::AllTracks {
                                scope,
                                thumb: Some(thumb),
                            } => {
                                if let Some(artist_key) = scope.artist_key()
                                    && !state.artwork.grid_cache.contains_key(artist_key)
                                    && !state.artwork.grid_pending.contains(artist_key)
                                {
                                    to_load.push((artist_key.to_string(), thumb.clone()));
                                }
                            }
                            BrowseItem::Artist {
                                key,
                                thumb: Some(thumb),
                                ..
                            } if !state.artwork.grid_cache.contains_key(key)
                                && !state.artwork.grid_pending.contains(key) =>
                            {
                                to_load.push((key.clone(), thumb.clone()));
                            }
                            _ => {}
                        }
                    }
                }
                if !to_load.is_empty() {
                    return vec![SystemAction::LoadAlbumArt(to_load).into()];
                }
            }

            // Visualizer data safety net: ensure waveform/spectrogram are generated
            if ((state.playback_waveform_available && state.view == View::NowPlaying)
                || (state.visualizers_available
                    && (matches!(state.view, View::NowPlaying | View::Queue)
                        || state.desktop_player_visible
                        || (state.fullscreen.is_open() && !state.fullscreen.is_artwork()))))
                && let Some(track) = state.current_track().cloned()
            {
                let tk = &track.rating_key;

                // Ensure track_key is set (handles track change while on this view)
                if state.waveform.track_key.as_ref() != Some(tk) {
                    state.waveform = crate::app::state::WaveformState::default();
                    state.waveform.track_key = Some(tk.clone());
                    state.spectrogram = crate::app::state::SpectrogramState::default();
                    state.spectrogram.track_key = Some(tk.clone());
                }

                // Request the waveform needed by the active playback surface.
                if state.waveform.data().is_none()
                    && !state.waveform.generating()
                    && state.waveform.error().is_none()
                {
                    return vec![SystemAction::LoadWaveform.into()];
                }

                // Trigger spectrogram independently if waveform is done but spectrogram isn't
                if state.visualizers_available
                    && state.spectrogram.data().is_none()
                    && !state.spectrogram.generating()
                    && state.spectrogram.error().is_none()
                    && state.waveform.data().is_some()
                {
                    return vec![SystemAction::LoadSpectrogram.into()];
                }
            }

            // Periodic cache save: save if dirty, idle for 30+ seconds, and 2+ minutes since last save

            helpers::refresh_due_caches(state, event_tx);

            vec![]
        }

        Event::Mouse(mouse_event) => super::mouse_input::handle_mouse(mouse_event, state),
        Event::Visualizer(VisualizerEvent::AnalysisProgress {
            track_key,
            progress,
        }) => {
            if state.waveform.track_key.as_ref() == Some(&track_key) && state.waveform.generating()
            {
                state.waveform.report(progress.clone());
            }
            if state.spectrogram.track_key.as_ref() == Some(&track_key)
                && state.spectrogram.generating()
            {
                state.spectrogram.report(progress);
            }
            vec![]
        }
        Event::Visualizer(VisualizerEvent::WaveformGenerated { track_key, data }) => {
            if state.waveform.track_key.as_ref() == Some(&track_key) {
                state.waveform.succeed(data);
                tracing::debug!("Waveform generated for track: {}", track_key);
            }
            vec![]
        }
        Event::Visualizer(VisualizerEvent::WaveformFailed { track_key, error }) => {
            if state.waveform.track_key.as_ref() == Some(&track_key)
                && state.waveform.data().is_none()
            {
                if state.waveform.retry_count < 3 {
                    state.waveform.retry_count += 1;
                    let retry_num = state.waveform.retry_count;
                    let tx = event_tx.clone();
                    let tk = track_key.clone();
                    let library_generation = state.library_generation;
                    let retry = crate::app::tasks::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(2 * retry_num as u64)).await;
                        let _ = tx
                            .send(Event::for_library(
                                library_generation,
                                VisualizerEvent::WaveformRetry(tk),
                            ))
                            .await;
                    });
                    state.waveform.phase = crate::app::state::AnalysisPhase::WaitingToRetry;
                    state.waveform.task = Some(crate::app::tasks::TaskLease::new(&retry));
                    tracing::info!(
                        "Waveform retry {}/3 for {}: {}",
                        retry_num,
                        track_key,
                        error
                    );
                } else {
                    state.waveform.fail(error.clone());
                    tracing::warn!(
                        "Waveform failed after 3 retries for {}: {}",
                        track_key,
                        error
                    );
                }
            }
            vec![]
        }
        Event::Visualizer(VisualizerEvent::WaveformCacheHit { track_key, data }) => {
            if state.waveform.track_key.as_ref() == Some(&track_key) {
                state.waveform.succeed(data);
                tracing::debug!("Waveform loaded from cache for track: {}", track_key);
            }
            vec![]
        }
        Event::Visualizer(VisualizerEvent::WaveformRetry(track_key)) => {
            if state.waveform.track_key.as_ref() == Some(&track_key)
                && matches!(
                    state.waveform.phase,
                    crate::app::state::AnalysisPhase::WaitingToRetry
                )
            {
                state.waveform.phase = crate::app::state::AnalysisPhase::Idle;
                vec![SystemAction::LoadWaveform.into()]
            } else {
                vec![]
            }
        }
        Event::Visualizer(VisualizerEvent::SpectrogramGenerated { track_key, data }) => {
            if state.spectrogram.track_key.as_ref() == Some(&track_key) {
                state.spectrogram.succeed(data);
                tracing::debug!("Spectrogram generated for track: {}", track_key);
            }
            vec![]
        }
        Event::Visualizer(VisualizerEvent::SpectrogramCacheMiss { track_key }) => {
            if state.spectrogram.track_key.as_ref() == Some(&track_key)
                && state.spectrogram.data().is_none()
            {
                state.spectrogram.phase = crate::app::state::AnalysisPhase::Idle;
            }
            vec![]
        }
        Event::Visualizer(VisualizerEvent::SpectrogramFailed { track_key, error }) => {
            if state.spectrogram.track_key.as_ref() == Some(&track_key)
                && state.spectrogram.data().is_none()
            {
                state.spectrogram.phase = crate::app::state::AnalysisPhase::Idle;
                state.spectrogram.fail(error.clone());
                tracing::warn!("Spectrogram failed for {}: {}", track_key, error);
            }
            vec![]
        }
        Event::Visualizer(VisualizerEvent::SpectrogramCacheHit { track_key, data }) => {
            if state.spectrogram.track_key.as_ref() == Some(&track_key) {
                state.spectrogram.succeed(data);
                tracing::debug!("Spectrogram loaded from cache for track: {}", track_key);
            }
            vec![]
        }
        Event::Radio(event) => super::events_radio::handle(event, state),
        Event::Playlist(event) => super::events_playlist::handle(event, state),

        Event::Ui(UiEvent::ArtistInfoLoaded {
            generation,
            request_id,
            artist_key,
            result,
        }) => {
            if generation != state.library_generation {
                return vec![];
            }
            let mut result = Some(result);
            let mut picture = None;
            'owners: for nav in [
                &mut state.artist_nav,
                &mut state.playlist_nav,
                &mut state.tag_nav,
            ] {
                for column in &mut nav.columns {
                    let standalone = matches!(
                        column.details,
                        Some(crate::app::state::BrowseDetails::Artist(_))
                    );
                    let owner = match &mut column.details {
                        Some(crate::app::state::BrowseDetails::Artist(info)) => Some(info),
                        _ => column.artist_header.as_mut(),
                    };
                    if let Some(info) = owner {
                        if info.request_id != request_id || info.artist_key != artist_key {
                            continue;
                        }
                        info.task = None;
                        info.document = result.take().unwrap().unwrap_or_else(|error| {
                            crate::services::biography::Biography {
                                unavailable: true,
                                text: error,
                                ..Default::default()
                            }
                        });
                        crate::app::sources::navidrome::album_artists::filter_recommendations(
                            &state.library,
                            &mut info.document,
                        );
                        state.artist_bios.insert(&info.artist_key, &info.document);
                        picture = info
                            .document
                            .images
                            .get(info.image_index)
                            .map(|image| (info.artist_key.clone(), image.data.clone()));
                        if standalone {
                            column.selected_index = 0;
                        }
                        break 'owners;
                    }
                }
            }
            if let Some((key, data)) = picture
                && !state.artwork.grid_cache.contains_key(&key)
            {
                state.artwork.insert_grid_art(key, data);
            }
            vec![]
        }
        Event::Ui(UiEvent::ArtistBioLoaded {
            generation,
            request_id,
            result,
        }) => {
            if generation != state.library_generation || state.artist_bio_request_id != request_id {
                return vec![];
            }
            if let Some(popup) = state.popups.artist_bio_mut() {
                popup.task = None;
                popup.document =
                    result.unwrap_or_else(|error| crate::services::biography::Biography {
                        unavailable: true,
                        text: error,
                        ..Default::default()
                    });
                crate::app::sources::navidrome::album_artists::filter_recommendations(
                    &state.library,
                    &mut popup.document,
                );
                state.artist_bios.insert(&popup.artist_key, &popup.document);
                popup.scroll = 0;
            }
            vec![]
        }

        // Inline list filter completed
        Event::Ui(UiEvent::ListFilterCompleted {
            version,
            generation,
            results,
        }) => {
            if state.list_filter.active
                && version == state.list_filter.version
                && generation == state.library_generation
            {
                state.list_filter.loading = false;
                state.list_filter.selected = state
                    .list_filter
                    .refresh_anchor
                    .take()
                    .and_then(|anchor| {
                        results
                            .matched_indices
                            .iter()
                            .position(|index| *index == anchor)
                    })
                    .unwrap_or(0);
                state.list_filter.results = Some(results);
                // Only update column selection if user is still on the filter column.
                // If they've drilled deeper (e.g., into subfolders), preserve their
                // current navigation — changing the selection would jump them away.
                if super::dispatch_search::is_on_filter_column(state)
                    && let Some(ref results) = state.list_filter.results
                    && let Some(&first_idx) =
                        results.matched_indices.get(state.list_filter.selected)
                {
                    super::key_input::update_filter_column_selection(state, first_idx);
                }
            }
            vec![]
        }
        // Remote player control events
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn account_reset_preserves_request_generations_and_drops_scoped_data() {
        let mut state = AppState::new();
        state.active_library = Some("old-library".to_string());
        state.library.artists_total = 99;
        state.search.query = "old account query".to_string();
        state.artwork.current_data = Some(vec![1, 2, 3]);
        state.artwork.grid_generation = 7;

        state.advance_library_generation();
        let library_generation = state.library_generation;
        reset_account_scoped_state(&mut state);

        assert_eq!(state.library_generation, library_generation);
        assert_eq!(state.artwork.grid_generation, 8);
        assert!(state.active_library.is_none());
        assert_eq!(state.library.artists_total, 0);
        assert!(state.search.query.is_empty());
        assert!(state.artwork.current_data.is_none());

        // Let the deferred destruction task run before this test runtime exits.
        tokio::task::yield_now().await;
    }
}
