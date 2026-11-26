//! Sidebar hierarchy and keyboard navigation share one visible-row ordering.
use super::*;

pub use crate::app::sidebar::{allows_disclosure, rail_indices as visible_rows};

pub(super) fn icon(row: &crate::app::sidebar::SidebarRow) -> gpui_kit::component::Icon {
    glyph(row)
        .map(gpui_kit::component::Icon::new)
        .unwrap_or_else(super::player_devices::sidebar_icon)
}

pub(super) fn glyph(row: &crate::app::sidebar::SidebarRow) -> Option<gpui_kit::assets::IconName> {
    use crate::app::{sidebar::SidebarGroup, state::LibraryRoot};
    use gpui_kit::assets::IconName as Glyph;

    let glyph = if row.playlist_key.is_some() {
        Glyph::ListMusic
    } else if row.parent == Some(SidebarGroup::Radio) {
        match row.label.as_str() {
            "Play Random Albums" => Glyph::Shuffle,
            "Random Artist Mix" => Glyph::Dices,
            "Artist Mix" => Glyph::UsersRound,
            "Library Mix" => Glyph::Library,
            "Deep Cuts Mix" => Glyph::Pickaxe,
            "Time Travel Mix" => Glyph::ClockArrowLeft,
            "On This Day" => Glyph::CalendarDays,
            "Mood Mix" => Glyph::Drama,
            "Style / Genre Mix" => Glyph::Blend,
            "Decade Mix" => Glyph::Hourglass,
            "Sonic Mix" => Glyph::AudioWaveform,
            "Stay Close Mix" => Glyph::Magnet,
            _ => Glyph::Antenna,
        }
    } else if let Some(root) = row.library_root {
        match root {
            LibraryRoot::Artists => Glyph::MicVocal,
            LibraryRoot::Albums => Glyph::Disc3,
            LibraryRoot::Compilations => Glyph::Layers,
            LibraryRoot::Singles => Glyph::Disc2,
            LibraryRoot::Tracks => Glyph::Music2,
            LibraryRoot::Radio => Glyph::Radio,
            LibraryRoot::Downloads => Glyph::Download,
            LibraryRoot::Journey => Glyph::Route,
            LibraryRoot::DescribeMusic => Glyph::MessageSquareText,
            LibraryRoot::Mp3Players => return None,
        }
    } else {
        match row.label.as_str() {
            "Albums" => Glyph::Album,
            "Playlists" => Glyph::ListMusic,
            "AI" => Glyph::Sparkles,
            "Folders" => Glyph::Folder,
            "Genres" => Glyph::Guitar,
            "Artist Genres" => Glyph::Mic,
            "Moods" => Glyph::Drama,
            "Styles" => Glyph::Blend,
            "Decades" => Glyph::Hourglass,
            "Years" => Glyph::CalendarDays,
            "Collections" => Glyph::Library,
            "Countries" => Glyph::Globe,
            "Labels" => Glyph::Tag,
            "Formats" => Glyph::FileMusic,
            "Studios" => Glyph::SlidersVertical,
            "Random" => Glyph::Shuffle,
            "Favourites" | "Favorite tracks" => Glyph::Heart,
            "Favorite artists" => Glyph::ContactRound,
            "Top Rated" => Glyph::Star,
            "Recently Added" => Glyph::ListPlus,
            "Recently Played" => Glyph::ClockArrowLeft,
            "Most Played" => Glyph::ChartNoAxesColumnIncreasing,
            "AI labels" => Glyph::Tags,
            "Tempo" => Glyph::Timer,
            "Energy" => Glyph::Zap,
            "Musical key" => Glyph::Piano,
            "Analyzed tracks" => Glyph::AudioLines,
            "Search lyrics…" => Glyph::TextSearch,
            _ => Glyph::Library,
        }
    };
    Some(glyph)
}

pub fn open_command(state: &crate::app::AppState, index: usize) -> Option<Command> {
    let rows = super::catalog::sidebar(state);
    let row = rows.get(index)?.clone();
    let generation = state.library_generation;
    Some(
        if row.header && (!state.tui_sidebar_visible || !allows_disclosure(&rows, &row.label)) {
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
            Command::OpenPlaylist { generation, key }
        } else {
            Command::GuardedActions {
                generation,
                target: None,
                actions: row.actions,
            }
        },
    )
}

/// Retained rows from a different playlist do not count as an open destination.
pub fn playlist_is_open(state: &crate::app::AppState, key: &str) -> bool {
    let nav = &state.playlist_nav;
    state.browse_category == crate::app::BrowseCategory::Playlists
        && nav.columns.first().and_then(|column|column.selected_item()).is_some_and(|item|item.key()==key)
        && (nav.loading || nav.columns.get(1).and_then(|column|column.play_all_row.as_ref())
            .is_some_and(|row|matches!(row,crate::app::state::PlayAllRow::Playlist{rating_key,..} if rating_key==key)))
}

/// Arrow movement changes keyboard focus within visible destinations.
pub fn navigate(
    state: &crate::app::AppState,
    cursor: &mut Option<usize>,
    column_delta: i8,
    row_delta: i8,
) -> Option<Command> {
    let index = (*cursor)?;
    let rows = super::catalog::sidebar(state);
    let visible = visible_rows(
        &rows,
        &state.desktop_collapsed_sections,
        !state.tui_sidebar_visible,
    );
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
                && state.tui_sidebar_visible
                && allows_disclosure(&rows, &r.label)
                && !state.desktop_collapsed_sections.contains(&r.label)
        }) {
            *cursor = visible
                .iter()
                .position(|i| *i == index)
                .and_then(|i| visible.get(i + 1))
                .copied();
        } else {
            *cursor = None;
        }
    } else if state.tui_sidebar_visible
        && let Some((header, row)) = rows
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
        assert!(navigate(&state, &mut cursor, 1, 0).is_none());
        assert_eq!(cursor, None);
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
            false,
        );
        assert!(visible.contains(&albums));
        assert!(!visible.contains(&all_albums));
        assert!(visible.contains(&artists));
        cursor = Some(albums);
        assert!(navigate(&state, &mut cursor, 1, 0).is_none());
        assert_eq!(cursor, None);
        assert!(
            matches!(open_command(&state,albums),Some(Command::ToggleSidebarSection{title,..}) if title=="Albums")
        );
    }
}
