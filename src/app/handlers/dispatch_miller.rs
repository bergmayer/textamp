//! Miller column dispatch handlers for all *ForMiller and *FromMiller actions.

use crate::app::{Action, AppState, Event};
use crate::app::state::{BrowseColumn, BrowseItem, PlaybackMode, View};
use crate::api::PlexClient;
use crate::audio::AudioPlayer;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Dispatch Miller column actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: Action,
    state: &mut AppState,
    client: &mut PlexClient,
    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    match action {
        // ================================================================
        // Miller Column Actions for Artists View
        // ================================================================

        Action::LoadArtistAlbumsForMiller { artist_key } => {
            // Load albums for artist and add as new column in artist_nav
            // Prepend "All Tracks" entry before albums (same as old render path)
            state.artist_nav.loading = true;

            match client.get_artist_albums(&artist_key).await {
                Ok(albums) => {
                    // Create "All Tracks" entry first
                    let all_tracks = BrowseItem::AllTracks {
                        artist_key: artist_key.clone(),
                        artist_name: state.selected_artist_name.clone(),
                    };
                    // Then add albums
                    let mut items = vec![all_tracks];
                    items.extend(BrowseItem::from_albums(&albums));

                    let title = state.selected_artist_name.clone();
                    let col = BrowseColumn::new(title, items);
                    state.artist_nav.push_column(col);
                }
                Err(e) => {
                    state.set_error(format!("Failed to load albums: {}", e));
                }
            }
            state.artist_nav.loading = false;
        }

        Action::LoadAlbumTracksForMiller { album_key } => {
            // Load tracks for album and add as new column in artist_nav
            state.artist_nav.loading = true;

            match client.get_album_tracks(&album_key).await {
                Ok(tracks) => {
                    let items = BrowseItem::from_tracks(&tracks);
                    let title = state.selected_album_title.clone();
                    // Store full tracks for playback (includes media info)
                    let col = BrowseColumn::new_with_tracks(title, items, tracks);
                    state.artist_nav.push_column(col);
                }
                Err(e) => {
                    state.set_error(format!("Failed to load tracks: {}", e));
                }
            }
            state.artist_nav.loading = false;
        }

        Action::LoadArtistAllTracksForMiller { artist_key } => {
            // Load all tracks by an artist and add as new column in artist_nav
            // This is triggered by selecting "All Tracks" entry in the albums column
            state.artist_nav.loading = true;

            match client.get_artist_all_tracks(&artist_key).await {
                Ok(tracks) => {
                    let items = BrowseItem::from_tracks(&tracks);
                    let title = state.selected_album_title.clone();
                    // Store full tracks for playback (includes media info)
                    let col = BrowseColumn::new_with_tracks(title, items, tracks);
                    state.artist_nav.push_column(col);
                }
                Err(e) => {
                    state.set_error(format!("Failed to load tracks: {}", e));
                }
            }
            state.artist_nav.loading = false;
        }

        Action::PlayTrackFromMiller { column_index, track_index } => {
            // Get tracks from the specified column and play from track_index
            if let Some(col) = state.artist_nav.columns.get(column_index) {
                let tracks = helpers::collect_tracks_from_column(col);
                if !tracks.is_empty() {
                    audio.track_cache.flush();
                    state.queue.clear();
                    state.queue.extend(tracks);
                    state.queue_index = Some(track_index);
                    state.playback_mode = PlaybackMode::Queue;
                    state.view = View::NowPlaying;
                    helpers::play_current_track(event_tx, state, client, audio).await;
                }
            }
        }

        // Miller Column Actions for Genres View
        // ================================================================

        Action::LoadGenreAlbumsForMiller { genre_key } => {
            // Load albums for genre and add as new column in genre_nav
            state.genre_nav.loading = true;

            if let Some(lib_key) = &state.active_library.clone() {
                // Determine which API to call based on genre content type
                let albums_result = match state.genre_content_type {
                    crate::app::state::GenreContentType::ArtistGenres => {
                        client.get_artist_genre_albums(lib_key, &genre_key).await
                    }
                    crate::app::state::GenreContentType::AlbumGenres => {
                        client.get_album_genre_albums(lib_key, &genre_key).await
                    }
                    crate::app::state::GenreContentType::Moods => {
                        client.get_mood_albums(lib_key, &genre_key).await
                    }
                    crate::app::state::GenreContentType::Styles => {
                        client.get_style_albums(lib_key, &genre_key).await
                    }
                    _ => {
                        // Default genres use file-based tags
                        client.get_genre_albums(lib_key, &genre_key).await
                    }
                };

                match albums_result {
                    Ok(albums) => {
                        let items = BrowseItem::from_albums(&albums);
                        let col = BrowseColumn::new("albums", items);
                        state.genre_nav.push_column(col);
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load albums: {}", e));
                    }
                }
            }
            state.genre_nav.loading = false;
        }

        Action::LoadGenreTracksForMiller { album_key } => {
            // Load tracks for album and add as new column in genre_nav
            state.genre_nav.loading = true;

            match client.get_album_tracks(&album_key).await {
                Ok(tracks) => {
                    let items = BrowseItem::from_tracks(&tracks);
                    // Store full tracks for playback (includes media info)
                    let col = BrowseColumn::new_with_tracks("tracks", items, tracks);
                    state.genre_nav.push_column(col);
                }
                Err(e) => {
                    state.set_error(format!("Failed to load tracks: {}", e));
                }
            }
            state.genre_nav.loading = false;
        }

        Action::PlayGenreTrackFromMiller { column_index, track_index } => {
            // Get tracks from the specified column and play from track_index
            if let Some(col) = state.genre_nav.columns.get(column_index) {
                let tracks = helpers::collect_tracks_from_column(col);
                if !tracks.is_empty() {
                    audio.track_cache.flush();
                    state.queue.clear();
                    state.queue.extend(tracks);
                    state.queue_index = Some(track_index);
                    state.playback_mode = PlaybackMode::Queue;
                    state.view = View::NowPlaying;
                    helpers::play_current_track(event_tx, state, client, audio).await;
                }
            }
        }

        // Miller Column Actions for Playlists View
        // ================================================================

        Action::LoadPlaylistTracksForMiller { playlist_key } => {
            // Load tracks for playlist and add as new column in playlist_nav
            state.playlist_nav.loading = true;

            match client.get_playlist_tracks(&playlist_key).await {
                Ok(tracks) => {
                    let items = BrowseItem::from_tracks(&tracks);
                    // Store full tracks for playback (includes media info)
                    let col = BrowseColumn::new_with_tracks("tracks", items, tracks);
                    state.playlist_nav.push_column(col);
                }
                Err(e) => {
                    state.set_error(format!("Failed to load playlist tracks: {}", e));
                }
            }
            state.playlist_nav.loading = false;
        }

        Action::LoadAlbumTracksForPlaylistMiller { album_key } => {
            // Load tracks for album (in Recently Added mode) and add as new column in playlist_nav
            state.playlist_nav.loading = true;

            match client.get_album_tracks(&album_key).await {
                Ok(tracks) => {
                    let items = BrowseItem::from_tracks(&tracks);
                    let title = state.selected_album_title.clone();
                    // Store full tracks for playback (includes media info)
                    let col = BrowseColumn::new_with_tracks(title, items, tracks);
                    state.playlist_nav.push_column(col);
                }
                Err(e) => {
                    state.set_error(format!("Failed to load album tracks: {}", e));
                }
            }
            state.playlist_nav.loading = false;
        }

        Action::PlayPlaylistTrackFromMiller { column_index, track_index } => {
            // Get tracks from the specified column and play from track_index
            if let Some(col) = state.playlist_nav.columns.get(column_index) {
                let tracks = helpers::collect_tracks_from_column(col);
                if !tracks.is_empty() {
                    audio.track_cache.flush();
                    state.queue.clear();
                    state.queue.extend(tracks);
                    state.queue_index = Some(track_index);
                    state.playback_mode = PlaybackMode::Queue;
                    state.view = View::NowPlaying;
                    helpers::play_current_track(event_tx, state, client, audio).await;
                }
            }
        }

        _ => unreachable!("dispatch_miller called with non-miller action: {:?}", action),
    }
    Ok(vec![])
}
