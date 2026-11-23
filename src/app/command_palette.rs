//! Neovim-style command-palette overlay for the TUI.
//!
//! Open with `:` from any view. Type to fuzzy-search; Up/Down to
//! navigate; Enter to execute the selected entry; Esc to abort.
//!
//! The candidate list is rebuilt on every keystroke from a static
//! registry of built-in commands plus runtime content (radio
//! stations) so the user can launch any radio station by typing its
//! name into the same overlay used for keyboard shortcuts.

use crate::app::state::RepeatMode;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;

use crate::app::action::{
    Action, BrowseAction, NavigationAction, PlaybackAction, QueueAction, RadioAction, SearchAction,
    SettingsAction, SystemAction,
};
use crate::app::state::{
    AppState, BrowseCategory, ColumnSortMode, LibraryRoot, PaletteCommandKind, PaletteEntry, View,
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
    let mut entries = vec![
        StaticEntry {
            label: "Cycle Application Mode",
            hint: "⌘⌥T",
            command: PaletteCommandKind::CycleApplicationMode,
            aliases: &["gui single multiple window text tui cycle"],
        },
        StaticEntry {
            label: crate::app::launch::ApplicationMode::GuiMultiple.label(),
            hint: "",
            command: PaletteCommandKind::SetApplicationMode(
                crate::app::launch::ApplicationMode::GuiMultiple,
            ),
            aliases: &["multi graphical windows"],
        },
        StaticEntry {
            label: crate::app::launch::ApplicationMode::GuiSingle.label(),
            hint: "",
            command: PaletteCommandKind::SetApplicationMode(
                crate::app::launch::ApplicationMode::GuiSingle,
            ),
            aliases: &["single graphical window"],
        },
        StaticEntry {
            label: crate::app::launch::ApplicationMode::Text.label(),
            hint: "",
            command: PaletteCommandKind::SetApplicationMode(
                crate::app::launch::ApplicationMode::Text,
            ),
            aliases: &["terminal tui interface"],
        },
        StaticEntry {
            label: "Singles",
            hint: "",
            command: PaletteCommandKind::GotoSingles,
            aliases: &["short releases", "two tracks"],
        },
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
            label: "Split View",
            hint: "",
            command: PaletteCommandKind::ToggleSplitView,
            aliases: &["split", "stacked"],
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
            label: "Export Playlist as M3U8…",
            hint: "",
            command: PaletteCommandKind::ExportPlaylist {
                format: crate::services::playlist_export::Format::M3u8,
                queue: false,
            },
            aliases: &["export", "playlist file"],
        },
        StaticEntry {
            label: "Export Playlist as CSV…",
            hint: "",
            command: PaletteCommandKind::ExportPlaylist {
                format: crate::services::playlist_export::Format::Csv,
                queue: false,
            },
            aliases: &["export", "playlist file"],
        },
        StaticEntry {
            label: "Export Queue as M3U8…",
            hint: "",
            command: PaletteCommandKind::ExportPlaylist {
                format: crate::services::playlist_export::Format::M3u8,
                queue: true,
            },
            aliases: &["export", "playlist file"],
        },
        StaticEntry {
            label: "Export Queue as CSV…",
            hint: "",
            command: PaletteCommandKind::ExportPlaylist {
                format: crate::services::playlist_export::Format::Csv,
                queue: true,
            },
            aliases: &["export", "playlist file"],
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
            label: "Undo",
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
            label: "Artist Mix",
            hint: "",
            command: PaletteCommandKind::ArtistRadio,
            aliases: &["radio", "blend artists"],
        },
        StaticEntry {
            label: "Go to Random Album",
            hint: "^⇧R",
            command: PaletteCommandKind::GoToRandomAlbum,
            aliases: &["browse random album", "surprise album", "discover album"],
        },
        StaticEntry {
            label: "Open in Library",
            hint: "^J",
            command: PaletteCommandKind::OpenInLibrary,
            aliases: &["jump to library", "reveal", "show in library"],
        },
        StaticEntry {
            label: "Journey\u{2026}",
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
            label: "Sort: By Date Added",
            hint: "",
            command: PaletteCommandKind::ApplySort(ColumnSortMode::ByDateAdded),
            aliases: &["newest first", "recently added order"],
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
            label: "Show Compilation-Only Artists",
            hint: "",
            command: PaletteCommandKind::ToggleCompilationArtists,
            aliases: &["hide compilation artists", "show compilation artists"],
        },
        StaticEntry {
            label: "Quit",
            hint: "^Q / :q",
            command: PaletteCommandKind::Quit,
            aliases: &["q", "exit", "close", "leave"],
        },
        // Remix tools.
        StaticEntry {
            label: "Remix: Twins",
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
            label: "Remix: Opposite Day",
            hint: "",
            command: PaletteCommandKind::RemixOppositeDay,
            aliases: &["distant", "opposites"],
        },
        StaticEntry {
            label: "Remix: Enemies",
            hint: "",
            command: PaletteCommandKind::RemixEnemies,
            aliases: &["distant pair"],
        },
        StaticEntry {
            label: "Repeat Off",
            hint: "",
            command: PaletteCommandKind::SetQueueRepeat(RepeatMode::Off),
            aliases: &["loop off"],
        },
        StaticEntry {
            label: "Repeat Track",
            hint: "",
            command: PaletteCommandKind::SetQueueRepeat(RepeatMode::Track),
            aliases: &["loop song"],
        },
        StaticEntry {
            label: "Repeat Queue",
            hint: "",
            command: PaletteCommandKind::SetQueueRepeat(RepeatMode::Queue),
            aliases: &["loop playlist"],
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
    ];
    entries.extend(
        crate::app::downloads::Menu::ALL
            .into_iter()
            .map(|command| StaticEntry {
                label: command.label(),
                hint: "",
                command: PaletteCommandKind::Downloads(command),
                aliases: &["offline", "download"],
            }),
    );
    entries
}

