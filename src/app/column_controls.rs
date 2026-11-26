//! Terminal header focus and commands over shared library and queue actions.

use super::{Action, AppState, action::*, state::*};

/// A keyboard- or pointer-accessible column header control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnControl {
    Filter,
    Details,
    ArtistBio,
    ArtistMix,
    Next,
    Play,
    Add,
    DownloadAlbum,
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

pub(crate) fn artist_mix(
    state: &AppState,
    column: &BrowseColumn,
) -> Option<super::playback_controls::Control> {
    let artist = column.artist_header.as_ref()?;
    if state.downloads.is_offline()
        || !state
            .sources
            .active
            .capabilities()
            .supports(crate::library::capabilities::Feature::ArtistRadio)
    {
        return None;
    }
    Some(super::playback_controls::Control::new(
        "Artist Mix",
        RadioAction::StartArtistRadio {
            key: artist.artist_key.clone(),
            title: artist.artist_name.clone(),
        },
    ))
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
        if column.artist_header.is_some() {
            result.push(ColumnControl::ArtistBio);
            if artist_mix(state, column).is_some() {
                result.push(ColumnControl::ArtistMix);
            }
        }
        if (column.details.is_some() || column.has_album_details()) && column.play_all_row.is_some()
        {
            if super::details::album_bio(state, column).is_some() {
                result.push(ColumnControl::ArtistBio);
            }
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
            result.push(ColumnControl::DownloadAlbum);
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
        ColumnControl::DownloadAlbum => {
            let Some(PlayAllRow::Album { rating_key, .. }) = state
                .browse_nav()
                .and_then(|nav| nav.columns.get(index))
                .and_then(|c| c.play_all_row.as_ref())
            else {
                return vec![];
            };
            vec![super::downloads::Command::Album(rating_key.clone()).into()]
        }
        ColumnControl::ArtistBio => state
            .browse_nav()
            .and_then(|nav| nav.columns.get(index))
            .and_then(|column| {
                if let Some(info) = &column.artist_header {
                    if info.needs_web_search() {
                        let provider = state.web_search_provider;
                        return Some(super::playback_controls::Control::new(
                            provider.search_label(),
                            SystemAction::OpenBrowser(provider.artist_url(&info.artist_name)),
                        ));
                    }
                    Some(super::playback_controls::Control::new(
                        "Artist Biography",
                        SearchAction::ShowArtistBio {
                            artist_key: info.artist_key.clone(),
                            artist_name: info.artist_name.clone(),
                        },
                    ))
                } else {
                    super::details::album_bio(state, column)
                }
            })
            .map(|control| control.actions.as_ref().clone())
            .unwrap_or_default(),
        ColumnControl::ArtistMix => state
            .browse_nav()
            .and_then(|nav| nav.columns.get(index))
            .and_then(|column| artist_mix(state, column))
            .map_or_else(Vec::new, |control| control.actions.as_ref().clone()),
        ColumnControl::Details => {
            super::details::open_album_notes(state, index);
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
        ColumnControl::Options => super::view_options::open_for_input(state),
        ColumnControl::Close => {
            state.tui_column_header = None;
            crate::app::handlers::key_input::close_browse_column_at(state, index);
            crate::app::tui_navigation::focus_root_after_close(state);
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
