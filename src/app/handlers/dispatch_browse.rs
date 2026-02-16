//! Browse dispatch handlers: LoadStations, LoadGenres, LoadArtistGenres, LoadAlbumGenres,
//! LoadMoods, LoadStyles, LoadGenreAlbums, LoadArtistGenreAlbums, LoadAlbumGenreAlbums,
//! LoadMoodAlbums, LoadStyleAlbums, CycleGenreContentType, RefreshGenreView,
//! CycleArtistViewMode, RefreshArtistView, CycleGenreTab, SetGenreTab.

use crate::app::{Action, AppState, Event};
use crate::app::state::{
    BrowseCategory, BrowseItem, BrowseNavigationState, Focus,
    GenreContentType, GenreTab, LibrarySubMode, RefreshCategory, RightPanelMode, StationColumn,
};
use crate::api::PlexClient;

use anyhow::Result;

use super::helpers;
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
                    Ok(mut stations) => {
                        helpers::append_station_action_items(&mut stations, state.shuffle_undo_queue.is_some());

                        // Initialize with root column
                        state.station_nav.columns.clear();
                        state.station_nav.columns.push(StationColumn::new(
                            None,
                            "Radio".to_string(),
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
            let tier1 = match state.genre_content_type {
                GenreContentType::Genres => RefreshCategory::Genres,
                GenreContentType::ArtistGenres => RefreshCategory::ArtistGenres,
                GenreContentType::AlbumGenres => RefreshCategory::AlbumGenres,
                GenreContentType::Moods => RefreshCategory::Moods,
                GenreContentType::Styles => RefreshCategory::Styles,
            };
            follow_ups.push(Action::CheckStaleness(tier1));
        }
        Action::RefreshGenreView => {
            state.genres_index = 0;
            state.genre_albums.clear();
            state.genre_albums_index = 0;

            // Reset genre_nav when cycling
            state.genre_nav = BrowseNavigationState::new();

            // Handle "All" tab separately - builds merged list
            if state.genre_tab == GenreTab::All {
                let items = build_merged_genres(state);
                if items.is_empty() {
                    // If all lists are empty, trigger loads for all genre types
                    if state.genres.is_empty() { follow_ups.push(Action::LoadGenres); }
                    if state.artist_genres.is_empty() { follow_ups.push(Action::LoadArtistGenres); }
                    if state.album_genres.is_empty() { follow_ups.push(Action::LoadAlbumGenres); }
                    if state.moods.is_empty() { follow_ups.push(Action::LoadMoods); }
                    if state.styles.is_empty() { follow_ups.push(Action::LoadStyles); }
                } else {
                    state.genre_nav.reset("all genres", items);
                }
            } else {
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
                }
            }
        }
        Action::CycleArtistViewMode => {
            state.artist_view_mode = state.artist_view_mode.next();
            follow_ups.push(Action::RefreshArtistView);
            follow_ups.push(Action::CheckStaleness(RefreshCategory::Artists));
        }
        Action::RefreshArtistView => {
            // Clear right panel state (drill-down state) but keep preloaded data
            state.selected_artist_albums.clear();
            state.selected_album_tracks.clear();
            state.list_state.artists_index = 0;
            state.list_state.albums_index = 0;
            state.right_panel_mode = RightPanelMode::Empty;
            state.focus = Focus::Left;

            if state.artists.is_empty() {
                follow_ups.push(Action::LoadArtists);
            }
            // Reset artist_nav with new data
            let title = state.artist_view_mode.name();
            let items = state.build_artist_root_items();
            state.artist_nav.reset(title, items);
        }
        Action::CycleGenreTab => {
            state.genre_tab = state.genre_tab.next();

            // Update genre_content_type from tab (for non-All tabs)
            if let Some(ct) = state.genre_tab.to_content_type() {
                state.genre_content_type = ct;
            }
            follow_ups.push(Action::RefreshGenreView);
            // Check staleness for the relevant category
            let tier1 = match state.genre_tab {
                GenreTab::All => RefreshCategory::Genres, // Check genres as representative
                GenreTab::Library => RefreshCategory::Genres,
                GenreTab::Artist => RefreshCategory::ArtistGenres,
                GenreTab::Album => RefreshCategory::AlbumGenres,
                GenreTab::Mood => RefreshCategory::Moods,
                GenreTab::Style => RefreshCategory::Styles,
            };
            follow_ups.push(Action::CheckStaleness(tier1));
        }
        Action::SetGenreTab(tab) => {
            if state.genre_tab != tab {
                state.genre_tab = tab;
    
                if let Some(ct) = tab.to_content_type() {
                    state.genre_content_type = ct;
                }
                follow_ups.push(Action::RefreshGenreView);
            }
        }
        Action::CycleLibrarySubMode => {
            state.library_sub_mode = state.library_sub_mode.next();
            match state.library_sub_mode {
                LibrarySubMode::Normal => {
                    // Reset to normal artist list
                    let title = state.artist_view_mode.name();
                    let items = state.build_artist_root_items();
                    state.artist_nav.reset(title, items);
                }
                LibrarySubMode::AllByArtist => {
                    // All albums sorted by artist, then by year
                    let mut sorted = state.albums.clone();
                    sorted.sort_by(|a, b| {
                        a.artist_name().to_lowercase().cmp(&b.artist_name().to_lowercase())
                            .then_with(|| a.year.cmp(&b.year))
                    });
                    let items = BrowseItem::from_albums(&sorted);
                    state.artist_nav.reset("all albums", items);
                }
                LibrarySubMode::AllShuffled => {
                    // Shuffle the current column
                    if let Some(col) = state.artist_nav.columns.first_mut() {
                        col.shuffle();
                        col.title = "all albums (shuffled)".to_string();
                    }
                }
            }
        }
        Action::ToggleBrowseShuffle => {
            match state.browse_category {
                BrowseCategory::Library => {
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
                    if let Some(col) = state.playlist_nav.focused_mut() {
                        if col.is_shuffled() {
                            col.unshuffle();
                        } else {
                            col.shuffle();
                        }
                    }
                    state.playlist_nav.truncate_right();
                }
                BrowseCategory::Genres => {
                    if let Some(col) = state.genre_nav.focused_mut() {
                        if col.is_shuffled() {
                            col.unshuffle();
                        } else {
                            col.shuffle();
                        }
                    }
                    state.genre_nav.truncate_right();
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
                        folder_state.truncate_right();
                    }
                }
            }
        }
        _ => unreachable!("dispatch_browse called with non-browse action: {:?}", action),
    }
    Ok(follow_ups)
}

/// Build a merged genre list from all genre types with type suffixes.
/// Items are sorted alphabetically by title, with type suffix for disambiguation.
fn build_merged_genres(state: &AppState) -> Vec<BrowseItem> {
    let mut items = Vec::new();

    for g in &state.genres {
        items.push(BrowseItem::Genre {
            key: format!("lib:{}", g.key),
            title: format!("{} (Library)", g.title),
        });
    }
    for g in &state.artist_genres {
        items.push(BrowseItem::Genre {
            key: format!("art:{}", g.key),
            title: format!("{} (Artist)", g.title),
        });
    }
    for g in &state.album_genres {
        items.push(BrowseItem::Genre {
            key: format!("alb:{}", g.key),
            title: format!("{} (Album)", g.title),
        });
    }
    for g in &state.moods {
        items.push(BrowseItem::Genre {
            key: format!("mood:{}", g.key),
            title: format!("{} (Mood)", g.title),
        });
    }
    for g in &state.styles {
        items.push(BrowseItem::Genre {
            key: format!("style:{}", g.key),
            title: format!("{} (Style)", g.title),
        });
    }

    items.sort_by(|a, b| a.title().to_lowercase().cmp(&b.title().to_lowercase()));
    items
}
