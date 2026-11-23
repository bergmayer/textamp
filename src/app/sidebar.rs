//! Library sidebar hierarchy shared by terminal and desktop frontends.
use crate::app::{AppState, action::*, state::*};

pub const INLINE_LIST_LIMIT: usize = 512;

fn grouped_library_root(root: LibraryRoot) -> bool {
    LibraryRoot::ALL.contains(&root) && root != LibraryRoot::Downloads
}

/// Resolve individual visibility together with the persisted library group mask.
pub fn library_root_visible(state: &AppState, root: LibraryRoot) -> bool {
    !(state.desktop_hidden_library_roots.contains(&root)
        || grouped_library_root(root) && state.hidden_sections.contains(&BrowseCategory::Library))
}

/// Change one destination without revealing other destinations hidden by a group mask.
pub fn set_library_root_visible(state: &mut AppState, root: LibraryRoot, visible: bool) {
    if grouped_library_root(root) && state.hidden_sections.contains(&BrowseCategory::Library) {
        for item in LibraryRoot::ALL
            .into_iter()
            .filter(|item| grouped_library_root(*item))
        {
            if !state.desktop_hidden_library_roots.contains(&item) {
                state.desktop_hidden_library_roots.push(item);
            }
        }
        state
            .hidden_sections
            .retain(|category| *category != BrowseCategory::Library);
    }
    state
        .desktop_hidden_library_roots
        .retain(|item| *item != root);
    if !visible {
        state.desktop_hidden_library_roots.push(root);
    }
}

/// Parent group of a sidebar destination; peer rows never inherit a preceding heading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarGroup {
    Albums,
    Ai,
    Playlists,
    Radio,
}
impl SidebarGroup {
    /// Disclosure title used by persisted visibility settings.
    pub fn label(self) -> &'static str {
        match self {
            Self::Albums => "Albums",
            Self::Ai => "AI",
            Self::Playlists => "Playlists",
            Self::Radio => "Radio",
        }
    }
}
#[derive(Clone)]
pub struct SidebarRow {
    pub parent: Option<SidebarGroup>,
    pub label: String,
    pub header: bool,
    pub playlist_key: Option<String>,
    pub library_root: Option<LibraryRoot>,
    pub active: bool,
    pub actions: Vec<Action>,
}
pub const ALBUM_COLLECTIONS: [(
    &str,
    crate::app::sources::navidrome::commands::CollectionKind,
); 6] = {
    use crate::app::sources::navidrome::commands::CollectionKind::*;
    [
        ("Random", RandomAlbums),
        ("Favourites", FavoriteAlbums),
        ("Top Rated", TopRatedAlbums),
        ("Recently Added", RecentlyAdded),
        ("Recently Played", RecentlyPlayed),
        ("Most Played", MostPlayed),
    ]
};
/// Describe Music is a library destination; other analysis collections keep their own rows.
pub fn collection_row(kind: super::sources::navidrome::commands::CollectionKind) -> CategoryRow {
    if kind
        == super::sources::navidrome::commands::CollectionKind::AudioMuse(
            crate::audiomuse::Feature::Describe,
        )
    {
        CategoryRow::Root(LibraryRoot::DescribeMusic)
    } else {
        CategoryRow::NavidromeCollection(kind)
    }
}

