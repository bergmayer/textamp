//! Library destinations for track, album and artist inspection.
use super::{Action, AppState, action::*, state::*};
use crate::library::models::Track;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Track,
    Album,
    Artist,
}
#[derive(Debug)]
pub struct Pending {
    generation: u64,
    jump: Option<u64>,
    root_key: Option<String>,
    root: LibraryRoot,
    track: Track,
    destination: Destination,
}

pub(crate) fn album_target(album: &crate::library::models::Album) -> Track {
    Track {
        parent_rating_key: Some(album.rating_key.clone()),
        parent_title: Some(album.title.clone()),
        grandparent_rating_key: album.parent_rating_key.clone(),
        grandparent_title: Some(album.artist_name().into()),
        ..Default::default()
    }
}

pub(super) fn selected_destination(state: &AppState) -> Option<(Track, Destination)> {
    if !state.palette.player_context && (state.popups.search_active() || state.view == View::Search)
    {
        return search_selection(state)
            .and_then(|(section, index)| search_destination(state, section, index));
    }
    if state.view == View::Browse
        && !state.category_column_focused
        && !state.palette.player_context
        && let Some(target) = state
            .browse_nav()
            .and_then(|nav| nav.selected_item())
            .and_then(browse_destination)
    {
        return Some(target);
    }
    state
        .palette_target_track()
        .or_else(|| state.current_track().cloned())
        .map(|track| (track, Destination::Track))
}

pub(crate) fn browse_destination(item: &BrowseItem) -> Option<(Track, Destination)> {
    match item {
        BrowseItem::Artist { key, title, .. } => Some((
            Track {
                grandparent_rating_key: Some(key.clone()),
                grandparent_title: Some(title.clone()),
                ..Default::default()
            },
            Destination::Artist,
        )),
        BrowseItem::Album {
            key, title, artist, ..
        } => Some((
            Track {
                parent_rating_key: Some(key.clone()),
                parent_title: Some(title.clone()),
                grandparent_title: Some(artist.clone()),
                ..Default::default()
            },
            Destination::Album,
        )),
        _ => None,
    }
}

fn search_selection(state: &AppState) -> Option<(SearchTab, usize)> {
    let results = state.search.results.as_ref()?;
    Some(if state.search.tab == SearchTab::Global {
        super::handlers::dispatch_search::resolve_global_index(
            results,
            state.list_state.search_item_index,
        )
    } else {
        (state.search.tab, state.list_state.search_item_index)
    })
}
fn search_destination(
    state: &AppState,
    section: SearchTab,
    index: usize,
) -> Option<(Track, Destination)> {
    let results = state.search.results.as_ref()?;
    match section {
        SearchTab::Tracks => results
            .tracks
            .get(index)
            .cloned()
            .map(|track| (track, Destination::Track)),
        SearchTab::Albums => results
            .albums
            .get(index)
            .map(|album| (album_target(album), Destination::Album)),
        SearchTab::Artists => results.artists.get(index).map(|artist| {
            (
                Track {
                    grandparent_rating_key: Some(artist.rating_key.clone()),
                    grandparent_title: Some(artist.title.clone()),
                    ..Default::default()
                },
                Destination::Artist,
            )
        }),
        _ => None,
    }
}
/// Build library navigation for the selected search result, retaining its detail type.
pub fn search_result_actions(state: &AppState) -> Vec<Action> {
    let Some((section, index)) = search_selection(state) else {
        return vec![];
    };
    if !matches!(
        section,
        SearchTab::Artists | SearchTab::Albums | SearchTab::Tracks
    ) {
        return vec![SearchAction::SelectSearchResult.into()];
    }
    search_destination(state, section, index)
        .map(|(track, destination)| {
            vec![
                BrowseAction::OpenDetailsInLibrary {
                    track: Box::new(track),
                    destination,
                }
                .into(),
            ]
        })
        .unwrap_or_default()
}

