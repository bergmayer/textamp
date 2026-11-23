use super::*;
use crate::app::state::{AllTracksScope, BrowseColumn, BrowseItem};

pub fn album_tracks(state: &AppState, key: &str) -> Vec<Track> {
    let key = state
        .library
        .compilations
        .complete_releases
        .get(key)
        .map_or(key, String::as_str);
    if let Some(members) = state.library.compilations.release_members.get(key) {
        let mut tracks: Vec<_> = members
            .iter()
            .flat_map(|member| indexed_album_tracks(state, member))
            .collect();
        tracks.sort_by_key(|track| {
            (
                track.parent_index.unwrap_or(1),
                track.index.unwrap_or(u32::MAX),
                track.title.clone(),
                track.rating_key.clone(),
            )
        });
        let mut seen = std::collections::HashSet::new();
        tracks.retain(|track| seen.insert(track.rating_key.clone()));
        return tracks;
    }
    indexed_album_tracks(state, key)
}
fn indexed_album_tracks(state: &AppState, key: &str) -> Vec<Track> {
    if let Some(tracks) = state
        .library
        .visited_album_tracks
        .get(key)
        .filter(|tracks| !tracks.is_empty())
    {
        return tracks.clone();
    }
    if matches!(
        state.library.catalog_loading,
        crate::app::catalog_loading::State::Browsing { .. }
    ) {
        return vec![];
    }
    if state.library.track_lookup.len == state.library.all_tracks.len()
        && let Some(rows) = state.library.track_lookup.albums.get(key)
    {
        return rows
            .iter()
            .filter_map(|row| state.library.all_tracks.get(*row).cloned())
            .collect();
    }
    state
        .library
        .all_tracks
        .iter()
        .filter(|t| t.parent_rating_key.as_deref() == Some(key))
        .cloned()
        .collect()
}
pub fn artist_tracks(state: &AppState, key: &str) -> Vec<Track> {
    artist_track_refs(state, key).into_iter().cloned().collect()
}
fn artist_track_refs<'a>(state: &'a AppState, key: &str) -> Vec<&'a Track> {
    if state.library.track_lookup.len == state.library.all_tracks.len()
        && let Some(rows) = state.library.track_lookup.artists.get(key)
    {
        return rows
            .iter()
            .filter_map(|row| state.library.all_tracks.get(*row))
            .collect();
    }
    let name = state
        .library
        .artists
        .iter()
        .chain(&state.library.track_artists)
        .find(|a| a.rating_key == key)
        .map(|a| crate::services::artist_alias_service::normalize_artist_name(&a.title));
    let albums: std::collections::HashSet<_> = state
        .library
        .albums
        .iter()
        .filter(|a| a.parent_rating_key.as_deref() == Some(key))
        .map(|a| a.rating_key.as_str())
        .collect();
    state
        .library
        .all_tracks
        .iter()
        .filter(|t| {
            t.grandparent_rating_key.as_deref() == Some(key)
                || t.parent_rating_key
                    .as_deref()
                    .is_some_and(|k| albums.contains(k))
                || name.as_ref().is_some_and(|n| {
                    *n == crate::services::artist_alias_service::normalize_artist_name(
                        t.track_artist(),
                    )
                })
        })
        .collect()
}
fn album_name(state: &AppState, key: &str) -> String {
    state
        .library
        .albums
        .iter()
        .find(|a| a.rating_key == key)
        .map(|a| a.title.clone())
        .unwrap_or_default()
}

