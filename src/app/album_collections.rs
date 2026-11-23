//! Catalog-derived album destinations shared by both frontends.
use crate::app::{
    AppState,
    state::{BrowseCategory, BrowseColumn, BrowseItem, LibraryRoot, View},
};
pub fn open_singles(state: &mut AppState) {
    state.popups.close_all();
    state.palette.close();
    state.desktop_player_visible = false;
    state.desktop_player_fullscreen = crate::app::state::DesktopFullscreenView::Closed;
    state.sonic_details = None;
    state.track_pane_open = false;
    state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
    for slot in ["artist-browse", "album-browse"] {
        state.sources.nav_tasks.remove(slot);
    }
    state.scroll.browse = None;
    state.set_view(View::Browse);
    state.set_browse_category(BrowseCategory::Library, false);
    state.desktop_library_root = LibraryRoot::Singles;
    state.sources.nav_collection = None;
    state.category_column_focused = false;
    state.list_filter.deactivate();
    state.artist_nav.reset("Singles", vec![]);
    refresh_singles(state);
}
pub fn refresh_singles(state: &mut AppState) {
    let selected = state
        .artist_nav
        .columns
        .first()
        .and_then(|column| column.selected_item())
        .map(|item| item.key().to_owned());
    let albums = crate::services::singles::albums(
        &state.library.albums,
        &state.library.compilations,
        state.singles_track_limit,
    );
    let items = BrowseItem::from_albums(&albums, &state.library.album_display_artist);
    let selected_index = selected
        .as_ref()
        .and_then(|key| items.iter().position(|item| item.key() == key));
    let mut column = BrowseColumn::new("Singles", items);
    state.initialize_column_artwork(&mut column);
    column.selected_index = selected_index.unwrap_or(0);
    if state.artist_nav.columns.is_empty() {
        state.artist_nav.columns.push(column);
    } else {
        state.artist_nav.columns[0] = column;
    }
    if selected_index.is_none() {
        if selected.is_some() {
            state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
            for slot in ["artist-browse", "album-browse"] {
                state.sources.nav_tasks.remove(slot);
            }
            state.artist_nav.loading = false;
        }
        state.artist_nav.columns.truncate(1);
        state.artist_nav.focused_column = 0;
    }
}
