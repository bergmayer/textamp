//! Event result handlers.
//!
//! Processes async event results (auth, data loading, playback, cache, etc.)

use super::helpers;
use crate::app::action::*;
use crate::app::event::*;
use crate::app::state::{BrowseCategory, BrowseItem, PlayStatus, View};
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
    let old_stations = std::mem::take(&mut state.stations);
    let old_station_children = std::mem::take(&mut state.station_children_cache);
    let old_similar = std::mem::take(&mut state.similar);
    let old_related = std::mem::take(&mut state.related);
    let old_search = std::mem::take(&mut state.search);
    let old_adventure = std::mem::take(&mut state.adventure);
    let old_track_pane_similar = std::mem::take(&mut state.track_pane_similar);
    let old_track_pane_loading = std::mem::take(&mut state.track_pane_similar_loading);
    let old_image_loaded = std::mem::take(&mut state.image_loaded);
    let old_waveform = std::mem::take(&mut state.waveform);
    let old_spectrogram = std::mem::take(&mut state.spectrogram);

    let artwork_visible = state.artwork.default_visible;
    let artwork_mode = state.artwork.mode;
    let old_artwork = std::mem::take(&mut state.artwork);
    state.artwork.default_visible = artwork_visible;
    state.artwork.mode = artwork_mode;
    state.artwork.grid_generation = artwork_generation;

    state.active_library = None;
    state.connected_server_url = None;

    state.stations_loading = false;
    state.library_loading = false;
    state.cache_mgmt = crate::app::state::CacheManagement::default();
    state.library_cache_stats = None;
    state.waveform_cache_stats = None;

    state.list_filter.deactivate();
    state.list_state.reset();
    state.scroll = crate::app::state::ScrollPins::default();
    state.popups.close_all();
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
        drop(old_stations);
        drop(old_station_children);
        drop(old_similar);
        drop(old_related);
        drop(old_search);
        drop(old_adventure);
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
        Event::Data(DataEvent::ArtistsLoaded {
            library_key,
            result,
        }) => {
            if state.active_library.as_ref() != Some(&library_key) {
                return vec![];
            }
            let mut artists = match result {
                Ok(artists) => artists,
                Err(error) => {
                    state.library.artists_loading = false;

                    state.set_error(error.message);
                    return vec![];
                }
            };
            // Sort by display title, ignoring "The " prefix
            artists.sort_by_key(|a| helpers::sort_key(&a.title));
            state.library.artists_total = artists.len() as u32;
            state.library.artists = artists;
            state.library.artists_loading = false;

            // Update artist_nav if we're in Artists category. No auto-drill —
            // child columns only open on explicit Enter/Right.
            if state.browse_category == BrowseCategory::Library {
                let title = "artists";
                let items = state.build_artist_root_items();
                state.artist_nav.update_root_items(title, items);
            }
            vec![]
        }
        Event::Data(DataEvent::ArtistsPageLoaded {
            library_key,
            selected_key,
            artists,
            total,
        }) => {
            if state.active_library.as_ref() != Some(&library_key) {
                return vec![];
            }
            helpers::sorted_merge(&mut state.library.artists, artists, |artist| {
                helpers::sort_key(&artist.title)
            });
            state.library.artists_total = total;
            if let Some(selected_key) = selected_key {
                if let Some(position) = state
                    .library
                    .artists
                    .iter()
                    .position(|artist| artist.rating_key == selected_key)
                {
                    state.list_state.artists_index = position;
                }
            }
            state.library.artists_loading = false;
            vec![]
        }
        Event::Data(DataEvent::ArtistsPageFailed { library_key }) => {
            if state.active_library.as_ref() == Some(&library_key) {
                state.library.artists_loading = false;
            }
            vec![]
        }
        Event::Data(DataEvent::AlbumsLoaded(mut albums)) => {
            // Sort by display title, ignoring "The " prefix
            albums.sort_by_key(|a| helpers::sort_key(&a.title));
            state.library.albums = albums;
            state.library.albums_loading = false;
            vec![]
        }
        Event::Data(DataEvent::PlaylistsLoaded { server_url, result }) => {
            if state.connected_server_url != server_url {
                return vec![];
            }
            let mut playlists = match result {
                Ok(playlists) => playlists,
                Err(error) => {
                    state.library.playlists_loading = false;

                    state.set_error(error.message);
                    return vec![];
                }
            };
            // Move "Recently Played" to top of playlist list
            if let Some(pos) = playlists.iter().position(|p| p.title == "Recently Played") {
                if pos > 0 {
                    let rp = playlists.remove(pos);
                    playlists.insert(0, rp);
                }
            }

            // Update playlist_nav with the playlists list. No auto-drill —
            // child columns only open on explicit Enter/Right.
            let items = crate::app::state::BrowseItem::from_playlists(&playlists);
            state.playlist_nav.update_root_items("playlists", items);

            // Build the live-keys set for stale-pruning of saved
            // per-playlist view toggles. Anything saved against a
            // playlist that's no longer in this library's list (i.e.
            // the user deleted it on server) gets dropped from config.
            let live_keys: std::collections::HashSet<String> =
                playlists.iter().map(|p| p.rating_key.clone()).collect();
            let prune_action = state.active_library.as_ref().cloned().map(|lib_key| {
                SettingsAction::PrunePlaylistViews {
                    library_key: lib_key,
                    live_playlist_keys: live_keys,
                }
                .into()
            });

            state.library.playlists = playlists;
            state.library.playlists_loading = false;
            prune_action.map(|a: Action| vec![a]).unwrap_or_default()
        }
        Event::Data(DataEvent::TracksLoaded(tracks)) => {
            state.library.selected_album_tracks = tracks;
            state.library.right_panel_loading = false;
            vec![]
        }
        Event::Data(DataEvent::AlbumTracksLoaded {
            request_key,
            tracks,
        }) => {
            if state.library.right_panel_request_key.as_deref() != Some(&request_key) {
                tracing::debug!("Ignoring stale album-tracks completion: {}", request_key);
                return vec![];
            }
            state.library.right_panel_request_key = None;
            state.library.selected_album_tracks = tracks;
            state.library.right_panel_loading = false;
            vec![]
        }
        Event::Data(DataEvent::ArtistAlbumsLoaded {
            request_key,
            albums,
        }) => {
            if state.library.right_panel_request_key.as_deref() != Some(&request_key) {
                tracing::debug!("Ignoring stale artist-albums completion: {}", request_key);
                return vec![];
            }
            state.library.right_panel_request_key = None;
            state.library.selected_artist_albums = albums;
            state.library.right_panel_loading = false;
            state.focus = crate::app::state::Focus::Right;

            // Check if we need to auto-select a specific album (e.g., from Similar view)
            if let Some(album_key) = state.search.pending_album_key.take() {
                // Find the album in the list (+1 offset for "All Tracks" at index 0)
                if let Some(album_idx) = state
                    .library
                    .selected_artist_albums
                    .iter()
                    .position(|a| a.rating_key == album_key)
                {
                    state.list_state.right_albums_index = album_idx + 1; // +1 for "All Tracks"
                    state.library.selected_album_title = state.library.selected_artist_albums
                        [album_idx]
                        .title
                        .clone();
                    return vec![DataAction::LoadAlbumTracks {
                        rating_key: album_key,
                    }
                    .into()];
                }
            }
            vec![]
        }
        Event::Data(DataEvent::ArtistAllTracksLoaded {
            request_key,
            tracks,
        }) => {
            if state.library.right_panel_request_key.as_deref() != Some(&request_key) {
                tracing::debug!("Ignoring stale artist-tracks completion: {}", request_key);
                return vec![];
            }
            state.library.right_panel_request_key = None;
            state.library.selected_album_tracks = tracks;
            state.library.right_panel_loading = false;
            vec![]
        }
        Event::Data(DataEvent::CategoryTracksLoaded {
            request_key,
            tracks,
        }) => {
            if state.library.right_panel_request_key.as_deref() != Some(&request_key) {
                tracing::debug!("Ignoring stale category-tracks completion: {}", request_key);
                return vec![];
            }
            state.library.right_panel_request_key = None;
            if state.library.selected_album_title.is_empty() {
                if let Some(first) = tracks.first() {
                    state.library.selected_album_title = first.album_name().to_string();
                }
            }
            state.library.selected_album_tracks = tracks;
            state.library.right_panel_loading = false;
            vec![]
        }
        Event::Data(DataEvent::CategoryAlbumsLoaded {
            albums,
            status_message,
        }) => {
            state.library.right_panel_mode = crate::app::state::RightPanelMode::CategoryAlbums;
            state.library.tag_albums = albums;
            state.library.tag_albums_index = 0;
            state.set_status(status_message);
            state.library.right_panel_loading = false;
            vec![]
        }
        Event::Data(DataEvent::ScopedLoadError {
            request_key,
            message,
        }) => {
            let mut current = false;
            if state.library.right_panel_request_key.as_deref() == Some(&request_key) {
                state.library.right_panel_request_key = None;
                state.library.right_panel_loading = false;
                current = true;
            }
            if state.similar.request_key.as_deref() == Some(&request_key) {
                state.similar.request_key = None;
                state.similar.loading = false;
                current = true;
            }
            if state.related.source_key == request_key {
                state.related.loading = false;
                current = true;
            }
            if current {
                state.set_error(message);
            } else {
                tracing::debug!("Ignoring stale scoped load error: {}", request_key);
            }
            vec![]
        }
        Event::Data(DataEvent::AllAlbumsForMillerLoaded {
            library_key,
            request_id,
            replace_child,
            mut albums,
        }) => {
            if state.active_library.as_ref() != Some(&library_key)
                || state.artist_nav_request_id != request_id
            {
                return vec![];
            }
            // Async completion for LoadAllAlbumsForMiller when state.library.albums was empty
            albums.sort_by_key(|a| helpers::sort_key(&a.title));
            state.library.albums = albums;
            state.library.albums_total = state.library.albums.len() as u32;
            // Now push the column (same as the sync path in dispatch_miller)
            vec![MillerAction::LoadAllAlbumsForMiller { replace_child }.into()]
        }
        Event::Data(DataEvent::AllAlbumsForMillerFailed {
            library_key,
            request_id,
            error,
        }) => {
            if state.active_library.as_ref() != Some(&library_key)
                || state.artist_nav_request_id != request_id
            {
                return vec![];
            }
            state.artist_nav.loading = false;

            state.set_error(error.message);
            vec![]
        }
        Event::Data(DataEvent::SimilarAlbumsLoaded {
            request_key,
            albums,
        }) => {
            if state.similar.request_key.as_deref() != Some(&request_key) {
                tracing::debug!("Ignoring stale similar-albums completion: {}", request_key);
                return vec![];
            }
            state.similar.request_key = None;
            state.similar.albums = albums;
            state.similar.mode = crate::app::state::SimilarMode::Albums;
            state.similar.loading = false;
            state.list_state.similar_index = 0;
            vec![]
        }
        Event::Data(DataEvent::SimilarTracksLoaded {
            request_key,
            tracks,
        }) => {
            if state.similar.request_key.as_deref() != Some(&request_key) {
                tracing::debug!("Ignoring stale similar-tracks completion: {}", request_key);
                return vec![];
            }
            state.similar.request_key = None;
            state.similar.tracks = tracks;
            state.similar.mode = crate::app::state::SimilarMode::Tracks;
            state.similar.loading = false;
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
            if state.similar.request_key.as_deref() != Some(&request_key) {
                tracing::debug!("Ignoring stale similar-artists completion: {}", request_key);
                return vec![];
            }
            state.similar.request_key = None;
            state.similar.artists = artists;
            state.similar.mode = crate::app::state::SimilarMode::Artists;
            state.similar.loading = false;
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

        Event::Ui(UiEvent::AdventureLauncherAlbumsLoaded {
            server_url,
            request_id,
            artist_key,
            artist_name,
            result,
        }) => {
            if state.connected_server_url.as_deref() != server_url.as_deref()
                || state.adventure_launcher_request_id != request_id
            {
                return vec![];
            }
            if let Some(ref mut launcher) = state.popups.adventure_launcher {
                launcher.loading = false;
                match result {
                    Ok(albums) => {
                        launcher.drill = crate::app::state::AdventureDrillLevel::ArtistAlbums {
                            artist_key,
                            artist_name,
                            albums,
                        };
                        launcher.item_index = 0;
                        launcher.focus = crate::app::state::SearchFocus::Results;
                    }
                    Err(error) => {
                        state.set_error(error.message);
                    }
                }
            }
            vec![]
        }
        Event::Ui(UiEvent::AdventureLauncherTracksLoaded {
            server_url,
            request_id,
            album_key,
            album_title,
            artist_name,
            result,
        }) => {
            if state.connected_server_url.as_deref() != server_url.as_deref()
                || state.adventure_launcher_request_id != request_id
            {
                return vec![];
            }
            if let Some(ref mut launcher) = state.popups.adventure_launcher {
                launcher.loading = false;
                match result {
                    Ok(tracks) => {
                        launcher.drill = crate::app::state::AdventureDrillLevel::AlbumTracks {
                            album_key,
                            album_title,
                            artist_name,
                            tracks,
                        };
                        launcher.item_index = 0;
                        launcher.focus = crate::app::state::SearchFocus::Results;
                    }
                    Err(error) => {
                        state.set_error(error.message);
                    }
                }
            }
            vec![]
        }
        Event::Data(DataEvent::ApiError(msg)) => {
            state.set_error(msg);
            vec![]
        }
        Event::Playback(PlaybackEvent::TrackStarted) => {
            state.playback.status = PlayStatus::Playing;
            state.playback.position_ms = 0;
            // Successful playback supersedes any prior error banner
            // ("Playback stopped after multiple consecutive errors",
            // "Track Not Found", etc). Without this the red banner
            // sticks around even after the user resumes / the queue
            // recovers, making the app feel broken when it isn't.
            state.consecutive_playback_errors = 0;
            state.clear_error();
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
                if !state.playback.scrobble_reported {
                    state.playback.scrobble_reported = true;
                    crate::app::sources::navidrome::scrobble(state, event_tx, &track);
                }
                // Use track duration as position (track finished)
                let _position = track.duration_ms();
            }
            if state
                .station_starting
                .as_ref()
                .is_some_and(|start| start.continue_playback == Some(state.playback.request_id))
            {
                // Natural completion waits for Sonic Radio, rather than playing
                // the old queue's next song. Explicit transport input remains free
                // to cancel or supersede the pending station.
                state.set_status("Waiting for Sonic Radio…".into());
                vec![]
            } else {
                vec![PlaybackAction::Next.into()]
            }
        }
        Event::Playback(PlaybackEvent::PlaybackPaused) => {
            state.playback.status = PlayStatus::Paused;
            vec![]
        }
        Event::Playback(PlaybackEvent::PlaybackResumed) => {
            state.playback.status = PlayStatus::Playing;
            vec![]
        }
        Event::Playback(PlaybackEvent::PlaybackStopped) => {
            state.playback.status = PlayStatus::Stopped;
            state.playback.position_ms = 0;
            vec![]
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
                state.popups.confirm_dialog = Some(crate::app::state::ConfirmDialog {
                    title: "Track Not Found".to_string(),
                    message: "This track may have been removed. Refresh cache?".to_string(),
                    on_confirm: crate::app::state::ConfirmAction::RefreshCache,
                    selected_yes: true,
                });
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

        Event::Playback(PlaybackEvent::BufferingStart) => {
            state.playback.status = PlayStatus::Buffering;
            vec![]
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
        Event::Playback(PlaybackEvent::PositionUpdate(pos)) => {
            state.playback.position_ms = pos;
            vec![]
        }
        Event::Artwork(ArtworkEvent::ArtworkLoaded {
            generation,
            thumb_path,
            data,
        }) => {
            if generation != state.artwork.grid_generation
                || state.artwork.pending_thumb.as_deref() != Some(&thumb_path)
            {
                tracing::debug!("Ignoring stale artwork completion: {}", thumb_path);
                return vec![];
            }
            state.artwork.current_thumb = Some(thumb_path);
            state.artwork.current_data = Some(data);
            state.artwork.loading = false;
            state.artwork.pending_thumb = None;
            vec![]
        }
        Event::Artwork(ArtworkEvent::ArtworkFailed {
            generation,
            thumb_path,
        }) => {
            if generation != state.artwork.grid_generation
                || state.artwork.pending_thumb.as_deref() != Some(&thumb_path)
            {
                tracing::debug!("Ignoring stale artwork failure: {}", thumb_path);
                return vec![];
            }
            state.artwork.current_thumb = None;
            state.artwork.current_data = None;
            state.artwork.loading = false;
            state.artwork.pending_thumb = None;
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

        Event::Artwork(ArtworkEvent::ArtworkCacheStats { count, total_bytes }) => {
            state.artwork.cache_stats = Some((count, total_bytes));
            vec![]
        }
        Event::Cache(CacheEvent::LibraryCacheStats {
            total_bytes,
            breakdown,
        }) => {
            state.library_cache_stats = Some((total_bytes, breakdown));
            vec![]
        }
        Event::Cache(CacheEvent::WaveformCacheStats { count, total_bytes }) => {
            state.waveform_cache_stats = Some((count, total_bytes));
            vec![]
        }

        Event::Tick => {
            // Clear expired modifier bars
            if let Some(deadline) = state.alt_bar_until {
                if std::time::Instant::now() >= deadline {
                    state.alt_bar_until = None;
                }
            }

            // Clear expired toasts (5 second display)
            if let Some(show_time) = state.notifications.toast_show_time {
                if show_time.elapsed() > Duration::from_secs(5) {
                    state.notifications.toast_message = None;
                    state.notifications.toast_show_time = None;
                }
            }

            // Clear expired status messages (5 second display)
            if let Some(show_time) = state.notifications.status_show_time {
                if show_time.elapsed() > Duration::from_secs(5) {
                    state.clear_status();
                }
            }

            // Periodic playback progress report to server (~10 seconds)
            if state.playback.status == PlayStatus::Playing {
                let reached_scrobble_threshold = state.playback.duration_ms > 0
                    && state.playback.position_ms.saturating_mul(10)
                        >= state.playback.duration_ms.saturating_mul(9);
                if reached_scrobble_threshold && !state.playback.scrobble_reported {
                    if let Some(track) = state.current_track().cloned() {
                        state.playback.scrobble_reported = true;
                        crate::app::sources::navidrome::scrobble(state, event_tx, &track);
                    }
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

            // Lazy-art settle: if `suppress_loads` was raised by a
            // recent rapid-nav gesture and the user has been still for
            // `ART_LOAD_PAUSE_MS`, clear the gate and dispatch one
            // viewport-wide `LoadAlbumArt` batch.
            if let Some(actions) = super::lazy_art::settle(state) {
                if !actions.is_empty() {
                    return actions;
                }
            }

            // Track-details pane: lazy-fetch sonically-similar tracks
            // for whatever Track row is currently focused. Returned
            // as a follow-up `LoadTrackPaneSimilar` action so the
            // dispatcher (which has client access) actually fires
            // the API call. Idempotent — the dispatcher early-returns
            // when the entry is already loaded or in flight.
            if state.view == crate::app::state::View::Browse {
                if let Some(track) = state.focused_track() {
                    let key = track.rating_key.clone();
                    if crate::app::sources::sonic::enabled(state)
                        && !key.is_empty()
                        && !state.track_pane_similar.contains_key(&key)
                        && !state.track_pane_similar_loading.contains(&key)
                    {
                        return vec![crate::app::action::DataAction::LoadTrackPaneSimilar {
                            rating_key: key,
                        }
                        .into()];
                    }
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
            if state.view == crate::app::state::View::Browse && state.artwork.grid_pending.len() < 4
            {
                let nav = match state.browse_category {
                    crate::app::state::BrowseCategory::Library => &state.artist_nav,
                    crate::app::state::BrowseCategory::Playlists => &state.playlist_nav,
                    cat if cat.is_tag_section() => &state.tag_nav,
                    _ => &state.artist_nav,
                };
                let max_batch = 4usize.saturating_sub(state.artwork.grid_pending.len());
                let mut to_load: Vec<(String, String)> = Vec::new();
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
                    let col = match nav.columns.get(ci) {
                        Some(c) => c,
                        None => continue,
                    };
                    if !col.artwork_visible {
                        continue;
                    }
                    let total_items = col.items.len();
                    if total_items == 0 {
                        continue;
                    }
                    // Same visible-window math as the renderer, used
                    // here as a heuristic for "what the user is most
                    // likely about to see". Keeping it consistent with
                    // the TUI's `render_album_art_grid` formula avoids
                    // wasted prefetches on rows that are off-screen.
                    let inner_height = state.terminal_height.saturating_sub(4) as usize;
                    let target_visible = 3usize.max((total_items).min(5));
                    let row_height = (inner_height / target_visible).max(3);
                    let visible_rows = (inner_height / row_height).max(1);
                    let scroll_offset = crate::services::NavigationService::calc_scroll_offset(
                        col.selected_index,
                        visible_rows,
                        total_items,
                    );
                    let end = (scroll_offset + visible_rows).min(total_items);
                    for item in &col.items[scroll_offset..end] {
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
                                if let Some(artist_key) = scope.artist_key() {
                                    if !state.artwork.grid_cache.contains_key(artist_key)
                                        && !state.artwork.grid_pending.contains(artist_key)
                                    {
                                        to_load.push((artist_key.to_string(), thumb.clone()));
                                    }
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
            // when the user is on a view where the visualizer panel is rendered.
            // The GUI's Queue view now shows the visualizer always-on in its
            // bottom half, so we trip this on both View::NowPlaying and
            // View::Queue. Catches all edge cases (track change, re-entry,
            // failed downloads) without fragile event-based triggering.
            if matches!(state.view, View::NowPlaying | View::Queue) {
                if let Some(track) = state.current_track().cloned() {
                    let tk = &track.rating_key;

                    // Ensure track_key is set (handles track change while on this view)
                    if state.waveform.track_key.as_ref() != Some(tk) {
                        state.waveform = crate::app::state::WaveformState::default();
                        state.waveform.track_key = Some(tk.clone());
                        state.spectrogram = crate::app::state::SpectrogramState::default();
                        state.spectrogram.track_key = Some(tk.clone());
                    }

                    // Trigger waveform if needed (co-generates spectrogram)
                    if state.waveform.data.is_none()
                        && !state.waveform.generating
                        && state.waveform.error.is_none()
                    {
                        return vec![SystemAction::LoadWaveform.into()];
                    }

                    // Trigger spectrogram independently if waveform is done but spectrogram isn't
                    if state.spectrogram.data.is_none()
                        && !state.spectrogram.generating
                        && state.spectrogram.error.is_none()
                        && state.waveform.data.is_some()
                    {
                        state.spectrogram.error = None;
                        return vec![SystemAction::LoadSpectrogram.into()];
                    }
                }
            }

            // Periodic cache save: save if dirty, idle for 30+ seconds, and 2+ minutes since last save

            helpers::refresh_due_caches(state, event_tx);

            vec![]
        }

        Event::Mouse(mouse_event) => super::mouse_input::handle_mouse(mouse_event, state),
        Event::Visualizer(VisualizerEvent::WaveformGenerated { track_key, data }) => {
            if state.waveform.track_key.as_ref() == Some(&track_key) {
                state.waveform.data = Some(data);
                state.waveform.generating = false;
                state.waveform.error = None;
                tracing::debug!("Waveform generated for track: {}", track_key);
            }
            vec![]
        }
        Event::Visualizer(VisualizerEvent::WaveformFailed { track_key, error }) => {
            if state.waveform.track_key.as_ref() == Some(&track_key) {
                if state.waveform.retry_count < 3 {
                    // Silent retry: keep generating=true so UI shows "Generating..."
                    state.waveform.retry_count += 1;
                    let retry_num = state.waveform.retry_count;
                    let tx = event_tx.clone();
                    let tk = track_key.clone();
                    let library_generation = state.library_generation;
                    crate::app::tasks::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(2 * retry_num as u64)).await;
                        let _ = tx
                            .send(Event::for_library(
                                library_generation,
                                VisualizerEvent::WaveformRetry(tk),
                            ))
                            .await;
                    });
                    tracing::info!(
                        "Waveform retry {}/3 for {}: {}",
                        retry_num,
                        track_key,
                        error
                    );
                } else {
                    // Max retries exhausted — show error
                    state.waveform.error = Some(error.clone());
                    state.waveform.generating = false;
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
                state.waveform.data = Some(data);
                state.waveform.generating = false;
                state.waveform.error = None;
                tracing::debug!("Waveform loaded from cache for track: {}", track_key);
            }
            vec![]
        }
        Event::Visualizer(VisualizerEvent::WaveformRetry(track_key)) => {
            // generating is still true from the failed attempt; clear it so
            // LoadWaveform's needs_generation check passes.
            if state.waveform.track_key.as_ref() == Some(&track_key)
                && state.waveform.data.is_none()
            {
                state.waveform.generating = false;
                vec![SystemAction::LoadWaveform.into()]
            } else {
                vec![]
            }
        }
        Event::Visualizer(VisualizerEvent::SpectrogramGenerated { track_key, data }) => {
            if state.spectrogram.track_key.as_ref() == Some(&track_key) {
                state.spectrogram.data = Some(data);
                state.spectrogram.generating = false;
                state.spectrogram.error = None;
                tracing::debug!("Spectrogram generated for track: {}", track_key);
            }
            vec![]
        }
        Event::Visualizer(VisualizerEvent::SpectrogramFailed { track_key, error }) => {
            if state.spectrogram.track_key.as_ref() == Some(&track_key) {
                state.spectrogram.generating = false;
                // Only set error for real failures, not empty signals from cache miss
                if !error.is_empty() {
                    state.spectrogram.error = Some(error.clone());
                    tracing::warn!("Spectrogram failed for {}: {}", track_key, error);
                }
            }
            vec![]
        }
        Event::Visualizer(VisualizerEvent::SpectrogramCacheHit { track_key, data }) => {
            if state.spectrogram.track_key.as_ref() == Some(&track_key) {
                state.spectrogram.data = Some(data);
                state.spectrogram.generating = false;
                state.spectrogram.error = None;
                tracing::debug!("Spectrogram loaded from cache for track: {}", track_key);
            }
            vec![]
        }
        Event::Radio(event) => super::events_radio::handle(event, state),
        Event::Playlist(event) => super::events_playlist::handle(event, state),

        // Playlist tracks preloaded in background

        // DJ mode tracks ready (continuous modes)
        Event::Ui(UiEvent::DjTracksReady {
            result,
            insert_next,
        }) => {
            vec![RadioAction::DjModeTracksReady(result, insert_next).into()]
        }
        // DJ mode batch ready (inserter modes)
        Event::Ui(UiEvent::DjBatchReady { inserts }) => {
            vec![RadioAction::DjModeBatchReady(inserts).into()]
        }
        // Queue remix batch ready
        Event::Ui(UiEvent::RemixBatchReady { outcome }) => {
            vec![QueueAction::RemixBatchReady(outcome).into()]
        }
        Event::Ui(UiEvent::RemixDoppelgangerReady { outcome }) => {
            vec![QueueAction::RemixDoppelgangerReady(outcome).into()]
        }

        // Multi-artist radio complete
        Event::Ui(UiEvent::ArtistRadioComplete { outcome }) => {
            vec![SettingsAction::ArtistRadioComplete(outcome).into()]
        }

        Event::Ui(UiEvent::ArtistBioLoaded {
            generation,
            request_id,
            result,
        }) => {
            if generation != state.library_generation || state.artist_bio_request_id != request_id {
                return vec![];
            }
            if let Some(popup) = &mut state.popups.artist_bio {
                popup.loading = false;
                popup.task = None;
                popup.document =
                    result.unwrap_or_else(|error| crate::services::biography::Biography {
                        text: error,
                        ..Default::default()
                    });
                popup.scroll = 0;
            }
            vec![]
        }

        // Inline list filter completed
        Event::Ui(UiEvent::ListFilterCompleted {
            version,
            column_results,
        }) => {
            // Only apply if this is the most recent filter version
            if version == state.list_filter.version {
                state.list_filter.loading = false;
                state.list_filter.selected = 0;
                state.list_filter.results = Some(
                    column_results
                        .get(state.list_filter.column)
                        .cloned()
                        .unwrap_or_default(),
                );
                state.list_filter.column_results = column_results;
                // Only update column selection if user is still on the filter column.
                // If they've drilled deeper (e.g., into subfolders), preserve their
                // current navigation — changing the selection would jump them away.
                if super::dispatch_search::is_on_filter_column(state) {
                    if let Some(ref results) = state.list_filter.results {
                        if let Some(&first_idx) = results.matched_indices.first() {
                            super::key_input::update_filter_column_selection(state, first_idx);
                        }
                    }
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
