//! Miller column dispatch handlers for all *ForMiller and *FromMiller actions.

use crate::app::event::*;
use crate::app::event::LibraryEventSender;
use crate::app::{Action, AppState, Event};
use crate::app::action::{AsyncError, MillerAction, SystemAction};
use crate::app::state::{BrowseColumn, BrowseItem};
use crate::plex::PlexClient;
use crate::plex::models::Track;
use crate::audio::AudioPlayer;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Collect tracks from a column for playback (shared by Play*TrackFromMiller handlers).
fn collect_tracks_from_column(col: &BrowseColumn, track_index: usize, single_track: bool) -> Vec<Track> {
    if single_track {
        col.tracks.get(track_index).cloned().into_iter().collect()
    } else {
        col.tracks[track_index..].to_vec()
    }
}

/// Max number of album art entries to load at once to avoid blocking the event loop
/// with synchronous disk I/O in LoadAlbumArt.
const ART_BATCH_LIMIT: usize = 30;

/// Collect album art (key, thumb) pairs from a column that aren't already cached or pending.
/// Limited to `ART_BATCH_LIMIT` items around the column's selected_index to avoid
/// blocking the event loop with thousands of synchronous disk reads.
pub(crate) fn collect_art_to_load(
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
            BrowseItem::AllTracks { scope, thumb: Some(thumb) } => {
                if let Some(artist_key) = scope.artist_key() {
                    if !cache.contains_key(artist_key) && !pending.contains(artist_key) {
                        to_load.push((artist_key.to_string(), thumb.clone()));
                    }
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

    collect_art_to_load(Some(col), &state.artwork.grid_cache, &state.artwork.grid_pending)
}

/// Collect art for EVERY row of `col`, ignoring the
/// `ART_BATCH_LIMIT` viewport window. Used when the user explicitly
/// enables artwork on a long column (a 5,000-track playlist's
/// album-grouping view should fill in cover art across the whole
/// list, not just rows near the cursor). The result still respects
/// the `cache` / `pending` dedup so already-loaded keys aren't
/// requested again.
pub(crate) fn collect_all_art_to_load(
    col: Option<&BrowseColumn>,
    cache: &std::collections::HashMap<String, Vec<u8>>,
    pending: &std::collections::HashSet<String>,
) -> Vec<(String, String)> {
    let Some(col) = col else { return vec![] };
    let mut to_load = Vec::with_capacity(col.items.len());
    for item in &col.items {
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
            BrowseItem::AllTracks { scope, thumb: Some(thumb) } => {
                if let Some(artist_key) = scope.artist_key() {
                    if !cache.contains_key(artist_key) && !pending.contains(artist_key) {
                        to_load.push((artist_key.to_string(), thumb.clone()));
                    }
                }
            }
            _ => {}
        }
    }
    to_load
}

/// Dispatch Miller column actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: MillerAction,
    state: &mut AppState,
    client: &mut PlexClient,
    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    match action {
        // ================================================================
        // Miller Column Actions for Artists View
        // ================================================================

        MillerAction::LoadArtistAlbumsForMiller { artist_key, replace_child } => {
            // Load albums for artist and add as new column in artist_nav
            // Prepend "All Tracks" entry before albums (same as old render path)
            state.artist_nav.loading = true;
            state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
            let request_id = state.artist_nav_request_id;

            // Sync `selected_artist_name` from the cached artist roster
            // before any column titles are formatted. Some callers (the
            // GUI's miller-click handler, in particular) dispatch this
            // action without setting it themselves, which would leave
            // the album column header reading "albums — " (no artist).
            // The OpenInLibrary path sets it pre-dispatch and is
            // therefore unaffected by this fallback.
            if let Some(name) = state.library.artists.iter()
                .find(|a| a.rating_key == artist_key)
                .or_else(|| state.library.track_artists.iter().find(|a| a.rating_key == artist_key))
                .map(|a| a.title.clone())
            {
                if !name.is_empty() {
                    state.library.selected_artist_name = name;
                }
            }

            // Check if this is a derived track-artist without a real Plex artist entry
            let is_plex_artist = state.library.artists.iter().any(|a| a.rating_key == artist_key);

            if !is_plex_artist {
                let albums = build_albums_from_tracks(
                    &artist_key,
                    &state.library.selected_artist_name,
                    &state.library.all_tracks,
                    &state.library.albums,
                );
                return Ok(vec![MillerAction::ArtistAlbumsForMillerLoaded {
                    request_id,
                    artist_key,
                    replace_child,
                    is_plex_artist,
                    result: Ok(albums),
                }
                .into()]);
            }

            let tx = LibraryEventSender::new(event_tx.clone(), state.library_generation);
            let request_client = client.clone();
            tokio::spawn(async move {
                let result = request_client
                    .get_artist_albums(&artist_key)
                    .await
                    .map_err(|error| AsyncError::from_api("Failed to load albums", &error));
                let _ = tx
                    .send(Event::Effect(
                        MillerAction::ArtistAlbumsForMillerLoaded {
                            request_id,
                            artist_key,
                            replace_child,
                            is_plex_artist,
                            result,
                        }
                        .into(),
                    ))
                    .await;
            });
        }

        MillerAction::ArtistAlbumsForMillerLoaded {
            request_id,
            artist_key,
            replace_child,
            is_plex_artist,
            result,
        } => {
            if state.artist_nav_request_id != request_id {
                return Ok(vec![]);
            }
            match result {
                Ok(albums) => {
                    if is_plex_artist {
                        state.connection.mark_healthy();
                    }
                    // Albums first, then pinned action rows at the
                    // bottom. Order: Albums → ArtistRadio → AllTracks
                    // → CompilationTracks. Action rows carry no thumb
                    // since they aren't albums.
                    let artist_radio = BrowseItem::ArtistRadio {
                        artist_key: artist_key.clone(),
                        artist_name: state.library.selected_artist_name.clone(),
                        thumb: None,
                    };
                    let all_tracks = BrowseItem::AllTracks {
                        scope: crate::app::state::AllTracksScope::Artist {
                            artist_key: artist_key.clone(),
                            artist_name: state.library.selected_artist_name.clone(),
                        },
                        thumb: None,
                    };

                    // Start with albums, then merge any from preloaded state.library.albums
                    // that the API missed (e.g. compilation-subtype albums)
                    let mut all_albums = albums;
                    if is_plex_artist {
                        let api_keys: std::collections::HashSet<&str> = all_albums.iter()
                            .map(|a| a.rating_key.as_str())
                            .collect();
                        let missing: Vec<_> = state.library.albums.iter()
                            .filter(|a| a.parent_rating_key.as_deref() == Some(&*artist_key)
                                && !api_keys.contains(a.rating_key.as_str()))
                            .cloned()
                            .collect();
                        if !missing.is_empty() {
                            tracing::debug!("Merging {} albums from preload that API didn't return for artist {}", missing.len(), artist_key);
                            all_albums.extend(missing);
                        }
                    }

                    let mut items: Vec<BrowseItem> = BrowseItem::from_albums(&all_albums, &state.library.album_display_artist);

                    // Append single-artist compilations (albums where this artist
                    // is the sole performer but parent is a different artist)
                    if state.library.compilations.detected {
                        if let Some(solo_comps) = state.library.compilations.single_artist.get(&artist_key) {
                            let existing_keys: std::collections::HashSet<&str> = all_albums.iter()
                                .map(|a| a.rating_key.as_str())
                                .collect();
                            let new_comps: Vec<_> = solo_comps.iter()
                                .filter(|a| !existing_keys.contains(a.rating_key.as_str()))
                                .cloned()
                                .collect();
                            if !new_comps.is_empty() {
                                items.extend(BrowseItem::from_albums(&new_comps, &state.library.album_display_artist));
                            }
                        }
                    }

                    // Pinned action rows go at the bottom, after all
                    // album rows.
                    items.push(artist_radio);
                    items.push(all_tracks);
                    if state.library.compilations.detected
                        && state.library.compilations.artist_map.contains_key(&artist_key)
                    {
                        items.push(BrowseItem::CompilationTracks {
                            artist_key: artist_key.clone(),
                            artist_name: state.library.selected_artist_name.clone(),
                        });
                    }

                    let title = format!("albums \u{2014} {}", state.library.selected_artist_name);
                    let mut col = BrowseColumn::new(title, items);
                    col.artwork_visible = state.artwork.default_visible;
                    state.artist_nav.drill_column(col, replace_child);

                    // Preload all album art for the newly pushed column
                    let art_batch = if state.artwork.default_visible {
                        collect_art_to_load(state.artist_nav.columns.last(), &state.artwork.grid_cache, &state.artwork.grid_pending)
                    } else {
                        vec![]
                    };

                    // Auto-select album and drill into tracks if pending_album_key is set (Alt+B)
                    if let Some(album_key) = state.search.pending_album_key.take() {
                        if let Some(col) = state.artist_nav.columns.last_mut() {
                            if let Some(idx) = col.items.iter().position(|item| {
                                matches!(item, BrowseItem::Album { key, .. } if *key == album_key)
                            }) {
                                col.selected_index = idx;
                                if let Some(BrowseItem::Album { title, .. }) = col.items.get(idx) {
                                    state.library.selected_album_title = title.clone();
                                }
                                let mut actions: Vec<Action> = vec![MillerAction::LoadAlbumTracksForMiller { album_key, replace_child: false }.into()];
                                if !art_batch.is_empty() {
                                    actions.push(SystemAction::LoadAlbumArt(art_batch).into());
                                }
                                state.artist_nav.loading = false;
                                return Ok(actions);
                            }
                        }
                    }

                    if !art_batch.is_empty() {
                        state.artist_nav.loading = false;
                        return Ok(vec![SystemAction::LoadAlbumArt(art_batch).into()]);
                    }
                }
                Err(error) => {
                    if error.connection_error {
                        state.connection.mark_degraded(error.message.clone());
                    }
                    state.set_error(error.message);
                }
            }
            state.artist_nav.loading = false;
        }

        MillerAction::LoadAlbumTracksForMiller { album_key, replace_child } => {
            // Load tracks for album and add as new column in artist_nav
            state.artist_nav.loading = true;
            state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
            let request_id = state.artist_nav_request_id;
            let album_title = state.library.selected_album_title.clone();
            let tx = LibraryEventSender::new(event_tx.clone(), state.library_generation);
            let request_client = client.clone();
            tokio::spawn(async move {
                let result = request_client
                    .get_album_tracks(&album_key)
                    .await
                    .map_err(|error| AsyncError::from_api("Failed to load tracks", &error));
                let _ = tx
                    .send(Event::Effect(
                        MillerAction::AlbumTracksForMillerLoaded {
                            request_id,
                            album_key,
                            album_title,
                            replace_child,
                            result,
                        }
                        .into(),
                    ))
                    .await;
            });
        }

        MillerAction::AlbumTracksForMillerLoaded {
            request_id,
            album_key,
            album_title,
            replace_child,
            result,
        } => {
            if state.artist_nav_request_id != request_id {
                return Ok(vec![]);
            }
            match result {
                Ok(tracks) => {
                    state.connection.mark_healthy();
                    let items = BrowseItem::from_tracks(&tracks);
                    let title = format!("tracks \u{2014} {}", album_title);
                    // Store full tracks for playback (includes media info)
                    let mut col = BrowseColumn::new_with_tracks(title, items, tracks);
                    col.play_all_row = Some(crate::app::state::PlayAllRow::Album {
                        rating_key: album_key.clone(),
                        title: album_title,
                    });
                    col.on_play_row = true;
                    state.artist_nav.drill_column(col, replace_child);

                    // Auto-select track if pending from search navigation
                    if let Some(ref tk) = state.search.pending_track_key {
                        if let Some(col) = state.artist_nav.columns.last_mut() {
                            if let Some(pos) = col.items.iter().position(|i| i.key() == tk.as_str()) {
                                col.selected_index = pos;
                            }
                        }
                        state.search.pending_track_key = None;
                    }
                }
                Err(error) => {
                    if error.connection_error {
                        state.connection.mark_degraded(error.message.clone());
                    }
                    state.set_error(error.message);
                }
            }
            state.artist_nav.loading = false;
        }

        MillerAction::LoadArtistAllTracksForMiller { artist_key, replace_child } => {
            // Load all tracks by an artist and add as new column in artist_nav
            // This is triggered by selecting "All Tracks" entry in the albums column
            state.artist_nav.loading = true;
            state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
            let request_id = state.artist_nav_request_id;
            let tx = LibraryEventSender::new(event_tx.clone(), state.library_generation);
            let request_client = client.clone();
            tokio::spawn(async move {
                let result = request_client
                    .get_artist_all_tracks(&artist_key)
                    .await
                    .map_err(|error| AsyncError::from_api("Failed to load tracks", &error));
                let _ = tx
                    .send(Event::Effect(
                        MillerAction::ArtistAllTracksForMillerLoaded {
                            request_id,
                            replace_child,
                            result,
                        }
                        .into(),
                    ))
                    .await;
            });
        }

        MillerAction::ArtistAllTracksForMillerLoaded {
            request_id,
            replace_child,
            result,
        } => {
            if state.artist_nav_request_id != request_id {
                return Ok(vec![]);
            }
            match result {
                Ok(tracks) => {
                    state.connection.mark_healthy();
                    let items = BrowseItem::from_tracks(&tracks);
                    let title = format!("tracks ({})", tracks.len());
                    // Store full tracks for playback (includes media info).
                    let mut col = BrowseColumn::new_with_tracks(title, items, tracks);
                    col.play_all_row = Some(crate::app::state::PlayAllRow::AllTracks {
                        label: "Play all tracks".to_string(),
                    });
                    col.on_play_row = true;
                    state.artist_nav.drill_column(col, replace_child);
                }
                Err(error) => {
                    if error.connection_error {
                        state.connection.mark_degraded(error.message.clone());
                    }
                    state.set_error(error.message);
                }
            }
            state.artist_nav.loading = false;
        }

        MillerAction::LoadAllAlbumsForMiller { replace_child } => {
            // Load all albums as a Miller column (triggered by "► All Artists" entry)
            // Uses already-loaded state.library.albums; fetches async if empty.
            let auto_drill = replace_child;
            if state.library.albums.is_empty() {
                state.artist_nav.loading = true;
                state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
                let request_id = state.artist_nav_request_id;
                // Fetch in background to avoid blocking the event loop
                let tx = LibraryEventSender::new(event_tx.clone(), state.library_generation);
                let client_clone = client.clone();
                let lib_key = state.active_library.clone().unwrap_or_default();
                tokio::spawn(async move {
                    match client_clone.get_albums(&lib_key).await {
                        Ok(albums) => {
                            let _ = tx.send(DataEvent::AllAlbumsForMillerLoaded {
                                library_key: lib_key,
                                request_id,
                                replace_child,
                                albums,
                            }.into()).await;
                        }
                        Err(e) => {
                            let _ = tx.send(DataEvent::AllAlbumsForMillerFailed {
                                library_key: lib_key,
                                request_id,
                                error: AsyncError::from_api("Failed to load albums", &e),
                            }.into()).await;
                        }
                    }
                });
                return Ok(vec![]);
            }
            let mut items: Vec<BrowseItem> = BrowseItem::from_albums(&state.library.albums, &state.library.album_display_artist);
            items.push(BrowseItem::AllTracks {
                scope: crate::app::state::AllTracksScope::Library,
                thumb: None,
            });
            let mut col = BrowseColumn::new("all albums", items);
            col.artwork_visible = state.artwork.default_visible;
            state.artist_nav.drill_column(col, auto_drill);

            // Preload album art if in art view (viewport-limited)
            let art_batch = if state.artwork.default_visible {
                collect_art_to_load(state.artist_nav.columns.last(), &state.artwork.grid_cache, &state.artwork.grid_pending)
            } else {
                vec![]
            };
            state.artist_nav.loading = false;
            if !art_batch.is_empty() {
                return Ok(vec![SystemAction::LoadAlbumArt(art_batch).into()]);
            }
        }

        MillerAction::PlayTrackFromMiller { column_index, track_index, single_track } => {
            if let Some(col) = state.artist_nav.columns.get(column_index) {
                let tracks = collect_tracks_from_column(col, track_index, single_track);
                if !tracks.is_empty() {
                    helpers::queue_and_play(event_tx, state, client, audio, tracks, 0);
                }
            }
        }
        // Miller Column Actions for Genres View
        // ================================================================

        MillerAction::LoadGenreAlbumsForMiller { genre_key, replace_child } => {
            // Load albums for the selected tag in the active section,
            // and push a new column into tag_nav.
            state.tag_nav.loading = true;
            state.tag_nav_request_id = state.tag_nav_request_id.wrapping_add(1);
            let request_id = state.tag_nav_request_id;

            if let Some(lib_key) = &state.active_library.clone() {
                use crate::app::state::BrowseCategory;
                let section = state.browse_category;
                let genre_name = state
                    .tag_nav
                    .focused()
                    .and_then(|column| column.selected_item())
                    .map(|item| item.title().to_string())
                    .unwrap_or_default();
                let library_key = lib_key.clone();
                let tx = LibraryEventSender::new(event_tx.clone(), state.library_generation);
                let request_client = client.clone();
                tokio::spawn(async move {
                    let result = match section {
                        BrowseCategory::AlbumGenres | BrowseCategory::ArtistGenres => {
                            request_client.get_genre_albums(&library_key, &genre_key).await
                        }
                        BrowseCategory::Moods => request_client.get_mood_albums(&library_key, &genre_key).await,
                        BrowseCategory::Styles => request_client.get_style_albums(&library_key, &genre_key).await,
                        BrowseCategory::Decades => request_client.get_decade_albums(&library_key, &genre_key).await,
                        BrowseCategory::Years => request_client.get_year_albums(&library_key, &genre_key).await,
                        BrowseCategory::Collections => request_client.get_collection_albums(&library_key, &genre_key).await,
                        BrowseCategory::Countries => request_client.get_country_albums(&library_key, &genre_key).await,
                        BrowseCategory::Labels => request_client.get_label_albums(&library_key, &genre_key).await,
                        BrowseCategory::Formats => request_client.get_format_albums(&library_key, &genre_key).await,
                        BrowseCategory::Studios => request_client.get_studio_albums(&library_key, &genre_key).await,
                        _ => request_client.get_genre_albums(&library_key, &genre_key).await,
                    }
                    .map_err(|error| AsyncError::from_api("Failed to load albums", &error));
                    let _ = tx
                        .send(Event::Effect(
                            MillerAction::GenreAlbumsForMillerLoaded {
                                request_id,
                                genre_name,
                                replace_child,
                                result,
                            }
                            .into(),
                        ))
                        .await;
                });
            } else {
                state.tag_nav.loading = false;
            }
        }

        MillerAction::GenreAlbumsForMillerLoaded {
            request_id,
            genre_name,
            replace_child,
            result,
        } => {
            if state.tag_nav_request_id != request_id {
                return Ok(vec![]);
            }
            match result {
                    Ok(albums) => {
                        state.connection.mark_healthy();
                        let items = BrowseItem::from_albums(&albums, &state.library.album_display_artist);
                        let title = if genre_name.is_empty() {
                            "albums".to_string()
                        } else {
                            format!("albums \u{2014} {}", genre_name)
                        };
                        let mut col = BrowseColumn::new(title, items);
                        col.artwork_visible = state.artwork.default_visible;
                        state.tag_nav.drill_column(col, replace_child);

                        // Preload all album art for the newly pushed column
                        if state.artwork.default_visible {
                            let art_batch = collect_art_to_load(state.tag_nav.columns.last(), &state.artwork.grid_cache, &state.artwork.grid_pending);
                            if !art_batch.is_empty() {
                                state.tag_nav.loading = false;
                                return Ok(vec![SystemAction::LoadAlbumArt(art_batch).into()]);
                            }
                        }
                    }
                    Err(error) => {
                        if error.connection_error {
                            state.connection.mark_degraded(error.message.clone());
                        }
                        state.set_error(error.message);
                    }
                }
            state.tag_nav.loading = false;
        }

        MillerAction::LoadGenreTracksForMiller { album_key, replace_child } => {
            // Load tracks for album and add as new column in genre_nav
            state.tag_nav.loading = true;
            state.tag_nav_request_id = state.tag_nav_request_id.wrapping_add(1);
            let request_id = state.tag_nav_request_id;

            // Get album name from the focused item for the column title
            let album_name = state.tag_nav.focused()
                .and_then(|c| c.selected_item())
                .map(|item| item.title().to_string())
                .unwrap_or_default();

            let tx = LibraryEventSender::new(event_tx.clone(), state.library_generation);
            let request_client = client.clone();
            tokio::spawn(async move {
                let result = request_client
                    .get_album_tracks(&album_key)
                    .await
                    .map_err(|error| AsyncError::from_api("Failed to load tracks", &error));
                let _ = tx
                    .send(Event::Effect(
                        MillerAction::GenreTracksForMillerLoaded {
                            request_id,
                            album_key,
                            album_name,
                            replace_child,
                            result,
                        }
                        .into(),
                    ))
                    .await;
            });
        }

        MillerAction::GenreTracksForMillerLoaded {
            request_id,
            album_key,
            album_name,
            replace_child,
            result,
        } => {
            if state.tag_nav_request_id != request_id {
                return Ok(vec![]);
            }
            match result {
                Ok(tracks) => {
                    state.connection.mark_healthy();
                    let items = BrowseItem::from_tracks(&tracks);
                    let title = if album_name.is_empty() {
                        "tracks".to_string()
                    } else {
                        format!("tracks \u{2014} {}", album_name)
                    };
                    // Store full tracks for playback (includes media info)
                    let mut col = BrowseColumn::new_with_tracks(title, items, tracks);
                    col.play_all_row = Some(crate::app::state::PlayAllRow::Album {
                        rating_key: album_key.clone(),
                        title: album_name.clone(),
                    });
                    col.on_play_row = true;
                    state.tag_nav.drill_column(col, replace_child);
                }
                Err(error) => {
                    if error.connection_error {
                        state.connection.mark_degraded(error.message.clone());
                    }
                    state.set_error(error.message);
                }
            }
            state.tag_nav.loading = false;
        }

        MillerAction::PlayGenreTrackFromMiller { column_index, track_index, single_track } => {
            if let Some(col) = state.tag_nav.columns.get(column_index) {
                let tracks = collect_tracks_from_column(col, track_index, single_track);
                if !tracks.is_empty() {
                    helpers::queue_and_play(event_tx, state, client, audio, tracks, 0);
                }
            }
        }
        // Miller Column Actions for Playlists View
        // ================================================================

        MillerAction::LoadPlaylistTracksForMiller { playlist_key, replace_child: _ } => {
            // Fetch only the first page; the GUI then triggers
            // `LoadMorePlaylistTracks` as the user scrolls. This keeps
            // the initial open of giant smart playlists ("Recently
            // Added" can be 70k+ tracks) responsive — without it
            // Plex drops the connection mid-stream and the playlist
            // never appears at all.
            //
            // `replace_child` is accepted but ignored: the playlist
            // arrival event always anchors the new tracks column at
            // index 1 of the playlist nav and pushes (matches the
            // pre-refactor behaviour, which forcibly cleared the
            // auto-drill flag on the same arrival event).
            const FIRST_PAGE: u32 = 500;
            if state.playlist_nav.columns.len() <= state.playlist_nav.focused_column + 1 {
                state.playlist_nav.loading = true;
            }
            state.playlist_nav_request_id = state.playlist_nav_request_id.wrapping_add(1);
            let request_id = state.playlist_nav_request_id;
            let tx = LibraryEventSender::new(event_tx.clone(), state.library_generation);
            let client_clone = client.clone();
            let library_key = state.active_library.clone().unwrap_or_default();
            let pk = playlist_key.clone();
            tokio::spawn(async move {
                let backoff = [1u64, 2, 4];
                let mut last_error = None;
                for attempt in 0..3u32 {
                    match client_clone.get_playlist_tracks_page(&pk, 0, FIRST_PAGE).await {
                        Ok((tracks, total)) => {
                            let _ = tx.send(RadioEvent::PlaylistFirstPageLoaded {
                                library_key,
                                request_id,
                                playlist_key: pk,
                                tracks,
                                total,
                            }.into()).await;
                            return;
                        }
                        Err(error) => {
                            let retry = error.is_connection_error() && attempt < 2;
                            let async_error = AsyncError::from_api(
                                "Failed to load playlist",
                                &error,
                            );
                            tracing::debug!(
                                "Playlist load attempt {} failed: {}",
                                attempt + 1,
                                async_error.message,
                            );
                            last_error = Some(async_error);
                            if retry {
                                let delay = backoff[attempt as usize];
                                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                            } else {
                                break;
                            }
                        }
                    }
                }
                let _ = tx.send(RadioEvent::PlaylistTracksForMillerFailed {
                    library_key,
                    request_id,
                    playlist_key: pk,
                    error: last_error.unwrap_or(AsyncError {
                        message: "Failed to load playlist".to_string(),
                        connection_error: false,
                    }),
                }.into()).await;
            });
        }

        MillerAction::LoadMorePlaylistTracks { playlist_key, offset } => {
            const PAGE: u32 = 500;
            // Mark the column as loading so the scroll handler doesn't
            // fire a duplicate request while this one's in flight.
            if let Some(lazy) = state.playlist_nav.columns.iter_mut()
                .filter_map(|c| c.lazy.as_mut())
                .find(|l| l.key == *playlist_key)
            {
                lazy.loading = true;
            }
            let tx = LibraryEventSender::new(event_tx.clone(), state.library_generation);
            let client_clone = client.clone();
            let library_key = state.active_library.clone().unwrap_or_default();
            let pk = playlist_key.clone();
            tokio::spawn(async move {
                match client_clone.get_playlist_tracks_page(&pk, offset, PAGE).await {
                    Ok((tracks, total)) => {
                        let _ = tx.send(RadioEvent::PlaylistMorePageLoaded {
                            library_key,
                            playlist_key: pk,
                            offset,
                            tracks,
                            total,
                        }.into()).await;
                    }
                    Err(e) => {
                        let _ = tx.send(RadioEvent::PlaylistMorePageFailed {
                            library_key,
                            playlist_key: pk,
                            offset,
                            error: AsyncError::from_api("Playlist page fetch failed", &e),
                        }.into()).await;
                    }
                }
            });
        }

        MillerAction::PlayPlaylistTrackFromMiller { column_index, track_index, single_track } => {
            if let Some(col) = state.playlist_nav.columns.get(column_index) {
                let tracks = collect_tracks_from_column(col, track_index, single_track);
                if !tracks.is_empty() {
                    helpers::queue_and_play(event_tx, state, client, audio, tracks, 0);
                }
            }
        }
        MillerAction::LoadCompilationsForMiller { replace_child } => {
            // Push a new column with compilation albums; "All Tracks"
            // sits at the bottom as a pinned action row.
            let auto_drill = replace_child;
            let mut items: Vec<BrowseItem> = BrowseItem::from_albums(&state.library.compilations.albums, &state.library.album_display_artist);
            items.push(BrowseItem::AllTracks {
                scope: crate::app::state::AllTracksScope::AllCompilations,
                thumb: None,
            });
            let mut col = BrowseColumn::new("compilations", items);
            col.artwork_visible = state.artwork.default_visible;
            state.artist_nav.drill_column(col, auto_drill);

            // Batch load album art for visible items
            let art_batch = collect_viewport_art(state);
            if !art_batch.is_empty() {
                return Ok(vec![SystemAction::LoadAlbumArt(art_batch).into()]);
            }
        }

        MillerAction::LoadCompilationAlbumsForMiller { artist_key, artist_name, replace_child } => {
            // Show compilation albums for this artist, with "All Tracks" pinned at top
            let auto_drill = replace_child;
            if let Some(album_keys) = state.library.compilations.artist_map.get(&artist_key) {
                let album_keys_set: std::collections::HashSet<&str> = album_keys.iter().map(|s| s.as_str()).collect();
                let albums: Vec<_> = state.library.compilations.albums.iter()
                    .filter(|a| album_keys_set.contains(a.rating_key.as_str()))
                    .cloned()
                    .collect();

                let artist_thumb = state.library.artists.iter()
                    .find(|a| a.rating_key == artist_key)
                    .or_else(|| state.library.track_artists.iter().find(|a| a.rating_key == artist_key))
                    .and_then(|a| a.thumb.clone());

                let mut items: Vec<BrowseItem> = BrowseItem::from_albums(&albums, &state.library.album_display_artist);
                items.push(BrowseItem::AllTracks {
                    scope: crate::app::state::AllTracksScope::CompilationsByArtist {
                        artist_key: artist_key.clone(),
                        artist_name: artist_name.clone(),
                    },
                    thumb: artist_thumb,
                });

                let title = format!("compilations \u{2014} {}", artist_name);
                let mut col = BrowseColumn::new(title, items);
                col.artwork_visible = state.artwork.default_visible;
                state.artist_nav.drill_column(col, auto_drill);

                // Preload album art
                let art_batch = if state.artwork.default_visible {
                    collect_art_to_load(state.artist_nav.columns.last(), &state.artwork.grid_cache, &state.artwork.grid_pending)
                } else {
                    vec![]
                };
                if !art_batch.is_empty() {
                    return Ok(vec![SystemAction::LoadAlbumArt(art_batch).into()]);
                }
            }
        }

        MillerAction::LoadCompilationAllTracksForMiller { artist_key, artist_name: _, replace_child } => {
            // Load all tracks from this artist's compilation albums (all artists, not filtered)
            let auto_drill = replace_child;
            if let Some(album_keys) = state.library.compilations.artist_map.get(&artist_key) {
                let album_keys_set: std::collections::HashSet<&str> = album_keys.iter().map(|s| s.as_str()).collect();
                let tracks: Vec<_> = state.library.all_tracks.iter()
                    .filter(|t| t.parent_rating_key.as_deref()
                        .map_or(false, |pk| album_keys_set.contains(pk)))
                    .cloned()
                    .collect();
                let items = BrowseItem::from_tracks(&tracks);
                let title = format!("tracks ({})", tracks.len());
                let mut col = BrowseColumn::new_with_tracks(title, items, tracks);
                col.play_all_row = Some(crate::app::state::PlayAllRow::AllTracks {
                    label: "Play compilation tracks".to_string(),
                });
                col.on_play_row = true;
                state.artist_nav.drill_column(col, auto_drill);
            }
        }

        MillerAction::LoadAllCompilationTracksForMiller { replace_child } => {
            // Load all tracks from all compilation albums
            let auto_drill = replace_child;
            let album_keys_set: std::collections::HashSet<&str> = state.library.compilations.albums.iter()
                .map(|a| a.rating_key.as_str())
                .collect();
            let tracks: Vec<_> = state.library.all_tracks.iter()
                .filter(|t| t.parent_rating_key.as_deref()
                    .map_or(false, |pk| album_keys_set.contains(pk)))
                .cloned()
                .collect();
            let items = BrowseItem::from_tracks(&tracks);
            let title = format!("tracks ({})", tracks.len());
            let mut col = BrowseColumn::new_with_tracks(title, items, tracks);
            col.play_all_row = Some(crate::app::state::PlayAllRow::AllTracks {
                label: "Play all compilation tracks".to_string(),
            });
            col.on_play_row = true;
            state.artist_nav.drill_column(col, auto_drill);
        }

        MillerAction::LoadAllLibraryTracksForMiller { replace_child } => {
            // Load all library tracks into a Miller column (from "All Tracks" in All Artists)
            let auto_drill = replace_child;
            if state.library.all_tracks.is_empty() {
                // Push an empty placeholder column; AllTracksPreloaded will fill it
                let col = BrowseColumn::new("tracks (loading...)", vec![]);
                state.artist_nav.drill_column(col, auto_drill);
                // Trigger preload if not already in progress
                if let Some(ref lib_key) = state.active_library.clone() {
                    state.cache_mgmt.preloads_in_progress.insert("Tracks".to_string());
                    if state.cache_mgmt.preloads_total == 0 { state.cache_mgmt.preloads_total = 1; }
                    helpers::preload_data(
                        event_tx,
                        helpers::PreloadType::AllTracks,
                        lib_key,
                        client,
                        state.library_generation,
                    );
                }
                return Ok(vec![]);
            }
            let items = BrowseItem::from_tracks(&state.library.all_tracks);
            let title = format!("tracks ({})", state.library.all_tracks.len());
            let mut col = BrowseColumn::new_with_tracks(title, items, state.library.all_tracks.clone());
            col.play_all_row = Some(crate::app::state::PlayAllRow::AllTracks {
                label: "Play all library tracks".to_string(),
            });
            col.on_play_row = true;
            state.artist_nav.drill_column(col, auto_drill);
        }

        MillerAction::RefreshAlbumTracks { album_key } => {
            // Refresh album tracks in the currently focused Miller column.
            // Works for both artist_nav and genre_nav.
            let tag_section = state.browse_category.is_tag_section();
            let (request_id, column_index) = if tag_section {
                state.tag_nav_request_id = state.tag_nav_request_id.wrapping_add(1);
                (state.tag_nav_request_id, state.tag_nav.focused_column)
            } else {
                state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
                (state.artist_nav_request_id, state.artist_nav.focused_column)
            };
            let tx = LibraryEventSender::new(event_tx.clone(), state.library_generation);
            let request_client = client.clone();
            tokio::spawn(async move {
                let result = request_client
                    .get_album_tracks(&album_key)
                    .await
                    .map_err(|error| AsyncError::from_api("Failed to refresh album tracks", &error));
                let _ = tx
                    .send(Event::Effect(
                        MillerAction::AlbumTracksRefreshed {
                            request_id,
                            tag_section,
                            column_index,
                            result,
                        }
                        .into(),
                    ))
                    .await;
            });
        }

        MillerAction::AlbumTracksRefreshed {
            request_id,
            tag_section,
            column_index,
            result,
        } => {
            let current_request_id = if tag_section {
                state.tag_nav_request_id
            } else {
                state.artist_nav_request_id
            };
            if current_request_id != request_id {
                return Ok(vec![]);
            }
            match result {
                Ok(tracks) => {
                    state.connection.mark_healthy();
                    let items = BrowseItem::from_tracks(&tracks);

                    // Determine which nav owns the focused track column
                    let nav = if state.browse_category.is_tag_section() {
                        &mut state.tag_nav
                    } else {
                        &mut state.artist_nav
                    };

                    if let Some(col) = nav.columns.get_mut(column_index) {
                        let old_idx = col.selected_index;
                        col.items = items;
                        col.tracks = tracks;
                        col.selected_index = old_idx.min(col.items.len().saturating_sub(1));
                    }
                    state.set_status("Album tracks refreshed".to_string());
                }
                Err(error) => {
                    if error.connection_error {
                        state.connection.mark_degraded(error.message.clone());
                    }
                    state.set_error(error.message);
                }
            }
        }

    }
    Ok(vec![])
}

/// Build album list for a derived track-artist from the all_tracks cache.
///
/// Groups tracks by parent_rating_key (album key) where the track's artist name
/// matches, then looks up Album metadata from state.library.albums.
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
