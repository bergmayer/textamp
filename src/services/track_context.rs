//! Shared track context-menu / palette entry list.
//!
//! Single source of truth for "what actions should be offered when
//! the user invokes the contextual menu on a track row?" Both the
//! TUI command palette and the GUI right-click context menu consume
//! the [`ContextEntry`] list returned here. Adding, reordering, or
//! gating an entry is a one-line edit in this file that flows to
//! both front-ends without duplication.
//!
//! Returns a UI-neutral list — labels + a [`ContextKind`]
//! discriminator. Each renderer pattern-matches the kind and emits
//! its own dispatch shape:
//!   - GUI: `context_menu::Entry::Entry { actions: vec![Action::…] }`
//!     for direct dispatch, or `Entry::Custom { message: GuiMessage::… }`
//!     for popup widgets the GUI maintains separately.
//!   - TUI palette: `PaletteEntry { command: PaletteCommandKind::… }`,
//!     where each command kind resolves to the same dispatch path on
//!     execute.
//!
//! The kinds are intentionally semantic (`PlayTrack`,
//! `OpenInLibrary`, `ShowSimilarTracks`, …) rather than
//! action-typed — that keeps each UI free to map them to whatever
//! widget is most native (popup overlay vs. view switch, for
//! example) without the shared module needing to know which.

use crate::app::state::{AppState, BrowseCategory, View};
use crate::plex::models::Track;
use crate::services::external_search::SearchTarget;

/// One row in a contextual entry list. Renderers consume this.
#[derive(Debug, Clone)]
pub struct ContextEntry {
    /// User-visible label.
    pub label: String,
    /// Optional shortcut hint (TUI palette renders this on the right).
    pub hint: Option<String>,
    /// What this row does, in UI-agnostic terms. Renderers translate
    /// the kind into their UI's dispatch shape.
    pub kind: ContextKind,
}

/// Semantic action that a [`ContextEntry`] represents. Both UIs
/// pattern-match on this enum to produce their own dispatch shape;
/// each variant carries any track-specific data the dispatch needs
/// (rating keys, titles, etc.) so callers don't have to re-derive it
/// from the original `Track`.
#[derive(Debug, Clone)]
pub enum ContextKind {
    /// Visual separator. UIs render a divider; activate is a no-op.
    Separator,

    /// Play this single track now (replace queue with just it).
    PlayTrack,
    /// Play this track and every following track in the focused
    /// column / list. UI computes the "following" set at dispatch
    /// time from its current focused list (because that depends on
    /// which column is focused, not on the track itself).
    PlayTrackAndFollowing,
    /// Insert this track into the queue right after the now-playing
    /// track (next-up).
    PlayNextInQueue,
    /// Append this track to the end of the queue.
    AddToEndOfQueue,

    /// Switch to the Library category and drill into this track's
    /// artist + album. Hidden when the track is already shown in its
    /// natural Miller chain.
    OpenInLibrary,

    /// Show sonically-similar tracks for this rating key.
    ShowSimilarTracks { rating_key: String, title: String },
    /// Show sonically-similar albums for this album rating key.
    ShowSimilarAlbums { rating_key: String, title: String },
    /// Show related artists for this artist rating key.
    ShowRelatedArtists { artist_key: String, title: String },

    /// Open the Sonic Adventure launcher pre-seeded with this track
    /// as the starting song.
    SonicAdventure,
    /// Show the artist biography for this track's artist.
    ArtistBio { artist_key: String, artist_name: String },

    /// Open the system browser to search the named external service
    /// for this track. UIs build the query string from current state
    /// at dispatch time.
    SearchExternal(SearchTarget),
}

