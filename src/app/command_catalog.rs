//! Shared command names, menu placement and searchable palette hierarchy.
use crate::app::state::{PaletteCommandKind, PaletteEntry};
/// Unavailable commands remain discoverable but cannot be activated.
pub fn disabled_reason(
    state: &super::AppState,
    command: &PaletteCommandKind,
) -> Option<&'static str> {
    if let PaletteCommandKind::QueueTool(tool) = command {
        return super::queue_controls::definition(state, *tool).disabled_reason;
    }
    use super::{presentation::NpSidebarButton as Tool, state::*};
    use PaletteCommandKind::*;
    if let ConfigureStation(key) = command {
        return disabled_reason(state, &PlayStation(key.clone()));
    }
    if super::sources::audiomuse::connection(state).is_none() && analysis_command(command) {
        return Some("Connect an AudioMuse account for this library");
    }
    if let Some(target) = super::external_search::command_target(command) {
        return super::external_search::disabled_reason(state, target);
    }
    if !super::sources::command_visible(state, command) {
        return Some("Unavailable for this library");
    }
    let tool = match command {
        SaveQueue => Some(Tool::Save),
        ToggleAutoMix => Some(Tool::AutoMix),
        ClearQueue => Some(Tool::ClearQueue),
        UndoQueueEdit => Some(Tool::Undo),
        RemixGemini => Some(Tool::Twins),
        RemixTwofer => Some(Tool::Twofer),
        RemixStretch => Some(Tool::Stretch),
        RemixDoppelganger => Some(Tool::Doppelganger),
        RemixOppositeDay => Some(Tool::OppositeDay),
        RemixEnemies => Some(Tool::Enemies),
        _ => None,
    };
    if let Some(tool) = tool {
        return super::queue_controls::definition(state, tool).disabled_reason;
    }
    let column = (state.view == View::Browse && !state.category_column_focused)
        .then(|| state.browse_nav().and_then(|nav| nav.focused()))
        .flatten();
    let column_kind = column.and_then(|_| {
        state
            .browse_nav()
            .and_then(|nav| state.column_options_type(nav.focused_column))
    });
    let target = || {
        state
            .palette_target_track()
            .or_else(|| state.current_track().cloned())
    };
    let queue_len = state.playback_tracks().len();
    match command {
        PlayerSync(menu) => super::player_sync::disabled(state, *menu),
        LyricsControl(_) if state.popups.lyrics().is_none() => Some("Open lyrics first"),
        LyricsControl(super::lyrics::Control::Search)
            if state.popups.lyrics().is_some_and(|panel| {
                !panel.documents().is_empty()
                    || matches!(panel.content, super::lyrics::Content::Loading)
            }) =>
        {
            Some("Lyrics are available in the reader")
        }
        LyricsControl(super::lyrics::Control::NextLanguage)
            if state
                .popups
                .lyrics()
                .is_none_or(|panel| panel.documents().len() < 2) =>
        {
            Some("No other lyrics language")
        }
        LyricsControl(super::lyrics::Control::ToggleSync)
            if state
                .popups
                .lyrics()
                .and_then(|panel| panel.documents().get(panel.language))
                .is_none_or(|document| !document.synced) =>
        {
            Some("No synchronized lyrics")
        }
        Navidrome(_) if state.downloads.is_offline() => Some("Unavailable offline"),
        Navidrome(
            super::sources::navidrome::commands::Command::SaveQueue
            | super::sources::navidrome::commands::Command::ConfirmReplacePlaylist(_)
            | super::sources::navidrome::commands::Command::ReplacePlaylist(_),
        ) if queue_len == 0 => Some("The queue is empty"),
        Downloads(menu) => menu.disabled_reason(state),
        Refresh | OpenSearch | ArtistRadio | SonicAdventure | SonicAdventureFromFocusedTrack
            if state.active_library.is_none() =>
        {
            Some("Choose a library first")
        }
        Refresh
        | ArtistRadio
        | SonicAdventure
        | SonicAdventureFromFocusedTrack
        | PlayStation(_)
        | ConfigureStation(_)
        | BrowseStations { .. }
            if state.downloads.is_offline() =>
        {
            Some("Unavailable offline")
        }
        PlayPause if state.current_track().is_none() && queue_len == 0 => {
            Some("Choose music to play")
        }
        StopPlayback | NextTrack | PrevTrack | ShowLyrics if state.current_track().is_none() => {
            Some("Nothing is playing")
        }
        SeekForward | SeekBackward
            if state.current_track().is_none() || state.playback.duration_ms == 0 =>
        {
            Some("No seekable track")
        }
        VolumeUp if state.playback.volume >= 1.0 => Some("Volume is at maximum"),
        VolumeDown if state.playback.volume <= 0.0 => Some("Volume is at minimum"),
        OpenAiBrowser if super::sidebar::group_rows(state, "AI").is_empty() => {
            Some("AI is unavailable for this library")
        }
        LibraryMix | PlayRandomAlbums
            if matches!(state.sources.active, super::sources::ActiveSource::None) =>
        {
            Some("Choose a library")
        }
        LibraryMix | PlayRandomAlbums
            if state.downloads.is_offline() && state.sources.active.navidrome().is_some() =>
        {
            Some("Unavailable offline")
        }
        GoToRandomAlbum if state.library.albums.is_empty() => Some("No albums available"),
        StopRadio
            if state.playback_mode != PlaybackMode::Radio && state.station_starting.is_none() =>
        {
            Some("Manual Queue is already selected")
        }
        ToggleAutoPlay if state.playback_tracks().is_empty() => {
            Some("Add tracks to the queue first")
        }
        ToggleAutoPlay if state.downloads.is_offline() => Some("Unavailable offline"),
        SetQueueRepeat(mode) if *mode != RepeatMode::Off && queue_len == 0 => {
            Some("The queue is empty")
        }
        RemixShuffle if queue_len < 2 => Some("Add at least two tracks to the queue"),
        RemixUndoShuffle if state.queue.shuffle_undo_queue.is_none() => Some("No shuffle to undo"),
        RemoveFocusedFromQueue | MoveQueueSelectionUp | MoveQueueSelectionDown
            if !matches!(state.view, View::Queue | View::NowPlaying)
                || state.list_state.queue_index >= queue_len =>
        {
            Some("Select a queue item")
        }
        MoveQueueSelectionUp if state.list_state.queue_index == 0 => {
            Some("Already at the beginning")
        }
        MoveQueueSelectionDown if state.list_state.queue_index + 1 >= queue_len => {
            Some("Already at the end")
        }
        ExportPlaylist { queue: true, .. } if queue_len == 0 => Some("The queue is empty"),
        ExportPlaylist { queue: false, .. }
            if !column
                .is_some_and(|c| matches!(c.play_all_row, Some(PlayAllRow::Playlist { .. }))) =>
        {
            Some("Open a playlist first")
        }
        ToggleFilter
        | CloseColumn
        | OpenSort
        | ApplySort(_)
        | ReverseSort
        | ToggleArtwork
        | ToggleCompilationArtists
        | ToggleGroupByAlbum
            if column.is_none()
                && !(state.view == View::Browse
                    && state.browse_category == BrowseCategory::Folders) =>
        {
            Some("Focus a library column")
        }
        OpenSort | ReverseSort if column_kind.is_none() => Some("No sortable column"),
        ApplySort(mode)
            if column_kind.is_none_or(|kind| !kind.available_modes().contains(mode)) =>
        {
            Some("Unavailable for this column")
        }
        ToggleFilter if column.is_some_and(|column| column.details.is_some()) => {
            Some("Focus a list column")
        }
        ToggleArtwork
            if !matches!(
                column_kind,
                Some(
                    SortColumnType::Album
                        | SortColumnType::RecentAlbums
                        | SortColumnType::Track
                        | SortColumnType::AllTracks
                )
            ) =>
        {
            Some("This column has no artwork option")
        }
        ToggleCompilationArtists if column_kind != Some(SortColumnType::Artist) => {
            Some("Focus an artist list")
        }
        ToggleGroupByAlbum
            if !state
                .browse_nav()
                .is_some_and(|nav| super::downloads::is_root_column(state, nav.focused_column))
                && (state.browse_category != BrowseCategory::Playlists
                    || !matches!(
                        column_kind,
                        Some(SortColumnType::AllTracks | SortColumnType::Album)
                    )) =>
        {
            Some("Open Downloads or a playlist’s track list")
        }
        PlayFocusedTrack | PlayFocusedTrackAndFollowing | SonicAdventureFromFocusedTrack
            if state.palette_target_track().is_none() =>
        {
            Some("Select a track")
        }
        PlayFocusedAlbum if state.focused_album().is_none() => Some("Select an album"),
        OpenInLibrary if super::library_details::selected_destination(state).is_none() => {
            Some("Select music first")
        }
        EnqueueSelectionEnd | EnqueueSelectionNext
            if target().is_none() && state.focused_album().is_none() =>
        {
            Some("Select music first")
        }
        ShowArtistBio if super::external_search::subject(state).is_none() => {
            Some("Select music first")
        }
        JumpToArtist(track)
            if track.grandparent_rating_key.is_none() && track.artist_name().is_empty() =>
        {
            Some("No artist available")
        }
        JumpToAlbum(track) if track.parent_rating_key.is_none() => Some("No album available"),
        FromTrackContext {
            kind: super::track_context::ContextKind::Download,
            ..
        } if state.downloads.is_offline() => Some("Unavailable offline"),
        _ => None,
    }
}