pub fn request(
    tx: &tokio::sync::mpsc::Sender<super::Event>,
    state: &mut AppState,
    track: Track,
    destination: Destination,
) -> anyhow::Result<Vec<Action>> {
    if let crate::library::track::TrackOrigin::Folder { source_id, path } = &track.origin {
        if destination != Destination::Track || state.sources.active.folder() != Some(source_id) {
            state.set_status("This item has no library detail destination".into());
            return Ok(vec![]);
        }
        let target = state.folder_state.as_ref().and_then(|nav| {
            nav.columns.iter().enumerate().find_map(|(column, data)| {
                data.items
                    .iter()
                    .position(|item| &item.key == path)
                    .map(|row| (column, row))
            })
        });
        let Some((column, row)) = target else {
            state.set_status("Track is no longer available in this folder".into());
            return Ok(vec![]);
        };
        super::navigation_history::record(state);
        state.library_navigation = super::navigation_history::LibraryNavigation::Columns(None);
        state.popups.set_search_active(false);

        state.set_view(View::Browse);
        state.browse_category = BrowseCategory::Folders;
        state.category_column_focused = false;
        let nav = state
            .folder_state
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Folder navigation is unavailable"))?;
        nav.focused_column = column;
        nav.columns[column].selected_index = row;
        return Ok(vec![BrowseAction::OpenTrackDetails.into()]);
    }
    state.pending_library_details = None;
    let album = state
        .library
        .albums
        .iter()
        .find(|album| track.parent_rating_key.as_ref() == Some(&album.rating_key));
    let artist_key = if destination == Destination::Artist {
        state
            .artist_destination_for_track(&track)
            .map(|(key, _)| key)
    } else {
        album.and_then(|album| album.parent_rating_key.clone())
    }
    .or_else(|| track.grandparent_rating_key.clone())
    .or_else(|| {
        state
            .library
            .artists
            .iter()
            .find(|artist| artist.title == track.artist_name())
            .map(|artist| artist.rating_key.clone())
    });
    let artist_key = match artist_key {
        Some(key) => key,
        None if destination == Destination::Album && album.is_some() => String::new(),
        None => {
            state.set_status("This track has no library artist".into());
            return Ok(vec![]);
        }
    };
    if destination != Destination::Artist && track.parent_rating_key.is_none() {
        state.set_status("This track has no library album".into());
        return Ok(vec![]);
    }
    let actions = super::handlers::dispatch_browse::dispatch(
        tx,
        BrowseAction::OpenInLibrary {
            artist_key,
            artist_name: if destination == Destination::Artist {
                track.track_artist().to_owned()
            } else {
                track.artist_name().to_owned()
            },
            album_key: (destination != Destination::Artist)
                .then(|| track.parent_rating_key.clone())
                .flatten(),
            album_title: track.parent_title.clone(),
        },
        state,
    )?;
    state.search.pending_track_key = None;
    wait_for_columns(state, track, destination);
    Ok(actions)
}

pub(crate) fn wait_for_columns(state: &mut AppState, track: Track, destination: Destination) {
    state.pending_library_details = Some(Pending {
        generation: state.library_generation,
        jump: state.jump_history.head(),
        root: state.desktop_library_root,
        root_key: state
            .artist_nav
            .columns
            .first()
            .and_then(|column| column.selected_item())
            .map(|item| item.key().to_owned()),
        track,
        destination,
    });
}

