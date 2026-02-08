//! List navigation, scrolling, pagination, and filter selection.

use crate::app::{Action, AppState, Event};
use crate::app::state::{
    BrowseCategory, Focus, PlaybackMode,
    RightPanelMode, SearchTab, View,
};
use crate::api::PlexClient;
use super::{sort_key, PAGE_SIZE};
use tokio::sync::mpsc;

/// Calculate the scroll offset to keep the selected item centered.
pub fn calc_scroll_offset(selected: usize, viewport_height: usize, total_items: usize) -> usize {
    if total_items == 0 || viewport_height == 0 {
        return 0;
    }
    let half_height = viewport_height / 2;
    if selected < half_height {
        0
    } else if selected + half_height >= total_items {
        total_items.saturating_sub(viewport_height)
    } else {
        selected.saturating_sub(half_height)
    }
}

/// Load artists in background.
pub fn load_artists(event_tx: &mpsc::Sender<Event>, state: &mut AppState, client: &PlexClient) {
    if let Some(lib_key) = &state.active_library {
        tracing::info!("Loading all artists from library: {}", lib_key);
        state.artists_loading = true;

        let event_tx = event_tx.clone();
        let client = client.clone();
        let lib_key = lib_key.clone();
        tokio::spawn(async move {
            match client.get_artists(&lib_key).await {
                Ok(artists) => {
                    tracing::info!("Loaded {} artists", artists.len());
                    let _ = event_tx.send(Event::ArtistsLoaded(artists)).await;
                }
                Err(e) => {
                    tracing::error!("Failed to load artists: {}", e);
                }
            }
        });
    } else {
        tracing::warn!("load_artists called but no active_library set");
    }
}

/// Load albums in background.
pub fn load_albums(event_tx: &mpsc::Sender<Event>, state: &mut AppState, client: &PlexClient) {
    if let Some(lib_key) = &state.active_library {
        tracing::info!("Loading all albums from library: {}", lib_key);
        state.albums_loading = true;

        let event_tx = event_tx.clone();
        let client = client.clone();
        let lib_key = lib_key.clone();
        tokio::spawn(async move {
            match client.get_albums(&lib_key).await {
                Ok(albums) => {
                    tracing::info!("Loaded {} albums", albums.len());
                    let _ = event_tx.send(Event::AlbumsLoaded(albums)).await;
                }
                Err(e) => {
                    tracing::error!("Failed to load albums: {}", e);
                }
            }
        });
    }
}

/// Load playlists in background, filtered by active library.
pub fn load_playlists(event_tx: &mpsc::Sender<Event>, state: &mut AppState, client: &PlexClient) {
    tracing::info!("Loading playlists");
    state.playlists_loading = true;

    let event_tx = event_tx.clone();
    let client = client.clone();
    let section_id = state.active_library.clone();
    tokio::spawn(async move {
        match client.get_playlists(section_id.as_deref()).await {
            Ok(playlists) => {
                tracing::info!("Loaded {} playlists", playlists.len());
                let _ = event_tx.send(Event::PlaylistsLoaded(playlists)).await;
            }
            Err(e) => {
                tracing::error!("Failed to load playlists: {}", e);
            }
        }
    });
}

/// Load more data when nearing the end of a paginated list.
pub async fn maybe_load_more(state: &mut AppState, client: &PlexClient) {
    if state.view != View::Browse || state.focus != Focus::Left {
        return;
    }

    if let Some(lib_key) = &state.active_library.clone() {
        if state.browse_category == BrowseCategory::Artists {
            let idx = state.list_state.artists_index;
            let loaded = state.artists.len();
            let total = state.artists_total as usize;

            if idx + 20 >= loaded && loaded < total && !state.artists_loading {
                state.artists_loading = true;
                let offset = loaded as u32;
                // Remember selected artist before re-sort
                let selected_key = state.artists.get(idx)
                    .map(|a| a.rating_key.clone());
                if let Ok((more, _)) = client.get_artists_page(lib_key, offset, PAGE_SIZE).await {
                    sorted_merge(&mut state.artists, more, |a| sort_key(&a.title));
                    // Restore selection to the same artist after re-sort
                    if let Some(ref key) = selected_key {
                        if let Some(pos) = state.artists.iter().position(|a| &a.rating_key == key) {
                            state.list_state.artists_index = pos;
                        }
                    }
                }
                state.artists_loading = false;
            }
        }
    }
}

