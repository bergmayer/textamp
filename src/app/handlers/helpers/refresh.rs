//! View refresh, stale data detection, and background category refresh.

use crate::app::{Action, AppState, Event};
use crate::app::state::{BrowseCategory, View};
use crate::api::PlexClient;
use crate::cache::LibraryCache;
use std::time::Duration;
use tokio::sync::mpsc;

/// Refresh the current view's category and return actions.
pub fn refresh_current_view(state: &mut AppState) -> Vec<Action> {
    use crate::app::state::{RefreshCategory, ArtistViewMode, PlaylistsMode, GenreContentType};

    // Special handling for Folders
    if state.view == View::Browse && state.browse_category == BrowseCategory::Folders {
        let subfolder_key = state.folder_state.as_ref().and_then(|folder_state| {
            if folder_state.focused_column > 0 {
                folder_state.columns.get(folder_state.focused_column)
                    .and_then(|col| col.key.clone())
            } else {
                None
            }
        });

        if let Some(folder_key) = subfolder_key {
            state.set_status("Refreshing folder...".to_string());
            return vec![Action::RefreshSubfolder(folder_key)];
        }
    }

    let category = match state.view {
        View::Browse => match state.browse_category {
            BrowseCategory::Artists => match state.artist_view_mode {
                ArtistViewMode::Artist => Some(RefreshCategory::Artists),
                ArtistViewMode::AlbumArtist => Some(RefreshCategory::Artists),
                ArtistViewMode::Album => Some(RefreshCategory::Albums),
            },
            BrowseCategory::Playlists => match state.playlists_mode {
                PlaylistsMode::All => Some(RefreshCategory::Playlists),
                PlaylistsMode::Stations => Some(RefreshCategory::Stations),
                PlaylistsMode::RecentlyAdded => Some(RefreshCategory::RecentlyAdded),
                PlaylistsMode::RecentlyPlayed => Some(RefreshCategory::RecentlyPlayed),
            },
            BrowseCategory::Genres => match state.genre_content_type {
                GenreContentType::Genres => Some(RefreshCategory::Genres),
                GenreContentType::ArtistGenres => Some(RefreshCategory::ArtistGenres),
                GenreContentType::AlbumGenres => Some(RefreshCategory::AlbumGenres),
                GenreContentType::Moods => Some(RefreshCategory::Moods),
                GenreContentType::Styles => Some(RefreshCategory::Styles),
                GenreContentType::Stations => Some(RefreshCategory::Stations),
            },
            BrowseCategory::Folders => Some(RefreshCategory::Folders),
        },
        _ => None,
    };

    if let Some(cat) = category {
        if !state.background_refresh_in_progress.contains(&cat) {
            state.set_status(format!("Refreshing {}...", cat.display_name()));
            return vec![Action::RefreshCategory(cat)];
        }
    }
    vec![]
}

/// Check if the user is currently viewing a specific category.
pub fn is_viewing_category(category: &crate::app::state::RefreshCategory, state: &AppState) -> bool {
    use crate::app::state::{RefreshCategory, ArtistViewMode, PlaylistsMode, GenreContentType};

    if state.view != View::Browse {
        return false;
    }

    match (state.browse_category, category) {
        (BrowseCategory::Artists, RefreshCategory::Artists) => {
            matches!(state.artist_view_mode, ArtistViewMode::Artist)
        }
        (BrowseCategory::Artists, RefreshCategory::AlbumArtists) => {
            matches!(state.artist_view_mode, ArtistViewMode::AlbumArtist)
        }
        (BrowseCategory::Artists, RefreshCategory::Albums) => {
            matches!(state.artist_view_mode, ArtistViewMode::Album)
        }
        (BrowseCategory::Playlists, RefreshCategory::Playlists) => {
            matches!(state.playlists_mode, PlaylistsMode::All)
        }
        (BrowseCategory::Playlists, RefreshCategory::RecentlyAdded) => {
            matches!(state.playlists_mode, PlaylistsMode::RecentlyAdded)
        }
        (BrowseCategory::Playlists, RefreshCategory::RecentlyPlayed) => {
            matches!(state.playlists_mode, PlaylistsMode::RecentlyPlayed)
        }
        (BrowseCategory::Playlists, RefreshCategory::Stations) => {
            matches!(state.playlists_mode, PlaylistsMode::Stations)
        }
        (BrowseCategory::Genres, RefreshCategory::Genres) => {
            matches!(state.genre_content_type, GenreContentType::Genres)
        }
        (BrowseCategory::Genres, RefreshCategory::ArtistGenres) => {
            matches!(state.genre_content_type, GenreContentType::ArtistGenres)
        }
        (BrowseCategory::Genres, RefreshCategory::AlbumGenres) => {
            matches!(state.genre_content_type, GenreContentType::AlbumGenres)
        }
        (BrowseCategory::Genres, RefreshCategory::Moods) => {
            matches!(state.genre_content_type, GenreContentType::Moods)
        }
        (BrowseCategory::Genres, RefreshCategory::Styles) => {
            matches!(state.genre_content_type, GenreContentType::Styles)
        }
        (BrowseCategory::Genres, RefreshCategory::Stations) => {
            matches!(state.genre_content_type, GenreContentType::Stations)
        }
        (BrowseCategory::Folders, RefreshCategory::Folders) => true,
        _ => false,
    }
}

