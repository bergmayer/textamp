//! Search dispatch handlers: ExecuteLocalSearch, ClearSearch, SelectSearchResult,
//! ActivateListFilter, DeactivateListFilter, FilteredList*,
//! SelectFilteredItem, AppendListFilterChar, DeleteListFilterChar, ClearListFilter,
//! ExecuteListFilter, OpenSearchPopup, CloseSearchPopup, OpenLibraryPicker,
//! CloseLibraryPicker.

use crate::app::{Action, AppState, Event};
use crate::app::state::{BrowseCategory, SearchFocus, SearchTab, View};
use crate::api::PlexClient;
use crate::api::models::SearchResults;

use anyhow::Result;
use tokio::sync::mpsc;

/// Max results per category in launcher/radio search.
const SEARCH_RESULT_LIMIT: usize = 20;
/// Max results in artist radio picker filter.
const ARTIST_PICKER_LIMIT: usize = 50;

/// Normalize a query string for fuzzy matching (lowercase, alphanumeric + whitespace only).
fn normalize_query(query: &str) -> (String, String) {
    let lower = query.to_lowercase();
    let normalized: String = lower.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    (lower, normalized)
}

/// Check if a title fuzzy-matches a query (exact lowercase or normalized match).
fn fuzzy_matches(title: &str, query: &str, query_normalized: &str) -> bool {
    let lower = title.to_lowercase();
    lower.contains(query) || {
        let norm: String = lower.chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect();
        norm.contains(query_normalized)
    }
}


