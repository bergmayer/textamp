//! View refresh, stale data detection, and background category refresh.

use crate::app::event::*;
use crate::app::action::*;
use crate::app::{Action, AppState, Event};
use crate::app::state::{BrowseCategory, View};
use crate::plex::PlexClient;
use tokio::sync::mpsc;

/// Refresh the current view's category and return actions.
pub fn refresh_current_view(state: &mut AppState) -> Vec<Action> {
    use crate::app::state::{RefreshCategory, GenreContentType};

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
            return vec![FolderAction::RefreshSubfolder(folder_key).into()];
        }
    }

    // Check if we're viewing the All Library Tracks column (artist_nav, All Artists → All Tracks)
    if state.view == View::Browse && state.browse_category == BrowseCategory::Library {
        if state.artist_nav.focused_column >= 2 {
            // Check if parent column's selected item is the "__all_library__" AllTracks entry
            if let Some(parent_col) = state.artist_nav.columns.get(state.artist_nav.focused_column - 1) {
                if let Some(item) = parent_col.selected_item() {
                    if matches!(item, crate::app::state::BrowseItem::AllTracks { ref artist_key, .. } if artist_key == "__all_library__") {
                        state.set_status("Refreshing all tracks...".to_string());
                        return vec![SystemAction::RefreshCategory(RefreshCategory::AllTracks).into()];
                    }
                }
            }
        }
    }

    // Check if we're viewing album tracks in a Miller column — refresh just that album
    if state.view == View::Browse {
        let album_key = match state.browse_category {
            BrowseCategory::Library => {
                // Check if focused column contains tracks loaded from an album
                // (depth >= 2: root artists → albums → tracks)
                if state.artist_nav.focused_column >= 2 {
                    state.artist_nav.focused().and_then(|col| {
                        if col.items.iter().any(|i| matches!(i, crate::app::state::BrowseItem::Track { .. })) && !col.tracks.is_empty() {
                            // The album key is on the parent column's selected item
                            state.artist_nav.columns.get(state.artist_nav.focused_column - 1)
                                .and_then(|parent| parent.selected_item())
                                .map(|item| item.key().to_string())
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            }
            BrowseCategory::Genres => {
                // Genre nav: root genres → albums → tracks (depth >= 2)
                if state.genre_nav.focused_column >= 2 {
                    state.genre_nav.focused().and_then(|col| {
                        if col.items.iter().any(|i| matches!(i, crate::app::state::BrowseItem::Track { .. })) && !col.tracks.is_empty() {
                            state.genre_nav.columns.get(state.genre_nav.focused_column - 1)
                                .and_then(|parent| parent.selected_item())
                                .map(|item| item.key().to_string())
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some(key) = album_key {
            state.set_status("Refreshing album tracks...".to_string());
            return vec![MillerAction::RefreshAlbumTracks { album_key: key }.into()];
        }
    }

    let category = match state.view {
        View::Browse => match state.browse_category {
            BrowseCategory::Library => Some(RefreshCategory::Artists),
            BrowseCategory::Playlists => Some(RefreshCategory::Playlists),
            BrowseCategory::Genres => match state.library.genre_content_type {
                GenreContentType::Genres => Some(RefreshCategory::Genres),
                GenreContentType::ArtistGenres => Some(RefreshCategory::ArtistGenres),
                GenreContentType::AlbumGenres => Some(RefreshCategory::AlbumGenres),
                GenreContentType::Moods => Some(RefreshCategory::Moods),
                GenreContentType::Styles => Some(RefreshCategory::Styles),
            },
            BrowseCategory::Folders => Some(RefreshCategory::Folders),
        },
        _ => None,
    };

    if let Some(cat) = category {
        if !state.cache_mgmt.background_refresh.contains(&cat) {
            state.set_status(format!("Refreshing {}...", cat.display_name()));
            return vec![SystemAction::RefreshCategory(cat).into()];
        }
    }
    vec![]
}

/// Check if the user is currently viewing a specific category.
pub fn is_viewing_category(category: &crate::app::state::RefreshCategory, state: &AppState) -> bool {
    use crate::app::state::{RefreshCategory, GenreContentType};

    if state.view != View::Browse {
        return false;
    }

    match (state.browse_category, category) {
        (BrowseCategory::Library, RefreshCategory::Artists) => true,
        (BrowseCategory::Library, RefreshCategory::AlbumArtists) => true,
        (BrowseCategory::Playlists, RefreshCategory::Playlists) => true,
        (BrowseCategory::Genres, RefreshCategory::Genres) => {
            matches!(state.library.genre_content_type, GenreContentType::Genres)
        }
        (BrowseCategory::Genres, RefreshCategory::ArtistGenres) => {
            matches!(state.library.genre_content_type, GenreContentType::ArtistGenres)
        }
        (BrowseCategory::Genres, RefreshCategory::AlbumGenres) => {
            matches!(state.library.genre_content_type, GenreContentType::AlbumGenres)
        }
        (BrowseCategory::Genres, RefreshCategory::Moods) => {
            matches!(state.library.genre_content_type, GenreContentType::Moods)
        }
        (BrowseCategory::Genres, RefreshCategory::Styles) => {
            matches!(state.library.genre_content_type, GenreContentType::Styles)
        }
        (BrowseCategory::Folders, RefreshCategory::Folders) => true,
        _ => false,
    }
}

/// Map current view state to its primary RefreshCategory.
pub fn current_view_category(state: &AppState) -> Option<crate::app::state::RefreshCategory> {
    use crate::app::state::{RefreshCategory, GenreContentType};

    match state.view {
        View::Browse => match state.browse_category {
            BrowseCategory::Library => Some(RefreshCategory::Artists),
            BrowseCategory::Playlists => Some(RefreshCategory::Playlists),
            BrowseCategory::Genres => match state.library.genre_content_type {
                GenreContentType::Genres => Some(RefreshCategory::Genres),
                GenreContentType::ArtistGenres => Some(RefreshCategory::ArtistGenres),
                GenreContentType::AlbumGenres => Some(RefreshCategory::AlbumGenres),
                GenreContentType::Moods => Some(RefreshCategory::Moods),
                GenreContentType::Styles => Some(RefreshCategory::Styles),
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
    if !state.cache_mgmt.background_refresh.contains(&tier1_category) {
        let is_stale = match state.cache_mgmt.category_timestamps.get(&tier1_category) {
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
        if state.cache_mgmt.background_refresh.contains(&cat) {
            continue;
        }
        let is_stale = match state.cache_mgmt.category_timestamps.get(&cat) {
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

    state.cache_mgmt.background_refresh.insert(category);

    let old_count = match category {
        RefreshCategory::Artists | RefreshCategory::AlbumArtists => state.library.artists.len(),
        RefreshCategory::Albums => state.library.albums.len(),
        RefreshCategory::Playlists => state.library.playlists.len(),
        RefreshCategory::Genres => state.library.genres.len(),
        RefreshCategory::ArtistGenres => state.library.artist_genres.len(),
        RefreshCategory::AlbumGenres => state.library.album_genres.len(),
        RefreshCategory::Moods => state.library.moods.len(),
        RefreshCategory::Styles => state.library.styles.len(),
        RefreshCategory::Stations => state.stations.len(),
        RefreshCategory::AllTracks => state.library.all_tracks.len(),
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
                        let _ = event_tx.send(PreloadEvent::ArtistsPreloaded { library_key: lib_key.clone(), artists }.into()).await;
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
                        let _ = event_tx.send(PreloadEvent::AlbumsPreloaded { library_key: lib_key.clone(), albums }.into()).await;
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
                        let _ = event_tx.send(PreloadEvent::PlaylistsPreloaded { library_key: lib_key.clone(), playlists }.into()).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh playlists: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::Genres => {
                match client.get_genres(&lib_key).await {
                    Ok(genres) => {
                        let new_count = genres.len();
                        let _ = event_tx.send(PreloadEvent::GenresPreloaded { library_key: lib_key.clone(), genres }.into()).await;
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
                        let _ = event_tx.send(PreloadEvent::ArtistGenresPreloaded { library_key: lib_key.clone(), genres }.into()).await;
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
                        let _ = event_tx.send(PreloadEvent::AlbumGenresPreloaded { library_key: lib_key.clone(), genres }.into()).await;
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
                        let _ = event_tx.send(PreloadEvent::MoodsPreloaded { library_key: lib_key.clone(), moods }.into()).await;
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
                        let _ = event_tx.send(PreloadEvent::StylesPreloaded { library_key: lib_key.clone(), styles }.into()).await;
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
                        let _ = event_tx.send(PreloadEvent::StationsPreloaded { library_key: lib_key.clone(), stations }.into()).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh stations: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::AllTracks => {
                match client.get_tracks(&lib_key).await {
                    Ok(tracks) => {
                        let new_count = tracks.len();
                        let _ = event_tx.send(PreloadEvent::AllTracksPreloaded { library_key: lib_key.clone(), tracks }.into()).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh all tracks: {}", e);
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
                        let _ = event_tx.send(FolderEvent::FoldersPreloaded { library_key: lib_key.clone(), folder_state }.into()).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh folders: {}", e);
                        false
                    }
                }
            }
        };

        let _ = event_tx.send(CacheEvent::CacheRefreshCompleted { category, changed }.into()).await;
    });
}
