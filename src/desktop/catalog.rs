//! Desktop catalog projection. Provider policy and commands stay in the app.
use crate::app::{AppState, action::*, state::*};
use crate::library::models::Track;

pub use crate::app::playback_controls::Control;

/// Native tooltip presentation for a shared command definition.
pub trait ControlTooltip {
    fn actions_for(&self, option: bool, shift: bool) -> Vec<Action>;
    fn tooltip(
        &self,
    ) -> impl Fn(&mut gpui_kit::Window, &mut gpui_kit::App) -> gpui_kit::AnyView + 'static;
}
fn gesture(control: &Control, option: bool, shift: bool) -> crate::app::playback_controls::Variant {
    use crate::app::playback_controls::Variant;
    let _ = shift;
    if option && control.has_next() {
        Variant::Next
    } else {
        Variant::Primary
    }
}
impl ControlTooltip for Control {
    fn actions_for(&self, option: bool, shift: bool) -> Vec<Action> {
        self.resolve(gesture(self, option, shift)).1.to_vec()
    }
    fn tooltip(
        &self,
    ) -> impl Fn(&mut gpui_kit::Window, &mut gpui_kit::App) -> gpui_kit::AnyView + 'static {
        let control = self.clone();
        move |window, cx| {
            let control = control.clone();
            gpui_kit::component::tooltip::Tooltip::element(move |window, _| {
                use gpui_kit::{InteractiveElement, ParentElement};
                let label = control
                    .resolve(gesture(
                        &control,
                        window.modifiers().alt,
                        window.modifiers().shift,
                    ))
                    .0;
                gpui_kit::div()
                    .child(label.to_owned())
                    .on_modifiers_changed(|_, window, _| window.refresh())
            })
            .build(window, cx)
        }
    }
}

pub(super) fn compact_controls(mut controls: Vec<Control>) -> Vec<Control> {
    if controls.len() >= 3 {
        let next = controls.remove(1);
        controls[0] = controls[0].clone().with_next_control(&next);
    }
    controls
}

pub fn collection_controls(row: &PlayAllRow, tracks: &[Track]) -> Vec<Control> {
    let mut controls = crate::app::playback_controls::collection_controls(row, tracks);
    if matches!(row, PlayAllRow::Album { .. }) {
        controls[0] = controls[0].clone().with_next_control(&controls[1]);
        controls
    } else {
        compact_controls(controls)
    }
}
pub use crate::app::details::Kind as PreviewKind;
pub use crate::app::details::{Document as Preview, similar_seed};
pub use crate::app::sidebar::{SidebarRow, sidebar};

