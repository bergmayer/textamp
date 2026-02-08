//! Browse dispatch handlers: LoadStations, LoadGenres, LoadArtistGenres, LoadAlbumGenres,
//! LoadMoods, LoadStyles, LoadGenreAlbums, LoadArtistGenreAlbums, LoadAlbumGenreAlbums,
//! LoadMoodAlbums, LoadStyleAlbums, CycleGenreContentType, RefreshGenreView,
//! CycleArtistViewMode, RefreshArtistView, CycleNowPlayingMode, RefreshNowPlayingView,
//! LoadRecentlyPlayedAlbums, CyclePlaylistsMode, RefreshPlaylistsView, LoadRecentlyAddedAlbums.

use crate::app::{Action, AppState, Event};
use crate::app::state::{
    ArtistViewMode, BrowseCategory, BrowseItem, BrowseNavigationState, Focus,
    GenreContentType, NowPlayingMode, PlaylistsMode, RightPanelMode, StationColumn,
};
use crate::api::PlexClient;

use anyhow::Result;
use tokio::sync::mpsc;

/// Dispatch browse-related actions. Returns follow-up actions.
pub async fn dispatch(
    _event_tx: &mpsc::Sender<Event>,
    action: Action,
    state: &mut AppState,
    client: &mut PlexClient,
) -> Result<Vec<Action>> {
    let mut follow_ups = vec![];

    match action {
        Action::LoadStations => {
            if let Some(lib_key) = &state.active_library.clone() {
                state.stations_loading = true;
                state.station_nav.loading = true;
                match client.get_stations(lib_key).await {
                    Ok(stations) => {
                        // Initialize with root column
                        state.station_nav.columns.clear();
                        state.station_nav.columns.push(StationColumn::new(
                            None,
                            "Stations".to_string(),
                            stations.clone(),
                        ));
                        state.station_nav.focused_column = 0;
                        state.station_nav.loading = false;
                        // Keep legacy state in sync
                        state.stations = stations;
                        state.stations_loading = false;
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load stations: {}", e));
                        state.stations_loading = false;
                        state.station_nav.loading = false;
                    }
                }
            }
        }
        Action::LoadGenres => {
            if let Some(lib_key) = &state.active_library.clone() {
                state.genres_loading = true;
                state.genre_nav.loading = true;
                match client.get_genres(lib_key).await {
                    Ok(genres) => {
                        // Initialize genre_nav with the genres list
                        let items = BrowseItem::from_genres(&genres);
                        state.genre_nav.reset("genres", items);
                        state.genres = genres;
                        state.genres_loading = false;
                        state.genres_index = 0;
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load genres: {}", e));
                        state.genres_loading = false;
                        state.genre_nav.loading = false;
                    }
                }
            }
        }
        Action::LoadArtistGenres => {
            if let Some(lib_key) = &state.active_library.clone() {
                state.artist_genres_loading = true;
                state.genre_nav.loading = true;
                match client.get_artist_genres(lib_key).await {
                    Ok(genres) => {
                        let items = BrowseItem::from_genres(&genres);
                        state.genre_nav.reset("artist genres", items);
                        state.artist_genres = genres;
                        state.artist_genres_loading = false;
                        state.genres_index = 0;
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load artist genres: {}", e));
                        state.artist_genres_loading = false;
                        state.genre_nav.loading = false;
                    }
                }
            }
        }
        Action::LoadAlbumGenres => {
            if let Some(lib_key) = &state.active_library.clone() {
                state.album_genres_loading = true;
                state.genre_nav.loading = true;
                match client.get_album_genres(lib_key).await {
                    Ok(genres) => {
                        let items = BrowseItem::from_genres(&genres);
                        state.genre_nav.reset("album genres", items);
                        state.album_genres = genres;
                        state.album_genres_loading = false;
                        state.genres_index = 0;
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load album genres: {}", e));
                        state.album_genres_loading = false;
                        state.genre_nav.loading = false;
                    }
                }
            }
        }
        Action::LoadMoods => {
            if let Some(lib_key) = &state.active_library.clone() {
                state.moods_loading = true;
                state.genre_nav.loading = true;
                match client.get_moods(lib_key).await {
                    Ok(moods) => {
                        let items = BrowseItem::from_genres(&moods);
                        state.genre_nav.reset("moods", items);
                        state.moods = moods;
                        state.moods_loading = false;
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load moods: {}", e));
                        state.moods_loading = false;
                        state.genre_nav.loading = false;
                    }
                }
            }
        }
        Action::LoadStyles => {
            if let Some(lib_key) = &state.active_library.clone() {
                state.styles_loading = true;
                state.genre_nav.loading = true;
                match client.get_styles(lib_key).await {
                    Ok(styles) => {
                        let items = BrowseItem::from_genres(&styles);
                        state.genre_nav.reset("styles", items);
                        state.styles = styles;
                        state.styles_loading = false;
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load styles: {}", e));
                        state.styles_loading = false;
                        state.genre_nav.loading = false;
                    }
                }
            }
        }
        Action::LoadGenreAlbums => {
            // Get the selected genre based on current content type
            let genre = state.current_genre_list().get(state.genres_index).cloned();

            if let Some(genre) = genre {
                let genre_key = genre.effective_key().to_string();
                let lib_key = state.active_library.clone();
                state.right_panel_loading = true;
                state.genre_albums.clear();
                state.genre_albums_index = 0;

                if genre_key.is_empty() {
                    state.set_error("Genre/mood/style has no valid key".to_string());
                    state.right_panel_loading = false;
                } else if let Some(lib_key) = lib_key {
                    let result = match state.genre_content_type {
                        GenreContentType::Genres => {
                            client.get_genre_albums(&lib_key, &genre_key).await
                        }
                        GenreContentType::ArtistGenres => {
                            client.get_artist_genre_albums(&lib_key, &genre_key).await
                        }
                        GenreContentType::AlbumGenres => {
                            client.get_album_genre_albums(&lib_key, &genre_key).await
                        }
                        GenreContentType::Moods => {
                            client.get_mood_albums(&lib_key, &genre_key).await
                        }
                        GenreContentType::Styles => {
                            client.get_style_albums(&lib_key, &genre_key).await
                        }
                        GenreContentType::Stations => {
                            // Stations don't have albums - this shouldn't be called
                            Ok(Vec::new())
                        }
                    };

                    match result {
                        Ok(mut albums) => {
                            // Sort albums by artist
                            albums.sort_by(|a, b| {
                                let a_artist = a.parent_title.as_deref().unwrap_or("").to_lowercase();
                                let b_artist = b.parent_title.as_deref().unwrap_or("").to_lowercase();
                                a_artist.cmp(&b_artist)
                            });
                            state.genre_albums = albums;
                        }
                        Err(e) => {
                            state.set_error(format!("Failed to load albums: {}", e));
                        }
                    }
                    state.right_panel_loading = false;
                } else {
                    state.set_error("No library selected".to_string());
                    state.right_panel_loading = false;
                }
            }
        }
        Action::LoadArtistGenreAlbums => {
            // Handled by LoadGenreAlbums - just dispatch to it
            follow_ups.push(Action::LoadGenreAlbums);
        }
        Action::LoadAlbumGenreAlbums => {
            // Handled by LoadGenreAlbums - just dispatch to it
            follow_ups.push(Action::LoadGenreAlbums);
        }
        Action::LoadMoodAlbums => {
            // Handled by LoadGenreAlbums - just dispatch to it
            follow_ups.push(Action::LoadGenreAlbums);
        }
        Action::LoadStyleAlbums => {
            // Handled by LoadGenreAlbums - just dispatch to it
            follow_ups.push(Action::LoadGenreAlbums);
        }
        Action::CycleGenreContentType => {
            state.genre_content_type = state.genre_content_type.next();
            follow_ups.push(Action::RefreshGenreView);
        }
        Action::RefreshGenreView => {
            state.genres_index = 0;
            state.genre_albums.clear();
            state.genre_albums_index = 0;

            // Reset genre_nav when cycling
            state.genre_nav = BrowseNavigationState::new();

            // Load the appropriate content based on current type
            match state.genre_content_type {
                GenreContentType::Genres => {
                    if state.genres.is_empty() {
                        follow_ups.push(Action::LoadGenres);
                    } else {
                        // Initialize genre_nav from cached data
                        let items = BrowseItem::from_genres(&state.genres);
                        state.genre_nav.reset("genres", items);
                    }
                }
                GenreContentType::ArtistGenres => {
                    if state.artist_genres.is_empty() {
                        follow_ups.push(Action::LoadArtistGenres);
                    } else {
                        let items = BrowseItem::from_genres(&state.artist_genres);
                        state.genre_nav.reset("artist genres", items);
                    }
                }
                GenreContentType::AlbumGenres => {
                    if state.album_genres.is_empty() {
                        follow_ups.push(Action::LoadAlbumGenres);
                    } else {
                        let items = BrowseItem::from_genres(&state.album_genres);
                        state.genre_nav.reset("album genres", items);
                    }
                }
                GenreContentType::Moods => {
                    if state.moods.is_empty() {
                        follow_ups.push(Action::LoadMoods);
                    } else {
                        let items = BrowseItem::from_genres(&state.moods);
                        state.genre_nav.reset("moods", items);
                    }
                }
                GenreContentType::Styles => {
                    if state.styles.is_empty() {
                        follow_ups.push(Action::LoadStyles);
                    } else {
                        let items = BrowseItem::from_genres(&state.styles);
                        state.genre_nav.reset("styles", items);
                    }
                }
                GenreContentType::Stations => {
                    // Reset station navigation and load stations
                    state.station_nav.columns.clear();
                    state.station_nav.focused_column = 0;
                    follow_ups.push(Action::LoadStations);
                }
            }
        }
        Action::CycleArtistViewMode => {
            state.artist_view_mode = state.artist_view_mode.next();
            follow_ups.push(Action::RefreshArtistView);
        }
        Action::RefreshArtistView => {
            // Clear right panel state (drill-down state) but keep preloaded data
            state.selected_artist_albums.clear();
            state.selected_album_tracks.clear();
            state.list_state.artists_index = 0;
            state.list_state.albums_index = 0;
            state.right_panel_mode = RightPanelMode::Empty;
            state.focus = Focus::Left;

            // Only load if data is empty (not already preloaded)
            match state.artist_view_mode {
                ArtistViewMode::Artist |
                ArtistViewMode::AlbumArtist => {
                    if state.artists.is_empty() {
                        follow_ups.push(Action::LoadArtists);
                    }
                    // Reset artist_nav with new data
                    let title = state.artist_view_mode.name();
                    let items = BrowseItem::from_artists(&state.artists);
                    state.artist_nav.reset(title, items);
                }
                ArtistViewMode::Album => {
                    if state.albums.is_empty() {
                        follow_ups.push(Action::LoadAlbums);
                    }
                    // Reset artist_nav with albums
                    let title = state.artist_view_mode.name();
                    let items = BrowseItem::from_albums(&state.albums);
                    state.artist_nav.reset(title, items);
                }
            }
        }
        Action::CycleNowPlayingMode => {
            state.now_playing_mode = state.now_playing_mode.next();
            follow_ups.push(Action::RefreshNowPlayingView);
        }
        Action::RefreshNowPlayingView => {
            // Load data for the current mode if needed
            match state.now_playing_mode {
                NowPlayingMode::Queue => {
                    // Queue mode - nothing to load, already have queue
                }
                NowPlayingMode::NowPlaying => {
                    // Now Playing mode - load waveform
                    follow_ups.push(Action::LoadWaveform);
                }
            }
        }
        Action::LoadRecentlyPlayedAlbums => {
            if let Some(library_key) = &state.active_library {
                state.recently_played_loading = true;
                // Use the lastViewedAt sort approach - more reliable than hubs
                match client.get_recently_played_albums(library_key, 50).await {
                    Ok(albums) => {
                        tracing::info!("Loaded {} recently played albums", albums.len());
                        state.recently_played_albums = albums;
                        state.recently_played_loading = false;
                        // Reset playlist_nav if currently in RecentlyPlayed mode
                        if state.playlists_mode == PlaylistsMode::RecentlyPlayed {
                            let items = BrowseItem::from_albums(&state.recently_played_albums);
                            state.playlist_nav.reset("recently played", items);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load recently played albums: {}", e);
                        state.recently_played_loading = false;
                    }
                }
            }
        }
        Action::CyclePlaylistsMode => {
            state.playlists_mode = state.playlists_mode.next();
            follow_ups.push(Action::RefreshPlaylistsView);
        }
        Action::RefreshPlaylistsView => {
            // Load data for the current mode if needed and reset playlist_nav
            match state.playlists_mode {
                PlaylistsMode::All => {
                    // All playlists - reload if empty
                    if state.playlists.is_empty() {
                        follow_ups.push(Action::LoadPlaylists);
                    } else {
                        // Reset playlist_nav with playlists
                        let items = BrowseItem::from_playlists(&state.playlists);
                        state.playlist_nav.reset("playlists", items);
                    }
                }
                PlaylistsMode::Stations => {
                    // Stations mode - load stations into station_nav
                    if state.station_nav.columns.is_empty() && !state.station_nav.loading {
                        follow_ups.push(Action::LoadStations);
                    }
                }
                PlaylistsMode::RecentlyAdded => {
                    // Recently added albums
                    if state.recently_added_albums.is_empty() {
                        follow_ups.push(Action::LoadRecentlyAddedAlbums);
                    }
                    // Reset playlist_nav with recently added albums
                    let items = BrowseItem::from_albums(&state.recently_added_albums);
                    state.playlist_nav.reset("recently added", items);
                }
                PlaylistsMode::RecentlyPlayed => {
                    // Recently played albums
                    if state.recently_played_albums.is_empty() && !state.recently_played_loading {
                        follow_ups.push(Action::LoadRecentlyPlayedAlbums);
                    }
                    // Reset playlist_nav with recently played albums
                    let items = BrowseItem::from_albums(&state.recently_played_albums);
                    state.playlist_nav.reset("recently played", items);
                }
            }
        }
        Action::LoadRecentlyAddedAlbums => {
            if let Some(library_key) = &state.active_library {
                state.recently_added_loading = true;
                match client.get_recently_added_albums(library_key, 50).await {
                    Ok(albums) => {
                        state.recently_added_albums = albums;
                        state.recently_added_loading = false;
                        // Reset playlist_nav with recently added albums
                        let items = BrowseItem::from_albums(&state.recently_added_albums);
                        state.playlist_nav.reset("recently added", items);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load recently added albums: {}", e);
                        state.recently_added_loading = false;
                    }
                }
            }
        }
        Action::ToggleBrowseShuffle => {
            match state.browse_category {
                BrowseCategory::Artists => {
                    if let Some(col) = state.artist_nav.focused_mut() {
                        if col.is_shuffled() {
                            col.unshuffle();
                        } else {
                            col.shuffle();
                        }
                    }
                    state.artist_nav.truncate_right();
                }
                BrowseCategory::Playlists => {
                    if state.playlists_mode == PlaylistsMode::Stations {
                        if let Some(col) = state.station_nav.focused_mut() {
                            if col.is_shuffled() {
                                col.unshuffle();
                            } else {
                                col.shuffle();
                            }
                        }
                        state.station_nav.truncate_right_columns();
                    } else {
                        if let Some(col) = state.playlist_nav.focused_mut() {
                            if col.is_shuffled() {
                                col.unshuffle();
                            } else {
                                col.shuffle();
                            }
                        }
                        state.playlist_nav.truncate_right();
                    }
                }
                BrowseCategory::Genres => {
                    if state.genre_content_type == GenreContentType::Stations {
                        if let Some(col) = state.station_nav.focused_mut() {
                            if col.is_shuffled() {
                                col.unshuffle();
                            } else {
                                col.shuffle();
                            }
                        }
                        state.station_nav.truncate_right_columns();
                    } else {
                        if let Some(col) = state.genre_nav.focused_mut() {
                            if col.is_shuffled() {
                                col.unshuffle();
                            } else {
                                col.shuffle();
                            }
                        }
                        state.genre_nav.truncate_right();
                    }
                }
                BrowseCategory::Folders => {
                    if let Some(ref mut folder_state) = state.folder_state {
                        if let Some(col) = folder_state.focused_mut() {
                            if col.is_shuffled() {
                                col.unshuffle();
                            } else {
                                col.shuffle();
                            }
                        }
                        folder_state.columns.truncate(folder_state.focused_column + 1);
                    }
                }
            }
        }
        _ => unreachable!("dispatch_browse called with non-browse action: {:?}", action),
    }
    Ok(follow_ups)
}
