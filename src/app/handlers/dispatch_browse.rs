//! Browse dispatch handlers: LoadStations, LoadTagList, LoadTagAlbums,
//! RefreshTagView, OpenTrackDetails, OpenInLibrary.

use crate::app::action::{BrowseAction, MillerAction, SystemAction};
use crate::app::state::{BrowseCategory, BrowseColumn, BrowseItem, StationColumn};
use crate::app::{Action, AppState, Event};

use anyhow::Result;

use tokio::sync::mpsc;

/// Dispatch browse-related actions. Returns follow-up actions.
pub fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: BrowseAction,
    state: &mut AppState,
) -> Result<Vec<Action>> {
    let mut follow_ups = vec![];

    match action {
        BrowseAction::SetPlaylistEditing { playlist, editing } => {
            crate::app::sources::navidrome::playlist_edit::set_editing(state, &playlist, editing);
        }
        BrowseAction::OpenSingles => crate::app::album_collections::open_singles(state),
        BrowseAction::LoadTagList(BrowseCategory::Decades) => {
            follow_ups.push(
                BrowseAction::TagListLoaded {
                    library_key: state.active_library.clone().unwrap_or_default(),
                    section: BrowseCategory::Decades,
                    result: Ok(crate::services::release_dates::decades(
                        &state.library.albums,
                    )),
                }
                .into(),
            );
        }
        BrowseAction::LoadTagAlbums { replace_child }
            if state.browse_category == BrowseCategory::Decades =>
        {
            if let Some(BrowseItem::Genre { key, title }) = state
                .tag_nav
                .columns
                .first()
                .and_then(|column| column.selected_item())
                .cloned()
            {
                let albums = state
                    .library
                    .albums
                    .iter()
                    .filter(|album| crate::services::release_dates::matches_decade(album, &key))
                    .cloned()
                    .collect();
                follow_ups.push(
                    BrowseAction::TagAlbumsLoaded {
                        library_key: state.active_library.clone().unwrap_or_default(),
                        section: BrowseCategory::Decades,
                        tag_key: key,
                        tag_title: title,
                        replace_child,
                        result: Ok(albums),
                    }
                    .into(),
                );
            }
        }
        BrowseAction::StationsLoaded {
            library_key,
            result,
        } => {
            if state.active_library.as_ref() != Some(&library_key) {
                return Ok(follow_ups);
            }
            state.station_nav.loading = false;
            match result {
                Ok(stations) => {
                    state.station_nav.columns.clear();
                    state.station_nav.columns.push(StationColumn::new(
                        None,
                        "stations".to_string(),
                        stations,
                    ));
                    state.station_nav.focused_column = 0;
                }
                Err(error) => {
                    state.set_error(error.message);
                }
            }
        }

        BrowseAction::TagListLoaded {
            library_key,
            section,
            result,
        } => {
            if state.active_library.as_ref() != Some(&library_key) {
                return Ok(follow_ups);
            }
            set_tag_loading(state, section, false);
            match result {
                Ok(items) => {
                    store_tag_list(state, section, items);
                    if state.browse_category == section
                        && (state.tag_nav.loading || !state.tag_nav.columns.is_empty())
                    {
                        follow_ups.push(BrowseAction::RefreshTagView.into());
                    } else {
                        state.tag_nav.loading = false;
                    }
                }
                Err(error) => {
                    state.set_error(error.message);
                    state.tag_nav.loading = false;
                }
            }
        }

        BrowseAction::TagAlbumsLoaded {
            library_key,
            section,
            tag_key,
            tag_title,
            replace_child,
            result,
        } => {
            let selected_tag_matches = state
                .tag_nav
                .columns
                .first()
                .and_then(|column| column.items.get(column.selected_index))
                .is_some_and(
                    |item| matches!(item, BrowseItem::Genre { key, .. } if key == &tag_key),
                );
            if state.active_library.as_ref() != Some(&library_key)
                || state.browse_category != section
                || !selected_tag_matches
            {
                return Ok(follow_ups);
            }
            match result {
                Ok(mut albums) => {
                    albums.sort_by_cached_key(|album| {
                        album.parent_title.as_deref().unwrap_or("").to_lowercase()
                    });
                    state.library.tag_albums = albums.clone();
                    let items =
                        BrowseItem::from_albums(&albums, &state.library.album_display_artist);
                    let col = BrowseColumn::new(tag_title, items);
                    state.tag_nav.drill_column(col, replace_child);
                    state.miller_reveal_right = true;
                }
                Err(error) => {
                    state.set_error(error.message);
                }
            }
        }
        BrowseAction::RefreshTagView => {
            // Populate column 0 of tag_nav with the active section's tag list.
            let section = state.browse_category;
            if !section.is_tag_section() {
                return Ok(follow_ups);
            }
            if section == BrowseCategory::Decades {
                state.library.decades =
                    crate::services::release_dates::decades(&state.library.albums);
                state.tag_nav.reset(
                    section.name(),
                    BrowseItem::from_genres(&state.library.decades),
                );
                state.tag_nav.loading = false;
                return Ok(follow_ups);
            }
            let list_empty = state.tag_list_for(section).is_empty();
            if list_empty && !is_tag_loading(state, section) {
                follow_ups.push(BrowseAction::LoadTagList(section).into());
                state.tag_nav.loading = true;
                return Ok(follow_ups);
            }
            let items = BrowseItem::from_genres(state.tag_list_for(section));
            state.tag_nav.reset(section.name(), items);
            state.tag_nav.loading = false;
        }
        BrowseAction::OpenDetailsInLibrary { track, destination } => {
            return crate::app::library_details::request(event_tx, state, *track, destination);
        }
        BrowseAction::OpenTrackDetails => {
            if state.browse_category == BrowseCategory::Folders {
                let source = state.sources.active.folder();
                if let Some(column) = state
                    .folder_state
                    .as_mut()
                    .and_then(|nav| nav.focused_mut())
                    && let Some(item) = column
                        .items
                        .get(column.selected_index)
                        .filter(|item| item.is_track())
                {
                    let key = item.rating_key.as_ref().unwrap_or(&item.key);
                    if !column.tracks.contains_key(key) {
                        let mut track = state
                            .library
                            .all_tracks
                            .iter()
                            .find(|track| &track.rating_key == key)
                            .cloned()
                            .unwrap_or_else(|| {
                                source.map_or_else(
                                    || crate::library::models::Track {
                                        rating_key: key.clone(),
                                        ..Default::default()
                                    },
                                    |source| {
                                        crate::library::models::Track::from_folder(
                                            source, &item.key,
                                        )
                                    },
                                )
                            });
                        if track.title.is_empty() {
                            track.title.clone_from(&item.title);
                        }
                        track.duration = track.duration.or(item.duration_ms);
                        track.parent_rating_key = track
                            .parent_rating_key
                            .or_else(|| item.parent_rating_key.clone());
                        track.grandparent_rating_key = track
                            .grandparent_rating_key
                            .or_else(|| item.grandparent_rating_key.clone());
                        column.tracks.insert(key.clone(), track);
                    }
                }
            }
            if let Some(track) = state.focused_track()
                && let Some(thumb) = crate::app::details::track_cover(state, track)
                && !state.artwork.grid_cache.contains_key(&track.rating_key)
                && !state.artwork.grid_pending.contains(&track.rating_key)
            {
                follow_ups.push(
                    SystemAction::LoadAlbumArt(vec![(track.rating_key.clone(), thumb.to_owned())])
                        .into(),
                );
            }
            if let Some(column) = state.browse_nav_mut().and_then(|nav| nav.focused_mut()) {
                column.header_selection = crate::app::state::ColumnHeaderSelection::Items;
            }
            state.track_pane_open = true;
            state.miller_reveal_right = true;
            if !state.desktop_frontend {
                state.track_pane_focused = true;
                state.category_column_focused = false;
                state.tui_column_header = None;
                state.track_pane_index = 0;
            }
        }
        BrowseAction::OpenSimilarTrack {
            seed_key,
            track_key,
        } => {
            let child = crate::app::sonic_details::selected_track(state);
            let seed = child.or_else(|| state.focused_track());
            if seed.is_some_and(|track| track.rating_key == seed_key) {
                let track = state
                    .track_pane_similar
                    .get(&seed_key)
                    .and_then(|result| result.as_ref().ok())
                    .and_then(|tracks| tracks.iter().find(|track| track.rating_key == track_key))
                    .cloned();
                if let Some(track) = track {
                    return crate::app::library_details::request(
                        event_tx,
                        state,
                        track,
                        crate::app::library_details::Destination::Track,
                    );
                }
            }
        }
        BrowseAction::CloseTrackDetails => {
            state.sonic_details = None;
            state.track_pane_open = false;
        }
        BrowseAction::OpenInLibrary {
            artist_key,
            artist_name,
            album_key,
            album_title,
        } => {
            let inspect_artist = album_key.is_none();
            let catalog_album = album_key
                .as_ref()
                .and_then(|key| {
                    state
                        .library
                        .albums
                        .iter()
                        .chain(&state.library.selected_artist_albums)
                        .chain(&state.library.tag_albums)
                        .find(|album| &album.rating_key == key)
                })
                .cloned();
            let compilation = album_key.as_ref().is_some_and(|key| {
                state
                    .library
                    .compilations
                    .albums
                    .iter()
                    .any(|album| &album.rating_key == key)
            });
            let artist_key = if compilation {
                artist_key
            } else {
                catalog_album
                    .as_ref()
                    .and_then(|album| album.parent_rating_key.clone())
                    .unwrap_or(artist_key)
            };
            let artist_key = if state
                .library
                .artists
                .iter()
                .any(|artist| artist.rating_key == artist_key)
            {
                artist_key
            } else {
                let name = catalog_album
                    .as_ref()
                    .and_then(|album| album.parent_title.as_deref())
                    .unwrap_or(&artist_name);
                let mut matches = state
                    .library
                    .artists
                    .iter()
                    .filter(|artist| artist.title == name);
                match (matches.next(), matches.next()) {
                    (Some(artist), None) => artist.rating_key.clone(),
                    _ => artist_key,
                }
            };
            let artist_name = state
                .library
                .artists
                .iter()
                .chain(&state.library.track_artists)
                .find(|artist| artist.rating_key == artist_key)
                .map(|artist| artist.title.clone())
                .unwrap_or(artist_name);
            state.pending_library_details = None;
            state.tui_description_focused = false;
            state.tui_description_submit_focused = false;
            state.tui_transport_focus = None;
            state.tui_footer_focus = None;
            state.tui_column_header = None;
            crate::app::navigation_history::record(state);
            state.library_navigation =
                crate::app::navigation_history::LibraryNavigation::Columns(None);
            state.tui_sidebar_group = None;
            state.miller_scroll_manual = false;
            state.revealed_compilation_artist = (album_key.is_none()
                && !state.show_compilation_only_artists
                && state.library.compilations.artist_keys.contains(&artist_key))
            .then(|| artist_key.clone());
            state.popups.close_all();
            state.palette.close();
            state.sonic_details = None;
            state.track_pane_open = false;
            state.set_view(crate::app::state::View::Browse);
            state.set_browse_category(crate::app::state::BrowseCategory::Library, false);
            state.desktop_library_root = if compilation {
                crate::app::state::LibraryRoot::Compilations
            } else {
                crate::app::state::LibraryRoot::Artists
            };
            state.category_column_focused = false;
            state.list_filter.deactivate();
            state.scroll.browse = None;
            state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
            for slot in ["artist-browse", "album-browse"] {
                state.sources.nav_tasks.remove(slot);
            }
            state.search.pending_album_key = album_key.clone();
            if let Some(title) = catalog_album
                .as_ref()
                .map(|album| album.title.clone())
                .or(album_title)
            {
                state.library.selected_album_title = title;
            }
            state.library.selected_artist_name = artist_name;
            state
                .artist_nav
                .reset("artists", state.build_artist_root_items());
            let root_index = state.artist_nav.columns[0].items.iter().position(|item| {
                if compilation {
                    matches!(item, BrowseItem::Compilations)
                } else {
                    matches!(item, BrowseItem::Artist {key,..} if key == &artist_key)
                }
            });
            state.artist_nav.columns[0].selected_index = root_index.unwrap_or(0);
            if let Some(index) = state
                .library
                .artists
                .iter()
                .position(|artist| artist.rating_key == artist_key)
            {
                state.list_state.artists_index = index;
            }
            if let Some(album_key) = album_key.filter(|_| compilation || artist_key.is_empty()) {
                let albums = if compilation {
                    &state.library.compilations.albums
                } else {
                    &state.library.albums
                };
                let mut column = BrowseColumn::new(
                    if compilation {
                        "compilations"
                    } else {
                        "all albums"
                    },
                    BrowseItem::from_albums(albums, &state.library.album_display_artist),
                );
                column.selected_index = column
                    .items
                    .iter()
                    .position(
                        |item| matches!(item, BrowseItem::Album {key,..} if key == &album_key),
                    )
                    .unwrap_or(0);
                state.initialize_column_artwork(&mut column);
                state.artist_nav.drill_column(column, false);
                state.miller_reveal_right = true;
                state.search.pending_album_key = None;
                follow_ups.push(
                    MillerAction::LoadAlbumTracksForMiller {
                        album_key,
                        replace_child: false,
                    }
                    .into(),
                );
            } else {
                follow_ups.push(
                    MillerAction::LoadArtistAlbumsForMiller {
                        artist_key,
                        replace_child: false,
                    }
                    .into(),
                );
            }
            if inspect_artist {
                state.search.pending_track_key = None;
                crate::app::library_details::wait_for_columns(
                    state,
                    Default::default(),
                    crate::app::library_details::Destination::Artist,
                );
            }
        }
        _ => anyhow::bail!("Unsupported library operation reached shared handler"),
    }
    Ok(follow_ups)
}

