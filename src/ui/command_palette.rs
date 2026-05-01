//! Neovim-style command-palette overlay for the TUI.
//!
//! Open with `:` from any view. Type to fuzzy-search; Up/Down to
//! navigate; Enter to execute the selected entry; Esc to abort.
//!
//! The candidate list is rebuilt on every keystroke from a static
//! registry of built-in commands plus runtime content (radio
//! stations) so the user can launch any radio station by typing its
//! name into the same overlay used for keyboard shortcuts.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

use crate::app::action::{Action, BrowseAction, MillerAction, NavigationAction, PlaybackAction, QueueAction, RadioAction, SearchAction, SettingsAction, SystemAction};
use crate::app::state::{AppState, BrowseCategory, ColumnSortMode, DjMode, PaletteCommandKind, PaletteEntry, RefreshCategory, View};
use crate::ui::theme::theme;
use rand::seq::IteratorRandom;

/// Built-in commands that don't depend on runtime data.
struct StaticEntry {
    label: &'static str,
    hint: &'static str,
    command: PaletteCommandKind,
}

fn static_entries() -> Vec<StaticEntry> {
    vec![
        StaticEntry { label: "Library",        hint: "^L",  command: PaletteCommandKind::GotoLibrary },
        StaticEntry { label: "Genres",         hint: "^G",  command: PaletteCommandKind::GotoGenres },
        StaticEntry { label: "Folders",        hint: "^O",  command: PaletteCommandKind::GotoFolders },
        StaticEntry { label: "Now Playing",    hint: "^N",  command: PaletteCommandKind::GotoNowPlaying },
        StaticEntry { label: "Help",           hint: "F1",  command: PaletteCommandKind::OpenHelp },
        StaticEntry { label: "Settings",       hint: "F2",  command: PaletteCommandKind::OpenSettings },
        StaticEntry { label: "Refresh",        hint: "F5",  command: PaletteCommandKind::Refresh },
        StaticEntry { label: "Find / Search",  hint: "^F",  command: PaletteCommandKind::OpenSearch },
        StaticEntry { label: "Filter",         hint: "/",   command: PaletteCommandKind::ToggleFilter },
        StaticEntry { label: "Similar",        hint: "^M",  command: PaletteCommandKind::OpenSimilar },
        StaticEntry { label: "Related",        hint: "^R",  command: PaletteCommandKind::OpenRelated },
        StaticEntry { label: "Save Queue",     hint: "^S",  command: PaletteCommandKind::SaveQueue },
        StaticEntry { label: "Clear Queue",    hint: "^X",  command: PaletteCommandKind::ClearQueue },
        StaticEntry { label: "Play / Pause",   hint: "Spc", command: PaletteCommandKind::PlayPause },
        StaticEntry { label: "Next Track",     hint: ">>",  command: PaletteCommandKind::NextTrack },
        StaticEntry { label: "Previous Track", hint: "<<",  command: PaletteCommandKind::PrevTrack },
        StaticEntry { label: "Random Album",       hint: "",    command: PaletteCommandKind::RandomAlbum },
        StaticEntry { label: "Open in Library",    hint: "^J",  command: PaletteCommandKind::OpenInLibrary },
        StaticEntry { label: "Sonic Adventure\u{2026}", hint: "", command: PaletteCommandKind::SonicAdventure },
        // Mirror the Now Playing sidebar buttons so the palette is a
        // first-class alternative to the GUI sidebar. Each maps to
        // either an existing PaletteCommandKind (Goto*, ClearQueue)
        // or to "Now Playing" itself (the TUI surfaces stations in
        // that view's sidebar — there's no separate popup like the
        // GUI). Each individual DJ / Remix entry is still listed
        // below for direct access.
        StaticEntry { label: "Radio",          hint: "",    command: PaletteCommandKind::GotoNowPlaying },
        StaticEntry { label: "DJ Modes",       hint: "",    command: PaletteCommandKind::GotoNowPlaying },
        StaticEntry { label: "Remix Tools",    hint: "",    command: PaletteCommandKind::GotoNowPlaying },

        // Sort cluster — typing "sort" in the palette surfaces all
        // of these together: the umbrella popup entry plus each
        // individual sort mode and the toggles. Labels share the
        // "Sort: …" prefix so fuzzy matching keeps them grouped.
        StaticEntry { label: "View Options",               hint: "^V", command: PaletteCommandKind::OpenSort },
        StaticEntry { label: "Close Column",               hint: "^W", command: PaletteCommandKind::CloseColumn },
        StaticEntry { label: "Sort: Default",              hint: "",   command: PaletteCommandKind::ApplySort(ColumnSortMode::Default) },
        StaticEntry { label: "Sort: By Artist",            hint: "",   command: PaletteCommandKind::ApplySort(ColumnSortMode::ByArtist) },
        StaticEntry { label: "Sort: By Album",             hint: "",   command: PaletteCommandKind::ApplySort(ColumnSortMode::ByAlbum) },
        StaticEntry { label: "Sort: By Title",             hint: "",   command: PaletteCommandKind::ApplySort(ColumnSortMode::ByTitle) },
        StaticEntry { label: "Sort: By Duration",          hint: "",   command: PaletteCommandKind::ApplySort(ColumnSortMode::ByDuration) },
        StaticEntry { label: "Sort: Shuffle",              hint: "",   command: PaletteCommandKind::ApplySort(ColumnSortMode::Shuffled) },
        StaticEntry { label: "Sort: Reverse Direction",    hint: "",   command: PaletteCommandKind::ReverseSort },
        StaticEntry { label: "Sort: Toggle Group by Album", hint: "",  command: PaletteCommandKind::ToggleGroupByAlbum },
        StaticEntry { label: "Sort: Toggle Cover Art",     hint: "",   command: PaletteCommandKind::ToggleArtwork },

        StaticEntry { label: "Quit",           hint: "^Q",  command: PaletteCommandKind::Quit },

        // DJ modes — surfaced under "DJ ..." so the now-playing
        // sidebar can prefilter the palette to just these.
        StaticEntry { label: "DJ Stretch",     hint: "",    command: PaletteCommandKind::ToggleDj(DjMode::Stretch) },
        StaticEntry { label: "DJ Gemini",      hint: "",    command: PaletteCommandKind::ToggleDj(DjMode::Gemini) },
        StaticEntry { label: "DJ Freeze",      hint: "",    command: PaletteCommandKind::ToggleDj(DjMode::Freeze) },
        StaticEntry { label: "DJ Twofer",      hint: "",    command: PaletteCommandKind::ToggleDj(DjMode::Twofer) },
        StaticEntry { label: "DJ Contempo",    hint: "",    command: PaletteCommandKind::ToggleDj(DjMode::Contempo) },
        StaticEntry { label: "DJ Groupie",     hint: "",    command: PaletteCommandKind::ToggleDj(DjMode::Groupie) },

        // Remix tools.
        StaticEntry { label: "Remix: Gemini",        hint: "", command: PaletteCommandKind::RemixGemini },
        StaticEntry { label: "Remix: Twofer",        hint: "", command: PaletteCommandKind::RemixTwofer },
        StaticEntry { label: "Remix: Stretch",       hint: "", command: PaletteCommandKind::RemixStretch },
        StaticEntry { label: "Remix: Doppelganger",  hint: "", command: PaletteCommandKind::RemixDoppelganger },
        StaticEntry { label: "Remix: Shuffle",       hint: "", command: PaletteCommandKind::RemixShuffle },
        StaticEntry { label: "Remix: Undo Shuffle",  hint: "", command: PaletteCommandKind::RemixUndoShuffle },
    ]
}

