//! Search dispatch handlers: ExecuteLocalSearch, ClearSearch, SelectSearchResult,
//! ActivateListFilter, DeactivateListFilter, FilteredList*,
//! SelectFilteredItem, AppendListFilterChar, DeleteListFilterChar, ClearListFilter,
//! ExecuteListFilter, OpenSearchPopup, CloseSearchPopup, OpenLibraryPicker,
//! CloseLibraryPicker.

use crate::app::action::SearchAction;
use crate::app::event::*;
use crate::app::state::{BrowseCategory, BrowseItem, SearchFocus, SearchTab, View};
use crate::app::{Action, AppState, Event};
use crate::library::models::SearchResults;

use anyhow::Result;
use tokio::sync::mpsc;

/// Max results per category in launcher/radio search.
const SEARCH_RESULT_LIMIT: usize = 20;
/// Max results in artist radio picker filter.
const ARTIST_PICKER_LIMIT: usize = 50;

/// Normalize a query string for fuzzy matching (lowercase, alphanumeric + whitespace only).
fn normalize_query(query: &str) -> (String, String) {
    let lower = query.to_lowercase();
    let normalized: String = lower
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    (lower, normalized)
}

/// Check if a title fuzzy-matches a query (exact lowercase or normalized match).
fn fuzzy_matches(title: &str, query: &str, query_normalized: &str) -> bool {
    let lower = title.to_lowercase();
    lower.contains(query) || {
        let norm: String = lower
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect();
        norm.contains(query_normalized)
    }
}

