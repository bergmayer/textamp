//! Shared utility functions used across multiple handler modules.
//!
//! Split into focused submodules:
//! - `cache` — periodic cache saving
//! - `connection` — server connection discovery
//! - `navigation` — list scrolling, pagination, filter selection
//! - `playback` — track playing, Plex reporting, radio
//! - `preload` — background data preloading
//! - `refresh` — view refresh, stale data detection

mod cache;
mod compilations;
mod connection;
pub(in crate::app::handlers) mod navigation;
mod playback;
mod preload;
mod refresh;

// Re-export all public items for backward compatibility.
// Call sites continue to use `helpers::function_name()`.
pub use cache::maybe_save_cache_async;
pub use connection::{find_working_connection, find_working_connection_from_servers};
pub use navigation::{
    adjust_list_index, calc_scroll_offset, load_artists,
    load_playlists, maybe_load_more, set_list_index,
};
pub use playback::{
    fetch_more_radio_tracks, generate_plex_session_id,
    get_upcoming_tracks, insert_tracks_next, play_current_track, play_track, queue_and_play,
    report_playback_progress_to_plex, report_playback_stop_to_plex,
};
pub use preload::{maybe_start_subfolder_preload, preload_all_library_data, preload_data, PreloadType, SubfolderPreloadResult};
pub use compilations::maybe_detect as maybe_detect_compilations;
pub use refresh::{
    is_viewing_category, check_staleness_on_view_load, current_view_category,
    refresh_current_view, spawn_category_refresh,
};

/// Page size for paginated API requests.
pub const PAGE_SIZE: u32 = 100;

