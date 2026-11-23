//! Sidebar hierarchy and keyboard navigation share one visible-row ordering.
use super::*;

pub use crate::app::sidebar::{allows_disclosure, visible_indices as visible_rows};

pub fn open_command(state: &crate::app::AppState, index: usize) -> Option<Command> {
    let rows = super::catalog::sidebar(state);
    let row = rows.get(index)?.clone();
    let generation = state.library_generation;
    Some(if row.header && !allows_disclosure(&rows, &row.label) {
        Command::SidebarGroup {
            generation,
            title: row.label,
        }
    } else if row.header {
        Command::ToggleSidebarSection {
            generation,
            title: row.label,
        }
    } else if let Some(root) = row.library_root {
        Command::OpenLibraryRoot { generation, root }
    } else if let Some(key) = row.playlist_key {
        Command::OpenPlaylist {
            generation,
            key,
            focus: PlaylistFocus::Content,
        }
    } else {
        Command::GuardedActions {
            generation,
            target: None,
            actions: row.actions,
        }
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PlaylistFocus {
    Sidebar,
    Content,
}

#[derive(Default)]
pub struct PlaylistInspection(Option<PendingPlaylist>);
struct PendingPlaylist {
    generation: u64,
    index: usize,
    key: String,
    until: std::time::Instant,
}
fn playlist_at_cursor(state: &crate::app::AppState, cursor: Option<usize>) -> Option<String> {
    let index = cursor?;
    if state.view != crate::app::View::Browse {
        return None;
    }
    let rows = super::catalog::sidebar(state);
    if !visible_rows(&rows, &state.desktop_collapsed_sections).contains(&index) {
        return None;
    }
    rows.get(index)?.playlist_key.clone()
}
impl PlaylistInspection {
    pub fn schedule(
        &mut self,
        state: &crate::app::AppState,
        cursor: Option<usize>,
        now: std::time::Instant,
    ) {
        self.0 = cursor.and_then(|index| {
            playlist_at_cursor(state, Some(index)).map(|key| PendingPlaylist {
                generation: state.library_generation,
                index,
                key,
                until: now + super::navigation::PAUSE,
            })
        });
    }
    pub fn take(
        &mut self,
        state: &crate::app::AppState,
        cursor: Option<usize>,
        now: std::time::Instant,
    ) -> Option<Command> {
        let pending = self.0.as_ref()?;
        if pending.generation != state.library_generation
            || cursor != Some(pending.index)
            || playlist_at_cursor(state, cursor).as_deref() != Some(pending.key.as_str())
        {
            self.0 = None;
            return None;
        }
        if now < pending.until {
            return None;
        }
        let pending = self.0.take()?;
        Some(Command::OpenPlaylist {
            generation: pending.generation,
            key: pending.key,
            focus: PlaylistFocus::Sidebar,
        })
    }
}
pub fn playlist_has_focus(state: &crate::app::AppState, cursor: Option<usize>, key: &str) -> bool {
    playlist_at_cursor(state, cursor).as_deref() == Some(key)
}

/// Retained rows from a different playlist do not count as an open destination.
pub fn playlist_is_open(state: &crate::app::AppState, key: &str) -> bool {
    let nav = &state.playlist_nav;
    state.browse_category == crate::app::BrowseCategory::Playlists
        && nav.columns.first().and_then(|column|column.selected_item()).is_some_and(|item|item.key()==key)
        && (nav.loading || nav.columns.get(1).and_then(|column|column.play_all_row.as_ref())
            .is_some_and(|row|matches!(row,crate::app::state::PlayAllRow::Playlist{rating_key,..} if rating_key==key)))
}

/// Arrow movement changes keyboard focus; playlist inspection is deferred.
pub fn navigate(
    state: &crate::app::AppState,
    cursor: &mut Option<usize>,
    column_delta: i8,
    row_delta: i8,
) -> Option<Command> {
    let index = (*cursor)?;
    let rows = super::catalog::sidebar(state);
    let visible = visible_rows(&rows, &state.desktop_collapsed_sections);
    if row_delta != 0 {
        let position = visible.iter().position(|i| *i == index).unwrap_or(0);
        *cursor = visible
            .get(
                position
                    .saturating_add_signed(row_delta as isize)
                    .min(visible.len().saturating_sub(1)),
            )
            .copied();
    } else if column_delta > 0 {
        if rows.get(index).is_some_and(|r| {
            r.header
                && allows_disclosure(&rows, &r.label)
                && !state.desktop_collapsed_sections.contains(&r.label)
        }) {
            *cursor = visible
                .iter()
                .position(|i| *i == index)
                .and_then(|i| visible.get(i + 1))
                .copied();
        } else {
            return Some(Command::SidebarOpen {
                generation: state.library_generation,
                index,
            });
        }
    } else if let Some((header, row)) = rows
        .iter()
        .enumerate()
        .take(index + 1)
        .rev()
        .find(|(_, r)| r.header && !matches!(r.label.as_str(), "Library" | "Browse"))
    {
        *cursor = Some(header);
        if !state.desktop_collapsed_sections.contains(&row.label) {
            return Some(Command::ToggleSidebarSection {
                generation: state.library_generation,
                title: row.label.clone(),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppState, state::LibraryRoot};
    fn playlists() -> (AppState, Vec<usize>) {
        let mut state = AppState::new();
        state.view = crate::app::View::Browse;
        let source = crate::navidrome::Source {
            id: "fixture".into(),
            name: "Fixture".into(),
            url: "http://127.0.0.1:9".into(),
            username: "fixture".into(),
            libraries: vec![],
        };
        state.sources.active = crate::app::sources::ActiveSource::Navidrome(Box::new(
            crate::app::sources::navidrome::Session {
                client: crate::navidrome::Client::new(&source, "fixture-password".into(), None)
                    .unwrap(),
                source,
                extensions: Default::default(),
            },
        ));
        state.library.playlists = std::sync::Arc::new(
            ["A", "B"]
                .into_iter()
                .map(|key| {
                    serde_json::from_value(
                        serde_json::json!({"ratingKey":key,"key":key,"title":key,"type":"audio"}),
                    )
                    .unwrap()
                })
                .collect(),
        );
        let indices = super::super::catalog::sidebar(&state)
            .iter()
            .enumerate()
            .filter_map(|(i, row)| row.playlist_key.as_ref().map(|_| i))
            .collect();
        (state, indices)
    }
    #[::core::prelude::v1::test]
    fn playlist_highlight_uses_identity_and_loading_has_a_specific_status() {
        let (mut state, _) = playlists();
        state.browse_category = crate::app::BrowseCategory::Playlists;
        assert!(state.select_playlist("B"));
        state.category_column_index = 0;
        state.playlist_nav.loading = true;
        let rows = super::super::catalog::sidebar(&state);
        assert!(
            rows.iter()
                .any(|row| row.playlist_key.as_deref() == Some("B") && row.active)
        );
        assert!(
            !rows
                .iter()
                .any(|row| row.playlist_key.as_deref() == Some("A") && row.active)
        );
        assert_eq!(
            crate::app::activity::message(&state),
            Some("Loading playlist…")
        );
        state.playlist_nav.loading = false;
        assert_eq!(crate::app::activity::message(&state), None);
    }

    #[::core::prelude::v1::test]
    fn retained_tracks_from_another_playlist_do_not_prevent_open_or_retry() {
        use crate::app::state::{BrowseColumn, BrowseItem, PlayAllRow};
        let (mut state, _) = playlists();
        state.browse_category = crate::app::BrowseCategory::Playlists;
        state.playlist_nav.columns = vec![
            BrowseColumn::new(
                "Playlists",
                BrowseItem::from_playlists(&state.library.playlists),
            ),
            BrowseColumn::new("A", vec![]),
        ];
        state.playlist_nav.columns[0].selected_index = 1;
        state.playlist_nav.columns[1].play_all_row = Some(PlayAllRow::Playlist {
            rating_key: "A".into(),
            title: "A".into(),
        });
        assert!(!playlist_is_open(&state, "B"));
        state.playlist_nav.loading = true;
        assert!(playlist_is_open(&state, "B"));
        state.playlist_nav.loading = false;
        state.playlist_nav.columns[1].play_all_row = Some(PlayAllRow::Playlist {
            rating_key: "B".into(),
            title: "B".into(),
        });
        assert!(playlist_is_open(&state, "B"));
        assert!(!playlist_is_open(&state, "A"));
    }
    #[::core::prelude::v1::test]
    fn sidebar_playlist_inspection_waits_for_the_latest_row_and_keeps_sidebar_focus() {
        let (state, rows) = playlists();
        assert_eq!(rows.len(), 2);
        let now = std::time::Instant::now();
        let mut inspection = PlaylistInspection::default();
        let mut cursor = Some(rows[0]);
        inspection.schedule(&state, cursor, now);
        assert!(
            inspection
                .take(&state, cursor, now + super::super::navigation::PAUSE / 2)
                .is_none()
        );
        assert!(navigate(&state, &mut cursor, 0, 1).is_none());
        assert_eq!(cursor, Some(rows[1]));
        inspection.schedule(&state, cursor, now + std::time::Duration::from_millis(100));
        assert!(
            inspection
                .take(&state, cursor, now + super::super::navigation::PAUSE)
                .is_none()
        );
        assert!(
            matches!(inspection.take(&state, cursor, now + super::super::navigation::PAUSE + std::time::Duration::from_millis(100)), Some(Command::OpenPlaylist{key,focus:PlaylistFocus::Sidebar,..}) if key=="B")
        );
        assert_eq!(cursor, Some(rows[1]));
        assert!(state.queue.tracks.is_empty());
        assert!(
            inspection
                .take(&state, cursor, now + std::time::Duration::from_secs(1))
                .is_none()
        );
        assert!(matches!(
            open_command(&state, rows[1]),
            Some(Command::OpenPlaylist {
                focus: PlaylistFocus::Content,
                ..
            })
        ));
    }
    #[::core::prelude::v1::test]
    fn leaving_sidebar_changing_source_or_hiding_playlist_cancels_inspection() {
        let (mut state, rows) = playlists();
        let now = std::time::Instant::now();
        let due = now + super::super::navigation::PAUSE;
        let mut inspection = PlaylistInspection::default();
        inspection.schedule(&state, Some(rows[0]), now);
        assert!(inspection.take(&state, None, due).is_none());
        inspection.schedule(&state, Some(rows[0]), now);
        state.library_generation += 1;
        assert!(inspection.take(&state, Some(rows[0]), due).is_none());
        inspection.schedule(&state, Some(rows[0]), now);
        state.desktop_collapsed_sections.push("Playlists".into());
        assert!(inspection.take(&state, Some(rows[0]), due).is_none());
        state.desktop_collapsed_sections.clear();
        inspection.schedule(&state, Some(rows[0]), now);
        std::sync::Arc::make_mut(&mut state.library.playlists).swap(0, 1);
        assert!(inspection.take(&state, Some(rows[0]), due).is_none());
    }
    #[::core::prelude::v1::test]
    fn keyboard_sidebar_skips_collapsed_contents_and_opens_the_chosen_destination() {
        let mut state = AppState::new();
        let rows = super::super::catalog::sidebar(&state);
        let artists = rows
            .iter()
            .position(|r| r.library_root == Some(LibraryRoot::Artists))
            .unwrap();
        let all_albums = rows
            .iter()
            .position(|r| r.library_root == Some(LibraryRoot::Albums))
            .unwrap();
        let albums = rows
            .iter()
            .position(|r| r.header && r.label == "Albums")
            .unwrap();
        let mut cursor = Some(artists);
        assert!(navigate(&state, &mut cursor, 0, 1).is_none());
        assert_ne!(cursor, Some(artists));
        cursor = Some(artists);
        assert!(
            matches!(navigate(&state,&mut cursor,1,0),Some(Command::SidebarOpen{index,..}) if index==artists)
        );
        assert!(matches!(
            open_command(&state, artists),
            Some(Command::OpenLibraryRoot {
                root: LibraryRoot::Artists,
                ..
            })
        ));
        assert!(state.queue.tracks.is_empty());
        state.desktop_collapsed_sections.push("Albums".into());
        let visible = visible_rows(
            &super::super::catalog::sidebar(&state),
            &state.desktop_collapsed_sections,
        );
        assert!(visible.contains(&albums));
        assert!(!visible.contains(&all_albums));
        assert!(visible.contains(&artists));
        cursor = Some(albums);
        assert!(
            matches!(navigate(&state,&mut cursor,1,0),Some(Command::SidebarOpen{index,..}) if index==albums)
        );
        assert!(
            matches!(open_command(&state,albums),Some(Command::ToggleSidebarSection{title,..}) if title=="Albums")
        );
    }
}
