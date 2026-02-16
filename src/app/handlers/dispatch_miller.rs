//! Miller column dispatch handlers for all *ForMiller and *FromMiller actions.

use crate::app::{Action, AppState, Event};
use crate::app::state::{BrowseCategory, BrowseColumn, BrowseItem, PlaybackMode, View};
use crate::api::PlexClient;
use crate::audio::AudioPlayer;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Max number of album art entries to load at once to avoid blocking the event loop
/// with synchronous disk I/O in LoadAlbumArt.
const ART_BATCH_LIMIT: usize = 30;

/// Collect album art (key, thumb) pairs from a column that aren't already cached or pending.
/// Limited to `ART_BATCH_LIMIT` items around the column's selected_index to avoid
/// blocking the event loop with thousands of synchronous disk reads.
fn collect_art_to_load(
    col: Option<&BrowseColumn>,
    cache: &std::collections::HashMap<String, Vec<u8>>,
    pending: &std::collections::HashSet<String>,
) -> Vec<(String, String)> {
    let Some(col) = col else { return vec![] };
    let total = col.items.len();
    if total == 0 { return vec![]; }

    // Window around selected_index
    let half = ART_BATCH_LIMIT / 2;
    let center = col.selected_index;
    let start = center.saturating_sub(half);
    let end = (center + ART_BATCH_LIMIT).min(total);

    let mut to_load = Vec::new();
    for item in &col.items[start..end] {
        match item {
            BrowseItem::Album { key, thumb: Some(thumb), .. } => {
                if !cache.contains_key(key) && !pending.contains(key) {
                    to_load.push((key.clone(), thumb.clone()));
                }
            }
            BrowseItem::AllTracks { artist_key, thumb: Some(thumb), .. } => {
                if !cache.contains_key(artist_key) && !pending.contains(artist_key) {
                    to_load.push((artist_key.clone(), thumb.clone()));
                }
            }
            BrowseItem::Artist { key, thumb: Some(thumb), .. } => {
                if !cache.contains_key(key) && !pending.contains(key) {
                    to_load.push((key.clone(), thumb.clone()));
                }
            }
            _ => {}
        }
    }
    to_load
}

/// Collect art for the viewport of the focused album column.
/// Called after scroll navigation to lazily load art for newly visible items.
pub fn collect_viewport_art(state: &AppState) -> Vec<(String, String)> {
    if !state.album_art_view && !state.artist_art_view { return vec![]; }

    let nav = match state.browse_nav() {
        Some(n) => n,
        None => return vec![],
    };

    let Some(col) = nav.focused() else { return vec![] };
    // Only bother for album columns
    let has_art_items = col.items.iter().any(|item| matches!(item, BrowseItem::Album { .. } | BrowseItem::AllTracks { .. } | BrowseItem::Artist { .. }));
    if !has_art_items { return vec![]; }

    collect_art_to_load(Some(col), &state.album_art_cache, &state.album_art_pending)
}