pub fn menu(command: &PaletteCommandKind) -> &'static str {
    use PaletteCommandKind::*;
    match command {
        FromTrackContext { kind, .. } => {
            use super::track_context::ContextKind as K;
            match kind {
                K::PlayTrack | K::PlayTrackAndFollowing => "Playback",
                K::PlayFollowingNext
                | K::AddFollowing
                | K::PlayNextInQueue
                | K::AddToEndOfQueue => "Queue",
                K::AddToPlaylist => "Edit",
                K::Download => "File",
                K::OpenInLibrary | K::ShowDetails(_) => "View",
                _ => "Discover",
            }
        }
        LyricsControl(_) | PlayFocusedTrack | PlayFocusedTrackAndFollowing | PlayFocusedAlbum => {
            "Playback"
        }
        SearchAppleMusic | SearchSpotify | SearchYouTube | ArtistDetail(_) => "Discover",
        Navidrome(
            super::sources::navidrome::commands::Command::ChoosePlaylist(_)
            | super::sources::navidrome::commands::Command::EditEntry { .. },
        ) => "Edit",
        PlayerSync(_)
        | Navidrome(super::sources::navidrome::commands::Command::PromptNewPlaylist(None)) => {
            "File"
        }
        Quit | OpenSettings | SwitchLibrary | Refresh => "Textamp",
        Downloads(_) => "File",
        QueueTool(_) => "Queue",
        SaveQueue | ExportPlaylist { .. } => "File",
        VolumeUp | VolumeDown | SeekForward | SeekBackward | PlayPause | StopPlayback
        | NextTrack | PrevTrack | ToggleMute | ShowLyrics => "Playback",
        RemixGemini
        | RemixTwofer
        | RemixStretch
        | RemixDoppelganger
        | RemixOppositeDay
        | RemixEnemies
        | RemixShuffle
        | RemixUndoShuffle
        | ClearQueue
        | UndoQueueEdit
        | MoveQueueSelectionUp
        | MoveQueueSelectionDown
        | ToggleAutoPlay
        | ConfigureStation(_)
        | StopRadio
        | PlayStation(_)
        | BrowseStations { .. }
        | ToggleAutoMix
        | SetQueueRepeat(_)
        | EnqueueSelectionEnd
        | EnqueueSelectionNext
        | RemoveFocusedFromQueue => "Queue",
        Navidrome(
            crate::app::sources::navidrome::commands::Command::SaveQueue
            | crate::app::sources::navidrome::commands::Command::RestoreQueue,
        ) => "Queue",
        OpenAiBrowser
        | PlayRandomAlbums
        | LibraryMix
        | GoToRandomAlbum
        | ArtistRadio
        | OpenRadioBrowser
        | SonicAdventure
        | SonicAdventureFromFocusedTrack
        | ShowArtistBio
        | Navidrome(_) => "Discover",
        OpenHelp => "Help",
        _ => "View",
    }
}

