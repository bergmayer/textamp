//! Miller column dispatch handlers for all *ForMiller and *FromMiller actions.

use crate::app::{Action, AppState, Event};
use crate::app::state::{BrowseCategory, BrowseColumn, BrowseItem};
use crate::api::PlexClient;
use crate::api::models::Track;
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
            BrowseItem::Artist { key, thumb: Some(thumb), .. } => {
                if !cache.contains_key(key) && !pending.contains(key) {
                    to_load.push((key.clone(), thumb.clone()));
                }
            }
            BrowseItem::AllTracks { artist_key, thumb: Some(thumb), .. } => {
                if !cache.contains_key(artist_key) && !pending.contains(artist_key) {
                    to_load.push((artist_key.clone(), thumb.clone()));
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
    let nav = match state.browse_nav() {
        Some(n) => n,
        None => return vec![],
    };

    let Some(col) = nav.focused() else { return vec![] };
    // Only load art for columns with artwork_visible enabled
    if !col.artwork_visible { return vec![]; }

    let has_art_items = col.items.iter().any(|item| matches!(item, BrowseItem::Album { .. } | BrowseItem::Artist { .. } | BrowseItem::AllTracks { .. }));
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
            let auto_drill = std::mem::take(&mut state.auto_drill_pending);
            state.artist_nav.loading = true;

            // Check if this is a derived track-artist without a real Plex artist entry
            let is_plex_artist = state.artists.iter().any(|a| a.rating_key == artist_key);

            let albums_result = if is_plex_artist {
                // Real Plex artist: use API
                client.get_artist_albums(&artist_key).await
            } else {
                // Derived artist: build album list from all_tracks
                Ok(build_albums_from_tracks(&artist_key, &state.selected_artist_name, &state.all_tracks, &state.albums))
            };

            match albums_result {
                Ok(albums) => {
                    // Create special entries
                    let artist_thumb = state.artists.iter()
                        .find(|a| a.rating_key == artist_key)
                        .or_else(|| state.track_artists.iter().find(|a| a.rating_key == artist_key))
                        .and_then(|a| a.thumb.clone());
                    let artist_radio = BrowseItem::ArtistRadio {
                        artist_key: artist_key.clone(),
                        artist_name: state.selected_artist_name.clone(),
                        thumb: None, // No artwork for ArtistRadio
                    };
                    let all_tracks = BrowseItem::AllTracks {
                        artist_key: artist_key.clone(),
                        artist_name: state.selected_artist_name.clone(),
                        thumb: artist_thumb, // Artist artwork on AllTracks
                    };
                    // Order: ArtistRadio → CompilationTracks → AllTracks → Albums
                    let mut items = vec![artist_radio];

                    // CompilationTracks after ArtistRadio (if artist has compilation tracks)
                    if state.compilations_detected {
                        if state.artist_compilation_map.contains_key(&artist_key) {
                            items.push(BrowseItem::CompilationTracks {
                                artist_key: artist_key.clone(),
                                artist_name: state.selected_artist_name.clone(),
                            });
                        }
                    }

                    items.push(all_tracks);

                    // Start with albums, then merge any from preloaded state.albums
                    // that the API missed (e.g. compilation-subtype albums)
                    let mut all_albums = albums;
                    if is_plex_artist {
                        let api_keys: std::collections::HashSet<&str> = all_albums.iter()
                            .map(|a| a.rating_key.as_str())
                            .collect();
                        let missing: Vec<_> = state.albums.iter()
                            .filter(|a| a.parent_rating_key.as_deref() == Some(&*artist_key)
                                && !api_keys.contains(a.rating_key.as_str()))
                            .cloned()
                            .collect();
                        if !missing.is_empty() {
                            tracing::debug!("Merging {} albums from preload that API didn't return for artist {}", missing.len(), artist_key);
                            all_albums.extend(missing);
                        }
                    }

                    items.extend(BrowseItem::from_albums(&all_albums, &state.album_display_artist));

                    // Append single-artist compilations (albums where this artist
                    // is the sole performer but parent is a different artist)
                    if state.compilations_detected {
                        if let Some(solo_comps) = state.single_artist_compilations.get(&artist_key) {
                            let existing_keys: std::collections::HashSet<&str> = all_albums.iter()
                                .map(|a| a.rating_key.as_str())
                                .collect();
                            let new_comps: Vec<_> = solo_comps.iter()
                                .filter(|a| !existing_keys.contains(a.rating_key.as_str()))
                                .cloned()
                                .collect();
                            if !new_comps.is_empty() {
                                items.extend(BrowseItem::from_albums(&new_comps, &state.album_display_artist));
                            }
                        }
                    }

                    let title = format!("albums \u{2014} {}", state.selected_artist_name);
                    let mut col = BrowseColumn::new(title, items);
                    col.artwork_visible = state.default_artwork_visible;
                    state.artist_nav.drill_column(col, auto_drill);

                    // Preload all album art for the newly pushed column
                    let art_batch = if state.default_artwork_visible {
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
            let auto_drill = std::mem::take(&mut state.auto_drill_pending);
            state.artist_nav.loading = true;

            match client.get_album_tracks(&album_key).await {
                Ok(tracks) => {
                    let items = BrowseItem::from_tracks(&tracks);
                    let title = format!("tracks \u{2014} {}", state.selected_album_title);
                    // Store full tracks for playback (includes media info)
                    let col = BrowseColumn::new_with_tracks(title, items, tracks);
                    state.artist_nav.drill_column(col, auto_drill);

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
            let auto_drill = std::mem::take(&mut state.auto_drill_pending);
            state.artist_nav.loading = true;

            match client.get_artist_all_tracks(&artist_key).await {
                Ok(tracks) => {
                    let items = BrowseItem::from_tracks(&tracks);
                    let title = format!("tracks ({})", tracks.len());
                    // Store full tracks for playback (includes media info)
                    let col = BrowseColumn::new_with_tracks(title, items, tracks);
                    state.artist_nav.drill_column(col, auto_drill);
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
            let auto_drill = std::mem::take(&mut state.auto_drill_pending);
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
            let mut items = vec![BrowseItem::AllTracks {
                artist_key: "__all_library__".to_string(),
                artist_name: "All Artists".to_string(),
                thumb: None,
            }];
            items.extend(BrowseItem::from_albums(&state.albums, &state.album_display_artist));
            let mut col = BrowseColumn::new("all albums", items);
            col.artwork_visible = state.default_artwork_visible;
            state.artist_nav.drill_column(col, auto_drill);

            // Preload album art if in art view (viewport-limited)
            let art_batch = if state.default_artwork_visible {
                collect_art_to_load(state.artist_nav.columns.last(), &state.album_art_cache, &state.album_art_pending)
            } else {
                vec![]
            };
            state.artist_nav.loading = false;
            if !art_batch.is_empty() {
                return Ok(vec![Action::LoadAlbumArt(art_batch)]);
            }
        }

        Action::PlayTrackFromMiller { column_index, track_index, single_track } => {
            if let Some(col) = state.artist_nav.columns.get(column_index) {
                let tracks: Vec<Track> = if single_track {
                    col.tracks.get(track_index).cloned().into_iter().collect()
                } else {
                    // Shift+Enter: selected track + all following
                    col.tracks[track_index..].to_vec()
                };
                if !tracks.is_empty() {
                    helpers::queue_and_play(event_tx, state, client, audio, tracks, 0).await;
                }
            }
        }

        // Miller Column Actions for Genres View
        // ================================================================

        Action::LoadGenreAlbumsForMiller { genre_key } => {
            // Load albums for genre and add as new column in genre_nav
            let auto_drill = std::mem::take(&mut state.auto_drill_pending);
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
                        let items = BrowseItem::from_albums(&albums, &state.album_display_artist);
                        let genre_name = state.genre_nav.focused()
                            .and_then(|c| c.selected_item())
                            .map(|item| item.title().to_string())
                            .unwrap_or_default();
                        let title = if genre_name.is_empty() {
                            "albums".to_string()
                        } else {
                            format!("albums \u{2014} {}", genre_name)
                        };
                        let mut col = BrowseColumn::new(title, items);
                        col.artwork_visible = state.default_artwork_visible;
                        state.genre_nav.drill_column(col, auto_drill);

                        // Preload all album art for the newly pushed column
                        if state.default_artwork_visible {
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
            let auto_drill = std::mem::take(&mut state.auto_drill_pending);
            state.genre_nav.loading = true;

            // Get album name from the focused item for the column title
            let album_name = state.genre_nav.focused()
                .and_then(|c| c.selected_item())
                .map(|item| item.title().to_string())
                .unwrap_or_default();

            match client.get_album_tracks(&album_key).await {
                Ok(tracks) => {
                    let items = BrowseItem::from_tracks(&tracks);
                    let title = if album_name.is_empty() {
                        "tracks".to_string()
                    } else {
                        format!("tracks \u{2014} {}", album_name)
                    };
                    // Store full tracks for playback (includes media info)
                    let col = BrowseColumn::new_with_tracks(title, items, tracks);
                    state.genre_nav.drill_column(col, auto_drill);
                }
                Err(e) => {
                    state.set_error(format!("Failed to load tracks: {}", e));
                }
            }
            state.genre_nav.loading = false;
        }

        Action::PlayGenreTrackFromMiller { column_index, track_index, single_track } => {
            if let Some(col) = state.genre_nav.columns.get(column_index) {
                let tracks: Vec<Track> = if single_track {
                    col.tracks.get(track_index).cloned().into_iter().collect()
                } else {
                    col.tracks[track_index..].to_vec()
                };
                if !tracks.is_empty() {
                    helpers::queue_and_play(event_tx, state, client, audio, tracks, 0).await;
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

        Action::PlayPlaylistTrackFromMiller { column_index, track_index, single_track } => {
            if let Some(col) = state.playlist_nav.columns.get(column_index) {
                let tracks: Vec<Track> = if single_track {
                    col.tracks.get(track_index).cloned().into_iter().collect()
                } else {
                    col.tracks[track_index..].to_vec()
                };
                if !tracks.is_empty() {
                    helpers::queue_and_play(event_tx, state, client, audio, tracks, 0).await;
                }
            }
        }

        Action::LoadCompilationsForMiller => {
            // Push a new column with compilation albums, "All Tracks" pinned at top
            let auto_drill = std::mem::take(&mut state.auto_drill_pending);
            let mut items = vec![BrowseItem::AllTracks {
                artist_key: "__all_comp__".to_string(),
                artist_name: "Compilations".to_string(),
                thumb: None,
            }];
            items.extend(BrowseItem::from_albums(&state.compilation_albums, &state.album_display_artist));
            let mut col = BrowseColumn::new("compilations", items);
            col.artwork_visible = state.default_artwork_visible;
            state.artist_nav.drill_column(col, auto_drill);

            // Batch load album art for visible items
            let art_batch = collect_viewport_art(state);
            if !art_batch.is_empty() {
                return Ok(vec![Action::LoadAlbumArt(art_batch)]);
            }
        }

        Action::LoadCompilationAlbumsForMiller { artist_key, artist_name } => {
            // Show compilation albums for this artist, with "All Tracks" pinned at top
            let auto_drill = std::mem::take(&mut state.auto_drill_pending);
            if let Some(album_keys) = state.artist_compilation_map.get(&artist_key) {
                let album_keys_set: std::collections::HashSet<&str> = album_keys.iter().map(|s| s.as_str()).collect();
                let albums: Vec<_> = state.compilation_albums.iter()
                    .filter(|a| album_keys_set.contains(a.rating_key.as_str()))
                    .cloned()
                    .collect();

                let artist_thumb = state.artists.iter()
                    .find(|a| a.rating_key == artist_key)
                    .or_else(|| state.track_artists.iter().find(|a| a.rating_key == artist_key))
                    .and_then(|a| a.thumb.clone());

                let mut items = vec![BrowseItem::AllTracks {
                    artist_key: format!("__comp_tracks:{}", artist_key),
                    artist_name: artist_name.clone(),
                    thumb: artist_thumb,
                }];
                items.extend(BrowseItem::from_albums(&albums, &state.album_display_artist));

                let title = format!("compilations \u{2014} {}", artist_name);
                let mut col = BrowseColumn::new(title, items);
                col.artwork_visible = state.default_artwork_visible;
                state.artist_nav.drill_column(col, auto_drill);

                // Preload album art
                let art_batch = if state.default_artwork_visible {
                    collect_art_to_load(state.artist_nav.columns.last(), &state.album_art_cache, &state.album_art_pending)
                } else {
                    vec![]
                };
                if !art_batch.is_empty() {
                    return Ok(vec![Action::LoadAlbumArt(art_batch)]);
                }
            }
        }

        Action::LoadCompilationAllTracksForMiller { artist_key, artist_name: _ } => {
            // Load all tracks from this artist's compilation albums (all artists, not filtered)
            let auto_drill = std::mem::take(&mut state.auto_drill_pending);
            if let Some(album_keys) = state.artist_compilation_map.get(&artist_key) {
                let album_keys_set: std::collections::HashSet<&str> = album_keys.iter().map(|s| s.as_str()).collect();
                let tracks: Vec<_> = state.all_tracks.iter()
                    .filter(|t| t.parent_rating_key.as_deref()
                        .map_or(false, |pk| album_keys_set.contains(pk)))
                    .cloned()
                    .collect();
                let items = BrowseItem::from_tracks(&tracks);
                let title = format!("tracks ({})", tracks.len());
                let col = BrowseColumn::new_with_tracks(title, items, tracks);
                state.artist_nav.drill_column(col, auto_drill);
            }
        }

        Action::LoadAllCompilationTracksForMiller => {
            // Load all tracks from all compilation albums
            let auto_drill = std::mem::take(&mut state.auto_drill_pending);
            let album_keys_set: std::collections::HashSet<&str> = state.compilation_albums.iter()
                .map(|a| a.rating_key.as_str())
                .collect();
            let tracks: Vec<_> = state.all_tracks.iter()
                .filter(|t| t.parent_rating_key.as_deref()
                    .map_or(false, |pk| album_keys_set.contains(pk)))
                .cloned()
                .collect();
            let items = BrowseItem::from_tracks(&tracks);
            let title = format!("tracks ({})", tracks.len());
            let col = BrowseColumn::new_with_tracks(title, items, tracks);
            state.artist_nav.drill_column(col, auto_drill);
        }

        Action::LoadAllLibraryTracksForMiller => {
            // Load all library tracks into a Miller column (from "All Tracks" in All Artists)
            let auto_drill = std::mem::take(&mut state.auto_drill_pending);
            if state.all_tracks.is_empty() {
                // Push an empty placeholder column; AllTracksPreloaded will fill it
                let col = BrowseColumn::new("tracks (loading...)", vec![]);
                state.artist_nav.drill_column(col, auto_drill);
                // Trigger preload if not already in progress
                if let Some(ref lib_key) = state.active_library.clone() {
                    state.preloads_in_progress.insert("Tracks".to_string());
                    if state.preloads_total == 0 { state.preloads_total = 1; }
                    helpers::preload_data(event_tx, crate::app::event_loop::PreloadType::AllTracks, lib_key, client);
                }
                return Ok(vec![]);
            }
            let items = BrowseItem::from_tracks(&state.all_tracks);
            let title = format!("tracks ({})", state.all_tracks.len());
            let col = BrowseColumn::new_with_tracks(title, items, state.all_tracks.clone());
            state.artist_nav.drill_column(col, auto_drill);
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

/// Build album list for a derived track-artist from the all_tracks cache.
///
/// Groups tracks by parent_rating_key (album key) where the track's artist name
/// matches, then looks up Album metadata from state.albums.
fn build_albums_from_tracks(
    artist_key: &str,
    artist_name: &str,
    all_tracks: &[crate::plex::models::Track],
    albums: &[crate::plex::models::Album],
) -> Vec<crate::plex::models::Album> {
    use std::collections::HashSet;

    let artist_lower = artist_name.to_lowercase();

    // Find album keys where this artist has tracks
    let mut album_keys: HashSet<String> = HashSet::new();
    for track in all_tracks {
        let track_artist = track.original_title.as_deref()
            .unwrap_or_else(|| track.artist_name());
        if track_artist.to_lowercase() == artist_lower {
            if let Some(ref key) = track.parent_rating_key {
                album_keys.insert(key.clone());
            }
        }
        // Also match by grandparent_rating_key for Plex album-artist matches
        if track.grandparent_rating_key.as_deref() == Some(artist_key) {
            if let Some(ref key) = track.parent_rating_key {
                album_keys.insert(key.clone());
            }
        }
    }

    // Look up album metadata
    let mut result: Vec<crate::plex::models::Album> = albums.iter()
        .filter(|a| album_keys.contains(&a.rating_key))
        .cloned()
        .collect();

    // Sort by year then title
    result.sort_by(|a, b| {
        a.year.cmp(&b.year)
            .then_with(|| super::helpers::sort_key(&a.title).cmp(&super::helpers::sort_key(&b.title)))
    });

    result
}
