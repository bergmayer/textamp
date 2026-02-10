//! View refresh, stale data detection, and background category refresh.

use crate::app::{Action, AppState, Event};
use crate::app::state::{BrowseCategory, View};
use crate::api::PlexClient;
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

/// Map current view state to its primary RefreshCategory.
pub fn current_view_category(state: &AppState) -> Option<crate::app::state::RefreshCategory> {
    use crate::app::state::{RefreshCategory, ArtistViewMode, PlaylistsMode, GenreContentType};

    match state.view {
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
    }
}

/// Two-tier staleness check on view navigation.
///
/// Tier 1 (72h): The active category — refresh if >72h old or missing timestamp.
/// Tier 2 (32d): All other categories — refresh if >32 days old (skip if no timestamp).
pub fn check_staleness_on_view_load(
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,
    client: &PlexClient,
    tier1_category: crate::app::state::RefreshCategory,
) {
    use crate::app::state::RefreshCategory;

    let lib_key = match &state.active_library {
        Some(k) => k.clone(),
        None => return,
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let stale_threshold = crate::plex::constants::CACHE_STALE_THRESHOLD_SECS;
    let very_stale_threshold = crate::plex::constants::CACHE_VERY_STALE_THRESHOLD_SECS;

    // Tier 1: Active category — refresh if >72h old or no timestamp
    if !state.background_refresh_in_progress.contains(&tier1_category) {
        let is_stale = match state.category_timestamps.get(&tier1_category) {
            Some(&ts) => now.saturating_sub(ts) > stale_threshold,
            None => true, // No timestamp = never refreshed
        };
        if is_stale {
            tracing::info!("Tier-1 staleness refresh: {:?}", tier1_category);
            spawn_category_refresh(event_tx, tier1_category, &lib_key, state, client);
        }
    }

    // Tier 2: All other categories — refresh if >32 days old or no timestamp
    for &cat in RefreshCategory::all() {
        if cat == tier1_category {
            continue;
        }
        if state.background_refresh_in_progress.contains(&cat) {
            continue;
        }
        let is_stale = match state.category_timestamps.get(&cat) {
            Some(&ts) => now.saturating_sub(ts) > very_stale_threshold,
            None => true, // No timestamp = never loaded, should be fetched
        };
        if is_stale {
            tracing::info!("Tier-2 staleness refresh: {:?}", cat);
            spawn_category_refresh(event_tx, cat, &lib_key, state, client);
        }
    }
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
