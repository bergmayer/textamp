//! Short terminal transitions for structural navigation and overlay changes.
use crate::app::{
    AppState,
    presentation::HitRegions,
    state::{Modal, View},
};
use ratatui::{buffer::Buffer, layout::Rect};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    time::{Duration, Instant},
};

const DURATION: Duration = Duration::from_millis(150);

#[derive(Clone, PartialEq, Eq)]
struct Stamp {
    navigation: u64,
    depth: usize,
    viewport: usize,
    popup: Option<u64>,
    view: View,
}
impl Stamp {
    fn of(state: &AppState) -> Self {
        let mut navigation = DefaultHasher::new();
        std::mem::discriminant(&state.view).hash(&mut navigation);
        state.browse_category.hash(&mut navigation);
        std::mem::discriminant(&state.desktop_library_root).hash(&mut navigation);
        let mut depth = 0;
        if let Some(nav) = state.browse_nav() {
            depth = nav.columns.len();
            for column in &nav.columns {
                column.title.hash(&mut navigation);
                column.items.len().hash(&mut navigation);
                column
                    .items
                    .first()
                    .map(|item| item.key())
                    .hash(&mut navigation);
                column
                    .items
                    .last()
                    .map(|item| item.key())
                    .hash(&mut navigation);
                column.detail_art_key().hash(&mut navigation);
            }
        } else if let Some(nav) = &state.folder_state {
            depth = nav.columns.len();
            for column in &nav.columns {
                column.key.hash(&mut navigation);
                column.title.hash(&mut navigation);
            }
        }
        state
            .pane_track()
            .map(|track| track.rating_key.as_str())
            .hash(&mut navigation);
        crate::app::sonic_details::selected_track(state)
            .map(|track| track.rating_key.as_str())
            .hash(&mut navigation);
        state
            .tui_sidebar_group
            .as_ref()
            .map(|(title, _)| title)
            .hash(&mut navigation);
        let popup = if matches!(state.popups.modal(), Modal::Closed)
            && !state.palette.open
            && state.notifications.last_error.is_none()
            && state.view != View::Settings
        {
            None
        } else {
            let mut key = DefaultHasher::new();
            std::mem::discriminant(state.popups.modal()).hash(&mut key);
            state.palette.open.hash(&mut key);
            state.notifications.last_error.is_some().hash(&mut key);
            state
                .popups
                .library_dialog()
                .map(std::mem::discriminant)
                .hash(&mut key);
            Some(key.finish())
        };
        Self {
            navigation: navigation.finish(),
            depth,
            viewport: if state.miller_scroll_manual {
                state.miller_scroll_col
            } else {
                0
            },
            popup,
            view: state.view,
        }
    }
}

enum Kind {
    Navigation { backwards: bool },
    OpenPopup,
    ClosePopup(Rect),
}
struct Transition {
    kind: Kind,
    started: Instant,
}