/// Select a filter result from the search/filter view.
pub fn select_filter_result(state: &mut AppState) -> Vec<Action> {
    let idx = state.list_state.search_item_index;
    let search_tab = state.search_tab;

    match search_tab {
        SearchTab::Global => {
            return vec![];
        }
        SearchTab::Artists => {
            if let Some(ref results) = state.filter_results {
                if let Some(artist) = results.artists.get(idx) {
                    state.selected_artist_name = artist.title.clone();
                    state.pending_filter_key = Some(artist.rating_key.clone());
                    state.search_query.clear();
                    state.filter_results = None;
                    state.view = View::Browse;
                    state.browse_category = BrowseCategory::Artists;
                    return vec![Action::LoadArtistAlbums];
                }
            }
            if let Some(artist) = state.artists.iter().enumerate()
                .filter(|(_, a)| state.search_query.is_empty() || a.title.to_lowercase().contains(&state.search_query.to_lowercase()))
                .nth(idx)
                .map(|(i, _)| i)
            {
                state.set_category_index(artist);
                state.search_query.clear();
                state.filter_results = None;
                state.view = View::Browse;
                state.browse_category = BrowseCategory::Artists;
            }
        }
        SearchTab::AlbumArtists => {
            let query = state.search_query.to_lowercase();
            let mut album_artists: Vec<(String, String)> = state.albums.iter()
                .filter_map(|a| {
                    let artist = a.parent_title.as_deref().unwrap_or("");
                    if !artist.is_empty() && (query.is_empty() || artist.to_lowercase().contains(&query)) {
                        Some((artist.to_string(), a.rating_key.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            album_artists.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
            album_artists.dedup_by(|a, b| a.0.to_lowercase() == b.0.to_lowercase());

            if let Some((_, _album_key)) = album_artists.get(idx) {
                state.search_query.clear();
                state.filter_results = None;
                state.view = View::Browse;
            }
        }
        SearchTab::Albums => {
            if let Some(ref results) = state.filter_results {
                if let Some(album) = results.albums.get(idx).cloned() {
                    state.search_query.clear();
                    state.filter_results = None;
                    state.view = View::Browse;
                    return vec![Action::PlayAlbum { rating_key: album.rating_key }];
                }
            }
        }
        SearchTab::Playlists => {
            let query = state.search_query.to_lowercase();
            if let Some((i, _playlist)) = state.playlists.iter().enumerate()
                .filter(|(_, p)| query.is_empty() || p.title.to_lowercase().contains(&query))
                .nth(idx)
            {
                state.set_category_index(i);
                state.search_query.clear();
                state.filter_results = None;
                state.view = View::Browse;
                state.browse_category = BrowseCategory::Playlists;
                return vec![Action::LoadCategoryTracks];
            }
        }
        SearchTab::Tracks => {
            if let Some(ref results) = state.filter_results {
                if let Some(track) = results.tracks.get(idx).cloned() {
                    state.search_query.clear();
                    state.filter_results = None;
                    state.view = View::Browse;
                    state.queue.clear();
                    state.queue.push(track.clone());
                    state.queue_index = Some(0);
                    state.playback_mode = PlaybackMode::Queue;
                    return vec![Action::PlayTrack(track)];
                }
            }
        }
        SearchTab::Genres => {
            let query = state.search_query.to_lowercase();
            if let Some(i) = state.genres.iter().enumerate()
                .filter(|(_, g)| query.is_empty() || g.title.to_lowercase().contains(&query))
                .nth(idx)
                .map(|(i, _)| i)
            {
                state.set_category_index(i);
                state.search_query.clear();
                state.filter_results = None;
                state.view = View::Browse;
                state.browse_category = BrowseCategory::Genres;
            }
        }
    }

    vec![]
}

/// Adjust a list index by a delta (relative movement).
pub fn adjust_list_index(state: &mut AppState, delta: isize) {
    match state.view {
        View::Browse => {
            if state.focus == Focus::Left {
                let len = state.category_len();
                if len > 0 {
                    let idx = state.category_index() as isize + delta;
                    state.set_category_index(idx.clamp(0, len as isize - 1) as usize);
                }
            } else {
                match state.right_panel_mode {
                    RightPanelMode::ArtistAlbums => {
                        let len = state.selected_artist_albums.len() + 1;
                        if len > 0 {
                            let idx = state.list_state.right_albums_index as isize + delta;
                            state.list_state.right_albums_index = idx.clamp(0, len as isize - 1) as usize;
                        }
                    }
                    RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                        let len = state.selected_album_tracks.len();
                        if len > 0 {
                            let idx = state.list_state.tracks_index as isize + delta;
                            state.list_state.tracks_index = idx.clamp(0, len as isize - 1) as usize;
                        }
                    }
                    RightPanelMode::CategoryAlbums => {
                        let len = state.genre_albums.len();
                        if len > 0 {
                            let idx = state.genre_albums_index as isize + delta;
                            state.genre_albums_index = idx.clamp(0, len as isize - 1) as usize;
                        }
                    }
                    RightPanelMode::Empty => {}
                }
            }
        }
        View::NowPlaying => {
            let len = state.queue.len();
            if len > 0 {
                let idx = state.list_state.queue_index as isize + delta;
                state.list_state.queue_index = idx.clamp(0, len as isize - 1) as usize;
            }
        }
        View::Similar => {
            let len = match state.similar_mode {
                crate::app::state::SimilarMode::Albums => state.similar_albums.len(),
                crate::app::state::SimilarMode::Tracks => state.similar_tracks.len(),
            };
            if len > 0 {
                let idx = state.list_state.similar_index as isize + delta;
                state.list_state.similar_index = idx.clamp(0, len as isize - 1) as usize;
            }
        }
        View::Search => {
            let filtered_len = if let Some(ref results) = state.filter_results {
                match state.search_tab {
                    SearchTab::Global => 0,
                    SearchTab::Artists => results.artists.len(),
                    SearchTab::AlbumArtists => {
                        let query = state.search_query.to_lowercase();
                        let mut artists: Vec<String> = state.albums.iter()
                            .filter_map(|a| a.parent_title.as_ref())
                            .filter(|t| query.is_empty() || t.to_lowercase().contains(&query))
                            .map(|s| s.to_lowercase())
                            .collect();
                        artists.sort();
                        artists.dedup();
                        artists.len()
                    }
                    SearchTab::Albums => results.albums.len(),
                    SearchTab::Playlists => {
                        let query = state.search_query.to_lowercase();
                        state.playlists.iter()
                            .filter(|p| query.is_empty() || p.title.to_lowercase().contains(&query))
                            .count()
                    }
                    SearchTab::Tracks => results.tracks.len(),
                    SearchTab::Genres => {
                        let query = state.search_query.to_lowercase();
                        state.genres.iter()
                            .filter(|g| query.is_empty() || g.title.to_lowercase().contains(&query))
                            .count()
                    }
                }
            } else {
                let query = state.search_query.to_lowercase();
                match state.search_tab {
                    SearchTab::Global => 0,
                    SearchTab::Artists => state.artists.iter()
                        .filter(|a| query.is_empty() || a.title.to_lowercase().contains(&query))
                        .count(),
                    SearchTab::AlbumArtists => {
                        let mut artists: Vec<String> = state.albums.iter()
                            .filter_map(|a| a.parent_title.as_ref())
                            .filter(|t| query.is_empty() || t.to_lowercase().contains(&query))
                            .map(|s| s.to_lowercase())
                            .collect();
                        artists.sort();
                        artists.dedup();
                        artists.len()
                    }
                    SearchTab::Albums => state.albums.iter()
                        .filter(|a| query.is_empty() || a.title.to_lowercase().contains(&query))
                        .count(),
                    SearchTab::Playlists => state.playlists.iter()
                        .filter(|p| query.is_empty() || p.title.to_lowercase().contains(&query))
                        .count(),
                    SearchTab::Tracks => state.selected_album_tracks.iter()
                        .filter(|t| query.is_empty() || t.title.to_lowercase().contains(&query))
                        .count(),
                    SearchTab::Genres => state.genres.iter()
                        .filter(|g| query.is_empty() || g.title.to_lowercase().contains(&query))
                        .count(),
                }
            };

            if filtered_len > 0 {
                let idx = state.list_state.search_item_index as isize + delta;
                state.list_state.search_item_index = idx.clamp(0, filtered_len as isize - 1) as usize;
            }
        }
        _ => {}
    }
}

/// Set a list index to an absolute position.
pub fn set_list_index(state: &mut AppState, index: isize) {
    match state.view {
        View::Browse => {
            if state.focus == Focus::Left {
                let len = state.category_len();
                let idx = if index == isize::MAX {
                    len.saturating_sub(1)
                } else {
                    (index as usize).min(len.saturating_sub(1))
                };
                state.set_category_index(idx);
            } else {
                match state.right_panel_mode {
                    RightPanelMode::ArtistAlbums => {
                        let len = state.selected_artist_albums.len() + 1;
                        state.list_state.right_albums_index = if index == isize::MAX {
                            len.saturating_sub(1)
                        } else {
                            (index as usize).min(len.saturating_sub(1))
                        };
                    }
                    RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                        let len = state.selected_album_tracks.len();
                        state.list_state.tracks_index = if index == isize::MAX {
                            len.saturating_sub(1)
                        } else {
                            (index as usize).min(len.saturating_sub(1))
                        };
                    }
                    RightPanelMode::CategoryAlbums => {
                        let len = state.genre_albums.len();
                        state.genre_albums_index = if index == isize::MAX {
                            len.saturating_sub(1)
                        } else {
                            (index as usize).min(len.saturating_sub(1))
                        };
                    }
                    RightPanelMode::Empty => {}
                }
            }
        }
        View::NowPlaying => {
            let len = state.queue.len();
            state.list_state.queue_index = if index == isize::MAX {
                len.saturating_sub(1)
            } else {
                (index as usize).min(len.saturating_sub(1))
            };
        }
        View::Similar => {
            let len = match state.similar_mode {
                crate::app::state::SimilarMode::Albums => state.similar_albums.len(),
                crate::app::state::SimilarMode::Tracks => state.similar_tracks.len(),
            };
            state.list_state.similar_index = if index == isize::MAX {
                len.saturating_sub(1)
            } else {
                (index as usize).min(len.saturating_sub(1))
            };
        }
        _ => {}
    }
}

/// Merge `new_items` into an already-sorted vec, maintaining sort order.
///
/// More efficient than extend + re-sort for appending small pages to large lists:
/// O(m log m + n + m) vs O((n+m) log(n+m)), where n = existing, m = new items.
/// Each sort key is computed exactly once.
fn sorted_merge<T>(existing: &mut Vec<T>, mut new_items: Vec<T>, key_fn: impl Fn(&T) -> String) {
    if new_items.is_empty() {
        return;
    }

    // Sort the new page
    new_items.sort_by(|a, b| key_fn(a).cmp(&key_fn(b)));

    // Pre-compute all sort keys (each computed once)
    let old = std::mem::take(existing);
    let old_keys: Vec<String> = old.iter().map(|item| key_fn(item)).collect();
    let new_keys: Vec<String> = new_items.iter().map(|item| key_fn(item)).collect();

    // Merge both sorted sequences
    *existing = Vec::with_capacity(old.len() + new_items.len());
    let mut old_iter = old.into_iter().enumerate();
    let mut new_iter = new_items.into_iter().enumerate();
    let mut old_next = old_iter.next();
    let mut new_next = new_iter.next();

    loop {
        match (&old_next, &new_next) {
            (Some((oi, _)), Some((ni, _))) => {
                if old_keys[*oi] <= new_keys[*ni] {
                    existing.push(old_next.take().unwrap().1);
                    old_next = old_iter.next();
                } else {
                    existing.push(new_next.take().unwrap().1);
                    new_next = new_iter.next();
                }
            }
            (Some(_), None) => {
                existing.push(old_next.take().unwrap().1);
                existing.extend(old_iter.map(|(_, item)| item));
                break;
            }
            (None, Some(_)) => {
                existing.push(new_next.take().unwrap().1);
                existing.extend(new_iter.map(|(_, item)| item));
                break;
            }
            (None, None) => break,
        }
    }
}
