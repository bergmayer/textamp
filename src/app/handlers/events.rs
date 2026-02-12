//! Event result handlers.
//!
//! Processes async event results (auth, data loading, playback, cache, etc.)

use crate::app::{Action, AppState, Event};
use crate::app::state::{
    BrowseCategory, BrowseItem, ConnectionState, PlayStatus, PlaybackMode,
    SearchSection, View,
};
use crate::api::{PlexAuth, PlexClient};
use super::helpers;
use std::time::Duration;
use tokio::sync::mpsc;

/// Handle non-input events (async results, timers, etc.) and return actions to dispatch.
pub fn handle_app_event(
    event: Event,
    state: &mut AppState,
    client: &mut PlexClient,
    event_tx: &mpsc::Sender<Event>,
) -> Vec<Action> {
    match event {
        Event::AuthSuccess { token, username, server_url, servers, client_identifier, has_plex_pass } => {
            tracing::info!("Authenticated as: {} ({} servers available)", username, servers.len());
            tracing::info!("AuthSuccess server_url: {}", server_url);
            tracing::info!("AuthSuccess client_identifier: {}", client_identifier);
            tracing::info!("PlexClient BEFORE update - client_id: {}, server: {:?}",
                client.client_identifier(), client.server_url());
            client.set_auth_token(token);
            client.set_server(server_url.clone());
            client.set_client_identifier(client_identifier);
            tracing::info!("PlexClient AFTER update - client_id: {}, server: {:?}",
                client.client_identifier(), client.server_url());

            // Persist server info for future restarts
            // Find the server that owns this URL to get its identifier and name
            if let Some(server) = servers.iter().find(|s| {
                s.connections.iter().any(|c| c.uri == server_url)
            }) {
                let server_info = crate::plex::ServerInfo {
                    url: server_url.clone(),
                    identifier: server.client_identifier.clone(),
                    name: server.name.clone(),
                };
                if let Err(e) = PlexAuth::update_server_info(&server_info) {
                    tracing::warn!("Failed to persist server info: {}", e);
                }
            }

            state.available_servers = servers.clone();
            state.connected_server_url = Some(server_url.clone());
            state.connection = ConnectionState::Connected { username: username.clone(), has_plex_pass };
            state.settings_state.discovering_servers = false;
            state.settings_state.username_input = username;
            state.settings_state.password_input.clear(); // Never keep password in memory
            state.settings_state.signing_in = false;
            if state.view != View::Settings {
                state.view = View::Browse;
            }

            // Track which server we're connected to
            if let Some(server) = servers.iter().find(|s| {
                s.connections.iter().any(|c| c.uri == server_url)
            }) {
                state.active_server_id = Some(server.client_identifier.clone());
            }

            // Prune artwork cache by size only (no TTL — warm cache serves stale entries)
            let artwork_cache = crate::plex::ArtworkCache::default();
            artwork_cache.prune_to_size(1024 * 1024 * 1024);

            // Compute artwork cache stats in background
            {
                let event_tx = event_tx.clone();
                tokio::task::spawn_blocking(move || {
                    let cache = crate::plex::ArtworkCache::default();
                    let (count, total_bytes) = cache.stats();
                    let _ = event_tx.blocking_send(Event::ArtworkCacheStats { count, total_bytes });
                });
            }

            // Fetch libraries from other servers in background (for multi-server support)
            if servers.len() > 1 {
                let token_str = client.token().unwrap_or("").to_string();
                let client_id = client.client_identifier().to_string();
                let active_server_id = state.active_server_id.clone();

                for server in servers.iter() {
                    // Skip the server we're already connected to
                    if Some(&server.client_identifier) == active_server_id.as_ref() {
                        continue;
                    }

                    let server_clone = server.clone();
                    let token = token_str.clone();
                    let cid = client_id.clone();
                    let tx = event_tx.clone();

                    tokio::spawn(async move {
                        // Find a working connection to this server
                        if let Some(url) = helpers::find_working_connection(&server_clone, &token, &cid).await {
                            let other_client = crate::api::PlexClient::new_with_url(&url, Some(&token), &cid);
                            match other_client.get_libraries().await {
                                Ok(libs) => {
                                    let music_libs: Vec<_> = libs.into_iter().filter(|l| l.is_music()).collect();
                                    if !music_libs.is_empty() {
                                        let _ = tx.send(Event::ServerLibrariesLoaded {
                                            server_identifier: server_clone.client_identifier.clone(),
                                            server_name: server_clone.name.clone(),
                                            libraries: music_libs,
                                        }).await;
                                    }
                                }
                                Err(e) => {
                                    tracing::debug!("Failed to load libraries from {}: {}", server_clone.name, e);
                                }
                            }
                        }
                    });
                }
            }

            vec![Action::LoadInitialData]
        }
        Event::ServersDiscovered(servers) => {
            tracing::info!("Discovered {} servers", servers.len());
            state.available_servers = servers;
            state.settings_state.discovering_servers = false;
            vec![]
        }
        Event::ServerDiscoveryFailed(error) => {
            tracing::warn!("Server discovery failed: {}", error);
            state.settings_state.discovering_servers = false;
            state.set_error(format!("Server discovery failed: {}", error));
            vec![]
        }
        Event::ServerConnectionSucceeded { server_name, url } => {
            tracing::info!("Server connection succeeded: {} at {}", server_name, url);
            client.set_server(url.clone());
            state.connected_server_url = Some(url.clone());

            // Update active server tracking
            if let Some(server) = state.available_servers.iter().find(|s| {
                s.connections.iter().any(|c| c.uri == url) || s.name == server_name
            }) {
                state.active_server_id = Some(server.client_identifier.clone());
            }

            // Persist the updated URL so future startups use the working URL
            if let Some(server) = state.available_servers.iter().find(|s| {
                s.connections.iter().any(|c| c.uri == url)
            }) {
                let server_info = crate::plex::ServerInfo {
                    url: url.clone(),
                    identifier: server.client_identifier.clone(),
                    name: server.name.clone(),
                };
                if let Err(e) = PlexAuth::update_server_info(&server_info) {
                    tracing::warn!("Failed to persist updated server URL: {}", e);
                }
            }

            state.set_status(format!("Connected to {}", server_name));

            // If libraries haven't loaded yet (stale URL recovery), retry
            if state.libraries.is_empty() {
                tracing::info!("Retrying data load with new server URL");
                return vec![Action::LoadInitialData];
            }
            vec![]
        }
        Event::ServerConnectionFailed { server_name } => {
            tracing::warn!("All connection tests failed for server {}", server_name);
            state.set_error(format!("Could not connect to {} - all connections failed", server_name));
            vec![]
        }
        Event::AuthFailed(msg) => {
            use crate::app::state::AuthStep;
            tracing::error!("Auth failed: {}", msg);
            state.connection = ConnectionState::Error(msg.clone());
            state.settings_state.discovering_servers = false;
            state.settings_state.password_input.clear(); // Clear password on failure
            state.auth_state.step = AuthStep::Login;
            state.auth_state.error_message = Some(msg.clone());
            state.set_error(msg);
            vec![]
        }
        Event::AuthShowLogin => {
            use crate::app::state::AuthStep;
            tracing::info!("No stored credentials, showing login form");
            state.connection = ConnectionState::Disconnected;
            state.auth_state.step = AuthStep::Login;
            state.auth_state.field_index = 0;
            state.auth_state.editing = false;
            state.auth_state.error_message = None;
            state.is_fresh_login = true;
            vec![]
        }
        Event::AuthServersReady { token, username, servers, client_identifier, has_plex_pass } => {
            use crate::app::state::AuthStep;
            tracing::info!("Login succeeded, {} servers available", servers.len());
            state.available_servers = servers.clone();

            // Auto-select if only one server, otherwise show selection
            if servers.len() == 1 {
                state.auth_state.step = AuthStep::Connecting;
                // Find working connection URL (tests connectivity)
                let event_tx = event_tx.clone();
                let token_clone = token.clone();
                let username_clone = username.clone();
                let servers_clone = servers.clone();
                let client_id_clone = client_identifier.clone();
                tokio::spawn(async move {
                    if let Some(url) = helpers::find_working_connection_from_servers(&servers_clone, &token_clone, &client_id_clone).await {
                        let _ = event_tx.send(Event::AuthSuccess {
                            token: token_clone,
                            username: username_clone,
                            server_url: url,
                            servers: servers_clone,
                            client_identifier: client_id_clone,
                            has_plex_pass,
                        }).await;
                    } else {
                        let _ = event_tx.send(Event::AuthFailed(
                            "Could not connect to server - all connection attempts failed".to_string()
                        )).await;
                    }
                });
            } else {
                // Multiple servers - let user choose
                state.view = View::Auth;
                state.auth_state.step = AuthStep::ServerSelect;
                state.auth_state.server_index = 0;
                state.settings_state.discovering_servers = false;
                // Store token temporarily for when server is selected
                state.settings_state.username_input = username;
                // Note: We need to pass the token through - store in a temp location
                // We'll use the client's token holder for this
                client.set_auth_token(token);
                client.set_client_identifier(client_identifier);
                state.auth_state.has_plex_pass = has_plex_pass;
            }
            vec![]
        }
        Event::AuthLoginFailed(msg) => {
            use crate::app::state::AuthStep;
            tracing::error!("Login failed: {}", msg);
            state.auth_state.step = AuthStep::Login;
            state.auth_state.error_message = Some(msg);
            state.auth_state.password_input.clear();
            vec![]
        }
        Event::ServerLibrariesLoaded { server_identifier, server_name, libraries } => {
            tracing::info!("Loaded {} music libraries from server {}", libraries.len(), server_name);
            // Add/update entry for this server
            if let Some(entry) = state.all_server_libraries.iter_mut()
                .find(|(id, _, _)| *id == server_identifier)
            {
                entry.2 = libraries;
            } else {
                state.all_server_libraries.push((server_identifier, server_name, libraries));
            }
            vec![]
        }
        Event::LibrariesLoaded(libs) => {
            tracing::info!("LibrariesLoaded: received {} libraries", libs.len());
            state.libraries = libs.into_iter().filter(|l| l.is_music()).collect();
            tracing::info!("After filtering: {} music libraries", state.libraries.len());

            // Update all_server_libraries for the current server
            if let Some(ref server_id) = state.active_server_id {
                let server_name = state.available_servers.iter()
                    .find(|s| &s.client_identifier == server_id)
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| "Unknown".to_string());
                if let Some(entry) = state.all_server_libraries.iter_mut()
                    .find(|(id, _, _)| id == server_id)
                {
                    entry.2 = state.libraries.clone();
                } else {
                    state.all_server_libraries.push((server_id.clone(), server_name, state.libraries.clone()));
                }
            }

            if state.libraries.is_empty() {
                tracing::warn!("No music libraries found after filtering!");
                return vec![];
            }

            // Check if active library is still valid
            let active_valid = state.active_library.as_ref()
                .map(|key| state.libraries.iter().any(|l| l.key == *key))
                .unwrap_or(false);

            if active_valid {
                // Fix folder root column title (was placeholder from cache load)
                if let Some(ref lib_key) = state.active_library.clone() {
                    if let Some(lib) = state.libraries.iter().find(|l| l.key == *lib_key) {
                        if let Some(ref mut fs) = state.folder_state {
                            if let Some(root_col) = fs.columns.first_mut() {
                                root_col.title = lib.title.clone();
                            }
                        }
                    }
                }

                // Cache was loaded and fresh — no API refresh needed
                if !state.artists.is_empty() {
                    tracing::info!("Library already loaded from cache, skipping reload");
                    return vec![];
                }

                // Active library valid but no cached data - start full preload
                let lib_key = state.active_library.clone().unwrap();
                let lib_title = state.libraries.iter()
                    .find(|l| l.key == lib_key)
                    .map(|l| l.title.clone())
                    .unwrap_or_else(|| "Music".to_string());
                tracing::info!("Active library {} has no cached data, starting preloads", lib_key);
                helpers::preload_all_library_data(event_tx, &lib_key, &lib_title, client);
                return vec![];
            }

            // No valid active library - pick one
            if let Some(lib) = state.libraries.first() {
                tracing::info!("Selected music library: {} (key={})", lib.title, lib.key);
                let lib_key = lib.key.clone();
                let lib_title = lib.title.clone();
                state.active_library = Some(lib_key.clone());
                helpers::preload_all_library_data(event_tx, &lib_key, &lib_title, client);
            }
            vec![]
        }
        Event::ArtistsLoaded(mut artists) => {
            // Sort by display title, ignoring "The " prefix
            artists.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
            state.artists = artists;
            state.artists_loading = false;

            // Update artist_nav if we're in Artists category
            if state.browse_category == BrowseCategory::Artists && !state.artists.is_empty() {
                let title = state.artist_view_mode.name();
                let items = crate::app::state::BrowseItem::from_artists(&state.artists);
                state.artist_nav.update_root_items(title, items);
            }
            vec![]
        }
        Event::AlbumsLoaded(mut albums) => {
            // Sort by display title, ignoring "The " prefix
            albums.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
            state.albums = albums;
            state.albums_loading = false;
            vec![]
        }
        Event::PlaylistsLoaded(playlists) => {
            // Preload tracks for playlists that need fetching:
            // - Smart playlists: always re-fetch (they auto-update)
            // - Regular playlists: re-fetch if not cached or stale (>72h)
            let uncached_keys: Vec<String> = playlists.iter()
                .filter(|p| {
                    if p.smart { return true; }
                    match state.playlist_tracks_cache.get(&p.rating_key) {
                        Some(cached) => cached.is_older_than(crate::plex::constants::CACHE_STALE_THRESHOLD_SECS),
                        None => true,
                    }
                })
                .map(|p| p.rating_key.clone())
                .collect();
            if !uncached_keys.is_empty() {
                let tx = event_tx.clone();
                let client_clone = client.clone();
                tokio::spawn(async move {
                    for pk in uncached_keys {
                        if let Ok(tracks) = client_clone.get_playlist_tracks(&pk).await {
                            let _ = tx.send(Event::PlaylistTracksPreloaded {
                                playlist_key: pk, tracks,
                            }).await;
                        }
                    }
                });
            }

            // Update playlist_nav with the playlists list
            let items = crate::app::state::BrowseItem::from_playlists(&playlists);
            state.playlist_nav.update_root_items("playlists", items);
            state.playlists = playlists;
            state.playlists_loading = false;
            vec![]
        }
        Event::TracksLoaded(tracks) => {
            state.selected_album_tracks = tracks;
            state.right_panel_loading = false;
            vec![]
        }
        Event::AlbumTracksLoaded(tracks) => {
            state.selected_album_tracks = tracks;
            state.right_panel_loading = false;
            vec![]
        }
        Event::ArtistAlbumsLoaded(albums) => {
            state.selected_artist_albums = albums;
            state.right_panel_loading = false;
            state.focus = crate::app::state::Focus::Right;

            // Check if we need to auto-select a specific album (e.g., from Similar view)
            if let Some(album_key) = state.pending_album_key.take() {
                // Find the album in the list (+1 offset for "All Tracks" at index 0)
                if let Some(album_idx) = state.selected_artist_albums.iter()
                    .position(|a| a.rating_key == album_key)
                {
                    state.list_state.right_albums_index = album_idx + 1; // +1 for "All Tracks"
                    state.selected_album_title = state.selected_artist_albums[album_idx].title.clone();
                    return vec![Action::LoadAlbumTracks { rating_key: album_key }];
                }
            }
            vec![]
        }
        Event::ArtistAllTracksLoaded(tracks) => {
            state.selected_album_tracks = tracks;
            state.right_panel_loading = false;
            vec![]
        }
        Event::CategoryTracksLoaded(tracks) => {
            if state.selected_album_title.is_empty() {
                if let Some(first) = tracks.first() {
                    state.selected_album_title = first.album_name().to_string();
                }
            }
            state.selected_album_tracks = tracks;
            state.right_panel_loading = false;
            vec![]
        }
        Event::CategoryAlbumsLoaded { albums, status_message } => {
            state.right_panel_mode = crate::app::state::RightPanelMode::CategoryAlbums;
            state.genre_albums = albums;
            state.genre_albums_index = 0;
            state.set_status(status_message);
            state.right_panel_loading = false;
            vec![]
        }
        Event::DataLoadError(msg) => {
            state.set_error(msg);
            state.right_panel_loading = false;
            state.similar_loading = false;
            vec![]
        }
        Event::SimilarAlbumsLoaded(albums) => {
            state.similar_albums = albums;
            state.similar_mode = crate::app::state::SimilarMode::Albums;
            state.similar_loading = false;
            state.list_state.similar_index = 0;
            vec![]
        }
        Event::SimilarTracksLoaded(tracks) => {
            state.similar_tracks = tracks;
            state.similar_mode = crate::app::state::SimilarMode::Tracks;
            state.similar_loading = false;
            state.list_state.similar_index = 0;
            vec![]
        }
        Event::SearchCompleted(results) => {
            // Legacy handler for non-debounced search (e.g., Enter key)
            tracing::info!(
                "Search completed: {} artists, {} albums, {} tracks, {} playlists",
                results.artists.len(),
                results.albums.len(),
                results.tracks.len(),
                results.playlists.len()
            );
            state.list_state.search_item_index = 0;
            if !results.artists.is_empty() {
                state.list_state.search_section = SearchSection::Artists;
            } else if !results.albums.is_empty() {
                state.list_state.search_section = SearchSection::Albums;
            } else if !results.tracks.is_empty() {
                state.list_state.search_section = SearchSection::Tracks;
            }
            state.search_results = Some(results);
            state.search_loading = false;
            vec![]
        }
        Event::GlobalSearchCompleted { version, results } => {
            // Only apply results if version matches (not stale)
            if version == state.global_search_version {
                let mut results = results;

                // Supplement with local albums matching by year (Plex API doesn't search by year)
                let query_lower = state.search_query.to_lowercase();
                if !query_lower.is_empty() {
                    let existing_keys: std::collections::HashSet<String> =
                        results.albums.iter().map(|a| a.rating_key.clone()).collect();
                    let year_matches: Vec<_> = state.albums.iter()
                        .filter(|a| {
                            !existing_keys.contains(&a.rating_key)
                                && a.year.map(|y| y.to_string().contains(&query_lower)).unwrap_or(false)
                        })
                        .cloned()
                        .collect();
                    if !year_matches.is_empty() {
                        tracing::info!("Global search: adding {} year-matched albums", year_matches.len());
                        results.albums.extend(year_matches);
                    }
                }

                tracing::info!(
                    "Global search completed: {} artists, {} albums, {} tracks, {} playlists",
                    results.artists.len(),
                    results.albums.len(),
                    results.tracks.len(),
                    results.playlists.len()
                );
                state.list_state.search_item_index = 0;
                if !results.artists.is_empty() {
                    state.list_state.search_section = SearchSection::Artists;
                } else if !results.albums.is_empty() {
                    state.list_state.search_section = SearchSection::Albums;
                } else if !results.tracks.is_empty() {
                    state.list_state.search_section = SearchSection::Tracks;
                }
                state.search_results = Some(results);
                state.search_loading = false;
            }
            // Stale results are silently ignored
            vec![]
        }
        Event::FilterSearchCompleted { version, results } => {
            // Only apply results if version matches (not stale)
            if version == state.filter_search_version {
                state.filter_results = Some(results);
                state.filter_loading = false;
                state.list_state.search_item_index = 0;
            }
            // Stale results are silently ignored
            vec![]
        }
        Event::ApiError(msg) => {
            state.set_error(msg);
            vec![]
        }
        Event::TrackStarted => {
            state.playback.status = PlayStatus::Playing;
            state.playback.position_ms = 0;
            vec![]
        }
        Event::TrackEnded => {
            // Ignore stale TrackEnded events. The tick loop only sends TrackEnded
            // when status is Playing. If status has since changed (e.g., a new station
            // started and set Buffering/Stopped), this event is from the old track.
            if state.playback.status != PlayStatus::Playing {
                return vec![];
            }

            // Additional guard: ignore if track was just started. The tick loop has
            // a 1-second grace period before sending TrackEnded, so any event arriving
            // within 1 second of playback start is stale (from a previous track).
            let playing_long_enough = state.playback.playback_started_at
                .map(|t| t.elapsed() >= Duration::from_secs(1))
                .unwrap_or(false);
            if !playing_long_enough {
                tracing::debug!("Ignoring stale TrackEnded (track just started)");
                return vec![];
            }

            // Immediately set Stopped to prevent any duplicate TrackEnded events
            // (still in the channel) from triggering another Next action.
            state.playback.status = PlayStatus::Stopped;

            // Report stop to Plex when track ends naturally
            // continuing=true because we're about to play the next track
            if let Some(track) = state.current_track().cloned() {
                // Use track duration as position (track finished)
                let position = track.duration_ms();
                helpers::report_playback_stop_to_plex(&track, position, true, state.plex_session_id.clone(), client);
            }
            vec![Action::Next]
        }
        Event::PlaybackPaused => {
            state.playback.status = PlayStatus::Paused;
            vec![]
        }
        Event::PlaybackResumed => {
            state.playback.status = PlayStatus::Playing;
            vec![]
        }
        Event::PlaybackStopped => {
            state.playback.status = PlayStatus::Stopped;
            state.playback.position_ms = 0;
            vec![]
        }
        Event::PlaybackError(msg) => {
            state.playback.status = PlayStatus::Stopped;
            state.consecutive_playback_errors += 1;

            let track_info = state.current_track()
                .map(|t| format!("{} - {}", t.artist_name(), t.title))
                .unwrap_or_else(|| "unknown".to_string());
            let qi = state.queue_index.unwrap_or(9999);

            // First 5 errors: retry the SAME track with increasing delays
            // (handles cold-start / remote relay warm-up)
            if state.consecutive_playback_errors <= 5 {
                let delays_ms = [500, 1000, 1500, 2000, 2500];
                let delay = delays_ms[(state.consecutive_playback_errors as usize - 1).min(delays_ms.len() - 1)];
                tracing::warn!("Playback error (retry {}/5 for queue[{}] '{}', delay {}ms): {}",
                    state.consecutive_playback_errors, qi, track_info, delay, msg);
                let tx = event_tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    let _ = tx.send(Event::RetryAfterDelay).await;
                });
                return vec![];
            }

            // Errors 6-8: auto-skip to next track
            if state.consecutive_playback_errors <= 8 {
                tracing::warn!("Playback error (skipping queue[{}] '{}', attempt {}/8): {}",
                    qi, track_info, state.consecutive_playback_errors, msg);
                return vec![Action::Next];
            }

            // After 8 consecutive failures, show error to user
            state.consecutive_playback_errors = 0;
            if msg.contains("404") || msg.to_lowercase().contains("not found") {
                state.confirm_dialog = Some(crate::app::state::ConfirmDialog {
                    title: "Track Not Found".to_string(),
                    message: "This track may have been removed. Refresh cache?".to_string(),
                    on_confirm: crate::app::state::ConfirmAction::RefreshCache,
                });
            } else {
                state.set_error("Playback stopped after multiple consecutive errors".to_string());
            }
            vec![]
        }
        Event::RetryAfterDelay => {
            vec![Action::RetryCurrentTrack]
        }
        Event::BufferingStart => {
            state.playback.status = PlayStatus::Buffering;
            vec![]
        }
        Event::BufferingEnd => {
            // Don't reset consecutive_playback_errors here — wait for sustained
            // playback (5s) to confirm the track is actually playing successfully.
            vec![Action::StartPendingPlayback]
        }
        Event::PositionUpdate(pos) => {
            state.playback.position_ms = pos;
            vec![]
        }
        Event::ArtworkLoaded { thumb_path, data } => {
            state.artwork_thumb = Some(thumb_path);
            state.artwork_data = Some(data);
            state.artwork_loading = false;
            vec![]
        }
        Event::ArtworkFailed { thumb_path: _ } => {
            state.artwork_thumb = None;
            state.artwork_data = None;
            state.artwork_loading = false;
            vec![]
        }
        Event::AlbumArtLoaded { key, data } => {
            state.album_art_pending.remove(&key);
            state.album_art_cache.insert(key, data);
            vec![]
        }
        Event::AlbumArtFailed { key } => {
            state.album_art_pending.remove(&key);
            vec![]
        }
        Event::FoldersPreloaded { library_key, folder_state } => {
            // Ignore if this is for a different library (race condition from library switch)
            if state.active_library.as_ref() != Some(&library_key) {
                tracing::debug!("Ignoring stale folders preload for library {}", library_key);
                return vec![];
            }
            // Only set if folders weren't already loaded (user might have navigated there)
            if state.folder_state.is_none() {
                state.folder_state = Some(folder_state);
                tracing::debug!("Folders preloaded and ready");
            }
            vec![]
        }
        Event::SubfoldersPreloaded { library_key, entries, done } => {
            // Ignore if this is for a different library (race condition from library switch)
            if state.active_library.as_ref() != Some(&library_key) {
                tracing::debug!("Ignoring stale subfolder preload for library {}", library_key);
                return vec![];
            }
            // Merge entries into cache
            if !entries.is_empty() {
                for (key, cached_folder) in entries {
                    state.folder_contents_cache.insert(key, cached_folder);
                }
                state.cache_dirty = true;
            }
            if done {
                state.subfolder_preload_active = false;
                tracing::info!("Subfolder preload finished, {} total cached subfolders", state.folder_contents_cache.len());
            }
            vec![]
        }
        Event::SubfolderRefreshed { folder_key, cached_folder } => {
            // Background warm-cache re-fetch completed — update cache and refresh UI
            state.folder_contents_cache.insert(folder_key.clone(), cached_folder.clone());
            state.cache_dirty = true;
            tracing::debug!("Warm subfolder re-fetched: {} ({} items)", folder_key, cached_folder.items.len());

            // Update the currently displayed column if it matches
            if let Some(ref mut folder_state) = state.folder_state {
                for col in folder_state.columns.iter_mut() {
                    if col.key.as_ref() == Some(&folder_key) {
                        let old_selected = col.selected_index;
                        col.items = cached_folder.items;
                        col.selected_index = old_selected.min(col.items.len().saturating_sub(1));
                        break;
                    }
                }
            }
            vec![]
        }
        Event::FolderPathDiscovered { folder_key, path } => {
            tracing::debug!("Path discovered for {}: {}", folder_key, path);
            // Update the displayed column title
            if let Some(ref mut folder_state) = state.folder_state {
                for col in folder_state.columns.iter_mut() {
                    if col.key.as_ref() == Some(&folder_key) && col.title.is_empty() {
                        col.title = path.clone();
                        break;
                    }
                }
                // Backfill parent columns too
                let num_cols = folder_state.columns.len();
                for i in 1..num_cols {
                    if folder_state.columns[i].key.as_ref() == Some(&folder_key) {
                        // This column just got its path — backfill parent if needed
                        let parent_path_pos = path.rfind(|c: char| c == '/' || c == '\\');
                        if let Some(pos) = parent_path_pos {
                            let parent_path = &path[..pos];
                            if !parent_path.is_empty() && folder_state.columns[i - 1].title.is_empty() {
                                folder_state.columns[i - 1].title = parent_path.to_string();
                            }
                        }
                        break;
                    }
                }
            }
            // Update the cache entry's path
            if let Some(cached) = state.folder_contents_cache.get_mut(&folder_key) {
                if cached.path.is_none() {
                    cached.path = Some(path);
                    state.cache_dirty = true;
                }
            }
            vec![]
        }
        Event::ArtworkCacheStats { count, total_bytes } => {
            state.artwork_cache_stats = Some((count, total_bytes));
            vec![]
        }
        Event::ArtistsPreloaded { library_key, mut artists } => {
            // Ignore if this is for a different library (race condition from library switch)
            if state.active_library.as_ref() != Some(&library_key) {
                tracing::debug!("Ignoring stale artists preload for library {}", library_key);
                return vec![];
            }
            if state.artists.is_empty() || !state.artists_loading {
                artists.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
                let count = artists.len();
                state.artists = artists;
                state.artists_total = count as u32;
                tracing::debug!("Artists preloaded: {} items", count);
                // Update Miller columns (preserves drill-down state)
                if !state.artists.is_empty() {
                    let items = crate::app::state::BrowseItem::from_artists(&state.artists);
                    state.artist_nav.update_root_items(state.artist_view_mode.name(), items);
                }
            }
            vec![]
        }
        Event::AlbumsPreloaded { library_key, mut albums } => {
            // Ignore if this is for a different library (race condition from library switch)
            if state.active_library.as_ref() != Some(&library_key) {
                tracing::debug!("Ignoring stale albums preload for library {}", library_key);
                return vec![];
            }
            if state.albums.is_empty() || !state.albums_loading {
                albums.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
                let count = albums.len();
                state.albums = albums;
                state.albums_total = count as u32;
                tracing::debug!("Albums preloaded: {} items", count);
            }
            vec![]
        }
        Event::PlaylistsPreloaded { library_key, playlists } => {
            // Ignore if this is for a different library (race condition from library switch)
            if state.active_library.as_ref() != Some(&library_key) {
                tracing::debug!("Ignoring stale playlists preload for library {}", library_key);
                return vec![];
            }
            if state.playlists.is_empty() || !state.playlists_loading {
                let count = playlists.len();

                // Preload tracks for playlists that need fetching:
                // - Smart playlists: always re-fetch (they auto-update)
                // - Regular playlists: re-fetch if not cached or stale (>72h)
                let uncached_keys: Vec<String> = playlists.iter()
                    .filter(|p| {
                        if p.smart { return true; }
                        match state.playlist_tracks_cache.get(&p.rating_key) {
                            Some(cached) => cached.is_older_than(crate::plex::constants::CACHE_STALE_THRESHOLD_SECS),
                            None => true,
                        }
                    })
                    .map(|p| p.rating_key.clone())
                    .collect();
                if !uncached_keys.is_empty() {
                    let tx = event_tx.clone();
                    let client_clone = client.clone();
                    tokio::spawn(async move {
                        for pk in uncached_keys {
                            if let Ok(tracks) = client_clone.get_playlist_tracks(&pk).await {
                                let _ = tx.send(Event::PlaylistTracksPreloaded {
                                    playlist_key: pk, tracks,
                                }).await;
                            }
                        }
                    });
                }

                state.playlists = playlists;
                tracing::debug!("Playlists preloaded: {} items", count);
                // Update Miller columns
                if !state.playlists.is_empty() {
                    let items = crate::app::state::BrowseItem::from_playlists(&state.playlists);
                    state.playlist_nav.update_root_items(state.playlists_mode.name(), items);
                } else {
                    // Clear nav when library has no playlists (overrides stale cache data)
                    state.playlist_nav = crate::app::state::BrowseNavigationState::new();
                }
            }
            vec![]
        }
        Event::GenresPreloaded { library_key, genres } => {
            // Ignore if this is for a different library (race condition from library switch)
            if state.active_library.as_ref() != Some(&library_key) {
                tracing::debug!("Ignoring stale genres preload for library {}", library_key);
                return vec![];
            }
            if state.genres.is_empty() && !state.genres_loading {
                let count = genres.len();
                state.genres = genres;
                tracing::debug!("Genres preloaded: {} items", count);
                if state.genre_content_type == crate::app::state::GenreContentType::Genres {
                    let items = crate::app::state::BrowseItem::from_genres(&state.genres);
                    state.genre_nav.update_root_items("genres", items);
                }
            }
            vec![]
        }
        Event::ArtistGenresPreloaded { library_key, genres } => {
            // Ignore if this is for a different library (race condition from library switch)
            if state.active_library.as_ref() != Some(&library_key) {
                tracing::debug!("Ignoring stale artist genres preload for library {}", library_key);
                return vec![];
            }
            if state.artist_genres.is_empty() && !state.artist_genres_loading {
                let count = genres.len();
                state.artist_genres = genres;
                tracing::debug!("Artist genres preloaded: {} items", count);
                if state.genre_content_type == crate::app::state::GenreContentType::ArtistGenres {
                    let items = crate::app::state::BrowseItem::from_genres(&state.artist_genres);
                    state.genre_nav.update_root_items("artist genres", items);
                }
            }
            vec![]
        }
        Event::AlbumGenresPreloaded { library_key, genres } => {
            // Ignore if this is for a different library (race condition from library switch)
            if state.active_library.as_ref() != Some(&library_key) {
                tracing::debug!("Ignoring stale album genres preload for library {}", library_key);
                return vec![];
            }
            if state.album_genres.is_empty() && !state.album_genres_loading {
                let count = genres.len();
                state.album_genres = genres;
                tracing::debug!("Album genres preloaded: {} items", count);
                if state.genre_content_type == crate::app::state::GenreContentType::AlbumGenres {
                    let items = crate::app::state::BrowseItem::from_genres(&state.album_genres);
                    state.genre_nav.update_root_items("album genres", items);
                }
            }
            vec![]
        }
        Event::MoodsPreloaded { library_key, moods } => {
            // Ignore if this is for a different library (race condition from library switch)
            if state.active_library.as_ref() != Some(&library_key) {
                tracing::debug!("Ignoring stale moods preload for library {}", library_key);
                return vec![];
            }
            if state.moods.is_empty() && !state.moods_loading {
                let count = moods.len();
                state.moods = moods;
                tracing::debug!("Moods preloaded: {} items", count);
                if state.genre_content_type == crate::app::state::GenreContentType::Moods {
                    let items = crate::app::state::BrowseItem::from_genres(&state.moods);
                    state.genre_nav.update_root_items("moods", items);
                }
            }
            vec![]
        }
        Event::StylesPreloaded { library_key, styles } => {
            // Ignore if this is for a different library (race condition from library switch)
            if state.active_library.as_ref() != Some(&library_key) {
                tracing::debug!("Ignoring stale styles preload for library {}", library_key);
                return vec![];
            }
            if state.styles.is_empty() && !state.styles_loading {
                let count = styles.len();
                state.styles = styles;
                tracing::debug!("Styles preloaded: {} items", count);
                if state.genre_content_type == crate::app::state::GenreContentType::Styles {
                    let items = crate::app::state::BrowseItem::from_genres(&state.styles);
                    state.genre_nav.update_root_items("styles", items);
                }
            }
            vec![]
        }
        Event::StationsPreloaded { library_key, stations } => {
            // Ignore if this is for a different library (race condition from library switch)
            if state.active_library.as_ref() != Some(&library_key) {
                tracing::debug!("Ignoring stale stations preload for library {}", library_key);
                return vec![];
            }
            if state.stations.is_empty() && !state.stations_loading {
                let count = stations.len();
                state.stations = stations.clone();
                tracing::debug!("Stations preloaded: {} items", count);
                // Rebuild station Miller columns
                state.station_nav.columns.clear();
                state.station_nav.columns.push(crate::app::state::StationColumn::new(
                    None,
                    "Stations".to_string(),
                    stations,
                ));
                state.station_nav.focused_column = 0;
            }
            vec![]
        }
        Event::RecentlyAddedPreloaded { library_key, albums } => {
            // Ignore if this is for a different library (race condition from library switch)
            if state.active_library.as_ref() != Some(&library_key) {
                tracing::debug!("Ignoring stale recently added preload for library {}", library_key);
                return vec![];
            }
            if state.recently_added_albums.is_empty() && !state.recently_added_loading {
                let count = albums.len();
                state.recently_added_albums = albums;
                tracing::debug!("Recently added albums preloaded: {} items", count);
            }
            vec![]
        }
        Event::Tick => {
            // Clear expired modifier bars
            if let Some(deadline) = state.alt_bar_until {
                if std::time::Instant::now() >= deadline {
                    state.alt_bar_until = None;
                }
            }
            if let Some(deadline) = state.ctrl_alt_bar_until {
                if std::time::Instant::now() >= deadline {
                    state.ctrl_alt_bar_until = None;
                }
            }

            // Clear expired toasts (5 second display)
            if let Some(show_time) = state.toast_show_time {
                if show_time.elapsed() > Duration::from_secs(5) {
                    state.toast_message = None;
                    state.toast_show_time = None;
                }
            }

            // Clear expired status messages (5 second display)
            if let Some(show_time) = state.status_show_time {
                if show_time.elapsed() > Duration::from_secs(5) {
                    state.clear_status();
                }
            }

            // Periodic playback progress report to Plex (~10 seconds)
            if state.playback.status == PlayStatus::Playing {
                let should_report = state.last_progress_report
                    .map(|t| t.elapsed() > Duration::from_secs(10))
                    .unwrap_or(true);
                if should_report {
                    if let Some(track) = state.current_track().cloned() {
                        helpers::report_playback_progress_to_plex(
                            &track, state.playback.position_ms,
                            state.plex_session_id.clone(), client,
                        );
                        state.last_progress_report = Some(std::time::Instant::now());
                    }
                }
            }

            // Marquee scroll animation (title + subtitle)
            state.marquee.borrow_mut().tick();
            state.marquee_subtitle.borrow_mut().tick();

            // Album art loading: lazy-load only for visible items in the focused column
            // Limit concurrent in-flight requests to avoid overwhelming the Plex transcoder
            if state.album_art_view && state.view == crate::app::state::View::Browse
                && state.album_art_pending.len() < 4
            {
                let nav = match state.browse_category {
                    crate::app::state::BrowseCategory::Artists => &state.artist_nav,
                    crate::app::state::BrowseCategory::Genres => &state.genre_nav,
                    crate::app::state::BrowseCategory::Playlists => &state.playlist_nav,
                    _ => &state.artist_nav, // won't match albums anyway
                };
                if let Some(col) = nav.focused() {
                    let total_items = col.items.len();
                    if total_items > 0 {
                        // Compute visible range using same formula as render_album_art_grid
                        let inner_height = state.terminal_height.saturating_sub(4) as usize;
                        let target_visible = 3usize.max((total_items).min(5));
                        let row_height = if target_visible > 0 { (inner_height / target_visible).max(3) } else { 3 };
                        let visible_rows = if row_height > 0 { (inner_height / row_height).max(1) } else { 1 };
                        let scroll_offset = crate::services::NavigationService::calc_scroll_offset(
                            col.selected_index, visible_rows, total_items,
                        );
                        let end = (scroll_offset + visible_rows).min(total_items);

                        let max_batch = 4usize.saturating_sub(state.album_art_pending.len());
                        let mut to_load: Vec<(String, String)> = Vec::new();
                        for item in &col.items[scroll_offset..end] {
                            if to_load.len() >= max_batch { break; }
                            match item {
                                BrowseItem::Album { key, thumb: Some(thumb), .. } => {
                                    if !state.album_art_cache.contains_key(key)
                                        && !state.album_art_pending.contains(key)
                                    {
                                        to_load.push((key.clone(), thumb.clone()));
                                    }
                                }
                                BrowseItem::AllTracks { artist_key, thumb: Some(thumb), .. } => {
                                    if !state.album_art_cache.contains_key(artist_key)
                                        && !state.album_art_pending.contains(artist_key)
                                    {
                                        to_load.push((artist_key.clone(), thumb.clone()));
                                    }
                                }
                                _ => {}
                            }
                        }
                        if !to_load.is_empty() {
                            return vec![Action::LoadAlbumArt(to_load)];
                        }
                    }
                }
            }

            // Periodic cache save: save if dirty, idle for 30+ seconds, and 2+ minutes since last save
            helpers::maybe_save_cache_async(event_tx, state);

            // (Per-category staleness checks are now done on view navigation, not on tick)

            vec![]
        }
        Event::LibraryCacheLoaded { library_key, cached } => {
            state.library_loading = false;

            // Ignore if user switched to a different library while loading
            if state.active_library.as_ref() != Some(&library_key) {
                tracing::debug!("Ignoring cache load for {} (active library changed)", library_key);
                return vec![];
            }

            // Validate cache belongs to this library
            if cached.library_key != library_key {
                tracing::warn!("Cache library_key mismatch: expected {}, got {} - ignoring cache",
                    library_key, cached.library_key);
                return vec![];
            }

            tracing::info!("Library switch: loaded from cache: {} artists, {} albums, {} folders",
                cached.artists.len(), cached.albums.len(), cached.root_folders.len());

            // Load per-category timestamps (with backward compat migration)
            if !cached.category_timestamps.is_empty() {
                for (key, ts) in &cached.category_timestamps {
                    if let Some(cat) = crate::app::state::RefreshCategory::from_cache_key(key) {
                        state.category_timestamps.insert(cat, *ts);
                    }
                }
            } else {
                // Migrate from legacy shared timestamps
                let lib_ts = cached.timestamp;
                let playlist_ts = if cached.playlist_timestamp > 0 { cached.playlist_timestamp } else { lib_ts };
                if lib_ts > 0 {
                    use crate::app::state::RefreshCategory;
                    for cat in RefreshCategory::all() {
                        let ts = if cat.is_playlist_group() { playlist_ts } else { lib_ts };
                        state.category_timestamps.insert(*cat, ts);
                    }
                }
            }

            // Find library name for folder root column
            let lib_name = state.libraries.iter()
                .find(|l| l.key == library_key)
                .map(|l| l.title.clone())
                .unwrap_or_else(|| library_key.clone());

            // Core library data - IMPORTANT: Always re-sort after loading from cache
            if !cached.artists.is_empty() {
                state.artists = cached.artists;
                state.artists.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
                state.artists_total = state.artists.len() as u32;
                let items = BrowseItem::from_artists(&state.artists);
                state.artist_nav.reset(state.artist_view_mode.name(), items);
            }
            if !cached.albums.is_empty() {
                state.albums = cached.albums;
                state.albums.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
                state.albums_total = state.albums.len() as u32;
            }
            if !cached.playlists.is_empty() {
                state.playlists = cached.playlists.clone();
                let items = BrowseItem::from_playlists(&state.playlists);
                state.playlist_nav.reset("playlists", items);
            }
            if !cached.playlist_tracks.is_empty() {
                state.playlist_tracks_cache = cached.playlist_tracks;
                tracing::debug!("Loaded {} cached playlist track lists", state.playlist_tracks_cache.len());
            }

            // Folders
            if !cached.root_folders.is_empty() {
                use crate::services::{FolderColumn, FolderNavigationState};
                let root_column = FolderColumn::new(None, lib_name, cached.root_folders);
                state.folder_state = Some(FolderNavigationState::with_root(library_key.clone(), root_column));
            }
            if !cached.folder_contents.is_empty() {
                state.folder_contents_cache = cached.folder_contents;
                // Stale entries are kept as a warm cache; the subfolder preload
                // crawl will re-fetch and overwrite them incrementally.
                tracing::debug!("Library switch: loaded {} cached subfolders", state.folder_contents_cache.len());
            } else {
                state.folder_contents_cache.clear();
            }

            // Genres, artist genres, album genres, moods, styles
            if !cached.genres.is_empty() {
                state.genres = cached.genres;
                if state.genre_content_type == crate::app::state::GenreContentType::Genres {
                    let items = BrowseItem::from_genres(&state.genres);
                    state.genre_nav.reset("genres", items);
                }
            }
            if !cached.artist_genres.is_empty() {
                state.artist_genres = cached.artist_genres;
                if state.genre_content_type == crate::app::state::GenreContentType::ArtistGenres {
                    let items = BrowseItem::from_genres(&state.artist_genres);
                    state.genre_nav.reset("artist genres", items);
                }
            }
            if !cached.album_genres.is_empty() {
                state.album_genres = cached.album_genres;
                if state.genre_content_type == crate::app::state::GenreContentType::AlbumGenres {
                    let items = BrowseItem::from_genres(&state.album_genres);
                    state.genre_nav.reset("album genres", items);
                }
            }
            if !cached.moods.is_empty() {
                state.moods = cached.moods;
                if state.genre_content_type == crate::app::state::GenreContentType::Moods {
                    let items = BrowseItem::from_genres(&state.moods);
                    state.genre_nav.reset("moods", items);
                }
            }
            if !cached.styles.is_empty() {
                state.styles = cached.styles;
                if state.genre_content_type == crate::app::state::GenreContentType::Styles {
                    let items = BrowseItem::from_genres(&state.styles);
                    state.genre_nav.reset("styles", items);
                }
            }

            // Stations
            if !cached.stations.is_empty() {
                state.stations = cached.stations.clone();
                state.station_nav.columns.clear();
                state.station_nav.columns.push(crate::app::state::StationColumn::new(
                    None,
                    "Stations".to_string(),
                    cached.stations,
                ));
                state.station_nav.focused_column = 0;
            }

            // Recent content
            if !cached.recently_added_albums.is_empty() {
                state.recently_added_albums = cached.recently_added_albums;
            }
            vec![]
        }
        Event::LibraryCacheLoadFailed { library_key } => {
            state.library_loading = false;
            if state.active_library.as_ref() == Some(&library_key) {
                tracing::debug!("No cache for library {} - waiting for API preload", library_key);
            }
            vec![]
        }
        Event::CacheSaved => {
            state.cache_save_in_progress = false;
            tracing::debug!("Periodic cache save completed");
            vec![]
        }
        Event::CacheRefreshCompleted { category, changed } => {
            state.background_refresh_in_progress.remove(&category);
            state.cache_dirty = true;
            // Data was refreshed from the server — update the per-category timestamp
            let now = crate::cache::CacheData::now();
            state.category_timestamps.insert(category, now);

            // Clear the "Refreshing X..." status message if it matches this category
            let refresh_msg = format!("Refreshing {}...", category.display_name());
            if state.status_message.as_ref() == Some(&refresh_msg) {
                state.clear_status();
            }

            if changed && helpers::is_viewing_category(&category, state) {
                state.set_toast(format!("{} updated", category.display_name()));
            }
            vec![]
        }
        Event::Mouse(mouse_event) => {
            super::mouse_input::handle_mouse(mouse_event, state)
        }
        Event::WaveformGenerated { track_key, data } => {
            if state.waveform.track_key.as_ref() == Some(&track_key) {
                state.waveform.data = Some(data);
                state.waveform.generating = false;
                state.waveform.error = None;
                tracing::debug!("Waveform generated for track: {}", track_key);
            }
            vec![]
        }
        Event::WaveformFailed { track_key, error } => {
            if state.waveform.track_key.as_ref() == Some(&track_key) {
                if state.waveform.retry_count < 3 {
                    // Silent retry: keep generating=true so UI shows "Generating..."
                    state.waveform.retry_count += 1;
                    let retry_num = state.waveform.retry_count;
                    let tx = event_tx.clone();
                    let tk = track_key.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(2 * retry_num as u64)).await;
                        let _ = tx.send(Event::WaveformRetry(tk)).await;
                    });
                    tracing::info!("Waveform retry {}/3 for {}: {}", retry_num, track_key, error);
                } else {
                    // Max retries exhausted — show error
                    state.waveform.error = Some(error.clone());
                    state.waveform.generating = false;
                    tracing::warn!("Waveform failed after 3 retries for {}: {}", track_key, error);
                }
            }
            vec![]
        }
        Event::WaveformCacheHit { track_key, data } => {
            if state.waveform.track_key.as_ref() == Some(&track_key) {
                state.waveform.data = Some(data);
                state.waveform.generating = false;
                state.waveform.error = None;
                tracing::debug!("Waveform loaded from cache for track: {}", track_key);
            }
            vec![]
        }
        Event::WaveformRetry(track_key) => {
            // generating is still true from the failed attempt; clear it so
            // LoadWaveform's needs_generation check passes.
            if state.waveform.track_key.as_ref() == Some(&track_key)
                && state.waveform.data.is_none()
            {
                state.waveform.generating = false;
                vec![Action::LoadWaveform]
            } else {
                vec![]
            }
        }
        Event::StationTracksLoaded { station_key, station_title, tracks, time_travel_decades } => {
            state.stations_loading = false;
            state.station_nav.loading = false;

            if tracks.is_empty() {
                state.set_error("Station returned no tracks (is Sonic Analysis enabled in Plex settings?)".to_string());
            } else {
                // Start playing the station in Radio mode
                state.playback_mode = PlaybackMode::Radio;
                state.radio.clear();
                state.radio.active_station = Some(crate::app::state::ActiveStation {
                    key: station_key,
                    title: station_title.clone(),
                });
                state.radio.tracks = tracks;
                state.radio.track_index = Some(0);

                // Time Travel Radio initialization
                if !time_travel_decades.is_empty() {
                    state.radio.time_travel_decades = time_travel_decades;
                    state.radio.time_travel_index = 3;
                    tracing::info!("Time Travel Radio: initialized with {} decades, next fetch from index 3",
                        state.radio.time_travel_decades.len());
                }

                state.view = View::NowPlaying;
                state.set_status(format!("Playing {} ({} tracks)", station_title, state.radio.tracks.len()));
                return vec![Action::PlayCurrentRadioTrack];
            }
            vec![]
        }
        Event::StationLoadFailed { station_key: _, error } => {
            state.stations_loading = false;
            state.station_nav.loading = false;
            state.set_error(error);
            vec![]
        }
        Event::StationChildrenLoaded { station_key, station_title, children } => {
            state.stations_loading = false;
            state.station_nav.loading = false;

            // Push new column with children (Miller columns style)
            state.station_nav.push_column(crate::app::state::StationColumn::new(
                Some(station_key),
                station_title,
                children.clone(),
            ));
            // Also update the legacy state for compatibility
            state.stations = children;
            state.clear_error();
            vec![]
        }
        Event::AlbumRadioTracksLoaded { tracks } => {
            state.radio_state.fetching = false;
            if !tracks.is_empty() {
                state.queue.extend(tracks);
                state.set_status(format!("Sonic album radio: {} tracks queued", state.queue.len()));
            }
            vec![]
        }
        Event::AlbumRadioLoadFailed { error } => {
            state.radio_state.fetching = false;
            tracing::warn!("{}", error);
            // Don't show error to user - seed album is already playing
            vec![]
        }
        Event::TrackRadioSimilarLoaded { mut tracks, title } => {
            use rand::seq::SliceRandom;

            state.radio_state.fetching = false;

            // Shuffle to break up album blocks and add diversity
            let mut rng = rand::rng();
            tracks.shuffle(&mut rng);

            // Filter out tracks already in history
            let new_tracks: Vec<_> = tracks.into_iter()
                .filter(|t| !state.radio_state.history.contains(&t.rating_key))
                .take(25)
                .collect();

            if !new_tracks.is_empty() {
                for track in &new_tracks {
                    state.radio_state.history.push(track.rating_key.clone());
                }
                state.queue.extend(new_tracks);
                state.set_status(format!("Sonic radio: {} ({} tracks)", title, state.queue.len()));
            } else if state.queue.is_empty() {
                state.set_error("No similar tracks found".to_string());
            }
            vec![]
        }

        // Radio track fetching completed (background)
        Event::RadioTracksLoaded { tracks, time_travel_index } => {
            let old_len = state.radio.tracks.len();

            // Deduplicate against existing tracks
            let existing_keys: std::collections::HashSet<_> = state.radio.tracks
                .iter()
                .map(|t| t.rating_key.clone())
                .collect();

            let unique_tracks: Vec<_> = tracks
                .into_iter()
                .filter(|t| !existing_keys.contains(&t.rating_key))
                .collect();

            let added = unique_tracks.len();
            if added > 0 {
                tracing::info!("Radio: adding {} new unique tracks", added);
                state.radio.tracks.extend(unique_tracks);
            }

            if let Some(idx) = time_travel_index {
                state.radio.time_travel_index = idx;
            }

            state.radio.fetching = false;

            // Auto-advance if we were stuck at end of radio list waiting for more tracks
            if added > 0
                && state.playback_mode == PlaybackMode::Radio
                && state.radio.track_index.is_some_and(|idx| idx + 1 >= old_len)
            {
                return vec![Action::Next];
            }

            vec![]
        }

        // Playlist tracks loaded (non-blocking)
        Event::PlaylistTracksForMillerLoaded { playlist_key, tracks } => {
            state.playlist_nav.loading = false;
            state.playlist_tracks_cache.insert(playlist_key, crate::plex::CachedPlaylistTracks::new(tracks.clone()));
            state.cache_dirty = true;
            let items = crate::app::state::BrowseItem::from_tracks(&tracks);
            let col = crate::app::state::BrowseColumn::new_with_tracks("tracks", items, tracks);
            state.playlist_nav.push_column(col);
            vec![]
        }
        Event::PlaylistTracksForMillerFailed { playlist_key: _, error } => {
            state.playlist_nav.loading = false;
            state.set_error(format!("Failed to load playlist: {}", error));
            vec![]
        }

        // Playlist tracks preloaded in background
        Event::PlaylistTracksPreloaded { playlist_key, tracks } => {
            if !tracks.is_empty() {
                state.playlist_tracks_cache.insert(playlist_key, crate::plex::CachedPlaylistTracks::new(tracks));
                state.cache_dirty = true;
            }
            vec![]
        }

        // Inline list filter completed
        Event::ListFilterCompleted { version, results } => {
            // Only apply if this is the most recent filter version
            if version == state.list_filter.version {
                state.list_filter.loading = false;
                state.list_filter.selected = 0;
                state.list_filter.results = Some(results);
                // Only update column selection if user is still on the filter column.
                // If they've drilled deeper (e.g., into subfolders), preserve their
                // current navigation — changing the selection would jump them away.
                if super::dispatch_search::is_on_filter_column(state) {
                    if let Some(ref results) = state.list_filter.results {
                        if let Some(&first_idx) = results.matched_indices.first() {
                            super::key_input::update_filter_column_selection(state, first_idx);
                        }
                    }
                }
            }
            vec![]
        }
        // Remote player control events
        Event::PlayersDiscovered(players) => {
            state.discovering_players = false;
            state.remote_players = players;
            let count = state.remote_players.len();
            if count > 0 {
                state.set_status(format!("Found {} player{}", count, if count == 1 { "" } else { "s" }));
            } else {
                state.set_status("No remote players found".to_string());
            }
            vec![]
        }
        Event::PlayerDiscoveryFailed(err) => {
            state.discovering_players = false;
            state.set_error(format!("Player discovery failed: {}", err));
            vec![]
        }
        Event::RemotePlayerStatus { session_found, playing, position_ms: _, track_key: _, finished } => {
            state.remote_playback.last_poll = Some(std::time::Instant::now());

            if let crate::app::state::OutputTarget::Remote { .. } = &state.output_target {
                if !session_found {
                    return vec![];
                }

                if finished {
                    return vec![Action::Next];
                }

                // Handle state transitions (pause/resume/stopped→playing).
                // Position is driven purely by the local clock from playback_started_at.
                if playing && state.playback.status != PlayStatus::Playing {
                    // Remote resumed (from paused or stopped): recalibrate local clock
                    let pos = state.playback.position_ms;
                    state.playback.playback_started_at = Some(
                        std::time::Instant::now() - Duration::from_millis(pos)
                    );
                    state.playback.status = PlayStatus::Playing;
                } else if !playing && state.playback.status == PlayStatus::Playing {
                    state.playback.status = PlayStatus::Paused;
                }
            }
            vec![]
        }
        Event::RemotePlayerError(err) => {
            tracing::warn!("Remote player error: {}", err);
            vec![]
        }

        _ => vec![],
    }
}
