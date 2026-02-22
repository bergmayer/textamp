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
    get_upcoming_tracks, play_current_track, play_track, queue_and_play,
    report_playback_progress_to_plex, report_playback_stop_to_plex,
};
pub use preload::{maybe_start_subfolder_preload, preload_all_library_data, preload_data, SubfolderPreloadResult};
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
pub fn append_station_action_items(stations: &mut Vec<crate::api::models::Station>, shuffle_active: bool) {
    use crate::api::models::Station;
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
    let title = col.items.get(album_idx)
        .map(|item| format!("tracks \u{2014} {}", item.title()))
        .unwrap_or_else(|| "tracks".to_string());
    Some(BrowseColumn::new_with_tracks(title, items, tracks))
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

/// Get artist key and name for the bio popup (F4).
/// Priority: selected track → selected album → selected artist → now-playing track.
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
                if let Some(found) = state.artists.iter().find(|a| a.title == *track_artist) {
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
                                // Look up album in state.albums to get artist key
                                if let Some(album) = state.albums.iter().find(|a| a.rating_key == *key) {
                                    if let (Some(artist_key), Some(artist_name)) = (&album.parent_rating_key, &album.parent_title) {
                                        return Some((artist_key.clone(), artist_name.clone()));
                                    }
                                }
                                // Fall back to artist name from BrowseItem
                                if !artist.is_empty() {
                                    // Try to find artist by name in state.artists
                                    if let Some(found) = state.artists.iter().find(|a| a.title == *artist) {
                                        return Some((found.rating_key.clone(), found.title.clone()));
                                    }
                                }
                            }
                            BrowseItem::Artist { key, title, .. } => {
                                return Some((key.clone(), title.clone()));
                            }
                            BrowseItem::AllTracks { artist_key, artist_name, .. } => {
                                return Some((artist_key.clone(), artist_name.clone()));
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
                _ => &state.queue,
            };
            if let Some(track) = tracks.get(state.list_state.queue_index) {
                if let Some(result) = artist_from_track(track) {
                    return Some(result);
                }
            }
        }
        View::Search => {
            // Check selected search result based on active tab
            if let Some(ref results) = state.search_results {
                use crate::app::state::SearchTab;
                let idx = state.list_state.search_item_index;

                match state.search_tab {
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
    client: &crate::api::PlexClient,
    call: F,
    on_success: fn(T) -> crate::app::Event,
    error_msg: &str,
) where
    F: FnOnce(crate::api::PlexClient) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, crate::api::ApiError>> + Send,
    T: Send + 'static,
{
    let tx = event_tx.clone();
    let c = client.clone();
    let msg = error_msg.to_string();
    tokio::spawn(async move {
        match call(c).await {
            Ok(data) => { let _ = tx.send(on_success(data)).await; }
            Err(e) => { let _ = tx.send(crate::app::Event::DataLoadError(format!("{}: {}", msg, e))).await; }
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