pub fn sidebar(state: &AppState) -> Vec<SidebarRow> {
    let mut rows: Vec<_> = state
        .raw_category_rows()
        .into_iter()
        .filter_map(|row| {
            let playlist_key = match &row {
                CategoryRow::Playlist(i) => state
                    .library
                    .playlists
                    .get(*i)
                    .map(|p| p.rating_key.clone()),
                _ => None,
            };
            let (label, actions) = match row {
                CategoryRow::Header(label) => {
                    return Some(SidebarRow {
                        parent: None,
                        label: label.into(),
                        header: true,
                        playlist_key: None,
                        library_root: None,
                        active: false,
                        actions: vec![],
                    })
                }
                CategoryRow::Search | CategoryRow::Root(_) | CategoryRow::Station(_) => return None,
                CategoryRow::Category(category) => (
                    category.display_label().into(),
                    vec![NavigationAction::set_category(category).into()],
                ),
                CategoryRow::NavidromeCollection(kind) => {
                    (kind.label().into(), vec![kind.action()])
                }
                CategoryRow::Playlist(i) => {
                    let playlist = state.library.playlists.get(i)?;
                    (playlist.title.clone(), vec![])
                }
            };
            Some(SidebarRow {
                        parent: None,
                label,
                header: false,
                playlist_key,
                library_root: None,
                active: state.view == View::Browse && matches!(row, CategoryRow::Category(category) if state.browse_category == category && state.sources.nav_collection.is_none()),
                actions,
            })
        })
        .flat_map(|row| {
            if row.header && row.label == "Browse" {
                return vec![SidebarRow {
                        parent: None,
                    label: "Library".into(),
                    ..row
                }];
            }
            if row.label == BrowseCategory::Library.display_label() && !row.header {
                return LibraryRoot::ALL
                    .into_iter()
                    .filter(|root| !state.desktop_hidden_library_roots.contains(root))
                    .map(|root| SidebarRow {
                        parent: None,
                        label: root.label().into(),
                        header: false,
                        playlist_key: None,
                        library_root: Some(root),
                        actions: vec![],
                        active: state.view == View::Browse
                            && state.browse_category == BrowseCategory::Library
                            && state.sources.nav_collection.is_none()
                            && state.desktop_library_root == root,
                    })
                    .collect();
            }
            vec![row]
        })
        .collect();
    // Match the Mac library hierarchy: album views are children of Albums;
    // Artists and Tracks are peer destinations, without a Library disclosure.
    if let Some(start) = rows.iter().position(|r| r.header && r.label == "Library") {
        let end = rows
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, r)| r.header)
            .map_or(rows.len(), |(i, _)| i);
        let mut library: Vec<_> = rows.drain(start..end).collect();
        let mut albums = vec![];
        if let Some(index) = library
            .iter()
            .position(|r| r.library_root == Some(LibraryRoot::Albums))
        {
            let mut all = library.remove(index);
            all.label = "All".into();
            albums.push(all);
        }
        if state.sources.active.navidrome().is_some() {
            library.retain(|row| {
                !ALBUM_COLLECTIONS
                    .iter()
                    .any(|(_, kind)| row.label == kind.label())
            });
            for (label, kind) in ALBUM_COLLECTIONS {
                if state.hidden_collections.contains(&kind) {
                    continue;
                }
                albums.push(SidebarRow {
                    parent: None,
                    label: label.into(),
                    header: false,
                    playlist_key: None,
                    library_root: None,
                    active: state.view == View::Browse
                        && state.sources.nav_collection == Some(kind),
                    actions: vec![kind.action()],
                });
            }
        }
        if let Some(index) = library
            .iter()
            .position(|row| row.library_root == Some(LibraryRoot::Compilations))
        {
            albums.push(library.remove(index));
        }
        if let Some(index) = library
            .iter()
            .position(|row| row.library_root == Some(LibraryRoot::Singles))
        {
            albums.push(library.remove(index));
        }
        let mut arranged = vec![];
        if let Some(index) = library
            .iter()
            .position(|row| row.library_root == Some(LibraryRoot::Artists))
        {
            arranged.push(library.remove(index));
        }
        if !albums.is_empty() {
            arranged.push(SidebarRow {
                parent: None,
                label: "Albums".into(),
                header: true,
                playlist_key: None,
                library_root: None,
                active: false,
                actions: vec![],
            });
            arranged.extend(albums);
        }
        if let Some(at) = rows
            .iter()
            .position(|row| row.header && row.label == "Playlists")
        {
            let count = rows[at + 1..]
                .iter()
                .take_while(|row| row.playlist_key.is_some())
                .count();
            arranged.extend(rows.drain(at..at + 1 + count));
        }
        arranged.extend(library);
        rows.splice(start..start, arranged);
    }
    if state.sources.active.navidrome().is_some() {
        let ai = rows
            .iter()
            .position(|row| row.header && row.label == "AudioMuse");
        if let Some(index) = ai {
            rows[index].label = "AI".into();
        } else {
            rows.push(SidebarRow {
                parent: None,
                label: "AI".into(),
                header: true,
                playlist_key: None,
                library_root: None,
                active: false,
                actions: vec![],
            });
        }
        rows.retain(|row| row.label != crate::audiomuse::Feature::Describe.label());
        let at = rows
            .iter()
            .position(|row| row.header && row.label == "AI")
            .map_or(rows.len(), |index| index + 1);
        for root in [LibraryRoot::Journey, LibraryRoot::DescribeMusic]
            .into_iter()
            .rev()
        {
            if state.desktop_hidden_library_roots.contains(&root) {
                continue;
            }
            rows.insert(
                at,
                SidebarRow {
                    parent: None,
                    label: root.label().into(),
                    header: false,
                    playlist_key: None,
                    library_root: Some(root),
                    active: state.desktop_library_root == root && state.view == View::Browse,
                    actions: vec![],
                },
            );
        }
    }

    if !state
        .desktop_hidden_library_roots
        .contains(&LibraryRoot::Downloads)
        && !rows
            .iter()
            .any(|row| row.library_root == Some(LibraryRoot::Downloads))
    {
        rows.push(SidebarRow {
            parent: None,
            label: LibraryRoot::Downloads.label().into(),
            header: false,
            playlist_key: None,
            library_root: Some(LibraryRoot::Downloads),
            actions: vec![],
            active: state.view == View::Browse
                && state.desktop_library_root == LibraryRoot::Downloads,
        });
    }

    for row in &mut rows {
        if let Some(kind) = state.sources.nav_collection {
            row.active = state.view == View::Browse && row.actions.iter().any(|action| {
                if let Action::Source(crate::app::sources::SourceAction::Navidrome(action)) = action {
                    matches!(action.as_ref(), crate::app::sources::navidrome::NavAction::Command(crate::app::sources::navidrome::commands::Command::Collection(selected)) if *selected == kind)
                } else { false }
            });
        }
        if row.label == "Favorite songs" {
            row.label = "Favorite tracks".into();
        }
        if let Some(key) = &row.playlist_key {
            row.active = state.view == View::Browse
                && state.browse_category == BrowseCategory::Playlists
                && state
                    .playlist_nav
                    .columns
                    .first()
                    .and_then(|column| column.selected_item())
                    .is_some_and(|item| item.key() == key);
        }
    }
    if !state.station_nav.columns.is_empty()
        && !state
            .desktop_hidden_library_roots
            .contains(&LibraryRoot::Radio)
    {
        let at = rows
            .iter()
            .position(|row| row.header && matches!(row.label.as_str(), "AI" | "AudioMuse"))
            .unwrap_or(rows.len());
        rows.insert(
            at,
            SidebarRow {
                parent: None,
                label: "Radio".into(),
                header: false,
                playlist_key: None,
                library_root: Some(LibraryRoot::Radio),
                active: state.view == View::Browse
                    && state.browse_category == BrowseCategory::Library
                    && state.desktop_library_root == LibraryRoot::Radio
                    && state.sources.nav_collection.is_none(),
                actions: vec![],
            },
        );
    }
    if matches!(
        state.desktop_library_root,
        LibraryRoot::Radio | LibraryRoot::Journey | LibraryRoot::DescribeMusic
    ) && state.view == View::Browse
    {
        for row in &mut rows {
            row.active = row.library_root == Some(state.desktop_library_root);
        }
    }
    for row in &mut rows {
        row.parent = match row.library_root {
            Some(LibraryRoot::Albums | LibraryRoot::Compilations | LibraryRoot::Singles) => {
                Some(SidebarGroup::Albums)
            }
            Some(LibraryRoot::Journey | LibraryRoot::DescribeMusic) => Some(SidebarGroup::Ai),
            _ if row.playlist_key.is_some() => Some(SidebarGroup::Playlists),
            _ => row.actions.iter().find_map(|action| {
                if let Action::Source(crate::app::sources::SourceAction::Navidrome(action)) = action
                    && let crate::app::sources::navidrome::NavAction::Command(
                        crate::app::sources::navidrome::commands::Command::Collection(kind),
                    ) = action.as_ref()
                {
                    if matches!(
                        kind,
                        crate::app::sources::navidrome::commands::CollectionKind::AudioMuse(_)
                    ) {
                        return Some(SidebarGroup::Ai);
                    }
                    if ALBUM_COLLECTIONS
                        .iter()
                        .any(|(_, candidate)| candidate == kind)
                    {
                        return Some(SidebarGroup::Albums);
                    }
                }
                None
            }),
        };
    }
    if let Some(at) = rows
        .iter()
        .position(|row| row.library_root == Some(LibraryRoot::Radio))
        && let Some(root) = state.station_nav.columns.first()
    {
        let children: Vec<_> = root
            .stations
            .iter()
            .filter(|station| station.is_browsable())
            .map(|station| SidebarRow {
                parent: Some(SidebarGroup::Radio),
                label: station.title.clone(),
                header: false,
                playlist_key: None,
                library_root: None,
                active: rows[at].active
                    && root
                        .stations
                        .get(root.selected_index)
                        .is_some_and(|selected| selected.key == station.key)
                    && (state.station_detail.is_some() || state.station_nav.columns.len() > 1),
                actions: vec![NavigationAction::OpenRadioStation(station.key.clone()).into()],
            })
            .collect();
        if !children.is_empty() {
            rows[at].header = true;
            rows.splice(at + 1..at + 1, children);
        }
    }
    if crate::app::sources::audiomuse::connection(state).is_none() {
        rows.retain(|row| {
            row.parent != Some(SidebarGroup::Ai) && !(row.header && row.label == "AI")
        });
    }
    if cfg!(target_os = "macos") && library_root_visible(state, LibraryRoot::Mp3Players) {
        rows.push(SidebarRow {
            parent: None,
            label: LibraryRoot::Mp3Players.label().into(),
            header: false,
            playlist_key: None,
            library_root: Some(LibraryRoot::Mp3Players),
            actions: vec![],
            active: super::player_sync::visible(state),
        });
    }
    rows
}