/// Dispatch search and filter actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: Action,
    state: &mut AppState,
    client: &mut PlexClient,
) -> Result<Vec<Action>> {
    let mut follow_ups = vec![];

    match action {
        Action::ExecuteLocalSearch => {
            let query = state.search_query.to_lowercase();
            if query.is_empty() {

                state.search_track_loading = false;
                return Ok(vec![]);
            }

            // Ranked local search: filter artists, albums, playlists, genres from cached data
            use crate::services::{search_with_ranking, search_albums_with_ranking};
            let mut artists = search_with_ranking(&state.artists, &query, |a| &a.title, 50);

            // Also find artists whose aliases match the query (using normalized matching)
            let alias_extras: Vec<_> = {
                let existing_keys: std::collections::HashSet<&str> = artists.iter()
                    .map(|a| a.rating_key.as_str())
                    .collect();
                let query_norm = crate::services::artist_alias_service::normalize_artist_name(&query);
                state.artists.iter()
                    .filter(|a| !existing_keys.contains(a.rating_key.as_str()))
                    .filter(|a| {
                        state.artist_aliases.get(&a.rating_key)
                            .map_or(false, |aliases| aliases.iter().any(|al| {
                                let norm = crate::services::artist_alias_service::normalize_artist_name(al);
                                norm.contains(&query_norm)
                            }))
                    })
                    .cloned()
                    .collect()
            };
            artists.extend(alias_extras);

            let albums = search_albums_with_ranking(&state.albums, &query, 50);
            let playlists = search_with_ranking(&state.playlists, &query, |p| &p.title, 50);
            let genres = search_with_ranking(&state.genres, &query, |g| &g.title, 50);

            // Store local results immediately (tracks filled async by API)
            state.search_results = Some(SearchResults {
                artists,
                albums,
                playlists,
                genres,
                tracks: vec![],
            });

            // Fire async API search for tracks with debounce
            let need_tracks = matches!(state.search_tab, SearchTab::Global | SearchTab::Tracks);
            if need_tracks && state.search_query.len() >= 2 {
                state.search_track_version = state.search_track_version.wrapping_add(1);
                let version = state.search_track_version;
                state.search_track_loading = true;

                let event_tx = event_tx.clone();
                let query = state.search_query.clone();
                let search_client = client.clone();

                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
                    match search_client.search(&query).await {
                        Ok(results) => {
                            let _ = event_tx.send(Event::TrackSearchCompleted {
                                version,
                                tracks: results.tracks,
                            }).await;
                        }
                        Err(_) => {
                            let _ = event_tx.send(Event::TrackSearchCompleted {
                                version,
                                tracks: vec![],
                            }).await;
                        }
                    }
                });
            } else {
                state.search_track_loading = false;
            }
        }
        Action::SelectSearchResult => {
            let follow_up_actions = select_search_result(state);
            follow_ups.extend(follow_up_actions);
        }

        // Inline list filter actions
        Action::ActivateListFilter => {
            state.list_filter.active = true;
            state.list_filter.query.clear();
            state.list_filter.results = None;
            state.list_filter.loading = false;
            state.list_filter.selected = 0;
            // Capture which category and column the filter was activated on
            state.list_filter.category = state.browse_category;
            state.list_filter.column = match state.browse_category {
                BrowseCategory::Library => state.artist_nav.focused_column,
                BrowseCategory::Playlists => state.playlist_nav.focused_column,
                BrowseCategory::Genres => state.genre_nav.focused_column,
                BrowseCategory::Folders => {
                    state.folder_state.as_ref().map(|fs| fs.focused_column).unwrap_or(0)
                }
            };
        }
        Action::DeactivateListFilter => {
            state.list_filter.active = false;
            state.list_filter.query.clear();
            state.list_filter.results = None;
            state.list_filter.loading = false;
            state.list_filter.selected = 0;
        }
        Action::FilteredListUp => {
            if state.list_filter.selected > 0 {
                state.list_filter.selected -= 1;
                if let Some(ref results) = state.list_filter.results {
                    if let Some(&item_idx) = results.matched_indices.get(state.list_filter.selected) {
                        super::key_input::update_filter_column_selection(state, item_idx);
                    }
                }
                super::key_input::truncate_filter_right_columns(state);
            }
        }
        Action::FilteredListDown => {
            if let Some(ref results) = state.list_filter.results {
                if state.list_filter.selected + 1 < results.matched_indices.len() {
                    state.list_filter.selected += 1;
                    if let Some(&item_idx) = results.matched_indices.get(state.list_filter.selected) {
                        super::key_input::update_filter_column_selection(state, item_idx);
                    }
                    super::key_input::truncate_filter_right_columns(state);
                }
            }
        }
        Action::SelectFilteredItem => {
            if let Some(ref results) = state.list_filter.results.clone() {
                if let Some(&item_idx) = results.matched_indices.get(state.list_filter.selected) {
                    super::key_input::update_filter_column_selection(state, item_idx);
                    // Deactivate filter before drill-down (new column clears filter)
                    state.list_filter.deactivate();
                    let drilldown_actions = super::key_input::get_filter_drilldown_actions(state);
                    follow_ups.extend(drilldown_actions);
                }
            }
        }
        Action::AppendListFilterChar(c) => {
            state.list_filter.query.push(c);
            state.list_filter.selected = 0;
            if is_on_filter_column(state) {
                super::key_input::truncate_filter_right_columns(state);
            }
            execute_list_filter(event_tx, state).await?;
        }
        Action::DeleteListFilterChar => {
            state.list_filter.query.pop();
            if state.list_filter.query.is_empty() {
                state.list_filter.active = false;
                state.list_filter.results = None;
                state.list_filter.loading = false;
                state.list_filter.selected = 0;
            } else if is_on_filter_column(state) {
                state.list_filter.selected = 0;
                super::key_input::truncate_filter_right_columns(state);
                execute_list_filter(event_tx, state).await?;
            } else {
                state.list_filter.selected = 0;
                execute_list_filter(event_tx, state).await?;
            }
        }
        // Search popup actions
        Action::OpenSearchPopup => {
            if state.list_filter.active {
                state.list_filter.active = false;
                state.list_filter.query.clear();
                state.list_filter.results = None;
                state.list_filter.loading = false;
                state.list_filter.selected = 0;
            }
            state.search_popup_active = true;
            state.search_focus = SearchFocus::Input;
            state.search_query.clear();
            state.search_results = None;
        }
        Action::CloseSearchPopup => {
            state.search_popup_active = false;
        }

        // Library picker popup actions
        Action::OpenLibraryPicker => {
            state.library_picker_active = true;
            if state.has_multiple_servers() {
                let all_libs = state.all_libraries_with_servers();
                state.library_picker_index = all_libs.iter()
                    .position(|(sid, _, lib)| {
                        state.active_library.as_deref() == Some(lib.key.as_str())
                            && state.active_server_id.as_deref() == Some(*sid)
                    })
                    .unwrap_or(0);
            } else if let Some(ref active_key) = state.active_library {
                state.library_picker_index = state.libraries.iter()
                    .position(|lib| lib.key == *active_key)
                    .unwrap_or(0);
            } else {
                state.library_picker_index = 0;
            }
        }
        Action::CloseLibraryPicker => {
            state.library_picker_active = false;
        }

        // Sort popup actions
        Action::OpenSortPopup => {
            use crate::app::state::{BrowseItem, SortColumnType, SortPopupState};

            if state.view != View::Browse {
                // no-op outside browse view
            } else if let Some(nav) = state.browse_nav() {
                let col_idx = nav.focused_column;
                if let Some(col) = nav.columns.get(col_idx) {
                    // Determine column type from content
                    let first_item = col.items.first();
                    let column_type = if first_item.map_or(false, |i| matches!(i, BrowseItem::Artist { .. })) || col.items.iter().take(3).any(|i| matches!(i, BrowseItem::Artist { .. })) {
                        SortColumnType::Artist
                    } else if first_item.map_or(false, |i| matches!(i, BrowseItem::Album { .. })) || col.items.iter().take(4).any(|i| matches!(i, BrowseItem::Album { .. })) {
                        SortColumnType::Album
                    } else if first_item.map_or(false, |i| matches!(i, BrowseItem::Track { .. })) {
                        // Determine if this is a special track column (all-tracks/playlist)
                        if state.is_special_track_column(nav, col_idx) {
                            SortColumnType::AllTracks
                        } else {
                            SortColumnType::Track
                        }
                    } else {
                        // Genre or other non-sortable column - don't open popup
                        return Ok(vec![]);
                    };

                    let is_playlist = state.browse_category == crate::app::state::BrowseCategory::Playlists;
                    let popup = SortPopupState::new(
                        col_idx,
                        col.title.clone(),
                        column_type,
                        col.sort_mode,
                        col.artwork_visible,
                        is_playlist,
                    );
                    state.sort_popup = Some(popup);
                }
            }
        }
        Action::CloseSortPopup => {
            state.sort_popup = None;
        }
        Action::CloseRadioLauncher => {
            state.radio_launcher = None;
        }
        Action::RadioLauncherSearch => {
            if let Some(ref mut launcher) = state.radio_launcher {
                if launcher.query.is_empty() {
                    launcher.results = None;
                    launcher.loading = false;
                } else {
                    let (query, query_normalized) = normalize_query(&launcher.query);

                    let artists: Vec<_> = state.artists.iter()
                        .filter(|a| {
                            if fuzzy_matches(&a.title, &query, &query_normalized) {
                                return true;
                            }
                            if let Some(aliases) = state.artist_aliases.get(&a.rating_key) {
                                aliases.iter().any(|alias| fuzzy_matches(alias, &query, &query_normalized))
                            } else {
                                false
                            }
                        })
                        .take(SEARCH_RESULT_LIMIT)
                        .cloned()
                        .collect();
                    let albums: Vec<_> = state.albums.iter()
                        .filter(|a| {
                            fuzzy_matches(&a.title, &query, &query_normalized)
                                || a.artist_name().to_lowercase().contains(&query)
                        })
                        .take(SEARCH_RESULT_LIMIT)
                        .cloned()
                        .collect();

                    // For tracks, search if query is 2+ characters (same as main search)
                    let need_tracks = launcher.query.len() >= 2;
                    if need_tracks {
                        launcher.loading = true;
                        // Fire async track search
                        let event_tx = event_tx.clone();
                        let client_clone = client.clone();
                        let q = launcher.query.clone();
                        tokio::spawn(async move {
                            match client_clone.search(&q).await {
                                Ok(results) => {
                                    let _ = event_tx.send(Event::TrackSearchCompleted {
                                        version: u64::MAX, // Special marker for radio launcher
                                        tracks: results.tracks,
                                    }).await;
                                }
                                Err(_) => {
                                    let _ = event_tx.send(Event::TrackSearchCompleted {
                                        version: u64::MAX,
                                        tracks: vec![],
                                    }).await;
                                }
                            }
                        });
                    }

                    // Store local results immediately (tracks will be updated async)
                    launcher.results = Some(SearchResults {
                        artists,
                        albums,
                        playlists: vec![],
                        genres: vec![],
                        tracks: vec![],
                    });
                    launcher.loading = need_tracks;
                    launcher.item_index = 0;
                    launcher.focus = SearchFocus::Input;
                }
            }
        }
        Action::RadioLauncherSelectResult => {
            follow_ups = select_radio_launcher_result(state);
        }

        // Adventure launcher popup actions
        Action::OpenAdventureLauncher => {
            state.adventure_launcher = Some(crate::app::state::AdventureLauncherState {
                step: crate::app::state::AdventureStep::FindStartTrack,
                query: String::new(),
                results: None,
                focus: SearchFocus::Input,
                item_index: 0,
                loading: false,
                drill: crate::app::state::AdventureDrillLevel::Search,
                start_track: None,
                track_count_input: String::new(),
                scroll_pin: None,
                search_tab: crate::app::state::SearchTab::default(),
            });
        }
        Action::CloseAdventureLauncher => {
            state.adventure_launcher = None;
        }
        Action::AdventureLauncherSearch => {
            adventure_launcher_search(event_tx, state, client).await?;
        }
        Action::AdventureLauncherDrillArtist { key, name } => {
            // Async fetch artist albums
            if let Some(ref mut launcher) = state.adventure_launcher {
                launcher.loading = true;
            }
            let event_tx = event_tx.clone();
            let client_clone = client.clone();
            let artist_key = key.clone();
            let artist_name = name.clone();
            tokio::spawn(async move {
                match client_clone.get_artist_albums(&artist_key).await {
                    Ok(albums) => {
                        let _ = event_tx.send(Event::AdventureLauncherAlbumsLoaded {
                            artist_key, artist_name, albums,
                        }).await;
                    }
                    Err(e) => {
                        tracing::warn!("Adventure launcher: failed to load artist albums: {}", e);
                        // Send empty to clear loading state
                        let _ = event_tx.send(Event::AdventureLauncherAlbumsLoaded {
                            artist_key, artist_name, albums: vec![],
                        }).await;
                    }
                }
            });
        }
        Action::AdventureLauncherDrillAlbum { key, title, artist_name } => {
            if let Some(ref mut launcher) = state.adventure_launcher {
                launcher.loading = true;
            }
            let event_tx = event_tx.clone();
            let client_clone = client.clone();
            let album_key = key.clone();
            let album_title = title.clone();
            let artist = artist_name.clone();
            tokio::spawn(async move {
                match client_clone.get_album_tracks(&album_key).await {
                    Ok(tracks) => {
                        let _ = event_tx.send(Event::AdventureLauncherTracksLoaded {
                            album_key, album_title, artist_name: artist, tracks,
                        }).await;
                    }
                    Err(e) => {
                        tracing::warn!("Adventure launcher: failed to load album tracks: {}", e);
                        let _ = event_tx.send(Event::AdventureLauncherTracksLoaded {
                            album_key, album_title, artist_name: artist, tracks: vec![],
                        }).await;
                    }
                }
            });
        }
        Action::AdventureLauncherSelectTrack => {
            follow_ups = adventure_launcher_select_track(event_tx, state, client).await?;
        }
        Action::AdventureLauncherBack => {
            adventure_launcher_back(state);
        }

        // Multi-artist radio picker actions
        Action::OpenArtistRadioPicker => {
            state.artist_radio_picker = Some(crate::app::state::ArtistRadioPickerState {
                step: crate::app::state::ArtistRadioPickerStep::EnterCount,
                max_artists: 0,
                count_input: String::new(),
                query: String::new(),
                filtered_artists: vec![],
                selected_artists: vec![],
                focus: SearchFocus::Input,
                item_index: 0,
                scroll_pin: None,
            });
        }
        Action::CloseArtistRadioPicker => {
            state.artist_radio_picker = None;
        }
        Action::ArtistRadioPickerSetCount => {
            if let Some(ref mut picker) = state.artist_radio_picker {
                let count = picker.count_input.parse::<usize>().unwrap_or(0).clamp(1, 12);
                picker.max_artists = count;
                picker.step = crate::app::state::ArtistRadioPickerStep::SelectArtists;
                picker.query.clear();
                picker.filtered_artists = state.artists.clone();
                picker.selected_artists.clear();
                picker.focus = SearchFocus::Input;
                picker.item_index = 0;
            }
        }
        Action::ArtistRadioPickerSearch => {
            if let Some(ref mut picker) = state.artist_radio_picker {
                if picker.query.is_empty() {
                    picker.filtered_artists = state.artists.clone();
                } else {
                    let (query, query_normalized) = normalize_query(&picker.query);

                    picker.filtered_artists = state.artists.iter()
                        .filter(|a| {
                            if fuzzy_matches(&a.title, &query, &query_normalized) {
                                return true;
                            }
                            // Also match artist aliases
                            if let Some(aliases) = state.artist_aliases.get(&a.rating_key) {
                                aliases.iter().any(|alias| fuzzy_matches(alias, &query, &query_normalized))
                            } else {
                                false
                            }
                        })
                        .take(ARTIST_PICKER_LIMIT)
                        .cloned()
                        .collect();
                }
            }
        }
        Action::ArtistRadioPickerToggleArtist => {
            if let Some(ref mut picker) = state.artist_radio_picker {
                if let Some(artist) = picker.filtered_artists.get(picker.item_index).cloned() {
                    // Toggle: remove if already selected, add if not
                    if let Some(pos) = picker.selected_artists.iter().position(|a| a.rating_key == artist.rating_key) {
                        picker.selected_artists.remove(pos);
                    } else if picker.selected_artists.len() < picker.max_artists {
                        let added_key = artist.rating_key.clone();
                        picker.selected_artists.push(artist);

                        // Auto-launch if max artists reached
                        if picker.selected_artists.len() == picker.max_artists {
                            follow_ups.push(Action::ArtistRadioPickerLaunch);
                        } else {
                            // Clear query and re-populate with all artists, position near selected
                            picker.query.clear();
                            picker.filtered_artists = state.artists.clone();
                            picker.focus = SearchFocus::Input;
                            // Position item_index at the just-added artist in the full list
                            picker.item_index = picker.filtered_artists.iter()
                                .position(|a| a.rating_key == added_key)
                                .unwrap_or(0);
                            picker.scroll_pin = None;
                        }
                    }
                }
            }
        }
        // Artist bio popup (F4)
        Action::ShowArtistBio { artist_key, artist_name } => {
            // Initialize popup in loading state
            state.artist_bio_popup = Some(crate::app::state::ArtistBioPopup {
                artist_name: artist_name.clone(),
                bio: String::new(),
                scroll: 0,
                loading: true,
                artwork_data: None,
                artwork_thumb: None,
            });

            // Fetch artist details from API
            let tx = event_tx.clone();
            let client_clone = client.clone();
            tokio::spawn(async move {
                match client_clone.get_artist(&artist_key).await {
                    Ok(artist) => {
                        let bio = artist.summary.unwrap_or_else(|| "No biography available.".to_string());
                        let thumb = artist.thumb.clone();
                        let _ = tx.send(Event::ArtistBioLoaded { artist_name, bio, thumb: thumb.clone() }).await;

                        // Fetch artwork if thumb is available
                        if let Some(thumb_path) = thumb {
                            match client_clone.fetch_artwork(&thumb_path, 600).await {
                                Ok(data) => {
                                    let _ = tx.send(Event::ArtistBioArtworkLoaded { data, thumb: thumb_path }).await;
                                }
                                Err(e) => {
                                    tracing::warn!("Failed to fetch artist artwork: {}", e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to fetch artist bio: {}", e);
                        let _ = tx.send(Event::ArtistBioLoaded {
                            artist_name,
                            bio: format!("Failed to load biography: {}", e),
                            thumb: None,
                        }).await;
                    }
                }
            });
        }

        Action::ArtistRadioPickerLaunch => {
            if let Some(picker) = state.artist_radio_picker.take() {
                if picker.selected_artists.is_empty() {
                    return Ok(vec![]);
                }
                state.set_status(format!("Artist radio: blending {} artists...", picker.selected_artists.len()));

                let tx = event_tx.clone();
                let client_clone = client.clone();
                let artists = picker.selected_artists;

                tokio::spawn(async move {
                    use std::collections::HashSet;

                    let mut all_tracks: Vec<Vec<crate::api::models::Track>> = Vec::new();

                    // Fetch radio for each artist in parallel
                    let mut handles = Vec::new();
                    for artist in &artists {
                        let mut c = client_clone.clone();
                        let key = artist.rating_key.clone();
                        handles.push(tokio::spawn(async move {
                            c.create_radio_from_metadata(&key).await
                        }));
                    }

                    for handle in handles {
                        match handle.await {
                            Ok(Ok(tracks)) => all_tracks.push(tracks),
                            Ok(Err(e)) => {
                                tracing::warn!("Artist radio fetch failed: {}", e);
                            }
                            Err(e) => {
                                tracing::warn!("Artist radio task failed: {}", e);
                            }
                        }
                    }

                    if all_tracks.is_empty() {
                        let _ = tx.send(Event::ArtistRadioComplete { tracks: vec![] }).await;
                        return;
                    }

                    // Round-robin interleave, deduplicate by rating_key
                    let mut merged = Vec::new();
                    let mut seen = HashSet::new();
                    let max_len = all_tracks.iter().map(|t| t.len()).max().unwrap_or(0);

                    for i in 0..max_len {
                        for artist_tracks in &all_tracks {
                            if let Some(track) = artist_tracks.get(i) {
                                if seen.insert(track.rating_key.clone()) {
                                    merged.push(track.clone());
                                }
                            }
                        }
                    }

                    let _ = tx.send(Event::ArtistRadioComplete { tracks: merged }).await;
                });
            }
        }

        _ => unreachable!("dispatch_search called with non-search action: {:?}", action),
    }
    Ok(follow_ups)
}

/// Handle radio launcher result selection — start Plex radio from the selected item.
fn select_radio_launcher_result(state: &mut AppState) -> Vec<Action> {
    let launcher = match state.radio_launcher.take() {
        Some(l) => l,
        None => return vec![],
    };
    let results = match launcher.results {
        Some(r) => r,
        None => return vec![],
    };
    let idx = launcher.item_index;

    // Map global index to (section, local_index) for All tab
    let (section_type, local_idx) = if launcher.tab == crate::app::state::RadioLauncherTab::All {
        resolve_radio_launcher_index(&results, idx)
    } else {
        (launcher.tab, idx)
    };

    use crate::app::state::RadioLauncherTab;
    match section_type {
        RadioLauncherTab::Artists | RadioLauncherTab::All => {
            if let Some(artist) = results.artists.get(local_idx) {
                return vec![Action::StartPlexRadio {
                    key: artist.rating_key.clone(),
                    title: format!("{} Radio", artist.title),
                }];
            }
        }
    }
    vec![]
}

/// Resolve a flat index in radio launcher All tab to (section, local_index).
/// Radio launcher is artist-only, so All tab just maps directly to artists.
fn resolve_radio_launcher_index(results: &SearchResults, global_idx: usize) -> (crate::app::state::RadioLauncherTab, usize) {
    use crate::app::state::RadioLauncherTab;

    // Artists only
    if global_idx < results.artists.len() {
        return (RadioLauncherTab::Artists, global_idx);
    }

    (RadioLauncherTab::All, 0)
}

/// Handle SelectSearchResult — navigate to the selected item in the library.
fn select_search_result(state: &mut AppState) -> Vec<Action> {
    let results = match state.search_results.take() {
        Some(r) => r,
        None => return vec![],
    };
    let idx = state.list_state.search_item_index;

    // For the All tab, map global index to (section, local_index)
    let (section, local_idx) = if state.search_tab == SearchTab::Global {
        resolve_global_index(&results, idx)
    } else {
        (state.search_tab, idx)
    };

    match section {
        SearchTab::Artists => {
            if let Some(artist) = results.artists.get(local_idx) {
                // Navigate to artist in Library view
                let artist_key = artist.rating_key.clone();
                state.search_query.clear();

                state.search_popup_active = false;
                state.browse_category = BrowseCategory::Library;
                state.set_view(View::Browse);

                // Find artist in artist_nav and select it
                if let Some(col) = state.artist_nav.columns.get_mut(0) {
                    if let Some(pos) = col.items.iter().position(|i| i.key() == artist_key) {
                        col.selected_index = pos;
                        state.artist_nav.focused_column = 0;
                        state.artist_nav.truncate_right();
                        return vec![Action::LoadArtistAlbumsForMiller { artist_key }];
                    }
                }
                // Artist not in nav (cache empty?) — load from scratch
                return vec![Action::LoadArtistAlbumsForMiller { artist_key }];
            }
        }
        SearchTab::Albums => {
            if let Some(album) = results.albums.get(local_idx) {
                let album_key = album.rating_key.clone();
                let artist_key = album.parent_rating_key.clone();
                state.search_query.clear();

                state.search_popup_active = false;
                state.browse_category = BrowseCategory::Library;
                state.set_view(View::Browse);
                state.pending_album_key = Some(album_key);

                // If we have the parent artist key, navigate to them
                if let Some(ref ak) = artist_key {
                    state.selected_artist_name = album.artist_name().to_string();
                    if let Some(col) = state.artist_nav.columns.get_mut(0) {
                        if let Some(pos) = col.items.iter().position(|i| i.key() == ak.as_str()) {
                            col.selected_index = pos;
                            state.artist_nav.focused_column = 0;
                            state.artist_nav.truncate_right();
                            return vec![Action::LoadArtistAlbumsForMiller { artist_key: ak.clone() }];
                        }
                    }
                    return vec![Action::LoadArtistAlbumsForMiller { artist_key: ak.clone() }];
                }
                // No parent artist key — try All Artists column
                state.selected_artist_name = "All Artists".to_string();
                if let Some(col) = state.artist_nav.columns.get_mut(0) {
                    // Select "All Artists" entry (index 0)
                    col.selected_index = 0;
                    state.artist_nav.focused_column = 0;
                    state.artist_nav.truncate_right();
                }
                return vec![Action::LoadAllAlbumsForMiller];
            }
        }
        SearchTab::Tracks => {
            if let Some(track) = results.tracks.get(local_idx) {
                let artist_key = track.grandparent_rating_key.clone();
                let album_key = track.parent_rating_key.clone();
                let track_key = track.rating_key.clone();
                let track_owned = track.clone();
                state.search_query.clear();

                state.search_popup_active = false;
                state.browse_category = BrowseCategory::Library;
                state.set_view(View::Browse);
                state.pending_album_key = album_key;
                state.pending_track_key = Some(track_key);

                if let Some(ref ak) = artist_key {
                    state.selected_artist_name = track.artist_name().to_string();
                    if let Some(col) = state.artist_nav.columns.get_mut(0) {
                        if let Some(pos) = col.items.iter().position(|i| i.key() == ak.as_str()) {
                            col.selected_index = pos;
                            state.artist_nav.focused_column = 0;
                            state.artist_nav.truncate_right();
                            return vec![Action::LoadArtistAlbumsForMiller { artist_key: ak.clone() }];
                        }
                    }
                    return vec![Action::LoadArtistAlbumsForMiller { artist_key: ak.clone() }];
                }
                // No artist key — play the track directly
                state.pending_album_key = None;
                state.pending_track_key = None;
                return vec![Action::PlayTrack(track_owned)];
            }
        }
        SearchTab::Playlists => {
            if let Some(playlist) = results.playlists.get(local_idx) {
                let playlist_key = playlist.rating_key.clone();
                state.search_query.clear();

                state.search_popup_active = false;
                state.browse_category = BrowseCategory::Playlists;
                state.set_view(View::Browse);

                // Find playlist in playlist_nav and select it
                if let Some(col) = state.playlist_nav.columns.get_mut(0) {
                    if let Some(pos) = col.items.iter().position(|i| i.key() == playlist_key) {
                        col.selected_index = pos;
                        state.playlist_nav.focused_column = 0;
                        state.playlist_nav.truncate_right();
                        return vec![Action::LoadPlaylistTracksForMiller { playlist_key }];
                    }
                }
                return vec![Action::LoadPlaylistTracksForMiller { playlist_key }];
            }
        }
        SearchTab::Genres => {
            if let Some(genre) = results.genres.get(local_idx) {
                let genre_key = genre.effective_key().to_string();
                state.search_query.clear();

                state.search_popup_active = false;
                state.browse_category = BrowseCategory::Genres;
                state.set_view(View::Browse);

                // Find genre in genre_nav and select it
                if let Some(col) = state.genre_nav.columns.get_mut(0) {
                    if let Some(pos) = col.items.iter().position(|i| i.key() == genre_key) {
                        col.selected_index = pos;
                        state.genre_nav.focused_column = 0;
                        state.genre_nav.truncate_right();
                        return vec![Action::LoadGenreAlbumsForMiller { genre_key }];
                    }
                }
                return vec![Action::LoadGenreAlbumsForMiller { genre_key }];
            }
        }
        _ => {}
    }

    vec![]
}

/// For the All tab, map a global flat index to (section, local_index).
/// Sections order: Artists, Albums, Playlists, Genres, Tracks.
pub fn resolve_global_index(results: &SearchResults, global_idx: usize) -> (SearchTab, usize) {
    let mut offset = 0;

    let artists_len = results.artists.len();
    if global_idx < offset + artists_len {
        return (SearchTab::Artists, global_idx - offset);
    }
    offset += artists_len;

    let albums_len = results.albums.len();
    if global_idx < offset + albums_len {
        return (SearchTab::Albums, global_idx - offset);
    }
    offset += albums_len;

    let playlists_len = results.playlists.len();
    if global_idx < offset + playlists_len {
        return (SearchTab::Playlists, global_idx - offset);
    }
    offset += playlists_len;

    let genres_len = results.genres.len();
    if global_idx < offset + genres_len {
        return (SearchTab::Genres, global_idx - offset);
    }
    offset += genres_len;

    let tracks_len = results.tracks.len();
    if global_idx < offset + tracks_len {
        return (SearchTab::Tracks, global_idx - offset);
    }

    // Fallback
    (SearchTab::Artists, 0)
}

/// Execute inline list filter with debounce.
async fn execute_list_filter(
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,
) -> Result<()> {
    use crate::services::{filter_browse_items, filter_folder_items, DEFAULT_MAX_RESULTS};

    state.list_filter.version = state.list_filter.version.wrapping_add(1);
    let version = state.list_filter.version;
    let query = state.list_filter.query.clone();

    if query.is_empty() {
        state.list_filter.results = None;
        state.list_filter.loading = false;
        return Ok(());
    }

    state.list_filter.loading = true;

    let event_tx = event_tx.clone();
    let category = state.list_filter.category;
    let column = state.list_filter.column;

    let aliases = state.artist_aliases.clone();
    let comp_keys = state.compilation_artist_keys.clone();
    let empty_comp_keys = std::collections::HashSet::new();

    match category {
        BrowseCategory::Library => {
            if let Some(col) = state.artist_nav.columns.get(column) {
                let items: Vec<_> = col.items.clone();
                let aliases = aliases.clone();
                let comp_keys = comp_keys.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                    let results = filter_browse_items(&items, &query, DEFAULT_MAX_RESULTS, &aliases, &comp_keys);
                    let _ = event_tx.send(Event::ListFilterCompleted { version, results }).await;
                });
            }
        }
        BrowseCategory::Playlists => {
            if let Some(col) = state.playlist_nav.columns.get(column) {
                let items: Vec<_> = col.items.clone();
                let aliases = aliases.clone();
                let empty = empty_comp_keys.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                    let results = filter_browse_items(&items, &query, DEFAULT_MAX_RESULTS, &aliases, &empty);
                    let _ = event_tx.send(Event::ListFilterCompleted { version, results }).await;
                });
            }
        }
        BrowseCategory::Genres => {
            if let Some(col) = state.genre_nav.columns.get(column) {
                let items: Vec<_> = col.items.clone();
                let aliases = aliases.clone();
                let empty = empty_comp_keys.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                    let results = filter_browse_items(&items, &query, DEFAULT_MAX_RESULTS, &aliases, &empty);
                    let _ = event_tx.send(Event::ListFilterCompleted { version, results }).await;
                });
            }
        }
        BrowseCategory::Folders => {
            if let Some(ref folder_state) = state.folder_state {
                if let Some(col) = folder_state.columns.get(column) {
                    let items: Vec<_> = col.items.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                        let results = filter_folder_items(&items, &query, DEFAULT_MAX_RESULTS);
                        let _ = event_tx.send(Event::ListFilterCompleted { version, results }).await;
                    });
                }
            }
        }
    }

    Ok(())
}

