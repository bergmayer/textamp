//! Visible activity derived from owned operations, shared by all presentations.
use super::{AppState, BrowseCategory};

/// Project existing request ownership; no UI timers or inferred completion state.
pub fn message(state: &AppState) -> Option<&str> {
    match state.player_sync.phase {
        super::player_sync::Phase::Preparing => return Some("Preparing player sync…"),
        super::player_sync::Phase::Copying(_) => return Some("Syncing MP3 player…"),
        super::player_sync::Phase::Cancelling => return Some("Stopping player sync…"),
        super::player_sync::Phase::Ejecting => return Some("Ejecting player…"),
        _ => {}
    }

    if target(state).is_some() {
        return Some("Downloading music…");
    }

    if state.browse_category == BrowseCategory::Playlists && state.playlist_nav.loading {
        return Some("Loading playlist…");
    }
    if let Some(message) = state.operation_progress() {
        return Some(message);
    }
    for (slot, label) in [
        ("playlist-browse", "Loading playlist…"),
        ("playlist-play", "Loading playlist…"),
        ("artist-browse", "Loading artist…"),
        ("album-browse", "Loading album…"),
        ("related", "Loading related music…"),
        ("similar", "Loading similar music…"),
        ("similar-pane", "Loading similar tracks…"),
        ("search", "Searching…"),
    ] {
        if state.sources.nav_tasks.contains_key(slot)
            && (slot != "similar-pane" || super::details::similar_seed(state).is_some())
        {
            return Some(label);
        }
    }
    if state.library.right_panel_loading {
        return Some("Loading details…");
    }
    if state.list_filter.loading {
        return Some("Filtering…");
    }
    if state.library_loading {
        return Some(
            if !state.library.artists.is_empty()
                && matches!(
                    state.library.catalog_loading,
                    super::catalog_loading::State::Browsing { .. }
                )
            {
                "Loading track catalog…"
            } else {
                "Loading library…"
            },
        );
    }
    if state.browse_nav().is_some_and(|nav| nav.loading)
        || (state.browse_category == BrowseCategory::Folders
            && state.folder_state.as_ref().is_some_and(|nav| nav.loading))
        || state.current_tag_loading()
        || state.library.artists_loading
        || state.library.albums_loading
        || state.library.playlists_loading
    {
        return Some("Loading music…");
    }
    if state.radio.refill != super::state::RadioRefill::Idle {
        return Some("Loading more station tracks…");
    }
    if state.station_starting.is_some() {
        return Some("Loading station…");
    }
    None
}

/// Action associated with the currently projected activity.
pub fn target(state: &AppState) -> Option<super::state::NotificationTarget> {
    state
        .downloads
        .jobs
        .values()
        .any(|job| !matches!(job.phase, super::downloads::Phase::Failed(_)))
        .then_some(super::state::NotificationTarget::Downloads)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn only_visible_operations_keep_activity_alive() {
        let mut state = AppState::new();
        let work = tokio::spawn(std::future::pending::<()>());
        let lease = crate::app::tasks::TaskLease::new(&work);
        state
            .sources
            .nav_tasks
            .insert("background-artwork", (1, lease.clone()));
        assert!(message(&state).is_none());
        state
            .sources
            .nav_tasks
            .insert("album-browse", (2, lease.clone()));
        assert_eq!(message(&state), Some("Loading album…"));
        state.sources.nav_tasks.remove("album-browse");
        assert!(message(&state).is_none());
        work.abort();
    }
}
