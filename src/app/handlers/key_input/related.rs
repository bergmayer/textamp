//! Related view key handling.

use crate::app::action::*;
use crossterm::event::{self, KeyCode};

use crate::app::Action;
use crate::app::state::View;
use crate::app::AppState;
use super::super::helpers::navigation::related_flat_resolve;

/// Handle Related view keys.
pub(in crate::app::handlers) fn handle_related_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    match key.code {
        KeyCode::Esc => {
            let target = state.previous_view.take().unwrap_or(View::Browse);
            vec![NavigationAction::SetView(target).into()]
        }
        KeyCode::F(1) | KeyCode::Char('?') => vec![NavigationAction::SetView(View::Help).into()],

        KeyCode::Up => { state.scroll.related = None; vec![DataAction::ListUp.into()] }
        KeyCode::Down => { state.scroll.related = None; vec![DataAction::ListDown.into()] }
        KeyCode::PageUp => { state.scroll.related = None; vec![DataAction::ListPageUp.into()] }
        KeyCode::PageDown => { state.scroll.related = None; vec![DataAction::ListPageDown.into()] }
        KeyCode::Home => { state.scroll.related = None; vec![DataAction::ListTop.into()] }
        KeyCode::End => { state.scroll.related = None; vec![DataAction::ListBottom.into()] }

        KeyCode::Enter => activate_related_item(state),

        // Alphabet jumping: jump to first artist starting with that letter
        KeyCode::Char(c) if c.is_ascii_alphabetic() && key.modifiers.is_empty() => {
            let letter_lower = c.to_ascii_lowercase();
            let mut offset = 0;
            for (_gi, group) in state.related.groups.iter().enumerate() {
                if group.artist.title.chars().next()
                    .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                    .unwrap_or(false)
                {
                    state.list_state.related_index = offset;
                    state.scroll.related = None;
                    break;
                }
                offset += 1 + group.albums.len();
            }
            vec![]
        }

        _ => vec![],
    }
}

/// Activate the currently highlighted related item (Enter or second click).
/// Artists: navigate to artist in library. Albums: navigate to album in library.
pub(in crate::app::handlers) fn activate_related_item(state: &mut AppState) -> Vec<Action> {
    let idx = state.list_state.related_index;
    let resolved = related_flat_resolve(&state.related.groups, idx);
    let Some((group_idx, is_header, album_idx)) = resolved else {
        return vec![];
    };

    // Clone data we need before mutating state
    let (artist_key, artist_name, album_key, album_title) = {
        let group = match state.related.groups.get(group_idx) {
            Some(g) => g,
            None => return vec![],
        };
        let ak = group.artist.rating_key.clone();
        let an = group.artist.title.clone();
        if is_header {
            (ak, an, None, None)
        } else {
            let album = match group.albums.get(album_idx) {
                Some(a) => a,
                None => return vec![],
            };
            // For synthetic alias albums, use the album's parent artist key
            let effective_artist_key = if ak.starts_with("alias:") {
                album.parent_rating_key.clone().unwrap_or(ak.clone())
            } else {
                ak.clone()
            };
            (effective_artist_key, an, Some(album.rating_key.clone()), Some(album.title.clone()))
        }
    };

    // Synthetic alias artist header — navigate to source artist instead
    let nav_artist_key = if artist_key.starts_with("alias:") {
        state.related.source_key.clone()
    } else {
        artist_key
    };

    state.set_view(View::Browse);
    state.set_browse_category(crate::app::state::BrowseCategory::Library);
    state.library.selected_artist_name = artist_name;

    if let Some(album_key_val) = album_key {
        state.search.pending_album_key = Some(album_key_val);
        state.library.selected_album_title = album_title.unwrap_or_default();
    }

    if let Some(pos) = state.artist_nav.columns.first()
        .and_then(|col| col.items.iter().position(|i| i.key() == nav_artist_key.as_str()))
    {
        if let Some(col) = state.artist_nav.columns.first_mut() {
            col.selected_index = pos;
        }
    }
    state.artist_nav.focused_column = 0;
    state.artist_nav.truncate_right();
    vec![MillerAction::LoadArtistAlbumsForMiller { artist_key: nav_artist_key }.into()]
}
