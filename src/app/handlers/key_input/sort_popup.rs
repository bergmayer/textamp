//! Sort popup key handling (Ctrl+S).

use crate::app::action::*;
use crossterm::event::{KeyCode, KeyEvent};

use crate::app::Action;
use crate::app::AppState;
use crate::app::state::{ColumnSortMode, SortPopupOption};

/// Handle keys when sort popup is active.
pub(super) fn handle_sort_popup_keys(key: KeyEvent, state: &mut AppState) -> Vec<Action> {
    let popup = match state.popups.sort_mut() {
        Some(p) => p,
        None => return vec![],
    };

    match key.code {
        KeyCode::Esc => {
            return vec![SearchAction::CloseSortPopup.into()];
        }
        KeyCode::Up if popup.selected_index > 0 => {
            popup.selected_index -= 1;
        }
        KeyCode::Down if popup.selected_index + 1 < popup.options.len() => {
            popup.selected_index += 1;
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            return apply_selected_option(state);
        }
        _ => {
            // Absorb all other keys while popup is open
        }
    }
    vec![]
}

/// Apply the currently selected sort popup option.
/// Used by both keyboard (Enter/Space) and mouse click handlers.
pub fn apply_selected_option(state: &mut AppState) -> Vec<Action> {
    let (option, col_idx) = match state.popups.sort() {
        Some(p) => (p.options[p.selected_index], p.column_idx),
        None => return vec![],
    };

    match option {
        SortPopupOption::SortMode(mode) => apply_sort_mode(state, col_idx, mode),
        SortPopupOption::Direction => toggle_sort_direction(state, col_idx),
        SortPopupOption::Artwork => toggle_artwork(state, col_idx),
        SortPopupOption::LargeArtwork => {
            vec![SettingsAction::SetLargeArtwork(!state.large_artwork).into()]
        }
        SortPopupOption::GroupByAlbum => toggle_group_by_album(state, col_idx),
        SortPopupOption::CompilationOnlyArtists => {
            vec![
                SettingsAction::SetShowCompilationOnlyArtists(!state.show_compilation_only_artists)
                    .into(),
            ]
        }
    }
}

/// Apply a sort mode to the column and update the popup.
pub fn apply_sort_mode(state: &mut AppState, col_idx: usize, mode: ColumnSortMode) -> Vec<Action> {
    let downloads = crate::app::downloads::is_root_column(state, col_idx);
    let nav = match state.browse_nav_mut() {
        Some(n) => n,
        None => return vec![],
    };

    let col = match nav.columns.get_mut(col_idx) {
        Some(c) => c,
        None => return vec![],
    };

    if mode == ColumnSortMode::ByDateAdded && col.date_added_order.is_none() {
        return vec![];
    }

    col.end_playlist_editing();
    if downloads && col.grouped_by_album {
        col.ungroup_by_album();
    }

    // First restore to original if we have originals and switching to non-shuffle mode
    if col.has_originals() && mode != ColumnSortMode::Shuffled {
        col.unshuffle();
    }

    // Apply the requested sort
    col.apply_sort(mode);

    // Truncate columns to the right after sort change
    nav.columns.truncate(col_idx + 1);
    if nav.focused_column > col_idx {
        nav.focused_column = col_idx;
    }

    // Non-Shuffle sort modes are permanent edits: clear originals so the new order
    // becomes the baseline. Only Shuffle is a reversible toggle.
    if mode != ColumnSortMode::Shuffled && mode != ColumnSortMode::Default {
        let nav = match state.browse_nav_mut() {
            Some(n) => n,
            None => return vec![],
        };
        if let Some(col) = nav.columns.get_mut(col_idx) {
            col.clear_originals();
        }
    }

    if downloads
        && state.downloads.group_by_album
        && let Some(col) = state.artist_nav.columns.get_mut(col_idx)
    {
        col.clear_originals();
        col.group_by_album();
    }

    // Rebuild popup options with the actual active mode
    if let Some(popup) = state.popups.sort_mut() {
        popup.rebuild_options(mode);
    }

    // Selection-only model: do not auto-drill after sort. User presses
    // Enter/Right to open a child column.
    let _ = state;
    vec![]
}

/// Toggle sort direction for the current column.
/// Reverses the non-pinned items/tracks and flips the ascending flag.
pub fn toggle_sort_direction(state: &mut AppState, col_idx: usize) -> Vec<Action> {
    let downloads = crate::app::downloads::is_root_column(state, col_idx);
    let nav = match state.browse_nav_mut() {
        Some(n) => n,
        None => return vec![],
    };

    if let Some(col) = nav.columns.get_mut(col_idx) {
        col.end_playlist_editing();
        let regroup = downloads && col.grouped_by_album;
        if regroup {
            col.ungroup_by_album();
        }
        col.sort_ascending = !col.sort_ascending;
        // Actually reverse the items (skip pinned items at the start)
        let start = col.pinned_count();
        col.items[start..].reverse();
        if start < col.tracks.len() {
            col.tracks[start..].reverse();
        }
        col.selected_index = start; // reset selection to top of sortable items
        if regroup {
            col.clear_originals();
            col.group_by_album();
        }

        // Truncate child columns since order changed
        nav.columns.truncate(col_idx + 1);
        if nav.focused_column > col_idx {
            nav.focused_column = col_idx;
        }
    }

    // Auto-drill to repopulate child column after direction change
    auto_drill_after_sort(state)
}