/// Adventure launcher: perform search (local artists/albums + async API tracks).
async fn adventure_launcher_search(
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,
    client: &mut PlexClient,
) -> Result<()> {
    let launcher = match state.adventure_launcher.as_mut() {
        Some(l) => l,
        None => return Ok(()),
    };

    // Reset drill to search level when searching
    launcher.drill = crate::app::state::AdventureDrillLevel::Search;

    if launcher.query.is_empty() {
        launcher.results = None;
        launcher.loading = false;
        return Ok(());
    }

    // Local ranked search for artists, albums, playlists, genres
    use crate::services::{search_with_ranking, search_albums_with_ranking};
    let query = launcher.query.to_lowercase();
    let mut artists = search_with_ranking(&state.artists, &query, |a| &a.title, SEARCH_RESULT_LIMIT);

    // Also find artists whose aliases match the query (using normalized matching)
    let alias_extras: Vec<_> = {
        let existing_keys: std::collections::HashSet<&str> = artists.iter()
            .map(|a| a.rating_key.as_str())
            .collect();
        let query_norm = crate::services::artist_alias_service::normalize_artist_name(&query);
        state.artists.iter()
            .filter(|a| !existing_keys.contains(a.rating_key.as_str()))
            .filter(|a| {
                state.artist_aliases.get(&a.rating_key)
                    .map_or(false, |aliases| aliases.iter().any(|al| {
                        let norm = crate::services::artist_alias_service::normalize_artist_name(al);
                        norm.contains(&query_norm)
                    }))
            })
            .cloned()
            .collect()
    };
    artists.extend(alias_extras);

    let albums = search_albums_with_ranking(&state.albums, &query, SEARCH_RESULT_LIMIT);
    let playlists = search_with_ranking(&state.playlists, &query, |p| &p.title, SEARCH_RESULT_LIMIT);
    let genres = search_with_ranking(&state.genres, &query, |g| &g.title, SEARCH_RESULT_LIMIT);

    // Async API search for tracks
    let need_tracks = launcher.query.len() >= 2;
    if need_tracks {
        launcher.loading = true;
        let event_tx = event_tx.clone();
        let client_clone = client.clone();
        let q = launcher.query.clone();
        tokio::spawn(async move {
            match client_clone.search(&q).await {
                Ok(results) => {
                    let _ = event_tx.send(Event::TrackSearchCompleted {
                        version: u64::MAX - 1, // Marker for adventure launcher
                        tracks: results.tracks,
                    }).await;
                }
                Err(_) => {
                    let _ = event_tx.send(Event::TrackSearchCompleted {
                        version: u64::MAX - 1,
                        tracks: vec![],
                    }).await;
                }
            }
        });
    }

    launcher.results = Some(SearchResults {
        artists,
        albums,
        playlists,
        genres,
        tracks: vec![],
    });
    launcher.loading = need_tracks;
    launcher.item_index = 0;
    launcher.focus = SearchFocus::Input;

    Ok(())
}