/// Adapt shared detail commands to the desktop's compact playback controls.
pub fn preview(state: &AppState) -> Option<Preview> {
    let mut document = crate::app::details::project(state)?;
    if document.kind != PreviewKind::Artist {
        document.controls = compact_controls(document.controls);
    }
    Some(document)
}
#[cfg(test)]
mod sidebar_tests {
    use super::*;
    #[test]
    fn album_details_keep_navigation_metadata_for_uncatalogued_playlist_albums() {
        let mut state = AppState::new();
        let track = Track {
            rating_key: "track".into(),
            parent_rating_key: Some("outside-album".into()),
            parent_title: Some("Outside Album".into()),
            grandparent_rating_key: Some("artist".into()),
            grandparent_title: Some("Artist".into()),
            genres: vec!["Ambient".into()],
            ..Default::default()
        };
        let tracks = BrowseColumn::new_with_tracks(
            "Tracks",
            BrowseItem::from_tracks(std::slice::from_ref(&track)),
            vec![track],
        );
        let mut details = BrowseColumn::new("Album Details", vec![]);
        details.details = Some(BrowseDetails::Album {
            artwork_key: "outside-album".into(),
            lines: vec!["Artist: Artist".into(), "Genres: Ambient".into()],
        });
        state.artist_nav.columns = vec![tracks, details];
        state.artist_nav.focused_column = 1;
        let detail = preview(&state).unwrap();
        let track = detail.track.unwrap();
        assert_eq!(track.parent_rating_key.as_deref(), Some("outside-album"));
        assert_eq!(track.grandparent_rating_key.as_deref(), Some("artist"));
        assert_eq!(detail.genres, vec!["Ambient"]);
    }
    #[test]
    fn explicit_album_details_uses_destination_tracks_for_similar_seed() {
        let mut state = AppState::new();
        let source = crate::navidrome::Source {
            id: "fixture".into(),
            name: "Fixture".into(),
            url: "http://127.0.0.1:1".into(),
            username: "fixture".into(),
            libraries: vec![],
        };
        let session = crate::app::sources::navidrome::Session {
            client: crate::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
            source,
            extensions: Default::default(),
        };
        state.sources.audiomuse.connections.insert(
            crate::app::sources::audiomuse::key(&session),
            crate::audiomuse::Connection {
                url: "http://127.0.0.1:1".into(),
                username: "fixture".into(),
                server: "fixture".into(),
            },
        );
        state.sources.active = crate::app::sources::ActiveSource::Navidrome(Box::new(session));
        let track = Track {
            rating_key: "destination-track".into(),
            parent_rating_key: Some("destination-album".into()),
            ..Default::default()
        };
        let mut tracks = BrowseColumn::new_with_tracks("Destination", vec![], vec![track]);
        tracks.play_all_row = Some(PlayAllRow::Album {
            rating_key: "destination-album".into(),
            title: "Destination".into(),
        });
        state.artist_nav.columns = vec![tracks];
        let (seed, albums) = similar_seed(&state).expect("destination seed");
        assert_eq!(seed.rating_key, "destination-track");
        assert!(albums);
        assert_eq!(preview(&state).unwrap().title, "Destination");
    }
    #[test]
    fn album_preview_offers_playback_without_redundant_details_button() {
        let mut state = AppState::new();
        state.artist_nav.columns = vec![BrowseColumn::new(
            "albums",
            vec![BrowseItem::Album {
                key: "album".into(),
                title: "Album".into(),
                artist: "Artist".into(),
                year: Some(2026),
                thumb: None,
                is_placeholder: false,
            }],
        )];
        assert!(preview(&state).is_none());
        let mut column = BrowseColumn::new_with_tracks("Album", vec![], vec![]);
        column.play_all_row = Some(PlayAllRow::Album {
            rating_key: "album".into(),
            title: "Album".into(),
        });
        column.header_selection = ColumnHeaderSelection::AlbumDetails;
        state.artist_nav.columns.push(column);
        state.artist_nav.focused_column = 1;
        let preview = preview(&state).unwrap();
        assert_eq!(preview.title, "Album");
        assert_eq!(
            preview
                .controls
                .iter()
                .map(|c| c.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Play Album", "Add Album to Queue", "Download Album"]
        );
    }
    #[test]
    fn stations_use_queue_controls_and_shared_radio_actions() {
        let mut state = AppState::new();
        state.station_nav.columns.push(StationColumn::new(
            None,
            "Radio".into(),
            vec![crate::library::station::Station {
                key: "library".into(),
                title: "Library Mix".into(),
                station_type: "station".into(),
                identifier: Some("library".into()),
                thumb: None,
                art: None,
                description: None,
            }],
        ));
        let rows = sidebar(&state);
        let station = rows.iter().find(|row| row.label == "Library Mix").unwrap();
        assert!(matches!(station.actions.as_slice(),
            [crate::app::Action::Navigation(crate::app::action::NavigationAction::OpenRadioStation(key))] if key == "library"));
        assert!(
            super::super::radio::project(&state).columns[0]
                .stations
                .iter()
                .any(|station| station.key == "library")
        );
    }
    #[test]
    fn mac_sidebar_places_artists_before_albums_and_groups_compilations() {
        let state = AppState::new();
        let rows = sidebar(&state);
        let albums = rows
            .iter()
            .position(|r| r.header && r.label == "Albums")
            .unwrap();
        assert_eq!(rows[albums + 1].label, "All");
        assert_eq!(rows[albums + 1].library_root, Some(LibraryRoot::Albums));
        let peers = rows
            .iter()
            .position(|r| r.header && r.label == "Library")
            .unwrap();
        assert!(peers > albums);
        assert!(
            rows[..albums]
                .iter()
                .any(|r| r.library_root == Some(LibraryRoot::Artists))
        );
        assert!(
            rows[albums + 1..peers]
                .iter()
                .any(|r| r.library_root == Some(LibraryRoot::Compilations))
        );
        let compilations = rows
            .iter()
            .position(|r| r.library_root == Some(LibraryRoot::Compilations))
            .unwrap();
        assert_eq!(
            rows[compilations + 1].library_root,
            Some(LibraryRoot::Singles)
        );
        assert!(rows[peers + 1..].iter().any(|r| r.label == "Tracks"));
    }

    #[test]
    fn library_is_a_heading_and_visible_destinations_follow_settings() {
        let mut state = AppState::new();
        state.desktop_hidden_library_roots = vec![LibraryRoot::Albums];
        let rows = sidebar(&state);
        assert!(rows.iter().any(|r| r.header && r.label == "Library"));
        assert!(!rows.iter().any(|r| !r.header && r.label == "Library"));
        assert!(
            rows.iter()
                .any(|r| r.library_root == Some(LibraryRoot::Artists))
        );
        assert!(
            !rows
                .iter()
                .any(|r| r.library_root == Some(LibraryRoot::Albums))
        );
        assert!(
            rows.iter()
                .any(|r| r.library_root == Some(LibraryRoot::Tracks))
        );
        assert!(
            state
                .category_rows()
                .contains(&CategoryRow::Root(LibraryRoot::Artists))
        );
    }
    #[test]
    fn option_click_selects_next_while_following_remains_a_menu_action() {
        let tracks: Vec<_> = ["first", "second"]
            .into_iter()
            .map(|key| Track {
                rating_key: key.into(),
                ..Default::default()
            })
            .collect();
        let controls = compact_controls(crate::app::playback_controls::track_controls(
            &tracks[0],
            Some(&tracks),
        ));
        assert_eq!(controls.len(), 2);
        assert_eq!(controls[1].label, "Add to Queue");
        let play = &controls[0];
        assert!(matches!(
            play.actions_for(false, false).as_slice(),
            [crate::app::Action::Queue(QueueAction::PlayTrack(_))]
        ));
        assert!(
            matches!(play.actions_for(true,false).as_slice(),[crate::app::Action::Queue(QueueAction::EnqueueTracksNext(rows))] if rows.len()==1)
        );
        assert!(
            matches!(play.actions_for(true,true).as_slice(),[crate::app::Action::Queue(QueueAction::EnqueueTracksNext(rows))] if rows.len()==1)
        );
        assert!(matches!(
            play.actions_for(false, true).as_slice(),
            [crate::app::Action::Queue(QueueAction::PlayTrack(_))]
        ));
        let album = compact_controls(crate::app::playback_controls::album_controls(
            "album", "Album",
        ));
        assert!(matches!(
            album[0].actions_for(true, false).as_slice(),
            [crate::app::Action::Queue(
                QueueAction::EnqueueAlbumNext { .. }
            )]
        ));
        assert!(matches!(
            album[0].actions_for(true, true).as_slice(),
            [crate::app::Action::Queue(
                QueueAction::EnqueueAlbumNext { .. }
            )]
        ));
        assert!(matches!(
            album[0].actions_for(false, true).as_slice(),
            [crate::app::Action::Queue(QueueAction::PlayAlbumNow { .. })]
        ));
    }
}