pub fn intercept(
    action: &Action,
    state: &mut AppState,
    tx: &mpsc::Sender<Event>,
    session: &Session,
) -> Option<Vec<Action>> {
    if crate::app::catalog_loading::defer(state, action) {
        return Some(vec![]);
    }
    let mut follow = vec![];
    match action.clone() {
        Action::Miller(MillerAction::LoadArtistAlbumsForMiller {
            artist_key,
            replace_child,
        }) => {
            state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
            state.library.selected_artist_name = state
                .library
                .artists
                .iter()
                .chain(&state.library.track_artists)
                .find(|a| a.rating_key == artist_key)
                .map(|a| a.title.clone())
                .unwrap_or_default();
            let tracks = artist_track_refs(state, &artist_key);
            let keys: std::collections::HashSet<_> = tracks
                .iter()
                .filter_map(|t| t.parent_rating_key.as_deref())
                .collect();
            let compilations = &state.library.compilations;
            let separate: std::collections::HashSet<_> = compilations
                .albums
                .iter()
                .chain(
                    compilations
                        .single_artist
                        .iter()
                        .filter(|(owner, _)| *owner != &artist_key)
                        .flat_map(|(_, albums)| albums),
                )
                .map(|a| a.rating_key.as_str())
                .collect();
            let albums: Vec<_> = state
                .library
                .albums
                .iter()
                .filter(|a| {
                    !separate.contains(a.rating_key.as_str())
                        && (a.parent_rating_key.as_deref() == Some(&artist_key)
                            || keys.contains(a.rating_key.as_str()))
                })
                .cloned()
                .collect();
            let request_id = state.artist_nav_request_id;
            let catalog_artist = !tracks.is_empty()
                || compilations.artist_map.contains_key(&artist_key)
                || compilations.single_artist.contains_key(&artist_key);
            let fetch_key = artist_key.clone();
            let completion = move |result| {
                MillerAction::ArtistAlbumsForMillerLoaded {
                    request_id,
                    artist_key,
                    replace_child,
                    is_catalog_artist: false,
                    result,
                }
                .into()
            };
            if albums.is_empty() && !catalog_artist {
                let session = session.clone();
                spawn(state, tx, "artist-browse", async move {
                    let result = async {
                        let artist = session.client.artist(&session.id(&fetch_key)?).await?;
                        Ok(artist
                            .album
                            .into_iter()
                            .map(|album| session.album(album))
                            .collect())
                    }
                    .await
                    .map_err(failure);
                    Ok(vec![completion(result)])
                });
            } else {
                follow.push(completion(Ok(albums)));
            }
        }
        Action::Miller(MillerAction::LoadAlbumTracksForMiller {
            album_key,
            replace_child,
        }) => {
            state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
            let request_id = state.artist_nav_request_id;
            let tracks = album_tracks(state, &album_key);
            let album_title = album_name(state, &album_key);
            let fetch_key = state
                .library
                .compilations
                .complete_releases
                .get(&album_key)
                .cloned()
                .unwrap_or_else(|| album_key.clone());
            let completion = move |result: Result<Vec<Track>, crate::app::action::AsyncError>| {
                let album_title = if album_title.is_empty() {
                    result
                        .as_ref()
                        .ok()
                        .and_then(|tracks| tracks.first())
                        .map(|track| track.album_name().to_owned())
                        .unwrap_or_default()
                } else {
                    album_title
                };
                MillerAction::AlbumTracksForMillerLoaded {
                    request_id,
                    album_title,
                    result,
                    album_key,
                    replace_child,
                }
                .into()
            };
            if tracks.is_empty() {
                let session = session.clone();
                // Account-wide playlists can reference albums outside the selected folder.
                spawn(state, tx, "album-browse", async move {
                    Ok(vec![completion(
                        session.album_tracks(&fetch_key).await.map_err(failure),
                    )])
                });
            } else {
                follow.push(completion(Ok(tracks)));
            }
        }
        Action::Miller(MillerAction::LoadArtistAllTracksForMiller {
            artist_key,
            replace_child,
        }) => {
            state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
            let request_id = state.artist_nav_request_id;
            let tracks = artist_tracks(state, &artist_key);
            let completion = move |result| {
                MillerAction::ArtistAllTracksForMillerLoaded {
                    request_id,
                    replace_child,
                    result,
                }
                .into()
            };
            if tracks.is_empty() {
                let session = session.clone();
                spawn(state, tx, "artist-browse", async move {
                    Ok(vec![completion(
                        session.artist_tracks(&artist_key).await.map_err(failure),
                    )])
                });
            } else {
                follow.push(completion(Ok(tracks)));
            }
        }
        Action::Miller(MillerAction::LoadAllAlbumsForMiller { replace_child }) => {
            let mut items =
                BrowseItem::from_albums(&state.library.albums, &state.library.album_display_artist);
            items.push(BrowseItem::AllTracks {
                scope: AllTracksScope::Library,
                thumb: None,
            });
            let mut col = BrowseColumn::new("all albums", items);
            state.initialize_column_artwork(&mut col);
            state.artist_nav.drill_column(col, replace_child);
            state.miller_reveal_right = true;
        }
        Action::Miller(MillerAction::LoadAllLibraryTracksForMiller { replace_child }) => {
            let tracks = state.library.all_tracks.as_ref().clone();
            let col = BrowseColumn::new_with_tracks(
                "all tracks",
                BrowseItem::from_tracks(&tracks),
                tracks,
            );
            state.artist_nav.drill_column(col, replace_child);
            state.miller_reveal_right = true;
        }
        Action::Miller(MillerAction::LoadGenreTracksForMiller {
            album_key,
            replace_child,
        }) => {
            state.tag_nav_request_id = state.tag_nav_request_id.wrapping_add(1);
            follow.push(
                MillerAction::GenreTracksForMillerLoaded {
                    request_id: state.tag_nav_request_id,
                    album_name: album_name(state, &album_key),
                    result: Ok(album_tracks(state, &album_key)),
                    album_key,
                    replace_child,
                }
                .into(),
            );
        }
        Action::Miller(MillerAction::LoadGenreAlbumsForMiller {
            genre_key,
            replace_child,
        }) => {
            state.tag_nav_request_id = state.tag_nav_request_id.wrapping_add(1);
            let albums = state
                .library
                .albums
                .iter()
                .filter(|a| a.genre.iter().any(|g| g.tag == genre_key))
                .cloned()
                .collect();
            follow.push(
                MillerAction::GenreAlbumsForMillerLoaded {
                    request_id: state.tag_nav_request_id,
                    genre_name: genre_key,
                    replace_child,
                    result: Ok(albums),
                }
                .into(),
            );
        }
        Action::Data(DataAction::LoadArtistAlbums) => {
            if let Some(artist) = state.library.artists.get(state.list_state.artists_index) {
                follow.push(
                    MillerAction::LoadArtistAlbumsForMiller {
                        artist_key: artist.rating_key.clone(),
                        replace_child: false,
                    }
                    .into(),
                );
            }
        }
        Action::Data(DataAction::LoadArtistAllTracks) => {
            if let Some(artist) = state.library.artists.get(state.list_state.artists_index) {
                follow.push(
                    MillerAction::LoadArtistAllTracksForMiller {
                        artist_key: artist.rating_key.clone(),
                        replace_child: false,
                    }
                    .into(),
                );
            }
        }
        Action::Data(DataAction::LoadAlbumTracks { rating_key }) => follow.push(
            MillerAction::LoadAlbumTracksForMiller {
                album_key: rating_key,
                replace_child: false,
            }
            .into(),
        ),
        Action::Data(DataAction::LoadSelectedAlbumTracks) => {
            if let Some(album) = state
                .library
                .selected_artist_albums
                .get(state.list_state.right_albums_index.saturating_sub(1))
            {
                follow.push(
                    MillerAction::LoadAlbumTracksForMiller {
                        album_key: album.rating_key.clone(),
                        replace_child: false,
                    }
                    .into(),
                );
            }
        }
        Action::Data(DataAction::LoadCategoryTracks) => {
            if let Some(key) = state.selected_category_key() {
                follow.push(
                    match state.browse_category {
                        BrowseCategory::Playlists => MillerAction::LoadPlaylistTracksForMiller {
                            playlist_key: key,
                            replace_child: false,
                        },
                        BrowseCategory::Library => MillerAction::LoadArtistAllTracksForMiller {
                            artist_key: key,
                            replace_child: false,
                        },
                        _ => MillerAction::LoadGenreAlbumsForMiller {
                            genre_key: key,
                            replace_child: false,
                        },
                    }
                    .into(),
                );
            }
        }
        Action::Miller(MillerAction::LoadPlaylistTracksForMiller { playlist_key, .. }) => {
            state.playlist_nav_request_id = state.playlist_nav_request_id.wrapping_add(1);
            let request_id = state.playlist_nav_request_id;
            state.playlist_nav.loading = true;
            let library_key = state.active_library.clone().unwrap_or_default();
            let session = session.clone();
            let playlist = state
                .library
                .playlists
                .iter()
                .find(|playlist| playlist.rating_key == playlist_key)
                .cloned();
            spawn(state, tx, "playlist-browse", async move {
                let result =
                    super::super::playlists::tracks(&session, &playlist_key, playlist.as_ref())
                        .await;
                let completion = match result {
                    Ok(tracks) => PlaylistEvent::PlaylistFirstPageLoaded {
                        library_key,
                        request_id,
                        playlist_key,
                        total: Some(tracks.len() as u32),
                        tracks,
                    },
                    Err(error) => PlaylistEvent::PlaylistTracksForMillerFailed {
                        library_key,
                        request_id,
                        playlist_key,
                        error: failure(error),
                    },
                };
                Ok(vec![event(completion)])
            });
        }
        Action::Miller(MillerAction::LoadMorePlaylistTracks { .. }) => {}
        Action::Miller(MillerAction::RefreshAlbumTracks { album_key }) => {
            state.track_pane_similar.retain(|_, result| result.is_ok());
            let tag_section = state.browse_category.is_tag_section();
            let (request_id, column_index) = if tag_section {
                state.tag_nav_request_id = state.tag_nav_request_id.wrapping_add(1);
                (state.tag_nav_request_id, state.tag_nav.focused_column)
            } else {
                state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
                (state.artist_nav_request_id, state.artist_nav.focused_column)
            };
            let session = session.clone();
            spawn(state, tx, "album-browse", async move {
                Ok(vec![
                    MillerAction::AlbumTracksRefreshed {
                        request_id,
                        tag_section,
                        column_index,
                        result: session
                            .fetch_album_tracks(&album_key)
                            .await
                            .map_err(failure),
                    }
                    .into(),
                ])
            });
        }
        Action::Browse(BrowseAction::RefreshTagView) => {
            let section = state.browse_category;
            if !section.is_tag_section() {
                return Some(vec![]);
            }
            if section == BrowseCategory::Decades {
                state.library.decades =
                    crate::services::release_dates::decades(&state.library.albums);
            }
            state.tag_nav.reset(
                section.name(),
                BrowseItem::from_genres(state.tag_list_for(section)),
            );
            state.tag_nav.loading = false;
        }
        Action::Browse(BrowseAction::LoadTagList(section)) => {
            follow.push(
                BrowseAction::TagListLoaded {
                    library_key: state.active_library.clone().unwrap_or_default(),
                    section,
                    result: Ok(state.library.album_genres.as_ref().clone()),
                }
                .into(),
            );
        }
        Action::Browse(BrowseAction::LoadTagAlbums { replace_child }) => {
            if let Some(BrowseItem::Genre { key, title }) = state
                .tag_nav
                .columns
                .first()
                .and_then(|c| c.items.get(c.selected_index))
                .cloned()
            {
                state.tag_nav.columns.truncate(1);
                state.tag_nav.focused_column = 0;
                let albums = state
                    .library
                    .albums
                    .iter()
                    .filter(|a| a.genre.iter().any(|g| g.tag == title))
                    .cloned()
                    .collect();
                follow.push(
                    BrowseAction::TagAlbumsLoaded {
                        library_key: state.active_library.clone().unwrap_or_default(),
                        section: state.browse_category,
                        tag_key: key,
                        tag_title: title,
                        replace_child,
                        result: Ok(albums),
                    }
                    .into(),
                );
            }
        }
        Action::Browse(BrowseAction::LoadStations) => super::super::radio::load(state),
        Action::Folders(action) => return Some(folders(action, state)),
        Action::Navigation(NavigationAction::SetCategory { category, .. })
            if !matches!(
                category,
                BrowseCategory::Library
                    | BrowseCategory::AlbumGenres
                    | BrowseCategory::Playlists
                    | BrowseCategory::Folders
            ) =>
        {
            state.set_status("This section is not provided by Navidrome".into())
        }
        _ => return None,
    }
    Some(follow)
}

