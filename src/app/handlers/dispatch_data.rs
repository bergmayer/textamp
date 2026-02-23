//! Data loading dispatch handlers: LoadInitialData, LoadLibraries, LoadArtists, LoadAlbums,
//! LoadPlaylists, LoadArtistAlbums, LoadArtistAllTracks, LoadSelectedAlbumTracks,
//! LoadAlbumTracks, LoadCategoryTracks, GoBackInRightPanel, LoadSimilarAlbums,
//! LoadSimilarTracks, ListUp/Down/PageUp/PageDown/Top/Bottom.

use crate::app::{Action, AppState, Event};
use crate::app::state::{BrowseCategory, BrowseItem, Focus, RightPanelMode, View};
use crate::plex::PlexClient;
use crate::plex::models::Track;
use crate::plex::{CacheData, LibraryCache};
use crate::config::Config;
use crate::services::{FolderColumn, FolderNavigationState};

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
    action: Action,
    state: &mut AppState,
    client: &mut PlexClient,
) -> Result<Vec<Action>> {
    match action {
        Action::LoadInitialData => {
            tracing::info!("Action::LoadInitialData - loading libraries and artists");

            // Load theme from config
            state.theme = crate::ui::theme::ThemeName::from_config(&config.ui.theme);
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

                            // Auto-drill: build second column synchronously from cached data
                            let drill_actions = auto_drill_from_cache(state);

                            // Trigger compilation detection if cache had no compilation data
                            helpers::maybe_detect_compilations(event_tx, state, client);

                            // Use two-tier staleness check for the current view
                            if let Some(tier1_cat) = helpers::current_view_category(state) {
                                helpers::check_staleness_on_view_load(event_tx, state, client, tier1_cat);
                            }

                            if !drill_actions.is_empty() {
                                // Fetch libraries in background before returning follow-ups
                                let tx = event_tx.clone();
                                let client_clone = client.clone();
                                tokio::spawn(async move {
                                    match client_clone.get_libraries().await {
                                        Ok(libs) => { let _ = tx.send(Event::LibrariesLoaded(libs)).await; }
                                        Err(e) => {
                                            tracing::error!("Failed to load libraries: {}", e);
                                            let _ = tx.send(Event::DataLoadError(format!("Failed to load libraries: {}", e))).await;
                                        }
                                    }
                                });
                                return Ok(drill_actions);
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
                        let _ = tx.send(Event::LibrariesLoaded(libs)).await;
                    }
                    Err(e) => {
                        tracing::error!("Failed to load libraries: {}", e);
                        let _ = tx.send(Event::DataLoadError(format!("Failed to load libraries: {}", e))).await;
                    }
                }
            });
        }
        Action::LoadArtists => {
            tracing::info!("Action::LoadArtists - active_library={:?}", state.active_library);
            helpers::load_artists(event_tx, state, client);
            tracing::info!("LoadArtists complete - loaded {} artists", state.artists.len());
        }
        Action::LoadPlaylists => {
            helpers::load_playlists(event_tx, state, client);
        }
        Action::LoadArtistAlbums => {
            // Load albums for selected artist (right panel shows albums)
            let artist_key = if let Some(artist) = state.artists.get(state.list_state.artists_index) {
                state.selected_artist_name = artist.title.clone();
                artist.rating_key.clone()
            } else {
                return Ok(vec![]);
            };

            state.right_panel_loading = true;
            state.right_panel_mode = RightPanelMode::ArtistAlbums;
            state.selected_artist_albums.clear();
            state.list_state.right_albums_index = 0;

            helpers::spawn_api_call(event_tx, client,
                move |c| async move { c.get_artist_albums(&artist_key).await },
                Event::ArtistAlbumsLoaded, "Failed to load albums",
            );
        }
        Action::LoadArtistAllTracks => {
            // Load all tracks by the selected artist
            if let Some(artist) = state.artists.get(state.list_state.artists_index) {
                let artist_key = artist.rating_key.clone();
                state.selected_album_title = format!("All tracks by {}", artist.title);
                state.right_panel_loading = true;
                state.right_panel_mode = RightPanelMode::AlbumTracks;
                state.selected_album_tracks.clear();
                state.list_state.tracks_index = 0;

                helpers::spawn_api_call(event_tx, client,
                    move |c| async move { c.get_artist_all_tracks(&artist_key).await },
                    Event::ArtistAllTracksLoaded, "Failed to load tracks",
                );
            }
        }
        Action::LoadSelectedAlbumTracks => {
            // Load tracks for selected album (drill down from artist albums)
            // Index 0 is "All Tracks", so actual albums start at index 1
            let album_idx = state.list_state.right_albums_index.saturating_sub(1);
            if let Some(album) = state.selected_artist_albums.get(album_idx) {
                let album_key = album.rating_key.clone();
                state.selected_album_title = album.title.clone();
                state.right_panel_loading = true;
                state.right_panel_mode = RightPanelMode::AlbumTracks;
                state.selected_album_tracks.clear();
                state.list_state.tracks_index = 0;

                helpers::spawn_api_call(event_tx, client,
                    move |c| async move { c.get_album_tracks(&album_key).await },
                    Event::AlbumTracksLoaded, "Failed to load tracks",
                );
            }
        }
        Action::LoadAlbumTracks { rating_key } => {
            // Load tracks for a specific album (used by genre albums)
            state.right_panel_loading = true;
            state.right_panel_mode = RightPanelMode::AlbumTracks;
            state.selected_album_tracks.clear();
            state.list_state.tracks_index = 0;

            helpers::spawn_api_call(event_tx, client,
                move |c| async move { c.get_album_tracks(&rating_key).await },
                Event::AlbumTracksLoaded, "Failed to load album tracks",
            );
        }
        Action::LoadCategoryTracks => {
            // Load tracks directly (for Playlists category)

            // Ensure category data is loaded first (synchronously - rare fallback)
            match state.browse_category {
                BrowseCategory::Library => {
                    if state.artists.is_empty() && !state.artists_loading {
                        state.artists_loading = true;
                        if let Some(lib_key) = &state.active_library {
                            match client.get_artists(lib_key).await {
                                Ok(mut artists) => {
                                    artists.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
                                    state.artists = artists;
                                }
                                Err(e) => {
                                    tracing::error!("Failed to load artists: {}", e);
                                }
                            }
                        }
                        state.artists_loading = false;
                    }
                }
                BrowseCategory::Playlists => {
                    if state.playlists.is_empty() {
                        let section_id = state.active_library.as_deref();
                        if let Ok(playlists) = client.get_playlists(section_id).await {
                            state.playlists = playlists;
                        }
                    }
                }
                BrowseCategory::Genres => {
                    if state.genres.is_empty() {
                        if let Some(lib_key) = &state.active_library {
                            match client.get_genres(lib_key).await {
                                Ok(genres) => state.genres = genres,
                                Err(e) => tracing::error!("Failed to load genres: {}", e),
                            }
                        }
                    }
                }
                BrowseCategory::Folders => {
                    // Folders don't use this load mechanism
                    return Ok(vec![]);
                }
            }

            // Get rating key AFTER category data is loaded
            let rating_key = state.selected_category_key();

            state.right_panel_mode = RightPanelMode::CategoryTracks;
            state.focus = Focus::Right;
            state.list_state.tracks_index = 0;

            if let Some(key) = rating_key {
                state.right_panel_loading = true;
                state.selected_album_tracks.clear();

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
                        BrowseCategory::Genres => {
                            if let Some(lib_key) = &lib_key {
                                match client.get_genre_tracks(lib_key, &key).await {
                                    Ok(tracks) => Ok(Either::Tracks(tracks)),
                                    Err(e) => Err(e),
                                }
                            } else {
                                Err(crate::plex::ApiError::NoServerSelected)
                            }
                        }
                        BrowseCategory::Folders => unreachable!(),
                    };

                    match result {
                        Ok(Either::Tracks(tracks)) => {
                            let _ = event_tx.send(Event::CategoryTracksLoaded(tracks)).await;
                        }
                        Err(e) => {
                            let error_str = e.to_string();
                            let clean_error = if error_str.contains("<html>") || error_str.contains("500") {
                                "This playlist cannot be loaded (server error)".to_string()
                            } else {
                                format!("Failed to load tracks: {}", e)
                            };
                            let _ = event_tx.send(Event::DataLoadError(clean_error)).await;
                        }
                    }
                });
            } else {
                // No key available
                state.right_panel_loading = false;
                state.selected_album_tracks.clear();
            }
        }
        Action::GoBackInRightPanel => {
            // Go from tracks back to albums view (for artist drill-down)
            if state.right_panel_mode == RightPanelMode::AlbumTracks {
                state.right_panel_mode = RightPanelMode::ArtistAlbums;
                state.selected_album_tracks.clear();
            }
        }
        Action::LoadSimilarAlbums { rating_key, title } => {
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
                Event::SimilarAlbumsLoaded, "Failed to load similar albums",
            );
        }
        Action::LoadSimilarTracks { rating_key, title } => {
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
                Event::SimilarTracksLoaded, "Failed to load similar tracks",
            );
        }
        Action::LoadRelated { artist_key, title } => {
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

            if let Some(alias_names) = state.artist_aliases.get(&artist_key) {
                // Build reverse lookup: alias_name → albums from album_display_artist
                let album_by_key: std::collections::HashMap<&str, &crate::plex::models::Album> = state.albums.iter()
                    .map(|a| (a.rating_key.as_str(), a))
                    .collect();

                for alias_name in alias_names {
                    if alias_name.eq_ignore_ascii_case("Various Artists") {
                        continue;
                    }
                    if let Some(artist) = state.artists.iter().find(|a| {
                        a.title.eq_ignore_ascii_case(alias_name)
                    }) {
                        // Real Plex artist — will fetch albums from API
                        real_alias_artists.push(artist.clone());
                    } else {
                        // No Plex artist entry — build group from source artist's albums
                        // where album_display_artist says this alias name
                        let mut albums: Vec<crate::plex::models::Album> = Vec::new();
                        for (album_key, display_name) in &state.album_display_artist {
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

            let tx = event_tx.clone();
            let c = client.clone();
            tokio::spawn(async move {
                let mut groups = Vec::new();

                // 1. Fetch Plex related artists
                let plex_artists = match c.get_related_artists(&artist_key).await {
                    Ok(artists) => artists,
                    Err(e) => {
                        tracing::warn!("Failed to load related artists: {}", e);
                        vec![]
                    }
                };

                // Filter out: the source artist itself and "Various Artists"
                let filtered_artists: Vec<_> = plex_artists.into_iter()
                    .filter(|a| a.rating_key != artist_key
                        && !a.title.eq_ignore_ascii_case("Various Artists"))
                    .collect();

                // Collect Plex keys to dedup real aliases
                let plex_keys: std::collections::HashSet<String> = filtered_artists.iter()
                    .map(|a| a.rating_key.clone())
                    .collect();

                // 2. Fetch albums for each Plex related artist (parallel)
                let mut handles = Vec::new();
                for artist in &filtered_artists {
                    let c2 = c.clone();
                    let key = artist.rating_key.clone();
                    handles.push(tokio::spawn(async move {
                        c2.get_artist_albums(&key).await.unwrap_or_default()
                    }));
                }

                let mut plex_results: Vec<Vec<crate::plex::models::Album>> = Vec::new();
                for handle in handles {
                    plex_results.push(handle.await.unwrap_or_default());
                }

                for (artist, albums) in filtered_artists.into_iter().zip(plex_results) {
                    if !albums.is_empty() {
                        groups.push(RelatedArtistGroup {
                            artist,
                            albums,
                            source: RelatedSource::Plex,
                        });
                    }
                }

                // 3. Add real alias artists that have Plex entries (dedup against Plex results)
                let mut alias_handles = Vec::new();
                let mut alias_artist_vec = Vec::new();
                for artist in &real_alias_artists {
                    if !plex_keys.contains(&artist.rating_key) {
                        let c2 = c.clone();
                        let key = artist.rating_key.clone();
                        alias_artist_vec.push(artist.clone());
                        alias_handles.push(tokio::spawn(async move {
                            c2.get_artist_albums(&key).await.unwrap_or_default()
                        }));
                    }
                }

                for (artist, handle) in alias_artist_vec.into_iter().zip(alias_handles) {
                    let albums = handle.await.unwrap_or_default();
                    if !albums.is_empty() {
                        groups.push(RelatedArtistGroup {
                            artist,
                            albums,
                            source: RelatedSource::Alias,
                        });
                    }
                }

                // 4. Add synthetic alias groups (aliases without Plex artist entries,
                //    albums derived from source artist's library)
                groups.extend(synthetic_alias_groups);

                let _ = tx.send(Event::RelatedDataLoaded { groups }).await;
            });
        }
        Action::ListUp => {
            helpers::adjust_list_index(state, -1);
        }
        Action::ListDown => {
            helpers::adjust_list_index(state, 1);
            // Lazy load more if needed
            helpers::maybe_load_more(state, client).await;
        }
        Action::ListPageUp => {
            helpers::adjust_list_index(state, -10);
        }
        Action::ListPageDown => {
            helpers::adjust_list_index(state, 10);
            helpers::maybe_load_more(state, client).await;
        }
        Action::ListTop => {
            helpers::set_list_index(state, 0);
        }
        Action::ListBottom => {
            helpers::set_list_index(state, isize::MAX);
        }
        _ => unreachable!("dispatch_data called with non-data action: {:?}", action),
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
        state.artists = cached.artists;
        state.artists.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
        state.artists_total = state.artists.len() as u32;
        let items = state.build_artist_root_items();
        state.artist_nav.reset("artists", items);
    }
    if !cached.albums.is_empty() {
        state.albums = cached.albums;
        state.albums.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
        state.albums_total = state.albums.len() as u32;
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
        state.playlists = playlists;
        let items = BrowseItem::from_playlists(&state.playlists);
        state.playlist_nav.reset("playlists", items);
    }
    if !cached.playlist_tracks.is_empty() {
        state.playlist_tracks_cache = cached.playlist_tracks;
    }

    // Folders
    if !cached.root_folders.is_empty() {
        let root_column = FolderColumn::new(None, lib_title.to_string(), cached.root_folders);
        state.folder_state = Some(FolderNavigationState::with_root(lib_key.to_string(), root_column));
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
    if !cached.genres.is_empty() { state.genres = cached.genres; }
    if !cached.artist_genres.is_empty() { state.artist_genres = cached.artist_genres; }
    if !cached.album_genres.is_empty() { state.album_genres = cached.album_genres; }
    if !cached.moods.is_empty() { state.moods = cached.moods; }
    if !cached.styles.is_empty() { state.styles = cached.styles; }

    // Stations — validate cached data is root stations (not corrupted drilled children)
    let stations_valid = !cached.stations.is_empty()
        && cached.stations.iter().any(|s| s.identifier.as_deref() == Some("library"));
    if stations_valid {
        let mut stations = cached.stations;
        helpers::append_station_action_items(&mut stations, state.shuffle_undo_queue.is_some());
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
        state.all_tracks = cached.all_tracks;
        tracing::debug!("Cache load: {} tracks", state.all_tracks.len());
    }
    if !cached.track_artists.is_empty() {
        state.track_artists = cached.track_artists;
        tracing::debug!("Cache load: {} track artists", state.track_artists.len());
    }

    // Artist aliases
    if !cached.artist_aliases.is_empty() {
        state.artist_aliases = cached.artist_aliases;
        state.album_display_artist = cached.album_display_artist;
        tracing::debug!("Cache load: {} artist aliases", state.artist_aliases.len());
    } else if !state.all_tracks.is_empty() && !state.albums.is_empty() {
        // Recompute from tracks if not cached
        state.build_artist_aliases();
    }

    // Compilation detection results
    if !cached.compilation_albums.is_empty() || !cached.compilation_artist_keys.is_empty() {
        state.compilations.albums = cached.compilation_albums;
        state.compilations.artist_keys = cached.compilation_artist_keys;
        state.compilations.track_artist_keys = cached.compilation_track_artist_keys;
        state.compilations.artist_map = cached.artist_compilation_map;
        state.compilations.single_artist = cached.single_artist_compilations;
        state.compilations.detected = true;
        // Re-build artist root items with compilation data
        let items = state.build_artist_root_items();
        state.artist_nav.update_root_items("artists", items);
    }

}

/// Build the auto-drill child column synchronously from cached data.
/// Returns follow-up actions (e.g. artwork loading, async playlist track fetch).
pub fn auto_drill_from_cache(state: &mut crate::app::AppState) -> Vec<Action> {
    use crate::app::state::{BrowseCategory, BrowseColumn, BrowseItem};

    let mut follow_ups = vec![];

    match state.browse_category {
        BrowseCategory::Library => {
            if state.artist_nav.columns.len() == 1 && !state.artist_nav.is_empty() && !state.albums.is_empty() {
                // "All Artists" (index 0) → show all albums
                let mut items = vec![BrowseItem::AllTracks {
                    artist_key: "__all_library__".to_string(),
                    artist_name: "All Artists".to_string(),
                    thumb: None,
                }];
                items.extend(BrowseItem::from_albums(&state.albums, &state.album_display_artist));
                let mut col = BrowseColumn::new("all albums", items);
                col.artwork_visible = state.artwork.default_visible;
                state.artist_nav.replace_child_column(col);

                // Trigger artwork loading for the new column
                if state.artwork.default_visible {
                    let art_batch = super::dispatch_miller::collect_art_to_load(
                        state.artist_nav.columns.last(),
                        &state.artwork.grid_cache,
                        &state.artwork.grid_pending,
                    );
                    if !art_batch.is_empty() {
                        follow_ups.push(Action::LoadAlbumArt(art_batch));
                    }
                }
            }
        }
        BrowseCategory::Playlists => {
            if state.playlist_nav.columns.len() == 1 && !state.playlist_nav.is_empty() {
                if let Some(playlist_item) = state.playlist_nav.selected_item().cloned() {
                    let playlist_key = playlist_item.key().to_string();
                    if let Some(cached) = state.playlist_tracks_cache.get(&playlist_key) {
                        // Build column synchronously from cached tracks
                        let tracks = cached.tracks.clone();
                        let playlist_name = playlist_item.title().to_string();
                        let title = format!("tracks \u{2014} {}", playlist_name);
                        let items = BrowseItem::from_tracks(&tracks);
                        let col = BrowseColumn::new_with_tracks(title, items, tracks);
                        state.playlist_nav.replace_child_column(col);
                    } else {
                        // Async fallback: tracks not in disk cache (e.g. smart playlists)
                        state.auto_drill_pending = true;
                        if let Some(action) = super::key_input::auto_drill_playlist_action(state) {
                            follow_ups.push(action);
                        }
                    }
                }
            }
        }
        _ => {}
    }

    follow_ups
}
