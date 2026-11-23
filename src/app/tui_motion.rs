//! Cell-based panel transitions with one exclusive position owner per panel.
use std::time::{Duration, Instant};

const DURATION: Duration = Duration::from_millis(240);
#[derive(Debug, Clone)]
enum Transition {
    Settled(bool),
    Moving {
        from: f32,
        visible: bool,
        started: Instant,
        duration: Duration,
    },
}
impl Transition {
    fn target(&self) -> bool {
        match self {
            Self::Settled(visible) | Self::Moving { visible, .. } => *visible,
        }
    }
    fn value(&self, now: Instant) -> f32 {
        match self {
            Self::Settled(visible) => f32::from(*visible),
            Self::Moving {
                from,
                visible,
                started,
                duration,
            } => {
                let progress = (now.saturating_duration_since(*started).as_secs_f32()
                    / duration.as_secs_f32())
                .clamp(0., 1.);
                let eased = progress * progress * (3. - 2. * progress);
                from + (f32::from(*visible) - from) * eased
            }
        }
    }
    fn retarget(&mut self, visible: bool, now: Instant) {
        let from = self.value(now);
        if visible != self.target() {
            let distance = (f32::from(visible) - from).abs();
            if distance < f32::EPSILON {
                *self = Self::Settled(visible);
            } else {
                *self = Self::Moving {
                    from,
                    visible,
                    started: now,
                    duration: DURATION.mul_f32(distance).max(Duration::from_millis(16)),
                };
            }
        } else if !self.active(now) {
            *self = Self::Settled(visible);
        }
    }
    fn active(&self, now: Instant) -> bool {
        matches!(self,Self::Moving {started,duration,..} if now.saturating_duration_since(*started)<*duration)
    }
}
#[derive(Debug, Clone)]
struct Panels {
    sidebar: Transition,
    queue: Transition,
    player: Transition,
}
/// The terminal adapter enables motion; static renderers remain deterministic.
#[derive(Debug, Clone, Default)]
pub struct Motion {
    panels: Option<Panels>,
}
impl Motion {
    /// Establish startup geometry without animating it.
    pub fn initialize(&mut self, sidebar: bool, queue: bool, player: bool, _now: Instant) {
        self.panels = Some(Panels {
            sidebar: Transition::Settled(sidebar),
            queue: Transition::Settled(queue),
            player: Transition::Settled(player),
        });
    }
    /// Reverse from the currently presented position, scaling duration to distance.
    pub fn sync(&mut self, sidebar: bool, queue: bool, player: bool, now: Instant) {
        if let Some(panels) = &mut self.panels {
            panels.sidebar.retarget(sidebar, now);
            panels.queue.retarget(queue, now);
            panels.player.retarget(player, now);
        }
    }
    /// Whether a panel requires another animation frame.
    pub fn active(&self, now: Instant) -> bool {
        self.panels.as_ref().is_some_and(|panels| {
            panels.sidebar.active(now) || panels.queue.active(now) || panels.player.active(now)
        })
    }
    /// Whether Now Playing is between layouts.
    pub fn player_active(&self, now: Instant) -> bool {
        self.panels
            .as_ref()
            .is_some_and(|panels| panels.player.active(now))
    }
    /// Presented fraction of sidebar width.
    pub fn sidebar(&self, visible: bool, now: Instant) -> f32 {
        self.panels
            .as_ref()
            .map_or(f32::from(visible), |panels| panels.sidebar.value(now))
    }
    /// Presented fraction of queue width.
    pub fn queue(&self, visible: bool, now: Instant) -> f32 {
        self.panels
            .as_ref()
            .map_or(f32::from(visible), |panels| panels.queue.value(now))
    }
    /// Presented fraction of Now Playing height.
    pub fn player(&self, visible: bool, now: Instant) -> f32 {
        self.panels
            .as_ref()
            .map_or(f32::from(visible), |panels| panels.player.value(now))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reversal_preserves_position_and_finishes_over_the_remaining_distance() {
        let start = Instant::now();
        let mut motion = Motion::default();
        motion.initialize(true, false, false, start);
        motion.sync(true, false, true, start);
        let reverse = start + Duration::from_millis(96);
        let position = motion.player(true, reverse);
        motion.sync(true, false, false, reverse);
        assert_eq!(motion.player(false, reverse), position);
        assert!(motion.player(false, reverse + Duration::from_millis(16)) < position);
        assert_eq!(
            motion.player(false, reverse + Duration::from_millis(120)),
            0.
        );
        assert!(!motion.player_active(reverse + Duration::from_millis(120)));
    }
    #[test]
    fn panel_targets_are_independent_and_start_gently() {
        let now = Instant::now();
        let mut motion = Motion::default();
        motion.initialize(true, false, false, now);
        motion.sync(false, true, true, now);
        let next = now + Duration::from_millis(16);
        assert!(motion.player(true, next) > 0. && motion.player(true, next) < 0.02);
        assert_eq!(motion.queue(true, next), motion.player(true, next));
        assert_eq!(motion.sidebar(false, now), 1.);
        assert_eq!(motion.player(true, now + DURATION), 1.);
    }
}
