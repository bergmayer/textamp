//! Terminal header focus and commands over shared library and queue actions.

use super::{Action, AppState, action::*, state::*};

/// A keyboard- or pointer-accessible column header control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnControl {
    Filter,
    Details,
    Next,
    Play,
    Add,
    PlaylistEdit,
    Options,
    Close,
}

/// Header cursor tied to a specific column and library category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnHeaderFocus {
    pub category: BrowseCategory,
    pub column: usize,
    pub control: ColumnControl,
}

/// Controls available for a column, in visual order.
pub fn controls(state: &AppState, index: usize) -> Vec<ColumnControl> {
    let mut result = Vec::new();
    if state
        .browse_nav()
        .and_then(|nav| nav.columns.get(index))
        .is_none_or(|column| column.details.is_none())
    {
        result.push(ColumnControl::Filter);
    }
    if let Some(column) = state.browse_nav().and_then(|nav| nav.columns.get(index)) {
        if column.details.is_some() && column.play_all_row.is_some() {
            result.push(ColumnControl::Play);
            if !matches!(column.play_all_row, Some(PlayAllRow::ArtistRadio { .. })) {
                result.push(ColumnControl::Next);
                result.push(ColumnControl::Add);
            }
        }
        if super::sources::navidrome::playlist_edit::editable(state, column).is_some() {
            result.push(ColumnControl::PlaylistEdit);
        }
        if column.has_album_details() {
            result.push(ColumnControl::Details);
        }
        if state.column_options_type(index).is_some() {
            result.push(ColumnControl::Options);
        }
    }
    result.push(ColumnControl::Close);
    result
}

/// Select rows in the current filter, including rows outside the viewport.
pub fn select_all(state: &mut AppState, index: usize) {
    let filter = &state.list_filter;
    let visible = if filter.active
        && !filter.query.is_empty()
        && filter.column == index
        && filter.category == state.browse_category
    {
        let Some(results) = &filter.results else {
            return;
        };
        Some(results.matched_indices.clone())
    } else {
        None
    };
    if state.browse_category == BrowseCategory::Folders {
        if let Some(col) = state
            .folder_state
            .as_mut()
            .and_then(|nav| nav.columns.get_mut(index))
        {
            col.selected_set = visible
                .unwrap_or_else(|| (0..col.items.len()).collect())
                .into_iter()
                .collect();
        }
    } else if let Some(nav) = state.browse_nav_mut()
        && let Some(col) = nav.columns.get(index)
    {
        if col.details.is_some() {
            return;
        }
        let rows = visible.unwrap_or_else(|| (0..col.items.len()).collect());
        if !super::sources::navidrome::playlist_edit::mark_group_rows(
            &mut nav.columns,
            index,
            rows.iter().map(|&i| (i, Some(true))),
        ) {
            nav.columns[index].selected_set = rows.into_iter().collect();
        }
    }
}

/// Focus a column without changing its selection or children.
pub fn focus(state: &mut AppState, index: usize, control: ColumnControl) {
    if let Some(nav) = state.browse_nav_mut() {
        nav.focused_column = index;
    } else if let Some(folders) = state.folder_state.as_mut() {
        folders.focused_column = index;
    }
    state.category_column_focused = false;
    state.alphabet_strip_focused = false;
    state.track_pane_focused = false;
    state.tui_column_header = Some(ColumnHeaderFocus {
        category: state.browse_category,
        column: index,
        control,
    });
}

/// Execute a header command using the shared source and queue reducers.
/// The placement distinguishes immediate playback from insertion or append.
pub fn activate(
    state: &mut AppState,
    index: usize,
    control: ColumnControl,
    placement: QueuePlacement,
) -> Vec<Action> {
    let next = control == ColumnControl::Next || placement == QueuePlacement::Next;
    focus(state, index, control);
    match control {
        ColumnControl::Filter => {
            state.tui_column_header = None;
            vec![SearchAction::ActivateListFilter.into()]
        }
        ColumnControl::Details => {
            state.tui_column_header = None;
            let Some(column) = state.browse_nav().and_then(|nav| nav.focused()) else {
                return vec![];
            };
            if column.has_album_details() {
                return vec![MillerAction::OpenAlbumDetails.into()];
            }
            vec![]
        }

        ColumnControl::PlaylistEdit => {
            state.tui_column_header = None;
            state
                .browse_nav()
                .and_then(|n| n.columns.get(index))
                .and_then(|c| {
                    super::sources::navidrome::playlist_edit::controls(state, c)
                        .into_iter()
                        .next()
                })
                .map(|c| c.actions.as_ref().clone())
                .unwrap_or_default()
        }
        ColumnControl::Options => vec![SearchAction::OpenSortPopup.into()],
        ColumnControl::Close => {
            state.tui_column_header = None;
            crate::app::handlers::key_input::close_browse_column_at(state, index);
            vec![]
        }
        ColumnControl::Play | ColumnControl::Next | ColumnControl::Add => {
            let Some(column) = state.browse_nav().and_then(|nav| nav.columns.get(index)) else {
                return vec![];
            };
            let Some(row) = column.play_all_row.as_ref() else {
                return vec![];
            };
            let controls =
                super::playback_controls::collection_controls(row, &column.ready_tracks());
            let selected = if control == ColumnControl::Add {
                2
            } else if next {
                1
            } else {
                0
            };
            controls
                .get(selected)
                .map_or_else(Vec::new, |control| control.actions.as_ref().clone())
        }
    }
}

pub use super::playback_controls::{DetailControl, QueuePlacement, TrackRange, track_action};

/// Translate modifiers that terminal keyboard and mouse protocols can report.
pub fn track_range(modifiers: crossterm::event::KeyModifiers) -> TrackRange {
    if modifiers.contains(crossterm::event::KeyModifiers::SHIFT) {
        TrackRange::Following
    } else {
        TrackRange::Single
    }
}