fn set_tag_loading(state: &mut AppState, section: BrowseCategory, val: bool) {
    match section {
        BrowseCategory::AlbumGenres => state.library.album_genres_loading = val,
        BrowseCategory::ArtistGenres => state.library.artist_genres_loading = val,
        BrowseCategory::Moods => state.library.moods_loading = val,
        BrowseCategory::Styles => state.library.styles_loading = val,
        BrowseCategory::Decades => state.library.decades_loading = val,
        BrowseCategory::Years => state.library.years_loading = val,
        BrowseCategory::Collections => state.library.collections_loading = val,
        BrowseCategory::Countries => state.library.countries_loading = val,
        BrowseCategory::Labels => state.library.labels_loading = val,
        BrowseCategory::Formats => state.library.formats_loading = val,
        BrowseCategory::Studios => state.library.studios_loading = val,
        _ => {}
    }
}

fn is_tag_loading(state: &AppState, section: BrowseCategory) -> bool {
    match section {
        BrowseCategory::AlbumGenres => state.library.album_genres_loading,
        BrowseCategory::ArtistGenres => state.library.artist_genres_loading,
        BrowseCategory::Moods => state.library.moods_loading,
        BrowseCategory::Styles => state.library.styles_loading,
        BrowseCategory::Decades => state.library.decades_loading,
        BrowseCategory::Years => state.library.years_loading,
        BrowseCategory::Collections => state.library.collections_loading,
        BrowseCategory::Countries => state.library.countries_loading,
        BrowseCategory::Labels => state.library.labels_loading,
        BrowseCategory::Formats => state.library.formats_loading,
        BrowseCategory::Studios => state.library.studios_loading,
        _ => false,
    }
}

fn store_tag_list(
    state: &mut AppState,
    section: BrowseCategory,
    items: Vec<crate::library::models::Genre>,
) {
    match section {
        BrowseCategory::AlbumGenres => state.library.album_genres = items.into(),
        BrowseCategory::ArtistGenres => state.library.artist_genres = items,
        BrowseCategory::Moods => state.library.moods = items,
        BrowseCategory::Styles => state.library.styles = items,
        BrowseCategory::Decades => state.library.decades = items,
        BrowseCategory::Years => state.library.years = items,
        BrowseCategory::Collections => state.library.collections = items,
        BrowseCategory::Countries => state.library.countries = items,
        BrowseCategory::Labels => state.library.labels = items,
        BrowseCategory::Formats => state.library.formats = items,
        BrowseCategory::Studios => state.library.studios = items,
        _ => {}
    }
}