/// Toggle group-by-album for a playlist track column.
///
/// Long playlists paginate: only the first page lives in `col.tracks`
/// at toggle time. If the column is paginated and grouping is being
/// turned ON, we kick off `LoadMorePlaylistTracks` to fetch the rest
/// of the pages — the event handler re-runs the grouping after each
/// page lands. Without this the user sees only the first page's
/// albums grouped and the tail silently missing.
pub fn toggle_group_by_album(state: &mut AppState, col_idx: usize) -> Vec<Action> {
    use crate::app::state::SortColumnType;

    if crate::app::downloads::is_root_column(state, col_idx) {
        return crate::app::downloads::toggle_grouping(state);
    }

    let nav = match state.browse_nav_mut() {
        Some(n) => n,
        None => return vec![],
    };

    let (now_grouped, sort_mode, pending_page, persist_state) =
        if let Some(col) = nav.columns.get_mut(col_idx) {
            col.end_playlist_editing();
            if col.grouped_by_album {
                col.ungroup_by_album();
            } else {
                col.group_by_album();
            }
            let now_grouped = col.grouped_by_album;
            let sort_mode = col.sort_mode;
            // Snapshot pagination info BEFORE truncating columns (which
            // re-borrows `nav.columns`).
            let pending = if now_grouped {
                col.lazy.as_ref().and_then(|lazy| {
                    let total = lazy.total? as usize;
                    if !lazy.loading && col.tracks.len() < total {
                        Some((lazy.key.clone(), col.tracks.len() as u32))
                    } else {
                        None
                    }
                })
            } else {
                None
            };
            // For a playlist's track column, the toggle is per-(library,
            // playlist) and gets persisted. The playlist key lives on
            // the column's `lazy` marker (every playlist tracks col gets
            // one — see `PlaylistFirstPageLoaded` event handler).
            let persist = col
                .lazy
                .as_ref()
                .map(|lazy| (lazy.key.clone(), now_grouped));
            (now_grouped, sort_mode, pending, persist)
        } else {
            return vec![];
        };
    // Now safe to mutate nav.columns again.
    nav.columns.truncate(col_idx + 1);
    if nav.focused_column > col_idx {
        nav.focused_column = col_idx;
    }

    // Rebuild popup options: column is now album-type or all-tracks-type
    if let Some(popup) = state.popups.sort_mut() {
        popup.column_type = if now_grouped {
            SortColumnType::Album
        } else {
            SortColumnType::AllTracks
        };
        popup.rebuild_options(sort_mode);
    }

    let mut actions = auto_drill_after_sort(state);
    if let Some((playlist_key, offset)) = pending_page {
        actions.push(
            crate::app::action::MillerAction::LoadMorePlaylistTracks {
                playlist_key,
                offset,
            }
            .into(),
        );
    }
    if let Some((playlist_key, group)) = persist_state
        && let Some(library_key) = state.active_library.clone()
    {
        actions.push(
            crate::app::action::SettingsAction::SavePlaylistView {
                library_key,
                playlist_key,
                view: crate::config::settings::PlaylistView {
                    group_by_album: group,
                },
            }
            .into(),
        );
    }
    actions
}

/// Toggle artwork visibility for an album column.
/// Retain visibility for this column type in the active library.
///
/// Enabling artwork requests every row so grouped playlist artwork is
/// available beyond the current viewport.
pub fn toggle_artwork(state: &mut AppState, col_idx: usize) -> Vec<Action> {
    let column_key = state.column_artwork_key(col_idx);
    let new_visible = {
        let nav = match state.browse_nav_mut() {
            Some(n) => n,
            None => return vec![],
        };

        if let Some(col) = nav.columns.get_mut(col_idx) {
            col.artwork_visible = !col.artwork_visible;
            col.artwork_visible
        } else {
            return vec![];
        }
    };

    let mut actions: Vec<Action> = Vec::new();
    if let Some(library_key) = state.active_library.clone() {
        actions.push(
            SettingsAction::SaveColumnArtwork {
                library_key,
                column_key,
                visible: new_visible,
            }
            .into(),
        );
    }

    // Eager-load every row's artwork for the column we just turned
    // ON. The viewport-window loader runs on scroll for cheap, but a
    // user explicitly enabling art expects the whole list to show
    // covers (especially for grouped-by-album playlists).
    if new_visible && let Some(nav) = state.browse_nav() {
        let batch = super::super::dispatch_miller::collect_all_art_to_load(
            nav.columns.get(col_idx),
            &state.artwork.grid_cache,
            &state.artwork.grid_pending,
        );
        if !batch.is_empty() {
            actions.push(crate::app::action::SystemAction::LoadAlbumArt(batch).into());
        }
    }

    actions
}

/// Selection-only model: never auto-drill after sort/direction/grouping
/// changes. Child columns only open on explicit Enter/Right.
fn auto_drill_after_sort(state: &mut AppState) -> Vec<Action> {
    let _ = state;
    vec![]
}