fn folders(action: FolderAction, state: &mut AppState) -> Vec<Action> {
    let mut nav = state.folder_state.take().unwrap_or_else(|| {
        FolderNavigationState::for_library(state.active_library.clone().unwrap_or_default())
    });
    match action {
        FolderAction::LoadFolderRoot => {
            let owners: std::collections::HashSet<_> = state
                .library
                .albums
                .iter()
                .filter_map(|album| album.parent_rating_key.as_deref())
                .collect();
            nav.columns = vec![FolderColumn::new(
                None,
                "Artists".into(),
                state
                    .library
                    .artists
                    .iter()
                    .filter(|artist| {
                        owners.contains(artist.rating_key.as_str())
                            && !state
                                .library
                                .compilations
                                .artist_keys
                                .contains(&artist.rating_key)
                    })
                    .map(|a| FolderItem::folder(a.rating_key.clone(), a.title.clone()))
                    .collect(),
            )];
            nav.focused_column = 0;
        }
        FolderAction::NavigateIntoFolder {
            folder_key,
            replace_child,
        } => {
            let (title, items, tracks) = if let Some(artist) = state
                .library
                .artists
                .iter()
                .find(|a| a.rating_key == folder_key)
            {
                let compilation_keys: std::collections::HashSet<_> = state
                    .library
                    .compilations
                    .albums
                    .iter()
                    .map(|album| album.rating_key.as_str())
                    .collect();
                (
                    artist.title.clone(),
                    state
                        .library
                        .albums
                        .iter()
                        .filter(|a| {
                            a.parent_rating_key.as_deref() == Some(&folder_key)
                                && !compilation_keys.contains(a.rating_key.as_str())
                        })
                        .map(|a| FolderItem::folder(a.rating_key.clone(), a.title.clone()))
                        .collect(),
                    Vec::new(),
                )
            } else {
                let tracks = album_tracks(state, &folder_key);
                (
                    album_name(state, &folder_key),
                    tracks
                        .iter()
                        .map(|t| {
                            FolderItem::track(
                                t.rating_key.clone(),
                                t.title.clone(),
                                t.rating_key.clone(),
                                t.duration,
                                t.parent_rating_key.clone(),
                                t.grandparent_rating_key.clone(),
                            )
                        })
                        .collect(),
                    tracks,
                )
            };
            let target = nav.focused_column + 1;
            nav.columns.truncate(target);
            let mut column = FolderColumn::new(Some(folder_key), title, items);
            column.tracks = tracks
                .into_iter()
                .map(|track| (track.rating_key.clone(), track))
                .collect();
            nav.columns.push(column);
            if !replace_child {
                nav.focused_column = target;
            }
        }
        FolderAction::PlayFolderTracks | FolderAction::PlayFolderTrack { .. } => {
            let index = match action {
                FolderAction::PlayFolderTrack { track_index } => track_index,
                _ => nav.focused().map_or(0, |col| col.selected_index),
            };
            let tracks = nav
                .focused()
                .map(|col| {
                    col.items
                        .iter()
                        .skip(index)
                        .filter_map(|i| {
                            i.rating_key
                                .as_ref()
                                .and_then(|key| col.tracks.get(key))
                                .cloned()
                        })
                        .collect()
                })
                .unwrap_or_default();
            state.folder_state = Some(nav);
            return vec![QueueAction::PlayTracksNow(tracks).into()];
        }
        FolderAction::RefreshSubfolder(_) => {}
    }
    state.folder_state = Some(nav);
    vec![]
}

