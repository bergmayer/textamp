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
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

use crate::app::action::{
    Action, BrowseAction, NavigationAction, PlaybackAction, QueueAction, RadioAction, SearchAction,
    SettingsAction, SystemAction,
};
use crate::app::state::{
    AppState, BrowseCategory, ColumnSortMode, DjMode, PaletteCommandKind, PaletteEntry, View,
};
use rand::seq::IteratorRandom;

/// Built-in commands that don't depend on runtime data. `aliases`
/// are extra fuzzy-search terms that don't appear in the rendered
/// label — typing "del", "remove", "skip", "mute", etc. should
/// surface the matching command even though that exact word isn't
/// in the label.
struct StaticEntry {
    label: &'static str,
    hint: &'static str,
    command: PaletteCommandKind,
    aliases: &'static [&'static str],
}

fn static_entries() -> Vec<StaticEntry> {
    vec![
        // View navigation. Each `Goto …` entry mirrors the
        // corresponding Ctrl+key shortcut and carries the common
        // synonyms a user might type ("home" → Library, "tracks" →
        // Queue, etc.).
        StaticEntry {
            label: "Library",
            hint: "^L",
            command: PaletteCommandKind::GotoLibrary,
            aliases: &["home", "browse", "artists", "albums"],
        },
        StaticEntry {
            label: "Genres",
            hint: "^G",
            command: PaletteCommandKind::GotoGenres,
            aliases: &["mood", "style", "tags"],
        },
        StaticEntry {
            label: "Folders",
            hint: "^O",
            command: PaletteCommandKind::GotoFolders,
            aliases: &["directory", "files", "tree"],
        },
        StaticEntry {
            label: "Queue",
            hint: "^U",
            command: PaletteCommandKind::GotoQueue,
            aliases: &["playlist", "tracks", "up next", "now playing list"],
        },
        StaticEntry {
            label: "Now Playing",
            hint: "^N",
            command: PaletteCommandKind::GotoNowPlaying,
            aliases: &["current track", "visualizer", "spectrum", "waveform"],
        },
        StaticEntry {
            label: "Help",
            hint: "F1",
            command: PaletteCommandKind::OpenHelp,
            aliases: &["keyboard", "shortcuts", "keys", "manual"],
        },
        StaticEntry {
            label: "Settings",
            hint: "F2",
            command: PaletteCommandKind::OpenSettings,
            aliases: &["preferences", "config", "options"],
        },
        StaticEntry {
            label: "Switch Library",
            hint: "F3",
            command: PaletteCommandKind::SwitchLibrary,
            aliases: &["change library", "library picker", "server"],
        },
        // Layout toggles.
        StaticEntry {
            label: "Tall Mode (split Library + Now Playing)",
            hint: "|",
            command: PaletteCommandKind::ToggleTallMode,
            aliases: &["split", "stacked"],
        },
        StaticEntry {
            label: "Scrolling Miller Layout",
            hint: "\\",
            command: PaletteCommandKind::ToggleScrollingMiller,
            aliases: &["fixed", "wide", "scroll", "columns"],
        },
        // Catalogue actions.
        StaticEntry {
            label: "Refresh",
            hint: "F5",
            command: PaletteCommandKind::Refresh,
            aliases: &["reload", "rescan"],
        },
        StaticEntry {
            label: "Find / Search",
            hint: "^F",
            command: PaletteCommandKind::OpenSearch,
            aliases: &["find", "search", "lookup"],
        },
        StaticEntry {
            label: "Filter",
            hint: "/",
            command: PaletteCommandKind::ToggleFilter,
            aliases: &["narrow", "filter list"],
        },
        StaticEntry {
            label: "Similar",
            hint: "^M",
            command: PaletteCommandKind::OpenSimilar,
            aliases: &["recommendations", "like this"],
        },
        StaticEntry {
            label: "Related",
            hint: "^R",
            command: PaletteCommandKind::OpenRelated,
            aliases: &["related artists"],
        },
        StaticEntry {
            label: "Artist Bio",
            hint: "F4",
            command: PaletteCommandKind::ShowArtistBio,
            aliases: &["biography", "about artist"],
        },
        // Queue mutation. Synonyms for "delete" / "remove" / "skip"
        // surface the right action even when the user doesn't
        // remember the exact wording the menu uses.
        StaticEntry {
            label: "Save Queue as Playlist",
            hint: "^S",
            command: PaletteCommandKind::SaveQueue,
            aliases: &["save", "export queue", "make playlist"],
        },
        StaticEntry {
            label: "Clear Queue",
            hint: "^X",
            command: PaletteCommandKind::ClearQueue,
            aliases: &["clear", "empty queue", "wipe queue", "reset queue"],
        },
        StaticEntry {
            label: "Remove from Queue",
            hint: "Del",
            command: PaletteCommandKind::RemoveFocusedFromQueue,
            aliases: &["delete", "del", "remove track", "drop"],
        },
        StaticEntry {
            label: "Add to End of Queue",
            hint: "^E",
            command: PaletteCommandKind::EnqueueSelectionEnd,
            aliases: &["enqueue", "append", "queue this"],
        },
        StaticEntry {
            label: "Play Next in Queue",
            hint: "^⇧E",
            command: PaletteCommandKind::EnqueueSelectionNext,
            aliases: &["insert next", "queue next", "up next"],
        },
        StaticEntry {
            label: "Move Selection Up",
            hint: "⇧↑",
            command: PaletteCommandKind::MoveQueueSelectionUp,
            aliases: &["reorder up", "shift up", "promote"],
        },
        StaticEntry {
            label: "Move Selection Down",
            hint: "⇧↓",
            command: PaletteCommandKind::MoveQueueSelectionDown,
            aliases: &["reorder down", "shift down", "demote"],
        },
        StaticEntry {
            label: "Undo Queue Edit",
            hint: "^Z",
            command: PaletteCommandKind::UndoQueueEdit,
            aliases: &["undo", "revert", "rollback"],
        },
        // Transport controls.
        StaticEntry {
            label: "Play / Pause",
            hint: "Spc",
            command: PaletteCommandKind::PlayPause,
            aliases: &["pause", "play", "resume", "toggle play"],
        },
        StaticEntry {
            label: "Stop Playback",
            hint: "",
            command: PaletteCommandKind::StopPlayback,
            aliases: &["stop", "cancel station", "cancel radio"],
        },
        StaticEntry {
            label: "Next Track",
            hint: ">",
            command: PaletteCommandKind::NextTrack,
            aliases: &["skip", "forward", "advance"],
        },
        StaticEntry {
            label: "Previous Track",
            hint: "<",
            command: PaletteCommandKind::PrevTrack,
            aliases: &["back", "rewind", "prev"],
        },
        StaticEntry {
            label: "Volume Up",
            hint: "^⇧↑",
            command: PaletteCommandKind::VolumeUp,
            aliases: &["louder", "raise volume"],
        },
        StaticEntry {
            label: "Volume Down",
            hint: "^⇧↓",
            command: PaletteCommandKind::VolumeDown,
            aliases: &["quieter", "lower volume", "softer"],
        },
        StaticEntry {
            label: "Mute / Unmute",
            hint: "",
            command: PaletteCommandKind::ToggleMute,
            aliases: &["silence", "mute", "unmute"],
        },
        StaticEntry {
            label: "Seek Forward 10s",
            hint: "⇧→",
            command: PaletteCommandKind::SeekForward,
            aliases: &["fast forward", "skip ahead", "+10"],
        },
        StaticEntry {
            label: "Seek Back 10s",
            hint: "⇧←",
            command: PaletteCommandKind::SeekBackward,
            aliases: &["rewind", "scrub back", "-10"],
        },
        StaticEntry {
            label: "Artist Radio",
            hint: "",
            command: PaletteCommandKind::ArtistRadio,
            aliases: &["radio", "blend artists"],
        },
        StaticEntry {
            label: "Random Album",
            hint: "",
            command: PaletteCommandKind::RandomAlbum,
            aliases: &["surprise", "shuffle album", "any album"],
        },
        StaticEntry {
            label: "Open in Library",
            hint: "^J",
            command: PaletteCommandKind::OpenInLibrary,
            aliases: &["jump to library", "reveal", "show in library"],
        },
        StaticEntry {
            label: "Sonic Adventure\u{2026}",
            hint: "",
            command: PaletteCommandKind::SonicAdventure,
            aliases: &["journey", "trip", "path", "build adventure"],
        },
        // Sort cluster — typing "sort" in the palette surfaces all
        // of these together: the umbrella popup entry plus each
        // individual sort mode and the toggles. Labels share the
        // "Sort: …" prefix so fuzzy matching keeps them grouped.
        StaticEntry {
            label: "View Options",
            hint: "F6",
            command: PaletteCommandKind::OpenSort,
            aliases: &["sort", "sort popup", "arrange"],
        },
        StaticEntry {
            label: "Close Column",
            hint: "^W",
            command: PaletteCommandKind::CloseColumn,
            aliases: &["dismiss column", "close pane"],
        },
        StaticEntry {
            label: "Sort: Default",
            hint: "",
            command: PaletteCommandKind::ApplySort(ColumnSortMode::Default),
            aliases: &["original order"],
        },
        StaticEntry {
            label: "Sort: By Artist",
            hint: "",
            command: PaletteCommandKind::ApplySort(ColumnSortMode::ByArtist),
            aliases: &[],
        },
        StaticEntry {
            label: "Sort: By Album",
            hint: "",
            command: PaletteCommandKind::ApplySort(ColumnSortMode::ByAlbum),
            aliases: &[],
        },
        StaticEntry {
            label: "Sort: By Title",
            hint: "",
            command: PaletteCommandKind::ApplySort(ColumnSortMode::ByTitle),
            aliases: &["alphabetical", "by name"],
        },
        StaticEntry {
            label: "Sort: By Duration",
            hint: "",
            command: PaletteCommandKind::ApplySort(ColumnSortMode::ByDuration),
            aliases: &["by length", "by time"],
        },
        StaticEntry {
            label: "Sort: Shuffle",
            hint: "",
            command: PaletteCommandKind::ApplySort(ColumnSortMode::Shuffled),
            aliases: &["randomize", "random order"],
        },
        StaticEntry {
            label: "Sort: Reverse Direction",
            hint: "",
            command: PaletteCommandKind::ReverseSort,
            aliases: &["flip order", "ascending descending"],
        },
        StaticEntry {
            label: "Sort: Toggle Group by Album",
            hint: "",
            command: PaletteCommandKind::ToggleGroupByAlbum,
            aliases: &["group", "cluster albums"],
        },
        StaticEntry {
            label: "Sort: Toggle Cover Art",
            hint: "",
            command: PaletteCommandKind::ToggleArtwork,
            aliases: &["thumbnails", "covers", "art tiles"],
        },
        StaticEntry {
            label: "Quit",
            hint: "^Q / :q",
            command: PaletteCommandKind::Quit,
            aliases: &["q", "exit", "close", "leave"],
        },
        // DJ modes — surfaced under "DJ ..." so the now-playing
        // sidebar can prefilter the palette to just these.
        StaticEntry {
            label: "DJ Stretch",
            hint: "",
            command: PaletteCommandKind::ToggleDj(DjMode::Stretch),
            aliases: &[],
        },
        StaticEntry {
            label: "DJ Gemini",
            hint: "",
            command: PaletteCommandKind::ToggleDj(DjMode::Gemini),
            aliases: &[],
        },
        StaticEntry {
            label: "DJ Freeze",
            hint: "",
            command: PaletteCommandKind::ToggleDj(DjMode::Freeze),
            aliases: &[],
        },
        StaticEntry {
            label: "DJ Twofer",
            hint: "",
            command: PaletteCommandKind::ToggleDj(DjMode::Twofer),
            aliases: &[],
        },
        StaticEntry {
            label: "DJ Contempo",
            hint: "",
            command: PaletteCommandKind::ToggleDj(DjMode::Contempo),
            aliases: &[],
        },
        StaticEntry {
            label: "DJ Groupie",
            hint: "",
            command: PaletteCommandKind::ToggleDj(DjMode::Groupie),
            aliases: &[],
        },
        // Remix tools.
        StaticEntry {
            label: "Remix: Gemini",
            hint: "",
            command: PaletteCommandKind::RemixGemini,
            aliases: &[],
        },
        StaticEntry {
            label: "Remix: Twofer",
            hint: "",
            command: PaletteCommandKind::RemixTwofer,
            aliases: &[],
        },
        StaticEntry {
            label: "Remix: Stretch",
            hint: "",
            command: PaletteCommandKind::RemixStretch,
            aliases: &[],
        },
        StaticEntry {
            label: "Remix: Doppelganger",
            hint: "",
            command: PaletteCommandKind::RemixDoppelganger,
            aliases: &[],
        },
        StaticEntry {
            label: "Remix: Shuffle",
            hint: "",
            command: PaletteCommandKind::RemixShuffle,
            aliases: &["scramble", "randomize queue"],
        },
        StaticEntry {
            label: "Remix: Undo Shuffle",
            hint: "",
            command: PaletteCommandKind::RemixUndoShuffle,
            aliases: &["unshuffle"],
        },
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
    let focused_album = (state.view == View::Browse && !state.category_column_focused)
        .then(|| state.focused_album())
        .flatten();

    if let Some(track) = target_track.clone() {
        // Build the contextual entry list from the shared
        // `track_context_entries` source so the palette stays
        // identical to the GUI right-click context menu. Adding /
        // reordering happens in
        // `crate::services::track_context::track_context_entries`.
        let entries =
            crate::services::track_context::track_context_entries(state, &track, target_is_similar);
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
                aliases: vec![],
            });
        }
    } else if focused_album.is_some() {
        out.push(PaletteEntry {
            label: "Play Album".to_string(),
            hint: String::new(),
            command: PaletteCommandKind::PlayFocusedAlbum,
            aliases: vec!["play this".into(), "start album".into()],
        });
        if not_in_library {
            out.push(PaletteEntry {
                label: "Open in Library".to_string(),
                hint: "^J".to_string(),
                command: PaletteCommandKind::OpenInLibrary,
                aliases: vec!["jump to library".into()],
            });
        }
        out.push(PaletteEntry {
            label: "Artist Bio".to_string(),
            hint: "F4".to_string(),
            command: PaletteCommandKind::ShowArtistBio,
            aliases: vec!["biography".into()],
        });
    }

    // 2. External search — non-empty query required AND each service
    //    must be enabled in Settings ("Search ⟨service⟩" toggle).
    //    Disabled services are completely hidden from the palette so
    //    the toggle reads as "remove this service from the app", not
    //    just "make this entry a no-op".
    //    Skipped when a track is focused: `track_context_entries`
    //    above already emits the same per-service entries with
    //    identical labels, and adding them again would duplicate
    //    every row.
    let ext_query = crate::app::handlers::key_input::build_external_search_query(state);
    if !ext_query.is_empty() && target_track.is_none() {
        if state.external_search.apple_music {
            out.push(PaletteEntry {
                label: "Search Apple Music for selection".to_string(),
                hint: String::new(),
                command: PaletteCommandKind::SearchAppleMusic,
                aliases: vec!["apple".into(), "itunes".into()],
            });
        }
        if state.external_search.spotify {
            out.push(PaletteEntry {
                label: "Search Spotify for selection".to_string(),
                hint: String::new(),
                command: PaletteCommandKind::SearchSpotify,
                aliases: vec![],
            });
        }
        if state.external_search.youtube {
            out.push(PaletteEntry {
                label: "Search YouTube for selection".to_string(),
                hint: String::new(),
                command: PaletteCommandKind::SearchYouTube,
                aliases: vec!["yt".into()],
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
    let queue_context = matches!(state.view, View::Queue | View::NowPlaying);

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
    //   - "Sonic Adventure…" when a track is focused (the contextual
    //     block above already added it via track_context_entries).
    let context_has_open_in_library = (target_track.is_some()
        && (not_in_library || target_is_similar))
        || (focused_album.is_some() && not_in_library);
    let context_has_sonic_adventure = target_track.is_some();
    // The contextual block (track context entries OR the album-
    // focused branch) already added "Artist Bio" / "Add to End of
    // Queue" / "Play Next in Queue", so the static entries with
    // the same effect would duplicate the row. The track-context
    // path emits the queue entries; the album-focused path emits
    // Artist Bio; either implies the static row should be hidden.
    let context_has_artist_bio = target_track.is_some() || focused_album.is_some();
    let context_has_enqueue = target_track.is_some();
    out.extend(crate::app::sources::navidrome::commands::entries(state));
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
        if matches!(e.command, PaletteCommandKind::SonicAdventure) && context_has_sonic_adventure {
            return None;
        }
        if matches!(e.command, PaletteCommandKind::ShowArtistBio) && context_has_artist_bio {
            return None;
        }
        if matches!(
            e.command,
            PaletteCommandKind::EnqueueSelectionEnd | PaletteCommandKind::EnqueueSelectionNext
        ) && context_has_enqueue
        {
            return None;
        }
        Some(PaletteEntry {
            label: e.label.to_string(),
            hint: e.hint.to_string(),
            command: e.command,
            aliases: e.aliases.iter().map(|s| (*s).to_string()).collect(),
        })
    }));

    // 4. Radio navigation. Utility rows already have native commands above;
    // never send their synthetic keys to server as station URLs.
    if state.station_nav.can_go_left() {
        out.push(PaletteEntry {
            label: "Radio: Back".into(),
            hint: String::new(),
            command: PaletteCommandKind::StationsBack,
            aliases: vec!["parent stations".into()],
        });
    }
    for s in &state.stations {
        if s.is_separator() || s.is_dj_mode() || s.is_remix() || s.is_action() {
            continue;
        }
        // Prefix station rows with "Radio: " so typing "Radio" in the
        // palette (e.g. via the sidebar button) surfaces every station
        // in one cluster, while still letting the user type the
        // station name directly.
        out.push(PaletteEntry {
            label: format!("Radio: {}", s.title),
            hint: String::new(),
            command: if s.is_category() {
                PaletteCommandKind::BrowseStations {
                    key: s.key.clone(),
                    title: s.title.clone(),
                }
            } else {
                PaletteCommandKind::PlayStation(s.key.clone())
            },
            aliases: vec!["station".into()],
        });
    }
    out.retain(|entry| crate::app::sources::command_visible(state, &entry.command));
    out
}