/// Adventure launcher: select track from current drill level.
async fn adventure_launcher_select_track(
    _event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,
    client: &mut PlexClient,
) -> Result<Vec<Action>> {
    use crate::app::state::{AdventureStep, AdventureDrillLevel};

    let launcher = match state.adventure_launcher.as_mut() {
        Some(l) => l,
        None => return Ok(vec![]),
    };

    // Extract the selected track based on drill level
    let track = match &launcher.drill {
        AdventureDrillLevel::AlbumTracks { tracks, .. } => {
            tracks.get(launcher.item_index).cloned()
        }
        AdventureDrillLevel::Search => {
            // Tab-aware track selection
            if let Some(ref results) = launcher.results {
                let idx = launcher.item_index;
                match launcher.search_tab {
                    crate::app::state::SearchTab::Tracks => {
                        results.tracks.get(idx).cloned()
                    }
                    crate::app::state::SearchTab::Global => {
                        let artist_count = results.artists.len();
                        let album_count = results.albums.len();
                        if idx >= artist_count + album_count {
                            results.tracks.get(idx - artist_count - album_count).cloned()
                        } else {
                            None
                        }
                    }
                    _ => None
                }
            } else {
                None
            }
        }
        AdventureDrillLevel::ArtistAlbums { .. } => {
            None // Can't select a track from album list
        }
    };

    let track = match track {
        Some(t) => t,
        None => return Ok(vec![]),
    };

    match launcher.step {
        AdventureStep::FindStartTrack => {
            launcher.start_track = Some(track);
            launcher.step = AdventureStep::EnterTrackCount;
            launcher.track_count_input = "20".to_string();
            // Clear search state for next step
            launcher.query.clear();
            launcher.results = None;
            launcher.drill = AdventureDrillLevel::Search;
            launcher.item_index = 0;
            launcher.focus = SearchFocus::Input;
        }
        AdventureStep::FindEndTrack => {
            let start = launcher.start_track.clone();
            let count_str = launcher.track_count_input.clone();
            let count = count_str.parse::<usize>().unwrap_or(20).clamp(5, 100);

            // Close launcher before starting generation
            state.adventure_launcher = None;

            if let Some(start_track) = start {
                state.set_status("Adventure: generating sonic bridge...".to_string());

                // Generate adventure
                match crate::services::generate_adventure_for_library(client, &start_track, &track, count, state.active_library.as_deref()).await {
                    Ok(tracks) => {
                        if tracks.len() <= 2 {
                            state.set_error("Adventure: no similar tracks found for these songs. Try different tracks with sonic analysis data.".to_string());
                            return Ok(vec![]);
                        }
                        return Ok(vec![Action::AdventureComplete(tracks)]);
                    }
                    Err(e) => {
                        return Ok(vec![Action::AdventureError(format!("{}", e))]);
                    }
                }
            }
        }
        AdventureStep::EnterTrackCount => {
            // Shouldn't happen (track count step doesn't select tracks)
        }
    }

    Ok(vec![])
}