pub fn station_actions(state: &AppState, index: usize) -> Vec<Action> {
    state
        .station_nav
        .columns
        .first()
        .and_then(|root| root.stations.get(index))
        .map(|s| vec![NavigationAction::OpenRadioStation(s.key.clone()).into()])
        .unwrap_or_default()
}

fn is_visible(row: &SidebarRow, collapsed: &[String]) -> bool {
    !(row.header && matches!(row.label.as_str(), "Library" | "Browse")
        || row
            .parent
            .is_some_and(|parent| collapsed.iter().any(|title| title == parent.label())))
}

pub fn allows_disclosure(rows: &[SidebarRow], title: &str) -> bool {
    let count = rows
        .iter()
        .filter(|row| row.parent.is_some_and(|group| group.label() == title))
        .count();
    (1..=INLINE_LIST_LIMIT).contains(&count)
}

pub fn visible_indices(rows: &[SidebarRow], collapsed: &[String]) -> Vec<usize> {
    let mut collapsed = collapsed.to_vec();
    collapsed.extend(
        rows.iter()
            .filter(|row| row.header && !allows_disclosure(rows, &row.label))
            .map(|row| row.label.clone()),
    );
    rows.iter()
        .enumerate()
        .filter_map(|(index, row)| is_visible(row, &collapsed).then_some(index))
        .collect()
}

