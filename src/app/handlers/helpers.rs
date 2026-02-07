//! Shared utility functions used across multiple handler modules.

use crate::app::{Action, AppState, Event};
use crate::app::event_loop::PreloadType;
use crate::app::state::{
    BrowseCategory, Focus, PlayStatus, PlaybackMode,
    RightPanelMode, SearchTab, View,
};
use crate::api::PlexClient;
use crate::api::models::Track;
use crate::audio::AudioPlayer;
use crate::audio::cache;
use crate::cache::LibraryCache;

use std::time::Duration;
use tokio::sync::mpsc;

/// Page size for paginated API requests.
pub const PAGE_SIZE: u32 = 100;

// ============================================================================
// Sorting
// ============================================================================

/// Generate a sort key for a title, ignoring "The " prefix.
pub fn sort_key(title: &str) -> String {
    let lower = title.to_lowercase();
    if lower.starts_with("the ") {
        lower[4..].to_string()
    } else {
        lower
    }
}

// ============================================================================
// Connection discovery
// ============================================================================

/// Find the first working server connection by testing ALL connections in PARALLEL.
/// Priority: local non-relay > non-relay > relay
pub async fn find_working_connection(
    server: &crate::api::models::PlexServer,
    token: &str,
    client_identifier: &str,
) -> Option<String> {
    use futures::future::join_all;

    let mut prioritized: Vec<(usize, &str)> = Vec::new();

    for conn in &server.connections {
        let priority = if conn.local && !conn.relay {
            0
        } else if !conn.relay {
            1
        } else {
            2
        };
        prioritized.push((priority, conn.uri.as_str()));
    }

    if prioritized.is_empty() {
        tracing::warn!("No connections available for server {}", server.name);
        return None;
    }

    let token_str = token.to_string();
    let client_id = client_identifier.to_string();
    let futures = prioritized.iter().map(|(priority, uri)| {
        let uri = uri.to_string();
        let token = token_str.clone();
        let client_id = client_id.clone();
        let prio = *priority;
        async move {
            match crate::plex::test_connection(&uri, &token, &client_id).await {
                Ok(()) => {
                    tracing::info!("Connection test succeeded: {} (priority {})", uri, prio);
                    Some((prio, uri))
                }
                Err(e) => {
                    tracing::debug!("Connection test failed for {}: {}", uri, e);
                    None
                }
            }
        }
    });

    let results: Vec<Option<(usize, String)>> = join_all(futures).await;

    let mut successes: Vec<(usize, String)> = results.into_iter().flatten().collect();
    successes.sort_by_key(|(prio, _)| *prio);

    if let Some((prio, url)) = successes.into_iter().next() {
        let prio_name = match prio {
            0 => "local",
            1 => "remote",
            _ => "relay",
        };
        tracing::info!("Selected {} connection: {}", prio_name, url);
        return Some(url);
    }

    tracing::warn!("All connection tests failed for server {}", server.name);
    None
}

/// Find the first working connection across multiple servers.
pub async fn find_working_connection_from_servers(
    servers: &[crate::api::models::PlexServer],
    token: &str,
    client_identifier: &str,
) -> Option<String> {
    for server in servers {
        if let Some(url) = find_working_connection(server, token, client_identifier).await {
            return Some(url);
        }
    }
    None
}

// NOTE: ConnectionState Display impl remains in event_loop.rs until full migration.

// ============================================================================
// Scroll calculation
// ============================================================================

/// Calculate the scroll offset to keep the selected item centered.
pub fn calc_scroll_offset(selected: usize, viewport_height: usize, total_items: usize) -> usize {
    if total_items == 0 || viewport_height == 0 {
        return 0;
    }
    let half_height = viewport_height / 2;
    if selected < half_height {
        0
    } else if selected + half_height >= total_items {
        total_items.saturating_sub(viewport_height)
    } else {
        selected.saturating_sub(half_height)
    }
}

// ============================================================================
// Data loading helpers
// ============================================================================

/// Load artists in background.
pub fn load_artists(event_tx: &mpsc::Sender<Event>, state: &mut AppState, client: &PlexClient) {
    if let Some(lib_key) = &state.active_library {
        tracing::info!("Loading all artists from library: {}", lib_key);
        state.artists_loading = true;

        let event_tx = event_tx.clone();
        let client = client.clone();
        let lib_key = lib_key.clone();
        tokio::spawn(async move {
            match client.get_artists(&lib_key).await {
                Ok(artists) => {
                    tracing::info!("Loaded {} artists", artists.len());
                    let _ = event_tx.send(Event::ArtistsLoaded(artists)).await;
                }
                Err(e) => {
                    tracing::error!("Failed to load artists: {}", e);
                }
            }
        });
    } else {
        tracing::warn!("load_artists called but no active_library set");
    }
}

/// Load albums in background.
pub fn load_albums(event_tx: &mpsc::Sender<Event>, state: &mut AppState, client: &PlexClient) {
    if let Some(lib_key) = &state.active_library {
        tracing::info!("Loading all albums from library: {}", lib_key);
        state.albums_loading = true;

        let event_tx = event_tx.clone();
        let client = client.clone();
        let lib_key = lib_key.clone();
        tokio::spawn(async move {
            match client.get_albums(&lib_key).await {
                Ok(albums) => {
                    tracing::info!("Loaded {} albums", albums.len());
                    let _ = event_tx.send(Event::AlbumsLoaded(albums)).await;
                }
                Err(e) => {
                    tracing::error!("Failed to load albums: {}", e);
                }
            }
        });
    }
}

/// Load playlists in background.
pub fn load_playlists(event_tx: &mpsc::Sender<Event>, state: &mut AppState, client: &PlexClient) {
    tracing::info!("Loading playlists");
    state.playlists_loading = true;

    let event_tx = event_tx.clone();
    let client = client.clone();
    tokio::spawn(async move {
        match client.get_playlists().await {
            Ok(playlists) => {
                tracing::info!("Loaded {} playlists", playlists.len());
                let _ = event_tx.send(Event::PlaylistsLoaded(playlists)).await;
            }
            Err(e) => {
                tracing::error!("Failed to load playlists: {}", e);
            }
        }
    });
}

/// Load more data when nearing the end of a paginated list.
pub async fn maybe_load_more(state: &mut AppState, client: &PlexClient) {
    if state.view != View::Browse || state.focus != Focus::Left {
        return;
    }

    if let Some(lib_key) = &state.active_library.clone() {
        match state.browse_category {
            BrowseCategory::Artists => {
                let idx = state.list_state.artists_index;
                let loaded = state.artists.len();
                let total = state.artists_total as usize;

                if idx + 20 >= loaded && loaded < total && !state.artists_loading {
                    state.artists_loading = true;
                    let offset = loaded as u32;
                    if let Ok((more, _)) = client.get_artists_page(lib_key, offset, PAGE_SIZE).await {
                        state.artists.extend(more);
                        state.artists.sort_by(|a, b| sort_key(&a.title).cmp(&sort_key(&b.title)));
                    }
                    state.artists_loading = false;
                }
            }
            _ => {}
        }
    }
}