pub fn submenu_for(menu: &str, command: &PaletteCommandKind) -> Option<&'static str> {
    use crate::app::sources::navidrome::commands::{CollectionKind, Command};
    use PaletteCommandKind::*;
    match (menu, command) {
        ("File", PlayerSync(_)) => Some("Devices"),
        ("Playback", ShowLyrics | LyricsControl(_)) => Some("Lyrics"),
        (
            "File",
            FromTrackContext {
                kind: super::track_context::ContextKind::Download,
                ..
            },
        ) => Some("Downloads"),
        ("View", FromTrackContext { .. } | JumpToArtist(_) | JumpToAlbum(_)) => Some("Browse"),
        (
            "Playback",
            FromTrackContext { .. }
            | PlayFocusedTrack
            | PlayFocusedTrackAndFollowing
            | PlayFocusedAlbum,
        ) => Some("Selection"),
        (
            "Queue",
            FromTrackContext { .. }
            | EnqueueSelectionEnd
            | EnqueueSelectionNext
            | RemoveFocusedFromQueue,
        ) => Some("Selection"),
        ("Discover", SearchAppleMusic | SearchSpotify | SearchYouTube) => Some("Search"),
        ("Discover", FromTrackContext { .. } | ArtistDetail(_) | SaveDescriptionPlaylist) => {
            Some("Selection")
        }
        ("File", Downloads(_)) => Some("Downloads"),
        ("File", ExportPlaylist { queue: true, .. }) => Some("Export Queue"),
        ("File", ExportPlaylist { queue: false, .. }) => Some("Export Playlist"),
        ("View", SetApplicationMode(_)) => Some("Modes"),
        ("View", ApplySort(_) | ReverseSort | ToggleGroupByAlbum | OpenSort) => Some("Sort"),
        ("View", ToggleFilter | ToggleArtwork | ToggleCompilationArtists | ToggleSplitView) => {
            Some("View Options")
        }
        ("View", GotoGenres | GotoFolders | GotoSingles | OpenInLibrary | GotoQueue) => {
            Some("Browse")
        }
        ("View", ToggleFullscreen | FullscreenArtwork | DesktopFullscreen(_)) => {
            Some("Full Screen")
        }
        ("Playback", VolumeUp | VolumeDown | ToggleMute) => Some("Volume"),
        ("Playback", SeekForward | SeekBackward) => Some("Seek"),
        (
            "Queue",
            RemixGemini | RemixTwofer | RemixStretch | RemixDoppelganger | RemixOppositeDay
            | RemixEnemies | RemixShuffle | RemixUndoShuffle,
        ) => Some("Queue Edits"),
        (
            "Queue",
            ToggleAutoPlay
            | StopRadio
            | PlayStation(_)
            | ConfigureStation(_)
            | BrowseStations { .. },
        ) => Some("Queue Type"),
        ("Queue", MoveQueueSelectionUp | MoveQueueSelectionDown) => Some("Reorder"),
        ("Queue", Navidrome(Command::SaveQueue | Command::RestoreQueue)) => Some("Saved Queue"),
        ("Discover", Navidrome(Command::Collection(CollectionKind::AudioMuse(_)))) => Some("Search"),

        _ => None,
    }
}

