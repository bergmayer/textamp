//! Debounce keyboard inspection while keeping selection and scrolling immediate.
use crate::app::{
    Action, AppState,
    state::{BrowseCategory, ColumnHeaderSelection, View},
};
use std::time::{Duration, Instant};
pub(super) const PAUSE: Duration = Duration::from_millis(250);
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Selection {
    generation: u64,
    category: BrowseCategory,
    column: usize,
    key: String,
    parents: Vec<String>,
    info: bool,
}
impl Selection {
    fn current(state: &AppState) -> Option<Self> {
        if state.view != View::Browse || state.category_column_focused || state.palette.open {
            return None;
        }
        let (column, key, parents, info) = if state.browse_category == BrowseCategory::Folders {
            let nav = state.folder_state.as_ref()?;
            let column = nav.focused_column;
            (
                column,
                nav.columns.get(column)?.selected_item()?.key.clone(),
                nav.columns
                    .iter()
                    .take(column)
                    .filter_map(|column| column.selected_item().map(|item| item.key.clone()))
                    .collect(),
                false,
            )
        } else {
            let nav = state.browse_nav()?;
            let column = nav.focused_column;
            let current = nav.columns.get(column)?;
            (
                column,
                current.selected_item()?.key().to_owned(),
                nav.columns
                    .iter()
                    .take(column)
                    .filter_map(|column| column.selected_item().map(|item| item.key().to_owned()))
                    .collect(),
                current.header_selection == ColumnHeaderSelection::AlbumDetails,
            )
        };
        Some(Self {
            generation: state.library_generation,
            category: state.browse_category,
            column,
            key,
            parents,
            info,
        })
    }
    fn truncate(&self, state: &mut AppState) {
        if state.library_generation != self.generation || state.browse_category != self.category {
            return;
        }
        if self.category == BrowseCategory::Folders {
            if let Some(nav) = state.folder_state.as_mut() {
                nav.columns.truncate(self.column + 1);
            }
        } else if let Some(nav) = state.browse_nav_mut() {
            nav.columns.truncate(self.column + 1);
        }
    }
}
/// A stable rendered browser while a new selection's dependent content is pending.
#[derive(Clone, Default)]
pub(super) struct Presentation {
    pub columns: Vec<super::browser::Column>,
    pub preview: Option<super::catalog::Preview>,
    pub artwork: Option<std::sync::Arc<Vec<u8>>>,
}

#[derive(Default)]
pub(super) enum Inspection {
    #[default]
    Idle,
    Waiting {
        selection: Selection,
        until: Instant,
        actions: Vec<Action>,
        retained: Presentation,
    },
    Resolving {
        selection: Selection,
        retained: Presentation,
    },
}
impl Inspection {
    pub fn pending_column(&self) -> Option<usize> {
        match self {
            Self::Waiting { selection, .. } | Self::Resolving { selection, .. } => {
                Some(selection.column)
            }
            Self::Idle => None,
        }
    }
    pub fn schedule(
        &mut self,
        state: &mut AppState,
        actions: Vec<Action>,
        now: Instant,
        retained: Presentation,
    ) {
        let Some(selection) = Selection::current(state) else {
            *self = Self::Idle;
            return;
        };
        cancel_listings(state);
        *self = Self::Waiting {
            selection,
            until: now + PAUSE,
            actions,
            retained,
        };
    }
    /// Commit only after the selected entry's owned listing has finished. Cached
    /// listings commit in the same actor turn; network listings keep their panels.
    pub fn settle(&mut self, state: &AppState) {
        let selection = match self {
            Self::Idle => return,
            Self::Waiting { selection, .. } | Self::Resolving { selection, .. } => selection,
        };
        let current = Selection::current(state).as_ref() == Some(selection);
        let loading = state.browse_nav().is_some_and(|nav| nav.loading)
            || (state.browse_category == BrowseCategory::Folders
                && state.folder_state.as_ref().is_some_and(|nav| nav.loading))
            || state.sources.listing.is_some()
            || [
                "artist-browse",
                "album-browse",
                "playlist-browse",
                "genre-browse",
            ]
            .iter()
            .any(|slot| state.sources.nav_tasks.contains_key(slot));
        if !current || (matches!(self, Self::Resolving { .. }) && !loading) {
            *self = Self::Idle;
        }
    }
    pub fn project(&self, mut current: Presentation) -> Presentation {
        let (selection, retained) = match self {
            Self::Idle => return current,
            Self::Waiting {
                selection,
                retained,
                ..
            }
            | Self::Resolving {
                selection,
                retained,
            } => (selection, retained),
        };
        current.columns.truncate(selection.column + 1);
        current
            .columns
            .extend(retained.columns.iter().skip(selection.column + 1).cloned());
        current.preview = retained.preview.clone();
        current.artwork = retained.artwork.clone();
        current
    }
    pub fn cancel(&mut self, state: &mut AppState) {
        match std::mem::take(self) {
            Self::Waiting { selection, .. } | Self::Resolving { selection, .. } => {
                cancel_listings(state);
                selection.truncate(state);
            }
            Self::Idle => {}
        }
    }
    pub fn take(
        &mut self,
        state: &mut AppState,
        now: Instant,
        immediate: bool,
    ) -> Option<Vec<Action>> {
        let Self::Waiting {
            selection, until, ..
        } = self
        else {
            return None;
        };
        if Selection::current(state).as_ref() != Some(selection) {
            *self = Self::Idle;
            return None;
        }
        if !immediate && now < *until {
            return None;
        }
        let Self::Waiting {
            selection,
            actions,
            retained,
            ..
        } = std::mem::take(self)
        else {
            unreachable!()
        };
        selection.truncate(state);
        *self = Self::Resolving {
            selection,
            retained,
        };
        Some(actions)
    }
}