/// Translate a palette command into the same `Action`s the shortcut
/// bar used to dispatch.
pub fn run(cmd: PaletteCommandKind, state: &mut AppState) -> Vec<Action> {
    match cmd {
        PaletteCommandKind::Navidrome(command) => {
            if matches!(
                command,
                crate::app::sources::navidrome::commands::Command::Collection(_)
            ) {
                state.category_column_focused = false;
            }
            vec![crate::app::sources::navidrome::NavAction::Command(command).into()]
        }
        PaletteCommandKind::Quit => vec![SystemAction::Quit.into()],
        PaletteCommandKind::GotoLibrary => {
            vec![NavigationAction::set_category(BrowseCategory::Library).into()]
        }
        PaletteCommandKind::GotoGenres => {
            vec![NavigationAction::set_category(BrowseCategory::AlbumGenres).into()]
        }
        PaletteCommandKind::GotoFolders => {
            vec![NavigationAction::set_category(BrowseCategory::Folders).into()]
        }
        PaletteCommandKind::GotoQueue => vec![NavigationAction::SetView(View::Queue).into()],
        PaletteCommandKind::GotoNowPlaying => {
            vec![NavigationAction::SetView(View::NowPlaying).into()]
        }
        PaletteCommandKind::OpenHelp => vec![NavigationAction::SetView(View::Help).into()],
        PaletteCommandKind::OpenSettings => vec![SettingsAction::OpenSettings.into()],
        PaletteCommandKind::OpenSearch => vec![SearchAction::OpenSearchPopup.into()],
        PaletteCommandKind::OpenSimilar => {
            crate::app::handlers::key_input::get_similar_action(state)
        }
        PaletteCommandKind::OpenRelated => {
            crate::app::handlers::key_input::get_related_action(state)
        }
        PaletteCommandKind::SaveQueue => vec![QueueAction::PromptSavePlaylist.into()],
        PaletteCommandKind::ClearQueue => vec![QueueAction::ClearQueue.into()],
        PaletteCommandKind::ToggleFilter => vec![SearchAction::ActivateListFilter.into()],
        PaletteCommandKind::ToggleTallMode => vec![SettingsAction::ToggleTallMode.into()],
        PaletteCommandKind::Refresh => crate::app::handlers::helpers::refresh_current_view(state),
        PaletteCommandKind::StopPlayback => vec![PlaybackAction::Stop.into()],
        PaletteCommandKind::PlayPause => vec![PlaybackAction::TogglePlayPause.into()],
        PaletteCommandKind::NextTrack => vec![PlaybackAction::Next.into()],
        PaletteCommandKind::PrevTrack => vec![PlaybackAction::Previous.into()],
        PaletteCommandKind::ToggleDj(mode) => vec![RadioAction::ToggleDjMode(mode).into()],
        PaletteCommandKind::SwitchLibrary => vec![SearchAction::OpenLibraryPicker.into()],
        PaletteCommandKind::ToggleScrollingMiller => {
            vec![SettingsAction::ToggleMillerLayout.into()]
        }
        PaletteCommandKind::VolumeUp => vec![PlaybackAction::VolumeUp.into()],
        PaletteCommandKind::VolumeDown => vec![PlaybackAction::VolumeDown.into()],
        PaletteCommandKind::ToggleMute => vec![PlaybackAction::ToggleMute.into()],
        PaletteCommandKind::SeekForward => vec![PlaybackAction::SeekRelative(10_000).into()],
        PaletteCommandKind::SeekBackward => vec![PlaybackAction::SeekRelative(-10_000).into()],
        PaletteCommandKind::UndoQueueEdit => vec![QueueAction::UndoLastRemix.into()],
        PaletteCommandKind::EnqueueSelectionEnd => vec![QueueAction::EnqueueSelection.into()],
        PaletteCommandKind::EnqueueSelectionNext => vec![QueueAction::EnqueueSelectionNext.into()],
        PaletteCommandKind::MoveQueueSelectionUp => vec![QueueAction::MoveSelectedTracksUp.into()],
        PaletteCommandKind::MoveQueueSelectionDown => {
            vec![QueueAction::MoveSelectedTracksDown.into()]
        }
        PaletteCommandKind::RemoveFocusedFromQueue => {
            // Resolves the row to delete from the current view:
            //   - Queue / Now Playing: the highlighted queue row.
            //   - Anywhere else: no-op with a status hint (the
            //     palette is open globally so the entry needs to
            //     gracefully sit out when it doesn't apply).
            // Mirrors the Del binding in
            // `key_input::now_playing::handle_now_playing_keys`.
            if matches!(state.view, View::Queue | View::NowPlaying) {
                let idx = state.list_state.queue_index;
                if !state.queue.selected.is_empty() {
                    vec![QueueAction::RemoveSelectedFromQueue.into()]
                } else if idx < state.playback_tracks().len() {
                    vec![QueueAction::RemoveFromQueue(idx).into()]
                } else {
                    state.set_status("Queue is empty".to_string());
                    vec![]
                }
            } else {
                state.set_status("Open Queue (Ctrl+U) to remove tracks".to_string());
                vec![]
            }
        }
        PaletteCommandKind::RemixGemini => vec![QueueAction::RemixGemini.into()],
        PaletteCommandKind::RemixTwofer => vec![QueueAction::RemixTwofer.into()],
        PaletteCommandKind::RemixStretch => vec![QueueAction::RemixStretch.into()],
        PaletteCommandKind::RemixDoppelganger => vec![QueueAction::RemixDoppelganger.into()],
        PaletteCommandKind::RemixShuffle => vec![QueueAction::RemixShuffle.into()],
        PaletteCommandKind::RemixUndoShuffle => vec![QueueAction::RemixUndoShuffle.into()],
        PaletteCommandKind::PlayStation(key) => {
            vec![RadioAction::PlayStation(key).into()]
        }
        PaletteCommandKind::BrowseStations { key, title } => {
            open_with_query(state, "Radio");
            vec![RadioAction::DrillIntoStation(key, title).into()]
        }
        PaletteCommandKind::StationsBack => {
            open_with_query(state, "Radio");
            vec![RadioAction::NavigateStationsBack.into()]
        }
        PaletteCommandKind::ArtistRadio => vec![SearchAction::OpenArtistRadioPicker.into()],
        PaletteCommandKind::RandomAlbum => {
            // Mirror the GUI's `PlayOneRandomAlbum`: pick one album
            // at random from the active library and dispatch
            // PlayAlbumNow. Falls back to a no-op error when the
            // album list isn't loaded yet.
            let pick = state
                .library
                .albums
                .iter()
                .choose(&mut rand::rng())
                .map(|a| (a.rating_key.clone(), a.title.clone()));
            match pick {
                Some((rating_key, title)) => {
                    vec![QueueAction::PlayAlbumNow { rating_key, title }.into()]
                }
                None => {
                    state.set_error("No albums in library to pick from".to_string());
                    vec![]
                }
            }
        }
        PaletteCommandKind::OpenInLibrary => {
            use crate::app::state::BrowseItem;

            // Use the visible context first, then a visible album, then playback.
            if let Some(track) = state.palette_target_track() {
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
            let focused_album_item = (state.view == View::Browse && !state.category_column_focused)
                .then(|| state.browse_nav())
                .flatten()
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
                state.set_status("Track has no library artist".to_string());
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
                    return vec![QueueAction::PlayTrack(Box::new(track)).into()];
                }
                return vec![];
            }
            play_focused_track(state, true)
        }
        PaletteCommandKind::PlayFocusedTrackAndFollowing => play_focused_track(state, false),
        PaletteCommandKind::SearchAppleMusic => {
            vec![SystemAction::OpenExternalSearch {
                target: crate::services::external_search::SearchTarget::AppleMusic,
                query: None,
            }
            .into()]
        }
        PaletteCommandKind::SearchSpotify => {
            vec![SystemAction::OpenExternalSearch {
                target: crate::services::external_search::SearchTarget::Spotify,
                query: None,
            }
            .into()]
        }
        PaletteCommandKind::SearchYouTube => {
            vec![SystemAction::OpenExternalSearch {
                target: crate::services::external_search::SearchTarget::YouTube,
                query: None,
            }
            .into()]
        }
        PaletteCommandKind::PlayFocusedAlbum => match state.focused_album() {
            Some((rating_key, title)) => {
                vec![QueueAction::PlayAlbumNow { rating_key, title }.into()]
            }
            None => {
                state.set_status("No album focused".to_string());
                vec![]
            }
        },
        PaletteCommandKind::ShowArtistBio => {
            match crate::app::handlers::helpers::get_artist_for_bio(state) {
                Some((artist_key, artist_name)) => vec![SearchAction::ShowArtistBio {
                    artist_key,
                    artist_name,
                }
                .into()],
                None => {
                    state.set_status("No artist context for bio".to_string());
                    vec![]
                }
            }
        }
        PaletteCommandKind::ApplySort(mode) => {
            vec![SearchAction::ApplyFocusedSortMode(mode).into()]
        }
        PaletteCommandKind::ReverseSort => vec![SearchAction::ReverseFocusedSortDirection.into()],
        PaletteCommandKind::CloseColumn => {
            crate::app::handlers::key_input::close_focused_browse_column(state);
            vec![]
        }
        PaletteCommandKind::SonicAdventureFromFocusedTrack => match state.palette_target_track() {
            Some(track) => vec![SearchAction::OpenAdventureLauncherWithStart {
                start_track: Box::new(track),
            }
            .into()],
            None => {
                state.set_status("No track focused".to_string());
                vec![]
            }
        },
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
            use crate::app::action::{
                BrowseAction, DataAction, NavigationAction, QueueAction, SearchAction, SystemAction,
            };
            use crate::app::state::{SimilarMode, View};
            use crate::services::track_context::ContextKind;
            match kind {
                ContextKind::Separator => vec![],
                ContextKind::PlayTrack => vec![QueueAction::PlayTrack(track).into()],
                ContextKind::PlayTrackAndFollowing => play_focused_track(state, false),
                ContextKind::PlayNextInQueue => {
                    vec![QueueAction::EnqueueTracksNext(vec![*track]).into()]
                }
                ContextKind::AddToEndOfQueue => vec![QueueAction::EnqueueTrack(track).into()],
                ContextKind::OpenInLibrary => {
                    if let Some(artist_key) = track.grandparent_rating_key.clone() {
                        vec![BrowseAction::OpenInLibrary {
                            artist_key,
                            artist_name: track.artist_name().to_string(),
                            album_key: track.parent_rating_key.clone(),
                            album_title: track.parent_title.clone(),
                        }
                        .into()]
                    } else {
                        state.set_status("Track has no artist key".to_string());
                        vec![]
                    }
                }
                ContextKind::SonicAdventure => {
                    vec![SearchAction::OpenAdventureLauncherWithStart { start_track: track }.into()]
                }
                ContextKind::SonicRadio => {
                    vec![RadioAction::StartSonicRadio(track).into()]
                }
                ContextKind::ArtistBio {
                    artist_key,
                    artist_name,
                } => vec![SearchAction::ShowArtistBio {
                    artist_key,
                    artist_name,
                }
                .into()],
                ContextKind::SearchExternal(target) => vec![SystemAction::OpenExternalSearch {
                    target,
                    query: None,
                }
                .into()],
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

/// Play from the same visible list used to construct the contextual command.
fn play_focused_track(state: &AppState, single: bool) -> Vec<Action> {
    let Some((tracks, index)) = state.palette_track_list() else {
        return vec![];
    };
    if single {
        vec![QueueAction::PlayTrack(Box::new(tracks[index].clone())).into()]
    } else {
        vec![QueueAction::PlayTracksNow(tracks[index..].to_vec()).into()]
    }
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
        let mut scored: Vec<((bool, i64), usize)> = entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                // Score against label AND every alias; take the
                // best match. Aliases are penalized lightly so a
                // direct label hit ranks above an alias-only hit
                // of the same nominal score (otherwise "queue"
                // could surface "Add to End of Queue" above the
                // bare "Queue" entry just because alias matches
                // tied with the label match).
                let label_score = matcher.fuzzy_match(&e.label, &query);
                let alias_score = e
                    .aliases
                    .iter()
                    .filter_map(|a| matcher.fuzzy_match(a, &query))
                    .max()
                    .map(|s| s - 1);
                let best = match (label_score, alias_score) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (Some(a), None) => Some(a),
                    (None, Some(b)) => Some(b),
                    (None, None) => None,
                };
                // Exact command names and aliases beat partial fuzzy matches:
                // the `q` alias must select Quit, not a queue-related command.
                let exact = e.label.eq_ignore_ascii_case(&query)
                    || e.aliases.iter().any(|a| a.eq_ignore_ascii_case(&query));
                best.map(|s| ((exact, s), i))
            })
            .collect();
        scored.sort_by_key(|item| std::cmp::Reverse(item.0));
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
