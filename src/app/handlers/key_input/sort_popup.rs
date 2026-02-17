//! Sort popup key handling (Ctrl+S).

use crossterm::event::{KeyCode, KeyEvent};

use crate::app::Action;
use crate::app::state::{ColumnSortMode, SortPopupOption};
use crate::app::AppState;

/// Handle keys when sort popup is active.
pub(super) fn handle_sort_popup_keys(key: KeyEvent, state: &mut AppState) -> Vec<Action> {
    let popup = match &mut state.sort_popup {
        Some(p) => p,
        None => return vec![],
    };

    match key.code {
        KeyCode::Esc => {
            return vec![Action::CloseSortPopup];
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
    let (option, col_idx) = match &state.sort_popup {
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
fn apply_sort_mode(state: &mut AppState, col_idx: usize, mode: ColumnSortMode) -> Vec<Action> {
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

    // Rebuild popup options (Direction availability may have changed)
    if let Some(popup) = &mut state.sort_popup {
        popup.rebuild_options(mode);
    }

    vec![]
}

/// Toggle sort direction for the current column.
fn toggle_sort_direction(state: &mut AppState, col_idx: usize) -> Vec<Action> {
    let nav = match state.browse_nav_mut() {
        Some(n) => n,
        None => return vec![],
    };

    if let Some(col) = nav.columns.get_mut(col_idx) {
        col.sort_ascending = !col.sort_ascending;
    }
    vec![]
}

/// Toggle group-by-album for a playlist track column.
fn toggle_group_by_album(state: &mut AppState, col_idx: usize) -> Vec<Action> {
    use crate::app::state::SortColumnType;

    let nav = match state.browse_nav_mut() {
        Some(n) => n,
        None => return vec![],
    };

    let (now_grouped, sort_mode) = if let Some(col) = nav.columns.get_mut(col_idx) {
        if col.grouped_by_album {
            col.ungroup_by_album();
        } else {
            col.group_by_album();
        }
        let result = (col.grouped_by_album, col.sort_mode);
        // Truncate columns to the right after grouping change
        nav.columns.truncate(col_idx + 1);
        if nav.focused_column > col_idx {
            nav.focused_column = col_idx;
        }
        result
    } else {
        return vec![];
    };

    // Rebuild popup options: column is now album-type or all-tracks-type
    if let Some(popup) = &mut state.sort_popup {
        popup.column_type = if now_grouped { SortColumnType::Album } else { SortColumnType::AllTracks };
        popup.rebuild_options(sort_mode);
    }

    vec![]
}

/// Toggle artwork visibility for an album column.
fn toggle_artwork(state: &mut AppState, col_idx: usize) -> Vec<Action> {
    let nav = match state.browse_nav_mut() {
        Some(n) => n,
        None => return vec![],
    };

    if let Some(col) = nav.columns.get_mut(col_idx) {
        col.artwork_visible = !col.artwork_visible;
    }
    vec![]
}