fn cancel_listings(state: &mut AppState) {
    state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
    state.tag_nav_request_id = state.tag_nav_request_id.wrapping_add(1);
    state.playlist_nav_request_id = state.playlist_nav_request_id.wrapping_add(1);
    state.sources.list_id = state.sources.list_id.wrapping_add(1);
    state.sources.listing = None;
    if state.browse_category == BrowseCategory::Folders {
        if let Some(nav) = state.folder_state.as_mut() {
            nav.loading = false;
        }
    } else if let Some(nav) = state.browse_nav_mut() {
        nav.loading = false;
    }

    for slot in [
        "artist-browse",
        "album-browse",
        "playlist-browse",
        "genre-browse",
    ] {
        state.sources.nav_tasks.remove(slot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{
        action::SystemAction,
        state::{BrowseColumn, BrowseItem},
    };
    fn browser() -> AppState {
        let mut state = AppState::new();
        state.view = View::Browse;
        state.browse_category = BrowseCategory::Library;
        state.category_column_focused = false;
        state.artist_nav.columns = vec![
            BrowseColumn::new(
                "artists",
                vec![
                    BrowseItem::Artist {
                        key: "a".into(),
                        title: "A".into(),
                        thumb: None,
                        is_placeholder: false,
                    },
                    BrowseItem::Artist {
                        key: "b".into(),
                        title: "B".into(),
                        thumb: None,
                        is_placeholder: false,
                    },
                ],
            ),
            BrowseColumn::new("old albums", vec![BrowseItem::AllArtists]),
        ];
        state
    }
    fn presentation(state: &AppState, title: &str) -> Presentation {
        Presentation {
            columns: super::super::browser::columns(state),
            preview: Some(super::super::catalog::Preview {
                title: title.into(),
                ..Default::default()
            }),
            artwork: Some(std::sync::Arc::new(vec![1, 2, 3])),
        }
    }
    #[test]
    fn dependent_columns_details_and_artwork_stay_visible_until_replacement_is_ready() {
        let mut state = browser();
        let retained = presentation(&state, "Old album");
        let identity = retained.columns[1].identity.clone();
        let target = retained.columns[1]
            .target(state.library_generation, 1, 0)
            .unwrap();
        state.artist_nav.columns[0].selected_index = 1;
        let now = Instant::now();
        let mut inspection = Inspection::default();
        inspection.schedule(&mut state, vec![], now, retained);
        let mut fresh = presentation(&state, "Unsettled");
        fresh.artwork = None;
        let held = inspection.project(fresh);
        assert_eq!(held.columns[0].selected, 1);
        assert_eq!(held.columns.len(), 2);
        assert_eq!(held.columns[1].identity, identity);
        assert!(!super::super::browser::valid(&state, &target));
        assert!(!held.preview.as_ref().unwrap().loading);
        assert_eq!(held.preview.unwrap().title, "Old album");
        assert_eq!(held.artwork, Some(std::sync::Arc::new(vec![1, 2, 3])));
        assert!(inspection.take(&mut state, now + PAUSE, false).is_some());
        state.artist_nav.loading = true;
        inspection.settle(&state);
        let loading = inspection.project(presentation(&state, "Loading"));
        assert_eq!(loading.columns.len(), 2);
        assert_eq!(loading.columns[1].identity, identity);
        assert_eq!(loading.preview.unwrap().title, "Old album");
        state
            .artist_nav
            .columns
            .push(BrowseColumn::new("new albums", vec![]));
        state.artist_nav.loading = false;
        inspection.settle(&state);
        let ready = inspection.project(presentation(&state, "New album"));
        assert_eq!(ready.columns[1].title, "new albums");
        assert_eq!(ready.preview.unwrap().title, "New album");
        assert!(inspection.pending_column().is_none());
    }
    #[test]
    fn another_key_during_loading_keeps_panels_and_supersedes_the_request() {
        let mut state = browser();
        let now = Instant::now();
        let retained = presentation(&state, "Old album");
        let mut inspection = Inspection::default();
        inspection.schedule(&mut state, vec![], now, retained.clone());
        inspection.take(&mut state, now + PAUSE, false).unwrap();
        state.artist_nav.loading = true;
        let held = inspection.project(presentation(&state, "Loading"));
        let request = state.artist_nav_request_id;
        state.artist_nav.columns[0].selected_index = 1;
        inspection.schedule(&mut state, vec![], now + PAUSE, held);
        assert_ne!(state.artist_nav_request_id, request);
        assert!(!state.artist_nav.loading);
        let held = inspection.project(presentation(&state, "Unsettled"));
        assert_eq!(held.columns.len(), 2);
        assert_eq!(held.columns[1].title, "old albums");
        assert_eq!(held.preview.unwrap().title, "Old album");
        assert!(
            inspection
                .take(&mut state, now + PAUSE + PAUSE / 2, false)
                .is_none()
        );
    }
    #[test]
    fn rapid_navigation_only_commits_the_last_selection_after_a_pause() {
        let mut state = browser();
        let mut inspection = Inspection::default();
        let now = Instant::now();
        state.artist_nav.loading = true;
        inspection.schedule(
            &mut state,
            vec![SystemAction::SetStatus("first".into()).into()],
            now,
            Presentation::default(),
        );
        assert!(!state.artist_nav.loading);
        assert!(
            inspection
                .take(&mut state, now + Duration::from_millis(149), false)
                .is_none()
        );
        state.artist_nav.columns[0].selected_index = 1;
        inspection.schedule(
            &mut state,
            vec![SystemAction::SetStatus("last".into()).into()],
            now + Duration::from_millis(100),
            Presentation::default(),
        );
        assert!(inspection.take(&mut state, now + PAUSE, false).is_none());
        let actions = inspection
            .take(&mut state, now + PAUSE + Duration::from_millis(100), false)
            .unwrap();
        assert!(
            matches!(actions.as_slice(), [Action::System(SystemAction::SetStatus(message))] if message == "last")
        );
        assert_eq!(state.artist_nav.columns.len(), 1);
        assert_eq!(state.artist_nav.columns[0].selected_index, 1);
        assert!(state.queue.tracks.is_empty());
    }
    #[test]
    fn source_changes_and_selection_changes_discard_pending_effects() {
        let mut state = browser();
        let mut inspection = Inspection::default();
        let now = Instant::now();
        inspection.schedule(
            &mut state,
            vec![SystemAction::Quit.into()],
            now,
            Presentation::default(),
        );
        state.library_generation += 1;
        assert!(inspection.take(&mut state, now + PAUSE, false).is_none());
        inspection.schedule(
            &mut state,
            vec![SystemAction::Quit.into()],
            now,
            Presentation::default(),
        );
        state.artist_nav.columns[0].selected_index = 1;
        assert!(inspection.take(&mut state, now + PAUSE, false).is_none());
    }
    #[test]
    fn explicit_entry_flushes_immediately_and_left_cancels() {
        let mut state = browser();
        let mut inspection = Inspection::default();
        let now = Instant::now();
        inspection.schedule(
            &mut state,
            vec![SystemAction::SetStatus("open".into()).into()],
            now,
            Presentation::default(),
        );
        assert_eq!(inspection.take(&mut state, now, true).unwrap().len(), 1);
        inspection.schedule(
            &mut state,
            vec![SystemAction::Quit.into()],
            now,
            Presentation::default(),
        );
        inspection.cancel(&mut state);
        assert!(inspection.take(&mut state, now + PAUSE, false).is_none());
    }
}

/// Acceleration belongs to the physical key hold, not to the selection's loading time.
#[derive(Default)]
pub(super) struct ArrowRepeat(Option<(i8, Instant, Instant)>);
impl ArrowRepeat {
    pub fn steps(&mut self, direction: i8, held: bool, now: Instant) -> i8 {
        let start = self
            .0
            .filter(|(previous, _, last)| {
                held && *previous == direction
                    && now.duration_since(*last) < Duration::from_millis(1000)
            })
            .map_or(now, |(_, start, _)| start);
        self.0 = Some((direction, start, now));
        match now.duration_since(start).as_millis() {
            0..=249 => 1,
            250..=499 => 2,
            500..=999 => 5,
            _ => 15,
        }
    }
}
#[cfg(test)]
mod repeat_tests {
    use super::*;
    #[test]
    fn held_arrows_accelerate_after_a_pause_and_reset_on_release_or_direction_change() {
        let mut repeat = ArrowRepeat::default();
        let now = Instant::now();
        assert_eq!(repeat.steps(1, false, now), 1);
        assert_eq!(repeat.steps(1, true, now + Duration::from_millis(500)), 5);
        assert_eq!(repeat.steps(1, true, now + Duration::from_millis(1050)), 15);
        assert_eq!(repeat.steps(-1, true, now + Duration::from_millis(1100)), 1);
        assert_eq!(
            repeat.steps(-1, false, now + Duration::from_millis(1200)),
            1
        );
        assert_eq!(repeat.steps(-1, true, now + Duration::from_millis(2500)), 1);
    }
}