/// Available selection-specific commands appear directly at the palette root.
pub fn contextual(command: &PaletteCommandKind) -> bool {
    use PaletteCommandKind::*;
    matches!(
        command,
        LyricsControl(_)
            | FromTrackContext { .. }
            | Downloads(
                super::downloads::Menu::Selection
                    | super::downloads::Menu::Remove
                    | super::downloads::Menu::CancelSelection
            )
            | ArtistDetail(_)
            | PlayFocusedAlbum
            | ShowArtistBio
            | EnqueueSelectionEnd
            | EnqueueSelectionNext
            | PlayFocusedTrack
            | PlayFocusedTrackAndFollowing
            | JumpToArtist(_)
            | JumpToAlbum(_)
            | SearchAppleMusic
            | SearchSpotify
            | SearchYouTube
            | SonicAdventureFromFocusedTrack
            | SaveDescriptionPlaylist
            | OpenInLibrary
            | RemoveFocusedFromQueue
            | MoveQueueSelectionUp
            | MoveQueueSelectionDown
            | Navidrome(
                super::sources::navidrome::commands::Command::Star(_)
                    | super::sources::navidrome::commands::Command::Rating(_)
                    | super::sources::navidrome::commands::Command::ChoosePlaylist(_)
                    | super::sources::navidrome::commands::Command::EditEntry { .. }
                    | super::sources::navidrome::commands::Command::RenamePlaylist(_)
                    | super::sources::navidrome::commands::Command::ConfirmDeletePlaylist(_)
                    | super::sources::navidrome::commands::Command::ConfirmReplacePlaylist(_)
            )
    )
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PalettePriority {
    Recent(usize),
    Contextual,
    Menu,
}

/// Root shortcuts precede the canonical menu hierarchy; unavailable recents never float.
pub fn palette_priority(
    command: &PaletteCommandKind,
    available: bool,
    history: &super::command_history::History,
) -> PalettePriority {
    if available
        && let Some(rank) = super::command_history::key(command).and_then(|key| history.rank(&key))
    {
        return PalettePriority::Recent(rank);
    }
    if available && contextual(command) {
        return PalettePriority::Contextual;
    }
    PalettePriority::Menu
}

pub fn path(command: &PaletteCommandKind) -> Vec<String> {
    if matches!(
        command,
        PaletteCommandKind::Navidrome(
            super::sources::navidrome::commands::Command::Append { .. }
                | super::sources::navidrome::commands::Command::PromptNewPlaylist(Some(_))
        )
    ) {
        return vec![super::sources::navidrome::playlist_edit::PICKER.into()];
    }
    if matches!(command, PaletteCommandKind::QueueTool(_)) {
        return vec![
            "Queue".into(),
            super::queue_controls::Menu::Tools.label().into(),
        ];
    }
    use PaletteCommandKind::*;
    match command {
        OpenCommandGroup(_) => return vec![],
        StationsBack => {
            return vec!["Discover".into(), "Radio Stations".into()];
        }
        _ => {}
    }
    let menu = menu(command);
    let mut path = vec![menu.into()];
    if let Some(submenu) = submenu_for(menu, command) {
        path.push(submenu.into());
    }
    path
}

pub fn label(command: &PaletteCommandKind, fallback: &str) -> String {
    use PaletteCommandKind::*;
    match command {
        GotoQueue => "Play Queue",
        GoToRandomAlbum => "Go to Random Album",
        OpenSearch => "Search…",
        OpenSettings => "Settings…",
        OpenHelp => "Textamp Help",
        Refresh => "Refresh Library",
        SwitchLibrary => "Switch Library…",
        SaveQueue => "Save Queue as Playlist…",
        OpenSort => "View Options…",
        ToggleFilter => "Filter Column…",
        ToggleArtwork => "Show Artwork",
        ToggleGroupByAlbum => "Group by Album",
        ToggleSplitView => "Split View",
        ToggleFullscreen => "Toggle Full Screen",
        ShowArtistBio => "Show Artist Details",
        EnqueueSelectionEnd => "Add to Queue",
        EnqueueSelectionNext => "Play Next",
        PlayPause => "Play/Pause",
        StopPlayback => "Stop",
        ToggleMute => "Mute",
        UndoQueueEdit => "Undo Queue Edit",
        ArtistRadio => "Artist Mix…",
        OpenRadioBrowser => "Radio",
        StopRadio => "Manual Queue",
        StationsBack => "Parent Stations",
        ExportPlaylist {
            format: crate::services::playlist_export::Format::M3u8,
            ..
        } => "M3U8…",
        ExportPlaylist { .. } => "CSV…",
        BrowseStations { .. } | PlayStation(_) | ConfigureStation(_) => {
            return station_label(fallback);
        }
        _ => {
            return [
                "Remix: ",
                "Sort: ",
                "Radio: ",
                "Full-screen ",
                "Seek: ",
                "Volume: ",
            ]
            .iter()
            .find_map(|prefix| fallback.strip_prefix(prefix))
            .unwrap_or(fallback)
            .into();
        }
    }
    .into()
}

fn synonyms(command: &PaletteCommandKind) -> &'static [&'static str] {
    use super::{
        details::ArtistCommand,
        downloads::Menu,
        presentation::NpSidebarButton as Tool,
        sources::navidrome::commands::{CollectionKind, Command},
        state::ColumnSortMode,
        track_context::ContextKind,
    };
    use PaletteCommandKind::*;
    match command {
        ApplySort(ColumnSortMode::ByArtist) => &["performer order", "artist alphabetical"],
        ApplySort(ColumnSortMode::ByAlbum) => &["record title order", "album alphabetical"],
        RemixGemini | QueueTool(Tool::Twins) => &["pair similar songs", "add sonic matches"],
        RemixTwofer | QueueTool(Tool::Twofer) => &["same artist pairs", "two songs per artist"],
        RemixStretch | QueueTool(Tool::Stretch) => &["bridge tracks", "fill gaps"],
        RemixDoppelganger | QueueTool(Tool::Doppelganger) => {
            &["swap for similar songs", "sonic replacements"]
        }
        RemixOppositeDay | QueueTool(Tool::OppositeDay) => &["replace with contrasting songs"],
        RemixEnemies | QueueTool(Tool::Enemies) => &["alternate contrasting songs"],
        QueueTool(Tool::Shuffle) => &["scramble queue", "randomize queue"],
        QueueTool(Tool::Repeat) => &["loop playback", "cycle repeat"],
        QueueTool(Tool::AutoMix) | ToggleAutoMix => &["crossfade", "blend transitions"],
        QueueTool(Tool::ClearQueue) => &["empty queue", "wipe queue"],
        QueueTool(Tool::Save) => &["make playlist", "save lineup"],
        QueueTool(Tool::Undo) => &["revert queue change", "restore previous queue"],
        FullscreenArtwork => &["enlarge cover", "album cover only"],
        ToggleFullscreen | DesktopFullscreen(_) => &["maximize player", "immersive playback"],
        OpenRadioBrowser => &["station directory", "choose station"],
        PlayStation(_) => &["start station", "tune in"],
        ConfigureStation(_) => &["set up station", "choose radio seed"],
        BrowseStations { .. } => &["station categories", "browse station directory"],
        JumpToArtist(_) => &["show performer in library", "reveal artist"],
        JumpToAlbum(_) => &["show record in library", "reveal album"],
        ToggleAutoPlay => &["keep playing", "auto queue similar songs"],
        ShowLyrics => &["song words", "read lyrics"],
        SearchAppleMusic => &["find in itunes", "search apple catalog"],
        SearchSpotify => &["look up on spotify", "open spotify results"],
        SearchYouTube => &["find music video", "search videos"],
        ArtistDetail(ArtistCommand::PreviousPhoto) => &["back one image"],
        ArtistDetail(ArtistCommand::NextPhoto) => &["show next picture"],
        ArtistDetail(ArtistCommand::OpenSource) => &["open biography website", "view citation"],
        ArtistDetail(ArtistCommand::SearchWeb) => &["look up artist online"],
        Downloads(menu) => match menu {
            Menu::Open => &["offline music", "local copies"],
            Menu::Selection => &["save selected music offline"],
            Menu::Queue => &["save queue offline"],
            Menu::Manage => &["download manager", "manage storage"],
            Menu::Remove => &["delete local copy", "free space"],
            Menu::CancelSelection => &["stop this download"],
            Menu::Cancel => &["stop all transfers"],
            Menu::Clear => &["clear offline storage"],
            Menu::Retry => &["resume failed transfers"],
            Menu::Offline => &["work offline", "use local copies"],
        },
        Navidrome(command) => match command {
            Command::ChoosePlaylist(_) | Command::Append { .. } => &["add to playlist"],
            Command::PromptNewPlaylist(_) | Command::CreatePlaylist { .. } => {
                &["create new playlist"]
            }
            Command::EditEntry { .. } => &["edit playlist"],
            Command::SaveQueue => &["save playback to server", "sync queue"],
            Command::RestoreQueue => &["load saved queue", "resume server queue"],
            Command::Collection(CollectionKind::RandomMix) => &["shuffle library", "random tracks"],
            Command::Collection(CollectionKind::AudioMuse(_)) => {
                &["music analysis", "discover by sound"]
            }
            Command::Collection(_) => &["library collection", "browse server catalog"],
            Command::Analysis => &["audio analysis settings"],
            Command::Star(true) => &["favorite", "like"],
            Command::Star(false) => &["unfavorite", "unlike"],
            Command::Rating(_) => &["stars", "score"],
            Command::Lyrics => &["song words"],
            Command::RenamePlaylist(_) | Command::SetPlaylistName { .. } => {
                &["change playlist name"]
            }
            Command::ConfirmDeletePlaylist(_) | Command::DeletePlaylist(_) => &["remove playlist"],
            Command::ConfirmReplacePlaylist(_) | Command::ReplacePlaylist(_) => {
                &["overwrite playlist"]
            }
        },
        FromTrackContext { kind, .. } => match kind {
            ContextKind::Separator => &[],
            ContextKind::PlayTrack => &["play song", "start this track"],
            ContextKind::AddToPlaylist => &["add song to playlist"],
            ContextKind::Download => &["save song offline", "keep local copy"],
            ContextKind::PlayTrackAndFollowing => &["play from here", "play rest of list"],
            ContextKind::PlayFollowingNext => &["queue rest next", "play remainder next"],
            ContextKind::AddFollowing => &["append rest of list"],
            ContextKind::PlayNextInQueue => &["queue next", "insert after current"],
            ContextKind::AddToEndOfQueue => &["append song", "enqueue track"],
            ContextKind::OpenInLibrary => &["reveal song", "locate in library"],
            ContextKind::ShowDetails(_) => &["metadata", "information"],
            ContextKind::SonicAdventure => &["start journey here", "connect songs"],
            ContextKind::SonicRadio => &["station from this song", "similar tracks radio"],
            ContextKind::ArtistBio { .. } => &["artist biography", "about performer"],
            ContextKind::SearchExternal(_) => &["look up track online"],
        },
        _ => &[],
    }
}

