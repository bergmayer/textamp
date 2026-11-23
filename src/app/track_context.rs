//! Shared navigation and command policy for application frontends.
use crate::app::state::{AppState, View};
use crate::library::models::Track;
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
    /// Save a complete local copy for offline playback.
    Download,
    /// Play this track and every following track in the focused
    /// column / list. UI computes the "following" set at dispatch
    /// time from its current focused list (because that depends on
    /// which column is focused, not on the track itself).
    PlayTrackAndFollowing,
    PlayFollowingNext,
    AddFollowing,
    /// Insert this track into the queue right after the now-playing
    /// track (next-up).
    PlayNextInQueue,
    /// Append this track to the end of the queue.
    AddToEndOfQueue,
    AddToPlaylist,

    /// Reveal the selected track in its library detail column.
    OpenInLibrary,

    ShowDetails(crate::app::library_details::Destination),

    /// Open the Journey launcher pre-seeded with this track
    /// as the starting song.
    SonicAdventure,
    /// Continue sonic radio from this track (preserving it if already playing).
    SonicRadio,
    /// Show the artist biography for this track's artist.
    ArtistBio {
        artist_key: String,
        artist_name: String,
    },

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
#[derive(Clone, Copy, Default)]
pub struct ContextPolicy {
    pub queued: bool,
    pub server_playlists: bool,
    pub sonic: bool,
    pub external: crate::app::state::ExternalSearchSettings,
}
impl ContextPolicy {
    pub fn for_state(state: &AppState) -> Self {
        Self {
            server_playlists: state.sources.active.navidrome().is_some()
                && !state.downloads.is_offline(),
            queued: matches!(state.view, View::Queue | View::NowPlaying)
                || state.palette.player_context,
            sonic: crate::app::sources::sonic::enabled(state),
            external: state.external_search,
        }
    }
}
pub fn track_context_entries(state: &AppState, track: &Track, floating: bool) -> Vec<ContextEntry> {
    track_context_with_policy(ContextPolicy::for_state(state), track, floating)
}
/// Same command catalog for native menus without copying provider policy into a UI.
pub fn track_context_with_policy(
    policy: ContextPolicy,
    track: &Track,
    floating: bool,
) -> Vec<ContextEntry> {
    let mut out: Vec<ContextEntry> = Vec::new();

    if policy.server_playlists && track.rating_key.starts_with("navidrome:") {
        out.push(ContextEntry {
            label: "Add Track to Playlist…".into(),
            hint: None,
            kind: ContextKind::AddToPlaylist,
        });
    }

    let separator = || ContextEntry {
        label: String::new(),
        hint: None,
        kind: ContextKind::Separator,
    };
    if !out.is_empty() {
        out.push(separator());
    }
    // 1. Playback actions.
    out.push(ContextEntry {
        label: "Play track".to_string(),
        hint: None,
        kind: ContextKind::PlayTrack,
    });
    if !floating && !policy.queued {
        out.push(ContextEntry {
            label: "Play track and following".to_string(),
            hint: None,
            kind: ContextKind::PlayTrackAndFollowing,
        });
    }
    if !policy.queued {
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
    }
    if !floating && !policy.queued {
        out.push(ContextEntry {
            label: "Play track and following next".into(),
            hint: None,
            kind: ContextKind::PlayFollowingNext,
        });
        out.push(ContextEntry {
            label: "Add track and following to queue".into(),
            hint: None,
            kind: ContextKind::AddFollowing,
        });
    }
    out.push(separator());
    out.push(ContextEntry {
        label: "Download".into(),
        hint: None,
        kind: ContextKind::Download,
    });
    out.push(separator());
    out.push(ContextEntry {
        label: "Open in Library".into(),
        hint: Some("^J".into()),
        kind: ContextKind::OpenInLibrary,
    });

    out.push(ContextEntry {
        label: String::new(),
        hint: None,
        kind: ContextKind::Separator,
    });

    // 4. Track-seeded sonic playback. Shared by palette and right-click.
    out.push(ContextEntry {
        label: "Start Sonic Mix based on track".into(),
        hint: None,
        kind: ContextKind::SonicRadio,
    });
    out.push(ContextEntry {
        label: "Journey\u{2026}".to_string(),
        hint: None,
        kind: ContextKind::SonicAdventure,
    });

    // 6. External-search services. Only the services the user has
    //    enabled in Settings appear.
    let mut sep_added = false;
    let mut push_external = |out: &mut Vec<ContextEntry>, label: &str, target: SearchTarget| {
        if !sep_added {
            out.push(ContextEntry {
                label: String::new(),
                hint: None,
                kind: ContextKind::Separator,
            });
            sep_added = true;
        }
        out.push(ContextEntry {
            label: format!("Search {label} for selection"),
            hint: None,
            kind: ContextKind::SearchExternal(target),
        });
    };
    if policy.external.apple_music {
        push_external(&mut out, "Apple Music", SearchTarget::AppleMusic);
    }
    if policy.external.spotify {
        push_external(&mut out, "Spotify", SearchTarget::Spotify);
    }
    if policy.external.youtube {
        push_external(&mut out, "YouTube", SearchTarget::YouTube);
    }

    out.retain(|entry| {
        policy.sonic
            || !matches!(
                entry.kind,
                ContextKind::SonicAdventure | ContextKind::SonicRadio
            )
    });
    out.dedup_by(|a, b| {
        matches!(a.kind, ContextKind::Separator) && matches!(b.kind, ContextKind::Separator)
    });
    if out
        .last()
        .is_some_and(|e| matches!(e.kind, ContextKind::Separator))
    {
        out.pop();
    }
    out
}

/// Compare known artist identities without treating missing metadata as a match.
pub fn same_artist(left: &Track, right: &Track) -> bool {
    if let (Some(left), Some(right)) = (
        left.grandparent_rating_key
            .as_deref()
            .filter(|s| !s.is_empty()),
        right
            .grandparent_rating_key
            .as_deref()
            .filter(|s| !s.is_empty()),
    ) && left == right
    {
        return true;
    }
    let left = left.track_artist().trim();
    let right = right.track_artist().trim();
    !left.is_empty()
        && !right.is_empty()
        && !left.eq_ignore_ascii_case("Unknown Artist")
        && left.eq_ignore_ascii_case(right)
}

/// Actions for the currently playing occurrence, excluding playback restarts.
pub fn now_playing_context(policy: ContextPolicy, track: &Track) -> Vec<ContextEntry> {
    let entries = track_context_with_policy(
        ContextPolicy {
            queued: true,
            ..policy
        },
        track,
        false,
    )
    .into_iter()
    .filter(|entry| !matches!(entry.kind, ContextKind::PlayTrack));
    let mut normalized = Vec::new();
    for entry in entries {
        if matches!(entry.kind, ContextKind::Separator)
            && normalized
                .last()
                .is_none_or(|entry: &ContextEntry| matches!(entry.kind, ContextKind::Separator))
        {
            continue;
        }
        normalized.push(entry);
    }
    if normalized
        .last()
        .is_some_and(|entry| matches!(entry.kind, ContextKind::Separator))
    {
        normalized.pop();
    }
    normalized
}
