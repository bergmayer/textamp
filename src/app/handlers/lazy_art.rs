//! Shared "lazy art" settle logic.
//!
//! While the user is rapidly navigating (held arrow key, mouse wheel,
//! alphabet jumps) keyboard handlers enter `ArtworkNavigation::Moving`
//! with the latest motion time. The
//! shared `dispatch_system::SystemAction::LoadAlbumArt` short-circuits
//! while navigation is Moving, so column changes happen immediately and
//! no fetches pile up.
//!
//! Once the user has been still for [`ART_LOAD_PAUSE_MS`], the
//! front-end's tick handler calls [`settle`] to enter Settled
//! and returns a single `LoadAlbumArt` action against the current
//! viewport — one batch, not one per key event.

use crate::app::action::{Action, SystemAction};
use crate::app::state::{AppState, ART_LOAD_PAUSE_MS};
use std::time::Duration;

/// If artwork navigation is Moving and motion has been still for at least
/// [`ART_LOAD_PAUSE_MS`], transition to Settled and return the actions the
/// caller should dispatch (typically a single [`SystemAction::LoadAlbumArt`]
/// for the now-visible viewport). Returns `None` when nothing needs
/// to happen this tick.
pub fn settle(state: &mut AppState) -> Option<Vec<Action>> {
    let crate::app::state::ArtworkNavigation::Moving { last_motion } = state.artwork.navigation
    else {
        return None;
    };
    if last_motion.elapsed() < Duration::from_millis(ART_LOAD_PAUSE_MS) {
        return None;
    }
    state.artwork.navigation = crate::app::state::ArtworkNavigation::Settled;
    let batch = super::dispatch_miller::collect_viewport_art(state);
    if batch.is_empty() {
        return Some(vec![]);
    }
    Some(vec![Action::System(SystemAction::LoadAlbumArt(batch))])
}