/// Build the materialized entry list for the current `state`.
///
/// Order, top to bottom:
///   1. **Context-aware** — actions that depend on what's currently
///      focused (e.g. "Play Track" / "Play Track and Following" when
///      a track row is selected). These come first so the user can
///      hit the most relevant action with a single Enter when they
///      pop the palette.
///   2. **External search** — Apple Music / Spotify / YouTube. Only
///      surfaced when there is something to search for (selection
///      or now-playing track produces a non-empty query).
///   3. **Static registry** — the global commands (Library, Queue,
///      Help, Settings, Quit, Remix tools, etc.).
///   4. **Radio stations** — every loaded station as a plain row.
pub fn materialize_entries(state: &AppState) -> Vec<PaletteEntry> {
    let mut out: Vec<PaletteEntry> = Vec::new();

    // 1. Context-aware row entries — listed first so a single Enter
    //    on `:` performs the most relevant action for whatever's
    //    highlighted. Highlighted rows include both Miller-column
    //    items AND the Sonically-Similar list inside the track
    //    pane: when a similar row is focused, all the track-context
    //    commands target THAT track, not the parent row.
    // "In a library-style context" = the track is being shown inside a
    // proper artist → album → track Miller chain. That's true for the
    // Library category itself, AND for every tag-style section
    // (Album Genres / Artist Genres / Moods / Styles / Decades /
    // Years / Collections / Countries / Labels / Formats / Studios) —
    // each of those drills `tag → album → tracks`, so the highlighted
    // track is already next to its album+artist context. Folders and
    // Playlists list tracks without an artist+album drill, so they're
    // NOT a library context — "Open in Library" still helps the user
    // find that track's album page.
    let in_library_context = state.view == View::Browse
        && (state.browse_category == BrowseCategory::Library
            || state.browse_category.is_tag_section());
    let not_in_library = !in_library_context;
    let target_track = state.palette_target_track();
    let target_is_similar = state.palette_target_is_similar();

    if let Some(track) = target_track.clone() {
        // Build the contextual entry list from the shared
        // `track_context_entries` source so the palette stays
        // identical to the GUI right-click context menu. Adding /
        // reordering happens in
        // `crate::services::track_context::track_context_entries`.
        let entries = crate::services::track_context::track_context_entries(
            state, &track, target_is_similar,
        );
        let track_box = Box::new(track);
        for ce in entries {
            use crate::services::track_context::ContextKind;
            // The palette doesn't render visual separators (its rows
            // are a fuzzy-search list, not a structured menu).
            if matches!(ce.kind, ContextKind::Separator) {
                continue;
            }
            out.push(PaletteEntry {
                label: ce.label,
                hint: ce.hint.unwrap_or_default(),
                command: PaletteCommandKind::FromTrackContext {
                    kind: ce.kind,
                    track: track_box.clone(),
                },
            });
        }
    } else if state.focused_album().is_some() {
        out.push(PaletteEntry {
            label: "Play Album".to_string(),
            hint: String::new(),
            command: PaletteCommandKind::PlayFocusedAlbum,
        });
        if not_in_library {
            out.push(PaletteEntry {
                label: "Open in Library".to_string(),
                hint: "^J".to_string(),
                command: PaletteCommandKind::OpenInLibrary,
            });
        }
        out.push(PaletteEntry {
            label: "Artist Bio".to_string(),
            hint: "F4".to_string(),
            command: PaletteCommandKind::ShowArtistBio,
        });
    }

    // 2. External search — non-empty query required AND each service
    //    must be enabled in Settings ("Search ⟨service⟩" toggle).
    //    Disabled services are completely hidden from the palette so
    //    the toggle reads as "remove this service from the app", not
    //    just "make this entry a no-op".
    let ext_query = crate::app::handlers::key_input::build_external_search_query(state);
    if !ext_query.is_empty() {
        if state.external_search.apple_music {
            out.push(PaletteEntry {
                label: "Search Apple Music for selection".to_string(),
                hint: String::new(),
                command: PaletteCommandKind::SearchAppleMusic,
            });
        }
        if state.external_search.spotify {
            out.push(PaletteEntry {
                label: "Search Spotify for selection".to_string(),
                hint: String::new(),
                command: PaletteCommandKind::SearchSpotify,
            });
        }
        if state.external_search.youtube {
            out.push(PaletteEntry {
                label: "Search YouTube for selection".to_string(),
                hint: String::new(),
                command: PaletteCommandKind::SearchYouTube,
            });
        }
    }

    // DJ + Remix entries belong to the Now Playing context — they're
    // the palette analogue of the "DJ Modes" / "Remix Tools" sidebar
    // buttons, so they should surface any time the user is in Now
    // Playing (matching what the sidebar does). Don't gate on
    // queue.tracks.is_empty(): the queue may still be populating
    // (e.g. radio station starting) and the user typing "DJ" expects
    // results regardless.
    let queue_context = state.view == View::NowPlaying;

    // Same gate as the contextual block above — "Open in Library"
    // makes no sense from inside a library-style miller chain, but is
    // still useful from Folders / Playlists / Queue / Now Playing.
    let in_library = in_library_context;

    // 3. Static registry. Filter:
    //   - DJ + Remix entries unless `queue_context` is satisfied.
    //   - "Open in Library" when the user is already in a library-
    //     style context (the entry would just take them where they
    //     are), or when the contextual section already added one for
    //     the focused track/album (avoid duplicates).
    let context_has_open_in_library =
        (target_track.is_some() && (not_in_library || target_is_similar))
            || (state.focused_album().is_some() && not_in_library);
    out.extend(static_entries().into_iter().filter_map(|e| {
        let is_dj_or_remix = matches!(
            e.command,
            PaletteCommandKind::ToggleDj(_)
                | PaletteCommandKind::RemixGemini
                | PaletteCommandKind::RemixTwofer
                | PaletteCommandKind::RemixStretch
                | PaletteCommandKind::RemixDoppelganger
                | PaletteCommandKind::RemixShuffle
                | PaletteCommandKind::RemixUndoShuffle
        );
        if is_dj_or_remix && !queue_context {
            return None;
        }
        if matches!(e.command, PaletteCommandKind::OpenInLibrary)
            && (in_library || context_has_open_in_library)
        {
            return None;
        }
        Some(PaletteEntry {
            label: e.label.to_string(),
            hint: e.hint.to_string(),
            command: e.command,
        })
    }));

    // 4. Radio stations. The Now Playing sidebar uses the same
    // `state.stations` list but interleaves "─" separators and
    // synthetic "DJ Mode" / "Remix" rows. The palette only wants
    // playable radio stations:
    //   - Drop separator rows always (they're sidebar chrome).
    //   - Drop dj_mode / remix synthetic rows unless `queue_context`
    //     applies (same gate as the static entries above).
    for s in &state.stations {
        match s.station_type.as_str() {
            "separator" => continue,
            "dj_mode" | "remix" if !queue_context => continue,
            _ => {}
        }
        // Prefix station rows with "Radio: " so typing "Radio" in the
        // palette (e.g. via the sidebar button) surfaces every station
        // in one cluster, while still letting the user type the
        // station name directly.
        out.push(PaletteEntry {
            label: format!("Radio: {}", s.title),
            hint: String::new(),
            command: PaletteCommandKind::PlayStation {
                key: s.key.clone(),
                title: s.title.clone(),
            },
        });
    }
    out
}

