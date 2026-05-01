//! List navigation, scrolling, pagination, and filter selection.

use crate::app::event::*;
use crate::app::{AppState, Event};
use crate::app::state::{
    BrowseCategory, Focus,
    RightPanelMode, View,
};
use crate::plex::PlexClient;
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
        state.library.artists_loading = true;

        let event_tx = event_tx.clone();
        let client = client.clone();
        let lib_key = lib_key.clone();
        tokio::spawn(async move {
            match client.get_artists(&lib_key).await {
                Ok(artists) => {
                    tracing::info!("Loaded {} artists", artists.len());
                    let _ = event_tx.send(DataEvent::ArtistsLoaded(artists).into()).await;
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

/// Load all audio playlists from the server.
///
/// Previously this filtered by `state.active_library` (sectionID). Plex
/// only returns a playlist on a `sectionID=X` query when *every* track
/// in the playlist belongs to that section — so user-created lists that
/// span libraries (or were authored against a different one) silently
/// vanished. Plexamp itself doesn't apply that filter; we mirror its
/// behaviour and show every audio playlist the server exposes. Smart
/// playlists with duplicate per-library titles are still de-duped
/// inside `client.get_playlists`.
pub fn load_playlists(event_tx: &mpsc::Sender<Event>, state: &mut AppState, client: &PlexClient) {
    tracing::info!("Loading playlists (server-wide, no section filter)");
    state.library.playlists_loading = true;

    let event_tx = event_tx.clone();
    let client = client.clone();
    tokio::spawn(async move {
        match client.get_playlists(None).await {
            Ok(playlists) => {
                tracing::info!("Loaded {} playlists", playlists.len());
                let _ = event_tx.send(DataEvent::PlaylistsLoaded(playlists).into()).await;
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
            let loaded = state.library.artists.len();
            let total = state.library.artists_total as usize;

            if idx + 20 >= loaded && loaded < total && !state.library.artists_loading {
                state.library.artists_loading = true;
                let offset = loaded as u32;
                // Remember selected artist before re-sort
                let selected_key = state.library.artists.get(idx)
                    .map(|a| a.rating_key.clone());
                if let Ok((more, _)) = client.get_artists_page(lib_key, offset, PAGE_SIZE).await {
                    sorted_merge(&mut state.library.artists, more, |a| sort_key(&a.title));
                    // Restore selection to the same artist after re-sort
                    if let Some(ref key) = selected_key {
                        if let Some(pos) = state.library.artists.iter().position(|a| &a.rating_key == key) {
                            state.list_state.artists_index = pos;
                        }
                    }
                }
                state.library.artists_loading = false;
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
                match state.library.right_panel_mode {
                    RightPanelMode::ArtistAlbums => {
                        let len = state.library.selected_artist_albums.len() + 1;
                        if len > 0 {
                            let idx = state.list_state.right_albums_index as isize + delta;
                            state.list_state.right_albums_index = idx.clamp(0, len as isize - 1) as usize;
                        }
                    }
                    RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                        let len = state.library.selected_album_tracks.len();
                        if len > 0 {
                            let idx = state.list_state.tracks_index as isize + delta;
                            state.list_state.tracks_index = idx.clamp(0, len as isize - 1) as usize;
                        }
                    }
                    RightPanelMode::CategoryAlbums => {
                        let len = state.library.tag_albums.len();
                        if len > 0 {
                            let idx = state.library.tag_albums_index as isize + delta;
                            state.library.tag_albums_index = idx.clamp(0, len as isize - 1) as usize;
                        }
                    }
                    RightPanelMode::Empty => {}
                }
            }
        }
        View::NowPlaying => {
            let len = state.queue.tracks.len();
            if len > 0 {
                let idx = state.list_state.queue_index as isize + delta;
                state.list_state.queue_index = idx.clamp(0, len as isize - 1) as usize;
            }
        }
        View::Similar => {
            let len = match state.similar.mode {
                crate::app::state::SimilarMode::Albums => state.similar.albums.len(),
                crate::app::state::SimilarMode::Tracks => state.similar.tracks.len(),
                crate::app::state::SimilarMode::Artists => state.similar.artists.len(),
            };
            if len > 0 {
                let idx = state.list_state.similar_index as isize + delta;
                state.list_state.similar_index = idx.clamp(0, len as isize - 1) as usize;
            }
        }
        View::Related => {
            let len = related_flat_count(&state.related.groups);
            if len > 0 {
                let idx = state.list_state.related_index as isize + delta;
                state.list_state.related_index = idx.clamp(0, len as isize - 1) as usize;
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
                match state.library.right_panel_mode {
                    RightPanelMode::ArtistAlbums => {
                        let len = state.library.selected_artist_albums.len() + 1;
                        state.list_state.right_albums_index = if index == isize::MAX {
                            len.saturating_sub(1)
                        } else {
                            (index as usize).min(len.saturating_sub(1))
                        };
                    }
                    RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                        let len = state.library.selected_album_tracks.len();
                        state.list_state.tracks_index = if index == isize::MAX {
                            len.saturating_sub(1)
                        } else {
                            (index as usize).min(len.saturating_sub(1))
                        };
                    }
                    RightPanelMode::CategoryAlbums => {
                        let len = state.library.tag_albums.len();
                        state.library.tag_albums_index = if index == isize::MAX {
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
            let len = state.queue.tracks.len();
            state.list_state.queue_index = if index == isize::MAX {
                len.saturating_sub(1)
            } else {
                (index as usize).min(len.saturating_sub(1))
            };
        }
        View::Similar => {
            let len = match state.similar.mode {
                crate::app::state::SimilarMode::Albums => state.similar.albums.len(),
                crate::app::state::SimilarMode::Tracks => state.similar.tracks.len(),
                crate::app::state::SimilarMode::Artists => state.similar.artists.len(),
            };
            state.list_state.similar_index = if index == isize::MAX {
                len.saturating_sub(1)
            } else {
                (index as usize).min(len.saturating_sub(1))
            };
        }
        View::Related => {
            let len = related_flat_count(&state.related.groups);
            state.list_state.related_index = if index == isize::MAX {
                len.saturating_sub(1)
            } else {
                (index as usize).min(len.saturating_sub(1))
            };
        }
        _ => {}
    }
}

/// Count total flat items in related groups (1 header + N albums per group).
pub fn related_flat_count(groups: &[crate::app::state::RelatedArtistGroup]) -> usize {
    groups.iter().map(|g| 1 + g.albums.len()).sum()
}

/// Resolve flat index into (group_idx, is_header, album_idx_within_group).
pub fn related_flat_resolve(groups: &[crate::app::state::RelatedArtistGroup], flat_idx: usize) -> Option<(usize, bool, usize)> {
    let mut offset = 0;
    for (gi, group) in groups.iter().enumerate() {
        let group_size = 1 + group.albums.len();
        if flat_idx < offset + group_size {
            let local = flat_idx - offset;
            if local == 0 {
                return Some((gi, true, 0));
            } else {
                return Some((gi, false, local - 1));
            }
        }
        offset += group_size;
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{SimilarMode, RelatedArtistGroup, RelatedSource};
    use crate::plex::models::{Artist, Album, Track};

    fn make_track(key: &str, title: &str) -> Track {
        Track {
            rating_key: key.to_string(),
            title: title.to_string(),
            ..Default::default()
        }
    }

    fn make_artist(key: &str, title: &str) -> Artist {
        Artist {
            rating_key: key.to_string(),
            title: title.to_string(),
            ..Default::default()
        }
    }

    fn make_album(key: &str, title: &str) -> Album {
        Album {
            rating_key: key.to_string(),
            title: title.to_string(),
            ..Default::default()
        }
    }

    // --- adjust_list_index tests ---

    #[test]
    fn adjust_queue_index_down() {
        let mut state = AppState::new();
        state.view = View::NowPlaying;
        state.queue.tracks = vec![make_track("1", "A"), make_track("2", "B"), make_track("3", "C")];
        state.list_state.queue_index = 0;

        adjust_list_index(&mut state, 1);
        assert_eq!(state.list_state.queue_index, 1);

        adjust_list_index(&mut state, 1);
        assert_eq!(state.list_state.queue_index, 2);
    }

    #[test]
    fn adjust_queue_index_up() {
        let mut state = AppState::new();
        state.view = View::NowPlaying;
        state.queue.tracks = vec![make_track("1", "A"), make_track("2", "B"), make_track("3", "C")];
        state.list_state.queue_index = 2;

        adjust_list_index(&mut state, -1);
        assert_eq!(state.list_state.queue_index, 1);
    }

    #[test]
    fn adjust_queue_clamps_at_boundaries() {
        let mut state = AppState::new();
        state.view = View::NowPlaying;
        state.queue.tracks = vec![make_track("1", "A"), make_track("2", "B")];
        state.list_state.queue_index = 0;

        // Clamp at top
        adjust_list_index(&mut state, -5);
        assert_eq!(state.list_state.queue_index, 0);

        // Clamp at bottom
        adjust_list_index(&mut state, 100);
        assert_eq!(state.list_state.queue_index, 1);
    }

    #[test]
    fn adjust_queue_empty_is_noop() {
        let mut state = AppState::new();
        state.view = View::NowPlaying;
        // queue is empty

        adjust_list_index(&mut state, 1);
        assert_eq!(state.list_state.queue_index, 0);
    }

    #[test]
    fn adjust_similar_albums() {
        let mut state = AppState::new();
        state.view = View::Similar;
        state.similar.mode = SimilarMode::Albums;
        state.similar.albums = vec![make_album("1", "A"), make_album("2", "B"), make_album("3", "C")];
        state.list_state.similar_index = 0;

        adjust_list_index(&mut state, 2);
        assert_eq!(state.list_state.similar_index, 2);

        // Clamp at end
        adjust_list_index(&mut state, 5);
        assert_eq!(state.list_state.similar_index, 2);
    }

    #[test]
    fn adjust_similar_tracks() {
        let mut state = AppState::new();
        state.view = View::Similar;
        state.similar.mode = SimilarMode::Tracks;
        state.similar.tracks = vec![make_track("1", "A"), make_track("2", "B")];
        state.list_state.similar_index = 1;

        adjust_list_index(&mut state, -1);
        assert_eq!(state.list_state.similar_index, 0);
    }

    #[test]
    fn adjust_browse_left_category() {
        let mut state = AppState::new();
        state.view = View::Browse;
        state.focus = Focus::Left;
        state.set_browse_category(BrowseCategory::Library);
        state.library.artists = vec![make_artist("1", "A"), make_artist("2", "B"), make_artist("3", "C")];
        state.list_state.artists_index = 0;

        adjust_list_index(&mut state, 1);
        assert_eq!(state.list_state.artists_index, 1);
    }

    // --- set_list_index tests ---

    #[test]
    fn set_queue_index_absolute() {
        let mut state = AppState::new();
        state.view = View::NowPlaying;
        state.queue.tracks = vec![make_track("1", "A"), make_track("2", "B"), make_track("3", "C")];
        state.list_state.queue_index = 0;

        set_list_index(&mut state, 2);
        assert_eq!(state.list_state.queue_index, 2);
    }

    #[test]
    fn set_queue_index_max_jumps_to_end() {
        let mut state = AppState::new();
        state.view = View::NowPlaying;
        state.queue.tracks = vec![make_track("1", "A"), make_track("2", "B"), make_track("3", "C")];
        state.list_state.queue_index = 0;

        set_list_index(&mut state, isize::MAX);
        assert_eq!(state.list_state.queue_index, 2);
    }

    #[test]
    fn set_queue_index_zero_jumps_to_start() {
        let mut state = AppState::new();
        state.view = View::NowPlaying;
        state.queue.tracks = vec![make_track("1", "A"), make_track("2", "B"), make_track("3", "C")];
        state.list_state.queue_index = 2;

        set_list_index(&mut state, 0);
        assert_eq!(state.list_state.queue_index, 0);
    }

    // --- related_flat_count / related_flat_resolve tests ---

    fn make_related_groups() -> Vec<RelatedArtistGroup> {
        vec![
            RelatedArtistGroup {
                artist: make_artist("a1", "Artist 1"),
                albums: vec![make_album("al1", "Album 1"), make_album("al2", "Album 2")],
                source: RelatedSource::Plex,
            },
            RelatedArtistGroup {
                artist: make_artist("a2", "Artist 2"),
                albums: vec![make_album("al3", "Album 3")],
                source: RelatedSource::Plex,
            },
        ]
    }

    #[test]
    fn related_flat_count_sums_correctly() {
        let groups = make_related_groups();
        // Group 1: 1 header + 2 albums = 3
        // Group 2: 1 header + 1 album = 2
        assert_eq!(related_flat_count(&groups), 5);
    }

    #[test]
    fn related_flat_resolve_header_and_albums() {
        let groups = make_related_groups();

        // Index 0 = header of group 0
        assert_eq!(related_flat_resolve(&groups, 0), Some((0, true, 0)));
        // Index 1 = first album of group 0
        assert_eq!(related_flat_resolve(&groups, 1), Some((0, false, 0)));
        // Index 2 = second album of group 0
        assert_eq!(related_flat_resolve(&groups, 2), Some((0, false, 1)));
        // Index 3 = header of group 1
        assert_eq!(related_flat_resolve(&groups, 3), Some((1, true, 0)));
        // Index 4 = first album of group 1
        assert_eq!(related_flat_resolve(&groups, 4), Some((1, false, 0)));
        // Index 5 = out of bounds
        assert_eq!(related_flat_resolve(&groups, 5), None);
    }

    #[test]
    fn related_flat_empty_groups() {
        let groups: Vec<RelatedArtistGroup> = vec![];
        assert_eq!(related_flat_count(&groups), 0);
        assert_eq!(related_flat_resolve(&groups, 0), None);
    }

    // --- calc_scroll_offset tests ---

    #[test]
    fn scroll_offset_near_top() {
        assert_eq!(calc_scroll_offset(2, 20, 100), 0);
    }

    #[test]
    fn scroll_offset_middle() {
        // selected=50, viewport=20, half=10 → offset = 50 - 10 = 40
        assert_eq!(calc_scroll_offset(50, 20, 100), 40);
    }

    #[test]
    fn scroll_offset_near_bottom() {
        // selected=95, viewport=20, half=10
        // 95 + 10 >= 100, so offset = 100 - 20 = 80
        assert_eq!(calc_scroll_offset(95, 20, 100), 80);
    }

    #[test]
    fn scroll_offset_empty_or_zero() {
        assert_eq!(calc_scroll_offset(0, 0, 0), 0);
        assert_eq!(calc_scroll_offset(0, 20, 0), 0);
        assert_eq!(calc_scroll_offset(5, 0, 100), 0);
    }

    // --- sorted_merge tests ---

    #[test]
    fn sorted_merge_basic() {
        let mut existing = vec!["apple", "cherry", "elephant"];
        let new_items = vec!["banana", "dog"];
        sorted_merge(&mut existing, new_items, |s| s.to_string());
        assert_eq!(existing, vec!["apple", "banana", "cherry", "dog", "elephant"]);
    }

    #[test]
    fn sorted_merge_empty_new() {
        let mut existing = vec!["a", "b"];
        sorted_merge(&mut existing, vec![], |s| s.to_string());
        assert_eq!(existing, vec!["a", "b"]);
    }

    #[test]
    fn sorted_merge_empty_existing() {
        let mut existing: Vec<&str> = vec![];
        sorted_merge(&mut existing, vec!["x", "y"], |s| s.to_string());
        assert_eq!(existing, vec!["x", "y"]);
    }
}
