//! Track details inside search and similarity results. Opening never plays audio.
use super::{
    action::QueueAction,
    state::{SearchTab, SimilarMode, View},
    Action, AppState,
};
use crate::library::models::Track;
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};

#[derive(Debug, Clone, Default)]
pub struct State {
    pub open: bool,
    pub focused: bool,
    pub action_index: usize,
    pub scroll: u16,
}

pub fn state(app: &AppState) -> Option<&State> {
    if app.popups.search_active || app.view == View::Search {
        Some(&app.search.details)
    } else if app.view == View::Similar {
        Some(&app.similar.details)
    } else {
        None
    }
}
fn state_mut(app: &mut AppState) -> Option<&mut State> {
    if app.popups.search_active || app.view == View::Search {
        Some(&mut app.search.details)
    } else if app.view == View::Similar {
        Some(&mut app.similar.details)
    } else {
        None
    }
}
pub fn track_list(app: &AppState) -> Option<(&[Track], usize)> {
    let (tracks, index) = if app.popups.search_active || app.view == View::Search {
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
pub fn open(app: &mut AppState) -> bool {
    if track_list(app).is_none() {
        return false;
    }
    if let Some(details) = state_mut(app) {
        *details = State {
            open: true,
            focused: true,
            ..Default::default()
        };
        return true;
    }
    false
}
fn play(app: &AppState, following: bool) -> Vec<Action> {
    let Some((tracks, index)) = track_list(app) else {
        return vec![];
    };
    if following {
        vec![QueueAction::PlayTracksNow(tracks[index..].to_vec()).into()]
    } else {
        vec![QueueAction::PlayTrack(Box::new(tracks[index].clone())).into()]
    }
}
pub fn key(key: KeyEvent, app: &mut AppState) -> Option<Vec<Action>> {
    if !state(app).is_some_and(|d| d.open && d.focused) || track_list(app).is_none() {
        return None;
    }
    let max_scroll = app
        .hit_regions
        .track_pane
        .as_ref()
        .map_or(0, |r| r.max_scroll);
    let details = state_mut(app)?;
    match key.code {
        KeyCode::Left | KeyCode::Esc | KeyCode::Backspace | KeyCode::Tab | KeyCode::BackTab => {
            details.focused = false
        }
        KeyCode::Up | KeyCode::Home => details.action_index = 0,
        KeyCode::Down | KeyCode::End => details.action_index = 1,
        KeyCode::PageUp => details.scroll = details.scroll.saturating_sub(8),
        KeyCode::PageDown => details.scroll = details.scroll.saturating_add(8).min(max_scroll),
        KeyCode::Enter => {
            let following = details.action_index == 1;
            return Some(play(app, following));
        }
        _ if key.modifiers.is_empty() => return Some(vec![]),
        _ => return None,
    }
    Some(vec![])
}
pub fn mouse(event: MouseEvent, app: &mut AppState) -> Option<Vec<Action>> {
    if !state(app).is_some_and(|d| d.open) || track_list(app).is_none() {
        return None;
    }
    let pane = app.hit_regions.track_pane.clone()?;
    let point = (event.column, event.row).into();
    if !pane.outer.contains(point) {
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Left)) {
            state_mut(app)?.focused = false;
        }
        return None;
    }
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if pane.close_x.is_some_and(|r| r.contains(point)) {
                *state_mut(app)? = State::default();
            } else if pane.play_button.contains(point) {
                return Some(play(app, false));
            } else if pane.following_button.contains(point) {
                return Some(play(app, true));
            } else {
                state_mut(app)?.focused = true;
            }
        }
        MouseEventKind::ScrollUp => {
            let d = state_mut(app)?;
            d.scroll = d.scroll.saturating_sub(3);
        }
        MouseEventKind::ScrollDown => {
            let d = state_mut(app)?;
            d.scroll = d.scroll.saturating_add(3).min(pane.max_scroll);
        }
        _ => {}
    }
    Some(vec![])
}