pub fn finish(state: &mut AppState) -> Vec<Action> {
    let Some(pending) = &state.pending_library_details else {
        return vec![];
    };
    let root_key = state
        .artist_nav
        .columns
        .first()
        .and_then(|column| column.selected_item())
        .map(BrowseItem::key);
    if pending.generation != state.library_generation
        || pending.jump != state.jump_history.head()
        || state.desktop_library_root != pending.root
        || state.view != View::Browse
        || state.browse_category != BrowseCategory::Library
        || root_key != pending.root_key.as_deref()
    {
        state.pending_library_details = None;
        return vec![];
    }
    if state.artist_nav.loading {
        return vec![];
    }
    let nav = &mut state.artist_nav;
    let action = match pending.destination {
        Destination::Artist => {
            let Some((column, row, key, name)) =
                nav.columns.iter().enumerate().find_map(|(column, data)| {
                    data.items
                        .iter()
                        .enumerate()
                        .find_map(|(row, item)| match item {
                            BrowseItem::ArtistInfo {
                                artist_key,
                                artist_name,
                                ..
                            } => Some((column, row, artist_key.clone(), artist_name.clone())),
                            _ => None,
                        })
                })
            else {
                return vec![];
            };
            nav.focused_column = column;
            nav.columns.truncate(column + 1);
            nav.columns[column].selected_index = row;
            nav.columns[column].header_selection = ColumnHeaderSelection::Items;
            SearchAction::ShowArtistInfo {
                artist_key: key,
                artist_name: name,
            }
            .into()
        }
        Destination::Album | Destination::Track => {
            let Some(column) = nav.columns.iter().position(|column| matches!(&column.play_all_row, Some(PlayAllRow::Album { rating_key, .. }) if Some(rating_key) == pending.track.parent_rating_key.as_ref())) else { return vec![]; };
            nav.focused_column = column;
            nav.columns.truncate(column + 1);
            if pending.destination == Destination::Album {
                nav.columns[column].header_selection = ColumnHeaderSelection::AlbumDetails;
                MillerAction::OpenAlbumDetails.into()
            } else {
                let Some(row) = nav.columns[column]
                    .tracks
                    .iter()
                    .position(|track| track.rating_key == pending.track.rating_key)
                else {
                    state.pending_library_details = None;
                    state.set_status("Track is no longer available in this album".into());
                    return vec![];
                };
                nav.columns[column].selected_index = row;
                nav.columns[column].header_selection = ColumnHeaderSelection::Items;
                BrowseAction::OpenTrackDetails.into()
            }
        }
    };
    state.pending_library_details = None;
    vec![action]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::models::{Album, Artist};
    #[test]
    fn search_results_use_library_detail_destinations_for_each_type() {
        let (mut state, track) = fixture();
        state.search.results = Some(crate::library::models::SearchResults {
            tracks: vec![track],
            albums: state.library.albums.as_ref().clone(),
            artists: state.library.artists.as_ref().clone(),
            ..Default::default()
        });
        for (tab, expected) in [
            (SearchTab::Tracks, Destination::Track),
            (SearchTab::Albums, Destination::Album),
            (SearchTab::Artists, Destination::Artist),
        ] {
            state.search.tab = tab;
            let actions = search_result_actions(&state);
            assert!(
                matches!(actions.as_slice(), [Action::Browse(BrowseAction::OpenDetailsInLibrary {destination,..})] if *destination == expected)
            );
        }
    }
    fn fixture() -> (AppState, Track) {
        let mut state = AppState::new();
        state.library.artists = std::sync::Arc::new(vec![Artist {
            rating_key: "artist".into(),
            title: "Artist".into(),
            ..Default::default()
        }]);
        state.library.albums = std::sync::Arc::new(vec![Album {
            rating_key: "album".into(),
            title: "Album".into(),
            parent_rating_key: Some("artist".into()),
            ..Default::default()
        }]);
        let track = Track {
            rating_key: "target".into(),
            title: "Target".into(),
            grandparent_rating_key: Some("artist".into()),
            grandparent_title: Some("Artist".into()),
            parent_rating_key: Some("album".into()),
            parent_title: Some("Album".into()),
            ..Default::default()
        };
        (state, track)
    }
    #[test]
    fn detail_destinations_keep_the_natural_parent_selection() {
        for destination in [Destination::Track, Destination::Album, Destination::Artist] {
            let (mut state, track) = fixture();
            let (tx, _) = tokio::sync::mpsc::channel(32);
            request(&tx, &mut state, track.clone(), destination).unwrap();
            let mut audio = crate::audio::AudioPlayer::new_without_audio();
            let albums = state.library.albums.as_ref().clone();
            super::super::handlers::dispatch_miller::dispatch(
                &tx,
                MillerAction::ArtistAlbumsForMillerLoaded {
                    request_id: state.artist_nav_request_id,
                    artist_key: "artist".into(),
                    replace_child: false,
                    is_catalog_artist: true,
                    result: Ok(albums),
                },
                &mut state,
                &mut audio,
            )
            .unwrap();
            if destination != Destination::Artist {
                assert!(finish(&mut state).is_empty());
                super::super::handlers::dispatch_miller::dispatch(
                    &tx,
                    MillerAction::AlbumTracksForMillerLoaded {
                        request_id: state.artist_nav_request_id,
                        album_key: "album".into(),
                        album_title: "Album".into(),
                        replace_child: false,
                        result: Ok(vec![
                            Track {
                                rating_key: "first".into(),
                                ..track.clone()
                            },
                            track,
                        ]),
                    },
                    &mut state,
                    &mut audio,
                )
                .unwrap();
            }
            let actions = finish(&mut state);
            assert_eq!(
                state.artist_nav.columns[0].selected_item().unwrap().key(),
                "artist"
            );
            match destination {
                Destination::Artist => assert!(
                    matches!(actions.as_slice(), [Action::Search(SearchAction::ShowArtistInfo { artist_key, .. })] if artist_key == "artist")
                ),
                Destination::Album => assert!(matches!(
                    actions.as_slice(),
                    [Action::Miller(MillerAction::OpenAlbumDetails)]
                )),
                Destination::Track => {
                    assert!(matches!(
                        actions.as_slice(),
                        [Action::Browse(BrowseAction::OpenTrackDetails)]
                    ));
                    assert_eq!(state.focused_track().unwrap().rating_key, "target");
                }
            }
            if destination != Destination::Artist {
                assert_eq!(
                    state.artist_nav.columns[1].selected_item().unwrap().key(),
                    "album"
                );
            }
            assert!(state.pending_library_details.is_none());
        }
    }
    #[test]
    fn artist_name_and_palette_jumps_open_details_with_selected_ancestors() {
        for from_palette in [false, true] {
            let (mut state, track) = fixture();
            state.view = View::NowPlaying;
            let (tx, _) = tokio::sync::mpsc::channel(32);
            let action = if from_palette {
                super::super::command_palette::run(
                    PaletteCommandKind::JumpToArtist(Box::new(track)),
                    &mut state,
                )
                .pop()
                .unwrap()
            } else {
                BrowseAction::OpenInLibrary {
                    artist_key: "artist".into(),
                    artist_name: "Artist".into(),
                    album_key: None,
                    album_title: None,
                }
                .into()
            };
            let Action::Browse(action) = action else {
                panic!("artist library jump expected");
            };
            super::super::handlers::dispatch_browse::dispatch(&tx, action, &mut state).unwrap();
            assert!(state.pending_library_details.is_some());
            let albums = state.library.albums.as_ref().clone();
            let mut audio = crate::audio::AudioPlayer::new_without_audio();
            super::super::handlers::dispatch_miller::dispatch(
                &tx,
                MillerAction::ArtistAlbumsForMillerLoaded {
                    request_id: state.artist_nav_request_id,
                    artist_key: "artist".into(),
                    replace_child: false,
                    is_catalog_artist: true,
                    result: Ok(albums),
                },
                &mut state,
                &mut audio,
            )
            .unwrap();
            let actions = finish(&mut state);
            assert!(
                matches!(actions.as_slice(), [Action::Search(SearchAction::ShowArtistInfo {artist_key,..})] if artist_key == "artist")
            );
            assert_eq!(
                state.artist_nav.columns[0].selected_item().unwrap().key(),
                "artist"
            );
            assert!(matches!(
                state.artist_nav.columns[1].selected_item(),
                Some(BrowseItem::ArtistInfo { .. })
            ));
            assert_eq!(state.view, View::Browse);
            assert_eq!(audio.playback_id(), 0);
            assert!(state.queue.tracks.is_empty());
        }
    }

    #[test]
    fn now_playing_menu_offers_details_without_analysis_and_omits_restart_commands() {
        use crate::app::track_context::{ContextKind, ContextPolicy, now_playing_context};
        let (_, track) = fixture();
        let menu = now_playing_context(ContextPolicy::default(), &track);
        assert_eq!(
            menu.iter()
                .filter(|entry| matches!(entry.kind, ContextKind::OpenInLibrary))
                .count(),
            1
        );
        assert!(
            !menu
                .iter()
                .any(|entry| matches!(entry.kind, ContextKind::ShowDetails(_)))
        );
        assert!(!menu.iter().any(|entry| matches!(
            entry.kind,
            ContextKind::PlayTrack
                | ContextKind::PlayTrackAndFollowing
                | ContextKind::ArtistBio { .. }
        )));
    }
    #[test]
    fn obsolete_detail_jumps_cannot_change_another_destination() {
        let (mut state, track) = fixture();
        let (tx, _) = tokio::sync::mpsc::channel(8);
        request(&tx, &mut state, track, Destination::Track).unwrap();
        state.library_generation += 1;
        assert!(finish(&mut state).is_empty());
        assert!(state.pending_library_details.is_none());
        let (mut state, track) = fixture();
        request(&tx, &mut state, track, Destination::Artist).unwrap();
        state.desktop_library_root = LibraryRoot::Radio;
        assert!(finish(&mut state).is_empty());
        assert!(state.pending_library_details.is_none());
    }
}