/// Check for very stale cache and refresh in background when user is idle.
pub fn maybe_refresh_very_stale(event_tx: &mpsc::Sender<Event>, state: &mut AppState, client: &PlexClient) {
    use crate::app::state::RefreshCategory;

    if state.last_input_time.elapsed() < Duration::from_secs(120) {
        return;
    }

    if !state.background_refresh_in_progress.is_empty() {
        return;
    }

    let lib_key = match &state.active_library {
        Some(k) => k.clone(),
        None => return,
    };

    for category in RefreshCategory::all() {
        if is_viewing_category(category, state) {
            continue;
        }

        if is_category_very_stale(*category, state) {
            tracing::info!("Very stale background refresh: {:?}", category);
            spawn_category_refresh(event_tx, *category, &lib_key, state, client);
            break;
        }
    }
}

/// Check if a category's data is very stale (32+ days old).
pub fn is_category_very_stale(category: crate::app::state::RefreshCategory, state: &AppState) -> bool {
    use crate::app::state::RefreshCategory;

    let lib_key = match &state.active_library {
        Some(k) => k,
        None => return false,
    };

    if let Some(cache) = LibraryCache::new() {
        if let Some(data) = cache.load(lib_key) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);

            let age = now.saturating_sub(data.timestamp);
            let very_stale_threshold = crate::plex::VERY_STALE_CACHE_SECS;

            let has_data = match category {
                RefreshCategory::Artists | RefreshCategory::AlbumArtists => !data.artists.is_empty(),
                RefreshCategory::Albums => !data.albums.is_empty(),
                RefreshCategory::Playlists => !data.playlists.is_empty(),
                RefreshCategory::RecentlyAdded => !data.recently_added_albums.is_empty(),
                RefreshCategory::RecentlyPlayed => !data.recently_played_albums.is_empty(),
                RefreshCategory::Genres => !data.genres.is_empty(),
                RefreshCategory::ArtistGenres => !data.artist_genres.is_empty(),
                RefreshCategory::AlbumGenres => !data.album_genres.is_empty(),
                RefreshCategory::Moods => !data.moods.is_empty(),
                RefreshCategory::Styles => !data.styles.is_empty(),
                RefreshCategory::Stations => !data.stations.is_empty(),
                RefreshCategory::Folders => !data.root_folders.is_empty(),
            };

            return has_data && age > very_stale_threshold;
        }
    }
    false
}