/// Produce the contextual entry list for a track.
///
/// `floating = true` for tracks that aren't anchored in the user's
/// current Miller-drill context (e.g. similar-track rows in the
/// track-details pane, search popup hits). Floating tracks always
/// surface "Open in Library" near the top regardless of the active
/// view category. Floating also drops "Play track and following" —
/// floating rows aren't part of an ordered list to follow.
pub fn track_context_entries(
    state: &AppState,
    track: &Track,
    floating: bool,
) -> Vec<ContextEntry> {
    let mut out: Vec<ContextEntry> = Vec::new();

    let track_key = track.rating_key.clone();
    let track_label = format!("{} - {}", track.artist_name(), track.title);
    let album_key = track.parent_rating_key.clone();
    let album_title = track.parent_title.clone();
    let artist_key = track.grandparent_rating_key.clone();
    let artist_name = track.artist_name().to_string();

    // 1. Playback actions.
    out.push(ContextEntry { label: "Play track".to_string(), hint: None, kind: ContextKind::PlayTrack });
    if !floating {
        out.push(ContextEntry {
            label: "Play track and following".to_string(),
            hint: None,
            kind: ContextKind::PlayTrackAndFollowing,
        });
    }
    out.push(ContextEntry {
        label: "Play next in queue".to_string(),
        hint: None,
        kind: ContextKind::PlayNextInQueue,
    });
    out.push(ContextEntry {
        label: "Add to end of queue".to_string(),
        hint: None,
        kind: ContextKind::AddToEndOfQueue,
    });

    // 2. "Open in Library" — only when the track lives OUTSIDE its
    //    natural artist → album drill. Always shown for floating
    //    tracks; shown in non-library views (Folders, Playlists,
    //    Queue, Now Playing, Search); hidden in Library + tag-style
    //    sections where the track is already anchored.
    let in_library_context = !floating
        && state.view == View::Browse
        && (state.browse_category == BrowseCategory::Library
            || state.browse_category.is_tag_section());
    if !in_library_context && artist_key.is_some() {
        out.push(ContextEntry {
            label: "Open in Library".to_string(),
            hint: Some("^J".to_string()),
            kind: ContextKind::OpenInLibrary,
        });
    }

    out.push(ContextEntry { label: String::new(), hint: None, kind: ContextKind::Separator });

    // 3. Similar / related explorers.
    out.push(ContextEntry {
        label: "Show Similar Tracks".to_string(),
        hint: None,
        kind: ContextKind::ShowSimilarTracks {
            rating_key: track_key.clone(),
            title: track_label.clone(),
        },
    });
    if let (Some(ak), Some(at)) = (album_key.clone(), album_title.clone()) {
        if !at.is_empty() {
            out.push(ContextEntry {
                label: "Show Similar Albums".to_string(),
                hint: None,
                kind: ContextKind::ShowSimilarAlbums { rating_key: ak, title: at },
            });
        }
    }
    if let Some(ak) = artist_key.clone() {
        if !artist_name.is_empty() {
            out.push(ContextEntry {
                label: "Related Artists".to_string(),
                hint: None,
                kind: ContextKind::ShowRelatedArtists { artist_key: ak, title: artist_name.clone() },
            });
        }
    }

    // 4. Sonic Adventure.
    out.push(ContextEntry {
        label: "Sonic Adventure\u{2026}".to_string(),
        hint: None,
        kind: ContextKind::SonicAdventure,
    });

    // 5. Artist Bio.
    if let Some(ak) = artist_key {
        if !artist_name.is_empty() {
            out.push(ContextEntry { label: String::new(), hint: None, kind: ContextKind::Separator });
            out.push(ContextEntry {
                label: "Show Artist Bio".to_string(),
                hint: Some("F4".to_string()),
                kind: ContextKind::ArtistBio { artist_key: ak, artist_name: artist_name.clone() },
            });
        }
    }

    // 6. External-search services. Only the services the user has
    //    enabled in Settings appear.
    let mut sep_added = false;
    let mut push_external = |out: &mut Vec<ContextEntry>, label: &str, target: SearchTarget| {
        if !sep_added {
            out.push(ContextEntry { label: String::new(), hint: None, kind: ContextKind::Separator });
            sep_added = true;
        }
        out.push(ContextEntry {
            label: format!("Search {label} for selection"),
            hint: None,
            kind: ContextKind::SearchExternal(target),
        });
    };
    if state.external_search.apple_music {
        push_external(&mut out, "Apple Music", SearchTarget::AppleMusic);
    }
    if state.external_search.spotify {
        push_external(&mut out, "Spotify", SearchTarget::Spotify);
    }
    if state.external_search.youtube {
        push_external(&mut out, "YouTube", SearchTarget::YouTube);
    }

    out
}
