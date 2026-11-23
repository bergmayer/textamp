//! Track-dependent navigation while a compact browsing snapshot is visible.
use crate::app::{
    Action, AppState,
    action::MillerAction,
    state::{BrowseCategory, LibraryRoot, View},
};

#[derive(Debug, Default)]
pub enum State {
    #[default]
    Unloaded,
    Browsing {
        pending: Option<Pending>,
    },
    Ready,
    Incomplete,
}
#[derive(Debug)]
pub struct Pending {
    request_id: u64,
    category: BrowseCategory,
    root: LibraryRoot,
    column: usize,
    key: Option<String>,
    action: MillerAction,
}
fn selected(state: &AppState) -> Option<String> {
    state
        .artist_nav
        .columns
        .get(state.artist_nav.focused_column)
        .and_then(|column| column.selected_item())
        .map(|item| item.key().to_owned())
}
pub fn defer(state: &mut AppState, action: &Action) -> bool {
    let Action::Miller(action) = action else {
        return false;
    };
    if matches!(action, MillerAction::LoadAllLibraryTracksForMiller { .. })
        && !state.library.all_tracks.is_empty()
    {
        return false;
    }
    let needs_tracks = matches!(
        action,
        MillerAction::LoadAllLibraryTracksForMiller { .. }
            | MillerAction::LoadArtistAllTracksForMiller { .. }
            | MillerAction::LoadCompilationAllTracksForMiller { .. }
            | MillerAction::LoadAllCompilationTracksForMiller { .. }
    ) || matches!(action, MillerAction::LoadAlbumTracksForMiller { album_key, .. }
        if state.desktop_library_root == LibraryRoot::Singles
            || state.library.albums.iter().any(|album| album.rating_key == *album_key && album.is_compilation_candidate()));
    if !needs_tracks {
        return false;
    }
    if matches!(state.library.catalog_loading, State::Incomplete) {
        state.set_recoverable_error(
            "Track catalog unavailable. Refresh the library to try again.".into(),
            super::state::NotificationTarget::RefreshLibrary,
        );
        return true;
    }
    if !matches!(state.library.catalog_loading, State::Browsing { .. }) {
        return false;
    }
    state.library.catalog_loading = State::Browsing {
        pending: Some(Pending {
            request_id: state.artist_nav_request_id,
            category: state.browse_category,
            root: state.desktop_library_root,
            column: state.artist_nav.focused_column,
            key: selected(state),
            action: action.clone(),
        }),
    };
    state.artist_nav.loading = true;
    true
}
/// Release an all-tracks destination as soon as its first real page is available.
pub fn preview_ready(state: &mut AppState) -> Option<Action> {
    let State::Browsing {
        pending: Some(pending),
    } = &state.library.catalog_loading
    else {
        return None;
    };
    if state.view != View::Browse
        || state.popups.search_active()
        || state.palette.open
        || state.library.all_tracks.is_empty()
        || !matches!(
            pending.action,
            MillerAction::LoadAllLibraryTracksForMiller { .. }
        )
        || pending.request_id != state.artist_nav_request_id
        || pending.category != state.browse_category
        || pending.root != state.desktop_library_root
        || pending.column != state.artist_nav.focused_column
        || (pending.key.is_some() && pending.key != selected(state))
    {
        return None;
    }
    let State::Browsing { pending } = &mut state.library.catalog_loading else {
        return None;
    };
    pending.take().map(|pending| pending.action.into())
}
pub fn complete(state: &mut AppState) -> Option<Action> {
    let previous = std::mem::replace(&mut state.library.catalog_loading, State::Ready);
    let State::Browsing {
        pending: Some(pending),
    } = previous
    else {
        return None;
    };
    (pending.request_id == state.artist_nav_request_id
        && pending.category == state.browse_category
        && pending.root == state.desktop_library_root
        && pending.column == state.artist_nav.focused_column
        && (pending.key.is_none() || pending.key == selected(state))
        && state.view == View::Browse
        && !state.popups.search_active()
        && !state.palette.open)
        .then(|| pending.action.into())
}
pub fn failed(state: &mut AppState) {
    if matches!(state.library.catalog_loading, State::Browsing { .. }) {
        state.library.catalog_loading = State::Incomplete;
        state.artist_nav.loading = false;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_track_page_releases_a_waiting_destination_without_claiming_a_complete_catalog() {
        let mut state = AppState::new();
        state.view = View::Browse;
        state.desktop_library_root = LibraryRoot::Tracks;
        state.library.catalog_loading = State::Browsing { pending: None };
        assert!(defer(
            &mut state,
            &MillerAction::LoadAllLibraryTracksForMiller {
                replace_child: false
            }
            .into()
        ));
        state.library.all_tracks = std::sync::Arc::new(vec![crate::library::models::Track {
            rating_key: "first".into(),
            ..Default::default()
        }]);
        assert!(matches!(
            preview_ready(&mut state),
            Some(Action::Miller(
                MillerAction::LoadAllLibraryTracksForMiller { .. }
            ))
        ));
        assert!(matches!(
            state.library.catalog_loading,
            State::Browsing { pending: None }
        ));
        assert!(!defer(
            &mut state,
            &MillerAction::LoadAllLibraryTracksForMiller {
                replace_child: false
            }
            .into()
        ));
    }

    #[test]
    fn track_browsing_waits_for_catalog_and_never_replaces_newer_navigation() {
        let mut state = AppState::new();
        state.view = View::Browse;
        state.desktop_library_root = LibraryRoot::Singles;
        state.library.catalog_loading = State::Browsing { pending: None };
        assert!(defer(
            &mut state,
            &MillerAction::LoadAlbumTracksForMiller {
                album_key: "split-seven-inch".into(),
                replace_child: false
            }
            .into()
        ));
        assert!(matches!(
            complete(&mut state),
            Some(Action::Miller(
                MillerAction::LoadAlbumTracksForMiller { .. }
            ))
        ));
        state.library.catalog_loading = State::Browsing { pending: None };
        let action: Action = MillerAction::LoadAllLibraryTracksForMiller {
            replace_child: false,
        }
        .into();
        assert!(defer(&mut state, &action));
        assert!(state.artist_nav.loading);
        assert!(matches!(
            complete(&mut state),
            Some(Action::Miller(
                MillerAction::LoadAllLibraryTracksForMiller { .. }
            ))
        ));
        state.library.catalog_loading = State::Browsing { pending: None };
        assert!(defer(&mut state, &action));
        state.artist_nav_request_id += 1;
        assert!(complete(&mut state).is_none());
        state.library.catalog_loading = State::Browsing { pending: None };
        assert!(defer(&mut state, &action));
        failed(&mut state);
        assert!(!state.artist_nav.loading);
        assert!(matches!(state.library.catalog_loading, State::Incomplete));
    }
}