/// Spawn a background refresh task for a category.
pub fn spawn_category_refresh(
    event_tx: &mpsc::Sender<Event>,
    category: crate::app::state::RefreshCategory,
    lib_key: &str,
    state: &mut AppState,
    client: &PlexClient,
) {
    use crate::app::state::RefreshCategory;

    state.background_refresh_in_progress.insert(category);

    let old_count = match category {
        RefreshCategory::Artists | RefreshCategory::AlbumArtists => state.artists.len(),
        RefreshCategory::Albums => state.albums.len(),
        RefreshCategory::Playlists => state.playlists.len(),
        RefreshCategory::RecentlyAdded => state.recently_added_albums.len(),
        RefreshCategory::RecentlyPlayed => state.recently_played_albums.len(),
        RefreshCategory::Genres => state.genres.len(),
        RefreshCategory::ArtistGenres => state.artist_genres.len(),
        RefreshCategory::AlbumGenres => state.album_genres.len(),
        RefreshCategory::Moods => state.moods.len(),
        RefreshCategory::Styles => state.styles.len(),
        RefreshCategory::Stations => state.stations.len(),
        RefreshCategory::Folders => state.folder_state.as_ref().map(|f| f.columns.first().map(|c| c.items.len()).unwrap_or(0)).unwrap_or(0),
    };

    let event_tx = event_tx.clone();
    let lib_key = lib_key.to_string();
    let client = client.clone();

    tokio::spawn(async move {
        let changed = match category {
            RefreshCategory::Artists | RefreshCategory::AlbumArtists => {
                match client.get_artists(&lib_key).await {
                    Ok(artists) => {
                        let new_count = artists.len();
                        let _ = event_tx.send(Event::ArtistsPreloaded { library_key: lib_key.clone(), artists }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh artists: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::Albums => {
                match client.get_albums(&lib_key).await {
                    Ok(albums) => {
                        let new_count = albums.len();
                        let _ = event_tx.send(Event::AlbumsPreloaded { library_key: lib_key.clone(), albums }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh albums: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::Playlists => {
                match client.get_playlists(Some(&lib_key)).await {
                    Ok(playlists) => {
                        let new_count = playlists.len();
                        let _ = event_tx.send(Event::PlaylistsPreloaded { library_key: lib_key.clone(), playlists }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh playlists: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::RecentlyAdded => {
                match client.get_recently_added_albums(&lib_key, 50).await {
                    Ok(albums) => {
                        let new_count = albums.len();
                        let _ = event_tx.send(Event::RecentlyAddedPreloaded { library_key: lib_key.clone(), albums }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh recently added: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::RecentlyPlayed => {
                match client.get_recently_played_albums(&lib_key, 50).await {
                    Ok(albums) => {
                        let new_count = albums.len();
                        let _ = event_tx.send(Event::RecentlyPlayedPreloaded { library_key: lib_key.clone(), albums }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh recently played: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::Genres => {
                match client.get_genres(&lib_key).await {
                    Ok(genres) => {
                        let new_count = genres.len();
                        let _ = event_tx.send(Event::GenresPreloaded { library_key: lib_key.clone(), genres }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh genres: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::ArtistGenres => {
                match client.get_artist_genres(&lib_key).await {
                    Ok(genres) => {
                        let new_count = genres.len();
                        let _ = event_tx.send(Event::ArtistGenresPreloaded { library_key: lib_key.clone(), genres }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh artist genres: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::AlbumGenres => {
                match client.get_album_genres(&lib_key).await {
                    Ok(genres) => {
                        let new_count = genres.len();
                        let _ = event_tx.send(Event::AlbumGenresPreloaded { library_key: lib_key.clone(), genres }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh album genres: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::Moods => {
                match client.get_moods(&lib_key).await {
                    Ok(moods) => {
                        let new_count = moods.len();
                        let _ = event_tx.send(Event::MoodsPreloaded { library_key: lib_key.clone(), moods }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh moods: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::Styles => {
                match client.get_styles(&lib_key).await {
                    Ok(styles) => {
                        let new_count = styles.len();
                        let _ = event_tx.send(Event::StylesPreloaded { library_key: lib_key.clone(), styles }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh styles: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::Stations => {
                match client.get_stations(&lib_key).await {
                    Ok(stations) => {
                        let new_count = stations.len();
                        let _ = event_tx.send(Event::StationsPreloaded { library_key: lib_key.clone(), stations }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh stations: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::Folders => {
                use crate::services::{FolderColumn, FolderNavigationState, FolderService};
                match client.get_library_folders(&lib_key).await {
                    Ok(response) => {
                        let items = FolderService::from_response(&response);
                        let new_count = items.len();
                        let root_column = FolderColumn::new(None, "Music".to_string(), items);
                        let folder_state = FolderNavigationState::with_root(lib_key.clone(), root_column);
                        let _ = event_tx.send(Event::FoldersPreloaded { library_key: lib_key.clone(), folder_state }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh folders: {}", e);
                        false
                    }
                }
            }
        };

        let _ = event_tx.send(Event::CacheRefreshCompleted { category, changed }).await;
    });
}
