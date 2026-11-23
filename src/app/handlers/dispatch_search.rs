//! Search dispatch handlers: ExecuteLocalSearch, ClearSearch, SelectSearchResult,
//! ActivateListFilter, DeactivateListFilter, FilteredList*,
//! SelectFilteredItem, AppendListFilterChar, DeleteListFilterChar, ClearListFilter,
//! ExecuteListFilter, OpenSearchPopup, CloseSearchPopup, OpenLibraryPicker,
//! CloseLibraryPicker.

use crate::app::action::{BrowseAction, SearchAction};
use crate::app::event::*;
use crate::app::state::{BrowseCategory, BrowseItem, SearchFocus, SearchTab, View};
use crate::app::{Action, AppState, Event};
use crate::library::models::SearchResults;

use anyhow::Result;
use tokio::sync::mpsc;

pub(crate) const SEARCH_DEBOUNCE_SLOT: &str = "search-debounce";
const FIRST_LETTER_DELAY: std::time::Duration = std::time::Duration::from_millis(220);
const SEARCH_SLOT: &str = "catalog-search";

pub(crate) fn search_pending(state: &AppState) -> bool {
    state.sources.nav_tasks.contains_key(SEARCH_DEBOUNCE_SLOT)
        || state.sources.nav_tasks.contains_key(SEARCH_SLOT)
        || (!state.search.query.trim().is_empty()
            && state.sources.nav_tasks.contains_key("search-index"))
}

static SEARCH_WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

fn catalog_search(
    state: &mut AppState,
) -> std::sync::Arc<crate::services::catalog_search::IndexedCatalog> {
    let catalog = crate::services::catalog_search::CatalogSearch {
        artists: state.library.artists.clone(),
        albums: state.library.albums.clone(),
        playlists: state.library.playlists.clone(),
        genres: state.library.album_genres.clone(),
        aliases: state.library.artist_aliases.clone(),
        tracks: state.library.all_tracks.clone(),
    };
    state.library.search_cache.prepare(catalog)
}

