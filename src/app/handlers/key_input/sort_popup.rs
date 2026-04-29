//! Sort popup key handling (Ctrl+S).

use crate::app::action::*;
use crossterm::event::{KeyCode, KeyEvent};

use crate::app::Action;
use crate::app::state::{ColumnSortMode, SortPopupOption};
use crate::app::AppState;

/// Handle keys when sort popup is active.
pub(super) fn handle_sort_popup_keys(key: KeyEvent, state: &mut AppState) -> Vec<Action> {
    let popup = match &mut state.popups.sort {
        Some(p) => p,
        None => return vec![],
    };

    match key.code {
        KeyCode::Esc => {
            return vec![SearchAction::CloseSortPopup.into()];
        }
        KeyCode::Up => {
            if popup.selected_index > 0 {
                popup.selected_index -= 1;
            }
        }
        KeyCode::Down => {
            if popup.selected_index + 1 < popup.options.len() {
                popup.selected_index += 1;
            }
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
    let (option, col_idx) = match &state.popups.sort {
        Some(p) => (p.options[p.selected_index], p.column_idx),
        None => return vec![],
    };

    match option {
        SortPopupOption::SortMode(mode) => apply_sort_mode(state, col_idx, mode),
        SortPopupOption::Direction => toggle_sort_direction(state, col_idx),
        SortPopupOption::Artwork => toggle_artwork(state, col_idx),
        SortPopupOption::GroupByAlbum => toggle_group_by_album(state, col_idx),
    }
}

/// Apply a sort mode to the column and update the popup.
pub fn apply_sort_mode(state: &mut AppState, col_idx: usize, mode: ColumnSortMode) -> Vec<Action> {
    let nav = match state.browse_nav_mut() {
        Some(n) => n,
        None => return vec![],
    };

    let col = match nav.columns.get_mut(col_idx) {
        Some(c) => c,
        None => return vec![],
    };

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

    // Rebuild popup options with the actual active mode
    if let Some(popup) = &mut state.popups.sort {
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
    let nav = match state.browse_nav_mut() {
        Some(n) => n,
        None => return vec![],
    };

    if let Some(col) = nav.columns.get_mut(col_idx) {
        col.sort_ascending = !col.sort_ascending;
        // Actually reverse the items (skip pinned items at the start)
        let start = col.pinned_count();
        col.items[start..].reverse();
        if start < col.tracks.len() {
            col.tracks[start..].reverse();
        }
        col.selected_index = start; // reset selection to top of sortable items

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

    let nav = match state.browse_nav_mut() {
        Some(n) => n,
        None => return vec![],
    };

    let (now_grouped, sort_mode, pending_page, persist_state) = if let Some(col) = nav.columns.get_mut(col_idx) {
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
        let persist = col.lazy.as_ref().map(|lazy| {
            (lazy.key.clone(), now_grouped, col.artwork_visible)
        });
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
    if let Some(popup) = &mut state.popups.sort {
        popup.column_type = if now_grouped { SortColumnType::Album } else { SortColumnType::AllTracks };
        popup.rebuild_options(sort_mode);
    }

    let mut actions = auto_drill_after_sort(state);
    if let Some((playlist_key, offset)) = pending_page {
        actions.push(crate::app::action::MillerAction::LoadMorePlaylistTracks {
            playlist_key, offset,
        }.into());
    }
    if let Some((playlist_key, group, art)) = persist_state {
        if let Some(library_key) = state.active_library.clone() {
            actions.push(crate::app::action::SettingsAction::SavePlaylistView {
                library_key,
                playlist_key,
                view: crate::config::settings::PlaylistView { group_by_album: group, show_artwork: art },
            }.into());
        }
    }
    actions
}

/// Toggle artwork visibility for an album column.
/// Updates both the current column and the global default so all album columns
/// reflect the new setting (not just the one being viewed).
///
/// When turning artwork ON, the dispatcher also queues a load of
/// EVERY row's art (not just the viewport) so a long playlist's
/// album-grouping fills in art across the whole list — the previous
/// behaviour would only fetch ~30 thumbnails near the cursor and
/// leave the rest blank.
pub fn toggle_artwork(state: &mut AppState, col_idx: usize) -> Vec<Action> {
    let (new_visible, persist_state) = {
        let nav = match state.browse_nav_mut() {
            Some(n) => n,
            None => return vec![],
        };

        if let Some(col) = nav.columns.get_mut(col_idx) {
            col.artwork_visible = !col.artwork_visible;
            // Playlist tracks columns: snapshot the playlist key
            // (lives on `lazy`) along with the new toggle states so
            // the caller can dispatch a persist action.
            let persist = col.lazy.as_ref().map(|lazy| {
                (lazy.key.clone(), col.grouped_by_album, col.artwork_visible)
            });
            (col.artwork_visible, persist)
        } else {
            return vec![];
        }
    };

    // Update the global default so all future/existing album columns use this setting
    state.artwork.default_visible = new_visible;

    let mut actions: Vec<Action> = Vec::new();

    // Eager-load every row's artwork for the column we just turned
    // ON. The viewport-window loader runs on scroll for cheap, but a
    // user explicitly enabling art expects the whole list to show
    // covers (especially for grouped-by-album playlists).
    if new_visible {
        if let Some(nav) = state.browse_nav() {
            let batch = super::super::dispatch_miller::collect_all_art_to_load(
                nav.columns.get(col_idx),
                &state.artwork.grid_cache,
                &state.artwork.grid_pending,
            );
            if !batch.is_empty() {
                actions.push(crate::app::action::SystemAction::LoadAlbumArt(batch).into());
            }
        }
    }

    // Persist the per-playlist toggle to config when this is a
    // playlist tracks column (identified by `col.lazy.key`).
    if let Some((playlist_key, group, art)) = persist_state {
        if let Some(library_key) = state.active_library.clone() {
            actions.push(crate::app::action::SettingsAction::SavePlaylistView {
                library_key,
                playlist_key,
                view: crate::config::settings::PlaylistView { group_by_album: group, show_artwork: art },
            }.into());
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
