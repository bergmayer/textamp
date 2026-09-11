use super::*;
use crate::app::state::{AllTracksScope, BrowseColumn, BrowseItem};

pub fn album_tracks(state: &AppState, key: &str) -> Vec<Track> {
    state
        .library
        .all_tracks
        .iter()
        .filter(|t| t.parent_rating_key.as_deref() == Some(key))
        .cloned()
        .collect()
}
pub fn artist_tracks(state: &AppState, key: &str) -> Vec<Track> {
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
        .cloned()
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
            let tracks = artist_tracks(state, &artist_key);
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
            let fetch_key = album_key.clone();
            let completion = move |result| {
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
            col.artwork_visible = state.artwork.default_visible;
            state.artist_nav.drill_column(col, replace_child);
        }
        Action::Miller(MillerAction::LoadAllLibraryTracksForMiller { replace_child }) => {
            let tracks = state.library.all_tracks.clone();
            let col = BrowseColumn::new_with_tracks(
                "all tracks",
                BrowseItem::from_tracks(&tracks),
                tracks,
            );
            state.artist_nav.drill_column(col, replace_child);
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
            spawn(state, tx, "playlist-browse", async move {
                let result = async {
                    Ok(session
                        .client
                        .playlist(&session.id(&playlist_key)?)
                        .await?
                        .entry
                        .into_iter()
                        .map(|t| session.track(t))
                        .collect::<Vec<_>>())
                }
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
                Ok(vec![MillerAction::AlbumTracksRefreshed {
                    request_id,
                    tag_section,
                    column_index,
                    result: session.album_tracks(&album_key).await.map_err(failure),
                }
                .into()])
            });
        }
        Action::Browse(BrowseAction::RefreshTagView) => {
            // The catalog owns Navidrome tags. An empty list is legitimate,
            // not a signal to synchronously load and refresh it again.
            state.tag_nav.reset(
                BrowseCategory::AlbumGenres.name(),
                BrowseItem::from_genres(&state.library.album_genres),
            );
            state.tag_nav.loading = false;
        }
        Action::Browse(BrowseAction::LoadTagList(section)) => {
            follow.push(
                BrowseAction::TagListLoaded {
                    library_key: state.active_library.clone().unwrap_or_default(),
                    section,
                    result: Ok(state.library.album_genres.clone()),
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
            nav.columns = vec![FolderColumn::new(
                None,
                "artists (virtual folders)".into(),
                state
                    .library
                    .artists
                    .iter()
                    .map(|a| FolderItem::folder(a.rating_key.clone(), a.title.clone()))
                    .collect(),
            )];
            nav.focused_column = 0;
        }
        FolderAction::NavigateIntoFolder {
            folder_key,
            replace_child,
        } => {
            let (title, items) = if let Some(artist) = state
                .library
                .artists
                .iter()
                .find(|a| a.rating_key == folder_key)
            {
                (
                    artist.title.clone(),
                    state
                        .library
                        .albums
                        .iter()
                        .filter(|a| a.parent_rating_key.as_deref() == Some(&folder_key))
                        .map(|a| FolderItem::folder(a.rating_key.clone(), a.title.clone()))
                        .collect(),
                )
            } else {
                (
                    album_name(state, &folder_key),
                    album_tracks(state, &folder_key)
                        .into_iter()
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
                )
            };
            let target = nav.focused_column + 1;
            nav.columns.truncate(target);
            nav.columns
                .push(FolderColumn::new(Some(folder_key), title, items));
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
                            state
                                .library
                                .all_tracks
                                .iter()
                                .find(|t| Some(&t.rating_key) == i.rating_key.as_ref())
                                .cloned()
                        })
                        .collect()
                })
                .unwrap_or_default();
            state.folder_state = Some(nav);
            return vec![QueueAction::PlayTracksNow(tracks).into()];
        }
        FolderAction::RefreshSubfolder(_) => {}
        _ => {}
    }
    state.folder_state = Some(nav);
    vec![]
}