pub fn visible_rows(state: &AppState) -> Vec<SidebarRow> {
    let rows = sidebar(state);
    visible_indices(&rows, &state.desktop_collapsed_sections)
        .into_iter()
        .map(|index| rows[index].clone())
        .collect()
}

/// Open a library destination through the same source commands in every frontend.
pub fn open_root(root: LibraryRoot, state: &mut AppState) -> Vec<Action> {
    state.tui_sidebar_group = None;
    state.tui_description_submit_focused = false;
    state.tui_description_focused = !state.desktop_frontend && root == LibraryRoot::DescribeMusic;
    state.miller_scroll_manual = false;
    state.popups.close_all();
    state.palette.close();
    state.view = View::Browse;
    state.browse_category = BrowseCategory::Library;
    state.sources.nav_collection = None;
    state.sources.nav_tasks.remove("artist-browse");
    state.sources.nav_tasks.remove("album-browse");
    state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
    state.artist_nav.loading = false;
    let items = state.build_artist_root_items();
    state.artist_nav.reset("artists", items);
    state.desktop_library_root = root;
    state.desktop_player_visible = false;
    state.category_column_focused = false;
    state.focus = Focus::Right;
    let actions = match root {
        LibraryRoot::Downloads => vec![super::downloads::Command::Open.into()],
        LibraryRoot::Mp3Players => {
            state.player_sync.column_cursor = 0;
            state.player_sync.choosing_playlist = false;
            state.player_sync.webdav_filter_focused = false;
            let mut actions = vec![super::player_sync::Command::Discover { quiet: false }.into()];
            if super::player_sync::webdav_filter(state).is_some() {
                actions.push(super::player_sync::Command::BrowseWebdav(String::new()).into());
            }
            actions
        }
        LibraryRoot::Artists => vec![],
        LibraryRoot::Albums => vec![
            MillerAction::LoadAllAlbumsForMiller {
                replace_child: false,
            }
            .into(),
        ],
        LibraryRoot::Compilations => vec![
            MillerAction::LoadCompilationsForMiller {
                replace_child: false,
            }
            .into(),
        ],
        LibraryRoot::Tracks => vec![
            MillerAction::LoadAllLibraryTracksForMiller {
                replace_child: false,
            }
            .into(),
        ],
        LibraryRoot::Singles => vec![BrowseAction::OpenSingles.into()],
        LibraryRoot::Journey => vec![SearchAction::OpenAdventureLauncher.into()],
        LibraryRoot::DescribeMusic => vec![
            crate::app::sources::audiomuse::Command::BrowseDescriptions { take_focus: true }.into(),
        ],
        LibraryRoot::Radio => {
            if state.station_nav.columns.is_empty() && !state.station_nav.loading {
                vec![BrowseAction::LoadStations.into()]
            } else { vec![] }
        }
    };
    state.category_column_index = state.row_index_for_category(state.browse_category);
    actions
}