pub fn prepare(entry: &mut PaletteEntry) {
    let old = entry.label.clone();
    entry.label = label(&entry.command, &old);
    entry
        .aliases
        .extend(synonyms(&entry.command).iter().map(|alias| (*alias).into()));
    if old != entry.label {
        entry.aliases.push(old);
    }
    let mut full_path = path(&entry.command);
    full_path.push(entry.label.clone());
    let qualified = full_path.join(" ");
    if qualified != entry.label {
        entry.aliases.push(qualified);
    }
}

pub fn groups(paths: impl IntoIterator<Item = Vec<String>>) -> Vec<Vec<String>> {
    let mut groups = Vec::new();
    for path in paths {
        for depth in 1..=path.len() {
            let group = path[..depth].to_vec();
            if !groups.contains(&group) {
                groups.push(group);
            }
        }
    }
    let order = [
        "Textamp", "File", "Edit", "View", "Playback", "Queue", "Discover", "Window", "Help",
    ];
    groups.sort_by(|a, b| {
        order
            .iter()
            .position(|name| *name == a[0])
            .cmp(&order.iter().position(|name| *name == b[0]))
            .then_with(|| a.cmp(b))
    });
    groups
}

pub fn visible(
    command: &PaletteCommandKind,
    label: &str,
    current: &[String],
    searching: bool,
) -> bool {
    if let PaletteCommandKind::OpenCommandGroup(target) = command {
        return !searching
            && if label == "Back" {
                !current.is_empty() && target == &current[..current.len() - 1]
            } else {
                target.len() == current.len() + 1 && target.starts_with(current)
            };
    }
    searching || path(command) == current
}

