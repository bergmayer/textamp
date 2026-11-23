//! Layout motion around stock controls; playback and navigation stay in the actor.
use super::*;
use gpui_kit::base::motion::{Transition, transition};
use std::time::Duration;

pub const SIDEBAR_WIDTH: f32 = crate::ui::layout::SIDEBAR_WIDTH;
pub const SIDEBAR_DURATION: Duration = Duration::from_millis(320);
pub const COLUMN_DURATION: Duration = Duration::from_millis(180);
pub const PLAYER_DURATION: Duration = Duration::from_millis(240);
pub const TRANSPORT_HEIGHT: f32 = crate::ui::layout::TRANSPORT_HEIGHT;
#[cfg(test)]
pub const HERO_SIDE: f32 = 190.;

#[derive(Clone, Copy)]
pub struct SheetGeometry {
    pub height: f32,
    pub artwork_side: f32,
    pub artwork_x: f32,
    pub artwork_y: f32,
    pub controls_y: f32,
}
impl SheetGeometry {
    pub fn at(progress: f32, height: f32, hero_side: f32) -> Self {
        let p = progress.clamp(0., 1.);
        Self {
            height: TRANSPORT_HEIGHT + (height - TRANSPORT_HEIGHT) * p,
            artwork_side: 32. + (hero_side - 32.) * p,
            artwork_x: 172. - 156. * p,
            artwork_y: 16.,
            controls_y: (hero_side + 32.) * p,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaneKey {
    Column(usize),
    Inspector,
}
pub struct PaneMotion {
    target: Bounds<Pixels>,
    seeded: bool,
}

impl Desktop {
    pub(super) fn presentation_duration(&self, duration: Duration) -> Duration {
        if self.drawn_presentation == self.snapshot.presentation.identity() {
            duration
        } else {
            Duration::ZERO
        }
    }

    pub(super) fn sample_column_motion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let animate = self
            .column_motion_until
            .is_some_and(|until| std::time::Instant::now() < until);
        let policy = Transition::new(if animate {
            COLUMN_DURATION
        } else {
            Duration::ZERO
        });
        for (key, pane) in &mut self.pane_targets {
            let live = match key {
                PaneKey::Column(index) => self
                    .snapshot
                    .columns
                    .get(*index)
                    .is_some_and(|column| !column.hidden && !column.metadata),
                PaneKey::Inspector => self.snapshot.preview.is_some(),
            };
            let id = SharedString::from(format!(
                "column-motion-{}-{key:?}",
                self.snapshot.generation
            ));
            let collapsed = Bounds::new(
                point(pane.target.right(), pane.target.top()),
                size(px(0.), pane.target.size.height),
            );
            if !pane.seeded && live {
                transition(
                    id.clone(),
                    collapsed,
                    Transition::new(Duration::ZERO),
                    window,
                    cx,
                );
                pane.seeded = true;
            }
            let target = if live {
                pane.target
            } else {
                pane.seeded = false;
                collapsed
            };
            self.pane_presented
                .insert(*key, transition(id, target, policy.clone(), window, cx));
        }
    }
}

/// Reflow content at interpolated bounds; text and hit targets follow the pane
/// instead of scaling a screenshot. Kit retains ownership of the resize handles.
pub fn pane(
    key: PaneKey,
    child: AnyElement,
    owner: WeakEntity<Desktop>,
    presented: Option<Bounds<Pixels>>,
    animate: bool,
) -> AnyElement {
    div()
        .id(SharedString::from(format!("pane-{key:?}")))
        .size_full()
        .child(
            canvas(
                move |bounds, window, cx| {
                    let changed = owner
                        .update(cx, |desktop, _| {
                            if desktop
                                .pane_targets
                                .get(&key)
                                .is_some_and(|pane| pane.target == bounds)
                            {
                                false
                            } else {
                                desktop
                                    .pane_targets
                                    .entry(key)
                                    .and_modify(|pane| pane.target = bounds)
                                    .or_insert(PaneMotion {
                                        target: bounds,
                                        seeded: false,
                                    });
                                true
                            }
                        })
                        .unwrap_or(false);
                    if changed {
                        let owner = owner.clone();
                        window.defer(cx, move |_, cx| {
                            let _ = owner.update(cx, |_, cx| cx.notify());
                        });
                    }
                    let shown = if !animate || cx.reduce_motion() {
                        bounds
                    } else {
                        presented.unwrap_or_else(|| {
                            Bounds::new(
                                point(bounds.right(), bounds.top()),
                                size(px(0.), bounds.size.height),
                            )
                        })
                    };
                    let mut child = child;
                    child.layout_as_root(
                        size(
                            AvailableSpace::Definite(shown.size.width.max(px(1.))),
                            AvailableSpace::Definite(shown.size.height),
                        ),
                        window,
                        cx,
                    );
                    child.prepaint_at(shown.origin, window, cx);
                    child
                },
                |_, mut child, window, cx| child.paint(window, cx),
            )
            .size_full(),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn player_sheet_keeps_utility_footer_outside_its_moving_transport() {
        let collapsed = SheetGeometry::at(0., 740., HERO_SIDE);
        assert_eq!(collapsed.height, TRANSPORT_HEIGHT);
        let expanded = SheetGeometry::at(1., 740., HERO_SIDE);
        assert_eq!(expanded.height, 740.);
        assert_eq!(expanded.artwork_side, HERO_SIDE);
        for step in 0..=20 {
            let next = SheetGeometry::at(step as f32 / 20., 740., HERO_SIDE);
            assert!((0. ..=740.).contains(&next.height));
        }
    }
}