/// Adventure launcher: handle back navigation.
fn adventure_launcher_back(state: &mut AppState) {
    use crate::app::state::{AdventureStep, AdventureDrillLevel};

    let launcher = match state.adventure_launcher.as_mut() {
        Some(l) => l,
        None => return,
    };

    match &launcher.drill {
        AdventureDrillLevel::AlbumTracks { .. } => {
            // Go back to search level (we don't cache the artist albums level)
            launcher.drill = AdventureDrillLevel::Search;
            launcher.item_index = 0;
            launcher.focus = SearchFocus::Results;
        }
        AdventureDrillLevel::ArtistAlbums { .. } => {
            launcher.drill = AdventureDrillLevel::Search;
            launcher.item_index = 0;
            launcher.focus = SearchFocus::Results;
        }
        AdventureDrillLevel::Search => {
            // At search level, back depends on step
            match launcher.step {
                AdventureStep::FindStartTrack => {
                    // Close the launcher
                    state.adventure_launcher = None;
                }
                AdventureStep::EnterTrackCount => {
                    // Go back to FindStartTrack
                    launcher.step = AdventureStep::FindStartTrack;
                    launcher.start_track = None;
                    launcher.query.clear();
                    launcher.item_index = 0;
                    launcher.focus = SearchFocus::Input;
                    // Pre-populate with all artists
                    let _ = launcher;
                    let artists = state.artists.clone();
                    if let Some(ref mut l) = state.adventure_launcher {
                        l.results = Some(SearchResults {
                            artists,
                            albums: vec![],
                            playlists: vec![],
                            genres: vec![],
                            tracks: vec![],
                        });
                    }
                }
                AdventureStep::FindEndTrack => {
                    // Go back to EnterTrackCount
                    launcher.step = AdventureStep::EnterTrackCount;
                }
            }
        }
    }
}

/// Check if the currently focused column is the filter's target column.
pub(super) fn is_on_filter_column(state: &AppState) -> bool {
    let filter_col = state.list_filter.column;
    match state.list_filter.category {
        BrowseCategory::Library => state.artist_nav.focused_column == filter_col,
        BrowseCategory::Playlists => state.playlist_nav.focused_column == filter_col,
        BrowseCategory::Genres => state.genre_nav.focused_column == filter_col,
        BrowseCategory::Folders => {
            state.folder_state.as_ref()
                .map(|fs| fs.focused_column == filter_col)
                .unwrap_or(true)
        }
    }
}