/// Dispatch search and filter actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: SearchAction,
    state: &mut AppState,
) -> Result<Vec<Action>> {
    let mut follow_ups = vec![];

    match action {
        SearchAction::ExecuteLocalSearch => {
            let query = state.search.query.to_lowercase();
            if query.is_empty() {
                state.search.results = None;
                state.list_state.search_item_index = 0;
                return Ok(vec![]);
            }
            if let Some(source) = state.sources.active.folder() {
                let tracks: Vec<_> = state
                    .folder_state
                    .as_ref()
                    .and_then(|nav| nav.focused())
                    .into_iter()
                    .flat_map(|column| &column.items)
                    .filter(|item| item.is_track())
                    .map(|item| crate::library::track::Track::from_folder(source, &item.key))
                    .collect();
                state.search.results = Some(SearchResults {
                    tracks: crate::services::search_with_ranking(
                        &tracks,
                        &query,
                        |track| &track.title,
                        100,
                    ),
                    ..Default::default()
                });
                return Ok(vec![]);
            }

            // Ranked local search: filter artists, albums, playlists, genres from cached data
            use crate::services::{search_albums_with_ranking, search_with_ranking};
            let mut artists = search_with_ranking(&state.library.artists, &query, |a| &a.title, 50);

            // Also find artists whose aliases match the query (using normalized matching)
            let alias_extras: Vec<_> = {
                let existing_keys: std::collections::HashSet<&str> =
                    artists.iter().map(|a| a.rating_key.as_str()).collect();
                let query_norm =
                    crate::services::artist_alias_service::normalize_artist_name(&query);
                state.library.artists.iter()
                    .filter(|a| !existing_keys.contains(a.rating_key.as_str()))
                    .filter(|a| {
                        state.library.artist_aliases.get(&a.rating_key)
                            .is_some_and(|aliases| aliases.iter().any(|al| {
                                let norm = crate::services::artist_alias_service::normalize_artist_name(al);
                                norm.contains(&query_norm)
                            }))
                    })
                    .cloned()
                    .collect()
            };
            artists.extend(alias_extras);

            let albums = search_albums_with_ranking(&state.library.albums, &query, 50);
            let playlists = search_with_ranking(&state.library.playlists, &query, |p| &p.title, 50);
            let genres = search_with_ranking(&state.library.album_genres, &query, |g| &g.title, 50);

            // Store local results immediately (tracks filled async by API)
            state.search.results = Some(SearchResults {
                artists,
                albums,
                playlists,
                genres,
                tracks: vec![],
            });

            // Fire async API search for tracks with debounce
            if state.sources.active.navidrome().is_some() {
                if let Some(results) = &mut state.search.results {
                    results.tracks =
                        search_with_ranking(&state.library.all_tracks, &query, |t| &t.title, 100);
                }
                return Ok(vec![]);
            }
        }
        SearchAction::SelectSearchResult => {
            if state.sources.active.folder().is_some() {
                let track = state
                    .search
                    .results
                    .as_ref()
                    .and_then(|r| r.tracks.get(state.list_state.search_item_index));
                if let Some(crate::library::track::TrackOrigin::Folder { path, .. }) =
                    track.map(|t| &t.origin)
                {
                    if let Some(column) = state
                        .folder_state
                        .as_mut()
                        .and_then(|nav| nav.focused_mut())
                    {
                        if let Some(index) = column.items.iter().position(|item| &item.key == path)
                        {
                            column.selected_index = index;
                        }
                    }
                    state.popups.search_active = false;
                    state.category_column_focused = false;
                    state.browse_category = BrowseCategory::Folders;
                    state.set_view(View::Browse);
                }
                return Ok(vec![]);
            }
            let follow_up_actions = select_search_result(state);
            follow_ups.extend(follow_up_actions);
        }
        SearchAction::SetSearchQuery(q) => {
            state.search.query = q;
            state.list_state.search_item_index = 0;
            // Re-run the search on every keystroke. The shared dispatch
            // path issues local + server track searches and updates
            // state.search.results when complete.
            follow_ups.push(Action::Search(SearchAction::ExecuteLocalSearch));
        }
        SearchAction::SetSearchTab(tab) => {
            state.search.tab = if state.sources.active.folder().is_some() {
                SearchTab::Tracks
            } else {
                tab
            };
            // Tab switch resets the visible-result cursor so the
            // first item in the newly-filtered list is highlighted.
            state.list_state.search_item_index = 0;
        }

        // Inline list filter actions
        SearchAction::ActivateListFilter => {
            state.list_filter.version = state.list_filter.version.wrapping_add(1);
            state.list_filter.active = true;
            state.list_filter.query.clear();
            state.list_filter.results = None;
            state.list_filter.column_results.clear();
            state.list_filter.loading = false;
            state.list_filter.selected = 0;
            // Capture which category and column the filter was activated on
            state.list_filter.category = state.browse_category;
            state.list_filter.column = match state.browse_category {
                BrowseCategory::Library => state.artist_nav.focused_column,
                BrowseCategory::Playlists => state.playlist_nav.focused_column,
                BrowseCategory::Folders => state
                    .folder_state
                    .as_ref()
                    .map(|fs| fs.focused_column)
                    .unwrap_or(0),
                cat if cat.is_tag_section() => state.tag_nav.focused_column,
                _ => 0,
            };
            // Activating the filter implies the user wants to browse
            // items, not stay parked on the synthetic Play row —
            // otherwise FilteredListUp/Down would shift the
            // underlying `selected_index` while the highlight stays
            // glued to the play row, looking broken.
            let filter_col_idx = state.list_filter.column;
            if let Some(nav) = state.browse_nav_mut() {
                if let Some(col) = nav.columns.get_mut(filter_col_idx) {
                    col.on_play_row = false;
                }
            }
            // Same reason as on_play_row above — any transient
            // "focus is somewhere other than the nav stack" flag
            // would force the scrolling Miller ribbon to anchor
            // on a fixed slot (sections / artists / pane), hiding
            // any column the user drills into from the filter.
            // Once the filter input has keyboard focus, none of
            // these flags should still be set.
            state.category_column_focused = false;
            state.alphabet_strip_focused = false;
            state.track_pane_focused = false;
        }
        SearchAction::DeactivateListFilter => {
            state.list_filter.deactivate();
        }
        SearchAction::FilteredListUp => {
            // Wrap-around at the top: pressing Up on the first match
            // jumps to the last match. Mirrors the search-dropdown
            // convention. Without this the user sees Up "do nothing"
            // when the column is already at `matched_indices[0]`,
            // which reads as broken.
            //
            // Also: always sync the underlying column's
            // `selected_index` to `matched_indices[selected]`, even
            // when the cursor was already there. If the column had
            // drifted (e.g. the user clicked an unfiltered row before
            // the filter narrowed the visible set), this re-anchors
            // the visible highlight on the filtered cursor.
            let len = state
                .list_filter
                .results
                .as_ref()
                .map(|r| r.matched_indices.len())
                .unwrap_or(0);
            if len == 0 {
                return Ok(follow_ups);
            }
            state.list_filter.selected = if state.list_filter.selected == 0 {
                len - 1
            } else {
                state.list_filter.selected - 1
            };
            if let Some(ref results) = state.list_filter.results {
                if let Some(&item_idx) = results.matched_indices.get(state.list_filter.selected) {
                    super::key_input::update_filter_column_selection(state, item_idx);
                }
            }
            super::key_input::truncate_filter_right_columns(state);
        }
        SearchAction::FilteredListDown => {
            // Wrap-around at the bottom: Down on the last match
            // jumps to the first.
            let len = state
                .list_filter
                .results
                .as_ref()
                .map(|r| r.matched_indices.len())
                .unwrap_or(0);
            if len == 0 {
                return Ok(follow_ups);
            }
            state.list_filter.selected = (state.list_filter.selected + 1) % len;
            if let Some(ref results) = state.list_filter.results {
                if let Some(&item_idx) = results.matched_indices.get(state.list_filter.selected) {
                    super::key_input::update_filter_column_selection(state, item_idx);
                }
            }
            super::key_input::truncate_filter_right_columns(state);
        }
        SearchAction::SelectFilteredItem => {
            // Pick the item to drill: prefer the current filter
            // match (results.matched_indices[selected]); fall back
            // to the column's current selected_index when results
            // haven't arrived yet (30 ms debounce on a fast typist
            // can race the keypress). Without the fallback Enter
            // is a silent no-op and the user thinks the bind is
            // broken.
            let target_idx = state
                .list_filter
                .results
                .as_ref()
                .and_then(|r| r.matched_indices.get(state.list_filter.selected).copied())
                .or_else(|| {
                    let col_idx = state.list_filter.column;
                    let cat = state.list_filter.category;
                    let nav: Option<&crate::app::state::BrowseNavigationState> = match cat {
                        BrowseCategory::Library => Some(&state.artist_nav),
                        BrowseCategory::Playlists => Some(&state.playlist_nav),
                        cat if cat.is_tag_section() => Some(&state.tag_nav),
                        _ => None,
                    };
                    nav.and_then(|n| n.columns.get(col_idx).map(|c| c.selected_index))
                });
            if let Some(item_idx) = target_idx {
                super::key_input::update_filter_column_selection(state, item_idx);
                // Anchor `focused_column` to the filter target
                // before dispatching the drill. Without this, the
                // drill helper reads `selected_item()` off
                // whatever column happened to be focused — which
                // can be a stale child column if the user drilled
                // past the filter column before activating the
                // filter, and the truncation step missed it.
                // Anchoring guarantees the drill targets the row
                // the user just highlighted in the filter results.
                let filter_col_idx = state.list_filter.column;
                let filter_cat = state.list_filter.category;
                match filter_cat {
                    BrowseCategory::Library => {
                        state.artist_nav.focused_column = filter_col_idx;
                        state.artist_nav.columns.truncate(filter_col_idx + 1);
                    }
                    BrowseCategory::Playlists => {
                        state.playlist_nav.focused_column = filter_col_idx;
                        state.playlist_nav.columns.truncate(filter_col_idx + 1);
                    }
                    cat if cat.is_tag_section() => {
                        state.tag_nav.focused_column = filter_col_idx;
                        state.tag_nav.columns.truncate(filter_col_idx + 1);
                    }
                    BrowseCategory::Folders => {
                        if let Some(ref mut fs) = state.folder_state {
                            fs.focused_column = filter_col_idx;
                            fs.columns.truncate(filter_col_idx + 1);
                        }
                    }
                    _ => {}
                }
                // Deactivate filter before drill-down (new column clears filter)
                state.alphabet_strip_focused = false;
                state.list_filter.deactivate();
                let drilldown_actions = super::key_input::get_filter_drilldown_actions(state);
                follow_ups.extend(drilldown_actions);
            }
        }
        SearchAction::AppendListFilterChar(c) => {
            state.list_filter.query.push(c);
            state.list_filter.selected = 0;
            if is_on_filter_column(state) {
                super::key_input::truncate_filter_right_columns(state);
            }
            schedule_list_filter(event_tx, state)?;
        }
        SearchAction::DeleteListFilterChar => {
            state.list_filter.query.pop();
            if state.list_filter.query.is_empty() {
                state.list_filter.deactivate();
            } else if is_on_filter_column(state) {
                state.list_filter.selected = 0;
                super::key_input::truncate_filter_right_columns(state);
                schedule_list_filter(event_tx, state)?;
            } else {
                state.list_filter.selected = 0;
                schedule_list_filter(event_tx, state)?;
            }
        }
        SearchAction::SetListFilterQuery(query) => {
            // Empty query deactivates the filter (same rule as the
            // char-at-a-time deletion path above).
            if query.is_empty() {
                state.list_filter.deactivate();
            } else {
                // Activating from scratch mirrors ActivateListFilter:
                // record category and column so rendering can match.
                if !state.list_filter.active {
                    state.list_filter.active = true;
                    state.list_filter.category = state.browse_category;
                    state.list_filter.column = match state.browse_category {
                        BrowseCategory::Library => state.artist_nav.focused_column,
                        BrowseCategory::Playlists => state.playlist_nav.focused_column,
                        BrowseCategory::Folders => state
                            .folder_state
                            .as_ref()
                            .map(|fs| fs.focused_column)
                            .unwrap_or(0),
                        cat if cat.is_tag_section() => state.tag_nav.focused_column,
                        _ => 0,
                    };
                }
                state.list_filter.query = query;
                state.list_filter.selected = 0;
                // No truncation: the GUI filter is a pure visual narrowing
                // applied to every visible Miller column at once
                // (browse.rs computes per-column matches at render time),
                // so the right-of-filter columns must stay populated.
                // Truncating here was what made the album column vanish
                // when the user typed in the filter box with the artist
                // column focused.
                schedule_list_filter(event_tx, state)?;
            }
        }
        SearchAction::RunListFilter { version } => {
            run_list_filter(event_tx, state, version);
        }
        // Search popup actions
        SearchAction::OpenSearchPopup => {
            if state.list_filter.active {
                state.list_filter.deactivate();
            }
            state.popups.close_all();
            state.popups.search_active = true;
            state.search.focus = SearchFocus::Input;
            state.search.query.clear();
            state.search.results = None;
            state.list_state.search_item_index = 0;
            if state.sources.active.folder().is_some() {
                state.search.tab = SearchTab::Tracks;
            }
        }
        SearchAction::CloseSearchPopup => {
            state.popups.search_active = false;
        }

        // Library picker popup actions
        action @ (SearchAction::OpenLibraryPicker | SearchAction::ManageLibraries) => {
            state.popups.close_all();
            if matches!(action, SearchAction::ManageLibraries) {
                state.set_view(View::Settings);
                state.settings_state.section = crate::app::state::SettingsSection::Libraries;
                state.settings_state.focus = crate::app::state::SettingsFocus::Content;
            } else {
                state.popups.library_picker_active = true;
            }
            crate::app::sources::manager::reset_selection(state);
            state.sources.picker_scroll_pin = None;
            state.popups.library_picker_index = crate::app::sources::choices(state)
                .iter()
                .position(|entry| entry.active(state))
                .unwrap_or(0);
        }
        SearchAction::CloseLibraryPicker => {
            state.popups.library_picker_active = false;
        }

        // Sort popup actions
        SearchAction::OpenSortPopup => {
            use crate::app::state::{BrowseItem, SortColumnType, SortPopupState};

            if state.view != View::Browse {
                // no-op outside browse view
            } else if let Some(nav) = state.browse_nav() {
                let col_idx = nav.focused_column;
                if let Some(col) = nav.columns.get(col_idx) {
                    // Determine column type from content
                    let first_item = col.items.first();
                    let column_type = if first_item
                        .is_some_and(|i| matches!(i, BrowseItem::Artist { .. }))
                        || col
                            .items
                            .iter()
                            .take(3)
                            .any(|i| matches!(i, BrowseItem::Artist { .. }))
                    {
                        SortColumnType::Artist
                    } else if first_item.is_some_and(|i| matches!(i, BrowseItem::Album { .. }))
                        || col
                            .items
                            .iter()
                            .take(4)
                            .any(|i| matches!(i, BrowseItem::Album { .. }))
                    {
                        SortColumnType::Album
                    } else if first_item.is_some_and(|i| matches!(i, BrowseItem::Track { .. })) {
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

                    let is_playlist =
                        state.browse_category == crate::app::state::BrowseCategory::Playlists;
                    let popup = SortPopupState::new(
                        col_idx,
                        col.title.clone(),
                        column_type,
                        col.sort_mode,
                        col.artwork_visible,
                        is_playlist,
                    );
                    state.popups.close_all();
                    state.popups.sort = Some(popup);
                }
            }
        }
        SearchAction::CloseSortPopup => {
            state.popups.sort = None;
        }
        SearchAction::ApplyFocusedSortMode(mode) => {
            let col_idx = match state.browse_nav() {
                Some(n) => n.focused_column,
                None => return Ok(vec![]),
            };
            follow_ups.extend(super::key_input::sort_popup::apply_sort_mode(
                state, col_idx, mode,
            ));
        }
        SearchAction::ReverseFocusedSortDirection => {
            let col_idx = match state.browse_nav() {
                Some(n) => n.focused_column,
                None => return Ok(vec![]),
            };
            follow_ups.extend(super::key_input::sort_popup::toggle_sort_direction(
                state, col_idx,
            ));
        }
        SearchAction::ToggleFocusedColumnArtwork => {
            let col_idx = match state.browse_nav() {
                Some(n) => n.focused_column,
                None => return Ok(vec![]),
            };
            super::key_input::sort_popup::toggle_artwork(state, col_idx);
        }
        SearchAction::ToggleFocusedColumnGrouping => {
            let col_idx = match state.browse_nav() {
                Some(n) => n.focused_column,
                None => return Ok(vec![]),
            };
            // Toggling grouping reshapes the column (track list ↔
            // album list), so any open track-details pane is now
            // pointing at a row that may no longer exist. Close it.
            state.track_pane_open = false;
            follow_ups.extend(super::key_input::sort_popup::toggle_group_by_album(
                state, col_idx,
            ));
        }
        // Adventure launcher popup actions
        SearchAction::OpenAdventureLauncher => {
            state.adventure_launcher_request_id =
                state.adventure_launcher_request_id.wrapping_add(1);
            state.popups.close_all();
            state.popups.adventure_launcher = Some(crate::app::state::AdventureLauncherState {
                step: crate::app::state::AdventureStep::FindStartTrack,
                query: String::new(),
                results: None,
                focus: SearchFocus::Input,
                item_index: 0,
                loading: false,
                drill: crate::app::state::AdventureDrillLevel::Search,
                start_track: None,
                end_track: None,
                track_count_input: "20".to_string(),
                scroll_pin: None,
                // Sonic Adventure only ever picks tracks (start /
                // end), so force the tracks tab — that path makes
                // `adventure_launcher_select_track` index directly
                // into `results.tracks` without an artist+album
                // offset.
                search_tab: crate::app::state::SearchTab::Tracks,
            });
        }
        SearchAction::OpenAdventureLauncherWithStart { start_track } => {
            // Pre-select the start track (e.g. from a right-click on a
            // specific track row) and skip past FindStartTrack — the
            // user already chose; only the count + end track remain.
            state.adventure_launcher_request_id =
                state.adventure_launcher_request_id.wrapping_add(1);
            state.popups.close_all();
            state.popups.adventure_launcher = Some(crate::app::state::AdventureLauncherState {
                step: crate::app::state::AdventureStep::EnterTrackCount,
                query: String::new(),
                results: None,
                focus: SearchFocus::Input,
                item_index: 0,
                loading: false,
                drill: crate::app::state::AdventureDrillLevel::Search,
                start_track: Some(*start_track),
                end_track: None,
                track_count_input: "20".to_string(),
                scroll_pin: None,
                // Sonic Adventure only ever picks tracks (start /
                // end), so force the tracks tab — that path makes
                // `adventure_launcher_select_track` index directly
                // into `results.tracks` without an artist+album
                // offset.
                search_tab: crate::app::state::SearchTab::Tracks,
            });
        }
        SearchAction::CloseAdventureLauncher => {
            state.adventure_launcher_request_id =
                state.adventure_launcher_request_id.wrapping_add(1);
            state.popups.adventure_launcher = None;
        }
        SearchAction::AdventureLauncherSearch => {
            adventure_launcher_search(event_tx, state)?;
        }

        SearchAction::AdventureLauncherSelectTrack => {
            follow_ups = adventure_launcher_select_track(event_tx, state)?;
        }
        SearchAction::AdventureLauncherBack => {
            adventure_launcher_back(state);
        }
        SearchAction::AdventureLauncherSetStep(step) => {
            state.adventure_launcher_request_id =
                state.adventure_launcher_request_id.wrapping_add(1);
            if let Some(l) = state.popups.adventure_launcher.as_mut() {
                l.step = step;
                // Reset the search panel so a fresh query targets the
                // newly-active field.
                l.query.clear();
                l.results = None;
                l.drill = crate::app::state::AdventureDrillLevel::Search;
                l.item_index = 0;
                l.focus = SearchFocus::Input;
            }
        }
        SearchAction::AdventureLauncherReverse => {
            if let Some(l) = state.popups.adventure_launcher.as_mut() {
                std::mem::swap(&mut l.start_track, &mut l.end_track);
            }
        }
        SearchAction::AdventureLauncherSetTrackCount(s) => {
            if let Some(l) = state.popups.adventure_launcher.as_mut() {
                // Keep only digits, max 3 characters (so users can't
                // accidentally type a multi-thousand-track adventure).
                l.track_count_input = s.chars().filter(|c| c.is_ascii_digit()).take(3).collect();
            }
        }
        SearchAction::AdventureLauncherClearStart => {
            state.adventure_launcher_request_id =
                state.adventure_launcher_request_id.wrapping_add(1);
            if let Some(l) = state.popups.adventure_launcher.as_mut() {
                l.start_track = None;
                l.step = crate::app::state::AdventureStep::FindStartTrack;
            }
        }
        SearchAction::AdventureLauncherClearEnd => {
            state.adventure_launcher_request_id =
                state.adventure_launcher_request_id.wrapping_add(1);
            if let Some(l) = state.popups.adventure_launcher.as_mut() {
                l.end_track = None;
                l.step = crate::app::state::AdventureStep::FindEndTrack;
            }
        }

        SearchAction::AdventureLauncherSetQuery(q) => {
            if let Some(l) = state.popups.adventure_launcher.as_mut() {
                l.query = q;
            }
            // Trigger the search with the new query.
            adventure_launcher_search(event_tx, state)?;
        }

        // Multi-artist radio picker actions
        SearchAction::OpenArtistRadioPicker => {
            state.popups.close_all();
            state.popups.artist_radio_picker = Some(crate::app::state::ArtistRadioPickerState {
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
        SearchAction::CloseArtistRadioPicker => {
            state.popups.artist_radio_picker = None;
        }
        SearchAction::ArtistRadioPickerSetCount => {
            if let Some(ref mut picker) = state.popups.artist_radio_picker {
                let count = picker
                    .count_input
                    .parse::<usize>()
                    .unwrap_or(0)
                    .clamp(1, 12);
                picker.max_artists = count;
                picker.step = crate::app::state::ArtistRadioPickerStep::SelectArtists;
                picker.query.clear();
                picker.filtered_artists = state.library.artists.clone();
                picker.selected_artists.clear();
                picker.focus = SearchFocus::Input;
                picker.item_index = 0;
            }
        }
        SearchAction::ArtistRadioPickerSearch => {
            if let Some(ref mut picker) = state.popups.artist_radio_picker {
                if picker.query.is_empty() {
                    picker.filtered_artists = state.library.artists.clone();
                } else {
                    let (query, query_normalized) = normalize_query(&picker.query);

                    picker.filtered_artists = state
                        .library
                        .artists
                        .iter()
                        .filter(|a| {
                            if fuzzy_matches(&a.title, &query, &query_normalized) {
                                return true;
                            }
                            // Also match artist aliases
                            if let Some(aliases) = state.library.artist_aliases.get(&a.rating_key) {
                                aliases
                                    .iter()
                                    .any(|alias| fuzzy_matches(alias, &query, &query_normalized))
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
        SearchAction::ArtistRadioPickerToggleArtist => {
            if let Some(ref mut picker) = state.popups.artist_radio_picker {
                if let Some(artist) = picker.filtered_artists.get(picker.item_index).cloned() {
                    // Toggle: remove if already selected, add if not
                    if let Some(pos) = picker
                        .selected_artists
                        .iter()
                        .position(|a| a.rating_key == artist.rating_key)
                    {
                        picker.selected_artists.remove(pos);
                    } else if picker.selected_artists.len() < picker.max_artists {
                        let added_key = artist.rating_key.clone();
                        picker.selected_artists.push(artist);

                        // Auto-launch if max artists reached
                        if picker.selected_artists.len() == picker.max_artists {
                            follow_ups.push(SearchAction::ArtistRadioPickerLaunch.into());
                        } else {
                            // Clear query and re-populate with all artists, position near selected
                            picker.query.clear();
                            picker.filtered_artists = state.library.artists.clone();
                            picker.focus = SearchFocus::Input;
                            // Position item_index at the just-added artist in the full list
                            picker.item_index = picker
                                .filtered_artists
                                .iter()
                                .position(|a| a.rating_key == added_key)
                                .unwrap_or(0);
                            picker.scroll_pin = None;
                        }
                    }
                }
            }
        }
        // Artist bio popup (F4)
        action @ (SearchAction::OpenBiographySource | SearchAction::SearchBiographyOnGoogle) => {
            let url = state
                .popups
                .artist_bio
                .as_ref()
                .and_then(|popup| match action {
                    SearchAction::SearchBiographyOnGoogle
                        if !popup.artist_name.trim().is_empty() =>
                    {
                        Some(crate::services::biography::google_search_url(
                            &popup.artist_name,
                        ))
                    }
                    SearchAction::OpenBiographySource => popup.document.source_url.clone(),
                    _ => None,
                });
            if let Some(url) = url {
                let tx = event_tx.clone();
                crate::app::tasks::spawn(async move {
                    if let Err(error) = crate::services::external_search::open_browser(&url).await {
                        let _ = tx
                            .send(Event::Effect(
                                crate::app::action::SystemAction::ShowError(format!(
                                    "Open biography in browser: {error:#}"
                                ))
                                .into(),
                            ))
                            .await;
                    }
                });
            }
        }
        SearchAction::ShowArtistBio {
            artist_key,
            artist_name,
        } => {
            super::helpers::biography::show(state, event_tx, artist_key, artist_name);
        }

        _ => anyhow::bail!("Unsupported library operation reached shared handler"),
    }
    Ok(follow_ups)
}

/// Handle SelectSearchResult — navigate to the selected item in the library.
fn select_search_result(state: &mut AppState) -> Vec<Action> {
    let results = match state.search.results.take() {
        Some(r) => r,
        None => return vec![],
    };
    let idx = state.list_state.search_item_index;

    // For the All tab, map global index to (section, local_index)
    let (section, local_idx) = if state.search.tab == SearchTab::Global {
        resolve_global_index(&results, idx)
    } else {
        (state.search.tab, idx)
    };

    match section {
        SearchTab::Artists => {
            if let Some(artist) = results.artists.get(local_idx) {
                // Navigate to artist in Library view
                let artist_key = artist.rating_key.clone();
                state.search.query.clear();

                state.popups.search_active = false;
                state.set_browse_category(BrowseCategory::Library, false);
                state.set_view(View::Browse);

                // Find artist in artist_nav and select it
                if let Some(col) = state.artist_nav.columns.get_mut(0) {
                    if let Some(pos) = col.items.iter().position(|i| i.key() == artist_key) {
                        col.selected_index = pos;
                        state.artist_nav.focused_column = 0;
                        state.artist_nav.truncate_right();
                        return vec![
                            crate::app::action::MillerAction::LoadArtistAlbumsForMiller {
                                artist_key,
                                replace_child: false,
                            }
                            .into(),
                        ];
                    }
                }
                // Artist not in nav (cache empty?) — load from scratch
                return vec![
                    crate::app::action::MillerAction::LoadArtistAlbumsForMiller {
                        artist_key,
                        replace_child: false,
                    }
                    .into(),
                ];
            }
        }
        SearchTab::Albums => {
            if let Some(album) = results.albums.get(local_idx) {
                let album_key = album.rating_key.clone();
                let artist_key = album.parent_rating_key.clone();
                state.search.query.clear();

                state.popups.search_active = false;
                state.set_browse_category(BrowseCategory::Library, false);
                state.set_view(View::Browse);
                state.search.pending_album_key = Some(album_key);

                // If we have the parent artist key, navigate to them
                if let Some(ref ak) = artist_key {
                    state.library.selected_artist_name = album.artist_name().to_string();
                    if let Some(col) = state.artist_nav.columns.get_mut(0) {
                        if let Some(pos) = col.items.iter().position(|i| i.key() == ak.as_str()) {
                            col.selected_index = pos;
                            state.artist_nav.focused_column = 0;
                            state.artist_nav.truncate_right();
                            return vec![
                                crate::app::action::MillerAction::LoadArtistAlbumsForMiller {
                                    artist_key: ak.clone(),
                                    replace_child: false,
                                }
                                .into(),
                            ];
                        }
                    }
                    return vec![
                        crate::app::action::MillerAction::LoadArtistAlbumsForMiller {
                            artist_key: ak.clone(),
                            replace_child: false,
                        }
                        .into(),
                    ];
                }
                // No parent artist key — try All Artists column
                state.library.selected_artist_name = "All Artists".to_string();
                if let Some(col) = state.artist_nav.columns.get_mut(0) {
                    // Select "All Artists" entry (index 0)
                    col.selected_index = 0;
                    state.artist_nav.focused_column = 0;
                    state.artist_nav.truncate_right();
                }
                return vec![crate::app::action::MillerAction::LoadAllAlbumsForMiller {
                    replace_child: false,
                }
                .into()];
            }
        }
        SearchTab::Tracks => {
            if let Some(track) = results.tracks.get(local_idx) {
                // Navigate to the track in Library (artist → album).
                // Mirrors the right-click "Open in Library" action so
                // Miller columns end up in the same state the user
                // would have built by drilling through manually.
                let artist_key = track.grandparent_rating_key.clone();
                let album_key = track.parent_rating_key.clone();
                let artist_name = track.artist_name().to_string();
                let album_title = track.album_name().to_string();
                state.search.query.clear();
                state.popups.search_active = false;
                if let Some(akey) = artist_key {
                    return vec![crate::app::action::BrowseAction::OpenInLibrary {
                        artist_key: akey,
                        artist_name,
                        album_key,
                        album_title: Some(album_title),
                    }
                    .into()];
                }
                // Track has no artist key — fall back to playing it.
                return vec![
                    crate::app::action::QueueAction::PlayTrack(Box::new(track.clone())).into(),
                ];
            }
        }
        SearchTab::Playlists => {
            if let Some(playlist) = results.playlists.get(local_idx) {
                let playlist_key = playlist.rating_key.clone();
                state.search.query.clear();

                state.popups.search_active = false;
                state.set_browse_category(BrowseCategory::Playlists, false);
                state.set_view(View::Browse);

                // Find playlist in playlist_nav and select it
                if let Some(col) = state.playlist_nav.columns.get_mut(0) {
                    if let Some(pos) = col.items.iter().position(|i| i.key() == playlist_key) {
                        col.selected_index = pos;
                        state.playlist_nav.focused_column = 0;
                        state.playlist_nav.truncate_right();
                        return vec![
                            crate::app::action::MillerAction::LoadPlaylistTracksForMiller {
                                playlist_key,
                                replace_child: false,
                            }
                            .into(),
                        ];
                    }
                }
                return vec![
                    crate::app::action::MillerAction::LoadPlaylistTracksForMiller {
                        playlist_key,
                        replace_child: false,
                    }
                    .into(),
                ];
            }
        }
        SearchTab::Genres => {
            if let Some(genre) = results.genres.get(local_idx) {
                let genre_key = genre.effective_key().to_string();
                let genre_title = genre.title.clone();
                state.search.query.clear();

                state.popups.search_active = false;
                // Search results' genre list comes from album_genres, so
                // open the Album Genres section.
                state.set_browse_category(BrowseCategory::AlbumGenres, false);
                state.set_view(View::Browse);

                // Populate column 0 with the album-genre list, with the
                // searched genre selected. Column 1 (albums) is pushed by
                // LoadGenreAlbumsForMiller below.
                let genre_items = BrowseItem::from_genres(&state.library.album_genres);
                let mut col0 = crate::app::state::BrowseColumn::new(
                    BrowseCategory::AlbumGenres.name(),
                    genre_items,
                );
                if let Some(pos) = col0.items.iter().position(|i| i.key() == genre_key) {
                    col0.selected_index = pos;
                }
                state.tag_nav.columns.clear();
                state.tag_nav.columns.push(col0);
                state.tag_nav.focused_column = 0;
                state.library.selected_album_title = format!("genre: {}", genre_title);

                return vec![crate::app::action::MillerAction::LoadGenreAlbumsForMiller {
                    genre_key,
                    replace_child: false,
                }
                .into()];
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

/// Schedule inline list filtering without cloning the source column on every
/// keystroke. Only the debounce generation that survives snapshots the data.
fn schedule_list_filter(event_tx: &mpsc::Sender<Event>, state: &mut AppState) -> Result<()> {
    state.list_filter.version = state.list_filter.version.wrapping_add(1);
    let version = state.list_filter.version;

    if state.list_filter.query.is_empty() {
        state.list_filter.results = None;
        state.list_filter.column_results.clear();
        state.list_filter.loading = false;
        return Ok(());
    }

    state.list_filter.loading = true;
    state.list_filter.results = None;
    state.list_filter.column_results.clear();

    let tx = event_tx.clone();
    crate::app::tasks::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        let _ = tx
            .send(Event::Effect(
                SearchAction::RunListFilter { version }.into(),
            ))
            .await;
    });

    Ok(())
}

/// Snapshot and filter the winning debounce generation on a blocking worker.
fn run_list_filter(event_tx: &mpsc::Sender<Event>, state: &mut AppState, version: u64) {
    use crate::services::{
        browse_filter_records, filter_browse_records, filter_with_priority, DEFAULT_MAX_RESULTS,
    };

    if version != state.list_filter.version
        || !state.list_filter.active
        || state.list_filter.query.is_empty()
    {
        return;
    }

    let query = state.list_filter.query.clone();
    let tx = event_tx.clone();
    match state.list_filter.category {
        BrowseCategory::Library => {
            let columns: Vec<_> = state
                .artist_nav
                .columns
                .iter()
                .map(|column| browse_filter_records(&column.items))
                .collect();
            let aliases = state.library.artist_aliases.clone();
            let comp_keys = state.library.compilations.artist_keys.clone();
            crate::app::tasks::spawn_blocking(move || {
                let column_results = columns
                    .iter()
                    .map(|items| {
                        filter_browse_records(
                            items,
                            &query,
                            DEFAULT_MAX_RESULTS,
                            &aliases,
                            &comp_keys,
                        )
                    })
                    .collect();
                let _ = tx.blocking_send(
                    UiEvent::ListFilterCompleted {
                        version,
                        column_results,
                    }
                    .into(),
                );
            });
        }
        BrowseCategory::Playlists => {
            let columns: Vec<_> = state
                .playlist_nav
                .columns
                .iter()
                .map(|column| browse_filter_records(&column.items))
                .collect();
            crate::app::tasks::spawn_blocking(move || {
                let empty_aliases = std::collections::HashMap::new();
                let empty_keys = std::collections::HashSet::new();
                let column_results = columns
                    .iter()
                    .map(|items| {
                        filter_browse_records(
                            items,
                            &query,
                            DEFAULT_MAX_RESULTS,
                            &empty_aliases,
                            &empty_keys,
                        )
                    })
                    .collect();
                let _ = tx.blocking_send(
                    UiEvent::ListFilterCompleted {
                        version,
                        column_results,
                    }
                    .into(),
                );
            });
        }
        cat if cat.is_tag_section() => {
            let columns: Vec<_> = state
                .tag_nav
                .columns
                .iter()
                .map(|column| browse_filter_records(&column.items))
                .collect();
            crate::app::tasks::spawn_blocking(move || {
                let empty_aliases = std::collections::HashMap::new();
                let empty_keys = std::collections::HashSet::new();
                let column_results = columns
                    .iter()
                    .map(|items| {
                        filter_browse_records(
                            items,
                            &query,
                            DEFAULT_MAX_RESULTS,
                            &empty_aliases,
                            &empty_keys,
                        )
                    })
                    .collect();
                let _ = tx.blocking_send(
                    UiEvent::ListFilterCompleted {
                        version,
                        column_results,
                    }
                    .into(),
                );
            });
        }
        BrowseCategory::Folders => {
            let columns: Vec<Vec<String>> = state
                .folder_state
                .as_ref()
                .map(|folder_state| {
                    folder_state
                        .columns
                        .iter()
                        .map(|column| column.items.iter().map(|item| item.title.clone()).collect())
                        .collect()
                })
                .unwrap_or_default();
            crate::app::tasks::spawn_blocking(move || {
                let column_results = columns
                    .iter()
                    .map(|items| {
                        filter_with_priority(
                            items,
                            &query,
                            |title| title.as_str(),
                            DEFAULT_MAX_RESULTS,
                        )
                    })
                    .collect();
                let _ = tx.blocking_send(
                    UiEvent::ListFilterCompleted {
                        version,
                        column_results,
                    }
                    .into(),
                );
            });
        }
        _ => {
            state.list_filter.loading = false;
        }
    }
}

/// Adventure launcher: search the cached catalog.
fn adventure_launcher_search(_event_tx: &mpsc::Sender<Event>, state: &mut AppState) -> Result<()> {
    state.adventure_launcher_request_id = state.adventure_launcher_request_id.wrapping_add(1);
    let launcher = match state.popups.adventure_launcher.as_mut() {
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
    use crate::services::{search_albums_with_ranking, search_with_ranking};
    let query = launcher.query.to_lowercase();
    let mut artists = search_with_ranking(
        &state.library.artists,
        &query,
        |a| &a.title,
        SEARCH_RESULT_LIMIT,
    );

    // Also find artists whose aliases match the query (using normalized matching)
    let alias_extras: Vec<_> = {
        let existing_keys: std::collections::HashSet<&str> =
            artists.iter().map(|a| a.rating_key.as_str()).collect();
        let query_norm = crate::services::artist_alias_service::normalize_artist_name(&query);
        state
            .library
            .artists
            .iter()
            .filter(|a| !existing_keys.contains(a.rating_key.as_str()))
            .filter(|a| {
                state
                    .library
                    .artist_aliases
                    .get(&a.rating_key)
                    .is_some_and(|aliases| {
                        aliases.iter().any(|al| {
                            let norm =
                                crate::services::artist_alias_service::normalize_artist_name(al);
                            norm.contains(&query_norm)
                        })
                    })
            })
            .cloned()
            .collect()
    };
    artists.extend(alias_extras);

    let albums = search_albums_with_ranking(&state.library.albums, &query, SEARCH_RESULT_LIMIT);
    let playlists = search_with_ranking(
        &state.library.playlists,
        &query,
        |p| &p.title,
        SEARCH_RESULT_LIMIT,
    );
    let genres = search_with_ranking(
        &state.library.album_genres,
        &query,
        |g| &g.title,
        SEARCH_RESULT_LIMIT,
    );

    let navidrome = state.sources.active.navidrome().is_some();
    let tracks = if navidrome {
        search_with_ranking(
            &state.library.all_tracks,
            &query,
            |t| &t.title,
            SEARCH_RESULT_LIMIT,
        )
    } else {
        vec![]
    };
    launcher.results = Some(SearchResults {
        artists,
        albums,
        playlists,
        genres,
        tracks,
    });
    launcher.loading = false;
    launcher.item_index = 0;
    launcher.focus = SearchFocus::Input;

    Ok(())
}

/// Adventure launcher: select track from current drill level.
fn adventure_launcher_select_track(
    _event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,
) -> Result<Vec<Action>> {
    use crate::app::state::{AdventureDrillLevel, AdventureStep};

    state.adventure_launcher_request_id = state.adventure_launcher_request_id.wrapping_add(1);
    let launcher = match state.popups.adventure_launcher.as_mut() {
        Some(l) => l,
        None => return Ok(vec![]),
    };

    // Extract the selected track based on drill level
    let track = match &launcher.drill {
        AdventureDrillLevel::AlbumTracks { tracks, .. } => tracks.get(launcher.item_index).cloned(),
        AdventureDrillLevel::Search => {
            // Tab-aware track selection
            if let Some(ref results) = launcher.results {
                let idx = launcher.item_index;
                match launcher.search_tab {
                    crate::app::state::SearchTab::Tracks => results.tracks.get(idx).cloned(),
                    crate::app::state::SearchTab::Global => {
                        let artist_count = results.artists.len();
                        let album_count = results.albums.len();
                        if idx >= artist_count + album_count {
                            results
                                .tracks
                                .get(idx - artist_count - album_count)
                                .cloned()
                        } else {
                            None
                        }
                    }
                    _ => None,
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
        }
        AdventureStep::FindEndTrack => {
            launcher.end_track = Some(track);
        }
        AdventureStep::EnterTrackCount => {
            // Shouldn't happen (count step doesn't select tracks),
            // but defensively store as start so a stray click isn't lost.
            launcher.start_track = Some(track);
        }
    }
    // Reset the search panel so the user can pick the other field
    // immediately without manually clearing it.
    launcher.query.clear();
    launcher.results = None;
    launcher.drill = AdventureDrillLevel::Search;
    launcher.item_index = 0;
    launcher.focus = SearchFocus::Input;

    // Auto-advance the launcher's step so the UI keeps moving after a
    // selection instead of leaving the user on a now-empty search:
    //   - Picked a start track → step into EnterTrackCount.
    //   - Picked an end track when count is set → fire Generate.
    //   - Picked an end track when count is empty → step into
    //     EnterTrackCount so the user fills in a count.
    let both_tracks_set = launcher.start_track.is_some() && launcher.end_track.is_some();
    let count_set = !launcher.track_count_input.trim().is_empty();
    if both_tracks_set && count_set {
        return Ok(vec![SearchAction::AdventureLauncherGenerate.into()]);
    }
    if launcher.start_track.is_some() {
        launcher.step = AdventureStep::EnterTrackCount;
    }

    Ok(vec![])
}

/// Adventure launcher: handle back navigation.
fn adventure_launcher_back(state: &mut AppState) {
    use crate::app::state::{AdventureDrillLevel, AdventureStep};

    state.adventure_launcher_request_id = state.adventure_launcher_request_id.wrapping_add(1);
    let launcher = match state.popups.adventure_launcher.as_mut() {
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
                    state.popups.adventure_launcher = None;
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
                    let artists = state.library.artists.clone();
                    if let Some(ref mut l) = state.popups.adventure_launcher {
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
        BrowseCategory::Folders => state
            .folder_state
            .as_ref()
            .map(|fs| fs.focused_column == filter_col)
            .unwrap_or(true),
        cat if cat.is_tag_section() => state.tag_nav.focused_column == filter_col,
        _ => false,
    }
}