/// Dispatch Miller column actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: Action,
    state: &mut AppState,
    client: &mut PlexClient,
    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    match action {
        // ================================================================
        // Miller Column Actions for Artists View
        // ================================================================

        Action::LoadArtistAlbumsForMiller { artist_key } => {
            // Load albums for artist and add as new column in artist_nav
            // Prepend "All Tracks" entry before albums (same as old render path)
            state.artist_nav.loading = true;

            match client.get_artist_albums(&artist_key).await {
                Ok(albums) => {
                    // Create special entries: Artist Radio, then All Tracks
                    let artist_thumb = state.artists.iter()
                        .find(|a| a.rating_key == artist_key)
                        .and_then(|a| a.thumb.clone());
                    let artist_radio = BrowseItem::ArtistRadio {
                        artist_key: artist_key.clone(),
                        artist_name: state.selected_artist_name.clone(),
                        thumb: artist_thumb.clone(),
                    };
                    let all_tracks = BrowseItem::AllTracks {
                        artist_key: artist_key.clone(),
                        artist_name: state.selected_artist_name.clone(),
                        thumb: artist_thumb,
                    };
                    // Then add albums
                    let mut items = vec![artist_radio, all_tracks];

                    // Start with albums from API, then merge any from preloaded
                    // state.albums that the API missed (e.g. compilation-subtype
                    // albums that Plex hides from /children endpoint)
                    let mut all_albums = albums;
                    {
                        let api_keys: std::collections::HashSet<&str> = all_albums.iter()
                            .map(|a| a.rating_key.as_str())
                            .collect();
                        let missing: Vec<_> = state.albums.iter()
                            .filter(|a| a.parent_rating_key.as_deref() == Some(&artist_key)
                                && !api_keys.contains(a.rating_key.as_str()))
                            .cloned()
                            .collect();
                        if !missing.is_empty() {
                            tracing::debug!("Merging {} albums from preload that API didn't return for artist {}", missing.len(), artist_key);
                            all_albums.extend(missing);
                        }
                    }

                    items.extend(BrowseItem::from_albums(&all_albums));

                    if state.compilations_detected {
                        // Append single-artist compilations whose parent is NOT
                        // this artist (detected via track analysis)
                        if let Some(solo_comps) = state.single_artist_compilations.get(&artist_key) {
                            let existing_keys: std::collections::HashSet<&str> = all_albums.iter()
                                .map(|a| a.rating_key.as_str())
                                .collect();
                            let new_comps: Vec<_> = solo_comps.iter()
                                .filter(|a| !existing_keys.contains(a.rating_key.as_str()))
                                .cloned()
                                .collect();
                            if !new_comps.is_empty() {
                                items.extend(BrowseItem::from_albums(&new_comps));
                            }
                        }

                        // Append multi-artist compilation albums this artist appears on
                        if let Some(comp_album_keys) = state.artist_compilation_map.get(&artist_key) {
                            let comp_items: Vec<BrowseItem> = comp_album_keys.iter()
                                .filter_map(|key| {
                                    state.compilation_albums.iter()
                                        .find(|a| a.rating_key == *key)
                                        .map(|a| BrowseItem::Album {
                                            key: a.rating_key.clone(),
                                            title: a.title.clone(),
                                            artist: a.artist_name().to_string(),
                                            year: a.year,
                                            thumb: a.thumb.clone(),
                                            is_placeholder: false,
                                        })
                                })
                                .collect();
                            if !comp_items.is_empty() {
                                items.extend(comp_items);
                            }
                        }
                    }

                    let title = state.selected_artist_name.clone();
                    let col = BrowseColumn::new(title, items);
                    state.artist_nav.push_column(col);

                    // Preload all album art for the newly pushed column
                    let art_batch = if state.album_art_view {
                        collect_art_to_load(state.artist_nav.columns.last(), &state.album_art_cache, &state.album_art_pending)
                    } else {
                        vec![]
                    };

                    // Auto-select album and drill into tracks if pending_album_key is set (Alt+B)
                    if let Some(album_key) = state.pending_album_key.take() {
                        if let Some(col) = state.artist_nav.columns.last_mut() {
                            if let Some(idx) = col.items.iter().position(|item| {
                                matches!(item, BrowseItem::Album { key, .. } if *key == album_key)
                            }) {
                                col.selected_index = idx;
                                if let Some(BrowseItem::Album { title, .. }) = col.items.get(idx) {
                                    state.selected_album_title = title.clone();
                                }
                                let mut actions = vec![Action::LoadAlbumTracksForMiller { album_key }];
                                if !art_batch.is_empty() {
                                    actions.push(Action::LoadAlbumArt(art_batch));
                                }
                                return Ok(actions);
                            }
                        }
                    }

                    if !art_batch.is_empty() {
                        state.artist_nav.loading = false;
                        return Ok(vec![Action::LoadAlbumArt(art_batch)]);
                    }
                }
                Err(e) => {
                    state.set_error(format!("Failed to load albums: {}", e));
                }
            }
            state.artist_nav.loading = false;
        }

        Action::LoadAlbumTracksForMiller { album_key } => {
            // Load tracks for album and add as new column in artist_nav
            state.artist_nav.loading = true;

            match client.get_album_tracks(&album_key).await {
                Ok(tracks) => {
                    let items = BrowseItem::from_tracks(&tracks);
                    let title = state.selected_album_title.clone();
                    // Store full tracks for playback (includes media info)
                    let col = BrowseColumn::new_with_tracks(title, items, tracks);
                    state.artist_nav.push_column(col);

                    // Auto-select track if pending from search navigation
                    if let Some(ref tk) = state.pending_track_key {
                        if let Some(col) = state.artist_nav.columns.last_mut() {
                            if let Some(pos) = col.items.iter().position(|i| i.key() == tk.as_str()) {
                                col.selected_index = pos;
                            }
                        }
                        state.pending_track_key = None;
                    }
                }
                Err(e) => {
                    state.set_error(format!("Failed to load tracks: {}", e));
                }
            }
            state.artist_nav.loading = false;
        }

        Action::LoadArtistAllTracksForMiller { artist_key } => {
            // Load all tracks by an artist and add as new column in artist_nav
            // This is triggered by selecting "All Tracks" entry in the albums column
            state.artist_nav.loading = true;

            match client.get_artist_all_tracks(&artist_key).await {
                Ok(tracks) => {
                    let items = BrowseItem::from_tracks(&tracks);
                    let title = state.selected_album_title.clone();
                    // Store full tracks for playback (includes media info)
                    let col = BrowseColumn::new_with_tracks(title, items, tracks);
                    state.artist_nav.push_column(col);
                }
                Err(e) => {
                    state.set_error(format!("Failed to load tracks: {}", e));
                }
            }
            state.artist_nav.loading = false;
        }

        Action::LoadAllAlbumsForMiller => {
            // Load all albums as a Miller column (triggered by "► All Artists" entry)
            // Uses already-loaded state.albums; fetches async if empty.
            if state.albums.is_empty() {
                state.artist_nav.loading = true;
                // Fetch in background to avoid blocking the event loop
                let tx = event_tx.clone();
                let client_clone = client.clone();
                let lib_key = state.active_library.clone().unwrap_or_default();
                tokio::spawn(async move {
                    match client_clone.get_albums(&lib_key).await {
                        Ok(albums) => {
                            let _ = tx.send(Event::AllAlbumsForMillerLoaded(albums)).await;
                        }
                        Err(e) => {
                            let _ = tx.send(Event::DataLoadError(format!("Failed to load albums: {}", e))).await;
                        }
                    }
                });
                return Ok(vec![]);
            }
            let items = BrowseItem::from_albums(&state.albums);
            let col = BrowseColumn::new("all artists", items);
            state.artist_nav.push_column(col);

            // Preload album art if in art view (viewport-limited)
            let art_batch = if state.album_art_view {
                collect_art_to_load(state.artist_nav.columns.last(), &state.album_art_cache, &state.album_art_pending)
            } else {
                vec![]
            };
            state.artist_nav.loading = false;
            if !art_batch.is_empty() {
                return Ok(vec![Action::LoadAlbumArt(art_batch)]);
            }
        }

        Action::PlayTrackFromMiller { column_index, track_index } => {
            // Get tracks from the specified column and play from track_index
            if let Some(col) = state.artist_nav.columns.get(column_index) {
                let tracks = helpers::collect_tracks_from_column(col);
                if !tracks.is_empty() {
                    if state.playback_mode == PlaybackMode::Radio {
                        state.radio.clear();
                    }
                    // Move played tracks to history
                    if let Some(qi) = state.queue_index {
                        if qi < state.queue.len() {
                            let played: Vec<crate::api::models::Track> = state.queue.drain(..=qi).collect();
                            state.play_history.extend(played);
                        }
                    }
                    audio.track_cache.flush();
                    state.queue.splice(0..0, tracks);
                    state.queue_index = Some(track_index);
                    state.playback_mode = PlaybackMode::Queue;
                    state.list_state.queue_index = state.play_history.len();
                    state.view = View::NowPlaying;
                    helpers::play_current_track(event_tx, state, client, audio).await;
                }
            }
        }

        // Miller Column Actions for Genres View
        // ================================================================

        Action::LoadGenreAlbumsForMiller { genre_key } => {
            // Load albums for genre and add as new column in genre_nav
            state.genre_nav.loading = true;

            if let Some(lib_key) = &state.active_library.clone() {
                // For "All" tab, keys are prefixed ("lib:", "art:", "alb:", "mood:", "style:").
                // Parse the prefix to determine which API to call, or fall back to genre_content_type.
                let albums_result = if let Some(stripped) = genre_key.strip_prefix("lib:") {
                    client.get_genre_albums(lib_key, stripped).await
                } else if let Some(stripped) = genre_key.strip_prefix("art:") {
                    client.get_artist_genre_albums(lib_key, stripped).await
                } else if let Some(stripped) = genre_key.strip_prefix("alb:") {
                    client.get_album_genre_albums(lib_key, stripped).await
                } else if let Some(stripped) = genre_key.strip_prefix("mood:") {
                    client.get_mood_albums(lib_key, stripped).await
                } else if let Some(stripped) = genre_key.strip_prefix("style:") {
                    client.get_style_albums(lib_key, stripped).await
                } else {
                    // No prefix — use genre_content_type (for non-All tabs)
                    match state.genre_content_type {
                        crate::app::state::GenreContentType::ArtistGenres => {
                            client.get_artist_genre_albums(lib_key, &genre_key).await
                        }
                        crate::app::state::GenreContentType::AlbumGenres => {
                            client.get_album_genre_albums(lib_key, &genre_key).await
                        }
                        crate::app::state::GenreContentType::Moods => {
                            client.get_mood_albums(lib_key, &genre_key).await
                        }
                        crate::app::state::GenreContentType::Styles => {
                            client.get_style_albums(lib_key, &genre_key).await
                        }
                        _ => {
                            client.get_genre_albums(lib_key, &genre_key).await
                        }
                    }
                };

                match albums_result {
                    Ok(albums) => {
                        let items = BrowseItem::from_albums(&albums);
                        let col = BrowseColumn::new("albums", items);
                        state.genre_nav.push_column(col);

                        // Preload all album art for the newly pushed column
                        if state.album_art_view {
                            let art_batch = collect_art_to_load(state.genre_nav.columns.last(), &state.album_art_cache, &state.album_art_pending);
                            if !art_batch.is_empty() {
                                state.genre_nav.loading = false;
                                return Ok(vec![Action::LoadAlbumArt(art_batch)]);
                            }
                        }
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load albums: {}", e));
                    }
                }
            }
            state.genre_nav.loading = false;
        }

        Action::LoadGenreTracksForMiller { album_key } => {
            // Load tracks for album and add as new column in genre_nav
            state.genre_nav.loading = true;

            match client.get_album_tracks(&album_key).await {
                Ok(tracks) => {
                    let items = BrowseItem::from_tracks(&tracks);
                    // Store full tracks for playback (includes media info)
                    let col = BrowseColumn::new_with_tracks("tracks", items, tracks);
                    state.genre_nav.push_column(col);
                }
                Err(e) => {
                    state.set_error(format!("Failed to load tracks: {}", e));
                }
            }
            state.genre_nav.loading = false;
        }

        Action::PlayGenreTrackFromMiller { column_index, track_index } => {
            // Get tracks from the specified column and play from track_index
            if let Some(col) = state.genre_nav.columns.get(column_index) {
                let tracks = helpers::collect_tracks_from_column(col);
                if !tracks.is_empty() {
                    if state.playback_mode == PlaybackMode::Radio {
                        state.radio.clear();
                    }
                    if let Some(qi) = state.queue_index {
                        if qi < state.queue.len() {
                            let played: Vec<crate::api::models::Track> = state.queue.drain(..=qi).collect();
                            state.play_history.extend(played);
                        }
                    }
                    audio.track_cache.flush();
                    state.queue.splice(0..0, tracks);
                    state.queue_index = Some(track_index);
                    state.playback_mode = PlaybackMode::Queue;
                    state.list_state.queue_index = state.play_history.len();
                    state.view = View::NowPlaying;
                    helpers::play_current_track(event_tx, state, client, audio).await;
                }
            }
        }

        // Miller Column Actions for Playlists View
        // ================================================================

        Action::LoadPlaylistTracksForMiller { playlist_key } => {
            // Always fetch fresh from API (playlist contents may change, e.g. smart playlists)
            state.playlist_nav.loading = true;
            let tx = event_tx.clone();
            let client_clone = client.clone();
            let pk = playlist_key.clone();
            tokio::spawn(async move {
                let backoff = [1u64, 2, 4];
                let mut last_err = String::new();
                for attempt in 0..3u32 {
                    match client_clone.get_playlist_tracks(&pk).await {
                        Ok(tracks) => {
                            let _ = tx.send(Event::PlaylistTracksForMillerLoaded {
                                playlist_key: pk, tracks,
                            }).await;
                            return;
                        }
                        Err(e) => {
                            last_err = format!("{}", e);
                            if attempt < 2 {
                                let delay = backoff[attempt as usize];
                                tracing::debug!("Playlist load failed (attempt {}), retrying in {}s: {}", attempt + 1, delay, last_err);
                                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                            }
                        }
                    }
                }
                let _ = tx.send(Event::PlaylistTracksForMillerFailed {
                    playlist_key: pk, error: last_err,
                }).await;
            });
        }

        Action::PlayPlaylistTrackFromMiller { column_index, track_index } => {
            // Get tracks from the specified column and play from track_index
            if let Some(col) = state.playlist_nav.columns.get(column_index) {
                let tracks = helpers::collect_tracks_from_column(col);
                if !tracks.is_empty() {
                    if state.playback_mode == PlaybackMode::Radio {
                        state.radio.clear();
                    }
                    if let Some(qi) = state.queue_index {
                        if qi < state.queue.len() {
                            let played: Vec<crate::api::models::Track> = state.queue.drain(..=qi).collect();
                            state.play_history.extend(played);
                        }
                    }
                    audio.track_cache.flush();
                    state.queue.splice(0..0, tracks);
                    state.queue_index = Some(track_index);
                    state.playback_mode = PlaybackMode::Queue;
                    state.list_state.queue_index = state.play_history.len();
                    state.view = View::NowPlaying;
                    helpers::play_current_track(event_tx, state, client, audio).await;
                }
            }
        }

        Action::PlayPlaylistAlbumGroupTrack { track_index } => {
            // Build queue from all album groups flattened (in album order)
            let tracks: Vec<_> = state.playlist_album_groups.iter().flatten().cloned().collect();
            if !tracks.is_empty() && track_index < tracks.len() {
                if state.playback_mode == PlaybackMode::Radio {
                    state.radio.clear();
                }
                if let Some(qi) = state.queue_index {
                    if qi < state.queue.len() {
                        let played: Vec<crate::api::models::Track> = state.queue.drain(..=qi).collect();
                        state.play_history.extend(played);
                    }
                }
                audio.track_cache.flush();
                state.queue.splice(0..0, tracks);
                state.queue_index = Some(track_index);
                state.playback_mode = PlaybackMode::Queue;
                state.list_state.queue_index = state.play_history.len();
                state.view = View::NowPlaying;
                helpers::play_current_track(event_tx, state, client, audio).await;
            }
        }

        Action::LoadCompilationsForMiller => {
            // Push a new column with compilation albums
            let items = BrowseItem::from_albums(&state.compilation_albums);
            let col = BrowseColumn::new("Compilations", items);
            state.artist_nav.push_column(col);

            // Batch load album art for visible items
            let art_batch = collect_viewport_art(state);
            if !art_batch.is_empty() {
                return Ok(vec![Action::LoadAlbumArt(art_batch)]);
            }
        }

        Action::LoadCompilationTracksForMiller { artist_key, artist_name } => {
            // Load tracks from all compilation albums where this artist is the track artist
            let mut all_tracks = Vec::new();
            for album in &state.compilation_albums {
                match client.get_album_tracks(&album.rating_key).await {
                    Ok(tracks) => {
                        for track in tracks {
                            if track.grandparent_rating_key.as_deref() == Some(&artist_key)
                                || track.artist_name().to_lowercase() == artist_name.to_lowercase()
                            {
                                all_tracks.push(track);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!("Failed to load compilation tracks from {}: {}", album.rating_key, e);
                    }
                }
            }

            let items = BrowseItem::from_tracks(&all_tracks);
            let title = format!("Compilations: {}", artist_name);
            let col = BrowseColumn::new_with_tracks(title, items, all_tracks);
            state.artist_nav.push_column(col);
        }

        Action::RefreshAlbumTracks { album_key } => {
            // Refresh album tracks in the currently focused Miller column.
            // Works for both artist_nav and genre_nav.
            match client.get_album_tracks(&album_key).await {
                Ok(tracks) => {
                    let items = BrowseItem::from_tracks(&tracks);

                    // Determine which nav owns the focused track column
                    let nav = if state.browse_category == BrowseCategory::Genres {
                        &mut state.genre_nav
                    } else {
                        &mut state.artist_nav
                    };

                    if let Some(col) = nav.columns.get_mut(nav.focused_column) {
                        let old_idx = col.selected_index;
                        col.items = items;
                        col.tracks = tracks;
                        col.selected_index = old_idx.min(col.items.len().saturating_sub(1));
                    }
                    state.set_status("Album tracks refreshed".to_string());
                }
                Err(e) => {
                    state.set_error(format!("Failed to refresh album tracks: {}", e));
                }
            }
        }

        _ => unreachable!("dispatch_miller called with non-miller action: {:?}", action),
    }
    Ok(vec![])
}
