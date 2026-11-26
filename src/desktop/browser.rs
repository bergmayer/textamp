//! Music-specific projection and target validation for the Miller columns.
use crate::app::{AppState, BrowseCategory, View, state::BrowseItem};
use std::{
    collections::BTreeSet,
    hash::{Hash, Hasher},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Artist,
    Album,
    Track,
    Folder,
    Genre,
    Playlist,
    Info,
    Description,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub download_action: Option<&'static str>,
    pub download_id: Option<String>,
    pub download_status: Option<String>,
    pub year: Option<u16>,
    pub key: String,
    pub title: String,
    pub kind: Kind,
}
impl Hash for Row {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.year.hash(state);
        self.key.hash(state);
        self.title.hash(state);
        self.kind.hash(state);
        self.download_id.hash(state);
        self.download_action.hash(state);
    }
}
#[derive(Clone)]
pub struct Column {
    pub identity: String,
    pub artwork_keys: std::collections::HashMap<usize, String>,
    pub revision: u64,
    pub title: String,
    pub rows: Arc<Vec<Row>>,
    pub alphabet_ascending: Option<bool>,
    pub has_view_options: bool,
    pub named_heading: bool,
    pub header_controls: Vec<super::catalog::Control>,
    pub selected: usize,
    pub selected_set: BTreeSet<usize>,
    pub tracks: Arc<[crate::library::models::Track]>,
    pub artwork_visible: bool,
    pub album_grid: bool,
    pub album_info: bool,
    pub bio_search: Option<crate::services::web_search::Provider>,
    pub similar_artists: Vec<crate::library::models::Artist>,
    pub artist_header: Option<(String, String, String)>,
    pub album_page: Option<(String, super::catalog::Preview)>,
    pub album_info_selected: bool,
    pub editable_playlist: bool,
    pub partial_albums: BTreeSet<usize>,
    pub playlist_editing: bool,
    pub metadata: bool,
    pub hidden: bool,
    pub playing: Option<usize>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub generation: u64,
    pub column: usize,
    pub identity: String,
    pub revision: u64,
    pub row: usize,
    pub key: String,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Intent {
    Focus,
    Inspect,
    Play,
    PlayFollowing,
    Download,
    ManageDownload,
    OpenAlbumDetails,
    AddAlbumToPlaylist,
    AddTracksToPlaylist,
    EditPlaylist(crate::app::sources::navidrome::playlist_edit::Edit),
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Selection {
    #[default]
    Single,
    Toggle,
    Check,
    Extend,
    Context,
}
#[derive(Clone, Copy, Default, PartialEq)]
pub enum Origin {
    #[default]
    Pointer,
    Keyboard,
    AlphabetJump,
    LibraryJump,
}
pub const GRID_COVER_SIZE: f32 = 72.;
pub const GRID_ROW_HEIGHT: f32 = 138.;

impl Column {
    pub fn row_height(&self) -> f32 {
        if self.album_grid {
            GRID_ROW_HEIGHT
        } else if self.artwork_visible {
            64.
        } else if !self.tracks.is_empty() {
            48.
        } else {
            33.
        }
    }
    pub fn target(&self, generation: u64, column: usize, row: usize) -> Option<Target> {
        Some(Target {
            generation,
            column,
            identity: self.identity.clone(),
            revision: self.revision,
            row,
            key: self.rows.get(row)?.key.clone(),
        })
    }
    pub fn contains(&self, target: &Target) -> bool {
        self.identity == target.identity
            && self.revision == target.revision
            && self
                .rows
                .get(target.row)
                .is_some_and(|row| row.key == target.key)
    }
}
/// Count-bearing terminal root titles are represented by the desktop status bar.
/// Album and artist names retain their original punctuation and parentheses.
pub fn heading(column: &Column) -> String {
    if let Some((_, name, _)) = &column.artist_header {
        return name.clone();
    }
    for noun in ["artists", "tracks"] {
        if column.title.strip_prefix(noun).is_some_and(|suffix| {
            suffix
                .strip_prefix(" (")
                .and_then(|s| s.strip_suffix(')'))
                .is_some_and(|count| !count.is_empty() && count.bytes().all(|b| b.is_ascii_digit()))
        }) {
            return noun.to_owned();
        }
    }
    column.title.clone()
}

pub use crate::util::column_heading::destination as compact_heading;

/// Name the selected destination rather than repeating column-type prefixes.
pub fn breadcrumb_label(column: &Column) -> String {
    if column.album_info_selected
        || (!column.tracks.is_empty() && column.rows.get(column.selected).is_none())
    {
        return "Tracks".into();
    }
    column
        .rows
        .get(column.selected)
        .map(|row| row.title.clone())
        .unwrap_or_else(|| heading(column))
}

/// Breadcrumb destinations projected directly from the live terminal selection.
pub(super) fn terminal_breadcrumbs(state: &AppState) -> Vec<String> {
    use crate::app::{state::ColumnHeaderSelection, windowed_presentation::Workspace};
    if state.windowed_presentation.workspace() == Some(Workspace::NowPlaying) {
        return vec!["Now Playing".into()];
    }
    if let Some(title) = super::workspace::title(state) {
        return vec![title.into()];
    }
    let mut labels: Vec<String> = if state.browse_category == BrowseCategory::Folders {
        state
            .folder_state
            .as_ref()
            .map(|nav| {
                nav.columns
                    .iter()
                    .map(|column| {
                        column
                            .items
                            .get(column.selected_index)
                            .map_or_else(|| column.title.clone(), |item| item.title.clone())
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        state
            .browse_nav()
            .map(|nav| {
                nav.columns
                    .iter()
                    .enumerate()
                    .filter(|(index, column)| {
                        column.details.is_none()
                            && (*index >= first_content_column(state)
                                || column.items.get(column.selected_index).is_some_and(|item| {
                                    matches!(
                                        item,
                                        BrowseItem::Artist { .. }
                                            | BrowseItem::Playlist { .. }
                                            | BrowseItem::Genre { .. }
                                    )
                                }))
                    })
                    .map(|(_, column)| {
                        if column.header_selection == ColumnHeaderSelection::AlbumDetails {
                            "Tracks".into()
                        } else {
                            column.items.get(column.selected_index).map_or_else(
                                || column.title.clone(),
                                |item| item.title().to_owned(),
                            )
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    if labels.is_empty() {
        labels.push(state.browse_category.display_label().into());
    }
    labels
}

/// Reuse immutable rows for selection-only movement instead of cloning the catalog.
pub fn refresh_selection(state: &AppState, columns: &mut Vec<Column>) -> bool {
    if state.view != View::Browse {
        return false;
    }
    if state.browse_category == BrowseCategory::Folders {
        let Some(nav) = state.folder_state.as_ref() else {
            return false;
        };
        if nav.columns.len() > columns.len() {
            return false;
        }
        for (source, column) in nav.columns.iter().zip(columns.iter_mut()) {
            if source.title != column.title || source.items.len() != column.rows.len() {
                return false;
            }
            column.selected = source.selected_index;
            column.selected_set = source.selected_set.clone();
        }
        columns.truncate(nav.columns.len());
    } else {
        let Some(nav) = state.browse_nav() else {
            return false;
        };
        if nav.columns.len() > columns.len() {
            return false;
        }
        for (source, column) in nav.columns.iter().zip(columns.iter_mut()) {
            if source.title != column.title || source.items.len() != column.rows.len() {
                return false;
            }
            column.selected = source.selected_index;
            column.selected_set = source.selected_set.clone();
            if source.playlist_editing || column.playlist_editing {
                return false;
            }
            column.album_info_selected = column.album_info
                && source.header_selection
                    == crate::app::state::ColumnHeaderSelection::AlbumDetails;
        }
        columns.truncate(nav.columns.len());
    }
    true
}
fn column(
    identity: String,
    title: String,
    rows: Vec<Row>,
    selected: usize,
    selected_set: BTreeSet<usize>,
) -> Column {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rows.hash(&mut hasher);
    Column {
        identity,
        artwork_keys: Default::default(),
        revision: hasher.finish(),
        title,
        has_view_options: false,
        named_heading: false,
        header_controls: vec![],
        alphabet_ascending: None,
        rows: Arc::new(rows),
        selected,
        selected_set,
        tracks: Arc::from([]),
        artwork_visible: false,
        album_grid: false,
        album_info: false,
        album_page: None,
        bio_search: None,
        similar_artists: Vec::new(),
        artist_header: None,
        album_info_selected: false,
        editable_playlist: false,
        partial_albums: Default::default(),
        playlist_editing: false,
        metadata: false,
        hidden: false,
        playing: None,
    }
}
/// A stable queue target independent of the visible library workspace.
pub const PLAYER_QUEUE: usize = usize::MAX;
pub fn queue(state: &AppState) -> Column {
    let mut queue = column(
        format!(
            "{}:playback:{:?}",
            state.library_generation, state.playback_mode
        ),
        "Queue".into(),
        state
            .playback_tracks()
            .iter()
            .map(|track| Row {
                download_action: None,
                download_id: None,
                download_status: None,
                year: None,
                key: track.rating_key.clone(),
                title: track.title.clone(),
                kind: Kind::Track,
            })
            .collect(),
        state.list_state.queue_index,
        state.queue.selected.clone(),
    );
    queue.tracks = state.playback_tracks().to_vec().into();
    queue.playing = if state.playback_mode == crate::app::state::PlaybackMode::Radio {
        state.radio.track_index
    } else {
        state.queue.index
    };
    queue
}
pub fn target_column(state: &AppState, index: usize) -> Option<Column> {
    if index == PLAYER_QUEUE {
        Some(queue(state))
    } else {
        columns(state).into_iter().nth(index)
    }
}
pub fn columns(state: &AppState) -> Vec<Column> {
    let root = format!(
        "{}:{:?}:{:?}",
        state.library_generation, state.view, state.browse_category
    );
    if matches!(state.view, View::Queue | View::NowPlaying) {
        return vec![queue(state)];
    }
    if state.browse_category == BrowseCategory::Folders {
        return state
            .folder_state
            .as_ref()
            .map(|nav| {
                nav.columns
                    .iter()
                    .enumerate()
                    .map(|(index, c)| {
                        let mut result = column(
                            format!("{root}:{index}:{:?}", c.key),
                            c.title.clone(),
                            c.items
                                .iter()
                                .map(|i| Row {
                                    download_action: None,
                                    download_id: None,
                                    download_status: None,
                                    year: None,
                                    key: i.key.clone(),
                                    title: i.title.clone(),
                                    kind: if i.item_type
                                        == crate::library::models::FolderItemType::Folder
                                    {
                                        Kind::Folder
                                    } else {
                                        Kind::Track
                                    },
                                })
                                .collect(),
                            c.selected_index,
                            c.selected_set.clone(),
                        );
                        if let Some(source) = state.sources.active.folder() {
                            for (row, item) in c.items.iter().enumerate() {
                                let dir = if item.is_track() {
                                    item.key.rsplit_once('/').map_or("", |(dir, _)| dir)
                                } else {
                                    &item.key
                                };
                                let key =
                                    crate::library::track::Track::folder_cover_key(source, dir);
                                if state.artwork.grid_cache.contains_key(&key) {
                                    result.artwork_keys.insert(row, key);
                                }
                            }
                            result.artwork_visible = !result.artwork_keys.is_empty();
                        }
                        result.named_heading = index > 0;
                        if c.items.iter().any(|item| item.is_track()) {
                            result.tracks = c
                                .items
                                .iter()
                                .map(|item| {
                                    if item.is_track() {
                                        if let Some(source) = state.sources.active.folder() {
                                            crate::library::models::Track::from_folder(
                                                source, &item.key,
                                            )
                                        } else {
                                            item.rating_key
                                                .as_ref()
                                                .and_then(|key| c.tracks.get(key))
                                                .cloned()
                                                .unwrap_or_default()
                                        }
                                    } else {
                                        Default::default()
                                    }
                                })
                                .collect();
                        }
                        result
                    })
                    .collect()
            })
            .unwrap_or_default();
    }
    let mut ancestry = Vec::new();
    state
        .browse_nav()
        .map(|nav| {
            nav.columns
                .iter()
                .enumerate()
                .map(|(index, c)| {
                    let identity = format!(
                        "{root}:{}",
                        serde_json::to_string(&ancestry).expect("string path")
                    );
                    ancestry.push(
                        c.items
                            .get(c.selected_index)
                            .map(|i| i.key().to_owned())
                            .unwrap_or_default(),
                    );
                    let mut result = column(
                        identity,
                        c.title.clone(),
                        c.items
                            .iter()
                            .enumerate()
                            .map(|(index, i)| Row {
                                download_action: crate::app::downloads::item_command(
                                    state, c, index,
                                )
                                .map(|(label, _)| label),
                                download_id: i.pending_download().map(str::to_owned),
                                download_status: i.pending_download().and_then(|id| {
                                    crate::app::downloads::pending_status(state, id)
                                }),
                                year: match i {
                                    BrowseItem::Album { year, .. } => *year,
                                    _ => None,
                                },
                                key: i.key().to_owned(),
                                title: i.title().to_owned(),
                                kind: match i {
                                    BrowseItem::Description { .. } => Kind::Description,
                                    BrowseItem::Artist { .. } => Kind::Artist,
                                    BrowseItem::ArtistInfo { .. } => Kind::Info,
                                    BrowseItem::Album { .. } => Kind::Album,
                                    BrowseItem::Track { .. } => Kind::Track,
                                    BrowseItem::Genre { .. } => Kind::Genre,
                                    BrowseItem::Playlist { .. } => Kind::Playlist,
                                    _ => Kind::Folder,
                                },
                            })
                            .collect(),
                        c.selected_index,
                        c.selected_set.clone(),
                    );
                    result.editable_playlist =
                        crate::app::sources::navidrome::playlist_edit::editable(state, c).is_some();
                    result.playlist_editing = c.playlist_editing && result.editable_playlist;
                    if result.playlist_editing
                        && let Some(groups) = &c.album_groups
                    {
                        result.partial_albums = groups
                            .iter()
                            .enumerate()
                            .filter_map(|(row, positions)| {
                                (!c.selected_set.contains(&row)
                                    && positions
                                        .iter()
                                        .any(|p| c.grouped_playlist_marks.contains(p)))
                                .then_some(row)
                            })
                            .collect();
                    }
                    result.alphabet_ascending = match c.sort_mode {
                        crate::app::state::ColumnSortMode::ByTitle => Some(c.sort_ascending),
                        crate::app::state::ColumnSortMode::Default
                            if !c
                                .items
                                .iter()
                                .any(|i| matches!(i, BrowseItem::Track { .. })) =>
                        {
                            alphabet_order(
                                c.items
                                    .iter()
                                    .filter(|item| {
                                        matches!(
                                            item,
                                            BrowseItem::Artist { .. }
                                                | BrowseItem::Album { .. }
                                                | BrowseItem::Genre { .. }
                                                | BrowseItem::Playlist { .. }
                                        )
                                    })
                                    .map(|item| item.title()),
                            )
                        }
                        _ => None,
                    };
                    result.named_heading = c.artist_header.is_some()
                        || matches!(
                            c.play_all_row,
                            Some(
                                crate::app::state::PlayAllRow::Album { .. }
                                    | crate::app::state::PlayAllRow::Playlist { .. }
                            )
                        )
                        || compact_heading(&c.title) != c.title;
                    result.has_view_options = state.column_options_type(index).is_some();
                    result.hidden = index < first_content_column(state);
                    result.artwork_keys = c
                        .items
                        .iter()
                        .enumerate()
                        .filter_map(|(row, item)| {
                            crate::app::artwork_mosaic::key(item).map(|key| (row, key))
                        })
                        .collect();
                    result.tracks = c.tracks.clone().into();
                    result.album_grid = crate::app::view_options::album_grid(state, index);
                    result.artwork_visible = c.artwork_visible || result.album_grid;
                    result.metadata = c.details.is_some();
                    if let Some(crate::app::state::PlayAllRow::Album { rating_key, .. }) =
                        &c.play_all_row
                    {
                        result.album_page = crate::app::details::album_page(state, c)
                            .map(|page| (rating_key.clone(), page));
                    }
                    result.artist_header = c.artist_header.as_ref().map(|info| {
                        result.bio_search =
                            info.needs_web_search().then_some(state.web_search_provider);
                        result.similar_artists = info.document.related_artists.clone();
                        (
                            info.artist_key.clone(),
                            info.artist_name.clone(),
                            info.excerpt().to_owned(),
                        )
                    });
                    result.album_info = c.has_album_details();
                    result.album_info_selected = result.album_info
                        && c.header_selection
                            == crate::app::state::ColumnHeaderSelection::AlbumDetails;
                    if let Some(row) = &c.play_all_row {
                        result.header_controls =
                            super::catalog::collection_controls(row, &c.ready_tracks());
                    } else if state.sources.nav_collection
                        == Some(
                            crate::app::sources::navidrome::commands::CollectionKind::AudioMuse(
                                crate::audiomuse::Feature::Describe,
                            ),
                        )
                        && !c.tracks.is_empty()
                        && c.details.is_none()
                    {
                        result.header_controls = super::catalog::collection_controls(
                            &crate::app::state::PlayAllRow::AllTracks {
                                label: "Play Results".into(),
                            },
                            &c.ready_tracks(),
                        );
                    }
                    result
                        .header_controls
                        .extend(crate::app::column_controls::artist_mix(state, c));
                    if result.playlist_editing {
                        result.header_controls.clear();
                    }
                    result.header_controls.extend(
                        crate::app::sources::navidrome::playlist_edit::controls(state, c),
                    );
                    result
                })
                .collect()
        })
        .unwrap_or_default()
}
/// Match the shared title order, excluding synthetic pinned actions at the call site.
fn alphabet_order<'a>(titles: impl Iterator<Item = &'a str>) -> Option<bool> {
    let mut ascending = true;
    let mut descending = true;
    let mut previous = None;
    let mut count = 0;
    for title in titles {
        let key = crate::app::handlers::helpers::sort_key(title);
        if let Some(previous) = previous {
            ascending &= previous <= key;
            descending &= previous >= key;
        }
        previous = Some(key);
        count += 1;
        if !ascending && !descending {
            return None;
        }
    }
    if count < 2 {
        None
    } else if ascending {
        Some(true)
    } else if descending {
        Some(false)
    } else {
        None
    }
}

pub fn valid(state: &AppState, target: &Target) -> bool {
    target.generation == state.library_generation
        && target_column(state, target.column)
            .as_ref()
            .is_some_and(|column| column.contains(target))
}
pub fn selection(
    selected: &mut usize,
    set: &mut BTreeSet<usize>,
    anchor: &mut Option<usize>,
    row: usize,
    mode: Selection,
    multi: bool,
) {
    match mode {
        Selection::Context if set.contains(&row) || (set.is_empty() && *selected == row) => {}
        Selection::Check if multi => {
            if !set.remove(&row) {
                set.insert(row);
            }
            *anchor = Some(row);
        }
        Selection::Toggle if multi => {
            if set.is_empty() {
                set.insert(*selected);
            }
            if !set.remove(&row) {
                set.insert(row);
            }
            *anchor = Some(row);
        }
        Selection::Extend if multi => {
            let start = anchor.unwrap_or(*selected);
            *set = (start.min(row)..=start.max(row)).collect();
        }
        _ => {
            set.clear();
            *anchor = Some(row);
        }
    }
    *selected = row;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::models::{FolderColumn, FolderItem, FolderNavigationState};

    fn fixture() -> AppState {
        let mut state = AppState::new();
        state.browse_category = BrowseCategory::Folders;
        let mut nav = FolderNavigationState::for_library("fixture".into());
        nav.columns.push(FolderColumn::new(
            None,
            "Music".into(),
            ["A", "B", "C"]
                .into_iter()
                .map(|key| FolderItem::folder(key.into(), key.into()))
                .collect(),
        ));
        state.folder_state = Some(nav);
        state
    }

    #[test]
    fn headings_omit_terminal_counts_but_preserve_named_content() {
        let mut col = column(
            "fixture".into(),
            "artists (50129)".into(),
            vec![],
            0,
            BTreeSet::new(),
        );
        assert_eq!(heading(&col), "artists");
        col.title = "tracks (13)".into();
        assert_eq!(heading(&col), "tracks");
        col.title = "albums — Artist (Live)".into();
        assert_eq!(heading(&col), "albums — Artist (Live)");
    }

    #[test]
    fn compact_headings_only_remove_the_column_type() {
        for (full, compact) in [
            ("albums — 2.3", "2.3"),
            ("tracks — Mutant Pop: 78/79", "Mutant Pop: 78/79"),
            ("Albums — 日本語 — Live", "日本語 — Live"),
            ("Album Details", "Album Details"),
            ("Something — Else", "Something — Else"),
            ("artists", "artists"),
        ] {
            assert_eq!(compact_heading(full), compact);
        }
    }
    #[test]
    fn breadcrumbs_use_destination_names_and_tracks_when_album_info_is_selected() {
        for (kind, title, column_title) in [
            (Kind::Artist, "3 Forks", "artists"),
            (Kind::Album, "Firewood", "albums — 3 Forks"),
            (Kind::Track, "The Cranks", "tracks — Firewood"),
        ] {
            let mut col = column(
                "fixture".into(),
                column_title.into(),
                vec![Row {
                    download_action: None,
                    download_id: None,
                    download_status: None,
                    year: None,
                    key: "item".into(),
                    title: title.into(),
                    kind,
                }],
                0,
                BTreeSet::new(),
            );
            assert_eq!(breadcrumb_label(&col), title);
            if kind == Kind::Track {
                col.album_info_selected = true;
                assert_eq!(breadcrumb_label(&col), "Tracks");
            }
        }
    }
    #[test]
    fn grid_covers_grow_virtual_rows_without_changing_targets() {
        let mut state = crate::app::AppState::new();
        state.artist_nav.columns = vec![crate::app::state::BrowseColumn::new(
            "Albums",
            vec![crate::app::state::BrowseItem::Album {
                key: "album".into(),
                title: "Album".into(),
                artist: "Artist".into(),
                is_placeholder: false,
                year: None,
                thumb: None,
            }],
        )];
        let mut column = columns(&state).remove(0);
        let target = column.target(state.library_generation, 0, 0).unwrap();
        column.artwork_visible = true;
        column.album_grid = false;
        let small = column.row_height();
        column.album_grid = true;
        assert!(column.row_height() > small);
        assert_eq!(
            column.target(state.library_generation, 0, 0).unwrap(),
            target
        );
        column.album_grid = false;
        column.artwork_visible = false;
        assert_eq!(column.row_height(), 33.);
    }
    #[test]
    fn immutable_column_rows_and_tracks_are_shared_between_snapshot_clones() {
        let state = fixture();
        let original = columns(&state);
        let next = original.clone();
        assert!(Arc::ptr_eq(&original[0].rows, &next[0].rows));
        assert_eq!(original[0].revision, next[0].revision);
        assert_eq!(original[0].target(0, 0, 0), next[0].target(0, 0, 0));
        let mut state = state;
        state.playback_mode = crate::app::state::PlaybackMode::Queue;
        state.queue.tracks.push(crate::library::models::Track {
            rating_key: "fixture-track".into(),
            title: "Fixture Track".into(),
            ..Default::default()
        });
        let original = queue(&state);
        let next = original.clone();
        assert!(Arc::ptr_eq(&original.tracks, &next.tracks));
        assert_eq!(next.tracks[0].rating_key, "fixture-track");
    }

    #[test]
    fn virtual_folder_metadata_follows_row_keys_without_searching_the_catalog() {
        use crate::library::models::Track;
        let mut state = fixture();
        let track = |key: &str, title: &str| Track {
            rating_key: key.into(),
            title: title.into(),
            ..Default::default()
        };
        let items = vec![
            FolderItem::folder("child".into(), "Child".into()),
            FolderItem::track("a".into(), "A".into(), "a".into(), None, None, None),
            FolderItem::track("b".into(), "B".into(), "b".into(), None, None, None),
        ];
        let col = &mut state.folder_state.as_mut().unwrap().columns[0];
        col.items = items;
        col.tracks = [track("a", "Resolved A"), track("b", "Resolved B")]
            .into_iter()
            .map(|t| (t.rating_key.clone(), t))
            .collect();
        // The global cache may refresh independently of an already-open folder.
        state.library.all_tracks = std::sync::Arc::new(vec![track("a", "Stale catalog A")]);
        let projected = columns(&state);
        assert_eq!(projected[0].tracks[1].title, "Resolved A");
        assert_eq!(projected[0].tracks[2].title, "Resolved B");
        let col = &mut state.folder_state.as_mut().unwrap().columns[0];
        col.shuffle();
        let projected = columns(&state);
        for (row, track) in projected[0].rows.iter().zip(projected[0].tracks.iter()) {
            if row.kind == Kind::Track {
                assert_eq!(row.key, track.rating_key);
            } else {
                assert!(track.rating_key.is_empty());
            }
        }
        std::sync::Arc::make_mut(&mut state.library.all_tracks).clear();
        assert_eq!(
            columns(&state)[0]
                .tracks
                .iter()
                .filter(|t| !t.rating_key.is_empty())
                .count(),
            2
        );
        let json = serde_json::to_string(&state.folder_state.as_ref().unwrap().columns[0]).unwrap();
        assert!(!json.contains("Resolved"));
        let restored: FolderColumn = serde_json::from_str(&json).unwrap();
        assert!(restored.tracks.is_empty());
    }

    #[test]
    fn targets_reject_source_changes_reordered_rows_and_changed_ancestry() {
        let mut state = fixture();
        let target = columns(&state)[0]
            .target(state.library_generation, 0, 1)
            .unwrap();
        assert!(valid(&state, &target));
        state.library_generation += 1;
        assert!(!valid(&state, &target));
        state.library_generation -= 1;
        state.folder_state.as_mut().unwrap().columns[0]
            .items
            .swap(0, 2);
        // Even if the clicked row itself is unchanged, its list revision is stale.
        assert!(!valid(&state, &target));
        state.folder_state.as_mut().unwrap().columns[0]
            .items
            .swap(0, 2);
        state.folder_state.as_mut().unwrap().columns[0].key = Some("another-parent".into());
        assert!(!valid(&state, &target));
    }

    #[test]
    fn context_preserves_group_and_retargets_an_unselected_item() {
        let mut cursor = 1;
        let mut set = BTreeSet::new();
        let mut anchor = None;
        selection(
            &mut cursor,
            &mut set,
            &mut anchor,
            3,
            Selection::Extend,
            true,
        );
        assert_eq!(set, BTreeSet::from([1, 2, 3]));
        selection(
            &mut cursor,
            &mut set,
            &mut anchor,
            2,
            Selection::Context,
            true,
        );
        assert_eq!(set, BTreeSet::from([1, 2, 3]));
        assert_eq!(cursor, 2);
        selection(
            &mut cursor,
            &mut set,
            &mut anchor,
            7,
            Selection::Context,
            true,
        );
        assert!(set.is_empty());
        assert_eq!(cursor, 7);
        selection(
            &mut cursor,
            &mut set,
            &mut anchor,
            9,
            Selection::Toggle,
            true,
        );
        assert_eq!(set, BTreeSet::from([7, 9]));
        selection(
            &mut cursor,
            &mut set,
            &mut anchor,
            7,
            Selection::Single,
            true,
        );
        assert!(set.is_empty());
        assert_eq!(cursor, 7);
    }

    #[test]
    fn folder_cache_does_not_persist_ephemeral_selection() {
        let mut column = fixture().folder_state.unwrap().columns.remove(0);
        column.selected_set.insert(2);
        column.selection_anchor = Some(2);
        let encoded = serde_json::to_string(&column).unwrap();
        assert!(!encoded.contains("selected_set"));
        assert!(!encoded.contains("selection_anchor"));
        let restored: FolderColumn = serde_json::from_str(&encoded).unwrap();
        assert!(restored.selected_set.is_empty());
        assert_eq!(restored.selection_anchor, None);
    }
}

/// The playlist list and artist root already have destinations in the sidebar.
pub fn first_content_column(state: &AppState) -> usize {
    usize::from(
        (state.browse_category == BrowseCategory::Playlists)
            || (state.browse_category == BrowseCategory::Library
                && matches!(
                    state.desktop_library_root,
                    crate::app::state::LibraryRoot::Albums
                        | crate::app::state::LibraryRoot::Compilations
                        | crate::app::state::LibraryRoot::Tracks
                )
                && state.sources.nav_collection.is_none()),
    )
}

/// Shared title normalization matches the terminal's alphabetical navigation.
pub fn jump(column: &Column, letter: char, second: bool) -> Option<usize> {
    use crate::app::handlers::helpers::sort_letter;
    let anchor = if second {
        Some(sort_letter(&column.rows.get(column.selected)?.title, 0)?)
    } else {
        None
    };
    column.rows.iter().position(|row| {
        if anchor.is_some_and(|first| sort_letter(&row.title, 0) != Some(first)) {
            return false;
        }
        let c = sort_letter(&row.title, usize::from(second));
        match letter {
            '#' => c.is_some_and(|c| c.is_ascii_digit()),
            '%' => c.is_some_and(|c| !c.is_alphanumeric()),
            _ => c.is_some_and(|c| c.to_lowercase().eq(letter.to_lowercase())),
        }
    })
}

#[cfg(test)]
mod navigation_tests {
    use super::*;
    use crate::library::models::Track;
    #[test]
    fn playlist_tracks_are_the_first_visible_column_without_changing_core_targets() {
        let mut state = AppState::new();
        state.browse_category = BrowseCategory::Playlists;
        state.playlist_nav.columns = vec![
            crate::app::state::BrowseColumn::new("Playlists", vec![]),
            crate::app::state::BrowseColumn::new_with_tracks(
                "Fixture",
                BrowseItem::from_tracks(&[Track {
                    rating_key: "track".into(),
                    title: "Track".into(),
                    ..Default::default()
                }]),
                vec![Track {
                    rating_key: "track".into(),
                    title: "Track".into(),
                    ..Default::default()
                }],
            ),
        ];
        state.playlist_nav.columns[1].artwork_visible = true;
        let projected = columns(&state);
        assert!(projected[0].hidden);
        assert!(!projected[1].hidden);
        assert!(projected[1].artwork_visible);
        let target = projected[1].target(state.library_generation, 1, 0).unwrap();
        assert!(valid(&state, &target));
        assert_eq!(target.column, 1);
        assert_eq!(first_content_column(&state), 1);
        assert!(columns(&state)[0].hidden);
        assert!(valid(&state, &target));
    }

    #[test]
    fn alphabet_rail_tracks_sort_direction_and_excludes_non_alphabetical_views() {
        use crate::app::state::{BrowseColumn, ColumnSortMode};
        let artist = |name: &str| BrowseItem::Artist {
            key: name.into(),
            title: name.into(),
            thumb: None,
            is_placeholder: false,
        };
        let mut state = AppState::new();
        state.artist_nav.columns = vec![BrowseColumn::new(
            "artists",
            vec![
                BrowseItem::AllArtists,
                artist("The Air"),
                artist("Bach"),
                artist("Coil"),
            ],
        )];
        assert_eq!(columns(&state)[0].alphabet_ascending, Some(true));
        state.artist_nav.columns[0].sort_mode = ColumnSortMode::Shuffled;
        assert_eq!(columns(&state)[0].alphabet_ascending, None);
        state.artist_nav.columns[0].sort_mode = ColumnSortMode::ByTitle;
        state.artist_nav.columns[0].sort_ascending = false;
        assert_eq!(columns(&state)[0].alphabet_ascending, Some(false));
        state.artist_nav.columns[0].sort_mode = ColumnSortMode::ByDuration;
        assert_eq!(columns(&state)[0].alphabet_ascending, None);
        assert_eq!(
            alphabet_order(["Coil", "Bach", "Air"].into_iter()),
            Some(false)
        );
        assert_eq!(alphabet_order(["Coil", "Air", "Bach"].into_iter()), None);
    }

    #[test]
    fn playlist_playback_is_a_header_action_after_grouping_too() {
        use crate::app::state::{BrowseColumn, PlayAllRow};
        let track = Track {
            rating_key: "track".into(),
            title: "Track".into(),
            parent_rating_key: Some("album".into()),
            parent_title: Some("Album".into()),
            ..Default::default()
        };
        let mut state = AppState::new();
        state.browse_category = BrowseCategory::Playlists;
        let mut col = BrowseColumn::new_with_tracks(
            "Playlist",
            BrowseItem::from_tracks(std::slice::from_ref(&track)),
            vec![track],
        );
        col.play_all_row = Some(PlayAllRow::Playlist {
            rating_key: "playlist".into(),
            title: "Playlist".into(),
        });
        state.playlist_nav.columns = vec![col];
        assert_eq!(
            columns(&state)[0].header_controls.first().unwrap().label,
            "Play Playlist"
        );
        state.playlist_nav.columns[0].group_by_album();
        assert!(columns(&state)[0].header_controls.len() == 2);
        assert_eq!(
            state.column_options_type(0),
            Some(crate::app::state::SortColumnType::Album)
        );
        assert!(state.queue.tracks.is_empty());
        state.playlist_nav.columns[0].play_all_row = Some(PlayAllRow::AllTracks {
            label: "Play all tracks".into(),
        });
        let projected = columns(&state);
        let play = projected[0].header_controls.first().unwrap();
        assert_eq!(play.label, "Play All Tracks");
        assert!(
            matches!(play.actions.as_slice(), [crate::app::Action::Queue(crate::app::action::QueueAction::PlayTracksNow(tracks))] if tracks.len() == 1)
        );
    }

    #[test]
    fn loading_indication_ends_with_the_owned_request() {
        let mut state = AppState::new();
        state.browse_category = BrowseCategory::Playlists;
        state.playlist_nav.loading = true;
        assert!(crate::app::activity::message(&state).is_some());
        state.playlist_nav.loading = false;
        assert!(crate::app::activity::message(&state).is_none());
    }

    #[test]
    fn alphabet_jumps_match_terminal_title_normalization_and_special_groups() {
        let column = column(
            "fixture".into(),
            "Titles".into(),
            ["The Beatles", "7 Things", "!Song", "ABBA"]
                .map(|s| Row {
                    download_action: None,
                    download_id: None,
                    download_status: None,
                    year: None,
                    key: s.into(),
                    title: s.into(),
                    kind: Kind::Track,
                })
                .into(),
            0,
            Default::default(),
        );
        assert_eq!(jump(&column, 'b', false), Some(0));
        assert_eq!(jump(&column, '#', false), Some(1));
        assert_eq!(jump(&column, '%', false), Some(2));
        assert_eq!(jump(&column, 'b', true), None);
    }
    #[test]
    fn shifted_letters_stay_in_the_current_sort_letter_section() {
        let mut c = column(
            "fixture".into(),
            "Artists".into(),
            [
                "Air",
                "Alvvays",
                "The Alarm",
                "M83",
                "McCoy",
                "Midlake",
                "Miles Davis",
            ]
            .map(|title| Row {
                download_action: None,
                download_id: None,
                download_status: None,
                year: None,
                key: title.into(),
                title: title.into(),
                kind: Kind::Artist,
            })
            .into(),
            4,
            Default::default(),
        );
        assert_eq!(jump(&c, 'i', true), Some(5));
        assert_eq!(jump(&c, 'l', true), None);
        c.selected = 0;
        assert_eq!(jump(&c, 'l', true), Some(1));
        c.selected = 2;
        assert_eq!(jump(&c, 'l', true), Some(1));
        assert_eq!(jump(&c, 'm', false), Some(3));
    }

    #[test]
    fn playback_queue_targets_survive_workspace_changes_but_reject_reordering() {
        let mut state = AppState::new();
        state.queue.tracks = ["a", "b", "c"]
            .map(|s| Track {
                rating_key: s.into(),
                title: s.into(),
                ..Default::default()
            })
            .into();
        let target = queue(&state)
            .target(state.library_generation, PLAYER_QUEUE, 1)
            .unwrap();
        assert!(valid(&state, &target));
        state.view = View::NowPlaying;
        assert!(valid(&state, &target));
        state.view = View::Browse;
        state.queue.tracks.swap(0, 2);
        assert!(!valid(&state, &target));
    }
}

#[cfg(test)]
mod singles_tests {
    #[test]
    fn singles_album_column_is_visible_without_an_artist_parent() {
        let mut state = crate::app::AppState::new();
        state.library.albums = std::sync::Arc::new(vec![crate::library::models::Album {
            rating_key: "single".into(),
            title: "Single".into(),
            leaf_count: Some(2),
            ..Default::default()
        }]);
        crate::app::album_collections::open_singles(&mut state);
        assert_eq!(super::first_content_column(&state), 0);
        let columns = super::columns(&state);
        assert_eq!(columns.len(), 1);
        assert!(!columns[0].hidden);
        assert_eq!(columns[0].rows[0].title, "Single");
    }
}