/// Append DJ modes, actions, and remix items to a station list.
/// Used when building station_nav from any source (API, cache, preload).
///
/// Layout:
/// ```text
/// [Plex radio stations...]
/// ─────────────── (sep:dj)
/// DJ Freeze, Contempo, Groupie, Gemini, Twofer, Stretch (all continuous)
/// DJ Friendgänger (grayed)
/// ─────────────── (sep:actions)
/// Sonic Adventure, Artist Radio
/// ─────────────── (sep:remix)
/// Remix: Gemini, Twofer, Stretch, Shuffle
/// ```
pub fn append_station_action_items(stations: &mut Vec<crate::plex::models::Station>, shuffle_active: bool) {
    use crate::plex::models::Station;
    use crate::app::state::DjMode;

    // Strip any previously appended synthetic items so we always rebuild fresh.
    stations.retain(|s| {
        s.station_type != "action"
            && s.station_type != "separator"
            && s.station_type != "dj_mode"
            && s.station_type != "remix"
    });

    // ── DJ Modes ──
    stations.push(Station {
        key: "sep:dj".to_string(),
        title: "\u{2500}".to_string(), // ─
        station_type: "separator".to_string(),
        identifier: None, thumb: None, art: None, description: None,
    });

    // All 6 DJ modes are now continuous (insert on every track transition)
    for mode in &[DjMode::Freeze, DjMode::Contempo, DjMode::Groupie, DjMode::Gemini, DjMode::Twofer, DjMode::Stretch] {
        stations.push(Station {
            key: mode.key().to_string(),
            title: mode.name().to_string(),
            station_type: "dj_mode".to_string(),
            identifier: None, thumb: None, art: None,
            description: Some(mode.description().to_string()),
        });
    }

    // DJ Friendgänger (deferred/unavailable)
    stations.push(Station {
        key: "dj:friendganger".to_string(),
        title: "DJ Friendg\u{00e4}nger".to_string(),
        station_type: "dj_mode".to_string(),
        identifier: None, thumb: None, art: None,
        description: Some("Requires Sonic Analysis on shared libraries".to_string()),
    });

    // ── Actions ──
    stations.push(Station {
        key: "sep:actions".to_string(),
        title: "\u{2500}".to_string(), // ─
        station_type: "separator".to_string(),
        identifier: None, thumb: None, art: None, description: None,
    });

    stations.push(Station {
        key: "action:adventure".to_string(),
        title: "Sonic Adventure".to_string(),
        station_type: "action".to_string(),
        identifier: None, thumb: None, art: None,
        description: Some("Create a sonic bridge between two tracks".to_string()),
    });
    stations.push(Station {
        key: "action:artist_radio".to_string(),
        title: "Artist Radio".to_string(),
        station_type: "action".to_string(),
        identifier: None, thumb: None, art: None,
        description: Some("Blend radio from multiple artists".to_string()),
    });

    // ── Queue Remix ──
    stations.push(Station {
        key: "sep:remix".to_string(),
        title: "\u{2500}".to_string(), // ─
        station_type: "separator".to_string(),
        identifier: None, thumb: None, art: None, description: None,
    });

    stations.push(Station {
        key: "remix:gemini".to_string(),
        title: "Remix: Gemini".to_string(),
        station_type: "remix".to_string(),
        identifier: None, thumb: None, art: None,
        description: Some("Insert similar tracks between queue items".to_string()),
    });
    stations.push(Station {
        key: "remix:twofer".to_string(),
        title: "Remix: Twofer".to_string(),
        station_type: "remix".to_string(),
        identifier: None, thumb: None, art: None,
        description: Some("Insert same-artist tracks between queue items".to_string()),
    });
    stations.push(Station {
        key: "remix:stretch".to_string(),
        title: "Remix: Stretch".to_string(),
        station_type: "remix".to_string(),
        identifier: None, thumb: None, art: None,
        description: Some("Insert sonic bridge tracks between queue items".to_string()),
    });
    stations.push(Station {
        key: "remix:doppelganger".to_string(),
        title: "Remix: Doppelganger".to_string(),
        station_type: "remix".to_string(),
        identifier: None, thumb: None, art: None,
        description: Some("Replace each track with similar track by different artist".to_string()),
    });
    stations.push(Station {
        key: "remix:shuffle".to_string(),
        title: if shuffle_active { "Undo Shuffle" } else { "Remix: Shuffle" }.to_string(),
        station_type: "remix".to_string(),
        identifier: None, thumb: None, art: None,
        description: Some(if shuffle_active { "Restore original queue order" } else { "Shuffle the current queue" }.to_string()),
    });
}

/// Build a drill-down column from a grouped-by-album column's selected album group.
///
/// Used by keyboard (Enter/Right, Up/Down auto-drill) and mouse click handlers
/// to avoid duplicating the grouped-album expansion logic.
pub fn drill_grouped_album(col: &crate::app::state::BrowseColumn, album_idx: usize) -> Option<crate::app::state::BrowseColumn> {
    use crate::app::state::{BrowseColumn, BrowseItem};
    let groups = col.album_groups.as_ref()?;
    let indices = groups.get(album_idx)?;
    let tracks: Vec<_> = indices.iter()
        .filter_map(|&i| col.tracks.get(i).cloned())
        .collect();
    let items = BrowseItem::from_tracks(&tracks);
    let album_item = col.items.get(album_idx);
    let title = album_item
        .map(|item| format!("tracks \u{2014} {}", item.title()))
        .unwrap_or_else(|| "tracks".to_string());
    let mut new_col = BrowseColumn::new_with_tracks(title, items, tracks);
    if let Some(item) = album_item {
        new_col.play_all_row = Some(crate::app::state::PlayAllRow::Album {
            rating_key: item.key().to_string(),
            title: item.title().to_string(),
        });
        new_col.on_play_row = true;
    }
    Some(new_col)
}

/// Generate a sort key for a title, ignoring "The " prefix.
pub fn sort_key(title: &str) -> String {
    let lower = title.to_lowercase();
    if let Some(stripped) = lower.strip_prefix("the ") {
        stripped.to_string()
    } else {
        lower
    }
}