/// Translate a palette command into the same `Action`s the shortcut
/// bar used to dispatch.
pub fn run(cmd: PaletteCommandKind, state: &mut AppState) -> Vec<Action> {
    match cmd {
        PaletteCommandKind::Quit            => vec![SystemAction::Quit.into()],
        PaletteCommandKind::GotoLibrary     => vec![NavigationAction::SetCategory(BrowseCategory::Library).into()],
        PaletteCommandKind::GotoGenres      => vec![NavigationAction::SetCategory(BrowseCategory::AlbumGenres).into()],
        PaletteCommandKind::GotoFolders     => vec![NavigationAction::SetCategory(BrowseCategory::Folders).into()],
        PaletteCommandKind::GotoQueue       => vec![NavigationAction::SetView(View::Queue).into()],
        PaletteCommandKind::GotoNowPlaying  => vec![NavigationAction::SetView(View::NowPlaying).into()],
        PaletteCommandKind::OpenHelp        => vec![NavigationAction::SetView(View::Help).into()],
        PaletteCommandKind::OpenSettings    => vec![SettingsAction::OpenSettings.into()],
        PaletteCommandKind::OpenSearch      => vec![SearchAction::OpenSearchPopup.into()],
        PaletteCommandKind::OpenSimilar     => crate::app::handlers::key_input::get_similar_action(state),
        PaletteCommandKind::OpenRelated     => crate::app::handlers::key_input::get_related_action(state),
        PaletteCommandKind::SaveQueue       => vec![QueueAction::PromptSavePlaylist.into()],
        PaletteCommandKind::ClearQueue      => vec![QueueAction::ClearQueue.into()],
        PaletteCommandKind::ToggleFilter    => vec![SearchAction::ActivateListFilter.into()],
        PaletteCommandKind::Refresh         => vec![SystemAction::RefreshCategory(RefreshCategory::Artists).into()],
        PaletteCommandKind::PlayPause       => vec![PlaybackAction::TogglePlayPause.into()],
        PaletteCommandKind::NextTrack       => vec![PlaybackAction::Next.into()],
        PaletteCommandKind::PrevTrack       => vec![PlaybackAction::Previous.into()],
        PaletteCommandKind::ToggleDj(mode)  => vec![RadioAction::ToggleDjMode(mode).into()],
        PaletteCommandKind::RemixGemini     => vec![QueueAction::RemixGemini.into()],
        PaletteCommandKind::RemixTwofer     => vec![QueueAction::RemixTwofer.into()],
        PaletteCommandKind::RemixStretch    => vec![QueueAction::RemixStretch.into()],
        PaletteCommandKind::RemixDoppelganger => vec![QueueAction::RemixDoppelganger.into()],
        PaletteCommandKind::RemixShuffle    => vec![QueueAction::RemixShuffle.into()],
        PaletteCommandKind::RemixUndoShuffle => vec![QueueAction::RemixUndoShuffle.into()],
        PaletteCommandKind::PlayStation { key, title } => {
            vec![RadioAction::StartPlexRadio { key, title }.into()]
        }
        PaletteCommandKind::RandomAlbum => {
            // Mirror the GUI's `PlayOneRandomAlbum`: pick one album
            // at random from the active library and dispatch
            // PlayAlbumNow. Falls back to a no-op error when the
            // album list isn't loaded yet.
            let pick = state.library.albums.iter()
                .choose(&mut rand::thread_rng())
                .map(|a| (a.rating_key.clone(), a.title.clone()));
            match pick {
                Some((rating_key, title)) =>
                    vec![QueueAction::PlayAlbumNow { rating_key, title }.into()],
                None => {
                    state.set_error("No albums in library to pick from".to_string());
                    vec![]
                }
            }
        }
        PaletteCommandKind::OpenInLibrary => {
            use crate::app::state::BrowseItem;

            // Resolve target in priority order:
            //   1. Highlighted Sonically-Similar row in the focused
            //      track-details pane (lets the user one-press jump
            //      from a track's pane to one of its similar tracks
            //      in Library — "show me where THAT track lives").
            //   2. Focused track row  (uses parent/grandparent keys)
            //   3. Focused album row  (looks up artist via library)
            //   4. Now-playing track  (legacy fallback)

            // 1. Track-details pane → highlighted similar track.
            if state.track_pane_focused && state.track_pane_index > 0 {
                let parent_track = state.focused_track().cloned();
                let sim_idx = state.track_pane_index - 1;
                if let Some(parent) = parent_track {
                    if let Some(sim) = state
                        .track_pane_similar
                        .get(&parent.rating_key)
                        .and_then(|v| v.get(sim_idx))
                        .cloned()
                    {
                        if let Some(artist_key) = sim.grandparent_rating_key.clone() {
                            return vec![BrowseAction::OpenInLibrary {
                                artist_key,
                                artist_name: sim.track_artist().to_string(),
                                album_key: sim.parent_rating_key.clone(),
                                album_title: sim.parent_title.clone(),
                            }
                            .into()];
                        }
                    }
                }
            }

            // 2. Focused track row.
            if let Some(track) = state.focused_track().cloned() {
                if let Some(artist_key) = track.grandparent_rating_key.clone() {
                    return vec![BrowseAction::OpenInLibrary {
                        artist_key,
                        artist_name: track.track_artist().to_string(),
                        album_key: track.parent_rating_key.clone(),
                        album_title: track.parent_title.clone(),
                    }
                    .into()];
                }
            }

            // 2. Focused album row.
            let focused_album_item = state
                .browse_nav()
                .and_then(|n| n.columns.get(n.focused_column))
                .and_then(|c| c.items.get(c.selected_index).cloned());
            if let Some(BrowseItem::Album {
                key: album_key,
                title: album_title,
                artist,
                ..
            }) = focused_album_item
            {
                let artist_key = state
                    .library
                    .albums
                    .iter()
                    .chain(state.library.tag_albums.iter())
                    .chain(state.library.selected_artist_albums.iter())
                    .find(|a| a.rating_key == album_key)
                    .and_then(|a| a.parent_rating_key.clone());
                if let Some(artist_key) = artist_key {
                    return vec![BrowseAction::OpenInLibrary {
                        artist_key,
                        artist_name: artist,
                        album_key: Some(album_key),
                        album_title: Some(album_title),
                    }
                    .into()];
                }
            }

            // 3. Now-playing fallback.
            let Some(track) = state.current_track() else {
                state.set_status("Nothing to open in Library".to_string());
                return vec![];
            };
            let Some(artist_key) = track.grandparent_rating_key.clone() else {
                state.set_status("Track has no artist on Plex".to_string());
                return vec![];
            };
            vec![BrowseAction::OpenInLibrary {
                artist_key,
                artist_name: track.track_artist().to_string(),
                album_key: track.parent_rating_key.clone(),
                album_title: track.parent_title.clone(),
            }
            .into()]
        }
        PaletteCommandKind::OpenSort => vec![SearchAction::OpenSortPopup.into()],
        PaletteCommandKind::ToggleArtwork => {
            // Toggle on the focused Miller column. If we're on the
            // category column or there's no nav, no-op.
            let col_idx = state.browse_nav().map(|n| n.focused_column).unwrap_or(0);
            crate::app::handlers::key_input::sort_popup::toggle_artwork(state, col_idx)
        }
        PaletteCommandKind::ToggleGroupByAlbum => {
            let col_idx = state.browse_nav().map(|n| n.focused_column).unwrap_or(0);
            crate::app::handlers::key_input::sort_popup::toggle_group_by_album(state, col_idx)
        }
        PaletteCommandKind::PlayFocusedTrack => {
            // Highlighted Sonically-Similar row → play it as a
            // single track (no "and following" — it's a free-floating
            // recommendation, not a list with order). Otherwise
            // fall through to the miller-column list-aware dispatch.
            if state.palette_target_is_similar() {
                if let Some(track) = state.palette_target_track() {
                    return vec![QueueAction::PlayTrack(track).into()];
                }
                return vec![];
            }
            play_focused_track(state, true)
        }
        PaletteCommandKind::PlayFocusedTrackAndFollowing => {
            play_focused_track(state, false)
        }
        PaletteCommandKind::SearchAppleMusic => {
            vec![SystemAction::OpenExternalSearch {
                target: crate::services::external_search::SearchTarget::AppleMusic,
                query: None,
            }.into()]
        }
        PaletteCommandKind::SearchSpotify => {
            vec![SystemAction::OpenExternalSearch {
                target: crate::services::external_search::SearchTarget::Spotify,
                query: None,
            }.into()]
        }
        PaletteCommandKind::SearchYouTube => {
            vec![SystemAction::OpenExternalSearch {
                target: crate::services::external_search::SearchTarget::YouTube,
                query: None,
            }.into()]
        }
        PaletteCommandKind::PlayFocusedAlbum => {
            match state.focused_album() {
                Some((rating_key, title)) =>
                    vec![QueueAction::PlayAlbumNow { rating_key, title }.into()],
                None => {
                    state.set_status("No album focused".to_string());
                    vec![]
                }
            }
        }
        PaletteCommandKind::ShowArtistBio => {
            match crate::app::handlers::helpers::get_artist_for_bio(state) {
                Some((artist_key, artist_name)) =>
                    vec![SearchAction::ShowArtistBio { artist_key, artist_name }.into()],
                None => {
                    state.set_status("No artist context for bio".to_string());
                    vec![]
                }
            }
        }
        PaletteCommandKind::ApplySort(mode) =>
            vec![SearchAction::ApplyFocusedSortMode(mode).into()],
        PaletteCommandKind::ReverseSort =>
            vec![SearchAction::ReverseFocusedSortDirection.into()],
        PaletteCommandKind::CloseColumn => {
            crate::app::handlers::key_input::close_focused_browse_column(state);
            vec![]
        }
        PaletteCommandKind::SonicAdventureFromFocusedTrack => {
            match state.palette_target_track() {
                Some(track) => vec![SearchAction::OpenAdventureLauncherWithStart {
                    start_track: Box::new(track),
                }.into()],
                None => {
                    state.set_status("No track focused".to_string());
                    vec![]
                }
            }
        }
        PaletteCommandKind::SonicAdventure => {
            vec![SearchAction::OpenAdventureLauncher.into()]
        }
        PaletteCommandKind::FromTrackContext { kind, track } => {
            // Translate the shared `ContextKind` into the right
            // dispatch shape. Mirrors the GUI's
            // `build_track_context_menu_inner`. Whenever an entry
            // gets added or reordered in
            // `services::track_context::track_context_entries`,
            // both UIs pick it up automatically — the only
            // per-UI work is this kind-to-dispatch table.
            use crate::app::action::{BrowseAction, DataAction, NavigationAction, QueueAction, SearchAction, SystemAction};
            use crate::app::state::{SimilarMode, View};
            use crate::services::track_context::ContextKind;
            match kind {
                ContextKind::Separator => vec![],
                ContextKind::PlayTrack => vec![QueueAction::PlayTrack(*track).into()],
                ContextKind::PlayTrackAndFollowing => play_focused_track(state, false),
                ContextKind::PlayNextInQueue =>
                    vec![QueueAction::EnqueueTracksNext(vec![*track]).into()],
                ContextKind::AddToEndOfQueue =>
                    vec![QueueAction::EnqueueTrack(*track).into()],
                ContextKind::OpenInLibrary => {
                    if let Some(artist_key) = track.grandparent_rating_key.clone() {
                        vec![BrowseAction::OpenInLibrary {
                            artist_key,
                            artist_name: track.artist_name().to_string(),
                            album_key: track.parent_rating_key.clone(),
                            album_title: track.parent_title.clone(),
                        }.into()]
                    } else {
                        state.set_status("Track has no artist key".to_string());
                        vec![]
                    }
                }
                ContextKind::SonicAdventure =>
                    vec![SearchAction::OpenAdventureLauncherWithStart { start_track: track }.into()],
                ContextKind::ArtistBio { artist_key, artist_name } =>
                    vec![SearchAction::ShowArtistBio { artist_key, artist_name }.into()],
                ContextKind::SearchExternal(target) =>
                    vec![SystemAction::OpenExternalSearch { target, query: None }.into()],
                ContextKind::ShowSimilarTracks { rating_key, title } => {
                    state.similar.mode = SimilarMode::Tracks;
                    state.similar.source_title = title.clone();
                    vec![
                        DataAction::LoadSimilarTracks { rating_key, title }.into(),
                        NavigationAction::SetView(View::Similar).into(),
                    ]
                }
                ContextKind::ShowSimilarAlbums { rating_key, title } => {
                    state.similar.mode = SimilarMode::Albums;
                    state.similar.source_title = title.clone();
                    vec![
                        DataAction::LoadSimilarAlbums { rating_key, title }.into(),
                        NavigationAction::SetView(View::Similar).into(),
                    ]
                }
                ContextKind::ShowRelatedArtists { artist_key, title } => {
                    vec![
                        DataAction::LoadRelated { artist_key, title }.into(),
                        NavigationAction::SetView(View::Related).into(),
                    ]
                }
            }
        }
    }
}

