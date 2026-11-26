//! One adjacent detail pane for exploring related tracks, never an unbounded stack.
use super::{Action, AppState, result_details, state::View};
use crate::library::models::Track;
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};

#[derive(Debug, Clone)]
pub struct State {
    pub parent_key: String,
    /// Ordered source list for track-and-following operations.
    pub tracks: Vec<Track>,
    pub index: usize,
    pub details: result_details::State,
    pub scroll_pin: Option<u16>,
}

pub fn visible(app: &AppState) -> Option<&State> {
    if app.view != View::Browse || app.popups.search_active() || app.category_column_focused {
        return None;
    }
    let child = app.sonic_details.as_ref()?;
    (app.pane_track()?.rating_key == child.parent_key).then_some(child)
}

pub fn selected_track(app: &AppState) -> Option<&Track> {
    let child = visible(app)?;
    child.tracks.get(child.index)
}

pub fn focused(app: &AppState) -> bool {
    app.track_pane_focused && visible(app).is_some_and(|c| c.details.focused)
}

pub fn palette_tracks(app: &AppState) -> Option<(&[Track], usize)> {
    if !focused(app) {
        return None;
    }
    let child = visible(app)?;
    if child.details.action_index >= 3 {
        let track = child.tracks.get(child.index)?;
        let tracks = app
            .track_pane_similar
            .get(&track.rating_key)?
            .as_ref()
            .ok()?;
        let index = child.details.action_index - 3;
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
            focused: true,
            ..Default::default()
        },
    });
    app.track_pane_focused = true;
    app.miller_scroll_manual = false;
    artwork.into_iter().collect()
}

fn track_action(
    app: &AppState,
    control: usize,
    modifiers: crossterm::event::KeyModifiers,
) -> Vec<Action> {
    let Some(track) = selected_track(app) else {
        return vec![];
    };
    crate::app::column_controls::DetailControl::ALL
        .get(control)
        .map(|control| {
            control.activate(
                app,
                track,
                crate::app::column_controls::track_range(modifiers),
            )
        })
        .unwrap_or_default()
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
        .map_or(2, |ts| ts.len() + 2);
    let index = c.details.action_index;
    let (scroll, max_scroll) = app
        .hit_regions
        .sonic_details
        .as_ref()
        .map_or((0, 0), |p| (p.scroll, p.max_scroll));
    let c = app.sonic_details.as_mut()?;
    c.scroll_pin = None;
    match key.code {
        KeyCode::Right if index < 2 => c.details.action_index += 1,
        KeyCode::Left if index > 0 && index < 3 => c.details.action_index -= 1,
        KeyCode::Left | KeyCode::Esc | KeyCode::Backspace => c.details.focused = false,
        KeyCode::Up | KeyCode::Down => {
            c.details.action_index = crate::app::result_details::vertical_selection(
                index,
                max_index,
                key.code == KeyCode::Down,
            );
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
        KeyCode::Enter if index < 3 => return Some(track_action(app, index, key.modifiers)),
        KeyCode::Enter => return Some(open(app, index - 3, true)),
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
                let selected = was_focused && c.details.action_index == index + 3;
                c.details.action_index = index + 3;
                // Pin the viewport when highlighting a row with the pointer.
                c.scroll_pin = Some(pane.scroll);
                if button == MouseButton::Right {
                    return Some(super::command_palette::open(app));
                } else if selected && button == MouseButton::Left {
                    return Some(open(app, *index, true));
                }
            } else if button == MouseButton::Right {
                c.details.action_index = 0;
                return Some(super::command_palette::open(app));
            } else if pane.play_button.contains(point) {
                c.details.action_index = 0;
                return Some(track_action(app, 0, event.modifiers));
            } else if pane.next_button.contains(point) {
                c.details.action_index = 1;
                return Some(track_action(app, 1, event.modifiers));
            } else if pane.add_button.contains(point) {
                c.details.action_index = 2;
                return Some(track_action(app, 2, event.modifiers));
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
