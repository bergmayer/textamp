//! Ordered result tracks and focus state for adjacent library details.
use super::{
    AppState,
    state::{SearchTab, SimilarMode, View},
};
use crate::library::models::Track;

#[derive(Debug, Clone, Default)]
pub struct State {
    pub focused: bool,
    pub action_index: usize,
    pub scroll: u16,
}

pub fn track_list(app: &AppState) -> Option<(&[Track], usize)> {
    let (tracks, index) = if app.popups.search_active() || app.view == View::Search {
        let results = app.search.results.as_ref()?;
        let (section, index) = if app.search.tab == SearchTab::Global {
            super::handlers::dispatch_search::resolve_global_index(
                results,
                app.list_state.search_item_index,
            )
        } else {
            (app.search.tab, app.list_state.search_item_index)
        };
        if section != SearchTab::Tracks {
            return None;
        }
        (results.tracks.as_slice(), index)
    } else if app.view == View::Similar && app.similar.mode == SimilarMode::Tracks {
        (app.similar.tracks.as_slice(), app.list_state.similar_index)
    } else {
        return None;
    };
    tracks.get(index)?;
    Some((tracks, index))
}
