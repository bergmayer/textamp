//! Browse dispatch handlers: LoadStations, LoadGenres, LoadArtistGenres, LoadAlbumGenres,
//! LoadMoods, LoadStyles, LoadGenreAlbums, LoadArtistGenreAlbums, LoadAlbumGenreAlbums,
//! LoadMoodAlbums, LoadStyleAlbums, CycleGenreContentType, RefreshGenreView,
//! RefreshArtistView, CycleGenreTab, SetGenreTab.

use crate::app::{Action, AppState, Event};
use crate::app::action::{BrowseAction, MillerAction, SystemAction};
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
    action: BrowseAction,
    state: &mut AppState,
    client: &mut PlexClient,
) -> Result<Vec<Action>> {
    let mut follow_ups = vec![];

    match action {
        BrowseAction::LoadStations => {
            if let Some(lib_key) = &state.active_library.clone() {
                state.stations_loading = true;
                state.station_nav.loading = true;
                match client.get_stations(lib_key).await {
                    Ok(mut stations) => {
                        helpers::append_station_action_items(&mut stations, state.queue.shuffle_undo_queue.is_some());

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
        BrowseAction::LoadGenres => {
            if let Some(lib_key) = &state.active_library.clone() {
                state.library.genres_loading = true;
                state.genre_nav.loading = true;
                match client.get_genres(lib_key).await {
                    Ok(genres) => {
                        state.library.genres = genres;
                        state.library.genres_loading = false;
                        state.library.genres_index = 0;
                        // Re-drill the current category to populate column 1
                        if let Some(cat_key) = genre_selected_category_key(state) {
                            follow_ups.push(BrowseAction::DrillGenreCategory { category_key: cat_key }.into());
                        } else {
                            state.genre_nav.loading = false;
                        }
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load genres: {}", e));
                        state.library.genres_loading = false;
                        state.genre_nav.loading = false;
                    }
                }
            }
        }
        BrowseAction::LoadArtistGenres => {
            if let Some(lib_key) = &state.active_library.clone() {
                state.library.artist_genres_loading = true;
                state.genre_nav.loading = true;
                match client.get_artist_genres(lib_key).await {
                    Ok(genres) => {
                        state.library.artist_genres = genres;
                        state.library.artist_genres_loading = false;
                        state.library.genres_index = 0;
                        if let Some(cat_key) = genre_selected_category_key(state) {
                            follow_ups.push(BrowseAction::DrillGenreCategory { category_key: cat_key }.into());
                        } else {
                            state.genre_nav.loading = false;
                        }
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load artist genres: {}", e));
                        state.library.artist_genres_loading = false;
                        state.genre_nav.loading = false;
                    }
                }
            }
        }
        BrowseAction::LoadAlbumGenres => {
            if let Some(lib_key) = &state.active_library.clone() {
                state.library.album_genres_loading = true;
                state.genre_nav.loading = true;
                match client.get_album_genres(lib_key).await {
                    Ok(genres) => {
                        state.library.album_genres = genres;
                        state.library.album_genres_loading = false;
                        state.library.genres_index = 0;
                        if let Some(cat_key) = genre_selected_category_key(state) {
                            follow_ups.push(BrowseAction::DrillGenreCategory { category_key: cat_key }.into());
                        } else {
                            state.genre_nav.loading = false;
                        }
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load album genres: {}", e));
                        state.library.album_genres_loading = false;
                        state.genre_nav.loading = false;
                    }
                }
            }
        }
        BrowseAction::LoadMoods => {
            if let Some(lib_key) = &state.active_library.clone() {
                state.library.moods_loading = true;
                state.genre_nav.loading = true;
                match client.get_moods(lib_key).await {
                    Ok(moods) => {
                        state.library.moods = moods;
                        state.library.moods_loading = false;
                        if let Some(cat_key) = genre_selected_category_key(state) {
                            follow_ups.push(BrowseAction::DrillGenreCategory { category_key: cat_key }.into());
                        } else {
                            state.genre_nav.loading = false;
                        }
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load moods: {}", e));
                        state.library.moods_loading = false;
                        state.genre_nav.loading = false;
                    }
                }
            }
        }
        BrowseAction::LoadStyles => {
            if let Some(lib_key) = &state.active_library.clone() {
                state.library.styles_loading = true;
                state.genre_nav.loading = true;
                match client.get_styles(lib_key).await {
                    Ok(styles) => {
                        state.library.styles = styles;
                        state.library.styles_loading = false;
                        if let Some(cat_key) = genre_selected_category_key(state) {
                            follow_ups.push(BrowseAction::DrillGenreCategory { category_key: cat_key }.into());
                        } else {
                            state.genre_nav.loading = false;
                        }
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load styles: {}", e));
                        state.library.styles_loading = false;
                        state.genre_nav.loading = false;
                    }
                }
            }
        }
        BrowseAction::LoadGenreAlbums => {
            // Get the selected genre based on current content type
            let genre = state.current_genre_list().get(state.library.genres_index).cloned();

            if let Some(genre) = genre {
                let genre_key = genre.effective_key().to_string();
                let lib_key = state.active_library.clone();
                state.library.right_panel_loading = true;
                state.library.genre_albums.clear();
                state.library.genre_albums_index = 0;

                if genre_key.is_empty() {
                    state.set_error("Genre/mood/style has no valid key".to_string());
                    state.library.right_panel_loading = false;
                } else if let Some(lib_key) = lib_key {
                    let result = match state.library.genre_content_type {
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
                            state.library.genre_albums = albums;
                        }
                        Err(e) => {
                            state.set_error(format!("Failed to load albums: {}", e));
                        }
                    }
                    state.library.right_panel_loading = false;
                } else {
                    state.set_error("No library selected".to_string());
                    state.library.right_panel_loading = false;
                }
            }
        }
        BrowseAction::RefreshGenreView => {
            state.library.genres_index = 0;
            state.library.genre_albums.clear();
            state.library.genre_albums_index = 0;

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
                        follow_ups.push(BrowseAction::DrillGenreCategory { category_key: key }.into());
                    }
                }
            }
        }
        BrowseAction::CycleGenreTab => {
            state.genre_tab = state.genre_tab.next();

            // Update genre_content_type from tab (for non-All tabs)
            if let Some(ct) = state.genre_tab.to_content_type() {
                state.library.genre_content_type = ct;
            }
            follow_ups.push(BrowseAction::RefreshGenreView.into());
            // Check staleness for the relevant category
            let tier1 = match state.genre_tab {
                GenreTab::All => RefreshCategory::Genres, // Check genres as representative
                GenreTab::Library => RefreshCategory::Genres,
                GenreTab::Artist => RefreshCategory::ArtistGenres,
                GenreTab::Album => RefreshCategory::AlbumGenres,
                GenreTab::Mood => RefreshCategory::Moods,
                GenreTab::Style => RefreshCategory::Styles,
            };
            follow_ups.push(SystemAction::CheckStaleness(tier1).into());
        }
        BrowseAction::SetGenreTab(tab) => {
            if state.genre_tab != tab {
                state.genre_tab = tab;

                if let Some(ct) = tab.to_content_type() {
                    state.library.genre_content_type = ct;
                }
                follow_ups.push(BrowseAction::RefreshGenreView.into());
            }
        }
        BrowseAction::DrillGenreCategory { category_key } => {
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
                        if state.library.genres.is_empty() { follow_ups.push(BrowseAction::LoadGenres.into()); }
                        if state.library.artist_genres.is_empty() { follow_ups.push(BrowseAction::LoadArtistGenres.into()); }
                        if state.library.album_genres.is_empty() { follow_ups.push(BrowseAction::LoadAlbumGenres.into()); }
                        if state.library.moods.is_empty() { follow_ups.push(BrowseAction::LoadMoods.into()); }
                        if state.library.styles.is_empty() { follow_ups.push(BrowseAction::LoadStyles.into()); }
                        state.genre_nav.loading = true;
                    }
                    merged
                }
                "genre_cat:library" => {
                    if state.library.genres.is_empty() {
                        follow_ups.push(BrowseAction::LoadGenres.into());
                        state.genre_nav.loading = true;
                        vec![]
                    } else {
                        BrowseItem::from_genres(&state.library.genres)
                    }
                }
                "genre_cat:artist" => {
                    if state.library.artist_genres.is_empty() {
                        follow_ups.push(BrowseAction::LoadArtistGenres.into());
                        state.genre_nav.loading = true;
                        vec![]
                    } else {
                        BrowseItem::from_genres(&state.library.artist_genres)
                    }
                }
                "genre_cat:album" => {
                    if state.library.album_genres.is_empty() {
                        follow_ups.push(BrowseAction::LoadAlbumGenres.into());
                        state.genre_nav.loading = true;
                        vec![]
                    } else {
                        BrowseItem::from_genres(&state.library.album_genres)
                    }
                }
                "genre_cat:mood" => {
                    if state.library.moods.is_empty() {
                        follow_ups.push(BrowseAction::LoadMoods.into());
                        state.genre_nav.loading = true;
                        vec![]
                    } else {
                        BrowseItem::from_genres(&state.library.moods)
                    }
                }
                "genre_cat:style" => {
                    if state.library.styles.is_empty() {
                        follow_ups.push(BrowseAction::LoadStyles.into());
                        state.genre_nav.loading = true;
                        vec![]
                    } else {
                        BrowseItem::from_genres(&state.library.styles)
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
        BrowseAction::OpenTrackDetails(track) => {
            state.track_details = Some(track);
        }
        BrowseAction::CloseTrackDetails => {
            state.track_details = None;
        }
        BrowseAction::OpenInLibrary { artist_key, artist_name, album_key, album_title } => {
            // Mirrors the Ctrl+J keyboard handler: switch to the
            // Library category, set the pending album for auto-select,
            // then trigger the artist's albums load. See
            // `key_input::mod.rs::navigate_to_album` for the original
            // sequence.
            state.track_details = None;
            if let Some(ak) = album_key {
                state.search.pending_album_key = Some(ak);
            }
            if let Some(at) = album_title {
                state.library.selected_album_title = at;
            }
            state.library.selected_artist_name = artist_name;
            state.set_view(crate::app::state::View::Browse);
            state.set_browse_category(crate::app::state::BrowseCategory::Library);

            // Reset the artist nav back to a single root column. If the
            // user opened "in Library" from a category that hadn't built
            // the artist root yet (Search, Folders, …), build it now so
            // the highlighted artist row is visible immediately.
            if state.artist_nav.columns.is_empty() {
                let items = state.build_artist_root_items();
                let title = format!("artists ({})", state.library.artists.len());
                state.artist_nav.columns.push(BrowseColumn::new(title, items));
            } else {
                state.artist_nav.columns.truncate(1);
            }
            state.artist_nav.focused_column = 0;

            // Selection lives on the column items, not the raw
            // library.artists list. The column has pinned rows
            // (AllArtists, optionally Compilations) prepended and
            // compilation-only artists filtered out, so the indices
            // diverge — match by key instead.
            if let Some(col) = state.artist_nav.columns.first_mut() {
                if let Some(idx) = col.items.iter().position(|item| {
                    matches!(item, BrowseItem::Artist { key, .. } if key == &artist_key)
                }) {
                    col.selected_index = idx;
                }
            }

            // Keep legacy list_state in sync for any non-Miller
            // consumers that still read it.
            if let Some(idx) = state.library.artists.iter().position(|a| a.rating_key == artist_key) {
                state.list_state.artists_index = idx;
            }

            follow_ups.push(MillerAction::LoadArtistAlbumsForMiller { artist_key }.into());
        }
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

    for g in &state.library.genres {
        items.push(BrowseItem::Genre {
            key: format!("lib:{}", g.key),
            title: format!("{} (Library)", g.title),
        });
    }
    for g in &state.library.artist_genres {
        items.push(BrowseItem::Genre {
            key: format!("art:{}", g.key),
            title: format!("{} (Artist)", g.title),
        });
    }
    for g in &state.library.album_genres {
        items.push(BrowseItem::Genre {
            key: format!("alb:{}", g.key),
            title: format!("{} (Album)", g.title),
        });
    }
    for g in &state.library.moods {
        items.push(BrowseItem::Genre {
            key: format!("mood:{}", g.key),
            title: format!("{} (Mood)", g.title),
        });
    }
    for g in &state.library.styles {
        items.push(BrowseItem::Genre {
            key: format!("style:{}", g.key),
            title: format!("{} (Style)", g.title),
        });
    }

    items.sort_by(|a, b| a.title().to_lowercase().cmp(&b.title().to_lowercase()));
    items
}
