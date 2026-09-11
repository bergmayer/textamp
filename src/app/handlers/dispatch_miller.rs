//! Miller column dispatch handlers for all *ForMiller and *FromMiller actions.

use crate::app::action::{MillerAction, SystemAction};
use crate::app::state::{BrowseColumn, BrowseItem};
use crate::app::{Action, AppState, Event};
use crate::audio::AudioPlayer;
use crate::library::models::Track;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Collect tracks from a column for playback (shared by Play*TrackFromMiller handlers).
fn collect_tracks_from_column(
    col: &BrowseColumn,
    track_index: usize,
    single_track: bool,
) -> Vec<Track> {
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
    if total == 0 {
        return vec![];
    }

    // Window around selected_index
    let half = ART_BATCH_LIMIT / 2;
    let center = col.selected_index;
    let start = center.saturating_sub(half);
    let end = (center + ART_BATCH_LIMIT).min(total);

    let mut to_load = Vec::new();
    for item in &col.items[start..end] {
        match item {
            BrowseItem::Album {
                key,
                thumb: Some(thumb),
                ..
            } if !cache.contains_key(key) && !pending.contains(key) => {
                to_load.push((key.clone(), thumb.clone()));
            }
            BrowseItem::Artist {
                key,
                thumb: Some(thumb),
                ..
            } if !cache.contains_key(key) && !pending.contains(key) => {
                to_load.push((key.clone(), thumb.clone()));
            }
            BrowseItem::AllTracks {
                scope,
                thumb: Some(thumb),
            } => {
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

    let Some(col) = nav.focused() else {
        return vec![];
    };
    // Only load art for columns with artwork_visible enabled
    if !col.artwork_visible {
        return vec![];
    }

    let has_art_items = col.items.iter().any(|item| {
        matches!(
            item,
            BrowseItem::Album { .. } | BrowseItem::Artist { .. } | BrowseItem::AllTracks { .. }
        )
    });
    if !has_art_items {
        return vec![];
    }

    collect_art_to_load(
        Some(col),
        &state.artwork.grid_cache,
        &state.artwork.grid_pending,
    )
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
            BrowseItem::Album {
                key,
                thumb: Some(thumb),
                ..
            } if !cache.contains_key(key) && !pending.contains(key) => {
                to_load.push((key.clone(), thumb.clone()));
            }
            BrowseItem::Artist {
                key,
                thumb: Some(thumb),
                ..
            } if !cache.contains_key(key) && !pending.contains(key) => {
                to_load.push((key.clone(), thumb.clone()));
            }
            BrowseItem::AllTracks {
                scope,
                thumb: Some(thumb),
            } => {
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

    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    match action {
        // ================================================================
        // Miller Column Actions for Artists View
        // ================================================================
        MillerAction::ArtistAlbumsForMillerLoaded {
            request_id,
            artist_key,
            replace_child,
            is_catalog_artist,
            result,
        } => {
            if state.artist_nav_request_id != request_id {
                return Ok(vec![]);
            }
            match result {
                Ok(albums) => {
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
                    if is_catalog_artist {
                        let api_keys: std::collections::HashSet<&str> =
                            all_albums.iter().map(|a| a.rating_key.as_str()).collect();
                        let missing: Vec<_> = state
                            .library
                            .albums
                            .iter()
                            .filter(|a| {
                                a.parent_rating_key.as_deref() == Some(&*artist_key)
                                    && !api_keys.contains(a.rating_key.as_str())
                            })
                            .cloned()
                            .collect();
                        if !missing.is_empty() {
                            tracing::debug!("Merging {} albums from preload that API didn't return for artist {}", missing.len(), artist_key);
                            all_albums.extend(missing);
                        }
                    }

                    let mut items: Vec<BrowseItem> =
                        BrowseItem::from_albums(&all_albums, &state.library.album_display_artist);

                    // Append single-artist compilations (albums where this artist
                    // is the sole performer but parent is a different artist)
                    if let Some(solo_comps) =
                        state.library.compilations.single_artist.get(&artist_key)
                    {
                        let existing_keys: std::collections::HashSet<&str> =
                            all_albums.iter().map(|a| a.rating_key.as_str()).collect();
                        let new_comps: Vec<_> = solo_comps
                            .iter()
                            .filter(|a| !existing_keys.contains(a.rating_key.as_str()))
                            .cloned()
                            .collect();
                        if !new_comps.is_empty() {
                            items.extend(BrowseItem::from_albums(
                                &new_comps,
                                &state.library.album_display_artist,
                            ));
                        }
                    }

                    // Pinned action rows go at the bottom, after all
                    // album rows.
                    items.push(artist_radio);
                    items.push(all_tracks);
                    if state
                        .library
                        .compilations
                        .artist_map
                        .contains_key(&artist_key)
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
                        collect_art_to_load(
                            state.artist_nav.columns.last(),
                            &state.artwork.grid_cache,
                            &state.artwork.grid_pending,
                        )
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
                    state.set_error(error.message);
                }
            }
            state.artist_nav.loading = false;
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
                            if let Some(pos) = col.items.iter().position(|i| i.key() == tk.as_str())
                            {
                                col.selected_index = pos;
                            }
                        }
                        state.search.pending_track_key = None;
                    }
                }
                Err(error) => {
                    state.set_error(error.message);
                }
            }
            state.artist_nav.loading = false;
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
                    state.set_error(error.message);
                }
            }
            state.artist_nav.loading = false;
        }

        MillerAction::PlayTrackFromMiller {
            column_index,
            track_index,
            single_track,
        } => {
            if let Some(col) = state.artist_nav.columns.get(column_index) {
                let tracks = collect_tracks_from_column(col, track_index, single_track);
                if !tracks.is_empty() {
                    helpers::queue_and_play(event_tx, state, audio, tracks, 0);
                }
            }
        }
        // Miller Column Actions for Genres View
        // ================================================================
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
                    let items =
                        BrowseItem::from_albums(&albums, &state.library.album_display_artist);
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
                        let art_batch = collect_art_to_load(
                            state.tag_nav.columns.last(),
                            &state.artwork.grid_cache,
                            &state.artwork.grid_pending,
                        );
                        if !art_batch.is_empty() {
                            state.tag_nav.loading = false;
                            return Ok(vec![SystemAction::LoadAlbumArt(art_batch).into()]);
                        }
                    }
                }
                Err(error) => {
                    state.set_error(error.message);
                }
            }
            state.tag_nav.loading = false;
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
                    state.set_error(error.message);
                }
            }
            state.tag_nav.loading = false;
        }

        MillerAction::PlayGenreTrackFromMiller {
            column_index,
            track_index,
            single_track,
        } => {
            if let Some(col) = state.tag_nav.columns.get(column_index) {
                let tracks = collect_tracks_from_column(col, track_index, single_track);
                if !tracks.is_empty() {
                    helpers::queue_and_play(event_tx, state, audio, tracks, 0);
                }
            }
        }
        // Miller Column Actions for Playlists View
        // ================================================================
        MillerAction::PlayPlaylistTrackFromMiller {
            column_index,
            track_index,
            single_track,
        } => {
            if let Some(col) = state.playlist_nav.columns.get(column_index) {
                let tracks = collect_tracks_from_column(col, track_index, single_track);
                if !tracks.is_empty() {
                    helpers::queue_and_play(event_tx, state, audio, tracks, 0);
                }
            }
        }
        MillerAction::LoadCompilationsForMiller { replace_child } => {
            // Push a new column with compilation albums; "All Tracks"
            // sits at the bottom as a pinned action row.
            let auto_drill = replace_child;
            let mut items: Vec<BrowseItem> = BrowseItem::from_albums(
                &state.library.compilations.albums,
                &state.library.album_display_artist,
            );
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

        MillerAction::LoadCompilationAlbumsForMiller {
            artist_key,
            artist_name,
            replace_child,
        } => {
            // Show compilation albums for this artist, with "All Tracks" pinned at top
            let auto_drill = replace_child;
            if let Some(album_keys) = state.library.compilations.artist_map.get(&artist_key) {
                let album_keys_set: std::collections::HashSet<&str> =
                    album_keys.iter().map(|s| s.as_str()).collect();
                let albums: Vec<_> = state
                    .library
                    .compilations
                    .albums
                    .iter()
                    .filter(|a| album_keys_set.contains(a.rating_key.as_str()))
                    .cloned()
                    .collect();

                let artist_thumb = state
                    .library
                    .artists
                    .iter()
                    .find(|a| a.rating_key == artist_key)
                    .or_else(|| {
                        state
                            .library
                            .track_artists
                            .iter()
                            .find(|a| a.rating_key == artist_key)
                    })
                    .and_then(|a| a.thumb.clone());

                let mut items: Vec<BrowseItem> =
                    BrowseItem::from_albums(&albums, &state.library.album_display_artist);
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
                    collect_art_to_load(
                        state.artist_nav.columns.last(),
                        &state.artwork.grid_cache,
                        &state.artwork.grid_pending,
                    )
                } else {
                    vec![]
                };
                if !art_batch.is_empty() {
                    return Ok(vec![SystemAction::LoadAlbumArt(art_batch).into()]);
                }
            }
        }

        MillerAction::LoadCompilationAllTracksForMiller {
            artist_key,
            artist_name: _,
            replace_child,
        } => {
            // Load all tracks from this artist's compilation albums (all artists, not filtered)
            let auto_drill = replace_child;
            if let Some(album_keys) = state.library.compilations.artist_map.get(&artist_key) {
                let album_keys_set: std::collections::HashSet<&str> =
                    album_keys.iter().map(|s| s.as_str()).collect();
                let tracks: Vec<_> = state
                    .library
                    .all_tracks
                    .iter()
                    .filter(|t| {
                        t.parent_rating_key
                            .as_deref()
                            .is_some_and(|pk| album_keys_set.contains(pk))
                    })
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
            let album_keys_set: std::collections::HashSet<&str> = state
                .library
                .compilations
                .albums
                .iter()
                .map(|a| a.rating_key.as_str())
                .collect();
            let tracks: Vec<_> = state
                .library
                .all_tracks
                .iter()
                .filter(|t| {
                    t.parent_rating_key
                        .as_deref()
                        .is_some_and(|pk| album_keys_set.contains(pk))
                })
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
                    state.set_error(error.message);
                }
            }
        }
        _ => anyhow::bail!("Unsupported library operation reached shared handler"),
    }
    Ok(vec![])
}
