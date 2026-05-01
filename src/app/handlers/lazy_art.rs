//! Shared "lazy art" settle logic.
//!
//! While the user is rapidly navigating (held arrow key, mouse wheel,
//! alphabet jumps) the keyboard handlers raise
//! `state.artwork.suppress_loads` and stamp `last_motion_at`. The
//! shared `dispatch_system::SystemAction::LoadAlbumArt` short-circuits
//! while that flag is set, so the column flips happen instantly and
//! no fetches pile up.
//!
//! Once the user has been still for [`ART_LOAD_PAUSE_MS`], the
//! front-end's tick handler calls [`settle`] which clears the gate
//! and returns a single `LoadAlbumArt` action against the current
//! viewport — one batch, not one per key event.

use crate::app::action::{Action, SystemAction};
use crate::app::state::{AppState, ART_LOAD_PAUSE_MS};
use std::time::Duration;

/// If the lazy-art gate is up and motion has been still for at least
/// [`ART_LOAD_PAUSE_MS`], clear the gate and return the actions the
/// caller should dispatch (typically a single [`SystemAction::LoadAlbumArt`]
/// for the now-visible viewport). Returns `None` when nothing needs
/// to happen this tick.
pub fn settle(state: &mut AppState) -> Option<Vec<Action>> {
    if !state.artwork.suppress_loads {
        return None;
    }
    let still_long_enough = state
        .artwork
        .last_motion_at
        .map(|t| t.elapsed() >= Duration::from_millis(ART_LOAD_PAUSE_MS))
        .unwrap_or(true);
    if !still_long_enough {
        return None;
    }
    state.artwork.suppress_loads = false;
    let batch = super::dispatch_miller::collect_viewport_art(state);
    if batch.is_empty() {
        return Some(vec![]);
    }
    Some(vec![Action::System(SystemAction::LoadAlbumArt(batch))])
}
