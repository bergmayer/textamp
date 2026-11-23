//! Bounded, library-scoped undo for explicit jumps, independent of playback.
use super::{Action, AppState, state::*};
use crate::app::action::QueueAction;
use crate::library::models::{Album, Track};
use crate::services::FolderNavigationState;
use std::{collections::VecDeque, time::Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LibraryNavigation {
    #[default]
    Home,
    Group(super::sidebar::SidebarGroup),
    Columns(Option<super::sidebar::SidebarGroup>),
}
impl LibraryNavigation {
    pub fn group(self) -> Option<super::sidebar::SidebarGroup> {
        match self {
            Self::Group(group) | Self::Columns(Some(group)) => Some(group),
            _ => None,
        }
    }
    pub fn has_columns(self) -> bool {
        matches!(self, Self::Columns(_))
    }
    pub(crate) fn open_columns(&mut self) {
        *self = Self::Columns(self.group());
    }
    #[cfg(feature = "mobile")]
    pub(crate) fn pop(&mut self, depth: usize) -> usize {
        let group = self.group();
        let column_depth = depth.saturating_sub(usize::from(group.is_some()));
        *self = match (depth, group, column_depth) {
            (0, _, _) => Self::Home,
            (_, Some(group), 0) => Self::Group(group),
            _ => Self::Columns(group),
        };
        column_depth
    }
}

#[derive(Debug, Default)]
pub struct History {
    entries: VecDeque<Snapshot>,
    next_id: u64,
    pub restored: Option<u64>,
}
impl History {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.restored = None;
    }
    pub fn head(&self) -> Option<u64> {
        self.entries.back().map(|entry| entry.id)
    }
}
#[derive(Debug)]
struct Snapshot {
    id: u64,
    generation: u64,
    recorded_at: Instant,
    view: View,
    library_navigation: LibraryNavigation,
    previous: Option<View>,
    category: BrowseCategory,
    focus: Focus,
    category_focused: bool,
    category_index: usize,
    root: LibraryRoot,
    player_visible: bool,
    track_pane_open: bool,
    track_pane_focused: bool,
    track_pane_index: usize,
    column_header: Option<super::column_controls::ColumnHeaderFocus>,
    player_fullscreen: DesktopFullscreenView,
    collection: Option<crate::app::sources::navidrome::commands::CollectionKind>,
    artist: BrowseNavigationState,
    stations: StationNavigationState,
    station_detail: Option<String>,
    playlist: BrowseNavigationState,
    tags: BrowseNavigationState,
    folder: Option<FolderNavigationState>,
    filter: ListFilterState,
    scroll: ScrollPins,
    search: SearchState,
    search_open: bool,
    artist_name: String,
    revealed_compilation_artist: Option<String>,
    album_title: String,
    albums: Vec<Album>,
    tracks: Vec<Track>,
    artist_index: usize,
    album_index: usize,
    search_index: usize,
    miller_col: usize,
    miller_manual: bool,
    sidebar_group: Option<(String, usize)>,
}
pub fn record(state: &mut AppState) {
    if state
        .jump_history
        .entries
        .back()
        .is_some_and(|entry| entry.generation != state.library_generation)
    {
        state.jump_history.entries.clear();
    }
    state.jump_history.next_id = state.jump_history.next_id.wrapping_add(1);
    let entry = Snapshot {
        id: state.jump_history.next_id,
        generation: state.library_generation,
        recorded_at: Instant::now(),
        view: state.view,
        library_navigation: state.library_navigation,
        previous: state.previous_view,
        category: state.browse_category,
        focus: state.focus,
        category_focused: state.category_column_focused,
        category_index: state.category_column_index,
        root: state.desktop_library_root,
        player_visible: state.desktop_player_visible,
        track_pane_open: state.track_pane_open,
        track_pane_focused: state.track_pane_focused,
        track_pane_index: state.track_pane_index,
        column_header: state.tui_column_header,
        player_fullscreen: state.desktop_player_fullscreen,
        collection: state.sources.nav_collection,
        artist: std::mem::take(&mut state.artist_nav),
        stations: state.station_nav.clone(),
        station_detail: state.station_detail.clone(),
        playlist: std::mem::take(&mut state.playlist_nav),
        tags: std::mem::take(&mut state.tag_nav),
        folder: state.folder_state.take(),
        filter: state.list_filter.take_for_navigation(),
        scroll: state.scroll.clone(),
        search: state.search.clone(),
        search_open: state.popups.search_active(),
        artist_name: state.library.selected_artist_name.clone(),
        revealed_compilation_artist: state.revealed_compilation_artist.clone(),
        album_title: state.library.selected_album_title.clone(),
        albums: std::mem::take(&mut state.library.selected_artist_albums),
        tracks: std::mem::take(&mut state.library.selected_album_tracks),
        artist_index: state.list_state.artists_index,
        album_index: state.list_state.right_albums_index,
        search_index: state.list_state.search_item_index,
        miller_col: state.miller_scroll_col,
        miller_manual: state.miller_scroll_manual,
        sidebar_group: state.tui_sidebar_group.clone(),
    };
    state.jump_history.entries.push_back(entry);
    while state.jump_history.entries.len() > 6 {
        state.jump_history.entries.pop_front();
    }
    state.jump_history.restored = None;
}
pub fn undo(state: &mut AppState) -> Vec<Action> {
    restore(state, true)
}
pub fn back(state: &mut AppState) -> Vec<Action> {
    state.tui_column_header = None;
    if state.view == View::NowPlaying
        || (state.view == View::Queue && state.tui_queue_workspace == TuiWorkspace::NowPlaying)
    {
        state.set_view(View::Browse);
        state.desktop_player_visible = false;
        state.desktop_player_fullscreen = DesktopFullscreenView::Closed;
        return vec![];
    }
    let actions = restore(state, false);
    state.desktop_player_visible = false;
    state.desktop_player_fullscreen = DesktopFullscreenView::Closed;
    if state.view == View::NowPlaying {
        state.view = View::Browse;
    }
    actions
}
fn restore(state: &mut AppState, allow_queue_undo: bool) -> Vec<Action> {
    while state
        .jump_history
        .entries
        .back()
        .is_some_and(|entry| entry.generation != state.library_generation)
    {
        state.jump_history.entries.pop_back();
    }
    let jump = state.jump_history.entries.back();
    let queue = state.queue.undo_snapshot.as_ref();
    if jump.is_none() {
        return if allow_queue_undo {
            vec![QueueAction::UndoLastRemix.into()]
        } else {
            vec![]
        };
    }
    if allow_queue_undo
        && queue
            .zip(jump)
            .is_some_and(|(queue, jump)| queue.recorded_at > jump.recorded_at)
    {
        return vec![QueueAction::UndoLastRemix.into()];
    }
    let Some(entry) = state.jump_history.entries.pop_back() else {
        return vec![];
    };
    for slot in [
        "artist-browse",
        "album-browse",
        "playlist-browse",
        "genre-browse",
        "collection",
        "audiomuse",
    ] {
        state.sources.nav_tasks.remove(slot);
    }
    state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
    state.tag_nav_request_id = state.tag_nav_request_id.wrapping_add(1);
    state.playlist_nav_request_id = state.playlist_nav_request_id.wrapping_add(1);
    state.popups.close_all();
    state.palette.close();
    state.sonic_details = None;
    state.pending_library_details = None;
    state.view = entry.view;
    state.library_navigation = entry.library_navigation;
    state.previous_view = entry.previous;
    state.browse_category = entry.category;
    state.focus = entry.focus;
    state.category_column_focused = entry.category_focused;
    state.category_column_index = entry.category_index;
    state.desktop_library_root = entry.root;
    state.desktop_player_visible = entry.player_visible;
    state.track_pane_open = entry.track_pane_open;
    state.track_pane_focused = entry.track_pane_focused;
    state.track_pane_index = entry.track_pane_index;
    state.tui_column_header = entry.column_header;
    state.desktop_player_fullscreen = entry.player_fullscreen;
    state.sources.nav_collection = entry.collection;
    state.artist_nav = entry.artist;
    state.station_nav = entry.stations;
    state.station_nav.loading = false;
    state.station_detail = entry.station_detail;
    state.station_navigation_generation = state.station_navigation_generation.wrapping_add(1);
    state.sources.nav_tasks.remove("station-children");
    state.playlist_nav = entry.playlist;
    state.tag_nav = entry.tags;
    state.folder_state = entry.folder;
    for nav in [
        &mut state.artist_nav,
        &mut state.playlist_nav,
        &mut state.tag_nav,
    ] {
        nav.loading = false;
    }
    if let Some(nav) = state.folder_state.as_mut() {
        nav.loading = false;
    }
    state.list_filter = entry.filter;
    state.list_filter.advance_version();
    state.list_filter.loading = false;
    state.scroll = entry.scroll;
    state.search = entry.search;
    state.popups.set_search_active(entry.search_open);
    state.library.selected_artist_name = entry.artist_name;
    state.revealed_compilation_artist = entry.revealed_compilation_artist;
    state.library.selected_album_title = entry.album_title;
    state.library.selected_artist_albums = entry.albums;
    state.library.selected_album_tracks = entry.tracks;
    state.list_state.artists_index = entry.artist_index;
    state.list_state.right_albums_index = entry.album_index;
    state.list_state.search_item_index = entry.search_index;
    state.miller_scroll_col = entry.miller_col;
    state.miller_scroll_manual = entry.miller_manual;
    state.miller_reveal_right = false;
    state.hit_regions.miller_viewport = None;
    state.tui_sidebar_group = entry.sidebar_group;
    state.jump_history.restored = Some(entry.id);
    state.set_status("Returned to previous library location".into());

    if state.list_filter.active
        && !state.list_filter.query.is_empty()
        && state.list_filter.results.is_none()
    {
        state.list_filter.loading = true;
        vec![
            crate::app::action::SearchAction::RunListFilter {
                version: state.list_filter.version,
            }
            .into(),
        ]
    } else {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> AppState {
        let mut state = AppState::new();
        state.view = View::Browse;
        state.artist_nav = BrowseNavigationState::with_root(
            "Original",
            vec![BrowseItem::Artist {
                key: "old".into(),
                title: "Old Artist".into(),
                thumb: None,
                is_placeholder: false,
            }],
        );
        state.scroll.browse = Some((0, 42));
        state
    }
    #[test]
    fn undo_restores_location_selection_search_and_scroll_without_touching_playback() {
        let mut state = fixture();
        state.search.query = "headcoats".into();
        state.popups.set_search_active(true);
        state.queue.tracks = vec![Track {
            rating_key: "playing".into(),
            ..Default::default()
        }];
        record(&mut state);
        state.artist_nav = BrowseNavigationState::with_root("Destination", vec![]);
        state.popups.close_all();
        state.search.query.clear();
        assert!(undo(&mut state).is_empty());
        assert_eq!(state.artist_nav.columns[0].title, "Original");
        assert_eq!(
            state.artist_nav.columns[0].selected_item().unwrap().key(),
            "old"
        );
        assert_eq!(state.scroll.browse, Some((0, 42)));
        assert_eq!(state.search.query, "headcoats");
        assert!(state.popups.search_active());
        assert_eq!(state.queue.tracks[0].rating_key, "playing");
    }
    #[test]
    fn jumps_cancel_filter_workers_without_reusing_their_generations() {
        let mut state = fixture();
        state.list_filter.active = true;
        state.list_filter.query = "original".into();
        state.list_filter.advance_version();
        let original_version = state.list_filter.version;
        let mut original_worker = state.list_filter.cancellation();
        record(&mut state);
        assert!(original_worker());
        assert!(!state.list_filter.active);
        state.list_filter.active = true;
        state.list_filter.query = "destination".into();
        state.list_filter.advance_version();
        let destination_version = state.list_filter.version;
        let mut destination_worker = state.list_filter.cancellation();
        assert!(destination_version > original_version);
        let actions = back(&mut state);
        assert!(destination_worker());
        assert_eq!(state.list_filter.query, "original");
        assert!(state.list_filter.version > destination_version);
        assert!(matches!(
            actions.as_slice(),
            [Action::Search(
                crate::app::action::SearchAction::RunListFilter { .. }
            )]
        ));
    }
    #[test]
    fn queue_edits_and_jumps_undo_in_user_action_order() {
        let mut state = fixture();
        record(&mut state);
        state.queue.undo_snapshot = Some(QueueSnapshot {
            recorded_at: Instant::now(),
            description: "Clear Queue".into(),
            contents: QueueContents::Queue {
                tracks: vec![],
                index: None,
            },
        });
        assert!(matches!(
            undo(&mut state).as_slice(),
            [Action::Queue(QueueAction::UndoLastRemix)]
        ));
        state.queue.undo_snapshot = None;
        assert!(undo(&mut state).is_empty());
        assert!(state.jump_history.restored.is_some());
    }
    #[test]
    fn back_restores_library_without_undoing_queue_edits_or_opening_player() {
        let mut state = fixture();
        state.desktop_player_visible = true;
        record(&mut state);
        state.track_pane_open = true;
        state.queue.undo_snapshot = Some(QueueSnapshot {
            recorded_at: Instant::now(),
            description: "Queue edit".into(),
            contents: QueueContents::Queue {
                tracks: vec![],
                index: None,
            },
        });
        assert!(back(&mut state).is_empty());
        assert_eq!(state.artist_nav.columns[0].title, "Original");
        assert!(!state.desktop_player_visible);
        assert!(!state.track_pane_open);
        assert!(state.queue.undo_snapshot.is_some());
        assert!(back(&mut state).is_empty());
        assert!(state.queue.undo_snapshot.is_some());
    }
    #[test]
    fn navigation_history_cannot_restore_another_library() {
        let mut state = fixture();
        record(&mut state);
        state.library_generation += 1;
        assert!(matches!(
            undo(&mut state).as_slice(),
            [Action::Queue(QueueAction::UndoLastRemix)]
        ));
        assert!(state.artist_nav.columns.is_empty());
    }
    #[test]
    fn returning_to_a_phone_group_restores_its_logical_destination() {
        let mut state = fixture();
        state.library_navigation =
            LibraryNavigation::Group(super::super::sidebar::SidebarGroup::Playlists);
        record(&mut state);
        state.library_navigation = LibraryNavigation::Columns(None);
        back(&mut state);
        assert_eq!(
            state.library_navigation,
            LibraryNavigation::Group(super::super::sidebar::SidebarGroup::Playlists)
        );
    }
    #[test]
    fn retains_six_nested_jump_destinations_in_reverse_order() {
        let mut state = fixture();
        for index in 0..7 {
            record(&mut state);
            state.artist_nav = BrowseNavigationState::with_root(format!("Jump {index}"), vec![]);
            state.scroll.browse = Some((0, index));
        }
        assert_eq!(state.jump_history.entries.len(), 6);
        for index in (0..=5).rev() {
            assert!(back(&mut state).is_empty());
            assert_eq!(state.artist_nav.columns[0].title, format!("Jump {index}"));
            assert_eq!(state.scroll.browse, Some((0, index)));
        }
        assert!(state.jump_history.head().is_none());
        back(&mut state);
        assert_eq!(state.artist_nav.columns[0].title, "Jump 0");
    }
}