// ============================================================================
// Filter/search selection
// ============================================================================

/// Select a filter result from the search/filter view.
pub fn select_filter_result(state: &mut AppState) -> Vec<Action> {
    let idx = state.list_state.search_item_index;
    let search_tab = state.search_tab;

    match search_tab {
        SearchTab::Global => {
            return vec![];
        }
        SearchTab::Artists => {
            if let Some(ref results) = state.filter_results {
                if let Some(artist) = results.artists.get(idx) {
                    state.selected_artist_name = artist.title.clone();
                    state.pending_filter_key = Some(artist.rating_key.clone());
                    state.search_query.clear();
                    state.filter_results = None;
                    state.view = View::Browse;
                    state.browse_category = BrowseCategory::Artists;
                    return vec![Action::LoadArtistAlbums];
                }
            }
            if let Some(artist) = state.artists.iter().enumerate()
                .filter(|(_, a)| state.search_query.is_empty() || a.title.to_lowercase().contains(&state.search_query.to_lowercase()))
                .nth(idx)
                .map(|(i, _)| i)
            {
                state.set_category_index(artist);
                state.search_query.clear();
                state.filter_results = None;
                state.view = View::Browse;
                state.browse_category = BrowseCategory::Artists;
            }
        }
        SearchTab::AlbumArtists => {
            let query = state.search_query.to_lowercase();
            let mut album_artists: Vec<(String, String)> = state.albums.iter()
                .filter_map(|a| {
                    let artist = a.parent_title.as_deref().unwrap_or("");
                    if !artist.is_empty() && (query.is_empty() || artist.to_lowercase().contains(&query)) {
                        Some((artist.to_string(), a.rating_key.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            album_artists.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
            album_artists.dedup_by(|a, b| a.0.to_lowercase() == b.0.to_lowercase());

            if let Some((_, _album_key)) = album_artists.get(idx) {
                state.search_query.clear();
                state.filter_results = None;
                state.view = View::Browse;
            }
        }
        SearchTab::Albums => {
            if let Some(ref results) = state.filter_results {
                if let Some(album) = results.albums.get(idx).cloned() {
                    state.search_query.clear();
                    state.filter_results = None;
                    state.view = View::Browse;
                    return vec![Action::PlayAlbum { rating_key: album.rating_key }];
                }
            }
        }
        SearchTab::Playlists => {
            let query = state.search_query.to_lowercase();
            if let Some((i, _playlist)) = state.playlists.iter().enumerate()
                .filter(|(_, p)| query.is_empty() || p.title.to_lowercase().contains(&query))
                .nth(idx)
            {
                state.set_category_index(i);
                state.search_query.clear();
                state.filter_results = None;
                state.view = View::Browse;
                state.browse_category = BrowseCategory::Playlists;
                return vec![Action::LoadCategoryTracks];
            }
        }
        SearchTab::Tracks => {
            if let Some(ref results) = state.filter_results {
                if let Some(track) = results.tracks.get(idx).cloned() {
                    state.search_query.clear();
                    state.filter_results = None;
                    state.view = View::Browse;
                    state.queue.clear();
                    state.queue.push(track.clone());
                    state.queue_index = Some(0);
                    state.playback_mode = PlaybackMode::Queue;
                    return vec![Action::PlayTrack(track)];
                }
            }
        }
        SearchTab::Genres => {
            let query = state.search_query.to_lowercase();
            if let Some(i) = state.genres.iter().enumerate()
                .filter(|(_, g)| query.is_empty() || g.title.to_lowercase().contains(&query))
                .nth(idx)
                .map(|(i, _)| i)
            {
                state.set_category_index(i);
                state.search_query.clear();
                state.filter_results = None;
                state.view = View::Browse;
                state.browse_category = BrowseCategory::Genres;
            }
        }
    }

    vec![]
}

// ============================================================================
// List index management
// ============================================================================

/// Adjust a list index by a delta (relative movement).
pub fn adjust_list_index(state: &mut AppState, delta: isize) {
    match state.view {
        View::Browse => {
            if state.focus == Focus::Left {
                let len = state.category_len();
                if len > 0 {
                    let idx = state.category_index() as isize + delta;
                    state.set_category_index(idx.clamp(0, len as isize - 1) as usize);
                }
            } else {
                match state.right_panel_mode {
                    RightPanelMode::ArtistAlbums => {
                        let len = state.selected_artist_albums.len() + 1;
                        if len > 0 {
                            let idx = state.list_state.right_albums_index as isize + delta;
                            state.list_state.right_albums_index = idx.clamp(0, len as isize - 1) as usize;
                        }
                    }
                    RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                        let len = state.selected_album_tracks.len();
                        if len > 0 {
                            let idx = state.list_state.tracks_index as isize + delta;
                            state.list_state.tracks_index = idx.clamp(0, len as isize - 1) as usize;
                        }
                    }
                    RightPanelMode::CategoryAlbums => {
                        let len = state.genre_albums.len();
                        if len > 0 {
                            let idx = state.genre_albums_index as isize + delta;
                            state.genre_albums_index = idx.clamp(0, len as isize - 1) as usize;
                        }
                    }
                    RightPanelMode::Empty => {}
                }
            }
        }
        View::NowPlaying => {
            let len = state.queue.len();
            if len > 0 {
                let idx = state.list_state.queue_index as isize + delta;
                state.list_state.queue_index = idx.clamp(0, len as isize - 1) as usize;
            }
        }
        View::Similar => {
            let len = match state.similar_mode {
                crate::app::state::SimilarMode::Albums => state.similar_albums.len(),
                crate::app::state::SimilarMode::Tracks => state.similar_tracks.len(),
            };
            if len > 0 {
                let idx = state.list_state.similar_index as isize + delta;
                state.list_state.similar_index = idx.clamp(0, len as isize - 1) as usize;
            }
        }
        View::Search => {
            let filtered_len = if let Some(ref results) = state.filter_results {
                match state.search_tab {
                    SearchTab::Global => 0,
                    SearchTab::Artists => results.artists.len(),
                    SearchTab::AlbumArtists => {
                        let query = state.search_query.to_lowercase();
                        let mut artists: Vec<String> = state.albums.iter()
                            .filter_map(|a| a.parent_title.as_ref())
                            .filter(|t| query.is_empty() || t.to_lowercase().contains(&query))
                            .map(|s| s.to_lowercase())
                            .collect();
                        artists.sort();
                        artists.dedup();
                        artists.len()
                    }
                    SearchTab::Albums => results.albums.len(),
                    SearchTab::Playlists => {
                        let query = state.search_query.to_lowercase();
                        state.playlists.iter()
                            .filter(|p| query.is_empty() || p.title.to_lowercase().contains(&query))
                            .count()
                    }
                    SearchTab::Tracks => results.tracks.len(),
                    SearchTab::Genres => {
                        let query = state.search_query.to_lowercase();
                        state.genres.iter()
                            .filter(|g| query.is_empty() || g.title.to_lowercase().contains(&query))
                            .count()
                    }
                }
            } else {
                let query = state.search_query.to_lowercase();
                match state.search_tab {
                    SearchTab::Global => 0,
                    SearchTab::Artists => state.artists.iter()
                        .filter(|a| query.is_empty() || a.title.to_lowercase().contains(&query))
                        .count(),
                    SearchTab::AlbumArtists => {
                        let mut artists: Vec<String> = state.albums.iter()
                            .filter_map(|a| a.parent_title.as_ref())
                            .filter(|t| query.is_empty() || t.to_lowercase().contains(&query))
                            .map(|s| s.to_lowercase())
                            .collect();
                        artists.sort();
                        artists.dedup();
                        artists.len()
                    }
                    SearchTab::Albums => state.albums.iter()
                        .filter(|a| query.is_empty() || a.title.to_lowercase().contains(&query))
                        .count(),
                    SearchTab::Playlists => state.playlists.iter()
                        .filter(|p| query.is_empty() || p.title.to_lowercase().contains(&query))
                        .count(),
                    SearchTab::Tracks => state.selected_album_tracks.iter()
                        .filter(|t| query.is_empty() || t.title.to_lowercase().contains(&query))
                        .count(),
                    SearchTab::Genres => state.genres.iter()
                        .filter(|g| query.is_empty() || g.title.to_lowercase().contains(&query))
                        .count(),
                }
            };

            if filtered_len > 0 {
                let idx = state.list_state.search_item_index as isize + delta;
                state.list_state.search_item_index = idx.clamp(0, filtered_len as isize - 1) as usize;
            }
        }
        _ => {}
    }
}

/// Set a list index to an absolute position.
pub fn set_list_index(state: &mut AppState, index: isize) {
    match state.view {
        View::Browse => {
            if state.focus == Focus::Left {
                let len = state.category_len();
                let idx = if index == isize::MAX {
                    len.saturating_sub(1)
                } else {
                    (index as usize).min(len.saturating_sub(1))
                };
                state.set_category_index(idx);
            } else {
                match state.right_panel_mode {
                    RightPanelMode::ArtistAlbums => {
                        let len = state.selected_artist_albums.len() + 1;
                        state.list_state.right_albums_index = if index == isize::MAX {
                            len.saturating_sub(1)
                        } else {
                            (index as usize).min(len.saturating_sub(1))
                        };
                    }
                    RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                        let len = state.selected_album_tracks.len();
                        state.list_state.tracks_index = if index == isize::MAX {
                            len.saturating_sub(1)
                        } else {
                            (index as usize).min(len.saturating_sub(1))
                        };
                    }
                    RightPanelMode::CategoryAlbums => {
                        let len = state.genre_albums.len();
                        state.genre_albums_index = if index == isize::MAX {
                            len.saturating_sub(1)
                        } else {
                            (index as usize).min(len.saturating_sub(1))
                        };
                    }
                    RightPanelMode::Empty => {}
                }
            }
        }
        View::NowPlaying => {
            let len = state.queue.len();
            state.list_state.queue_index = if index == isize::MAX {
                len.saturating_sub(1)
            } else {
                (index as usize).min(len.saturating_sub(1))
            };
        }
        View::Similar => {
            let len = match state.similar_mode {
                crate::app::state::SimilarMode::Albums => state.similar_albums.len(),
                crate::app::state::SimilarMode::Tracks => state.similar_tracks.len(),
            };
            state.list_state.similar_index = if index == isize::MAX {
                len.saturating_sub(1)
            } else {
                (index as usize).min(len.saturating_sub(1))
            };
        }
        _ => {}
    }
}

// ============================================================================
// Playback helpers
// ============================================================================

/// Play a track, setting up queue context.
pub async fn play_track(
    event_tx: &mpsc::Sender<Event>,
    track: Track,
    state: &mut AppState,
    client: &PlexClient,
    audio: &mut AudioPlayer,
) {
    // Report stop for currently playing track before switching
    if let Some(current) = state.current_track().cloned() {
        report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
    }

    // Generate new session ID for this playback context
    state.plex_session_id = Some(generate_plex_session_id());

    if state.view == View::NowPlaying || state.view == View::Similar {
        if state.playback_mode == PlaybackMode::Radio {
            state.radio.clear();
        }
        state.queue_original.clear();
        state.queue_sort_mode = crate::app::state::QueueSortMode::QueueOrder;
        state.playback_mode = PlaybackMode::Queue;
        play_current_track(event_tx, state, client, audio).await;
    } else {
        if state.playback_mode == PlaybackMode::Radio {
            state.radio.clear();
        }
        state.queue = vec![track];
        state.queue_index = Some(0);
        state.queue_original.clear();
        state.queue_sort_mode = crate::app::state::QueueSortMode::QueueOrder;
        state.playback_mode = PlaybackMode::Queue;
        play_current_track(event_tx, state, client, audio).await;
    }
}

/// Helper to collect tracks from a Miller column for playback.
pub fn collect_tracks_from_column(col: &crate::app::state::BrowseColumn) -> Vec<Track> {
    if !col.tracks.is_empty() {
        return col.tracks.clone();
    }

    let track_count = col.items.iter().filter(|item| matches!(item, crate::app::state::BrowseItem::Track { .. })).count();
    if track_count > 0 {
        tracing::warn!(
            "collect_tracks_from_column fallback: creating {} track stubs without media info for column '{}'. Direct playback may fail.",
            track_count,
            col.title
        );
    }

    col.items.iter()
        .filter_map(|item| {
            if let crate::app::state::BrowseItem::Track { key, title, duration_ms, track_number } = item {
                Some(Track {
                    rating_key: key.clone(),
                    title: title.clone(),
                    duration: Some(*duration_ms),
                    index: *track_number,
                    parent_title: None,
                    grandparent_title: None,
                    parent_rating_key: None,
                    grandparent_rating_key: None,
                    media: vec![],
                    thumb: None,
                    key: String::new(),
                    parent_thumb: None,
                    grandparent_thumb: None,
                })
            } else {
                None
            }
        })
        .collect()
}

/// Play the current track from the queue.
pub async fn play_current_track(
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,
    client: &PlexClient,
    audio: &mut AudioPlayer,
) {
    if let Some(track) = state.current_track().cloned() {
        tracing::info!("Playing: {} - {}", track.artist_name(), track.title);
        tracing::info!("PlayCurrentTrack: client_identifier={}", client.client_identifier());
        tracing::info!("PlayCurrentTrack: server_url={:?}", client.server_url());
        tracing::info!("PlayCurrentTrack: has_token={}", client.token().is_some());
        tracing::info!("PlayCurrentTrack: track.media.len()={}", track.media.len());

        state.playback.status = PlayStatus::Buffering;
        state.playback.duration_ms = track.duration_ms();
        state.playback.position_ms = 0;

        // Reset waveform state for new track
        if state.waveform.track_key.as_ref() != Some(&track.rating_key) {
            state.waveform = crate::app::state::WaveformState::default();
            state.waveform.track_key = Some(track.rating_key.clone());

            // Auto-generate waveform if currently in visualizer mode
            if state.view == View::NowPlaying
                && state.now_playing_mode == crate::app::state::NowPlayingMode::NowPlaying
            {
                if let Ok(stream_url) = client.get_stream_url(&track) {
                    state.waveform.generating = true;
                    let track_key = track.rating_key.clone();
                    let duration_ms = track.duration_ms();
                    let event_tx = event_tx.clone();
                    let token = client.token().map(|s| s.to_string());

                    tokio::spawn(async move {
                        let cache_dir = dirs::cache_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                            .join("textamp")
                            .join("waveforms");
                        let cache = crate::services::WaveformCache::new(cache_dir);

                        if let Some(data) = cache.load(&track_key) {
                            let _ = event_tx.send(Event::WaveformCacheHit {
                                track_key,
                                data,
                            }).await;
                            return;
                        }

                        let http_client = reqwest::Client::new();
                        let mut request = http_client.get(&stream_url);
                        if let Some(ref token) = token {
                            request = request.header("X-Plex-Token", token);
                        }

                        match request.send().await {
                            Ok(response) => {
                                match response.bytes().await {
                                    Ok(audio_data) => {
                                        match crate::services::generate_waveform(
                                            track_key.clone(),
                                            duration_ms,
                                            audio_data.to_vec(),
                                        ) {
                                            Ok(data) => {
                                                cache.save(&data);
                                                let _ = event_tx.send(Event::WaveformGenerated {
                                                    track_key,
                                                    data,
                                                }).await;
                                            }
                                            Err(e) => {
                                                let _ = event_tx.send(Event::WaveformFailed {
                                                    track_key,
                                                    error: e.to_string(),
                                                }).await;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let _ = event_tx.send(Event::WaveformFailed {
                                            track_key,
                                            error: format!("Download failed: {}", e),
                                        }).await;
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = event_tx.send(Event::WaveformFailed {
                                    track_key,
                                    error: format!("Request failed: {}", e),
                                }).await;
                            }
                        }
                    });
                }
            }
        }

        // Load artwork for the new track (non-blocking)
        if let Some(thumb_path) = track.best_thumb() {
            if state.artwork_thumb.as_deref() != Some(thumb_path) {
                if let Some(server_url) = client.server_url() {
                    state.artwork_loading = true;
                    let thumb_path_owned = thumb_path.to_string();
                    let event_tx = event_tx.clone();
                    let server_url = server_url.to_string();
                    let token = client.token().map(|s| s.to_string());
                    let client_id = client.client_identifier().to_string();

                    tokio::spawn(async move {
                        let client = crate::api::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            client.fetch_artwork(&thumb_path_owned, 300)
                        ).await {
                            Ok(Ok(data)) => {
                                let _ = event_tx.send(Event::ArtworkLoaded {
                                    thumb_path: thumb_path_owned,
                                    data,
                                }).await;
                            }
                            Ok(Err(e)) => {
                                tracing::warn!("Failed to load artwork: {}", e);
                                let _ = event_tx.send(Event::ArtworkFailed {
                                    thumb_path: thumb_path_owned,
                                }).await;
                            }
                            Err(_) => {
                                tracing::warn!("Artwork loading timed out");
                                let _ = event_tx.send(Event::ArtworkFailed {
                                    thumb_path: thumb_path_owned,
                                }).await;
                            }
                        }
                    });
                } else {
                    state.artwork_loading = false;
                    state.artwork_data = None;
                }
            } else {
                state.artwork_loading = false;
            }
        } else {
            state.artwork_thumb = None;
            state.artwork_data = None;
            state.artwork_loading = false;
        }

        // Check track cache first (pre-fetched audio data)
        if let Some(cached_data) = audio.track_cache.get(&track.rating_key) {
            tracing::info!("Cache hit for: {} - {}", track.artist_name(), track.title);
            // Stop current playback before starting cached playback
            audio.stop();
            match audio.play_data(cached_data) {
                Ok(()) => {
                    state.playback.status = PlayStatus::Playing;
                    report_playback_to_plex(event_tx, &track, state.plex_session_id.clone(), client);
                    state.last_progress_report = Some(std::time::Instant::now());
                    update_local_recently_played(state, &track);
                    // Trigger pre-fetch for next tracks
                    let upcoming = cache::get_upcoming_tracks(state);
                    cache::trigger_prefetch(&audio.track_cache, &upcoming, client);
                    return;
                }
                Err(e) => {
                    tracing::warn!("Cached playback failed, falling back to stream: {}", e);
                    audio.track_cache.remove(&track.rating_key);
                    // Fall through to normal streaming path
                }
            }
        }

        // Build stream URLs: primary (direct) + fallback (transcode)
        let primary_url = client.get_stream_url(&track).ok();
        let fallback_url = client.get_transcoded_stream_url(&track).ok();

        if let Some(url) = primary_url {
            tracing::debug!("Direct stream URL: {}", url);
            // play_url_with_headers spawns HTTP fetch in background — returns immediately
            if let Err(e) = audio.play_url_with_headers(&url, reqwest::header::HeaderMap::new(), fallback_url, event_tx.clone()).await {
                state.set_error(format!("Playback failed: {}", e));
                state.playback.status = PlayStatus::Stopped;
                return;
            }
            report_playback_to_plex(event_tx, &track, state.plex_session_id.clone(), client);
            state.last_progress_report = Some(std::time::Instant::now());
            update_local_recently_played(state, &track);
            // Trigger pre-fetch for next tracks
            let upcoming = cache::get_upcoming_tracks(state);
            cache::trigger_prefetch(&audio.track_cache, &upcoming, client);
        } else if let Some(url) = fallback_url {
            let redacted = url.split("X-Plex-Token=").next().unwrap_or(&url);
            tracing::info!("Using transcoded stream for: {} - URL: {}...", track.title, redacted);
            if let Err(e) = audio.play_url_with_headers(&url, reqwest::header::HeaderMap::new(), None, event_tx.clone()).await {
                state.set_error(format!("Playback failed: {}", e));
                state.playback.status = PlayStatus::Stopped;
                return;
            }
            report_playback_to_plex(event_tx, &track, state.plex_session_id.clone(), client);
            state.last_progress_report = Some(std::time::Instant::now());
            update_local_recently_played(state, &track);
            // Trigger pre-fetch for next tracks
            let upcoming = cache::get_upcoming_tracks(state);
            cache::trigger_prefetch(&audio.track_cache, &upcoming, client);
        } else {
            tracing::error!("Cannot get any stream URL (track has {} media items)", track.media.len());
            state.set_error("Failed to get stream URL".to_string());
            state.playback.status = PlayStatus::Stopped;
        }
    }
}

// ============================================================================
// Plex reporting
// ============================================================================

/// Report playback start to Plex server in background.
pub fn report_playback_to_plex(_event_tx: &mpsc::Sender<Event>, track: &Track, session_id: Option<String>, client: &PlexClient) {
    if let Some(server_url) = client.server_url() {
        let rating_key = track.rating_key.clone();
        let track_clone = track.clone();
        let server_url = server_url.to_string();
        let token = client.token().map(|s| s.to_string());
        let client_id = client.client_identifier().to_string();

        tokio::spawn(async move {
            let client = crate::api::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);

            if let Err(e) = client.report_playback_start(&track_clone, 0, session_id.as_deref()).await {
                tracing::debug!("Failed to report playback start: {}", e);
            }

            if let Err(e) = client.scrobble(&rating_key).await {
                tracing::debug!("Failed to scrobble: {}", e);
            } else {
                tracing::debug!("Scrobbled track: {}", rating_key);
            }
        });
    }
}

/// Report playback stop to Plex server in background.
pub fn report_playback_stop_to_plex(
    track: &Track,
    position_ms: u64,
    continuing: bool,
    session_id: Option<String>,
    client: &PlexClient,
) {
    if let Some(server_url) = client.server_url() {
        let track_clone = track.clone();
        let server_url = server_url.to_string();
        let token = client.token().map(|s| s.to_string());
        let client_id = client.client_identifier().to_string();

        tokio::spawn(async move {
            let client = crate::api::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);

            if let Err(e) = client.report_playback_stop(&track_clone, position_ms, continuing, session_id.as_deref()).await {
                tracing::debug!("Failed to report playback stop: {}", e);
            } else {
                tracing::debug!("Reported playback stop for: {} (continuing={}, session={:?})", track_clone.title, continuing, session_id);
            }
        });
    }
}

/// Report playback progress to Plex server in background.
pub fn report_playback_progress_to_plex(
    track: &Track,
    position_ms: u64,
    session_id: Option<String>,
    client: &PlexClient,
) {
    if let Some(server_url) = client.server_url() {
        let track_clone = track.clone();
        let server_url = server_url.to_string();
        let token = client.token().map(|s| s.to_string());
        let client_id = client.client_identifier().to_string();

        tokio::spawn(async move {
            let client = crate::api::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);

            if let Err(e) = client.report_playback_progress(&track_clone, position_ms, session_id.as_deref()).await {
                tracing::debug!("Failed to report playback progress: {}", e);
            }
        });
    }
}

/// Generate a new Plex session ID for timeline reporting.
pub fn generate_plex_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Update local recently played albums list when a track starts playing.
pub fn update_local_recently_played(state: &mut AppState, track: &Track) {
    use crate::api::models::Album;

    if let Some(album) = Album::from_track(track) {
        let album_key = album.rating_key.clone();
        state.recently_played_albums.retain(|a| a.rating_key != album_key);
        state.recently_played_albums.insert(0, album);
        state.recently_played_albums.truncate(50);
        state.cache_dirty = true;
        tracing::debug!("Updated local recently played: {} items", state.recently_played_albums.len());
    }
}

// ============================================================================
// Preloading
// ============================================================================

/// Preload data in background for faster access.
pub fn preload_data(event_tx: &mpsc::Sender<Event>, preload_type: PreloadType, lib_key: &str, client: &PlexClient) {
    use crate::services::{FolderColumn, FolderNavigationState, FolderService};

    let Some(server_url) = client.server_url() else { return };
    let server_url = server_url.to_string();
    let token = client.token().map(|s| s.to_string());
    let client_id = client.client_identifier().to_string();
    let lib_key = lib_key.to_string();
    let event_tx = event_tx.clone();

    tokio::spawn(async move {
        let client = crate::api::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);
        let lib_key_ref = lib_key.as_str();

        match preload_type {
            PreloadType::Artists => {
                tracing::debug!("Preloading artists for library: {}", lib_key);
                if let Ok(data) = client.get_artists(lib_key_ref).await {
                    tracing::debug!("Artists preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::ArtistsPreloaded { library_key: lib_key, artists: data }).await;
                }
            }
            PreloadType::Albums => {
                tracing::debug!("Preloading albums for library: {}", lib_key);
                if let Ok(data) = client.get_albums(lib_key_ref).await {
                    tracing::debug!("Albums preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::AlbumsPreloaded { library_key: lib_key, albums: data }).await;
                }
            }
            PreloadType::Playlists => {
                tracing::debug!("Preloading playlists");
                if let Ok(data) = client.get_playlists().await {
                    tracing::debug!("Playlists preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::PlaylistsPreloaded { library_key: lib_key, playlists: data }).await;
                }
            }
            PreloadType::Genres => {
                tracing::debug!("Preloading genres for library: {}", lib_key);
                if let Ok(data) = client.get_genres(lib_key_ref).await {
                    tracing::debug!("Genres preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::GenresPreloaded { library_key: lib_key, genres: data }).await;
                }
            }
            PreloadType::Moods => {
                tracing::debug!("Preloading moods for library: {}", lib_key);
                if let Ok(data) = client.get_moods(lib_key_ref).await {
                    tracing::debug!("Moods preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::MoodsPreloaded { library_key: lib_key, moods: data }).await;
                }
            }
            PreloadType::ArtistGenres => {
                tracing::debug!("Preloading artist genres for library: {}", lib_key);
                if let Ok(data) = client.get_artist_genres(lib_key_ref).await {
                    tracing::debug!("Artist genres preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::ArtistGenresPreloaded { library_key: lib_key, genres: data }).await;
                }
            }
            PreloadType::AlbumGenres => {
                tracing::debug!("Preloading album genres for library: {}", lib_key);
                if let Ok(data) = client.get_album_genres(lib_key_ref).await {
                    tracing::debug!("Album genres preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::AlbumGenresPreloaded { library_key: lib_key, genres: data }).await;
                }
            }
            PreloadType::Styles => {
                tracing::debug!("Preloading styles for library: {}", lib_key);
                if let Ok(data) = client.get_styles(lib_key_ref).await {
                    tracing::debug!("Styles preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::StylesPreloaded { library_key: lib_key, styles: data }).await;
                }
            }
            PreloadType::Stations => {
                tracing::debug!("Preloading stations for library: {}", lib_key);
                if let Ok(data) = client.get_stations(lib_key_ref).await {
                    tracing::debug!("Stations preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::StationsPreloaded { library_key: lib_key, stations: data }).await;
                }
            }
            PreloadType::RecentlyAdded => {
                tracing::debug!("Preloading recently added albums for library: {}", lib_key);
                if let Ok(data) = client.get_recently_added_albums(lib_key_ref, 50).await {
                    tracing::debug!("Recently added albums preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::RecentlyAddedPreloaded { library_key: lib_key, albums: data }).await;
                }
            }
            PreloadType::RecentlyPlayed => {
                tracing::debug!("Preloading recently played albums for library: {}", lib_key);
                if let Ok(data) = client.get_recently_played_albums(lib_key_ref, 50).await {
                    tracing::debug!("Recently played albums preloaded: {} items", data.len());
                    let _ = event_tx.send(Event::RecentlyPlayedPreloaded { library_key: lib_key, albums: data }).await;
                }
            }
            PreloadType::Folders { lib_title } => {
                tracing::debug!("Preloading folders for library: {}", lib_key);
                if let Ok(response) = client.get_library_folders(lib_key_ref).await {
                    let items = FolderService::from_response(&response);
                    let root_column = FolderColumn::new(None, lib_title, items);
                    let folder_state = FolderNavigationState {
                        library_key: lib_key.clone(),
                        columns: vec![root_column],
                        focused_column: 0,
                        loading: false,
                    };
                    tracing::debug!("Folders preloaded successfully");
                    let _ = event_tx.send(Event::FoldersPreloaded { library_key: lib_key, folder_state }).await;
                }
            }
        }
    });
}

/// Preload all library data in background.
pub fn preload_all_library_data(event_tx: &mpsc::Sender<Event>, lib_key: &str, lib_title: &str, client: &PlexClient) {
    preload_data(event_tx, PreloadType::Artists, lib_key, client);
    preload_data(event_tx, PreloadType::Folders { lib_title: lib_title.to_string() }, lib_key, client);
    preload_data(event_tx, PreloadType::Albums, lib_key, client);
    preload_data(event_tx, PreloadType::Genres, lib_key, client);
    preload_data(event_tx, PreloadType::ArtistGenres, lib_key, client);
    preload_data(event_tx, PreloadType::AlbumGenres, lib_key, client);
    preload_data(event_tx, PreloadType::Moods, lib_key, client);
    preload_data(event_tx, PreloadType::Styles, lib_key, client);
    preload_data(event_tx, PreloadType::Stations, lib_key, client);
    preload_data(event_tx, PreloadType::Playlists, lib_key, client);
    preload_data(event_tx, PreloadType::RecentlyAdded, lib_key, client);
    preload_data(event_tx, PreloadType::RecentlyPlayed, lib_key, client);
}

// ============================================================================
// Radio
// ============================================================================

/// Fetch more tracks for the current radio station (non-blocking).
///
/// Spawns the API call in a background task and sends results back
/// via `Event::RadioTracksLoaded`.
pub fn fetch_more_radio_tracks(event_tx: &mpsc::Sender<Event>, state: &mut AppState, client: &PlexClient) {
    if state.radio.fetching {
        return;
    }

    if let Some(ref station) = state.radio.active_station {
        state.radio.fetching = true;

        let event_tx = event_tx.clone();
        let client = client.clone();

        // Special handling for Time Travel Radio
        if station.key.contains("timeTravel") && !state.radio.time_travel_decades.is_empty() {
            if let Some(lib_key) = state.active_library.clone() {
                let decades = state.radio.time_travel_decades.clone();
                let current_index = state.radio.time_travel_index;

                tracing::info!("Time Travel Radio: fetching more tracks starting from decade index {} ({})",
                    current_index % decades.len(),
                    decades.get(current_index % decades.len()).unwrap_or(&"?".to_string()));

                tokio::spawn(async move {
                    match client.fetch_time_travel_tracks_from_index(&lib_key, &decades, current_index).await {
                        Ok(tracks) => {
                            let _ = event_tx.send(Event::RadioTracksLoaded {
                                tracks,
                                time_travel_index: Some(current_index + 3),
                            }).await;
                        }
                        Err(e) => {
                            tracing::warn!("Time Travel Radio: failed to fetch more tracks: {}", e);
                            // Send empty result to clear fetching flag
                            let _ = event_tx.send(Event::RadioTracksLoaded {
                                tracks: vec![],
                                time_travel_index: None,
                            }).await;
                        }
                    }
                });
                return;
            }
        }

        // Standard station fetch
        let station_key = station.key.clone();
        let station_title = station.title.clone();
        tracing::info!("Fetching more tracks for station: {}", station_title);

        tokio::spawn(async move {
            match client.create_station_queue(&station_key).await {
                Ok(tracks) => {
                    let _ = event_tx.send(Event::RadioTracksLoaded {
                        tracks,
                        time_travel_index: None,
                    }).await;
                }
                Err(e) => {
                    tracing::warn!("Failed to fetch more radio tracks: {}", e);
                    let _ = event_tx.send(Event::RadioTracksLoaded {
                        tracks: vec![],
                        time_travel_index: None,
                    }).await;
                }
            }
        });
    } else {
        state.radio.fetching = false;
    }
}

// ============================================================================
// Cache management
// ============================================================================

/// Check if we should save the cache and spawn async save if conditions are met.
pub fn maybe_save_cache_async(event_tx: &mpsc::Sender<Event>, state: &mut AppState) {
    if !state.cache_dirty || state.cache_save_in_progress {
        return;
    }

    let lib_key = match &state.active_library {
        Some(k) => k.clone(),
        None => return,
    };

    let idle_threshold = std::time::Duration::from_secs(30);
    if state.last_input_time.elapsed() < idle_threshold {
        return;
    }

    let save_interval = std::time::Duration::from_secs(120);
    if state.last_cache_save.elapsed() < save_interval {
        return;
    }

    state.cache_save_in_progress = true;
    state.cache_dirty = false;
    state.last_cache_save = std::time::Instant::now();

    use crate::cache::CacheData;
    let mut cache_data = CacheData::new(&lib_key);
    cache_data.artists = state.artists.clone();
    cache_data.albums = state.albums.clone();
    cache_data.playlists = state.playlists.clone();
    if let Some(ref folder_state) = state.folder_state {
        if folder_state.library_key == lib_key {
            if let Some(root_col) = folder_state.columns.first() {
                cache_data.root_folders = root_col.unshuffled_items().to_vec();
            }
        } else {
            tracing::debug!("Not saving folder_state (periodic) - belongs to different library (expected {}, got {})",
                lib_key, folder_state.library_key);
        }
    }
    cache_data.folder_contents = state.folder_contents_cache.clone();
    cache_data.genres = state.genres.clone();
    cache_data.artist_genres = state.artist_genres.clone();
    cache_data.album_genres = state.album_genres.clone();
    cache_data.moods = state.moods.clone();
    cache_data.styles = state.styles.clone();
    cache_data.stations = state.stations.clone();
    cache_data.recently_added_albums = state.recently_added_albums.clone();
    cache_data.recently_played_albums = state.recently_played_albums.clone();
    cache_data.playlist_tracks = state.playlist_tracks_cache.clone();

    let event_tx = event_tx.clone();
    tokio::spawn(async move {
        if let Some(cache) = LibraryCache::new() {
            match serde_json::to_string(&cache_data) {
                Ok(contents) => {
                    let path = cache.cache_path(&lib_key);
                    let temp_path = path.with_extension("json.tmp");

                    match tokio::fs::write(&temp_path, &contents).await {
                        Ok(_) => {
                            if let Err(e) = tokio::fs::rename(&temp_path, &path).await {
                                tracing::warn!("Failed to rename cache file: {}", e);
                                let _ = tokio::fs::remove_file(&temp_path).await;
                            } else {
                                tracing::debug!("Cache saved (periodic): {:?}", path);
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to write cache temp file: {}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to serialize cache: {}", e);
                }
            }
        }

        let _ = event_tx.send(Event::CacheSaved).await;
    });
}

// ============================================================================
// View refresh
// ============================================================================

/// Refresh the current view's category and return actions.
pub fn refresh_current_view(state: &mut AppState) -> Vec<Action> {
    use crate::app::state::{RefreshCategory, ArtistViewMode, PlaylistsMode, GenreContentType};

    // Special handling for Folders
    if state.view == View::Browse && state.browse_category == BrowseCategory::Folders {
        let subfolder_key = state.folder_state.as_ref().and_then(|folder_state| {
            if folder_state.focused_column > 0 {
                folder_state.columns.get(folder_state.focused_column)
                    .and_then(|col| col.key.clone())
            } else {
                None
            }
        });

        if let Some(folder_key) = subfolder_key {
            state.set_status("Refreshing folder...".to_string());
            return vec![Action::RefreshSubfolder(folder_key)];
        }
    }

    let category = match state.view {
        View::Browse => match state.browse_category {
            BrowseCategory::Artists => match state.artist_view_mode {
                ArtistViewMode::Artist => Some(RefreshCategory::Artists),
                ArtistViewMode::AlbumArtist => Some(RefreshCategory::AlbumArtists),
                ArtistViewMode::Album => Some(RefreshCategory::Albums),
            },
            BrowseCategory::Playlists => match state.playlists_mode {
                PlaylistsMode::All => Some(RefreshCategory::Playlists),
                PlaylistsMode::Stations => Some(RefreshCategory::Stations),
                PlaylistsMode::RecentlyAdded => Some(RefreshCategory::RecentlyAdded),
                PlaylistsMode::RecentlyPlayed => Some(RefreshCategory::RecentlyPlayed),
            },
            BrowseCategory::Genres => match state.genre_content_type {
                GenreContentType::Genres => Some(RefreshCategory::Genres),
                GenreContentType::ArtistGenres => Some(RefreshCategory::ArtistGenres),
                GenreContentType::AlbumGenres => Some(RefreshCategory::AlbumGenres),
                GenreContentType::Moods => Some(RefreshCategory::Moods),
                GenreContentType::Styles => Some(RefreshCategory::Styles),
                GenreContentType::Stations => Some(RefreshCategory::Stations),
            },
            BrowseCategory::Folders => Some(RefreshCategory::Folders),
        },
        _ => None,
    };

    if let Some(cat) = category {
        if !state.background_refresh_in_progress.contains(&cat) {
            state.set_status(format!("Refreshing {}...", cat.display_name()));
            return vec![Action::RefreshCategory(cat)];
        }
    }
    vec![]
}

/// Check if the user is currently viewing a specific category.
pub fn is_viewing_category(category: &crate::app::state::RefreshCategory, state: &AppState) -> bool {
    use crate::app::state::{RefreshCategory, ArtistViewMode, PlaylistsMode, GenreContentType};

    if state.view != View::Browse {
        return false;
    }

    match (state.browse_category, category) {
        (BrowseCategory::Artists, RefreshCategory::Artists) => {
            matches!(state.artist_view_mode, ArtistViewMode::Artist)
        }
        (BrowseCategory::Artists, RefreshCategory::AlbumArtists) => {
            matches!(state.artist_view_mode, ArtistViewMode::AlbumArtist)
        }
        (BrowseCategory::Artists, RefreshCategory::Albums) => {
            matches!(state.artist_view_mode, ArtistViewMode::Album)
        }
        (BrowseCategory::Playlists, RefreshCategory::Playlists) => {
            matches!(state.playlists_mode, PlaylistsMode::All)
        }
        (BrowseCategory::Playlists, RefreshCategory::RecentlyAdded) => {
            matches!(state.playlists_mode, PlaylistsMode::RecentlyAdded)
        }
        (BrowseCategory::Playlists, RefreshCategory::RecentlyPlayed) => {
            matches!(state.playlists_mode, PlaylistsMode::RecentlyPlayed)
        }
        (BrowseCategory::Playlists, RefreshCategory::Stations) => {
            matches!(state.playlists_mode, PlaylistsMode::Stations)
        }
        (BrowseCategory::Genres, RefreshCategory::Genres) => {
            matches!(state.genre_content_type, GenreContentType::Genres)
        }
        (BrowseCategory::Genres, RefreshCategory::ArtistGenres) => {
            matches!(state.genre_content_type, GenreContentType::ArtistGenres)
        }
        (BrowseCategory::Genres, RefreshCategory::AlbumGenres) => {
            matches!(state.genre_content_type, GenreContentType::AlbumGenres)
        }
        (BrowseCategory::Genres, RefreshCategory::Moods) => {
            matches!(state.genre_content_type, GenreContentType::Moods)
        }
        (BrowseCategory::Genres, RefreshCategory::Styles) => {
            matches!(state.genre_content_type, GenreContentType::Styles)
        }
        (BrowseCategory::Genres, RefreshCategory::Stations) => {
            matches!(state.genre_content_type, GenreContentType::Stations)
        }
        (BrowseCategory::Folders, RefreshCategory::Folders) => true,
        _ => false,
    }
}

/// Check for very stale cache and refresh in background when user is idle.
pub fn maybe_refresh_very_stale(event_tx: &mpsc::Sender<Event>, state: &mut AppState, client: &PlexClient) {
    use crate::app::state::RefreshCategory;

    if state.last_input_time.elapsed() < Duration::from_secs(120) {
        return;
    }

    if !state.background_refresh_in_progress.is_empty() {
        return;
    }

    let lib_key = match &state.active_library {
        Some(k) => k.clone(),
        None => return,
    };

    for category in RefreshCategory::all() {
        if is_viewing_category(category, state) {
            continue;
        }

        if is_category_very_stale(*category, state) {
            tracing::info!("Very stale background refresh: {:?}", category);
            spawn_category_refresh(event_tx, *category, &lib_key, state, client);
            break;
        }
    }
}

/// Check if a category's data is very stale (32+ days old).
pub fn is_category_very_stale(category: crate::app::state::RefreshCategory, state: &AppState) -> bool {
    use crate::app::state::RefreshCategory;

    let lib_key = match &state.active_library {
        Some(k) => k,
        None => return false,
    };

    if let Some(cache) = LibraryCache::new() {
        if let Some(data) = cache.load(lib_key) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);

            let age = now.saturating_sub(data.timestamp);
            let very_stale_threshold = crate::plex::VERY_STALE_CACHE_SECS;

            let has_data = match category {
                RefreshCategory::Artists | RefreshCategory::AlbumArtists => !data.artists.is_empty(),
                RefreshCategory::Albums => !data.albums.is_empty(),
                RefreshCategory::Playlists => !data.playlists.is_empty(),
                RefreshCategory::RecentlyAdded => !data.recently_added_albums.is_empty(),
                RefreshCategory::RecentlyPlayed => !data.recently_played_albums.is_empty(),
                RefreshCategory::Genres => !data.genres.is_empty(),
                RefreshCategory::ArtistGenres => !data.artist_genres.is_empty(),
                RefreshCategory::AlbumGenres => !data.album_genres.is_empty(),
                RefreshCategory::Moods => !data.moods.is_empty(),
                RefreshCategory::Styles => !data.styles.is_empty(),
                RefreshCategory::Stations => !data.stations.is_empty(),
                RefreshCategory::Folders => !data.root_folders.is_empty(),
            };

            return has_data && age > very_stale_threshold;
        }
    }
    false
}

/// Spawn a background refresh task for a category.
pub fn spawn_category_refresh(
    event_tx: &mpsc::Sender<Event>,
    category: crate::app::state::RefreshCategory,
    lib_key: &str,
    state: &mut AppState,
    client: &PlexClient,
) {
    use crate::app::state::RefreshCategory;

    state.background_refresh_in_progress.insert(category);

    let old_count = match category {
        RefreshCategory::Artists | RefreshCategory::AlbumArtists => state.artists.len(),
        RefreshCategory::Albums => state.albums.len(),
        RefreshCategory::Playlists => state.playlists.len(),
        RefreshCategory::RecentlyAdded => state.recently_added_albums.len(),
        RefreshCategory::RecentlyPlayed => state.recently_played_albums.len(),
        RefreshCategory::Genres => state.genres.len(),
        RefreshCategory::ArtistGenres => state.artist_genres.len(),
        RefreshCategory::AlbumGenres => state.album_genres.len(),
        RefreshCategory::Moods => state.moods.len(),
        RefreshCategory::Styles => state.styles.len(),
        RefreshCategory::Stations => state.stations.len(),
        RefreshCategory::Folders => state.folder_state.as_ref().map(|f| f.columns.first().map(|c| c.items.len()).unwrap_or(0)).unwrap_or(0),
    };

    let event_tx = event_tx.clone();
    let lib_key = lib_key.to_string();
    let client = client.clone();

    tokio::spawn(async move {
        let changed = match category {
            RefreshCategory::Artists | RefreshCategory::AlbumArtists => {
                match client.get_artists(&lib_key).await {
                    Ok(artists) => {
                        let new_count = artists.len();
                        let _ = event_tx.send(Event::ArtistsPreloaded { library_key: lib_key.clone(), artists }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh artists: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::Albums => {
                match client.get_albums(&lib_key).await {
                    Ok(albums) => {
                        let new_count = albums.len();
                        let _ = event_tx.send(Event::AlbumsPreloaded { library_key: lib_key.clone(), albums }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh albums: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::Playlists => {
                match client.get_playlists().await {
                    Ok(playlists) => {
                        let new_count = playlists.len();
                        let _ = event_tx.send(Event::PlaylistsPreloaded { library_key: lib_key.clone(), playlists }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh playlists: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::RecentlyAdded => {
                match client.get_recently_added_albums(&lib_key, 50).await {
                    Ok(albums) => {
                        let new_count = albums.len();
                        let _ = event_tx.send(Event::RecentlyAddedPreloaded { library_key: lib_key.clone(), albums }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh recently added: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::RecentlyPlayed => {
                match client.get_recently_played_albums(&lib_key, 50).await {
                    Ok(albums) => {
                        let new_count = albums.len();
                        let _ = event_tx.send(Event::RecentlyPlayedPreloaded { library_key: lib_key.clone(), albums }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh recently played: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::Genres => {
                match client.get_genres(&lib_key).await {
                    Ok(genres) => {
                        let new_count = genres.len();
                        let _ = event_tx.send(Event::GenresPreloaded { library_key: lib_key.clone(), genres }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh genres: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::ArtistGenres => {
                match client.get_artist_genres(&lib_key).await {
                    Ok(genres) => {
                        let new_count = genres.len();
                        let _ = event_tx.send(Event::ArtistGenresPreloaded { library_key: lib_key.clone(), genres }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh artist genres: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::AlbumGenres => {
                match client.get_album_genres(&lib_key).await {
                    Ok(genres) => {
                        let new_count = genres.len();
                        let _ = event_tx.send(Event::AlbumGenresPreloaded { library_key: lib_key.clone(), genres }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh album genres: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::Moods => {
                match client.get_moods(&lib_key).await {
                    Ok(moods) => {
                        let new_count = moods.len();
                        let _ = event_tx.send(Event::MoodsPreloaded { library_key: lib_key.clone(), moods }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh moods: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::Styles => {
                match client.get_styles(&lib_key).await {
                    Ok(styles) => {
                        let new_count = styles.len();
                        let _ = event_tx.send(Event::StylesPreloaded { library_key: lib_key.clone(), styles }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh styles: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::Stations => {
                match client.get_stations(&lib_key).await {
                    Ok(stations) => {
                        let new_count = stations.len();
                        let _ = event_tx.send(Event::StationsPreloaded { library_key: lib_key.clone(), stations }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh stations: {}", e);
                        false
                    }
                }
            }
            RefreshCategory::Folders => {
                use crate::services::{FolderColumn, FolderNavigationState, FolderService};
                match client.get_library_folders(&lib_key).await {
                    Ok(response) => {
                        let items = FolderService::from_response(&response);
                        let new_count = items.len();
                        let root_column = FolderColumn::new(None, "Music".to_string(), items);
                        let folder_state = FolderNavigationState {
                            library_key: lib_key.clone(),
                            columns: vec![root_column],
                            focused_column: 0,
                            loading: false,
                        };
                        let _ = event_tx.send(Event::FoldersPreloaded { library_key: lib_key.clone(), folder_state }).await;
                        new_count != old_count
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh folders: {}", e);
                        false
                    }
                }
            }
        };

        let _ = event_tx.send(Event::CacheRefreshCompleted { category, changed }).await;
    });
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sort_key_basic() {
        assert_eq!(sort_key("Alice"), "alice");
        assert_eq!(sort_key("The Beatles"), "beatles");
        assert_eq!(sort_key("Zeppelin"), "zeppelin");
    }

    #[test]
    fn test_sort_key_the_prefix_only() {
        assert_eq!(sort_key("Theater"), "theater");
        assert_eq!(sort_key("The "), "");
        assert_eq!(sort_key("The Band"), "band");
    }

    #[test]
    fn test_sort_key_no_last_name_parsing() {
        assert_eq!(sort_key("John Smith"), "john smith");
    }

    #[test]
    fn test_calc_scroll_offset() {
        assert_eq!(calc_scroll_offset(0, 10, 100), 0);
        assert_eq!(calc_scroll_offset(50, 10, 100), 45);
        assert_eq!(calc_scroll_offset(95, 10, 100), 90);
        assert_eq!(calc_scroll_offset(0, 0, 100), 0);
        assert_eq!(calc_scroll_offset(0, 10, 0), 0);
    }
}