/// Global command definitions, independent of selection and the current view.
/// Native menus reuse these definitions so palette additions remain discoverable.
pub fn global_entries() -> Vec<PaletteEntry> {
    let mut entries: Vec<_> = static_entries()
        .into_iter()
        .map(|e| PaletteEntry {
            label: e.label.into(),
            hint: e.hint.into(),
            command: e.command,
            aliases: e.aliases.iter().map(|s| (*s).into()).collect(),
        })
        .chain([
            PaletteEntry {
                label: "Full-screen Visualizer".into(),
                hint: "F7".into(),
                command: PaletteCommandKind::ToggleFullscreen,
                aliases: vec![],
            },
            PaletteEntry {
                label: "Full-screen Artwork".into(),
                hint: String::new(),
                command: PaletteCommandKind::FullscreenArtwork,
                aliases: vec![],
            },
        ])
        .collect();
    entries.extend(
        [
            (
                "Full-screen Artwork with Transport",
                PaletteCommandKind::DesktopFullscreen(
                    crate::app::state::DesktopFullscreenView::ArtworkTransport,
                ),
            ),
            (
                "Full-screen Visualizer",
                PaletteCommandKind::DesktopFullscreen(
                    crate::app::state::DesktopFullscreenView::Visualizer,
                ),
            ),
            (
                "Full-screen Visualizer with Transport",
                PaletteCommandKind::DesktopFullscreen(
                    crate::app::state::DesktopFullscreenView::VisualizerTransport,
                ),
            ),
            ("Radio", PaletteCommandKind::OpenRadioBrowser),
            ("AI", PaletteCommandKind::OpenAiBrowser),
            ("Play Random Albums", PaletteCommandKind::PlayRandomAlbums),
            ("Library Mix", PaletteCommandKind::LibraryMix),
        ]
        .into_iter()
        .map(|(label, command)| PaletteEntry {
            label: label.into(),
            hint: String::new(),
            command,
            aliases: vec![],
        }),
    );
    use super::sources::navidrome::commands::{CollectionKind, Command};
    entries.extend(
        [
            (
                "Random song mix",
                Command::Collection(CollectionKind::RandomMix),
            ),
            ("Create New Playlist…", Command::PromptNewPlaylist(None)),
            ("Save playback queue to Navidrome", Command::SaveQueue),
            (
                "Restore playback queue from Navidrome",
                Command::RestoreQueue,
            ),
        ]
        .into_iter()
        .map(|(label, command)| PaletteEntry {
            label: label.into(),
            hint: String::new(),
            command: PaletteCommandKind::Navidrome(command),
            aliases: vec![],
        }),
    );
    entries.extend(
        crate::audiomuse::Feature::ALL
            .into_iter()
            .filter(|f| f.is_search())
            .map(|feature| PaletteEntry {
                label: feature.label().into(),
                hint: String::new(),
                command: PaletteCommandKind::Navidrome(Command::Collection(
                    CollectionKind::AudioMuse(feature),
                )),
                aliases: vec![],
            }),
    );
    if let Some(index) = entries
        .iter()
        .position(|entry| matches!(entry.command, PaletteCommandKind::ToggleFullscreen))
    {
        let toggle = entries.remove(index);
        entries.push(toggle);
    }
    #[cfg(target_os = "macos")]
    entries.extend(
        [
            super::player_sync::Menu::Refresh,
            super::player_sync::Menu::Sync,
            super::player_sync::Menu::Cancel,
            super::player_sync::Menu::Eject,
        ]
        .into_iter()
        .map(|menu| PaletteEntry {
            label: menu.label().into(),
            hint: String::new(),
            command: PaletteCommandKind::PlayerSync(menu),
            aliases: vec!["usb portable device music player transfer sync".into()],
        }),
    );
    entries.extend(
        [
            super::lyrics::Control::Search,
            super::lyrics::Control::ToggleSync,
            super::lyrics::Control::NextLanguage,
            super::lyrics::Control::Retry,
        ]
        .into_iter()
        .map(|control| PaletteEntry {
            label: control.label().into(),
            hint: String::new(),
            command: PaletteCommandKind::LyricsControl(control),
            aliases: vec!["lyrics".into()],
        }),
    );
    entries
}

