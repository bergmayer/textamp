//! Captured logical workflow content and request identity, independent of window geometry.
use super::state::*;

#[derive(Clone)]
pub(crate) struct Context {
    pub(crate) popups: Popups,
    pub(crate) palette: PaletteState,
    view: View,
    previous_view: Option<View>,
    search: Option<SearchState>,
    indices: (usize, usize, usize),
    pub(crate) bio_request: u64,
}
impl Context {
    pub(crate) fn capture(state: &super::AppState) -> Self {
        Self {
            popups: state.popups.clone(),
            palette: state.palette.clone(),
            view: state.view,
            previous_view: state.previous_view,
            search: (state.view == View::Search || state.popups.search_active())
                .then(|| state.search.clone()),
            indices: (
                state.list_state.search_item_index,
                state.list_state.similar_index,
                state.list_state.related_index,
            ),
            bio_request: state.artist_bio_request_id,
        }
    }
    pub(crate) fn restore(&self, state: &mut super::AppState) {
        state.popups = self.popups.clone();
        state.palette = self.palette.clone();
        state.view = self.view;
        state.previous_view = self.previous_view;
        if let Some(search) = &self.search {
            state.search = search.clone();
        }
        (
            state.list_state.search_item_index,
            state.list_state.similar_index,
            state.list_state.related_index,
        ) = self.indices;
        state.artist_bio_request_id = self.bio_request;
    }
}