/// Destinations belonging to a disclosure group, including collapsed children.
pub fn group_rows(state: &AppState, title: &str) -> Vec<(usize, SidebarRow)> {
    sidebar(state)
        .into_iter()
        .enumerate()
        .filter(|(_, row)| row.parent.is_some_and(|parent| parent.label() == title))
        .collect()
}

pub fn filtered_group_rows(state: &AppState, title: &str, query: &str) -> Vec<(usize, SidebarRow)> {
    let rows = group_rows(state, title);
    if query.is_empty() {
        return rows;
    }
    let matches =
        crate::services::filter_with_priority(&rows, query, |(_, row)| &row.label, rows.len());
    matches
        .matched_indices
        .into_iter()
        .map(|i| rows[i].clone())
        .collect()
}

/// Activate the selected child of a terminal sidebar group column.
pub fn open_group_child(state: &mut AppState, title: &str, index: usize) -> Vec<Action> {
    state.tui_sidebar_group = None;
    let query = state
        .playlist_group_filter
        .take()
        .filter(|_| title == "Playlists")
        .unwrap_or_default();
    let Some((_, row)) = filtered_group_rows(state, title, &query)
        .into_iter()
        .nth(index)
    else {
        return vec![];
    };
    if let Some(root) = row.library_root {
        return vec![NavigationAction::OpenLibraryRoot(root).into()];
    }
    if let Some(key) = row.playlist_key {
        state.set_browse_category(BrowseCategory::Playlists, false);
        if !state.select_playlist(&key) {
            return vec![];
        }
        state.category_column_focused = false;
        state.playlist_nav.focused_column = 0;
        state.playlist_nav.truncate_right();
        return vec![
            MillerAction::LoadPlaylistTracksForMiller {
                playlist_key: key,
                replace_child: true,
            }
            .into(),
        ];
    }
    row.actions
}

