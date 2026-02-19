//! List navigation, scrolling, pagination, and filter selection.

use crate::app::{AppState, Event};
use crate::app::state::{
    BrowseCategory, Focus,
    RightPanelMode, View,
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
        if state.browse_category == BrowseCategory::Library {
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
            // Search results navigation handled in search key handler
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
