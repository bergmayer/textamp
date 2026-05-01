//! Browse-column click → action routing.
//!
//! Single source of truth for "the user clicked a row in column N
//! at index K — what should happen?" Both front-ends (TUI mouse
//! handler and GUI iced click) call this service to translate a
//! `BrowseItem` + click context into the action stream that drives
//! the drill (or lack thereof).
//!
//! Pre-extraction the same match-on-`BrowseItem` lived in two
//! places, walked separately through every category and item kind,
//! and silently drifted whenever one front-end was patched without
//! the other.
//!
//! The service is pure: it doesn't touch the audio backend, the
//! Plex client, or any I/O. It mutates `AppState` only for the side
//! effects that the click commits unconditionally — selection,
//! focused column, truncating stale child columns, closing the
//! track-details pane when drilling away from a Track row, and
//! emitting the drill's column-title hint into
//! `state.library.selected_album_title`.

use crate::app::action::{Action, BrowseAction, MillerAction, RadioAction};
use crate::app::state::{
    AllTracksScope, AppState, BrowseCategory, BrowseColumn, BrowseItem,
};

/// Inputs to `plan_drill`. The caller (TUI mouse / GUI click) gathers
/// these from the click event and the current state. `activate` is
/// `true` for double-click / Enter-equivalent gestures; the service
/// uses it to distinguish "first click selects" from "second click
/// drills" for non-leaf rows.
pub struct ClickContext {
    pub column_index: usize,
    pub item_index: usize,
    pub activate: bool,
}

/// Result of planning a click drill. The caller dispatches the
/// returned actions through the normal action stream — no other
/// state mutation is needed.
pub struct DrillPlan {
    /// Actions to dispatch. May be empty (selection-only click, or
    /// a synchronous grouped-album local push that needs no further
    /// action).
    pub actions: Vec<Action>,
    /// True when the click pushed or will push a rightward column.
    /// The caller uses this to decide whether to snap focus back to
    /// the clicked column (the GUI does this so the user's click
    /// target stays highlighted after the drill).
    pub did_drill: bool,
}

