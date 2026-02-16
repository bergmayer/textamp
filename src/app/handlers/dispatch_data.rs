//! Data loading dispatch handlers: LoadInitialData, LoadLibraries, LoadArtists, LoadAlbums,
//! LoadPlaylists, LoadArtistAlbums, LoadArtistAllTracks, LoadSelectedAlbumTracks,
//! LoadAlbumTracks, LoadCategoryTracks, GoBackInRightPanel, LoadSimilarAlbums,
//! LoadSimilarTracks, ListUp/Down/PageUp/PageDown/Top/Bottom.

use crate::app::{Action, AppState, Event};
use crate::app::state::{BrowseCategory, BrowseItem, Focus, RightPanelMode, View};
use crate::api::PlexClient;
use crate::api::models::Track;
use crate::cache::{CacheData, LibraryCache};
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
                        let _ = tx.send(Event::LibrariesLoaded(libs)).await;
                    }
                    Err(e) => {
                        tracing::error!("Failed to load libraries: {}", e);
                        let _ = tx.send(Event::DataLoadError(format!("Failed to load libraries: {}", e))).await;
                    }
                }
            });
        }
        Action::LoadLibraries => {
            tracing::info!("Action::LoadLibraries - fetching libraries");
            match client.get_libraries().await {
                Ok(libs) => {
                    tracing::info!("Fetched {} libraries", libs.len());
                    state.libraries = libs.into_iter().filter(|l| l.is_music()).collect();
                    if let Some(lib) = state.libraries.first() {
                        state.active_library = Some(lib.key.clone());
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to load libraries: {}", e);
                }
            }
        }
        Action::LoadArtists => {
            tracing::info!("Action::LoadArtists - active_library={:?}", state.active_library);
            helpers::load_artists(event_tx, state, client);
            tracing::info!("LoadArtists complete - loaded {} artists", state.artists.len());
        }
        Action::LoadAlbums => {
            helpers::load_albums(event_tx, state, client);
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

            let event_tx = event_tx.clone();
            let client = client.clone();
            tokio::spawn(async move {
                match client.get_artist_albums(&artist_key).await {
                    Ok(albums) => { let _ = event_tx.send(Event::ArtistAlbumsLoaded(albums)).await; }
                    Err(e) => { let _ = event_tx.send(Event::DataLoadError(format!("Failed to load albums: {}", e))).await; }
                }
            });
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

                let event_tx = event_tx.clone();
                let client = client.clone();
                tokio::spawn(async move {
                    match client.get_artist_all_tracks(&artist_key).await {
                        Ok(tracks) => { let _ = event_tx.send(Event::ArtistAllTracksLoaded(tracks)).await; }
                        Err(e) => { let _ = event_tx.send(Event::DataLoadError(format!("Failed to load tracks: {}", e))).await; }
                    }
                });
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

                let event_tx = event_tx.clone();
                let client = client.clone();
                tokio::spawn(async move {
                    match client.get_album_tracks(&album_key).await {
                        Ok(tracks) => { let _ = event_tx.send(Event::AlbumTracksLoaded(tracks)).await; }
                        Err(e) => { let _ = event_tx.send(Event::DataLoadError(format!("Failed to load tracks: {}", e))).await; }
                    }
                });
            }
        }
        Action::LoadAlbumTracks { rating_key } => {
            // Load tracks for a specific album (used by genre albums)
            state.right_panel_loading = true;
            state.right_panel_mode = RightPanelMode::AlbumTracks;
            state.selected_album_tracks.clear();
            state.list_state.tracks_index = 0;

            let event_tx = event_tx.clone();
            let client = client.clone();
            tokio::spawn(async move {
                match client.get_album_tracks(&rating_key).await {
                    Ok(tracks) => { let _ = event_tx.send(Event::AlbumTracksLoaded(tracks)).await; }
                    Err(e) => { let _ = event_tx.send(Event::DataLoadError(format!("Failed to load album tracks: {}", e))).await; }
                }
            });
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
                                Err(crate::api::ApiError::NoServerSelected)
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
            state.similar_source_title = title;
            state.similar_loading = true;
            state.similar_albums.clear();
            state.list_state.similar_index = 0;
            state.view = View::Similar;

            let event_tx = event_tx.clone();
            let client = client.clone();
            tokio::spawn(async move {
                match client.get_similar_albums(&rating_key, 50).await {
                    Ok(albums) => { let _ = event_tx.send(Event::SimilarAlbumsLoaded(albums)).await; }
                    Err(e) => { let _ = event_tx.send(Event::DataLoadError(format!("Failed to load similar albums: {}", e))).await; }
                }
            });
        }
        Action::LoadSimilarTracks { rating_key, title } => {
            state.similar_source_title = title;
            state.similar_loading = true;
            state.similar_tracks.clear();
            state.list_state.similar_index = 0;
            state.view = View::Similar;

            let event_tx = event_tx.clone();
            let client = client.clone();
            tokio::spawn(async move {
                match client.get_similar_tracks(&rating_key, 50).await {
                    Ok(tracks) => { let _ = event_tx.send(Event::SimilarTracksLoaded(tracks)).await; }
                    Err(e) => { let _ = event_tx.send(Event::DataLoadError(format!("Failed to load similar tracks: {}", e))).await; }
                }
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
                state.category_timestamps.insert(cat, *ts);
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
                state.category_timestamps.insert(*cat, ts);
            }
        }
    }

    // Core library data - IMPORTANT: Always re-sort after loading from cache
    if !cached.artists.is_empty() {
        state.artists = cached.artists;
        state.artists.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
        state.artists_total = state.artists.len() as u32;
        let items = state.build_artist_root_items();
        state.artist_nav.reset(state.artist_view_mode.name(), items);
    }
    if !cached.albums.is_empty() {
        state.albums = cached.albums;
        state.albums.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
        state.albums_total = state.albums.len() as u32;
    }
    if !cached.playlists.is_empty() {
        state.playlists = cached.playlists.clone();
        let items = BrowseItem::from_playlists(&state.playlists);
        state.playlist_nav.reset("playlists", items);
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
    if !cached.genres.is_empty() {
        state.genres = cached.genres;
        if state.genre_content_type == crate::app::state::GenreContentType::Genres {
            let items = BrowseItem::from_genres(&state.genres);
            state.genre_nav.reset("genres", items);
        }
    }
    if !cached.artist_genres.is_empty() {
        state.artist_genres = cached.artist_genres;
        if state.genre_content_type == crate::app::state::GenreContentType::ArtistGenres {
            let items = BrowseItem::from_genres(&state.artist_genres);
            state.genre_nav.reset("artist genres", items);
        }
    }
    if !cached.album_genres.is_empty() {
        state.album_genres = cached.album_genres;
        if state.genre_content_type == crate::app::state::GenreContentType::AlbumGenres {
            let items = BrowseItem::from_genres(&state.album_genres);
            state.genre_nav.reset("album genres", items);
        }
    }
    if !cached.moods.is_empty() {
        state.moods = cached.moods;
        if state.genre_content_type == crate::app::state::GenreContentType::Moods {
            let items = BrowseItem::from_genres(&state.moods);
            state.genre_nav.reset("moods", items);
        }
    }
    if !cached.styles.is_empty() {
        state.styles = cached.styles;
        if state.genre_content_type == crate::app::state::GenreContentType::Styles {
            let items = BrowseItem::from_genres(&state.styles);
            state.genre_nav.reset("styles", items);
        }
    }

    // Stations
    if !cached.stations.is_empty() {
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
    }

    // Compilation detection results
    if !cached.compilation_albums.is_empty() || !cached.compilation_artist_keys.is_empty() {
        state.compilation_albums = cached.compilation_albums;
        state.compilation_artist_keys = cached.compilation_artist_keys;
        state.compilation_track_artist_keys = cached.compilation_track_artist_keys;
        state.compilations_detected = true;
        // Re-build artist root items with compilation data
        let items = state.build_artist_root_items();
        state.artist_nav.update_root_items(state.artist_view_mode.name(), items);
    }

}