/// Translate "Play Track" / "Play Track and Following" into the
/// right `MillerAction` for the current browse category. Mirrors the
/// branching the GUI's right-click context menu uses.
///
/// Special case: when the palette target is a Sonically Similar row
/// inside the track pane (rather than a focused miller row), the
/// list itself acts as a "virtual album" — Play Track plays just
/// that similar track, Play Track and Following queues the similar
/// track and every later row in the pane's list. The same shape
/// applies to any future virtual track list (search hits, related
/// artist tracks…) — extend `palette_virtual_list` as new lists
/// land.
fn play_focused_track(state: &AppState, single: bool) -> Vec<Action> {
    use crate::app::state::BrowseCategory;
    use crate::app::action::QueueAction;

    if state.palette_target_is_similar() {
        // Pane similar list → virtual queue. parent_track holds the
        // list keyed by `parent.rating_key`; the highlighted similar
        // row sits at `track_pane_index - 1`.
        let Some(parent) = state.focused_track() else { return vec![] };
        let Some(list) = state.track_pane_similar.get(&parent.rating_key) else { return vec![] };
        let sim_idx = state.track_pane_index.saturating_sub(1);
        if sim_idx >= list.len() { return vec![] }
        let sim = &list[sim_idx];
        if single {
            return vec![QueueAction::PlayTrack(sim.clone()).into()];
        }
        let tail: Vec<_> = list[sim_idx..].iter().cloned().collect();
        return vec![QueueAction::PlayTracksNow(tail).into()];
    }

    let Some(nav) = state.browse_nav() else {
        return vec![];
    };
    let Some(col) = nav.columns.get(nav.focused_column) else {
        return vec![];
    };
    let column_index = nav.focused_column;
    let track_index = col.selected_index;
    let action = match state.browse_category {
        BrowseCategory::AlbumGenres => MillerAction::PlayGenreTrackFromMiller {
            column_index,
            track_index,
            single_track: single,
        },
        BrowseCategory::Playlists => MillerAction::PlayPlaylistTrackFromMiller {
            column_index,
            track_index,
            single_track: single,
        },
        _ => MillerAction::PlayTrackFromMiller {
            column_index,
            track_index,
            single_track: single,
        },
    };
    vec![action.into()]
}