/// Letters shown on the browse alphabet strip, in the natural sort
/// order: `%` (ASCII non-alphanumeric), `0` (digits), `a..z`, then
/// `'文'` (the bucket for everything starting with a non-ASCII glyph
/// — CJK, Cyrillic, Greek, etc., which sort after `z`).
/// Index space is shared between rendering, click hit-testing, and
/// keyboard nav of the strip itself.
pub const ALPHABET_STRIP_LETTERS: [char; 29] = [
    '%', '0',
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm',
    'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
    '文',
];

/// Find the row index in the artist root column whose `sort_key`
/// starts with the given alphabet-strip letter.
/// - `'%'` matches an ASCII non-alphanumeric first character (punctuation, &, etc.)
/// - `'0'` matches an ASCII digit
/// - `'a'..='z'` matches that letter
/// - `'文'` matches any non-ASCII first character (CJK, Cyrillic, etc.)
pub fn alphabet_target_index(state: &crate::app::state::AppState, ch: char) -> Option<usize> {
    use crate::app::state::BrowseCategory;
    if state.browse_category == BrowseCategory::Folders {
        return None;
    }
    let pred: Box<dyn Fn(&str) -> bool> = match ch {
        '0' => Box::new(|t: &str| sort_key(t).chars().next().map_or(false, |c| c.is_ascii_digit())),
        '%' => Box::new(|t: &str| sort_key(t).chars().next().map_or(false,
            |c| c.is_ascii() && !c.is_ascii_alphanumeric())),
        '文' => Box::new(|t: &str| sort_key(t).chars().next().map_or(false, |c| !c.is_ascii())),
        c if c.is_ascii_alphabetic() => {
            let lc = c.to_ascii_lowercase();
            Box::new(move |t: &str| {
                sort_key(t).chars().next().map_or(false, |first| first.to_ascii_lowercase() == lc)
            })
        }
        _ => return None,
    };
    let nav = state.browse_nav()?;
    let root = nav.columns.first()?;
    root.items.iter().position(|it| pred(it.title()))
}

/// Apply an alphabet-strip jump. Pure scroll action: pins the
/// artist root column's scroll offset to the matched row but does
/// NOT change the selection or close any drilled-in child columns
/// — same semantics as the GUI's alphabet jump. Returns the matched
/// row index, if any.
pub fn alphabet_jump(state: &mut crate::app::state::AppState, ch: char) -> Option<usize> {
    let target = alphabet_target_index(state, ch)?;
    state.scroll.browse = Some((0, target));
    Some(target)
}