#[cfg(test)]
mod compilation_visibility_tests {
    use super::*;
    use crate::library::models::Artist;
    #[test]
    fn hidden_compilation_artist_is_revealed_selected_and_ghosted_by_a_name_jump() {
        let mut state = AppState::new();
        state.library.artists = std::sync::Arc::new(vec![Artist {
            rating_key: "owner".into(),
            title: "Various Artists".into(),
            ..Default::default()
        }]);
        state.library.track_artists = vec![Artist {
            rating_key: "guest".into(),
            title: "Guest".into(),
            ..Default::default()
        }];
        state
            .library
            .compilations
            .artist_keys
            .insert("guest".into());
        assert!(
            !state
                .build_artist_root_items()
                .iter()
                .any(|item| item.key() == "guest")
        );
        let track = Track {
            grandparent_rating_key: Some("owner".into()),
            grandparent_title: Some("Various Artists".into()),
            original_title: Some("Guest".into()),
            ..Default::default()
        };
        assert_eq!(
            state.artist_destination_for_track(&track).unwrap().0,
            "guest"
        );
        let (tx, _) = tokio::sync::mpsc::channel(8);
        let action = crate::app::command_palette::run(
            PaletteCommandKind::JumpToArtist(Box::new(track)),
            &mut state,
        )
        .pop()
        .unwrap();
        let Action::Browse(action) = action else {
            panic!("Library navigation expected");
        };
        crate::app::handlers::dispatch_browse::dispatch(&tx, action, &mut state).unwrap();
        assert_eq!(
            state.artist_nav.columns[0].selected_item().unwrap().key(),
            "guest"
        );
        assert!(state.is_ghost_artist("guest"));
        state.set_compilation_artists_visible(true);
        assert!(!state.is_ghost_artist("guest"));
        assert_eq!(
            state.artist_nav.columns[0].selected_item().unwrap().key(),
            "guest"
        );
        state.set_compilation_artists_visible(false);
        assert!(state.is_ghost_artist("guest"));
        let mut popup = SortPopupState::new(
            0,
            "Artists".into(),
            SortColumnType::Artist,
            ColumnSortMode::Default,
            false,
            false,
        );
        popup.rebuild_options(ColumnSortMode::ByTitle);
        assert!(
            popup
                .options
                .contains(&SortPopupOption::CompilationOnlyArtists)
        );
    }
}