/// What the input handler decided to do with this key.
pub enum PaletteOutcome {
    Continue,
    Execute(PaletteCommandKind),
    Cancel,
}

/// Open the palette with no query.
pub fn open(state: &mut AppState) {
    open_with_query(state, "");
}

/// Open the palette with a pre-typed query. Used by the now-playing
/// sidebar buttons.
pub fn open_with_query(state: &mut AppState, q: &str) {
    state.palette.open = true;
    state.palette.query = q.to_string();
    state.palette.cursor = q.chars().count();
    state.palette.selected = 0;
    refresh_matches(state);
}

/// Recompute `entries` (snapshot of current candidates) and `matches`
/// (filtered + sorted indices into `entries`) for the current query.
pub fn refresh_matches(state: &mut AppState) {
    let entries = materialize_entries(state);
    let query = state.palette.query.clone();
    let matches: Vec<usize> = if query.is_empty() {
        (0..entries.len()).collect()
    } else {
        let matcher = SkimMatcherV2::default();
        let mut scored: Vec<(i64, usize)> = entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| matcher.fuzzy_match(&e.label, &query).map(|s| (s, i)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.into_iter().map(|(_, i)| i).collect()
    };
    state.palette.entries = entries;
    state.palette.matches = matches;
    if state.palette.selected >= state.palette.matches.len() {
        state.palette.selected = state.palette.matches.len().saturating_sub(1);
    }
}

/// Process a single key event while the palette is open.
pub fn handle_key(state: &mut AppState, key: KeyEvent) -> PaletteOutcome {
    use KeyCode as K;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    match key.code {
        K::Esc => return PaletteOutcome::Cancel,
        K::Enter => {
            if let Some(&idx) = state.palette.matches.get(state.palette.selected) {
                if let Some(entry) = state.palette.entries.get(idx) {
                    return PaletteOutcome::Execute(entry.command.clone());
                }
            }
            return PaletteOutcome::Cancel;
        }
        K::Up => {
            state.palette.selected = state.palette.selected.saturating_sub(1);
            return PaletteOutcome::Continue;
        }
        K::Down => {
            let max = state.palette.matches.len().saturating_sub(1);
            state.palette.selected = (state.palette.selected + 1).min(max);
            return PaletteOutcome::Continue;
        }
        K::Char('p') if ctrl => {
            state.palette.selected = state.palette.selected.saturating_sub(1);
            return PaletteOutcome::Continue;
        }
        K::Char('n') if ctrl => {
            let max = state.palette.matches.len().saturating_sub(1);
            state.palette.selected = (state.palette.selected + 1).min(max);
            return PaletteOutcome::Continue;
        }
        _ => {}
    }

    let mut input = Input::new(state.palette.query.clone()).with_cursor(state.palette.cursor);
    let event = crossterm::event::Event::Key(key);
    let _ = input.handle_event(&event);
    state.palette.query = input.value().to_string();
    state.palette.cursor = input.cursor();
    state.palette.selected = 0;
    refresh_matches(state);
    PaletteOutcome::Continue
}

/// Render the palette overlay centered on `area`.
pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    if !state.palette.open {
        return;
    }
    let t = theme();

    let popup_area = centered(area, 60, 20);
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.title_focused))
        .title(" command ")
        .title_style(Style::default().fg(t.colors.title_focused))
        .style(Style::default().bg(t.colors.bg_primary).fg(t.colors.fg_primary));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(1),
            ratatui::layout::Constraint::Length(1),
            ratatui::layout::Constraint::Min(1),
        ])
        .split(inner);

    let prompt = ":";
    let input_line = Line::from(vec![
        Span::styled(prompt, Style::default().fg(t.colors.fg_accent)),
        Span::raw(" "),
        Span::raw(state.palette.query.clone()),
    ]);
    frame.render_widget(Paragraph::new(input_line), chunks[0]);
    let cursor_x = chunks[0].x + 2 + state.palette.cursor as u16;
    if cursor_x < chunks[0].x + chunks[0].width {
        frame.set_cursor_position((cursor_x, chunks[0].y));
    }

    frame.render_widget(
        Paragraph::new("─".repeat(chunks[1].width as usize)).style(Style::default().fg(t.colors.border)),
        chunks[1],
    );

    let items: Vec<ListItem> = state.palette.matches.iter()
        .filter_map(|&i| state.palette.entries.get(i))
        .map(|e| {
            let label_w = chunks[2].width.saturating_sub(e.hint.len() as u16 + 2) as usize;
            let label = if e.label.chars().count() > label_w {
                let truncated: String = e.label.chars().take(label_w.saturating_sub(1)).collect();
                format!("{}…", truncated)
            } else {
                e.label.clone()
            };
            let label_display_len = label.chars().count();
            let pad = " ".repeat(label_w.saturating_sub(label_display_len));
            ListItem::new(Line::from(vec![
                Span::raw(label),
                Span::raw(pad),
                Span::styled(e.hint.clone(), Style::default().fg(t.colors.fg_muted)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(t.colors.bg_selection)
                .fg(t.colors.selection_text)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    if !state.palette.matches.is_empty() {
        list_state.select(Some(state.palette.selected));
    }
    frame.render_stateful_widget(list, chunks[2], &mut list_state);

    // Register hit regions so mouse clicks can pick a row. Each
    // visible row maps to its `state.palette.matches` index — the
    // mouse handler bumps `state.palette.selected` to that index and
    // then runs the same Execute path Enter triggers.
    let visible = chunks[2].height as usize;
    let total_matches = state.palette.matches.len();
    // Mirror ratatui's ListState scroll: keep `selected` in view.
    let scroll = if total_matches <= visible {
        0
    } else if state.palette.selected >= visible {
        state.palette.selected + 1 - visible
    } else {
        0
    };
    let mut rows = Vec::with_capacity(visible.min(total_matches));
    for vis_row in 0..visible {
        let match_idx = scroll + vis_row;
        if match_idx >= total_matches {
            break;
        }
        rows.push((
            Rect {
                x: chunks[2].x,
                y: chunks[2].y + vis_row as u16,
                width: chunks[2].width,
                height: 1,
            },
            match_idx,
        ));
    }
    state.hit_regions.borrow_mut().command_palette =
        Some(crate::ui::hit_regions::CommandPaletteRegions {
            outer: popup_area,
            rows,
        });
}

fn centered(area: Rect, width_pct: u16, height_lines: u16) -> Rect {
    let w = (area.width * width_pct / 100).max(40).min(area.width.saturating_sub(4));
    let h = height_lines.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 3;
    Rect { x, y, width: w, height: h }
}
