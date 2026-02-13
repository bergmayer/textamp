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
mod connection;
mod navigation;
mod playback;
mod preload;
mod refresh;

// Re-export all public items for backward compatibility.
// Call sites continue to use `helpers::function_name()`.
pub use cache::maybe_save_cache_async;
pub use connection::{find_working_connection, find_working_connection_from_servers};
pub use navigation::{
    adjust_list_index, calc_scroll_offset, load_albums, load_artists,
    load_playlists, maybe_load_more, set_list_index,
};
pub use playback::{
    collect_tracks_from_column, fetch_more_radio_tracks, generate_plex_session_id,
    get_upcoming_tracks, play_current_track, play_track, report_playback_progress_to_plex,
    report_playback_stop_to_plex,
};
pub use preload::{maybe_start_subfolder_preload, preload_all_library_data, preload_data, SubfolderPreloadResult};
pub use refresh::{
    is_viewing_category, check_staleness_on_view_load, current_view_category,
    refresh_current_view, spawn_category_refresh,
};

/// Page size for paginated API requests.
pub const PAGE_SIZE: u32 = 100;

/// Append "Start Radio" and "Sonic Adventure" action items to a station list.
/// Used when building station_nav from any source (API, cache, preload).
pub fn append_station_action_items(stations: &mut Vec<crate::api::models::Station>) {
    use crate::api::models::Station;
    // Only append if not already present (avoid duplicates on reload)
    if stations.iter().any(|s| s.key == "action:start_radio") {
        return;
    }
    stations.push(Station {
        key: "action:start_radio".to_string(),
        title: "Start Radio".to_string(),
        station_type: "action".to_string(),
        identifier: None,
        thumb: None,
        art: None,
        description: Some("Search for an artist, album, or track to start radio".to_string()),
    });
    stations.push(Station {
        key: "action:adventure".to_string(),
        title: "Sonic Adventure".to_string(),
        station_type: "action".to_string(),
        identifier: None,
        thumb: None,
        art: None,
        description: Some("Create a sonic bridge between two tracks".to_string()),
    });
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
