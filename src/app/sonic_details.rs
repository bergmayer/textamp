//! One adjacent detail pane for exploring related tracks, never an unbounded stack.
use super::{action::QueueAction, result_details, state::View, Action, AppState};
use crate::library::models::Track;
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};

#[derive(Debug, Clone)]
pub struct State {
    pub parent_key: String,
    /// Snapshot of the list this track came from, for Play Track++.
    pub tracks: Vec<Track>,
    pub index: usize,
    pub details: result_details::State,
    pub scroll_pin: Option<u16>,
}

pub fn visible(app: &AppState) -> Option<&State> {
    if app.view != View::Browse || app.popups.search_active || app.category_column_focused {
        return None;
    }
    let child = app.sonic_details.as_ref()?;
    (app.pane_track()?.rating_key == child.parent_key).then_some(child)
}

pub fn focused(app: &AppState) -> bool {
    app.track_pane_focused && visible(app).is_some_and(|c| c.details.focused)
}

pub fn palette_tracks(app: &AppState) -> Option<(&[Track], usize)> {
    if !focused(app) {
        return None;
    }
    let child = visible(app)?;
    if child.details.action_index >= 2 {
        let track = child.tracks.get(child.index)?;
        let tracks = app
            .track_pane_similar
            .get(&track.rating_key)?
            .as_ref()
            .ok()?;
        let index = child.details.action_index - 2;
        tracks.get(index)?;
        Some((tracks, index))
    } else {
        Some((&child.tracks, child.index))
    }
}

pub fn open(app: &mut AppState, index: usize, from_child: bool) -> Vec<Action> {
    let Some(parent) = app.pane_track() else {
        return vec![];
    };
    let parent_key = parent.rating_key.clone();
    let source = if from_child {
        visible(app).and_then(|c| c.tracks.get(c.index))
    } else {
        Some(parent)
    };
    let Some(tracks) = source
        .and_then(|t| app.track_pane_similar.get(&t.rating_key))
        .and_then(|r| r.as_ref().ok())
        .filter(|ts| index < ts.len())
        .cloned()
    else {
        return vec![];
    };
    let artwork = tracks[index]
        .parent_rating_key
        .as_ref()
        .zip(tracks[index].parent_thumb.as_ref())
        .filter(|(key, _)| {
            !app.artwork.grid_cache.contains_key(*key) && !app.artwork.grid_pending.contains(*key)
        })
        .map(|(key, thumb)| {
            super::action::SystemAction::LoadAlbumArt(vec![(key.clone(), thumb.clone())]).into()
        });
    app.sonic_details = Some(State {
        parent_key,
        tracks,
        index,
        scroll_pin: None,
        details: result_details::State {
            open: true,
            focused: true,
            ..Default::default()
        },
    });
    app.track_pane_focused = true;
    app.miller_scroll_manual = false;
    artwork.into_iter().collect()
}

fn play(app: &AppState, following: bool) -> Vec<Action> {
    let Some(c) = visible(app) else { return vec![] };
    if following {
        vec![QueueAction::PlayTracksNow(c.tracks[c.index..].to_vec()).into()]
    } else {
        vec![QueueAction::PlayTrack(Box::new(c.tracks[c.index].clone())).into()]
    }
}

pub fn key(key: KeyEvent, app: &mut AppState) -> Option<Vec<Action>> {
    if !focused(app) {
        return None;
    }
    let c = visible(app)?;
    let max_index = app
        .track_pane_similar
        .get(&c.tracks[c.index].rating_key)
        .and_then(|r| r.as_ref().ok())
        .map_or(1, |ts| ts.len() + 1);
    let index = c.details.action_index;
    let (scroll, max_scroll) = app
        .hit_regions
        .sonic_details
        .as_ref()
        .map_or((0, 0), |p| (p.scroll, p.max_scroll));
    let c = app.sonic_details.as_mut()?;
    c.scroll_pin = None;
    match key.code {
        KeyCode::Left | KeyCode::Esc | KeyCode::Backspace => c.details.focused = false,
        KeyCode::Up => {
            c.details.action_index = index.saturating_sub(1);
            c.details.scroll = 0;
        }
        KeyCode::Down => {
            c.details.action_index = (index + 1).min(max_index);
            c.details.scroll = 0;
        }
        KeyCode::Home => {
            c.details.action_index = 0;
            c.details.scroll = 0;
        }
        KeyCode::End => {
            c.details.action_index = max_index;
            c.details.scroll = 0;
        }
        KeyCode::PageUp => c.scroll_pin = Some(scroll.saturating_sub(8)),
        KeyCode::PageDown => c.scroll_pin = Some(scroll.saturating_add(8).min(max_scroll)),
        KeyCode::Enter if index < 2 => return Some(play(app, index == 1)),
        KeyCode::Enter => return Some(open(app, index - 2, true)),
        KeyCode::Right => {}
        _ => return None,
    }
    Some(vec![])
}

pub fn mouse(event: MouseEvent, app: &mut AppState) -> Option<Vec<Action>> {
    visible(app)?;
    let pane = app.hit_regions.sonic_details.clone()?;
    let point = (event.column, event.row).into();
    if !pane.outer.contains(point) {
        if matches!(event.kind, MouseEventKind::Down(_)) {
            app.sonic_details.as_mut()?.details.focused = false;
        }
        return None;
    }
    match event.kind {
        MouseEventKind::Down(button) => {
            let was_focused = app.sonic_details.as_ref()?.details.focused;
            app.track_pane_focused = true;
            let c = app.sonic_details.as_mut()?;
            c.details.focused = true;
            if pane.close_x.is_some_and(|r| r.contains(point)) && button == MouseButton::Left {
                app.sonic_details = None;
            } else if let Some((_, index)) =
                pane.similar_rows.iter().find(|(r, _)| r.contains(point))
            {
                let selected = was_focused && c.details.action_index == index + 2;
                c.details.action_index = index + 2;
                // Pin the viewport when highlighting a row with the pointer.
                c.scroll_pin = Some(pane.scroll);
                if button == MouseButton::Right {
                    super::command_palette::open(app);
                } else if selected && button == MouseButton::Left {
                    return Some(open(app, *index, true));
                }
            } else if button == MouseButton::Right {
                c.details.action_index = 0;
                super::command_palette::open(app);
            } else if pane.play_button.contains(point) {
                c.details.action_index = 0;
                return Some(play(app, false));
            } else if pane.following_button.contains(point) {
                c.details.action_index = 1;
                return Some(play(app, true));
            }
        }
        MouseEventKind::ScrollUp => {
            let c = app.sonic_details.as_mut()?;
            c.scroll_pin = Some(pane.scroll.saturating_sub(3));
        }
        MouseEventKind::ScrollDown => {
            let c = app.sonic_details.as_mut()?;
            c.scroll_pin = Some(pane.scroll.saturating_add(3).min(pane.max_scroll));
        }
        _ => {}
    }
    Some(vec![])
}
