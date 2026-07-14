//! Data loading dispatch handlers: LoadInitialData, LoadLibraries, LoadArtists, LoadAlbums,
//! LoadPlaylists, LoadArtistAlbums, LoadArtistAllTracks, LoadSelectedAlbumTracks,
//! LoadAlbumTracks, LoadCategoryTracks, GoBackInRightPanel, LoadSimilarAlbums,
//! LoadSimilarTracks, ListUp/Down/PageUp/PageDown/Top/Bottom.

use crate::app::event::*;
use crate::app::{Action, AppState, Event};
use crate::app::action::{BrowseAction, DataAction};
use crate::app::state::{BrowseCategory, Focus, RightPanelMode, View};
use crate::plex::PlexClient;
use crate::plex::models::Track;
use crate::plex::LibraryCache;
use crate::config::Config;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Helper enum for LoadCategoryTracks spawn result disambiguation.
enum Either {
    Tracks(Vec<Track>),
}

/// Dispatch data-loading actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    config: &Config,
    action: DataAction,
    state: &mut AppState,
    client: &mut PlexClient,
) -> Result<Vec<Action>> {
    match action {
        DataAction::LoadInitialData => {
            tracing::info!("Action::LoadInitialData - loading libraries and artists");

            // Load theme from config
            state.theme = crate::app::theme::ThemeName::from_config(&config.ui.theme);
            crate::ui::theme::set_theme(state.theme);
            tracing::info!("Loaded theme: {}", state.theme.display_name());

            // Determine library key from config (instant, no network)
            let saved_key = if state.is_fresh_login {
                state.is_fresh_login = false;
                None // Fresh login - wait for LibrariesLoaded to pick
            } else {
                config.libraries.default_library.clone()
            };

            // If we have a saved library key, load from cache immediately (no network)
            if let Some(ref lib_key) = saved_key {
                state.advance_library_generation();
                state.active_library = Some(lib_key.clone());
                state.keep_subfolder_cache = config.libraries.per_library
                    .get(lib_key.as_str())
                    .map(|s| s.keep_subfolder_cache)
                    .unwrap_or(false);

                // A library cache is ~19 MiB and JSON decoding is blocking. Load
                // it off the runtime/UI thread and version the result by library.
                state.library_loading = true;
                let tx = event_tx.clone();
                let cache_library_key = lib_key.clone();
                let cache_generation = state.library_generation;
                let cache_server_id = state.active_server_id.clone();
                tokio::task::spawn_blocking(move || {
                    let result = LibraryCache::new().and_then(|cache| {
                        cache.load_scoped(cache_server_id.as_deref(), &cache_library_key)
                    });
                    let event: Event = match result {
                        Some(cached) => PreloadEvent::LibraryCacheLoaded {
                            library_key: cache_library_key,
                            cached: Box::new(cached),
                        }.into(),
                        None => PreloadEvent::LibraryCacheLoadFailed {
                            library_key: cache_library_key,
                        }.into(),
                    };
                    let _ = tx.blocking_send(Event::for_library(cache_generation, event));
                });
            }

            // Fetch libraries from API in background (non-blocking)
            let tx = LibraryEventSender::new(
                event_tx.clone(),
                state.library_generation,
            );
            let client_clone = client.clone();
            tokio::spawn(async move {
                let server_url = client_clone.server_url().map(str::to_owned);
                let result = client_clone.get_libraries().await.map_err(|error| {
                    crate::app::action::AsyncError::from_api(
                        "Failed to load libraries",
                        &error,
                    )
                });
                let _ = tx
                    .send(DataEvent::LibrariesLoaded { server_url, result }.into())
                    .await;
            });
        }
        DataAction::LoadArtists => {
            tracing::info!("DataAction::LoadArtists - active_library={:?}", state.active_library);
            helpers::load_artists(event_tx, state, client);
            tracing::info!("LoadArtists complete - loaded {} artists", state.library.artists.len());
        }
        DataAction::LoadPlaylists => {
            helpers::load_playlists(event_tx, state, client);
        }
        DataAction::LoadArtistAlbums => {
            // Load albums for selected artist (right panel shows albums)
            let artist_key = if let Some(artist) = state.library.artists.get(state.list_state.artists_index) {
                state.library.selected_artist_name = artist.title.clone();
                artist.rating_key.clone()
            } else {
                return Ok(vec![]);
            };

            state.library.right_panel_loading = true;
            state.library.right_panel_mode = RightPanelMode::ArtistAlbums;
            state.library.selected_artist_albums.clear();
            state.list_state.right_albums_index = 0;
            let request_key = format!("artist-albums:{artist_key}");
            state.library.right_panel_request_key = Some(request_key.clone());

            helpers::spawn_scoped_api_call(event_tx, state.library_generation, client, request_key,
                move |c| async move { c.get_artist_albums(&artist_key).await },
                |request_key, albums| DataEvent::ArtistAlbumsLoaded { request_key, albums }.into(),
                "Failed to load albums",
            );
        }
        DataAction::LoadArtistAllTracks => {
            // Load all tracks by the selected artist
            if let Some(artist) = state.library.artists.get(state.list_state.artists_index) {
                let artist_key = artist.rating_key.clone();
                state.library.selected_album_title = format!("All tracks by {}", artist.title);
                state.library.right_panel_loading = true;
                state.library.right_panel_mode = RightPanelMode::AlbumTracks;
                state.library.selected_album_tracks.clear();
                state.list_state.tracks_index = 0;
                let request_key = format!("artist-tracks:{artist_key}");
                state.library.right_panel_request_key = Some(request_key.clone());

                helpers::spawn_scoped_api_call(event_tx, state.library_generation, client, request_key,
                    move |c| async move { c.get_artist_all_tracks(&artist_key).await },
                    |request_key, tracks| DataEvent::ArtistAllTracksLoaded { request_key, tracks }.into(),
                    "Failed to load tracks",
                );
            }
        }
        DataAction::LoadSelectedAlbumTracks => {
            // Load tracks for selected album (drill down from artist albums)
            // Index 0 is "All Tracks", so actual albums start at index 1
            let album_idx = state.list_state.right_albums_index.saturating_sub(1);
            if let Some(album) = state.library.selected_artist_albums.get(album_idx) {
                let album_key = album.rating_key.clone();
                state.library.selected_album_title = album.title.clone();
                state.library.right_panel_loading = true;
                state.library.right_panel_mode = RightPanelMode::AlbumTracks;
                state.library.selected_album_tracks.clear();
                state.list_state.tracks_index = 0;
                let request_key = format!("album-tracks:{album_key}");
                state.library.right_panel_request_key = Some(request_key.clone());

                helpers::spawn_scoped_api_call(event_tx, state.library_generation, client, request_key,
                    move |c| async move { c.get_album_tracks(&album_key).await },
                    |request_key, tracks| DataEvent::AlbumTracksLoaded { request_key, tracks }.into(),
                    "Failed to load tracks",
                );
            }
        }
        DataAction::LoadAlbumTracks { rating_key } => {
            // Load tracks for a specific album (used by genre albums)
            state.library.right_panel_loading = true;
            state.library.right_panel_mode = RightPanelMode::AlbumTracks;
            state.library.selected_album_tracks.clear();
            state.list_state.tracks_index = 0;
            let request_key = format!("album-tracks:{rating_key}");
            state.library.right_panel_request_key = Some(request_key.clone());

            helpers::spawn_scoped_api_call(event_tx, state.library_generation, client, request_key,
                move |c| async move { c.get_album_tracks(&rating_key).await },
                |request_key, tracks| DataEvent::AlbumTracksLoaded { request_key, tracks }.into(),
                "Failed to load album tracks",
            );
        }
        DataAction::LoadCategoryTracks => {
            // Load tracks directly (for Playlists category)

            // Ensure category data is loaded first (synchronously - rare fallback)
            match state.browse_category {
                BrowseCategory::Library => {
                    if state.library.artists.is_empty() && !state.library.artists_loading {
                        helpers::load_artists(event_tx, state, client);
                        return Ok(vec![]);
                    }
                }
                BrowseCategory::Playlists => {
                    if state.library.playlists.is_empty() {
                        helpers::load_playlists(event_tx, state, client);
                        return Ok(vec![]);
                    }
                }
                BrowseCategory::Folders => {
                    return Ok(vec![]);
                }
                cat if cat.is_tag_section() => {
                    // The active section's tag list is the source of truth
                    // for selected_category_key(). If empty, kick off a load
                    // and bail — the result will arrive via the
                    // PreloadEvent / TagListPreloaded path.
                    if state.tag_list_for(cat).is_empty() {
                        return Ok(vec![BrowseAction::LoadTagList(cat).into()]);
                    }
                }
                _ => return Ok(vec![]),
            }

            // Get rating key AFTER category data is loaded
            let rating_key = state.selected_category_key();

            state.library.right_panel_mode = RightPanelMode::CategoryTracks;
            state.focus = Focus::Right;
            state.list_state.tracks_index = 0;

            if let Some(key) = rating_key {
                state.library.right_panel_loading = true;
                state.library.selected_album_tracks.clear();

                // Capture branching data synchronously before spawning
                let browse_category = state.browse_category;
                let _playlist_title = state.selected_category_title()
                    .map(|s| s.to_lowercase());
                let lib_key = state.active_library.clone();
                let request_key = format!("category-tracks:{browse_category:?}:{key}");
                state.library.right_panel_request_key = Some(request_key.clone());

                let event_tx = LibraryEventSender::new(
                    event_tx.clone(),
                    state.library_generation,
                );
                let client = client.clone();
                tokio::spawn(async move {
                    let result = match browse_category {
                        BrowseCategory::Library => {
                            match client.get_artist_all_tracks(&key).await {
                                Ok(tracks) => Ok(Either::Tracks(tracks)),
                                Err(e) => Err(e),
                            }
                        }
                        BrowseCategory::Playlists => {
                            match client.get_playlist_tracks(&key).await {
                                Ok(tracks) => Ok(Either::Tracks(tracks)),
                                Err(e) => Err(e),
                            }
                        }
                        BrowseCategory::Folders => unreachable!(),
                        cat if cat.is_tag_section() => {
                            if let Some(lib_key) = &lib_key {
                                match client.get_genre_tracks(lib_key, &key).await {
                                    Ok(tracks) => Ok(Either::Tracks(tracks)),
                                    Err(e) => Err(e),
                                }
                            } else {
                                Err(crate::plex::ApiError::NoServerSelected)
                            }
                        }
                        _ => return,
                    };

                    match result {
                        Ok(Either::Tracks(tracks)) => {
                            let _ = event_tx.send(DataEvent::CategoryTracksLoaded {
                                request_key,
                                tracks,
                            }.into()).await;
                        }
                        Err(e) => {
                            let error_str = e.to_string();
                            let clean_error = if error_str.contains("<html>") || error_str.contains("500") {
                                "This playlist cannot be loaded (server error)".to_string()
                            } else {
                                format!("Failed to load tracks: {}", e)
                            };
                            let _ = event_tx.send(DataEvent::ScopedLoadError {
                                request_key,
                                message: clean_error,
                                connection_error: e.is_connection_error(),
                            }.into()).await;
                        }
                    }
                });
            } else {
                // No key available
                state.library.right_panel_loading = false;
                state.library.selected_album_tracks.clear();
            }
        }
        DataAction::GoBackInRightPanel => {
            // Go from tracks back to albums view (for artist drill-down)
            if state.library.right_panel_mode == RightPanelMode::AlbumTracks {
                state.library.right_panel_mode = RightPanelMode::ArtistAlbums;
                state.library.selected_album_tracks.clear();
            }
        }
        DataAction::LoadSimilarAlbums { rating_key, title } => {
            let request_key = format!("similar-albums:{rating_key}");
            state.similar.source_title = title;
            state.similar.request_key = Some(request_key.clone());
            state.similar.loading = true;
            state.similar.albums.clear();
            state.list_state.similar_index = 0;
            state.similar.mode = crate::app::state::SimilarMode::Albums;
            if state.view != View::Similar {
                state.set_view(View::Similar);
            }

            helpers::spawn_scoped_api_call(event_tx, state.library_generation, client, request_key,
                move |c| async move { c.get_similar_albums(&rating_key, 50).await },
                |request_key, albums| DataEvent::SimilarAlbumsLoaded { request_key, albums }.into(),
                "Failed to load similar albums",
            );
        }
        DataAction::LoadSimilarTracks { rating_key, title } => {
            let request_key = format!("similar-tracks:{rating_key}");
            state.similar.source_title = title;
            state.similar.request_key = Some(request_key.clone());
            state.similar.loading = true;
            state.similar.tracks.clear();
            state.list_state.similar_index = 0;
            state.similar.mode = crate::app::state::SimilarMode::Tracks;
            if state.view != View::Similar {
                state.set_view(View::Similar);
            }

            helpers::spawn_scoped_api_call(event_tx, state.library_generation, client, request_key,
                move |c| async move { c.get_similar_tracks(&rating_key, 50).await },
                |request_key, tracks| DataEvent::SimilarTracksLoaded { request_key, tracks }.into(),
                "Failed to load similar tracks",
            );
        }
        DataAction::LoadTrackPaneSimilar { rating_key } => {
            // Idempotent — return early if already loaded or in flight.
            if state.track_pane_similar.contains_key(&rating_key)
                || state.track_pane_similar_loading.contains(&rating_key)
            {
                return Ok(vec![]);
            }
            state.track_pane_similar_loading.insert(rating_key.clone());
            let tx = LibraryEventSender::new(
                event_tx.clone(),
                state.library_generation,
            );
            let request_client = client.clone();
            tokio::spawn(async move {
                let server_url = request_client.server_url().map(str::to_owned);
                let result = request_client
                    .get_similar_tracks(&rating_key, 12)
                    .await
                    .map_err(|error| {
                        crate::app::action::AsyncError::from_api(
                            "Failed to load similar tracks",
                            &error,
                        )
                    });
                let _ = tx
                    .send(
                        DataEvent::TrackPaneSimilarLoaded {
                            server_url,
                            rating_key,
                            result,
                        }
                        .into(),
                    )
                    .await;
            });
        }
        DataAction::LoadSimilarArtists { artist_key, title } => {
            let request_key = format!("similar-artists:{artist_key}");
            state.similar.source_title = title;
            state.similar.request_key = Some(request_key.clone());
            state.similar.loading = true;
            state.similar.artists.clear();
            state.list_state.similar_index = 0;
            state.similar.mode = crate::app::state::SimilarMode::Artists;
            if state.view != View::Similar {
                state.previous_view = Some(state.view);
                state.set_view(View::Similar);
            }

            helpers::spawn_scoped_api_call(event_tx, state.library_generation, client, request_key,
                move |c| async move { c.get_similar_artists(&artist_key, 50).await },
                |request_key, artists| DataEvent::SimilarArtistsLoaded { request_key, artists }.into(),
                "Failed to load similar artists",
            );
        }
        DataAction::LoadRelated { artist_key, title } => {
            state.related.source_title = title;
            state.related.source_key = artist_key.clone();
            state.related.loading = true;
            state.related.groups.clear();
            state.list_state.related_index = 0;
            state.scroll.related = None;
            if state.view != View::Related {
                state.previous_view = Some(state.view);
                state.set_view(View::Related);
            }

            // Collect alias data before spawning async task.
            // Two kinds:
            // - "real" aliases: alias name matches a Plex artist → fetch their albums from API
            // - "synthetic" aliases: alias name has no Plex artist → use albums from state
            //   (these are albums filed under the source artist where all tracks say the alias name)
            use crate::app::state::{RelatedArtistGroup, RelatedSource};

            let mut real_alias_artists: Vec<crate::plex::models::Artist> = Vec::new();
            let mut synthetic_alias_groups: Vec<RelatedArtistGroup> = Vec::new();

            if let Some(alias_names) = state.library.artist_aliases.get(&artist_key) {
                // Build reverse lookup: alias_name → albums from album_display_artist
                let album_by_key: std::collections::HashMap<&str, &crate::plex::models::Album> = state.library.albums.iter()
                    .map(|a| (a.rating_key.as_str(), a))
                    .collect();

                for alias_name in alias_names {
                    if alias_name.eq_ignore_ascii_case("Various Artists") {
                        continue;
                    }
                    if let Some(artist) = state.library.artists.iter().find(|a| {
                        a.title.eq_ignore_ascii_case(alias_name)
                    }) {
                        // Real Plex artist — will fetch albums from API
                        real_alias_artists.push(artist.clone());
                    } else {
                        // No Plex artist entry — build group from source artist's albums
                        // where album_display_artist says this alias name
                        let mut albums: Vec<crate::plex::models::Album> = Vec::new();
                        for (album_key, display_name) in &state.library.album_display_artist {
                            if display_name.eq_ignore_ascii_case(alias_name) {
                                if let Some(album) = album_by_key.get(album_key.as_str()) {
                                    albums.push((*album).clone());
                                }
                            }
                        }
                        if !albums.is_empty() {
                            albums.sort_by(|a, b| a.year.cmp(&b.year));
                            synthetic_alias_groups.push(RelatedArtistGroup {
                                artist: crate::plex::models::Artist {
                                    title: alias_name.clone(),
                                    rating_key: format!("alias:{}", alias_name),
                                    ..Default::default()
                                },
                                albums,
                                source: RelatedSource::Alias,
                            });
                        }
                    }
                }
            }

            // Build artist lookup map for fuzzy-matching Similar tags.
            // Keys: lowercase title, and "the "-stripped variant.
            let mut artists_by_title: std::collections::HashMap<String, crate::plex::models::Artist> = std::collections::HashMap::new();
            for artist in &state.library.artists {
                let lower = artist.title.to_lowercase();
                // Also index without leading "The "
                if let Some(stripped) = lower.strip_prefix("the ") {
                    artists_by_title.entry(stripped.to_string()).or_insert_with(|| artist.clone());
                }
                artists_by_title.entry(lower).or_insert_with(|| artist.clone());
            }

            // Pre-build cached albums by parent artist key for fallback
            // when the API returns 0 albums (e.g. compilation-subtype albums).
            let mut cached_albums_by_artist: std::collections::HashMap<String, Vec<crate::plex::models::Album>> = std::collections::HashMap::new();
            for album in &state.library.albums {
                if let Some(parent_key) = &album.parent_rating_key {
                    cached_albums_by_artist.entry(parent_key.clone()).or_default().push(album.clone());
                }
            }

            let tx = LibraryEventSender::new(
                event_tx.clone(),
                state.library_generation,
            );
            let request_key = artist_key.clone();
            let c = client.clone();
            tokio::spawn(async move {
                let mut groups = Vec::new();

                // 1. Fetch Plex related artists (hub endpoint)
                let plex_artists = match c.get_related_artists(&artist_key).await {
                    Ok(artists) => artists,
                    Err(e) => {
                        tracing::warn!("Failed to load related artists: {}", e);
                        vec![]
                    }
                };

                // Filter out: the source artist itself and "Various Artists"
                let plex_artist_names: Vec<String> = plex_artists.iter().map(|a| a.title.clone()).collect();
                tracing::debug!("Plex /related hub returned {} artists: {:?}", plex_artists.len(), plex_artist_names);
                let filtered_artists: Vec<_> = plex_artists.into_iter()
                    .filter(|a| a.rating_key != artist_key
                        && !a.title.eq_ignore_ascii_case("Various Artists"))
                    .collect();

                // 2. Cross-reference hub artists against library and fetch albums.
                //    The hub may return external references whose rating_key doesn't
                //    match the local library entry (e.g. "Urinals" vs "The Urinals"),
                //    so we fuzzy-match by title and prefer the library artist's key.
                let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
                seen_keys.insert(artist_key.clone());
                let mut resolved_artists: Vec<crate::plex::models::Artist> = Vec::new();
                for artist in &filtered_artists {
                    let lower = artist.title.to_lowercase();
                    let library_match = artists_by_title.get(&lower)
                        .or_else(|| lower.strip_prefix("the ").and_then(|s| artists_by_title.get(s)))
                        .or_else(|| artists_by_title.get(&format!("the {}", lower)));
                    let resolved = if let Some(lib_artist) = library_match {
                        tracing::debug!("Related: hub '{}' (key={}) → library '{}' (key={})",
                            artist.title, artist.rating_key, lib_artist.title, lib_artist.rating_key);
                        lib_artist.clone()
                    } else {
                        tracing::debug!("Related: hub '{}' (key={}) has no library match",
                            artist.title, artist.rating_key);
                        artist.clone()
                    };
                    // Dedup: skip if we've already resolved to this key
                    if seen_keys.insert(resolved.rating_key.clone()) {
                        resolved_artists.push(resolved);
                    }
                }

                let mut handles = Vec::new();
                for artist in &resolved_artists {
                    let c2 = c.clone();
                    let key = artist.rating_key.clone();
                    let title = artist.title.clone();
                    let cached = cached_albums_by_artist.get(&key).cloned().unwrap_or_default();
                    handles.push(tokio::spawn(async move {
                        let mut albums = c2.get_artist_albums(&key).await.unwrap_or_default();
                        // Merge cached albums the API missed (same pattern as Miller columns)
                        if !cached.is_empty() {
                            let api_keys: std::collections::HashSet<String> = albums.iter()
                                .map(|a| a.rating_key.clone())
                                .collect();
                            let missing: Vec<_> = cached.into_iter()
                                .filter(|a| !api_keys.contains(&a.rating_key))
                                .collect();
                            if !missing.is_empty() {
                                tracing::debug!("Related: merging {} cached albums for '{}' (key={}, API returned {})",
                                    missing.len(), title, key, albums.len());
                                albums.extend(missing);
                            }
                        }
                        albums
                    }));
                }

                let mut plex_results: Vec<Vec<crate::plex::models::Album>> = Vec::new();
                for handle in handles {
                    plex_results.push(handle.await.unwrap_or_default());
                }

                for (artist, albums) in resolved_artists.into_iter().zip(plex_results) {
                    groups.push(RelatedArtistGroup {
                        artist,
                        albums,
                        source: RelatedSource::Plex,
                    });
                }

                // 3. Fetch artist detail for "Similar" metadata tags
                //    Use raw fetch to inspect actual response structure.
                let similar_tags: Vec<String> = {
                    let path = format!("/library/metadata/{}", artist_key);
                    match c.get_raw(&path).await {
                        Ok(raw) => {
                            // Parse as generic JSON to inspect structure
                            let parsed: Result<serde_json::Value, _> = serde_json::from_str(&raw);
                            match parsed {
                                Ok(json) => {
                                    // The artist metadata might be in "Metadata" or "Directory" array
                                    let mc = json.get("MediaContainer");
                                    let artist_val = mc
                                        .and_then(|mc| mc.get("Metadata").or_else(|| mc.get("Directory")))
                                        .and_then(|arr| arr.as_array())
                                        .and_then(|arr| arr.first());

                                    if let Some(artist_obj) = artist_val {
                                        // Extract "Similar" array
                                        if let Some(similar_arr) = artist_obj.get("Similar").and_then(|s| s.as_array()) {
                                            let tags: Vec<String> = similar_arr.iter()
                                                .filter_map(|t| t.get("tag").and_then(|v| v.as_str()).map(|s| s.to_string()))
                                                .collect();
                                            tracing::debug!("Similar tags for artist {}: {:?}", artist_key, tags);
                                            tags
                                        } else {
                                            // Log available top-level keys for diagnosis
                                            let keys: Vec<&str> = artist_obj.as_object()
                                                .map(|m| m.keys().map(|k| k.as_str()).collect())
                                                .unwrap_or_default();
                                            tracing::debug!("No 'Similar' field on artist {}. Available keys: {:?}", artist_key, keys);
                                            vec![]
                                        }
                                    } else {
                                        let mc_keys: Vec<&str> = mc
                                            .and_then(|v| v.as_object())
                                            .map(|m| m.keys().map(|k| k.as_str()).collect())
                                            .unwrap_or_default();
                                        tracing::warn!("No artist metadata found in response. MediaContainer keys: {:?}", mc_keys);
                                        vec![]
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("Failed to parse artist detail JSON: {}", e);
                                    vec![]
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to fetch artist detail for Similar tags: {}", e);
                            vec![]
                        }
                    }
                };

                // Fuzzy-match Similar tags against library artists
                let mut tag_matched_artists: Vec<crate::plex::models::Artist> = Vec::new();
                for tag_name in &similar_tags {
                    if tag_name.eq_ignore_ascii_case("Various Artists") {
                        continue;
                    }
                    let lower = tag_name.to_lowercase();
                    // Try: exact, strip "The ", prepend "The "
                    let matched = artists_by_title.get(&lower)
                        .or_else(|| lower.strip_prefix("the ").and_then(|s| artists_by_title.get(s)))
                        .or_else(|| artists_by_title.get(&format!("the {}", lower)));
                    if let Some(artist) = matched {
                        if !seen_keys.contains(&artist.rating_key) {
                            seen_keys.insert(artist.rating_key.clone());
                            tag_matched_artists.push(artist.clone());
                            tracing::debug!("Similar tag '{}' matched library artist '{}'", tag_name, artist.title);
                        } else {
                            tracing::debug!("Similar tag '{}' matched '{}' but already in results", tag_name, artist.title);
                        }
                    } else {
                        tracing::debug!("Similar tag '{}' had no match in library ({} artists indexed)", tag_name, artists_by_title.len());
                    }
                }

                // Fetch albums for tag-matched artists (parallel, with cache merge)
                let mut tag_handles = Vec::new();
                for artist in &tag_matched_artists {
                    let c2 = c.clone();
                    let key = artist.rating_key.clone();
                    let cached = cached_albums_by_artist.get(&key).cloned().unwrap_or_default();
                    tag_handles.push(tokio::spawn(async move {
                        let mut albums = c2.get_artist_albums(&key).await.unwrap_or_default();
                        if !cached.is_empty() {
                            let api_keys: std::collections::HashSet<String> = albums.iter()
                                .map(|a| a.rating_key.clone()).collect();
                            let missing: Vec<_> = cached.into_iter()
                                .filter(|a| !api_keys.contains(&a.rating_key)).collect();
                            if !missing.is_empty() { albums.extend(missing); }
                        }
                        albums
                    }));
                }

                for (artist, handle) in tag_matched_artists.into_iter().zip(tag_handles) {
                    let albums = handle.await.unwrap_or_default();
                    groups.push(RelatedArtistGroup {
                        artist,
                        albums,
                        source: RelatedSource::SimilarTag,
                    });
                }

                // 4. Add real alias artists that have Plex entries (dedup against Plex + tag results)
                let mut alias_handles = Vec::new();
                let mut alias_artist_vec = Vec::new();
                for artist in &real_alias_artists {
                    if !seen_keys.contains(&artist.rating_key) {
                        seen_keys.insert(artist.rating_key.clone());
                        let c2 = c.clone();
                        let key = artist.rating_key.clone();
                        let cached = cached_albums_by_artist.get(&key).cloned().unwrap_or_default();
                        alias_artist_vec.push(artist.clone());
                        alias_handles.push(tokio::spawn(async move {
                            let mut albums = c2.get_artist_albums(&key).await.unwrap_or_default();
                            if !cached.is_empty() {
                                let api_keys: std::collections::HashSet<String> = albums.iter()
                                    .map(|a| a.rating_key.clone()).collect();
                                let missing: Vec<_> = cached.into_iter()
                                    .filter(|a| !api_keys.contains(&a.rating_key)).collect();
                                if !missing.is_empty() { albums.extend(missing); }
                            }
                            albums
                        }));
                    }
                }

                for (artist, handle) in alias_artist_vec.into_iter().zip(alias_handles) {
                    let albums = handle.await.unwrap_or_default();
                    groups.push(RelatedArtistGroup {
                        artist,
                        albums,
                        source: RelatedSource::Alias,
                    });
                }

                // 5. Add synthetic alias groups (aliases without Plex artist entries,
                //    albums derived from source artist's library)
                groups.extend(synthetic_alias_groups);

                let _ = tx.send(DataEvent::RelatedDataLoaded { request_key, groups }.into()).await;
            });
        }
        DataAction::ListUp => {
            helpers::adjust_list_index(state, -1);
        }
        DataAction::ListDown => {
            helpers::adjust_list_index(state, 1);
            // Lazy load more if needed
            helpers::maybe_load_more(event_tx, state, client);
        }
        DataAction::ListPageUp => {
            helpers::adjust_list_index(state, -10);
        }
        DataAction::ListPageDown => {
            helpers::adjust_list_index(state, 10);
            helpers::maybe_load_more(event_tx, state, client);
        }
        DataAction::ListTop => {
            helpers::set_list_index(state, 0);
        }
        DataAction::ListBottom => {
            helpers::set_list_index(state, isize::MAX);
        }
    }
    Ok(vec![])
}
