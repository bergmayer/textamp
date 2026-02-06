//! Search dispatch handlers: ExecuteSearch, ClearSearch, ExecuteFilterSearch,
//! SelectFilterResult, ActivateListFilter, DeactivateListFilter, FilteredList*,
//! SelectFilteredItem, AppendListFilterChar, DeleteListFilterChar, ClearListFilter,
//! ExecuteListFilter, OpenSearchPopup, CloseSearchPopup, OpenLibraryPicker,
//! CloseLibraryPicker.

use crate::app::{Action, AppState, Event};
use crate::app::state::{BrowseCategory, GenreContentType};
use crate::api::PlexClient;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Dispatch search and filter actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: Action,
    state: &mut AppState,
    client: &mut PlexClient,
) -> Result<Vec<Action>> {
    let mut follow_ups = vec![];

    match action {
        Action::ExecuteSearch => {
            if state.search_query.len() >= 2 {
                // Increment version to invalidate any pending searches
                state.global_search_version = state.global_search_version.wrapping_add(1);
                let version = state.global_search_version;
                state.search_loading = true;

                // Spawn search in background with debounce
                let event_tx = event_tx.clone();
                let query = state.search_query.clone();
                let search_client = client.clone();

                tokio::spawn(async move {
                    // Debounce: wait before searching
                    tokio::time::sleep(std::time::Duration::from_millis(350)).await;

                    // Execute search - stale results will be rejected by version check
                    match search_client.search(&query).await {
                        Ok(results) => {
                            let _ = event_tx.send(Event::GlobalSearchCompleted {
                                version,
                                results,
                            }).await;
                        }
                        Err(_) => {
                            let _ = event_tx.send(Event::GlobalSearchCompleted {
                                version,
                                results: Default::default(),
                            }).await;
                        }
                    }
                });
            } else {
                // Clear results for short queries
                state.search_results = None;
                state.search_loading = false;
            }
        }
        Action::ClearSearch => {
            state.search_query.clear();
            state.search_results = None;
        }
        Action::ExecuteFilterSearch => {
            if state.search_query.len() >= 2 {
                // Increment version to invalidate any pending searches
                state.filter_search_version = state.filter_search_version.wrapping_add(1);
                let version = state.filter_search_version;
                state.filter_loading = true;

                // Spawn search in background with debounce
                let event_tx = event_tx.clone();
                let query = state.search_query.clone();
                let search_client = client.clone();

                tokio::spawn(async move {
                    // Debounce: wait before searching
                    tokio::time::sleep(std::time::Duration::from_millis(350)).await;

                    // Execute search - stale results will be rejected by version check
                    match search_client.search(&query).await {
                        Ok(results) => {
                            let _ = event_tx.send(Event::FilterSearchCompleted {
                                version,
                                results
                            }).await;
                        }
                        Err(_) => {
                            let _ = event_tx.send(Event::FilterSearchCompleted {
                                version,
                                results: Default::default()
                            }).await;
                        }
                    }
                });
            } else {
                // Clear filter results for short queries (use local filtering)
                state.filter_results = None;
                state.filter_loading = false;
            }
        }
        Action::SelectFilterResult => {
            let follow_up_actions = helpers::select_filter_result(state);
            follow_ups.extend(follow_up_actions);
        }

        // Inline list filter actions
        Action::ActivateListFilter => {
            state.list_filter_active = true;
            state.list_filter_query.clear();
            state.list_filter_results = None;
            state.list_filter_loading = false;
            state.list_filter_selected = 0;
            // Capture which category and column the filter was activated on
            state.list_filter_category = state.browse_category;
            state.list_filter_column = match state.browse_category {
                BrowseCategory::Artists => state.artist_nav.focused_column,
                BrowseCategory::Playlists => state.playlist_nav.focused_column,
                BrowseCategory::Genres => {
                    if state.genre_content_type == GenreContentType::Stations {
                        state.station_nav.focused_column
                    } else {
                        state.genre_nav.focused_column
                    }
                }
                BrowseCategory::Folders => {
                    state.folder_state.as_ref().map(|fs| fs.focused_column).unwrap_or(0)
                }
            };
        }
        Action::DeactivateListFilter => {
            state.list_filter_active = false;
            state.list_filter_query.clear();
            state.list_filter_results = None;
            state.list_filter_loading = false;
            state.list_filter_selected = 0;
        }
        Action::FilteredListUp => {
            // Navigate up within filtered results and update the target column's selection
            if state.list_filter_selected > 0 {
                state.list_filter_selected -= 1;
                // Update the column's selected_index to match
                if let Some(ref results) = state.list_filter_results {
                    if let Some(&item_idx) = results.matched_indices.get(state.list_filter_selected) {
                        super::key_input::update_filter_column_selection(state, item_idx);
                    }
                }
            }
        }
        Action::FilteredListDown => {
            // Navigate down within filtered results and update the target column's selection
            if let Some(ref results) = state.list_filter_results {
                if state.list_filter_selected + 1 < results.matched_indices.len() {
                    state.list_filter_selected += 1;
                    if let Some(&item_idx) = results.matched_indices.get(state.list_filter_selected) {
                        super::key_input::update_filter_column_selection(state, item_idx);
                    }
                }
            }
        }
        Action::SelectFilteredItem => {
            // Select the currently highlighted filtered item and drill down
            // Filter stays active and continues to apply to the original column
            if let Some(ref results) = state.list_filter_results.clone() {
                if let Some(&item_idx) = results.matched_indices.get(state.list_filter_selected) {
                    // Update the column's selected_index to point to this item
                    super::key_input::update_filter_column_selection(state, item_idx);

                    // Get and dispatch drill-down actions (filter stays active)
                    let drilldown_actions = super::key_input::get_filter_drilldown_actions(state);
                    follow_ups.extend(drilldown_actions);
                }
            }
        }
        Action::AppendListFilterChar(c) => {
            state.list_filter_query.push(c);
            // Trigger filter execution
            execute_list_filter(event_tx, state).await?;
        }
        Action::DeleteListFilterChar => {
            state.list_filter_query.pop();
            if state.list_filter_query.is_empty() {
                state.list_filter_results = None;
                state.list_filter_loading = false;
            } else {
                execute_list_filter(event_tx, state).await?;
            }
        }
        Action::ClearListFilter => {
            state.list_filter_query.clear();
            state.list_filter_results = None;
            state.list_filter_loading = false;
        }
        Action::ExecuteListFilter => {
            execute_list_filter(event_tx, state).await?;
        }

        // Search popup actions
        Action::OpenSearchPopup => {
            state.search_popup_active = true;
            // Clear previous search when opening
            state.search_query.clear();
            state.search_results = None;
            state.filter_results = None;
        }
        Action::CloseSearchPopup => {
            state.search_popup_active = false;
        }

        // Library picker popup actions
        Action::OpenLibraryPicker => {
            state.library_picker_active = true;
            // Set index to current active library
            if let Some(ref active_key) = state.active_library {
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
        _ => unreachable!("dispatch_search called with non-search action: {:?}", action),
    }
    Ok(follow_ups)
}

/// Execute inline list filter with debounce.
/// Collects filterable items from the filter's target column (captured when filter was activated).
async fn execute_list_filter(
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,
) -> Result<()> {
    use crate::services::{filter_browse_items, filter_folder_items, filter_stations, DEFAULT_MAX_RESULTS};

    // Increment version for debouncing
    state.list_filter_version = state.list_filter_version.wrapping_add(1);
    let version = state.list_filter_version;
    let query = state.list_filter_query.clone();

    if query.is_empty() {
        state.list_filter_results = None;
        state.list_filter_loading = false;
        return Ok(());
    }

    state.list_filter_loading = true;

    // Use the filter's captured category and column (not the currently focused one)
    let event_tx = event_tx.clone();
    let category = state.list_filter_category;
    let column = state.list_filter_column;

    match category {
        BrowseCategory::Artists => {
            // Filter items in the captured column of artist_nav
            if let Some(col) = state.artist_nav.columns.get(column) {
                let items: Vec<_> = col.items.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                    let results = filter_browse_items(&items, &query, DEFAULT_MAX_RESULTS);
                    let _ = event_tx.send(Event::ListFilterCompleted { version, results }).await;
                });
            }
        }
        BrowseCategory::Playlists => {
            // Filter items in the captured column of playlist_nav
            if let Some(col) = state.playlist_nav.columns.get(column) {
                let items: Vec<_> = col.items.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                    let results = filter_browse_items(&items, &query, DEFAULT_MAX_RESULTS);
                    let _ = event_tx.send(Event::ListFilterCompleted { version, results }).await;
                });
            }
        }
        BrowseCategory::Genres => {
            if state.genre_content_type == GenreContentType::Stations {
                // Filter stations in the captured column
                if let Some(col) = state.station_nav.columns.get(column) {
                    let items: Vec<_> = col.stations.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                        let results = filter_stations(&items, &query, DEFAULT_MAX_RESULTS);
                        let _ = event_tx.send(Event::ListFilterCompleted { version, results }).await;
                    });
                }
            } else {
                // Filter items in the captured column of genre_nav
                if let Some(col) = state.genre_nav.columns.get(column) {
                    let items: Vec<_> = col.items.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                        let results = filter_browse_items(&items, &query, DEFAULT_MAX_RESULTS);
                        let _ = event_tx.send(Event::ListFilterCompleted { version, results }).await;
                    });
                }
            }
        }
        BrowseCategory::Folders => {
            // Filter folder items in the captured column
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