/// Translate a Miller-column click into an action sequence. Mutates
/// `state` for the immediate-commit side effects (selection, focus,
/// truncate stale children, close track pane on non-Track drill,
/// `selected_album_title` hint). The drill itself is communicated
/// via `replace_child: true` on each emitted Miller action.
///
/// Returns `DrillPlan { actions: vec![], did_drill: false }` for a
/// "selection only" click (typical first click on a non-leaf row).
pub fn plan_drill(state: &mut AppState, ctx: ClickContext) -> DrillPlan {
    // Any click inside a Miller column moves keyboard focus out of
    // the leftmost category column.
    state.category_column_focused = false;

    let pane_open = state.track_pane_open;

    // Selection / focus / truncate, plus a probe of the clicked item
    // type for the click-drill rule. The cloned `BrowseItem` is the
    // payload for the rest of the function so the borrow on `nav`
    // ends before we re-borrow `state` for category-specific work.
    let (item, grouped_album_col, auto_drill) = {
        let Some(nav) = state.browse_nav_mut() else { return empty() };
        let had_child = ctx.column_index + 1 < nav.columns.len() || pane_open;
        let Some(col) = nav.columns.get_mut(ctx.column_index) else { return empty() };

        // Leaf-style rows (AllTracks / AllArtists / Compilations /
        // CompilationTracks / Track) drill on a single click — they
        // have no further child to navigate to, so the two-step
        // "select then drill" gesture is misleading. Probe before
        // mutating selection so the read uses the click's target.
        let click_drills = matches!(
            col.items.get(ctx.item_index),
            Some(BrowseItem::AllTracks { .. })
                | Some(BrowseItem::AllArtists)
                | Some(BrowseItem::Compilations)
                | Some(BrowseItem::CompilationTracks { .. })
                | Some(BrowseItem::Track { .. })
        );

        col.selected_index = ctx.item_index;
        nav.focused_column = ctx.column_index;

        // Drill triggers:
        //   1. `activate` (double-click / Enter equivalent)
        //   2. `had_child` — something rightward is open
        //   3. `click_drills` — leaf rows always drill on one click
        let auto_drill = !ctx.activate && (had_child || click_drills);

        // Stale rightward columns are truncated on a non-activate
        // click; the drill (if any) repopulates them via the
        // dispatcher.
        if !ctx.activate {
            nav.columns.truncate(ctx.column_index + 1);
        }

        if !ctx.activate && !auto_drill {
            return empty();
        }

        let item = nav.columns.get(ctx.column_index)
            .and_then(|c| c.items.get(ctx.item_index)).cloned();

        // Playlists with `grouped_by_album` build the drill column
        // locally — the tracks already live in the playlist column,
        // no API round-trip needed.
        let grouped_album_col = if state.browse_category == BrowseCategory::Playlists {
            state.playlist_nav.columns.get(ctx.column_index).and_then(|c| {
                if c.grouped_by_album {
                    grouped_album_drill(c, ctx.item_index)
                } else {
                    None
                }
            })
        } else {
            None
        };

        (item, grouped_album_col, auto_drill)
    };
    let Some(item) = item else { return empty() };

    // Auto-drilling onto a non-Track row leaves the track-details
    // pane stranded — close it. The Track arm below reopens the
    // pane via OpenTrackDetails.
    if auto_drill && !matches!(item, BrowseItem::Track { .. }) {
        state.track_pane_open = false;
        state.track_pane_focused = false;
        state.track_pane_index = 0;
    }

    // Per-item dispatch. Click drills set `replace_child: true` so
    // the dispatcher replaces the rightward slot in place — the
    // click already moved focus to the clicked column and we don't
    // want push_column to advance focus to the new child.
    let mut is_drill_action = true;
    let mut local_push_drill = false;
    let actions: Vec<Action> = match item {
        BrowseItem::Artist { key, title, .. } => {
            state.library.selected_artist_name = title;
            vec![MillerAction::LoadArtistAlbumsForMiller { artist_key: key, replace_child: true }.into()]
        }
        BrowseItem::Album { key, title, .. } => {
            if let Some(new_col) = grouped_album_col {
                // Playlists+grouped_by_album: synchronous local push.
                state.playlist_nav.push_column(new_col);
                is_drill_action = false;
                local_push_drill = true;
                Vec::new()
            } else {
                state.library.selected_album_title = title;
                // Tag sections share `tag_nav`; their dispatcher
                // pushes onto that nav. Library / Playlists push
                // onto `artist_nav` / `playlist_nav` via
                // `LoadAlbumTracksForMiller`.
                let action = if state.browse_category.is_tag_section() {
                    MillerAction::LoadGenreTracksForMiller { album_key: key, replace_child: true }
                } else {
                    MillerAction::LoadAlbumTracksForMiller { album_key: key, replace_child: true }
                };
                vec![action.into()]
            }
        }
        BrowseItem::Track { .. } => {
            // Tracks are leaves: clicking opens the derived
            // track-details pane on the now-focused row. The
            // OpenTrackDetails action carries no payload because
            // the pane reads from `focused_track()`.
            is_drill_action = false;
            vec![BrowseAction::OpenTrackDetails.into()]
        }
        BrowseItem::Playlist { key, .. } => {
            vec![MillerAction::LoadPlaylistTracksForMiller { playlist_key: key, replace_child: true }.into()]
        }
        BrowseItem::Genre { key, .. } => {
            vec![MillerAction::LoadGenreAlbumsForMiller { genre_key: key, replace_child: true }.into()]
        }
        BrowseItem::AllTracks { scope, .. } => {
            match scope {
                AllTracksScope::Library => {
                    state.library.selected_album_title = "All Tracks".to_string();
                    vec![MillerAction::LoadAllLibraryTracksForMiller { replace_child: true }.into()]
                }
                AllTracksScope::AllCompilations => {
                    state.library.selected_album_title = "All Tracks".to_string();
                    vec![MillerAction::LoadAllCompilationTracksForMiller { replace_child: true }.into()]
                }
                AllTracksScope::CompilationsByArtist { artist_key, artist_name } => {
                    vec![MillerAction::LoadCompilationAllTracksForMiller {
                        artist_key,
                        artist_name,
                        replace_child: true,
                    }.into()]
                }
                AllTracksScope::Artist { artist_key, artist_name } => {
                    state.library.selected_album_title = format!("All tracks by {}", artist_name);
                    vec![MillerAction::LoadArtistAllTracksForMiller { artist_key, replace_child: true }.into()]
                }
            }
        }
        BrowseItem::AllArtists => {
            vec![MillerAction::LoadAllAlbumsForMiller { replace_child: true }.into()]
        }
        BrowseItem::Compilations => {
            vec![MillerAction::LoadCompilationsForMiller { replace_child: true }.into()]
        }
        BrowseItem::CompilationTracks { artist_key, artist_name } => {
            vec![MillerAction::LoadCompilationAlbumsForMiller {
                artist_key,
                artist_name,
                replace_child: true,
            }.into()]
        }
        BrowseItem::ArtistRadio { artist_key, artist_name, .. } => {
            is_drill_action = false;
            vec![RadioAction::StartPlexRadio {
                key: artist_key,
                title: artist_name,
            }.into()]
        }
        BrowseItem::GenreCategory { .. } => {
            // Tag-style drilling no longer goes through a category
            // column — clicking a category row is a no-op.
            Vec::new()
        }
    };

    DrillPlan {
        did_drill: local_push_drill || (is_drill_action && !actions.is_empty()),
        actions,
    }
}

fn empty() -> DrillPlan {
    DrillPlan { actions: Vec::new(), did_drill: false }
}

/// Build the local "tracks for this grouped-album row" column for
/// the Playlists category. Delegates to the long-lived helper used
/// by the keyboard drill paths so behaviour stays in sync.
fn grouped_album_drill(col: &BrowseColumn, item_index: usize) -> Option<BrowseColumn> {
    crate::app::handlers::helpers::drill_grouped_album(col, item_index)
}