/// Prepare normalized search fields independently of browsing and user input.
pub(crate) fn warm_catalog(state: &mut AppState, tx: &mpsc::Sender<Event>) {
    let catalog = catalog_search(state);
    if catalog.ready() {
        return;
    }
    crate::app::sources::navidrome::effects::spawn(state, tx, "search-index", async move {
        let (cancel, mut cancelled) = tokio::sync::oneshot::channel::<()>();
        let permit = SEARCH_WORKERS.acquire().await?;
        crate::app::tasks::spawn_blocking(move || {
            let _permit = permit;
            catalog.warm(|| {
                matches!(
                    cancelled.try_recv(),
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed)
                )
            });
        })
        .await?;
        drop(cancel);
        Ok(vec![])
    });
}

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
pub fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: SearchAction,
    state: &mut AppState,
) -> Result<Vec<Action>> {
    let mut follow_ups = vec![];

    match action {
        SearchAction::SubmitSearch => {
            if state
                .sources
                .nav_tasks
                .remove(SEARCH_DEBOUNCE_SLOT)
                .is_some()
                || (state.search.results.is_none()
                    && !state.sources.nav_tasks.contains_key(SEARCH_SLOT))
            {
                follow_ups.push(SearchAction::ExecuteLocalSearch.into());
            }
        }
        SearchAction::ExecuteLocalSearch => {
            if state.sources.nav_tasks.contains_key(SEARCH_DEBOUNCE_SLOT) {
                return Ok(vec![]);
            }
            let query = state.search.query.trim().to_lowercase();
            if query.is_empty() {
                state.sources.nav_tasks.remove(SEARCH_SLOT);
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

            let catalog = catalog_search(state);
            if catalog.item_count() <= 256 {
                state.sources.nav_tasks.remove(SEARCH_SLOT);
                state.search.results = Some(catalog.search(&query, || false));
                return Ok(vec![]);
            }
            let categories_only =
                !catalog.ready() && state.sources.nav_tasks.contains_key("search-index");
            let catalog_id = state.sources.list_id;
            let input = state.search.query.clone();
            crate::app::sources::navidrome::effects::spawn(
                state,
                event_tx,
                SEARCH_SLOT,
                async move {
                    let (cancel, mut cancelled) = tokio::sync::oneshot::channel::<()>();
                    let permit = SEARCH_WORKERS.acquire().await?;
                    let results = crate::app::tasks::spawn_blocking(move || {
                        let _permit = permit;
                        let cancelled = || {
                            matches!(
                                cancelled.try_recv(),
                                Err(tokio::sync::oneshot::error::TryRecvError::Closed)
                            )
                        };
                        if categories_only {
                            catalog.categories(&query, cancelled)
                        } else {
                            catalog.search(&query, cancelled)
                        }
                    })
                    .await?;
                    drop(cancel);
                    Ok(vec![
                        crate::app::sources::navidrome::NavAction::SearchReady {
                            query: input,
                            catalog_id,
                            results,
                        }
                        .into(),
                    ])
                },
            );
        }

        SearchAction::SelectSearchResult => {
            let section = state.search.results.as_ref().map(|results| {
                if state.search.tab == SearchTab::Global {
                    resolve_global_index(results, state.list_state.search_item_index).0
                } else {
                    state.search.tab
                }
            });
            if matches!(
                section,
                Some(SearchTab::Artists | SearchTab::Albums | SearchTab::Tracks)
            ) {
                return Ok(crate::app::library_details::search_result_actions(state));
            }
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
                        && let Some(index) = column.items.iter().position(|item| &item.key == path)
                    {
                        column.selected_index = index;
                    }
                    state.popups.set_search_active(false);
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
            if state.search.query == q {
                return Ok(vec![]);
            }
            state.sources.nav_tasks.remove(SEARCH_DEBOUNCE_SLOT);
            state.sources.nav_tasks.remove(SEARCH_SLOT);
            state.search.focus = SearchFocus::Input;
            state.list_state.search_item_index = 0;
            state.scroll.search = None;
            let previous = state.search.results.take();
            state.search.query = q;
            let query = state.search.query.trim();
            let mut characters = unicode_segmentation::UnicodeSegmentation::graphemes(query, true);
            if characters.next().is_none() {
                return Ok(vec![]);
            }
            if characters.next().is_none() {
                crate::app::sources::navidrome::effects::spawn(
                    state,
                    event_tx,
                    SEARCH_DEBOUNCE_SLOT,
                    async {
                        tokio::time::sleep(FIRST_LETTER_DELAY).await;
                        Ok(vec![SearchAction::ExecuteLocalSearch.into()])
                    },
                );
            } else {
                // Refine the visible matches while the indexed search fills in the rest.
                if let Some(previous) = previous {
                    state.search.results = Some(
                        crate::services::catalog_search::CatalogSearch {
                            artists: std::sync::Arc::new(previous.artists),
                            albums: std::sync::Arc::new(previous.albums),
                            playlists: std::sync::Arc::new(previous.playlists),
                            genres: std::sync::Arc::new(previous.genres),
                            tracks: std::sync::Arc::new(previous.tracks),
                            aliases: state.library.artist_aliases.clone(),
                        }
                        .search(query, || false),
                    );
                }
                follow_ups.push(SearchAction::ExecuteLocalSearch.into());
            }
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
            state.list_filter.advance_version();
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
            if let Some(nav) = state.browse_nav_mut()
                && let Some(col) = nav.columns.get_mut(filter_col_idx)
            {
                col.header_selection = crate::app::state::ColumnHeaderSelection::Items;
            }
            // Same reason as the header selection above — any transient
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
            state.scroll.browse = None;
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
            if let Some(ref results) = state.list_filter.results
                && let Some(&item_idx) = results.matched_indices.get(state.list_filter.selected)
            {
                super::key_input::update_filter_column_selection(state, item_idx);
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
            if let Some(ref results) = state.list_filter.results
                && let Some(&item_idx) = results.matched_indices.get(state.list_filter.selected)
            {
                super::key_input::update_filter_column_selection(state, item_idx);
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
                state.miller_scroll_manual = false;
                let drilldown_actions = super::key_input::get_filter_drilldown_actions(state);
                follow_ups.extend(drilldown_actions);
            }
        }
        SearchAction::AppendListFilterChar(c) => {
            state.list_filter.query.push(c);
            state.list_filter.selected = 0;
            schedule_list_filter(event_tx, state)?;
        }
        SearchAction::DeleteListFilterChar => {
            state.list_filter.query.pop();
            if state.list_filter.query.is_empty() {
                state.list_filter.deactivate();
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
                state.scroll.browse = None;
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
                schedule_list_filter(event_tx, state)?;
            }
        }
        SearchAction::RunListFilter { version } => {
            run_list_filter(event_tx, state, version);
        }
        // Search popup actions
        SearchAction::OpenSearchPopup => {
            state.sources.nav_tasks.remove(SEARCH_DEBOUNCE_SLOT);
            state.sources.nav_tasks.remove(SEARCH_SLOT);

            if state.list_filter.active {
                state.list_filter.deactivate();
            }
            state.popups.close_all();
            state.popups.set_search_active(true);
            state.search.focus = SearchFocus::Input;
            state.search.query.clear();
            state.search.results = None;
            state.list_state.search_item_index = 0;
            if state.sources.active.folder().is_some() {
                state.search.tab = SearchTab::Tracks;
            }
        }
        SearchAction::CloseSearchPopup => {
            state.sources.nav_tasks.remove(SEARCH_DEBOUNCE_SLOT);
            state.sources.nav_tasks.remove(SEARCH_SLOT);

            state.popups.set_search_active(false);
        }

        // Library picker popup actions
        action @ (SearchAction::OpenLibraryPicker | SearchAction::ManageLibraries) => {
            state.popups.close_all();
            if matches!(action, SearchAction::ManageLibraries) {
                state.set_view(View::Settings);
                state.settings_state.section = crate::app::state::SettingsSection::Libraries;
                state.settings_state.focus = crate::app::state::SettingsFocus::Sections;
            } else {
                state.popups.set_library_picker_active(true);
            }
            crate::app::sources::manager::reset_selection(state);
            state.sources.picker_scroll_pin = None;
            state.popups.library_picker_index = crate::app::sources::choices(state)
                .iter()
                .position(|entry| entry.active(state))
                .unwrap_or(0);
        }
        SearchAction::CloseLibraryPicker => {
            state.popups.set_library_picker_active(false);
        }

        // Sort popup actions
        SearchAction::OpenSortPopup => {
            if state.view == View::Browse
                && let Some(nav) = state.browse_nav()
                && let Some(popup) = crate::app::view_options::for_column(state, nav.focused_column)
            {
                state.popups.close_all();
                state.popups.set_sort(Some(popup));
            }
        }
        SearchAction::CloseSortPopup => {
            state.popups.set_sort(None);
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

        // Adventure launcher popup actions
        SearchAction::StartPickedTrackRadio => {
            if let Some(launcher) = state.popups.adventure_launcher()
                && let Some(track) = launcher.start_track.clone()
            {
                let action =
                    if launcher.purpose == crate::app::state::TrackPickerPurpose::StayCloseRadio {
                        crate::app::action::RadioAction::StartStation(
                            crate::app::state::ActiveStation {
                                source: crate::library::station::RadioSource::SonicFixed(
                                    track.rating_key.clone(),
                                ),
                                title: format!("Stay Close Mix · {}", track.title),
                            },
                        )
                    } else {
                        crate::app::action::RadioAction::StartSonicRadio(Box::new(track))
                    };
                state.popups.set_adventure_launcher(None);
                follow_ups.push(action.into());
            }
        }
        action @ (SearchAction::OpenAdventureLauncher
        | SearchAction::OpenTrackRadioPicker
        | SearchAction::OpenStayCloseRadioPicker) => {
            let radio = !matches!(action, SearchAction::OpenAdventureLauncher);
            let stay_close = matches!(action, SearchAction::OpenStayCloseRadioPicker);
            let current = if radio {
                state.current_track().cloned()
            } else {
                None
            };
            state.adventure_launcher_request_id =
                state.adventure_launcher_request_id.wrapping_add(1);
            state.popups.close_all();
            state
                .popups
                .set_adventure_launcher(Some(crate::app::state::AdventureLauncherState {
                    terminal_focus: Default::default(),
                    purpose: if stay_close {
                        crate::app::state::TrackPickerPurpose::StayCloseRadio
                    } else if radio {
                        crate::app::state::TrackPickerPurpose::SonicRadio
                    } else {
                        crate::app::state::TrackPickerPurpose::Journey
                    },
                    stage: Default::default(),
                    step: crate::app::state::AdventureStep::FindStartTrack,
                    query: String::new(),
                    results: None,
                    focus: SearchFocus::Input,
                    item_index: 0,
                    loading: false,
                    drill: crate::app::state::AdventureDrillLevel::Search,
                    start_track: current,
                    end_track: None,
                    track_count_input: "20".to_string(),
                    scroll_pin: None,
                    // Journey only ever picks tracks (start /
                    // end), so force the tracks tab — that path makes
                    // `adventure_launcher_select_track` index directly
                    // into `results.tracks` without an artist+album
                    // offset.
                    search_tab: crate::app::state::SearchTab::Tracks,
                }));
        }
        SearchAction::OpenAdventureLauncherWithStart { start_track } => {
            // Pre-select the start track (e.g. from a right-click on a
            // specific track row) and skip past FindStartTrack — the
            // user already chose; only the count + end track remain.
            state.adventure_launcher_request_id =
                state.adventure_launcher_request_id.wrapping_add(1);
            state.popups.close_all();
            state
                .popups
                .set_adventure_launcher(Some(crate::app::state::AdventureLauncherState {
                    terminal_focus: crate::app::journey::JourneyControl::End,
                    purpose: Default::default(),
                    stage: Default::default(),
                    step: crate::app::state::AdventureStep::FindEndTrack,
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
                    // Journey only ever picks tracks (start /
                    // end), so force the tracks tab — that path makes
                    // `adventure_launcher_select_track` index directly
                    // into `results.tracks` without an artist+album
                    // offset.
                    search_tab: crate::app::state::SearchTab::Tracks,
                }));
        }
        SearchAction::CloseAdventureLauncher => {
            if state.popups.adventure_launcher().is_some_and(|launcher| {
                launcher.purpose == crate::app::state::TrackPickerPurpose::Journey
            }) {
                state.sources.sonic_tasks.remove("adventure");
                state.adventure_request_id = state.adventure_request_id.wrapping_add(1);
                state.adventure = Default::default();
            }
            state.adventure_launcher_request_id =
                state.adventure_launcher_request_id.wrapping_add(1);
            state.popups.set_adventure_launcher(None);
        }
        SearchAction::AdventureLauncherPlay => {
            let tracks = state.popups.adventure_launcher_mut().and_then(|launcher| {
                if let crate::app::state::JourneyStage::Ready(tracks) = &launcher.stage {
                    Some(tracks.clone())
                } else {
                    None
                }
            });
            if let Some(tracks) = tracks {
                state.popups.set_adventure_launcher(None);
                follow_ups
                    .push(crate::app::action::SettingsAction::AdventureComplete(tracks).into());
            }
        }
        SearchAction::AdventureLauncherEdit => {
            state.sources.sonic_tasks.remove("adventure");
            state.adventure_request_id = state.adventure_request_id.wrapping_add(1);
            state.adventure = Default::default();
            if let Some(launcher) = state.popups.adventure_launcher_mut() {
                launcher.stage = crate::app::state::JourneyStage::Choosing;
                launcher.loading = false;
                launcher.query.clear();
                launcher.results = None;
                launcher.focus = SearchFocus::Input;
            }
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
            if let Some(l) = state.popups.adventure_launcher_mut() {
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
            if let Some(l) = state.popups.adventure_launcher_mut() {
                std::mem::swap(&mut l.start_track, &mut l.end_track);
            }
        }
        SearchAction::AdventureLauncherSetTrackCount(s) => {
            if let Some(l) = state.popups.adventure_launcher_mut() {
                // Keep only digits, max 3 characters (so users can't
                // accidentally type a multi-thousand-track adventure).
                l.track_count_input = s.chars().filter(|c| c.is_ascii_digit()).take(3).collect();
            }
        }

        SearchAction::AdventureLauncherSetEndpointQuery { step, query } => {
            if let Some(launcher) = state.popups.adventure_launcher_mut() {
                launcher.step = step;
                launcher.query = query;
            }
            adventure_launcher_search(event_tx, state)?;
        }
        SearchAction::AdventureLauncherSetQuery(q) => {
            if let Some(l) = state.popups.adventure_launcher_mut() {
                l.query = q;
            }
            // Trigger the search with the new query.
            adventure_launcher_search(event_tx, state)?;
        }

        // Multi-artist radio picker actions
        SearchAction::OpenArtistRadioPicker => {
            state.popups.close_all();
            state
                .popups
                .set_artist_radio_picker(Some(crate::app::state::ArtistRadioPickerState {
                    query: String::new(),
                    filtered_artists: state
                        .library
                        .artists
                        .iter()
                        .take(ARTIST_PICKER_LIMIT)
                        .cloned()
                        .collect(),
                    selected_artists: vec![],
                    focus: SearchFocus::Input,
                    item_index: 0,
                    scroll_pin: None,
                }));
        }
        SearchAction::CloseArtistRadioPicker => {
            state.popups.set_artist_radio_picker(None);
        }
        SearchAction::ArtistRadioPickerSearch => {
            if let Some(picker) = state.popups.artist_radio_picker_mut() {
                if picker.query.is_empty() {
                    picker.filtered_artists = state
                        .library
                        .artists
                        .iter()
                        .take(ARTIST_PICKER_LIMIT)
                        .cloned()
                        .collect();
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
            if let Some(picker) = state.popups.artist_radio_picker_mut()
                && let Some(artist) = picker.filtered_artists.get(picker.item_index).cloned()
            {
                // Toggle: remove if already selected, add if not
                if let Some(pos) = picker
                    .selected_artists
                    .iter()
                    .position(|a| a.rating_key == artist.rating_key)
                {
                    picker.selected_artists.remove(pos);
                } else if picker.selected_artists.len() < 12 {
                    picker.selected_artists.push(artist);
                }
            }
        }
        // Artist bio popup (F4)
        action @ (SearchAction::OpenBiographySource
        | SearchAction::SearchBiographyOnWeb
        | SearchAction::OpenArtistInfoSource
        | SearchAction::SearchArtistInfoOnWeb) => {
            let subject = if matches!(
                action,
                SearchAction::OpenArtistInfoSource | SearchAction::SearchArtistInfoOnWeb
            ) {
                state
                    .artist_info_column_index()
                    .and_then(|index| state.browse_nav()?.columns.get(index))
                    .and_then(|column| match &column.details {
                        Some(crate::app::state::BrowseDetails::Artist(info)) => {
                            Some((&info.artist_name, &info.document))
                        }
                        _ => None,
                    })
            } else {
                state
                    .popups
                    .artist_bio()
                    .map(|popup| (&popup.artist_name, &popup.document))
            };
            let url = subject.and_then(|(name, document)| match action {
                SearchAction::SearchBiographyOnWeb | SearchAction::SearchArtistInfoOnWeb
                    if !name.trim().is_empty() =>
                {
                    Some(state.web_search_provider.artist_url(name))
                }
                SearchAction::OpenBiographySource | SearchAction::OpenArtistInfoSource => {
                    document.source_url.clone()
                }
                _ => None,
            });
            if let Some(url) = url {
                // The native host owns browser presentation. Deliver the effect at
                // the event-loop boundary, including when a detail action produced it.
                let tx = event_tx.clone();
                crate::app::tasks::spawn(async move {
                    let _ = tx
                        .send(Event::Effect(
                            crate::app::action::SystemAction::OpenBrowser(url).into(),
                        ))
                        .await;
                });
            }
        }
        SearchAction::ShowArtistInfo {
            artist_key,
            artist_name,
        } => super::helpers::biography::show_info(state, event_tx, artist_key, artist_name),
        SearchAction::ArtistInfoPhoto(delta) => {
            super::helpers::biography::cycle_info_photo(state, delta)
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
    let results = match state.search.results.clone() {
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
                return vec![
                    BrowseAction::OpenInLibrary {
                        artist_key: artist.rating_key.clone(),
                        artist_name: artist.title.clone(),
                        album_key: None,
                        album_title: None,
                    }
                    .into(),
                ];
            }
        }
        SearchTab::Albums => {
            if let Some(album) = results.albums.get(local_idx) {
                return vec![
                    BrowseAction::OpenInLibrary {
                        artist_key: album.parent_rating_key.clone().unwrap_or_default(),
                        artist_name: album.artist_name().to_owned(),
                        album_key: Some(album.rating_key.clone()),
                        album_title: Some(album.title.clone()),
                    }
                    .into(),
                ];
            }
        }
        SearchTab::Tracks => {
            // Handled in place before taking the results.
            state.search.results = Some(results);
            return vec![];
        }
        SearchTab::Playlists => {
            if let Some(playlist) = results.playlists.get(local_idx) {
                crate::app::navigation_history::record(state);
                state.popups.set_search_active(false);
                state.library_navigation =
                    crate::app::navigation_history::LibraryNavigation::Columns(Some(
                        crate::app::sidebar::SidebarGroup::Playlists,
                    ));
                state.set_browse_category(BrowseCategory::Playlists, false);
                state.set_view(View::Browse);
                state.category_column_focused = false;
                state.playlist_nav.reset(
                    "playlists",
                    BrowseItem::from_playlists(&state.library.playlists),
                );
                state.playlist_nav.columns[0].selected_index = state.playlist_nav.columns[0]
                    .items
                    .iter()
                    .position(|item| item.key() == playlist.rating_key)
                    .unwrap_or(0);
                return vec![
                    crate::app::action::MillerAction::LoadPlaylistTracksForMiller {
                        playlist_key: playlist.rating_key.clone(),
                        replace_child: false,
                    }
                    .into(),
                ];
            }
        }
        SearchTab::Genres => {
            if let Some(genre) = results.genres.get(local_idx) {
                crate::app::navigation_history::record(state);
                let genre_key = genre.effective_key().to_string();
                let genre_title = genre.title.clone();
                state.search.query.clear();

                state.popups.set_search_active(false);
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

                return vec![
                    crate::app::action::MillerAction::LoadGenreAlbumsForMiller {
                        genre_key,
                        replace_child: false,
                    }
                    .into(),
                ];
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

    let global_idx = results
        .global_order
        .get(global_idx)
        .copied()
        .unwrap_or(global_idx);
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
    state.list_filter.advance_version();
    state.list_filter.refresh_anchor = None;
    let version = state.list_filter.version;

    if state.list_filter.query.is_empty() {
        state.scroll.browse = None;
        state.list_filter.results = None;
        state.list_filter.loading = false;
        return Ok(());
    }

    state.list_filter.loading = true;
    state.list_filter.results = None;

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
    use crate::app::filtering::browse_filter_records;
    use crate::services::{
        DEFAULT_MAX_RESULTS, filter_browse_records_until, filter_with_priority_until,
    };

    if version != state.list_filter.version
        || !state.list_filter.active
        || state.list_filter.query.is_empty()
    {
        return;
    }

    let cancelled = state.list_filter.cancellation();
    let generation = state.library_generation;
    let column = state.list_filter.column;
    let query = state.list_filter.query.clone();
    let tx = event_tx.clone();
    let category = state.list_filter.category;
    if category == BrowseCategory::Folders {
        let Some(items) = state
            .folder_state
            .as_ref()
            .and_then(|nav| nav.columns.get(column))
            .map(|column| {
                column
                    .items
                    .iter()
                    .map(|item| item.title.clone())
                    .collect::<Vec<_>>()
            })
        else {
            state.list_filter.deactivate();
            return;
        };
        crate::app::tasks::spawn_blocking(move || {
            let Some(results) = filter_with_priority_until(
                &items,
                &query,
                |title| title.as_str(),
                DEFAULT_MAX_RESULTS,
                cancelled,
            ) else {
                return;
            };
            let _ = tx.blocking_send(
                UiEvent::ListFilterCompleted {
                    version,
                    generation,
                    results,
                }
                .into(),
            );
        });
        return;
    }
    let nav = match category {
        BrowseCategory::Library => &state.artist_nav,
        BrowseCategory::Playlists => &state.playlist_nav,
        cat if cat.is_tag_section() => &state.tag_nav,
        _ => {
            state.list_filter.deactivate();
            return;
        }
    };
    let Some(column) = nav.columns.get(column) else {
        state.list_filter.deactivate();
        return;
    };
    let items = browse_filter_records(&column.items);
    let aliases = if category == BrowseCategory::Library {
        state.library.artist_aliases.clone()
    } else {
        Default::default()
    };
    let mut comp_keys =
        if category == BrowseCategory::Library && !state.show_compilation_only_artists {
            state.library.compilations.artist_keys.clone()
        } else {
            Default::default()
        };
    if let Some(key) = &state.revealed_compilation_artist {
        comp_keys.remove(key);
    }
    crate::app::tasks::spawn_blocking(move || {
        let Some(results) = filter_browse_records_until(
            &items,
            &query,
            DEFAULT_MAX_RESULTS,
            &aliases,
            &comp_keys,
            cancelled,
        ) else {
            return;
        };
        let _ = tx.blocking_send(
            UiEvent::ListFilterCompleted {
                version,
                generation,
                results,
            }
            .into(),
        );
    });
}

fn adventure_launcher_search(_event_tx: &mpsc::Sender<Event>, state: &mut AppState) -> Result<()> {
    state.adventure_launcher_request_id = state.adventure_launcher_request_id.wrapping_add(1);
    let launcher = match state.popups.adventure_launcher_mut() {
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
        global_order: vec![],
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
    let launcher = match state.popups.adventure_launcher_mut() {
        Some(l) => l,
        None => return Ok(vec![]),
    };

    let track = match crate::app::journey::choices(launcher).get(launcher.item_index) {
        Some(crate::app::journey::Choice::Track(track)) => (*track).clone(),
        _ => return Ok(vec![]),
    };
    match launcher.step {
        AdventureStep::FindStartTrack => launcher.start_track = Some(track),
        AdventureStep::FindEndTrack => launcher.end_track = Some(track),
    }
    // Reset the search panel so the user can pick the other field
    // immediately without manually clearing it.
    launcher.query.clear();
    launcher.results = None;
    launcher.drill = AdventureDrillLevel::Search;
    launcher.item_index = 0;
    launcher.focus = SearchFocus::Input;

    launcher.stage = crate::app::state::JourneyStage::Choosing;
    if launcher.start_track.is_some() && launcher.end_track.is_some() {
        launcher.terminal_focus = crate::app::journey::JourneyControl::Preview;
    } else if launcher.start_track.is_some() {
        launcher.step = AdventureStep::FindEndTrack;
        launcher.terminal_focus = crate::app::journey::JourneyControl::End;
    } else {
        launcher.step = AdventureStep::FindStartTrack;
        launcher.terminal_focus = crate::app::journey::JourneyControl::Start;
    }

    Ok(vec![])
}

/// Adventure launcher: handle back navigation.
fn adventure_launcher_back(state: &mut AppState) {
    use crate::app::state::AdventureDrillLevel;
    state.adventure_launcher_request_id = state.adventure_launcher_request_id.wrapping_add(1);
    let Some(launcher) = state.popups.adventure_launcher_mut() else {
        return;
    };
    if matches!(launcher.drill, AdventureDrillLevel::Search) {
        state.popups.set_adventure_launcher(None);
    } else {
        launcher.drill = AdventureDrillLevel::Search;
        launcher.item_index = 0;
        launcher.scroll_pin = None;
        launcher.focus = SearchFocus::Input;
    }
}

/// Check if the currently focused column is the filter's target column.
pub(super) fn is_on_filter_column(state: &AppState) -> bool {
    if state.browse_category != state.list_filter.category {
        return false;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::runtime::Session, audio::AudioPlayer, library::track::Track};
    use std::{sync::Arc, time::Duration};

    fn search_session() -> Session {
        let mut session = Session::with_audio(Default::default(), AudioPlayer::new_without_audio());
        session.state.popups.set_search_active(true);
        session.state.library.all_tracks = Arc::new(vec![Track {
            title: "Alpha".into(),
            ..Default::default()
        }]);
        session
    }

    #[tokio::test]
    async fn fuzzy_search_waits_for_one_grapheme_and_then_publishes() {
        let mut session = search_session();
        session
            .dispatch(SearchAction::SetSearchQuery("a".into()).into())
            .await
            .unwrap();
        assert!(search_pending(&session.state));
        assert!(session.state.search.results.is_none());

        let event = tokio::time::timeout(Duration::from_secs(2), session.completions.recv())
            .await
            .unwrap()
            .unwrap();
        let Event::Effect(action) = event else {
            panic!("expected search timer completion");
        };
        session.dispatch(action).await.unwrap();
        assert!(!search_pending(&session.state));
        assert_eq!(
            session.state.search.results.unwrap().tracks[0].title,
            "Alpha"
        );
    }

    #[tokio::test]
    async fn fuzzy_search_supersedes_first_letter_and_submits_without_waiting() {
        let mut session = search_session();
        session
            .dispatch(SearchAction::SetSearchQuery("á".into()).into())
            .await
            .unwrap();
        assert!(search_pending(&session.state));
        session
            .dispatch(SearchAction::ExecuteLocalSearch.into())
            .await
            .unwrap();
        assert!(session.state.search.results.is_none());
        let request_id = session.state.sources.nav_tasks[SEARCH_DEBOUNCE_SLOT].0;

        session
            .dispatch(SearchAction::SetSearchQuery("alp".into()).into())
            .await
            .unwrap();
        assert!(!search_pending(&session.state));
        assert_eq!(
            session.state.search.results.as_ref().unwrap().tracks[0].title,
            "Alpha"
        );
        session
            .dispatch(SearchAction::SetSearchQuery("a".into()).into())
            .await
            .unwrap();
        session
            .dispatch(
                crate::app::sources::navidrome::NavAction::Completed {
                    generation: session.state.library_generation,
                    request_id,
                    slot: SEARCH_DEBOUNCE_SLOT,
                    result: Ok(vec![SearchAction::ExecuteLocalSearch.into()]),
                }
                .into(),
            )
            .await
            .unwrap();
        assert!(search_pending(&session.state));
        assert!(session.state.search.results.is_none());

        session
            .dispatch(SearchAction::SubmitSearch.into())
            .await
            .unwrap();
        assert!(!search_pending(&session.state));
        assert_eq!(
            session.state.search.results.unwrap().tracks[0].title,
            "Alpha"
        );
    }
}