/// Visibility choices follow library navigation order and retain hidden destinations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarTarget {
    Root(crate::app::state::LibraryRoot),
    Section(crate::app::state::SidebarSection),
}
#[derive(Clone)]
pub struct SidebarOption {
    pub group: &'static str,
    pub label: &'static str,
    pub target: SidebarTarget,
    pub checked: bool,
}
pub fn sidebar_options(state: &AppState) -> Vec<SidebarOption> {
    use crate::app::sources::navidrome::commands::CollectionKind;
    use crate::app::state::{BrowseCategory, LibraryRoot, SidebarSection};
    let mut options = Vec::new();
    let library_visible = !state.hidden_sections.contains(&BrowseCategory::Library);
    let sections = state.sidebar_sections();
    if state.sources.active.folder().is_none() {
        options.push(SidebarOption {
            group: "Library",
            label: "Artists",
            target: SidebarTarget::Root(LibraryRoot::Artists),
            checked: library_visible
                && !state
                    .desktop_hidden_library_roots
                    .contains(&LibraryRoot::Artists),
        });
        options.push(SidebarOption {
            group: "Albums",
            label: "All",
            target: SidebarTarget::Root(LibraryRoot::Albums),
            checked: library_visible
                && !state
                    .desktop_hidden_library_roots
                    .contains(&LibraryRoot::Albums),
        });
        if state.sources.active.navidrome().is_some() {
            for (label, kind) in crate::app::sidebar::ALBUM_COLLECTIONS {
                let section = SidebarSection::Collection(kind);
                options.push(SidebarOption {
                    group: "Albums",
                    label,
                    target: SidebarTarget::Section(section),
                    checked: !section.hidden(state),
                });
            }
        }
        options.push(SidebarOption {
            group: "Albums",
            label: "Compilations",
            target: SidebarTarget::Root(LibraryRoot::Compilations),
            checked: library_visible
                && !state
                    .desktop_hidden_library_roots
                    .contains(&LibraryRoot::Compilations),
        });
        options.push(SidebarOption {
            group: "Albums",
            label: "Singles",
            target: SidebarTarget::Root(LibraryRoot::Singles),
            checked: library_visible
                && !state
                    .desktop_hidden_library_roots
                    .contains(&LibraryRoot::Singles),
        });
        let playlists = SidebarSection::Category(BrowseCategory::Playlists);
        if sections.contains(&playlists) {
            options.push(SidebarOption {
                group: "Playlists",
                label: "Playlists",
                target: SidebarTarget::Section(playlists),
                checked: !playlists.hidden(state),
            });
        }
        options.push(SidebarOption {
            group: "Library",
            label: "Downloads",
            target: SidebarTarget::Root(LibraryRoot::Downloads),
            checked: library_root_visible(state, LibraryRoot::Downloads),
        });
        options.push(SidebarOption {
            group: "Library",
            label: "Tracks",
            target: SidebarTarget::Root(LibraryRoot::Tracks),
            checked: library_visible
                && !state
                    .desktop_hidden_library_roots
                    .contains(&LibraryRoot::Tracks),
        });
    }
    for section in sections.iter().copied() {
        let (group, label) = match section {
            SidebarSection::Category(BrowseCategory::Library | BrowseCategory::Playlists) => {
                continue;
            }
            SidebarSection::Collection(CollectionKind::AudioMuse(_)) => ("AI", section.label()),
            SidebarSection::Collection(CollectionKind::FavoriteSongs) => {
                ("Library", "Favorite tracks")
            }
            SidebarSection::Collection(CollectionKind::FavoriteArtists) => {
                ("Library", "Favorite artists")
            }
            SidebarSection::Collection(_) => continue,
            _ => ("Library", section.label()),
        };
        options.push(SidebarOption {
            group,
            label,
            target: SidebarTarget::Section(section),
            checked: !section.hidden(state),
        });
    }
    if state.sources.active.navidrome().is_some() {
        options.retain(|option| {
            option.target
                != SidebarTarget::Section(SidebarSection::Collection(CollectionKind::AudioMuse(
                    crate::audiomuse::Feature::Describe,
                )))
        });
        let at = options
            .iter()
            .position(|option| option.group == "AI")
            .unwrap_or(options.len());
        for root in [LibraryRoot::DescribeMusic, LibraryRoot::Journey] {
            options.insert(
                at,
                SidebarOption {
                    group: "AI",
                    label: root.label(),
                    target: SidebarTarget::Root(root),
                    checked: !state.desktop_hidden_library_roots.contains(&root),
                },
            );
        }
        if !state.station_nav.columns.is_empty() {
            options.insert(
                at,
                SidebarOption {
                    group: "Library",
                    label: LibraryRoot::Radio.label(),
                    target: SidebarTarget::Root(LibraryRoot::Radio),
                    checked: !state
                        .desktop_hidden_library_roots
                        .contains(&LibraryRoot::Radio),
                },
            );
        }
    }
    if cfg!(target_os = "macos") {
        options.push(SidebarOption {
            group: "Library",
            label: LibraryRoot::Mp3Players.label(),
            target: SidebarTarget::Root(LibraryRoot::Mp3Players),
            checked: library_root_visible(state, LibraryRoot::Mp3Players),
        });
    }
    options
}

impl SidebarTarget {
    pub fn setting(self) -> TextampSetting {
        match self {
            Self::Root(root) => TextampSetting::LibraryRootVisible(root),
            Self::Section(section) => TextampSetting::Sidebar(section),
        }
    }
}