/// Display a station inside its Radio Stations submenu.
pub fn station_label(name: &str) -> String {
    let name = name.strip_prefix("Radio: ").unwrap_or(name);
    match name {
        "Stop Radio" => "Manual Queue".into(),
        "Back" => "Parent Stations".into(),
        _ => name.into(),
    }
}

#[cfg(test)]
mod availability_tests {
    use super::*;
    use crate::app::{AppState, state::*};
    use crate::library::track::Track;

    #[test]
    fn playback_and_queue_actions_require_applicable_content() {
        let mut state = AppState::new();
        for command in [
            PaletteCommandKind::PlayPause,
            PaletteCommandKind::SeekForward,
            PaletteCommandKind::ClearQueue,
            PaletteCommandKind::RemoveFocusedFromQueue,
        ] {
            assert!(disabled_reason(&state, &command).is_some());
            assert!(crate::app::command_palette::run(command, &mut state).is_empty());
        }
        state.queue.tracks = vec![Track {
            rating_key: "song".into(),
            title: "Song".into(),
            ..Default::default()
        }];
        state.queue.index = Some(0);
        state.playback.duration_ms = 60_000;
        assert!(disabled_reason(&state, &PaletteCommandKind::PlayPause).is_none());
        assert!(disabled_reason(&state, &PaletteCommandKind::SeekForward).is_none());
        assert!(disabled_reason(&state, &PaletteCommandKind::ClearQueue).is_none());
        state.view = View::Queue;
        assert!(disabled_reason(&state, &PaletteCommandKind::RemoveFocusedFromQueue).is_none());
        assert!(disabled_reason(&state, &PaletteCommandKind::MoveQueueSelectionUp).is_some());
        assert!(disabled_reason(&state, &PaletteCommandKind::MoveQueueSelectionDown).is_some());
    }