/// Materialize contextual actions, configured external searches and the global registry.
pub fn materialize_entries(state: &AppState) -> Vec<PaletteEntry> {
    let mut out: Vec<PaletteEntry> = Vec::new();
    let download_item = super::downloads::selected_item_command(state);
    let download_label = download_item.as_ref().map(|(label, _)| *label);
    if let Some((label, command)) = download_item {
        let menu = match command {
            super::downloads::Command::Cancel(_) => super::downloads::Menu::CancelSelection,
            _ => super::downloads::Menu::Remove,
        };
        out.push(PaletteEntry {
            label: label.into(),
            hint: String::new(),
            command: PaletteCommandKind::Downloads(menu),
            aliases: vec!["downloads".into()],
        });
    }

    if let Some(info) = super::details::focused_artist(state) {
        out.extend(
            super::details::artist_commands(info)
                .into_iter()
                .map(|command| PaletteEntry {
                    label: if command == super::details::ArtistCommand::SearchWeb {
                        state.web_search_provider.search_label()
                    } else {
                        command.label()
                    }
                    .into(),
                    hint: String::new(),
                    command: PaletteCommandKind::ArtistDetail(command),
                    aliases: vec!["artist details photo biography".into()],
                }),
        );
    }
    for (label, hint, command, aliases) in [
        (
            "Continue with Related Tracks",
            "",
            PaletteCommandKind::ToggleAutoPlay,
            "autoplay continuous related tracks",
        ),
        (
            "Auto-Mix",
            "",
            PaletteCommandKind::ToggleAutoMix,
            "automix smooth song transitions crossfade",
        ),
        ("Lyrics", "", PaletteCommandKind::ShowLyrics, "song words"),
        (
            "Full-screen Visualizer",
            "F7",
            PaletteCommandKind::ToggleFullscreen,
            "fullscreen visualizer cinema",
        ),
        (
            "Full-screen Artwork",
            "",
            PaletteCommandKind::FullscreenArtwork,
            "fullscreen artwork album cover",
        ),
    ] {
        out.push(PaletteEntry {
            label: label.into(),
            hint: hint.into(),
            command,
            aliases: vec![aliases.into()],
        });
    }

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
        // `crate::app::track_context::track_context_entries`.
        let entries = if state.view == View::NowPlaying {
            crate::app::track_context::now_playing_context(
                crate::app::track_context::ContextPolicy::for_state(state),
                &track,
            )
        } else {
            crate::app::track_context::track_context_entries(state, &track, target_is_similar)
        };
        let track_box = Box::new(track);
        for ce in entries {
            use crate::app::track_context::ContextKind;
            // The palette doesn't render visual separators (its rows
            // are a fuzzy-search list, not a structured menu).
            if matches!(
                ce.kind,
                ContextKind::Separator | ContextKind::SearchExternal(_)
            ) {
                continue;
            }
            if matches!(ce.kind, ContextKind::AddToPlaylist)
                && let Some(addition) =
                    super::sources::navidrome::playlist_edit::palette_addition(state)
            {
                out.push(PaletteEntry {
                    label: if matches!(
                        addition,
                        super::sources::navidrome::playlist_edit::Addition::Tracks(_)
                    ) {
                        "Add Selected Tracks to Playlist…".into()
                    } else {
                        ce.label
                    },
                    hint: String::new(),
                    aliases: vec![],
                    command: PaletteCommandKind::Navidrome(
                        super::sources::navidrome::commands::Command::ChoosePlaylist(addition),
                    ),
                });
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
    }

    out.extend(
        crate::services::external_search::SearchTarget::ALL
            .into_iter()
            .filter(|target| super::external_search::enabled(state, *target))
            .map(super::external_search::entry),
    );

    // Queue edits remain discoverable while a queue or station is loading.
    let queue_context = matches!(state.view, View::Queue | View::NowPlaying);

    if crate::app::sources::audiomuse::description_results(state).is_some() {
        out.push(PaletteEntry {
            label: "Save as playlist".into(),
            hint: String::new(),
            command: PaletteCommandKind::SaveDescriptionPlaylist,
            aliases: vec![
                "save description results".into(),
                "save music description".into(),
            ],
        });
    }
    out.extend(crate::app::sources::navidrome::commands::entries(state));
    out.extend(static_entries().into_iter().filter_map(|e| {
        if download_label == Some(e.label) {
            return None;
        }
        if !state.desktop_frontend && matches!(e.command, PaletteCommandKind::ToggleSplitView) {
            return None;
        }
        let is_remix = matches!(
            e.command,
            PaletteCommandKind::RemixGemini
                | PaletteCommandKind::RemixTwofer
                | PaletteCommandKind::RemixStretch
                | PaletteCommandKind::RemixDoppelganger
                | PaletteCommandKind::RemixOppositeDay
                | PaletteCommandKind::RemixEnemies
                | PaletteCommandKind::RemixShuffle
                | PaletteCommandKind::RemixUndoShuffle
        );
        if is_remix && !queue_context {
            return None;
        }
        if matches!(e.command, PaletteCommandKind::OpenInLibrary) && target_track.is_some() {
            return None;
        }
        if matches!(e.command, PaletteCommandKind::SonicAdventure) && target_track.is_some() {
            return None;
        }
        if matches!(
            e.command,
            PaletteCommandKind::EnqueueSelectionEnd | PaletteCommandKind::EnqueueSelectionNext
        ) && target_track.is_some()
        {
            return None;
        }
        let label = e.label;
        Some(PaletteEntry {
            label: label.to_string(),
            hint: e.hint.to_string(),
            command: e.command,
            aliases: e.aliases.iter().map(|s| (*s).to_string()).collect(),
        })
    }));

    {
        out.push(PaletteEntry {
            label: "Manual Queue".into(),
            hint: String::new(),
            command: PaletteCommandKind::StopRadio,
            aliases: vec!["stop station".into(), "turn off radio".into()],
        });
    }

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
    for choice in super::queue_modes::choices(state) {
        if !matches!(
            choice.command,
            PaletteCommandKind::PlayStation(_) | PaletteCommandKind::ConfigureStation(_)
        ) {
            continue;
        }
        out.push(PaletteEntry {
            label: format!("Radio: {}", choice.label),
            hint: String::new(),
            command: choice.command,
            aliases: vec!["station".into()],
        });
    }
    out.retain(|entry| {
        super::command_catalog::analysis_command(&entry.command)
            || crate::app::sources::command_visible(state, &entry.command)
    });
    out.extend(global_entries());
    out.extend(
        super::queue_controls::definitions(state)
            .into_iter()
            .map(|tool| PaletteEntry {
                label: tool.label.into(),
                hint: tool.description.into(),
                command: PaletteCommandKind::QueueTool(tool.tool),
                aliases: vec![],
            }),
    );
    let mut seen = std::collections::HashSet::new();
    out.retain(|entry| {
        let key =
            super::command_history::key(&entry.command).unwrap_or_else(|| entry.label.clone());
        seen.insert(key)
    });
    out
}

/// Translate a palette command into application actions.
pub fn remember(command: &PaletteCommandKind, state: &mut AppState) -> Vec<Action> {
    if super::command_catalog::disabled_reason(state, command).is_some() {
        return vec![];
    }
    if super::command_history::key(command).is_some_and(|key| state.palette.history.remember(key)) {
        vec![SettingsAction::PersistCommandHistory.into()]
    } else {
        vec![]
    }
}

pub fn run(cmd: PaletteCommandKind, state: &mut AppState) -> Vec<Action> {
    if let Some(reason) = super::command_catalog::disabled_reason(state, &cmd) {
        state.set_status(reason.into());
        return vec![];
    }
    match cmd {
        PaletteCommandKind::PlayerSync(menu) => vec![menu.command().into()],
        PaletteCommandKind::LyricsControl(control) => control.actions(state),
        PaletteCommandKind::QueueTool(tool) => super::queue_controls::activate(state, tool),
        PaletteCommandKind::Downloads(command) => command.actions(state),
        PaletteCommandKind::CycleApplicationMode => {
            if !state
                .windowed_presentation
                .can_activate(crate::app::launch::WindowedMode::Text)
            {
                return vec![];
            }
            state
                .application_mode()
                .map(|mode| {
                    SettingsAction::ActivatePreference(
                        crate::app::state::TextampSetting::Presentation(mode.next()),
                    )
                    .into()
                })
                .into_iter()
                .collect()
        }
        PaletteCommandKind::SetApplicationMode(mode) => {
            vec![
                SettingsAction::ActivatePreference(
                    crate::app::state::TextampSetting::Presentation(mode),
                )
                .into(),
            ]
        }
        PaletteCommandKind::GotoSingles => vec![BrowseAction::OpenSingles.into()],
        PaletteCommandKind::OpenCommandGroup(path) => {
            state.palette.path = path;
            state.palette.open = true;
            state.palette.query.clear();
            state.palette.cursor = 0;
            state.palette.selected = 0;
            refresh_matches(state);
            vec![]
        }
        PaletteCommandKind::Navidrome(command) => {
            if matches!(
                command,
                crate::app::sources::navidrome::commands::Command::Collection(_)
            ) {
                state.category_column_focused = false;
            }
            vec![crate::app::sources::navidrome::NavAction::Command(command).into()]
        }
        PaletteCommandKind::ToggleFullscreen => vec![NavigationAction::ToggleFullscreen.into()],
        PaletteCommandKind::FullscreenArtwork => vec![NavigationAction::FullscreenArtwork.into()],
        PaletteCommandKind::DesktopFullscreen(mode) => {
            vec![NavigationAction::DesktopFullscreen(mode).into()]
        }
        PaletteCommandKind::OpenRadioBrowser => {
            super::sidebar::open_root(LibraryRoot::Radio, state)
        }
        PaletteCommandKind::OpenAiBrowser => {
            state.popups.close_all();
            state.palette.close();
            state.set_view(View::Browse);
            state.tui_sidebar_group = Some(("AI".into(), 0));
            state.category_column_focused = false;
            vec![]
        }
        PaletteCommandKind::PlayRandomAlbums | PaletteCommandKind::LibraryMix => {
            let recipe = if matches!(cmd, PaletteCommandKind::LibraryMix) {
                "library"
            } else {
                "randomAlbum"
            };
            let prefix = match &state.sources.active {
                super::sources::ActiveSource::Folder(_) => "folder-radio",
                super::sources::ActiveSource::Navidrome(_) => "nav-radio",
                super::sources::ActiveSource::None => return vec![],
            };
            vec![RadioAction::PlayStation(format!("{prefix}/{recipe}")).into()]
        }
        PaletteCommandKind::StopRadio => vec![crate::app::action::RadioAction::StopStation.into()],
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
        PaletteCommandKind::GotoQueue => vec![NavigationAction::ToggleQueuePanel.into()],
        PaletteCommandKind::GotoNowPlaying => {
            vec![NavigationAction::SetView(View::NowPlaying).into()]
        }
        PaletteCommandKind::OpenHelp => vec![NavigationAction::SetView(View::Help).into()],
        PaletteCommandKind::OpenSettings => vec![SettingsAction::OpenSettings.into()],
        PaletteCommandKind::OpenSearch => vec![SearchAction::OpenSearchPopup.into()],
        PaletteCommandKind::SaveDescriptionPlaylist => {
            vec![crate::app::sources::audiomuse::Command::PromptSavePlaylist.into()]
        }
        PaletteCommandKind::ExportPlaylist { format, queue } => {
            vec![QueueAction::PromptExportPlaylist { format, queue }.into()]
        }
        PaletteCommandKind::SaveQueue => vec![QueueAction::PromptSavePlaylist.into()],
        PaletteCommandKind::ClearQueue => vec![QueueAction::ClearQueue.into()],
        PaletteCommandKind::ToggleFilter => vec![SearchAction::ActivateListFilter.into()],
        PaletteCommandKind::ToggleSplitView => vec![SettingsAction::ToggleSplitView.into()],
        PaletteCommandKind::Refresh => crate::app::handlers::helpers::refresh_current_view(state),
        PaletteCommandKind::ArtistDetail(command) => {
            if super::details::focused_artist(state)
                .is_some_and(|info| super::details::artist_commands(info).contains(&command))
            {
                vec![command.action().into()]
            } else {
                vec![]
            }
        }
        PaletteCommandKind::StopPlayback => vec![PlaybackAction::Stop.into()],
        PaletteCommandKind::PlayPause => vec![PlaybackAction::TogglePlayPause.into()],
        PaletteCommandKind::NextTrack => vec![PlaybackAction::Next.into()],
        PaletteCommandKind::PrevTrack => vec![PlaybackAction::Previous.into()],
        PaletteCommandKind::SwitchLibrary => vec![SearchAction::OpenLibraryPicker.into()],
        PaletteCommandKind::VolumeUp => vec![PlaybackAction::VolumeUp.into()],
        PaletteCommandKind::VolumeDown => vec![PlaybackAction::VolumeDown.into()],
        PaletteCommandKind::ToggleMute => vec![PlaybackAction::ToggleMute.into()],
        PaletteCommandKind::ToggleAutoPlay => vec![PlaybackAction::ToggleAutoPlay.into()],
        PaletteCommandKind::ToggleAutoMix => vec![PlaybackAction::ToggleAutoMix.into()],
        PaletteCommandKind::SetQueueRepeat(mode) => vec![QueueAction::SetRepeat(mode).into()],
        PaletteCommandKind::ShowLyrics => vec![PlaybackAction::ShowLyrics.into()],
        PaletteCommandKind::SeekForward => vec![PlaybackAction::SeekRelative(10_000).into()],
        PaletteCommandKind::SeekBackward => vec![PlaybackAction::SeekRelative(-10_000).into()],
        PaletteCommandKind::UndoQueueEdit => vec![NavigationAction::Undo.into()],
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
        PaletteCommandKind::RemixGemini => {
            super::queue_controls::activate(state, super::presentation::NpSidebarButton::Twins)
        }
        PaletteCommandKind::RemixTwofer => {
            super::queue_controls::activate(state, super::presentation::NpSidebarButton::Twofer)
        }
        PaletteCommandKind::RemixStretch => {
            super::queue_controls::activate(state, super::presentation::NpSidebarButton::Stretch)
        }
        PaletteCommandKind::RemixDoppelganger => super::queue_controls::activate(
            state,
            super::presentation::NpSidebarButton::Doppelganger,
        ),
        PaletteCommandKind::RemixOppositeDay => super::queue_controls::activate(
            state,
            super::presentation::NpSidebarButton::OppositeDay,
        ),
        PaletteCommandKind::RemixEnemies => {
            super::queue_controls::activate(state, super::presentation::NpSidebarButton::Enemies)
        }
        PaletteCommandKind::RemixShuffle => vec![QueueAction::RemixShuffle.into()],
        PaletteCommandKind::RemixUndoShuffle => vec![QueueAction::RemixUndoShuffle.into()],
        PaletteCommandKind::ConfigureStation(key) => {
            super::radio_navigation::configure(state, &key)
        }
        PaletteCommandKind::PlayStation(key) => {
            vec![RadioAction::PlayStation(key).into()]
        }
        PaletteCommandKind::BrowseStations { key, title } => {
            let mut actions = open_with_query(state, "Radio");
            actions.push(RadioAction::DrillIntoStation(key, title).into());
            actions
        }
        PaletteCommandKind::StationsBack => {
            let mut actions = open_with_query(state, "Radio");
            actions.push(RadioAction::NavigateStationsBack.into());
            actions
        }
        PaletteCommandKind::ArtistRadio => vec![SearchAction::OpenArtistRadioPicker.into()],
        PaletteCommandKind::GoToRandomAlbum => {
            if !crate::app::sources::command_visible(state, &PaletteCommandKind::GoToRandomAlbum) {
                state.set_status("This library does not provide an album catalog".into());
                return vec![];
            }
            let Some(album) = state.library.albums.iter().choose(&mut rand::rng()) else {
                state.set_status("No albums available in this library".into());
                return vec![];
            };
            vec![
                BrowseAction::OpenDetailsInLibrary {
                    track: Box::new(super::library_details::album_target(album)),
                    destination: super::library_details::Destination::Album,
                }
                .into(),
            ]
        }
        PaletteCommandKind::JumpToArtist(track) => vec![
            BrowseAction::OpenDetailsInLibrary {
                track,
                destination: crate::app::library_details::Destination::Artist,
            }
            .into(),
        ],
        PaletteCommandKind::JumpToAlbum(track) => vec![
            BrowseAction::OpenDetailsInLibrary {
                track,
                destination: crate::app::library_details::Destination::Album,
            }
            .into(),
        ],
        PaletteCommandKind::OpenInLibrary => super::library_details::selected_destination(state)
            .map_or_else(Vec::new, |(track, destination)| {
                vec![
                    BrowseAction::OpenDetailsInLibrary {
                        track: Box::new(track),
                        destination,
                    }
                    .into(),
                ]
            }),
        PaletteCommandKind::OpenSort => vec![SearchAction::OpenSortPopup.into()],
        PaletteCommandKind::ToggleCompilationArtists => {
            vec![
                SettingsAction::SetShowCompilationOnlyArtists(!state.show_compilation_only_artists)
                    .into(),
            ]
        }
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
            vec![
                SystemAction::OpenExternalSearch {
                    target: crate::services::external_search::SearchTarget::AppleMusic,
                    query: super::external_search::query(state),
                }
                .into(),
            ]
        }
        PaletteCommandKind::SearchSpotify => {
            vec![
                SystemAction::OpenExternalSearch {
                    target: crate::services::external_search::SearchTarget::Spotify,
                    query: super::external_search::query(state),
                }
                .into(),
            ]
        }
        PaletteCommandKind::SearchYouTube => {
            vec![
                SystemAction::OpenExternalSearch {
                    target: crate::services::external_search::SearchTarget::YouTube,
                    query: super::external_search::query(state),
                }
                .into(),
            ]
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
                Some((artist_key, artist_name)) if state.sources.active.folder().is_some() => {
                    vec![
                        SearchAction::ShowArtistBio {
                            artist_key,
                            artist_name,
                        }
                        .into(),
                    ]
                }
                Some((artist_key, artist_name)) => vec![
                    BrowseAction::OpenDetailsInLibrary {
                        track: Box::new(crate::library::models::Track {
                            grandparent_rating_key: Some(artist_key),
                            grandparent_title: Some(artist_name),
                            ..Default::default()
                        }),
                        destination: crate::app::library_details::Destination::Artist,
                    }
                    .into(),
                ],
                None => {
                    state.set_status("No artist selected".into());
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
            Some(track) => vec![
                SearchAction::OpenAdventureLauncherWithStart {
                    start_track: Box::new(track),
                }
                .into(),
            ],
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
            // `app::track_context::track_context_entries`,
            // both UIs pick it up automatically — the only
            // per-UI work is this kind-to-dispatch table.
            use crate::app::action::{BrowseAction, QueueAction, SearchAction, SystemAction};

            use crate::app::track_context::ContextKind;
            match kind {
                ContextKind::Separator => vec![],
                ContextKind::Download => {
                    vec![crate::app::downloads::Command::Tracks(vec![*track]).into()]
                }
                ContextKind::PlayTrack => vec![QueueAction::PlayTrack(track).into()],
                ContextKind::PlayTrackAndFollowing => play_focused_track(state, false),
                ContextKind::PlayFollowingNext | ContextKind::AddFollowing => {
                    crate::app::playback_controls::track_action(
                        state,
                        &track,
                        crate::app::playback_controls::TrackRange::Following,
                        if matches!(kind, ContextKind::PlayFollowingNext) {
                            crate::app::playback_controls::QueuePlacement::Next
                        } else {
                            crate::app::playback_controls::QueuePlacement::End
                        },
                    )
                }
                ContextKind::ShowDetails(destination) => {
                    vec![BrowseAction::OpenDetailsInLibrary { track, destination }.into()]
                }
                ContextKind::PlayNextInQueue => {
                    vec![QueueAction::EnqueueTracksNext(vec![*track]).into()]
                }
                ContextKind::AddToPlaylist => vec![
                    crate::app::sources::navidrome::NavAction::Command(
                        crate::app::sources::navidrome::commands::Command::ChoosePlaylist(
                            crate::app::sources::navidrome::playlist_edit::Addition::Track(
                                track.rating_key.clone(),
                            ),
                        ),
                    )
                    .into(),
                ],
                ContextKind::AddToEndOfQueue => vec![QueueAction::EnqueueTrack(track).into()],
                ContextKind::OpenInLibrary => vec![
                    BrowseAction::OpenDetailsInLibrary {
                        track,
                        destination: crate::app::library_details::Destination::Track,
                    }
                    .into(),
                ],
                ContextKind::SonicAdventure => {
                    vec![SearchAction::OpenAdventureLauncherWithStart { start_track: track }.into()]
                }
                ContextKind::SonicRadio => {
                    vec![RadioAction::StartSonicRadio(track).into()]
                }
                ContextKind::ArtistBio { .. } => vec![
                    BrowseAction::OpenDetailsInLibrary {
                        track,
                        destination: crate::app::library_details::Destination::Artist,
                    }
                    .into(),
                ],
                ContextKind::SearchExternal(target) => vec![
                    SystemAction::OpenExternalSearch {
                        target,
                        query: Some(super::external_search::track_query(&track)),
                    }
                    .into(),
                ],
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
        vec![
            QueueAction::PlayTracksNow(super::downloads::ready_selection(state, &tracks[index..]))
                .into(),
        ]
    }
}

/// What the input handler decided to do with this key.
pub enum PaletteOutcome {
    Continue,
    Execute(PaletteCommandKind),
    Cancel,
}

/// Open the palette with no query.
pub fn open(state: &mut AppState) -> Vec<Action> {
    open_with_query(state, "")
}

/// Open the palette with a pre-typed query. Used by the now-playing
/// sidebar buttons.
pub fn open_with_query(state: &mut AppState, q: &str) -> Vec<Action> {
    let changed = !state.palette.open && state.palette.history.opened();
    state.palette.path.clear();
    state.palette.open = true;
    state.palette.query = q.to_string();
    state.palette.cursor = q.chars().count();
    state.palette.selected = 0;
    refresh_matches(state);
    if changed {
        vec![SettingsAction::PersistCommandHistory.into()]
    } else {
        vec![]
    }
}

/// Recompute `entries` (snapshot of current candidates) and `matches`
/// (filtered + sorted indices into `entries`) for the current query.
pub fn refresh_matches(state: &mut AppState) {
    let mut entries = super::sources::navidrome::playlist_edit::choices(state)
        .unwrap_or_else(|| materialize_entries(state));
    for entry in &mut entries {
        super::command_catalog::prepare(entry);
    }
    let groups = super::command_catalog::groups(
        entries
            .iter()
            .map(|e| super::command_catalog::path(&e.command)),
    );
    for path in groups {
        entries.push(PaletteEntry {
            label: path.last().unwrap().clone(),
            hint: "›".into(),
            command: PaletteCommandKind::OpenCommandGroup(path),
            aliases: vec![],
        });
    }
    if !state.palette.path.is_empty() {
        entries.push(PaletteEntry {
            label: "Back".into(),
            hint: "←".into(),
            command: PaletteCommandKind::OpenCommandGroup(
                state.palette.path[..state.palette.path.len() - 1].to_vec(),
            ),
            aliases: vec![],
        });
    }
    if state.palette.path.is_empty() {
        entries.sort_by_key(|entry| {
            super::command_catalog::palette_priority(
                &entry.command,
                super::command_catalog::disabled_reason(state, &entry.command).is_none(),
                &state.palette.history,
            )
        });
    }
    let search = crate::services::CommandSearch::new(&state.palette.query);
    let mut matches: Vec<usize> = if search.text().is_empty() {
        entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                let available =
                    super::command_catalog::disabled_reason(state, &entry.command).is_none();
                let promoted = state.palette.path.is_empty()
                    && super::command_catalog::palette_priority(
                        &entry.command,
                        available,
                        &state.palette.history,
                    ) != super::command_catalog::PalettePriority::Menu;
                promoted
                    || super::command_catalog::visible(
                        &entry.command,
                        &entry.label,
                        &state.palette.path,
                        false,
                    )
            })
            .map(|(index, _)| index)
            .collect()
    } else {
        let mut scored: Vec<_> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                super::command_catalog::visible(
                    &entry.command,
                    &entry.label,
                    &state.palette.path,
                    true,
                )
            })
            .filter_map(|(index, entry)| {
                search
                    .score(&entry.label, entry.aliases.iter().map(String::as_str))
                    .map(|score| (score, index))
            })
            .collect();
        scored.sort_by_key(|item| std::cmp::Reverse(item.0));
        scored.into_iter().map(|(_, i)| i).collect()
    };
    if matches.is_empty()
        && let Some(i) = entries.iter().position(|entry| {
            super::sources::navidrome::playlist_edit::create_choice(&entry.command)
        })
    {
        matches.push(i);
    }
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
        K::Left if state.palette.query.is_empty() && !state.palette.path.is_empty() => {
            run(
                PaletteCommandKind::OpenCommandGroup(
                    state.palette.path[..state.palette.path.len() - 1].to_vec(),
                ),
                state,
            );
            return PaletteOutcome::Continue;
        }
        K::Enter | K::Right if key.code == K::Enter || state.palette.query.is_empty() => {
            if let Some(&idx) = state.palette.matches.get(state.palette.selected)
                && let Some(entry) = state.palette.entries.get(idx)
            {
                if super::command_catalog::disabled_reason(state, &entry.command).is_some() {
                    return PaletteOutcome::Continue;
                }
                let command = entry.command.clone();
                if matches!(command, PaletteCommandKind::OpenCommandGroup(_)) {
                    run(command, state);
                    return PaletteOutcome::Continue;
                }
                if key.code == K::Right {
                    return PaletteOutcome::Continue;
                }
                return PaletteOutcome::Execute(command);
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

/// Resolve the captured highlighted track; never redirect a missing destination to playback.
#[cfg(test)]
mod presentation_tests {
    use super::*;
    #[test]
    fn palette_search_matches_catalog_typos_and_hidden_synonyms() {
        let mut state = AppState::new();
        for query in ["settigns", "preferences"] {
            open_with_query(&mut state, query);
            let first = &state.palette.entries[state.palette.matches[0]];
            assert!(matches!(first.command, PaletteCommandKind::OpenSettings));
            assert_eq!(first.label, "Settings…");
        }

        state.view = View::Queue;
        open_with_query(&mut state, "swap for similar songs");
        let first = &state.palette.entries[state.palette.matches[0]];
        assert!(matches!(
            first.command,
            PaletteCommandKind::RemixDoppelganger
        ));
        assert_eq!(first.label, "Doppelganger");
    }

    #[cfg(all(feature = "desktop", target_os = "macos"))]
    #[tokio::test]
    async fn mode_cycle_uses_the_pending_destination_for_rapid_presses() {
        let profile = std::env::var_os("TEXTAMP_CARGO_TEST_PROFILE")
            .expect("Run settings persistence checks through the isolated Cargo test runner");
        assert!(crate::config::config_path().starts_with(std::path::PathBuf::from(profile)));
        use crate::app::launch::ApplicationMode;
        let mut state = AppState::new();
        state.windowed_presentation = crate::app::windowed_presentation::Presentation::windowed();
        let mut config = crate::config::Config::default();
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        let (events, _inbox) = tokio::sync::mpsc::channel(32);
        for expected in [
            ApplicationMode::Text,
            ApplicationMode::GuiMultiple,
            ApplicationMode::GuiSingle,
            ApplicationMode::Text,
            ApplicationMode::GuiMultiple,
            ApplicationMode::GuiSingle,
        ] {
            for action in run(PaletteCommandKind::CycleApplicationMode, &mut state) {
                crate::app::dispatch::dispatch_action(
                    action,
                    &mut state,
                    &mut audio,
                    &mut config,
                    &events,
                )
                .await
                .unwrap();
            }
            assert_eq!(state.application_mode(), Some(expected));
        }
    }
    #[test]
    fn external_terminal_omits_windowed_switch_commands() {
        let state = AppState::new();
        assert!(!crate::app::sources::command_visible(
            &state,
            &PaletteCommandKind::CycleApplicationMode
        ));
        for mode in crate::app::launch::ApplicationMode::ALL {
            assert!(!crate::app::sources::command_visible(
                &state,
                &PaletteCommandKind::SetApplicationMode(mode)
            ));
        }
    }
}
