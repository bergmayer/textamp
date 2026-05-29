//! Data loading dispatch handlers: LoadInitialData, LoadLibraries, LoadArtists, LoadAlbums,
//! LoadPlaylists, LoadArtistAlbums, LoadArtistAllTracks, LoadSelectedAlbumTracks,
//! LoadAlbumTracks, LoadCategoryTracks, GoBackInRightPanel, LoadSimilarAlbums,
//! LoadSimilarTracks, ListUp/Down/PageUp/PageDown/Top/Bottom.

use crate::app::event::*;
use crate::app::{Action, AppState, Event};
use crate::app::action::{BrowseAction, DataAction};
use crate::app::state::{BrowseCategory, BrowseItem, Focus, RightPanelMode, View};
use crate::plex::PlexClient;
use crate::plex::models::Track;
use crate::plex::{CacheData, LibraryCache};
use crate::config::Config;
use crate::services::{FolderColumn, FolderNavigationState, FolderService};

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
                state.active_library = Some(lib_key.clone());
                state.keep_subfolder_cache = config.libraries.per_library
                    .get(lib_key.as_str())
                    .map(|s| s.keep_subfolder_cache)
                    .unwrap_or(false);

                if let Some(cache) = LibraryCache::new() {
                    if let Some(cached) = cache.load(lib_key) {
                        if cached.library_key != *lib_key {
                            tracing::warn!("Cache library_key mismatch: expected {}, got {} - ignoring cache",
                                lib_key, cached.library_key);
                        } else {
                            tracing::info!("Loading from cache: {} artists, {} albums, {} folders, {} genres",
                                cached.artists.len(), cached.albums.len(), cached.root_folders.len(), cached.genres.len());

                            let lib_title = "Music".to_string();
                            load_from_cache(state, cached, lib_key, &lib_title);

                            // No auto-drill on cache load — leave the
                            // user on a clean cat+artists 2-col view
                            // and let them drill explicitly.

                            // Trigger compilation detection if cache had no compilation data
                            helpers::maybe_detect_compilations(event_tx, state, client);

                            // Use two-tier staleness check for the current view
                            if let Some(tier1_cat) = helpers::current_view_category(state) {
                                helpers::check_staleness_on_view_load(event_tx, state, client, tier1_cat);
                            }
                        }
                    }
                }
            }

            // Fetch libraries from API in background (non-blocking)
            let tx = event_tx.clone();
            let client_clone = client.clone();
            tokio::spawn(async move {
                match client_clone.get_libraries().await {
                    Ok(libs) => {
                        let _ = tx.send(DataEvent::LibrariesLoaded(libs).into()).await;
                    }
                    Err(e) => {
                        tracing::error!("Failed to load libraries: {}", e);
                        let _ = tx.send(DataEvent::DataLoadError(format!("Failed to load libraries: {}", e)).into()).await;
                    }
                }
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

            helpers::spawn_api_call(event_tx, client,
                move |c| async move { c.get_artist_albums(&artist_key).await },
                |x| DataEvent::ArtistAlbumsLoaded(x).into(), "Failed to load albums",
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

                helpers::spawn_api_call(event_tx, client,
                    move |c| async move { c.get_artist_all_tracks(&artist_key).await },
                    |x| DataEvent::ArtistAllTracksLoaded(x).into(), "Failed to load tracks",
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

                helpers::spawn_api_call(event_tx, client,
                    move |c| async move { c.get_album_tracks(&album_key).await },
                    |x| DataEvent::AlbumTracksLoaded(x).into(), "Failed to load tracks",
                );
            }
        }
        DataAction::LoadAlbumTracks { rating_key } => {
            // Load tracks for a specific album (used by genre albums)
            state.library.right_panel_loading = true;
            state.library.right_panel_mode = RightPanelMode::AlbumTracks;
            state.library.selected_album_tracks.clear();
            state.list_state.tracks_index = 0;

            helpers::spawn_api_call(event_tx, client,
                move |c| async move { c.get_album_tracks(&rating_key).await },
                |x| DataEvent::AlbumTracksLoaded(x).into(), "Failed to load album tracks",
            );
        }
        DataAction::LoadCategoryTracks => {
            // Load tracks directly (for Playlists category)

            // Ensure category data is loaded first (synchronously - rare fallback)
            match state.browse_category {
                BrowseCategory::Library => {
                    if state.library.artists.is_empty() && !state.library.artists_loading {
                        state.library.artists_loading = true;
                        if let Some(lib_key) = &state.active_library {
                            match client.get_artists(lib_key).await {
                                Ok(mut artists) => {
                                    artists.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
                                    state.library.artists = artists;
                                }
                                Err(e) => {
                                    tracing::error!("Failed to load artists: {}", e);
                                }
                            }
                        }
                        state.library.artists_loading = false;
                    }
                }
                BrowseCategory::Playlists => {
                    if state.library.playlists.is_empty() {
                        let section_id = state.active_library.as_deref();
                        if let Ok(playlists) = client.get_playlists(section_id).await {
                            state.library.playlists = playlists;
                        }
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

                let event_tx = event_tx.clone();
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
                            let _ = event_tx.send(DataEvent::CategoryTracksLoaded(tracks).into()).await;
                        }
                        Err(e) => {
                            let error_str = e.to_string();
                            let clean_error = if error_str.contains("<html>") || error_str.contains("500") {
                                "This playlist cannot be loaded (server error)".to_string()
                            } else {
                                format!("Failed to load tracks: {}", e)
                            };
                            let _ = event_tx.send(DataEvent::DataLoadError(clean_error).into()).await;
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
            state.similar.source_title = title;
            state.similar.loading = true;
            state.similar.albums.clear();
            state.list_state.similar_index = 0;
            state.similar.mode = crate::app::state::SimilarMode::Albums;
            if state.view != View::Similar {
                state.set_view(View::Similar);
            }

            helpers::spawn_api_call(event_tx, client,
                move |c| async move { c.get_similar_albums(&rating_key, 50).await },
                |x| DataEvent::SimilarAlbumsLoaded(x).into(), "Failed to load similar albums",
            );
        }
        DataAction::LoadSimilarTracks { rating_key, title } => {
            state.similar.source_title = title;
            state.similar.loading = true;
            state.similar.tracks.clear();
            state.list_state.similar_index = 0;
            state.similar.mode = crate::app::state::SimilarMode::Tracks;
            if state.view != View::Similar {
                state.set_view(View::Similar);
            }

            helpers::spawn_api_call(event_tx, client,
                move |c| async move { c.get_similar_tracks(&rating_key, 50).await },
                |x| DataEvent::SimilarTracksLoaded(x).into(), "Failed to load similar tracks",
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
            let key_for_event = rating_key.clone();
            helpers::spawn_api_call(event_tx, client,
                move |c| async move { c.get_similar_tracks(&rating_key, 12).await },
                move |tracks| DataEvent::TrackPaneSimilarLoaded {
                    rating_key: key_for_event.clone(),
                    tracks,
                }.into(),
                "Failed to load track-pane similar tracks",
            );
        }
        DataAction::LoadSimilarArtists { artist_key, title } => {
            state.similar.source_title = title;
            state.similar.loading = true;
            state.similar.artists.clear();
            state.list_state.similar_index = 0;
            state.similar.mode = crate::app::state::SimilarMode::Artists;
            if state.view != View::Similar {
                state.previous_view = Some(state.view);
                state.set_view(View::Similar);
            }

            helpers::spawn_api_call(event_tx, client,
                move |c| async move { c.get_similar_artists(&artist_key, 50).await },
                |x| DataEvent::SimilarArtistsLoaded(x).into(), "Failed to load similar artists",
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

            let tx = event_tx.clone();
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

                let _ = tx.send(DataEvent::RelatedDataLoaded { groups }.into()).await;
            });
        }
        DataAction::ListUp => {
            helpers::adjust_list_index(state, -1);
        }
        DataAction::ListDown => {
            helpers::adjust_list_index(state, 1);
            // Lazy load more if needed
            helpers::maybe_load_more(state, client).await;
        }
        DataAction::ListPageUp => {
            helpers::adjust_list_index(state, -10);
        }
        DataAction::ListPageDown => {
            helpers::adjust_list_index(state, 10);
            helpers::maybe_load_more(state, client).await;
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

/// Load cached library data into state for instant startup.
/// Mirrors the LibraryCacheLoaded event handler logic.
fn load_from_cache(state: &mut AppState, cached: CacheData, lib_key: &str, lib_title: &str) {
    // Load per-category timestamps (with backward compat migration)
    if !cached.category_timestamps.is_empty() {
        for (key, ts) in &cached.category_timestamps {
            if let Some(cat) = crate::app::state::RefreshCategory::from_cache_key(key) {
                state.cache_mgmt.category_timestamps.insert(cat, *ts);
            }
        }
    } else {
        // Migrate from legacy shared timestamps
        let lib_ts = cached.timestamp;
        let playlist_ts = if cached.playlist_timestamp > 0 { cached.playlist_timestamp } else { lib_ts };
        if lib_ts > 0 {
            use crate::app::state::RefreshCategory;
            for cat in RefreshCategory::all() {
                let ts = if cat.is_playlist_group() { playlist_ts } else { lib_ts };
                state.cache_mgmt.category_timestamps.insert(*cat, ts);
            }
        }
    }

    // Core library data - IMPORTANT: Always re-sort after loading from cache
    if !cached.artists.is_empty() {
        state.library.artists = cached.artists;
        state.library.artists.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
        state.library.artists_total = state.library.artists.len() as u32;
        let items = state.build_artist_root_items();
        state.artist_nav.reset("artists", items);
    }
    if !cached.albums.is_empty() {
        state.library.albums = cached.albums;
        state.library.albums.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
        state.library.albums_total = state.library.albums.len() as u32;
    }
    if !cached.playlists.is_empty() {
        let mut playlists = cached.playlists.clone();
        // Move "Recently Played" to top (matches PlaylistsLoaded behavior)
        if let Some(pos) = playlists.iter().position(|p| p.title == "Recently Played") {
            if pos > 0 {
                let rp = playlists.remove(pos);
                playlists.insert(0, rp);
            }
        }
        state.library.playlists = playlists;
        let items = BrowseItem::from_playlists(&state.library.playlists);
        state.playlist_nav.reset("playlists", items);
    }
    if !cached.playlist_tracks.is_empty() {
        state.playlist_tracks_cache = cached.playlist_tracks;
    }

    // Folders
    if !cached.root_folders.is_empty() {
        let folders = FolderService::filter_invalid(cached.root_folders);
        let root_column = FolderColumn::new(None, lib_title.to_string(), folders);
        let mut fs = FolderNavigationState::with_root(lib_key.to_string(), root_column);
        fs.ensure_placeholder();
        state.folder_state = Some(fs);
    }
    if !cached.folder_contents.is_empty() {
        state.folder_contents_cache = cached.folder_contents;
        // Stale entries are kept as a warm cache; the subfolder preload
        // crawl will re-fetch and overwrite them incrementally.
    } else {
        state.folder_contents_cache.clear();
    }

    // Genres, artist genres, album genres, moods, styles
    // Just store the data — genre_nav is populated lazily via DrillGenreCategory
    if !cached.genres.is_empty() { state.library.album_genres = cached.genres; }
    if !cached.artist_genres.is_empty() { state.library.artist_genres = cached.artist_genres; }
    if !cached.album_genres.is_empty() { state.library.album_genres = cached.album_genres; }
    if !cached.moods.is_empty() { state.library.moods = cached.moods; }
    if !cached.styles.is_empty() { state.library.styles = cached.styles; }

    // Stations — validate cached data is root stations (not corrupted drilled children)
    let stations_valid = !cached.stations.is_empty()
        && cached.stations.iter().any(|s| s.identifier.as_deref() == Some("library"));
    if stations_valid {
        let mut stations = cached.stations;
        helpers::append_station_action_items(&mut stations, state.queue.shuffle_undo_queue.is_some());
        state.stations = stations.clone();
        state.station_nav.columns.clear();
        state.station_nav.columns.push(crate::app::state::StationColumn::new(
            None,
            "Radio".to_string(),
            stations,
        ));
        state.station_nav.focused_column = 0;
    } else if !cached.stations.is_empty() {
        tracing::warn!("Ignoring corrupted station cache ({} items, missing root identifiers)", cached.stations.len());
    }
    if !cached.station_children.is_empty() {
        state.station_children_cache = cached.station_children;
    }

    // All tracks + track-level artists
    if !cached.all_tracks.is_empty() {
        state.library.all_tracks = cached.all_tracks;
        tracing::debug!("Cache load: {} tracks", state.library.all_tracks.len());
    }
    if !cached.track_artists.is_empty() {
        state.library.track_artists = cached.track_artists;
        tracing::debug!("Cache load: {} track artists", state.library.track_artists.len());
    }

    // Artist aliases
    if !cached.artist_aliases.is_empty() {
        state.library.artist_aliases = cached.artist_aliases;
        state.library.album_display_artist = cached.album_display_artist;
        tracing::debug!("Cache load: {} artist aliases", state.library.artist_aliases.len());
    } else if !state.library.all_tracks.is_empty() && !state.library.albums.is_empty() {
        // Recompute from tracks if not cached
        state.build_artist_aliases();
    }

    // Compilation detection results
    if !cached.compilation_albums.is_empty() || !cached.compilation_artist_keys.is_empty() {
        state.library.compilations.albums = cached.compilation_albums;
        state.library.compilations.artist_keys = cached.compilation_artist_keys;
        state.library.compilations.track_artist_keys = cached.compilation_track_artist_keys;
        state.library.compilations.artist_map = cached.artist_compilation_map;
        state.library.compilations.single_artist = cached.single_artist_compilations;
        state.library.compilations.detected = true;
        // Re-build artist root items with compilation data
        let items = state.build_artist_root_items();
        state.artist_nav.update_root_items("artists", items);
    }

}