/// Get artist key and name for the bio popup (F4).
/// Priority: highlighted Sonically-Similar row in the track pane
/// → selected track → selected album → selected artist → now-playing track.
/// For compilation tracks, uses the track artist (original_title) instead of album artist.
pub fn get_artist_for_bio(state: &crate::app::state::AppState) -> Option<(String, String)> {
    use crate::app::state::{View, BrowseItem, PlaybackMode};

    // Helper: extract artist info from a track, preferring track artist for compilations
    let artist_from_track = |track: &crate::plex::models::Track| -> Option<(String, String)> {
        // Check if this is a compilation track (has original_title different from album artist)
        if let Some(ref track_artist) = track.original_title {
            let album_artist = track.grandparent_title.as_deref().unwrap_or("");
            // If track artist differs from album artist, try to find the track artist
            if !track_artist.is_empty() && track_artist != album_artist {
                // Search for artist by name in cached artists
                if let Some(found) = state.library.artists.iter().find(|a| a.title == *track_artist) {
                    return Some((found.rating_key.clone(), found.title.clone()));
                }
                // Fall back to album artist if track artist not found in library
            }
        }
        // Use album artist
        if let (Some(key), Some(name)) = (&track.grandparent_rating_key, &track.grandparent_title) {
            return Some((key.clone(), name.clone()));
        }
        None
    };

    // 0. Highlighted Sonically-Similar row inside the track pane
    //    wins ahead of everything else — when the user has navigated
    //    into the pane and picked a similar song, "Artist Bio"
    //    should target THAT artist, not the parent track's.
    if state.track_pane_focused && state.track_pane_index > 0 {
        if let Some(parent) = state.focused_track() {
            let sim_idx = state.track_pane_index - 1;
            if let Some(sim) = state
                .track_pane_similar
                .get(&parent.rating_key)
                .and_then(|v| v.get(sim_idx))
            {
                if let Some(result) = artist_from_track(sim) {
                    return Some(result);
                }
            }
        }
    }

    // 1. Check selected item (in Browse, Queue, Search, etc.)
    match state.view {
        View::Browse => {
            // Check Miller columns for selected item
            if let Some(nav) = state.browse_nav() {
                let col_idx = nav.focused_column;
                if let Some(col) = nav.columns.get(col_idx) {
                    let item_idx = col.selected_index;

                    // Check if we have a track column (with tracks vec)
                    if !col.tracks.is_empty() {
                        if let Some(track) = col.tracks.get(item_idx) {
                            if let Some(result) = artist_from_track(track) {
                                return Some(result);
                            }
                        }
                    }

                    // Check browse item type
                    if let Some(item) = col.items.get(item_idx) {
                        match item {
                            BrowseItem::Album { key, artist, .. } => {
                                // Look up album in state.library.albums to get artist key
                                if let Some(album) = state.library.albums.iter().find(|a| a.rating_key == *key) {
                                    if let (Some(artist_key), Some(artist_name)) = (&album.parent_rating_key, &album.parent_title) {
                                        return Some((artist_key.clone(), artist_name.clone()));
                                    }
                                }
                                // Fall back to artist name from BrowseItem
                                if !artist.is_empty() {
                                    // Try to find artist by name in state.library.artists
                                    if let Some(found) = state.library.artists.iter().find(|a| a.title == *artist) {
                                        return Some((found.rating_key.clone(), found.title.clone()));
                                    }
                                }
                            }
                            BrowseItem::Artist { key, title, .. } => {
                                return Some((key.clone(), title.clone()));
                            }
                            BrowseItem::AllTracks { scope, .. } => {
                                if let (Some(artist_key), Some(artist_name)) =
                                    (scope.artist_key(), scope.artist_name())
                                {
                                    return Some((artist_key.to_string(), artist_name.to_string()));
                                }
                            }
                            BrowseItem::ArtistRadio { artist_key, artist_name, .. } => {
                                return Some((artist_key.clone(), artist_name.clone()));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        View::Queue | View::NowPlaying => {
            // Check selected queue/radio track
            let tracks = match state.playback_mode {
                PlaybackMode::Radio => &state.radio.tracks,
                _ => &state.queue.tracks,
            };
            if let Some(track) = tracks.get(state.list_state.queue_index) {
                if let Some(result) = artist_from_track(track) {
                    return Some(result);
                }
            }
        }
        View::Search => {
            // Check selected search result based on active tab
            if let Some(ref results) = state.search.results {
                use crate::app::state::SearchTab;
                let idx = state.list_state.search_item_index;

                match state.search.tab {
                    SearchTab::Tracks => {
                        if let Some(track) = results.tracks.get(idx) {
                            if let Some(result) = artist_from_track(track) {
                                return Some(result);
                            }
                        }
                    }
                    SearchTab::Albums => {
                        if let Some(album) = results.albums.get(idx) {
                            if let (Some(key), Some(name)) = (&album.parent_rating_key, &album.parent_title) {
                                return Some((key.clone(), name.clone()));
                            }
                        }
                    }
                    SearchTab::Artists => {
                        if let Some(artist) = results.artists.get(idx) {
                            return Some((artist.rating_key.clone(), artist.title.clone()));
                        }
                    }
                    SearchTab::Global => {
                        // All tab: figure out which section the index is in
                        let (section, local_idx) = crate::app::handlers::dispatch_search::resolve_global_index(results, idx);
                        match section {
                            SearchTab::Artists => {
                                if let Some(artist) = results.artists.get(local_idx) {
                                    return Some((artist.rating_key.clone(), artist.title.clone()));
                                }
                            }
                            SearchTab::Albums => {
                                if let Some(album) = results.albums.get(local_idx) {
                                    if let (Some(key), Some(name)) = (&album.parent_rating_key, &album.parent_title) {
                                        return Some((key.clone(), name.clone()));
                                    }
                                }
                            }
                            SearchTab::Tracks => {
                                if let Some(track) = results.tracks.get(local_idx) {
                                    if let Some(result) = artist_from_track(track) {
                                        return Some(result);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
        View::Similar => {
            // Check similar albums/tracks
            match state.similar.mode {
                crate::app::state::SimilarMode::Albums => {
                    if let Some(album) = state.similar.albums.get(state.list_state.similar_index) {
                        if let (Some(key), Some(name)) = (&album.parent_rating_key, &album.parent_title) {
                            return Some((key.clone(), name.clone()));
                        }
                    }
                }
                crate::app::state::SimilarMode::Tracks => {
                    if let Some(track) = state.similar.tracks.get(state.list_state.similar_index) {
                        if let Some(result) = artist_from_track(track) {
                            return Some(result);
                        }
                    }
                }
                crate::app::state::SimilarMode::Artists => {
                    if let Some(artist) = state.similar.artists.get(state.list_state.similar_index) {
                        return Some((artist.rating_key.clone(), artist.title.clone()));
                    }
                }
            }
        }
        _ => {}
    }

    // 2. Fall back to now-playing track
    if let Some(track) = state.current_track() {
        if let Some(result) = artist_from_track(track) {
            return Some(result);
        }
    }

    None
}

/// Spawn a simple async API call that sends an event on success or `DataLoadError` on failure.
///
/// Reduces boilerplate for the common pattern of:
/// clone client + event_tx → spawn → match client.method().await → send event.
pub fn spawn_api_call<T, F, Fut>(
    event_tx: &tokio::sync::mpsc::Sender<crate::app::Event>,
    client: &crate::plex::PlexClient,
    call: F,
    on_success: impl Fn(T) -> crate::app::Event + Send + 'static,
    error_msg: &str,
) where
    F: FnOnce(crate::plex::PlexClient) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, crate::plex::ApiError>> + Send,
    T: Send + 'static,
{
    let tx = event_tx.clone();
    let c = client.clone();
    let msg = error_msg.to_string();
    tokio::spawn(async move {
        match call(c).await {
            Ok(data) => { let _ = tx.send(on_success(data)).await; }
            Err(e) => { let _ = tx.send(crate::app::event::DataEvent::DataLoadError(format!("{}: {}", msg, e)).into()).await; }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sort_key_basic() {
        assert_eq!(sort_key("Alice"), "alice");
        assert_eq!(sort_key("The Beatles"), "beatles");
        assert_eq!(sort_key("Zeppelin"), "zeppelin");
    }

    #[test]
    fn test_sort_key_the_prefix_only() {
        assert_eq!(sort_key("Theater"), "theater");
        assert_eq!(sort_key("The "), "");
        assert_eq!(sort_key("The Band"), "band");
    }

    #[test]
    fn test_sort_key_no_last_name_parsing() {
        assert_eq!(sort_key("John Smith"), "john smith");
    }

    #[test]
    fn test_calc_scroll_offset() {
        assert_eq!(calc_scroll_offset(0, 10, 100), 0);
        assert_eq!(calc_scroll_offset(50, 10, 100), 45);
        assert_eq!(calc_scroll_offset(95, 10, 100), 90);
        assert_eq!(calc_scroll_offset(0, 0, 100), 0);
        assert_eq!(calc_scroll_offset(0, 10, 0), 0);
    }
}
