//! Browse dispatch handlers: LoadStations, LoadGenres, LoadArtistGenres, LoadAlbumGenres,
//! LoadMoods, LoadStyles, LoadGenreAlbums, LoadArtistGenreAlbums, LoadAlbumGenreAlbums,
//! LoadMoodAlbums, LoadStyleAlbums, CycleGenreContentType, RefreshGenreView,
//! RefreshArtistView, CycleGenreTab, SetGenreTab.

use crate::app::{Action, AppState, Event};
use crate::app::state::{
    BrowseColumn, BrowseItem,
    GenreContentType, GenreTab, RefreshCategory, StationColumn,
};
use crate::plex::PlexClient;

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
                            "stations".to_string(),
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
                        state.genres = genres;
                        state.genres_loading = false;
                        state.genres_index = 0;
                        // Re-drill the current category to populate column 1
                        if let Some(cat_key) = genre_selected_category_key(state) {
                            follow_ups.push(Action::DrillGenreCategory { category_key: cat_key });
                        } else {
                            state.genre_nav.loading = false;
                        }
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
                        state.artist_genres = genres;
                        state.artist_genres_loading = false;
                        state.genres_index = 0;
                        if let Some(cat_key) = genre_selected_category_key(state) {
                            follow_ups.push(Action::DrillGenreCategory { category_key: cat_key });
                        } else {
                            state.genre_nav.loading = false;
                        }
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
                        state.album_genres = genres;
                        state.album_genres_loading = false;
                        state.genres_index = 0;
                        if let Some(cat_key) = genre_selected_category_key(state) {
                            follow_ups.push(Action::DrillGenreCategory { category_key: cat_key });
                        } else {
                            state.genre_nav.loading = false;
                        }
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
                        state.moods = moods;
                        state.moods_loading = false;
                        if let Some(cat_key) = genre_selected_category_key(state) {
                            follow_ups.push(Action::DrillGenreCategory { category_key: cat_key });
                        } else {
                            state.genre_nav.loading = false;
                        }
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
                        state.styles = styles;
                        state.styles_loading = false;
                        if let Some(cat_key) = genre_selected_category_key(state) {
                            follow_ups.push(Action::DrillGenreCategory { category_key: cat_key });
                        } else {
                            state.genre_nav.loading = false;
                        }
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
        Action::RefreshGenreView => {
            state.genres_index = 0;
            state.genre_albums.clear();
            state.genre_albums_index = 0;

            // Populate column 0 with genre category items (Miller-style navigation)
            let categories = vec![
                BrowseItem::GenreCategory { key: "genre_cat:all".to_string(), title: "All".to_string() },
                BrowseItem::GenreCategory { key: "genre_cat:library".to_string(), title: "Library".to_string() },
                BrowseItem::GenreCategory { key: "genre_cat:artist".to_string(), title: "Artist".to_string() },
                BrowseItem::GenreCategory { key: "genre_cat:album".to_string(), title: "Album".to_string() },
                BrowseItem::GenreCategory { key: "genre_cat:mood".to_string(), title: "Mood".to_string() },
                BrowseItem::GenreCategory { key: "genre_cat:style".to_string(), title: "Style".to_string() },
            ];

            // Only reset if genre_nav is empty (first load) — otherwise preserve column 0 selection
            if state.genre_nav.columns.is_empty() {
                state.genre_nav.reset("genres", categories);
            } else if state.genre_nav.columns.first().map_or(true, |c| {
                c.items.first().map_or(true, |i| !matches!(i, BrowseItem::GenreCategory { .. }))
            }) {
                // Column 0 doesn't have categories yet — reset
                state.genre_nav.reset("genres", categories);
            }
            // If already showing categories with a selection, auto-drill into the selected one
            if state.genre_nav.focused_column == 0 {
                if let Some(item) = state.genre_nav.selected_item().cloned() {
                    if let BrowseItem::GenreCategory { key, .. } = item {
                        state.auto_drill_pending = true;
                        follow_ups.push(Action::DrillGenreCategory { category_key: key });
                    }
                }
            }
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
        Action::DrillGenreCategory { category_key } => {
            let auto_drill = std::mem::take(&mut state.auto_drill_pending);
            // Drill from genre category (column 0) into genre list (column 1)
            // Truncate to column 0 before loading new content
            state.genre_nav.columns.truncate(1);
            state.genre_nav.focused_column = 0;

            let items = match category_key.as_str() {
                "genre_cat:all" => {
                    let merged = build_merged_genres(state);
                    if merged.is_empty() {
                        // Trigger loads for all genre types if empty
                        if state.genres.is_empty() { follow_ups.push(Action::LoadGenres); }
                        if state.artist_genres.is_empty() { follow_ups.push(Action::LoadArtistGenres); }
                        if state.album_genres.is_empty() { follow_ups.push(Action::LoadAlbumGenres); }
                        if state.moods.is_empty() { follow_ups.push(Action::LoadMoods); }
                        if state.styles.is_empty() { follow_ups.push(Action::LoadStyles); }
                        state.genre_nav.loading = true;
                    }
                    merged
                }
                "genre_cat:library" => {
                    if state.genres.is_empty() {
                        follow_ups.push(Action::LoadGenres);
                        state.genre_nav.loading = true;
                        vec![]
                    } else {
                        BrowseItem::from_genres(&state.genres)
                    }
                }
                "genre_cat:artist" => {
                    if state.artist_genres.is_empty() {
                        follow_ups.push(Action::LoadArtistGenres);
                        state.genre_nav.loading = true;
                        vec![]
                    } else {
                        BrowseItem::from_genres(&state.artist_genres)
                    }
                }
                "genre_cat:album" => {
                    if state.album_genres.is_empty() {
                        follow_ups.push(Action::LoadAlbumGenres);
                        state.genre_nav.loading = true;
                        vec![]
                    } else {
                        BrowseItem::from_genres(&state.album_genres)
                    }
                }
                "genre_cat:mood" => {
                    if state.moods.is_empty() {
                        follow_ups.push(Action::LoadMoods);
                        state.genre_nav.loading = true;
                        vec![]
                    } else {
                        BrowseItem::from_genres(&state.moods)
                    }
                }
                "genre_cat:style" => {
                    if state.styles.is_empty() {
                        follow_ups.push(Action::LoadStyles);
                        state.genre_nav.loading = true;
                        vec![]
                    } else {
                        BrowseItem::from_genres(&state.styles)
                    }
                }
                _ => vec![],
            };

            if !items.is_empty() {
                let title = match category_key.as_str() {
                    "genre_cat:all" => "all genres",
                    "genre_cat:library" => "library genres",
                    "genre_cat:artist" => "artist genres",
                    "genre_cat:album" => "album genres",
                    "genre_cat:mood" => "moods",
                    "genre_cat:style" => "styles",
                    _ => "genres",
                };
                let col = BrowseColumn::new(title, items);
                state.genre_nav.drill_column(col, auto_drill);
            }
        }
        _ => unreachable!("dispatch_browse called with non-browse action: {:?}", action),
    }
    Ok(follow_ups)
}

/// Build a merged genre list from all genre types with type suffixes.
/// Items are sorted alphabetically by title, with type suffix for disambiguation.
/// Get the selected genre category key from column 0 (if a GenreCategory is selected).
fn genre_selected_category_key(state: &AppState) -> Option<String> {
    state.genre_nav.columns.first()
        .and_then(|c| c.items.get(c.selected_index))
        .and_then(|item| match item {
            BrowseItem::GenreCategory { key, .. } => Some(key.clone()),
            _ => None,
        })
}

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