#[cfg(test)]
mod folder_catalog_tests {
    use super::*;
    #[test]
    fn folders_exclude_empty_and_compilation_only_artist_directories() {
        let mut state = AppState::new();
        state.library.artists = std::sync::Arc::new(
            ["owner", "orphan", "guest"]
                .map(|key| crate::library::models::Artist {
                    rating_key: key.into(),
                    title: key.into(),
                    ..Default::default()
                })
                .to_vec(),
        );
        state.library.albums = std::sync::Arc::new(
            [("album", "owner"), ("compilation", "guest")]
                .map(|(key, owner)| crate::library::models::Album {
                    rating_key: key.into(),
                    parent_rating_key: Some(owner.into()),
                    title: key.into(),
                    ..Default::default()
                })
                .to_vec(),
        );
        state
            .library
            .compilations
            .artist_keys
            .insert("guest".into());
        folders(FolderAction::LoadFolderRoot, &mut state);
        let items = &state.folder_state.as_ref().unwrap().columns[0].items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].key, "owner");
    }
}

#[cfg(test)]
mod complete_release_tests {
    use super::*;
    #[test]
    fn disjoint_compilation_parts_browse_every_performer_from_any_part() {
        let mut state = AppState::new();
        state
            .library
            .compilations
            .complete_releases
            .insert("a".into(), "va".into());
        state
            .library
            .compilations
            .complete_releases
            .insert("b".into(), "va".into());
        state
            .library
            .compilations
            .release_members
            .insert("va".into(), vec!["a".into(), "b".into(), "va".into()]);
        state.library.all_tracks = std::sync::Arc::new(
            [("a", "A"), ("b", "B"), ("va", "C")]
                .into_iter()
                .map(|(album, artist)| Track {
                    rating_key: format!("song-{artist}"),
                    parent_rating_key: Some(album.into()),
                    original_title: Some(artist.into()),
                    ..Default::default()
                })
                .collect(),
        );
        state.library.track_lookup = crate::services::catalog_lookup::TrackLookup::build(
            &[],
            &[],
            &[],
            &state.library.all_tracks,
        );
        for key in ["a", "b", "va"] {
            let tracks = album_tracks(&state, key);
            assert_eq!(tracks.len(), 3);
            let performers: std::collections::HashSet<_> =
                tracks.iter().map(Track::track_artist).collect();
            assert_eq!(performers, std::collections::HashSet::from(["A", "B", "C"]));
        }
    }
    #[test]
    fn compilation_excerpt_opens_every_performer_in_the_complete_release() {
        let mut state = AppState::new();
        state
            .library
            .compilations
            .complete_releases
            .insert("excerpt".into(), "whole".into());
        state.library.all_tracks = std::sync::Arc::new(vec![
            Track {
                rating_key: "a".into(),
                parent_rating_key: Some("whole".into()),
                original_title: Some("Artist A".into()),
                ..Default::default()
            },
            Track {
                rating_key: "b".into(),
                parent_rating_key: Some("whole".into()),
                original_title: Some("Artist B".into()),
                ..Default::default()
            },
            Track {
                rating_key: "excerpt-song".into(),
                parent_rating_key: Some("excerpt".into()),
                original_title: Some("Artist A".into()),
                ..Default::default()
            },
        ]);
        state.library.track_lookup = crate::services::catalog_lookup::TrackLookup::build(
            &[],
            &[],
            &[],
            &state.library.all_tracks,
        );
        state
            .library
            .visited_album_tracks
            .insert("whole".into(), vec![]);
        let tracks = album_tracks(&state, "excerpt");
        assert_eq!(
            tracks
                .iter()
                .map(|t| t.rating_key.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_ne!(tracks[0].track_artist(), tracks[1].track_artist());
    }
}