/// Owned by the terminal event loop; static renders do not animate.
#[derive(Default)]
pub struct SceneMotion {
    enabled: bool,
    stamp: Option<Stamp>,
    previous: Option<Buffer>,
    popup: Option<Rect>,
    transition: Option<Transition>,
    capture: bool,
}
impl SceneMotion {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            ..Self::default()
        }
    }

    /// Observe structural changes; moving a list cursor alone never starts a transition.
    pub fn prepare(&mut self, state: &AppState, now: Instant) {
        if !self.enabled {
            return;
        }
        let next = Stamp::of(state);
        if state.tui_motion.active(now) {
            self.transition = None;
            self.previous = None;
            self.popup = None;
            self.capture = true;
            self.stamp = Some(next);
            return;
        }

        if let Some(previous) = &self.stamp {
            let kind = if next.popup != previous.popup {
                if next.popup.is_some() {
                    Some(Kind::OpenPopup)
                } else {
                    self.popup.map(Kind::ClosePopup)
                }
            } else if next.popup.is_none()
                && (next.navigation != previous.navigation || next.viewport != previous.viewport)
                && !state.fullscreen.is_open()
                && state.miller_h_drag_grab.is_none()
            {
                if next.navigation == previous.navigation && self.active(now) {
                    None
                } else {
                    Some(Kind::Navigation {
                        backwards: next.depth < previous.depth,
                    })
                }
            } else {
                None
            };
            if let Some(kind) = kind {
                self.transition = Some(Transition { kind, started: now });
                self.capture = true;
            }
        } else {
            self.capture = true;
        }
        self.stamp = Some(next);
    }
    pub fn active(&self, now: Instant) -> bool {
        self.transition
            .as_ref()
            .is_some_and(|transition| now.saturating_duration_since(transition.started) < DURATION)
    }
    /// An opening overlay takes its backdrop from the current underlying workspace.
    pub fn needs_backdrop(&self) -> bool {
        self.capture
            && self
                .transition
                .as_ref()
                .is_some_and(|transition| matches!(transition.kind, Kind::OpenPopup))
    }
    pub fn backdrop(&mut self, buffer: &Buffer) {
        self.previous = Some(buffer.clone());
    }

    /// Reveal final-position cells so visible controls retain their exact hit regions.
    pub fn compose(&mut self, buffer: &mut Buffer, regions: &mut HitRegions, now: Instant) {
        if !self.enabled {
            return;
        }
        if self
            .previous
            .as_ref()
            .is_some_and(|previous| previous.area != buffer.area)
        {
            self.transition = None;
            self.capture = true;
        }
        if let (Some(transition), Some(previous)) = (&self.transition, &self.previous) {
            let t = (now
                .saturating_duration_since(transition.started)
                .as_secs_f32()
                / DURATION.as_secs_f32())
            .clamp(0., 1.);
            let progress = 1. - (1. - t).powi(3);
            match transition.kind {
                Kind::Navigation { backwards } => {
                    let area = navigation_area(regions, buffer.area);
                    let remaining = (f32::from(area.width) * (1. - progress)).round() as u16;
                    let hidden = Rect::new(
                        if backwards {
                            area.right().saturating_sub(remaining)
                        } else {
                            area.x
                        },
                        area.y,
                        remaining,
                        area.height,
                    );
                    restore(buffer, previous, hidden);
                    if !hidden.is_empty() {
                        regions.motion_hidden.push(hidden);
                    }
                }
                Kind::OpenPopup => {
                    if let Some(area) = regions.popup_outer {
                        let height = (f32::from(area.height) * progress).round() as u16;
                        let top = area.y + (area.height - height) / 2;
                        for hidden in [
                            Rect::new(area.x, area.y, area.width, top - area.y),
                            Rect::new(
                                area.x,
                                top + height,
                                area.width,
                                area.bottom().saturating_sub(top + height),
                            ),
                        ] {
                            restore(buffer, previous, hidden);
                            if !hidden.is_empty() {
                                regions.motion_hidden.push(hidden);
                            }
                        }
                    }
                }
                Kind::ClosePopup(area) => {
                    let height = (f32::from(area.height) * (1. - progress)).round() as u16;
                    let visible = Rect::new(
                        area.x,
                        area.y + (area.height - height) / 2,
                        area.width,
                        height,
                    )
                    .intersection(buffer.area);
                    restore(buffer, previous, visible);
                    if !visible.is_empty() {
                        regions.motion_hidden.push(visible);
                    }
                }
            }
            if t >= 1. {
                self.transition = None;
                self.capture = true;
            }
        }
        self.popup = regions.popup_outer;
        if self.capture
            || self.transition.is_some()
            || self
                .stamp
                .as_ref()
                .is_some_and(|stamp| stamp.popup.is_some())
        {
            self.previous = Some(buffer.clone());
            self.capture = false;
        }
    }
}
fn navigation_area(regions: &HitRegions, frame: Rect) -> Rect {
    let mut area = regions.miller_columns.as_ref().map(|columns| columns.area);
    for pane in [&regions.track_pane, &regions.sonic_details]
        .into_iter()
        .flatten()
    {
        area = Some(area.map_or(pane.outer, |area| area.union(pane.outer)));
    }
    area.unwrap_or_else(|| {
        let left = regions
            .settings_categories
            .map_or(frame.x, |categories| categories.right());
        let bottom = regions
            .transport
            .as_ref()
            .map_or(frame.bottom().saturating_sub(1), |transport| {
                transport.area.y
            });
        Rect::new(
            left,
            frame.y,
            frame.right().saturating_sub(left),
            bottom.saturating_sub(frame.y),
        )
    })
    .intersection(frame)
}
fn restore(target: &mut Buffer, source: &Buffer, area: Rect) {
    let area = area.intersection(target.area).intersection(source.area);
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            target[(x, y)] = source[(x, y)].clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{BrowseColumn, BrowseItem};
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend};

    fn item(key: &str) -> BrowseItem {
        BrowseItem::Artist {
            key: key.into(),
            title: key.into(),
            thumb: None,
            is_placeholder: false,
        }
    }
    fn text(buffer: &Buffer) -> String {
        buffer.content.iter().map(|cell| cell.symbol()).collect()
    }

    #[test]
    fn panel_motion_owns_composition_and_discards_structural_snapshots() {
        let mut state = AppState::new();
        let now = Instant::now();
        let mut motion = SceneMotion::new(true);
        motion.prepare(&state, now);
        motion.previous = Some(Buffer::empty(Rect::new(0, 0, 80, 24)));
        motion.transition = Some(Transition {
            kind: Kind::Navigation { backwards: false },
            started: now,
        });
        state.tui_motion.initialize(true, false, false, now);
        state.tui_motion.sync(true, true, true, now);
        motion.prepare(&state, now + Duration::from_millis(50));
        assert!(!motion.active(now));
        assert!(motion.previous.is_none());
        assert!(motion.popup.is_none());
    }

    #[test]
    fn column_transition_settles_without_accepting_clicks_on_hidden_controls() {
        let mut state = AppState::new();
        state.tui_queue_visible = false;
        state.tui_sidebar_visible = false;
        state.artist_nav.reset("Before", vec![item("one")]);
        let mut terminal = Terminal::new(TestBackend::new(100, 32)).unwrap();
        let mut motion = SceneMotion::new(true);
        terminal
            .draw(|frame| {
                state.apply_render_feedback(crate::ui::render_with_motion(
                    frame,
                    &state,
                    &mut motion,
                    crate::ui::OutputHost::Terminal,
                ))
            })
            .unwrap();
        state
            .artist_nav
            .push_column(BrowseColumn::new("After", vec![item("two")]));
        terminal
            .draw(|frame| {
                state.apply_render_feedback(crate::ui::render_with_motion(
                    frame,
                    &state,
                    &mut motion,
                    crate::ui::OutputHost::Terminal,
                ))
            })
            .unwrap();
        let mask = state.hit_regions.motion_hidden[0];
        let event = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: mask.x,
            row: mask.y,
            modifiers: KeyModifiers::NONE,
        };
        assert!(crate::app::handlers::mouse_input::handle_mouse(event, &mut state).is_empty());
        assert_eq!(state.artist_nav.columns.len(), 2);
        motion.transition.as_mut().unwrap().started = Instant::now() - DURATION;
        terminal
            .draw(|frame| {
                state.apply_render_feedback(crate::ui::render_with_motion(
                    frame,
                    &state,
                    &mut motion,
                    crate::ui::OutputHost::Terminal,
                ))
            })
            .unwrap();
        assert!(state.hit_regions.motion_hidden.is_empty());
        assert!(text(terminal.backend().buffer()).contains("After"));
        assert!(!motion.active(Instant::now()));
    }

    #[test]
    fn opening_popup_reveals_against_the_workspace_and_closes_by_its_actual_button() {
        let mut state = AppState::new();
        let mut terminal = Terminal::new(TestBackend::new(100, 32)).unwrap();
        let mut motion = SceneMotion::new(true);
        terminal
            .draw(|frame| {
                state.apply_render_feedback(crate::ui::render_with_motion(
                    frame,
                    &state,
                    &mut motion,
                    crate::ui::OutputHost::Terminal,
                ))
            })
            .unwrap();
        crate::app::command_palette::open(&mut state);
        terminal
            .draw(|frame| {
                state.apply_render_feedback(crate::ui::render_with_motion(
                    frame,
                    &state,
                    &mut motion,
                    crate::ui::OutputHost::Terminal,
                ))
            })
            .unwrap();
        assert!(!state.hit_regions.motion_hidden.is_empty());
        let close = state.hit_regions.popup_close.unwrap();
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: close.x,
            row: close.y,
            modifiers: KeyModifiers::NONE,
        };
        crate::app::handlers::mouse_input::handle_mouse(click, &mut state);
        assert!(state.palette.open);
        motion.transition.as_mut().unwrap().started = Instant::now() - DURATION;
        terminal
            .draw(|frame| {
                state.apply_render_feedback(crate::ui::render_with_motion(
                    frame,
                    &state,
                    &mut motion,
                    crate::ui::OutputHost::Terminal,
                ))
            })
            .unwrap();
        crate::app::handlers::mouse_input::handle_mouse(click, &mut state);
        assert!(!state.palette.open);
    }
}