    #[test]
    fn library_commands_and_settings_follow_the_app_menu() {
        for command in [
            PaletteCommandKind::OpenSettings,
            PaletteCommandKind::Quit,
            PaletteCommandKind::SwitchLibrary,
            PaletteCommandKind::Refresh,
        ] {
            assert_eq!(path(&command), ["Textamp"]);
            assert_eq!(menu(&command), "Textamp");
            assert!(visible(&command, "", &["Textamp".into()], false));
        }
        assert!(!visible(&PaletteCommandKind::OpenSettings, "", &[], false));
        assert!(!visible(&PaletteCommandKind::Quit, "", &[], false));
    }
}

/// Analysis destinations remain discoverable even before an account is connected.
pub(crate) fn analysis_command(command: &PaletteCommandKind) -> bool {
    use PaletteCommandKind::*;
    matches!(
        command,
        SonicAdventure
            | SonicAdventureFromFocusedTrack
            | RemixGemini
            | RemixStretch
            | RemixDoppelganger
            | RemixOppositeDay
            | RemixEnemies
            | Navidrome(super::sources::navidrome::commands::Command::Analysis)
            | Navidrome(super::sources::navidrome::commands::Command::Collection(
                super::sources::navidrome::commands::CollectionKind::AudioMuse(_)
            ))
            | FromTrackContext {
                kind: super::track_context::ContextKind::SonicAdventure
                    | super::track_context::ContextKind::SonicRadio,
                ..
            }
    )
}

#[cfg(test)]
mod analysis_availability_tests {
    use super::*;
    use crate::app::AppState;
    #[test]
    fn missing_account_hides_main_controls_but_preserves_disabled_palette_commands() {
        let state = AppState::new();
        assert!(
            super::super::sidebar::sidebar(&state)
                .iter()
                .all(
                    |row| row.parent != Some(super::super::sidebar::SidebarGroup::Ai)
                        && !(row.header && row.label == "AI")
                )
        );
        use super::super::presentation::NpSidebarButton as Tool;
        let tools = super::super::queue_controls::definitions(&state);
        for tool in [Tool::Twins, Tool::Stretch, Tool::Doppelganger] {
            let definition = tools.iter().find(|item| item.tool == tool).unwrap();
            assert!(definition.disabled_reason.is_some());
        }
        let entries = super::super::command_palette::materialize_entries(&state);
        let journey = entries
            .iter()
            .find(|entry| matches!(entry.command, PaletteCommandKind::SonicAdventure))
            .unwrap();
        assert_eq!(
            disabled_reason(&state, &journey.command),
            Some("Connect an AudioMuse account for this library")
        );
    }
}
