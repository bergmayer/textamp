//! Event result handlers.
//!
//! Processes async event results (auth, data loading, playback, cache, etc.)

use crate::app::event::*;
use crate::app::action::*;
use crate::app::{Action, AppState, Event};
use crate::app::state::{
    BrowseCategory, BrowseItem, ConnectionState, PlayStatus, PlaybackMode,
    View,
};
use crate::plex::{PlexAuth, PlexClient};
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
        Event::Auth(AuthEvent::AuthSuccess { token, username, server_url, servers, client_identifier, has_plex_pass }) => {
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

            // Compare the persistent account marker against the user
            // we just authenticated as. The cache stays if it's the
            // same account AND the marker is < 30 days old; otherwise
            // a different user (or a long-stale cache) gets wiped so
            // we don't blend two libraries' data together.
            const CACHE_TTL_SECS: u64 = 30 * 24 * 60 * 60;
            let now_unix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let marker = PlexAuth::load_account_marker();
            let same_user = marker.as_ref().map_or(false, |m| m.username == username);
            let fresh = marker.as_ref().map_or(false, |m| now_unix.saturating_sub(m.last_seen_unix) < CACHE_TTL_SECS);
            let purge_cache = !same_user || !fresh;
            if purge_cache {
                if let Some(cache) = crate::plex::LibraryCache::new() {
                    match cache.clear_all() {
                        Ok(n) => {
                            let reason = if marker.is_none() {
                                "no prior marker"
                            } else if !same_user {
                                "different account"
                            } else {
                                "marker > 30 days old"
                            };
                            tracing::info!("Cache purged on sign-in ({n} library files, {reason})");
                        }
                        Err(e) => tracing::warn!("Failed to clear stale cache on sign-in: {e}"),
                    }
                }
                let artwork_cache = crate::plex::ArtworkCache::default();
                let _ = artwork_cache.clear_all();
            } else if let Some(m) = marker.as_ref() {
                let age_h = now_unix.saturating_sub(m.last_seen_unix) / 3600;
                tracing::info!("Cache preserved on sign-in (same user '{}' , {age_h}h since last seen)", m.username);
            }
            // Refresh the marker so the 30-day window starts from now
            // for any subsequent sign-in/out cycle.
            if let Err(e) = PlexAuth::save_account_marker(&username) {
                tracing::warn!("Failed to save account marker: {e}");
            }

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
                state.set_view(View::Browse);
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
                    let _ = event_tx.blocking_send(ArtworkEvent::ArtworkCacheStats { count, total_bytes }.into());
                });
            }

            // Compute library cache stats in background (per-library).
            // Only post the event when the on-disk cache file actually
            // exists with content — an empty breakdown would clobber
            // the in-memory estimate that `RefreshCacheStats` posts
            // when the user opens the Settings popup, leaving every
            // Size column row stuck at "—".
            {
                let event_tx = event_tx.clone();
                let lib_key = state.active_library.clone();
                tokio::task::spawn_blocking(move || {
                    if let (Some(cache), Some(key)) = (crate::plex::LibraryCache::new(), lib_key) {
                        let breakdown = cache.library_breakdown(&key);
                        if !breakdown.is_empty() {
                            let total_bytes = cache.library_size(&key);
                            let _ = event_tx.blocking_send(CacheEvent::LibraryCacheStats { total_bytes, breakdown }.into());
                        }
                    }
                });
            }

            // Compute waveform cache stats in background
            {
                let event_tx = event_tx.clone();
                tokio::task::spawn_blocking(move || {
                    let cache = crate::plex::WaveformCache::default();
                    let (count, total_bytes) = cache.stats();
                    let _ = event_tx.blocking_send(CacheEvent::WaveformCacheStats { count, total_bytes }.into());
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
                            let other_client = crate::plex::PlexClient::new_with_url(&url, Some(&token), &cid);
                            match other_client.get_libraries().await {
                                Ok(libs) => {
                                    let music_libs: Vec<_> = libs.into_iter().filter(|l| l.is_music()).collect();
                                    if !music_libs.is_empty() {
                                        let _ = tx.send(DataEvent::ServerLibrariesLoaded {
                                            server_identifier: server_clone.client_identifier.clone(),
                                            server_name: server_clone.name.clone(),
                                            libraries: music_libs,
                                        }.into()).await;
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

            vec![DataAction::LoadInitialData.into()]
        }
        Event::Auth(AuthEvent::ServersDiscovered(servers)) => {
            tracing::info!("Discovered {} servers", servers.len());
            state.available_servers = servers;
            state.settings_state.discovering_servers = false;
            vec![]
        }
        Event::Auth(AuthEvent::ServerDiscoveryFailed(error)) => {
            tracing::warn!("Server discovery failed: {}", error);
            state.settings_state.discovering_servers = false;
            state.set_error(format!("Server discovery failed: {}", error));
            vec![]
        }
        Event::Auth(AuthEvent::ServerConnectionSucceeded { server_name, url }) => {
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
                return vec![DataAction::LoadInitialData.into()];
            }
            vec![]
        }
        Event::Auth(AuthEvent::ServerConnectionFailed { server_name }) => {
            tracing::warn!("All connection tests failed for server {}", server_name);
            state.set_error(format!("Could not connect to {} - all connections failed", server_name));
            vec![]
        }
        Event::Auth(AuthEvent::AuthFailed(msg)) => {
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
        Event::Auth(AuthEvent::AuthShowLogin) => {
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
        Event::Auth(AuthEvent::AuthServersReady { token, username, servers, client_identifier, has_plex_pass }) => {
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
                        let _ = event_tx.send(AuthEvent::AuthSuccess {
                            token: token_clone,
                            username: username_clone,
                            server_url: url,
                            servers: servers_clone,
                            client_identifier: client_id_clone,
                            has_plex_pass,
                        }.into()).await;
                    } else {
                        let _ = event_tx.send(AuthEvent::AuthFailed(
                            "Could not connect to server - all connection attempts failed".to_string()
                        ).into()).await;
                    }
                });
            } else {
                // Multiple servers - let user choose
                state.set_view(View::Auth);
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
        Event::Auth(AuthEvent::AuthLoginFailed(msg)) => {
            use crate::app::state::AuthStep;
            tracing::error!("Login failed: {}", msg);
            state.auth_state.step = AuthStep::Login;
            state.auth_state.error_message = Some(msg);
            state.auth_state.password_input.clear();
            vec![]
        }
        Event::Data(DataEvent::ServerLibrariesLoaded { server_identifier, server_name, libraries }) => {
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
        Event::Data(DataEvent::LibrariesLoaded(libs)) => {
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
                if !state.library.artists.is_empty() {
                    tracing::info!("Library already loaded from cache, skipping reload");

                    // Trigger compilation detection if not already done (e.g., old cache without compilation data)
                    helpers::maybe_detect_compilations(event_tx, state, client);

                    return vec![];
                }

                // Active library valid but no cached data - start full preload
                let lib_key = match state.active_library.clone() {
                    Some(key) => key,
                    None => return vec![],
                };
                let lib_title = state.libraries.iter()
                    .find(|l| l.key == lib_key)
                    .map(|l| l.title.clone())
                    .unwrap_or_else(|| "Music".to_string());
                tracing::info!("Active library {} has no cached data, starting preloads", lib_key);
                helpers::preload_all_library_data(event_tx, &lib_key, &lib_title, client, state);
                return vec![];
            }

            // No valid active library - pick one
            if let Some(lib) = state.libraries.first() {
                tracing::info!("Selected music library: {} (key={})", lib.title, lib.key);
                let lib_key = lib.key.clone();
                let lib_title = lib.title.clone();
                state.active_library = Some(lib_key.clone());
                helpers::preload_all_library_data(event_tx, &lib_key, &lib_title, client, state);
            }
            vec![]
        }
        Event::Data(DataEvent::ArtistsLoaded(mut artists)) => {
            // Sort by display title, ignoring "The " prefix
            artists.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
            state.library.artists = artists;
            state.library.artists_loading = false;

            // Update artist_nav if we're in Artists category. No auto-drill —
            // child columns only open on explicit Enter/Right.
            if state.browse_category == BrowseCategory::Library && !state.library.artists.is_empty() {
                let title = "artists";
                let items = state.build_artist_root_items();
                state.artist_nav.update_root_items(title, items);
            }
            vec![]
        }
        Event::Data(DataEvent::AlbumsLoaded(mut albums)) => {
            // Sort by display title, ignoring "The " prefix
            albums.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
            state.library.albums = albums;
            state.library.albums_loading = false;
            vec![]
        }
        Event::Data(DataEvent::PlaylistsLoaded(mut playlists)) => {
            // Move "Recently Played" to top of playlist list
            if let Some(pos) = playlists.iter().position(|p| p.title == "Recently Played") {
                if pos > 0 {
                    let rp = playlists.remove(pos);
                    playlists.insert(0, rp);
                }
            }

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
                            let _ = tx.send(PreloadEvent::PlaylistTracksPreloaded {
                                playlist_key: pk, tracks,
                            }.into()).await;
                        }
                    }
                });
            }

            // Update playlist_nav with the playlists list. No auto-drill —
            // child columns only open on explicit Enter/Right.
            let items = crate::app::state::BrowseItem::from_playlists(&playlists);
            state.playlist_nav.update_root_items("playlists", items);

            // Build the live-keys set for stale-pruning of saved
            // per-playlist view toggles. Anything saved against a
            // playlist that's no longer in this library's list (i.e.
            // the user deleted it on Plex) gets dropped from config.
            let live_keys: std::collections::HashSet<String> =
                playlists.iter().map(|p| p.rating_key.clone()).collect();
            let prune_action = state.active_library.as_ref().cloned()
                .map(|lib_key| SettingsAction::PrunePlaylistViews {
                    library_key: lib_key,
                    live_playlist_keys: live_keys,
                }.into());

            state.library.playlists = playlists;
            state.library.playlists_loading = false;
            prune_action.map(|a: Action| vec![a]).unwrap_or_default()
        }
        Event::Data(DataEvent::TracksLoaded(tracks)) => {
            state.library.selected_album_tracks = tracks;
            state.library.right_panel_loading = false;
            vec![]
        }
        Event::Data(DataEvent::AlbumTracksLoaded(tracks)) => {
            state.library.selected_album_tracks = tracks;
            state.library.right_panel_loading = false;
            vec![]
        }
        Event::Data(DataEvent::ArtistAlbumsLoaded(albums)) => {
            state.library.selected_artist_albums = albums;
            state.library.right_panel_loading = false;
            state.focus = crate::app::state::Focus::Right;

            // Check if we need to auto-select a specific album (e.g., from Similar view)
            if let Some(album_key) = state.search.pending_album_key.take() {
                // Find the album in the list (+1 offset for "All Tracks" at index 0)
                if let Some(album_idx) = state.library.selected_artist_albums.iter()
                    .position(|a| a.rating_key == album_key)
                {
                    state.list_state.right_albums_index = album_idx + 1; // +1 for "All Tracks"
                    state.library.selected_album_title = state.library.selected_artist_albums[album_idx].title.clone();
                    return vec![DataAction::LoadAlbumTracks { rating_key: album_key }.into()];
                }
            }
            vec![]
        }
        Event::Data(DataEvent::ArtistAllTracksLoaded(tracks)) => {
            state.library.selected_album_tracks = tracks;
            state.library.right_panel_loading = false;
            vec![]
        }
        Event::Data(DataEvent::CategoryTracksLoaded(tracks)) => {
            if state.library.selected_album_title.is_empty() {
                if let Some(first) = tracks.first() {
                    state.library.selected_album_title = first.album_name().to_string();
                }
            }
            state.library.selected_album_tracks = tracks;
            state.library.right_panel_loading = false;
            vec![]
        }
        Event::Data(DataEvent::CategoryAlbumsLoaded { albums, status_message }) => {
            state.library.right_panel_mode = crate::app::state::RightPanelMode::CategoryAlbums;
            state.library.tag_albums = albums;
            state.library.tag_albums_index = 0;
            state.set_status(status_message);
            state.library.right_panel_loading = false;
            vec![]
        }
        Event::Data(DataEvent::DataLoadError(msg)) => {
            state.set_error(msg);
            state.library.right_panel_loading = false;
            state.similar.loading = false;
            state.artist_nav.loading = false;
            vec![]
        }
        Event::Data(DataEvent::AllAlbumsForMillerLoaded(mut albums)) => {
            // Async completion for LoadAllAlbumsForMiller when state.library.albums was empty
            albums.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
            state.library.albums = albums;
            state.library.albums_total = state.library.albums.len() as u32;
            // Now push the column (same as the sync path in dispatch_miller)
            vec![MillerAction::LoadAllAlbumsForMiller { replace_child: false }.into()]
        }
        Event::Data(DataEvent::SimilarAlbumsLoaded(albums)) => {
            state.similar.albums = albums;
            state.similar.mode = crate::app::state::SimilarMode::Albums;
            state.similar.loading = false;
            state.list_state.similar_index = 0;
            vec![]
        }
        Event::Data(DataEvent::SimilarTracksLoaded(tracks)) => {
            state.similar.tracks = tracks;
            state.similar.mode = crate::app::state::SimilarMode::Tracks;
            state.similar.loading = false;
            state.list_state.similar_index = 0;
            vec![]
        }
        Event::Data(DataEvent::TrackPaneSimilarLoaded { rating_key, tracks }) => {
            state.track_pane_similar_loading.remove(&rating_key);
            state.track_pane_similar.insert(rating_key, tracks);
            vec![]
        }
        Event::Data(DataEvent::SimilarArtistsLoaded(artists)) => {
            state.similar.artists = artists;
            state.similar.mode = crate::app::state::SimilarMode::Artists;
            state.similar.loading = false;
            state.list_state.similar_index = 0;
            vec![]
        }
        Event::Data(DataEvent::RelatedDataLoaded { groups }) => {
            state.related.groups = groups;
            state.related.loading = false;
            state.list_state.related_index = 0;
            state.scroll.related = None;
            vec![]
        }
        Event::Data(DataEvent::SearchCompleted(results)) => {
            // Legacy handler for non-debounced search
            state.list_state.search_item_index = 0;
            state.search.results = Some(results);
            vec![]
        }
        Event::Data(DataEvent::TrackSearchCompleted { version, tracks }) => {
            use crate::services::search_tracks_with_ranking;
            if version == u64::MAX {
                // Radio launcher track search result — re-rank by query
                if let Some(ref mut launcher) = state.popups.radio_launcher {
                    let ranked = search_tracks_with_ranking(&tracks, &launcher.query, 50);
                    if let Some(ref mut results) = launcher.results {
                        results.tracks = ranked;
                    }
                    launcher.loading = false;
                }
            } else if version == state.search.track_version {
                // Only apply results if version matches (not stale)
                let ranked = search_tracks_with_ranking(&tracks, &state.search.query, 50);
                if let Some(ref mut results) = state.search.results {
                    results.tracks = ranked;
                }
                state.search.track_loading = false;
            }
            vec![]
        }
        Event::Data(DataEvent::AdventureTrackSearchCompleted { version, tracks }) => {
            use crate::services::search_tracks_with_ranking;
            // Discard stale callbacks: only apply if the launcher is
            // still open AND its current `search_version` matches the
            // one we kicked off with. Anything older was superseded
            // by a later keystroke.
            if let Some(ref mut launcher) = state.popups.adventure_launcher {
                if launcher.search_version == version {
                    let ranked = search_tracks_with_ranking(&tracks, &launcher.query, 50);
                    if let Some(ref mut results) = launcher.results {
                        results.tracks = ranked;
                    }
                    launcher.loading = false;
                }
            }
            vec![]
        }
        Event::Ui(UiEvent::AdventureLauncherAlbumsLoaded { artist_key, artist_name, albums }) => {
            if let Some(ref mut launcher) = state.popups.adventure_launcher {
                launcher.drill = crate::app::state::AdventureDrillLevel::ArtistAlbums {
                    artist_key, artist_name, albums,
                };
                launcher.item_index = 0;
                launcher.focus = crate::app::state::SearchFocus::Results;
                launcher.loading = false;
            }
            vec![]
        }
        Event::Ui(UiEvent::AdventureLauncherTracksLoaded { album_key, album_title, artist_name, tracks }) => {
            if let Some(ref mut launcher) = state.popups.adventure_launcher {
                launcher.drill = crate::app::state::AdventureDrillLevel::AlbumTracks {
                    album_key, album_title, artist_name, tracks,
                };
                launcher.item_index = 0;
                launcher.focus = crate::app::state::SearchFocus::Results;
                launcher.loading = false;
            }
            vec![]
        }
        Event::Data(DataEvent::ApiError(msg)) => {
            state.set_error(msg);
            vec![]
        }
        Event::Playback(PlaybackEvent::TrackStarted) => {
            state.playback.status = PlayStatus::Playing;
            state.playback.position_ms = 0;
            // Successful playback supersedes any prior error banner
            // ("Playback stopped after multiple consecutive errors",
            // "Track Not Found", etc). Without this the red banner
            // sticks around even after the user resumes / the queue
            // recovers, making the app feel broken when it isn't.
            state.consecutive_playback_errors = 0;
            state.clear_error();
            vec![]
        }
        Event::Playback(PlaybackEvent::TrackEnded) => {
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
            vec![PlaybackAction::Next.into()]
        }
        Event::Playback(PlaybackEvent::PlaybackPaused) => {
            state.playback.status = PlayStatus::Paused;
            vec![]
        }
        Event::Playback(PlaybackEvent::PlaybackResumed) => {
            state.playback.status = PlayStatus::Playing;
            vec![]
        }
        Event::Playback(PlaybackEvent::PlaybackStopped) => {
            state.playback.status = PlayStatus::Stopped;
            state.playback.position_ms = 0;
            vec![]
        }
        Event::Playback(PlaybackEvent::PlaybackError(msg)) => {
            state.playback.status = PlayStatus::Stopped;
            state.consecutive_playback_errors += 1;

            let track_info = state.current_track()
                .map(|t| format!("{} - {}", t.artist_name(), t.title))
                .unwrap_or_else(|| "unknown".to_string());
            let qi = state.queue.index.unwrap_or(9999);

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
                    let _ = tx.send(PlaybackEvent::RetryAfterDelay.into()).await;
                });
                return vec![];
            }

            // Errors 6-8: auto-skip to next track
            if state.consecutive_playback_errors <= 8 {
                tracing::warn!("Playback error (skipping queue[{}] '{}', attempt {}/8): {}",
                    qi, track_info, state.consecutive_playback_errors, msg);
                return vec![PlaybackAction::Next.into()];
            }

            // After 8 consecutive failures, show error to user
            state.consecutive_playback_errors = 0;
            if msg.contains("404") || msg.to_lowercase().contains("not found") {
                state.popups.close_all();
                state.popups.confirm_dialog = Some(crate::app::state::ConfirmDialog {
                    title: "Track Not Found".to_string(),
                    message: "This track may have been removed. Refresh cache?".to_string(),
                    on_confirm: crate::app::state::ConfirmAction::RefreshCache,
                    selected_yes: true,
                });
            } else if let crate::app::state::OutputTarget::Remote { ref player_name, .. } = state.remote.output_target {
                state.set_error(format!("Playback failed — {} may not be running", player_name));
            } else {
                state.set_error("Playback stopped after multiple consecutive errors".to_string());
            }
            vec![]
        }
        Event::Playback(PlaybackEvent::RetryAfterDelay) => {
            vec![PlaybackAction::RetryCurrentTrack.into()]
        }
        Event::Playback(PlaybackEvent::BufferingStart) => {
            state.playback.status = PlayStatus::Buffering;
            vec![]
        }
        Event::Playback(PlaybackEvent::BufferingEnd) => {
            // Don't reset consecutive_playback_errors here — wait for sustained
            // playback (5s) to confirm the track is actually playing successfully.
            vec![PlaybackAction::StartPendingPlayback.into()]
        }
        Event::Playback(PlaybackEvent::PositionUpdate(pos)) => {
            state.playback.position_ms = pos;
            vec![]
        }
        Event::Artwork(ArtworkEvent::ArtworkLoaded { thumb_path, data }) => {
            state.artwork.current_thumb = Some(thumb_path);
            state.artwork.current_data = Some(data);
            state.artwork.loading = false;
            vec![]
        }
        Event::Artwork(ArtworkEvent::ArtworkFailed { thumb_path: _ }) => {
            state.artwork.current_thumb = None;
            state.artwork.current_data = None;
            state.artwork.loading = false;
            vec![]
        }
        Event::Artwork(ArtworkEvent::AlbumArtLoaded { key, data }) => {
            state.artwork.grid_pending.remove(&key);
            state.artwork.grid_cache.insert(key, data);
            vec![]
        }
        Event::Artwork(ArtworkEvent::AlbumArtFailed { key }) => {
            state.artwork.grid_pending.remove(&key);
            vec![]
        }
        Event::Folder(FolderEvent::FoldersPreloaded { library_key, folder_state }) => {
            // Ignore if this is for a different library (race condition from library switch)
            if state.active_library.as_ref() != Some(&library_key) {
                tracing::debug!("Ignoring stale folders preload for library {}", library_key);
                return vec![];
            }
            if state.folder_state.is_none() {
                // First load: set up folder state
                let mut fs = folder_state;
                fs.ensure_placeholder();
                state.folder_state = Some(fs);
                tracing::debug!("Folders preloaded and ready");
            } else if let Some(ref mut existing) = state.folder_state {
                // Refresh: update root column items while preserving navigation
                if let Some(new_root) = folder_state.inner.columns.into_iter().next() {
                    if let Some(old_root) = existing.columns.first_mut() {
                        let old_selected = old_root.selected_index;
                        old_root.items = new_root.items;
                        old_root.selected_index = old_selected.min(old_root.items.len().saturating_sub(1));
                    }
                }
                tracing::debug!("Folders root refreshed");
            }
            state.cache_mgmt.preloads_in_progress.remove("Folders");
            if state.cache_mgmt.preloads_in_progress.is_empty() { state.cache_mgmt.preloads_total = 0; }
            vec![]
        }
        Event::Folder(FolderEvent::SubfoldersPreloaded { library_key, entries, done, valid_keys }) => {
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
                state.cache_mgmt.dirty = true;
            }
            if done {
                state.subfolder_preload_active = false;

                // Prune cache entries for folders that no longer exist on the server.
                // valid_keys contains every folder key seen during the crawl.
                if let Some(valid) = valid_keys {
                    let before = state.folder_contents_cache.len();
                    state.folder_contents_cache.retain(|key, _| valid.contains(key));
                    let pruned = before - state.folder_contents_cache.len();
                    if pruned > 0 {
                        tracing::info!("Pruned {} stale subfolder cache entries", pruned);
                        state.cache_mgmt.dirty = true;
                    }
                }

                tracing::info!("Subfolder preload finished, {} total cached subfolders", state.folder_contents_cache.len());
            }
            vec![]
        }
        Event::Folder(FolderEvent::SubfolderRefreshed { folder_key, cached_folder }) => {
            // Background warm-cache re-fetch completed — update cache and refresh UI
            state.folder_contents_cache.insert(folder_key.clone(), cached_folder.clone());
            state.cache_mgmt.dirty = true;
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
        Event::Folder(FolderEvent::FolderRootLoaded { library_key, lib_title, items }) => {
            // Ignore if library changed while loading
            if state.active_library.as_ref() != Some(&library_key) {
                return vec![];
            }
            if state.folder_state.is_none() {
                use crate::services::{FolderColumn, FolderNavigationState};
                let root_column = FolderColumn::new(None, lib_title, items);
                let mut fs = FolderNavigationState::with_root(library_key, root_column);
                fs.ensure_placeholder();
                state.folder_state = Some(fs);
            } else if let Some(ref mut folder_state) = state.folder_state {
                // Refresh: update root column items while preserving navigation
                if let Some(root_col) = folder_state.columns.first_mut() {
                    let old_selected = root_col.selected_index;
                    root_col.items = items;
                    root_col.selected_index = old_selected.min(root_col.items.len().saturating_sub(1));
                }
            }
            vec![]
        }
        Event::Folder(FolderEvent::FolderContentsLoaded { folder_key, items, folder_path, item_path, replace_child }) => {
            use crate::plex::CachedFolder;
            use crate::services::FolderColumn;
            use super::dispatch_folders::{derive_path_from_children, backfill_parent_path, spawn_path_discovery};

            state.pending_folder_load = None;

            let resolved_path = item_path
                .or(folder_path)
                .or_else(|| derive_path_from_children(&items, &state.folder_contents_cache));
            let folder_title = resolved_path.clone().unwrap_or_default();

            // Store in cache
            state.folder_contents_cache.insert(folder_key.clone(), CachedFolder::with_path(items.clone(), resolved_path));
            state.cache_mgmt.dirty = true;
            tracing::debug!("Cached folder: {} ({} items)", folder_key, items.len());

            // If we couldn't determine the path, probe a child folder in background
            if folder_title.is_empty() {
                spawn_path_discovery(&folder_key, &items, event_tx, client);
            }

            // Only push column if no column with this folder_key already exists
            // (prevents duplicates from race with cache-hit navigation)
            if let Some(ref mut folder_state) = state.folder_state {
                let already_exists = folder_state.columns.iter()
                    .any(|col| col.key.as_ref() == Some(&folder_key));
                if !already_exists {
                    let new_column = FolderColumn::new(Some(folder_key), folder_title, items);
                    if replace_child {
                        folder_state.replace_child_column(new_column);
                    } else {
                        folder_state.push_column(new_column);
                    }
                    backfill_parent_path(folder_state);
                }
            }
            state.clear_status();
            vec![]
        }
        Event::Folder(FolderEvent::FolderLoadFailed(msg)) => {
            state.pending_folder_load = None;
            state.set_error(msg);
            vec![]
        }
        Event::Folder(FolderEvent::FolderRefreshLoaded { folder_key, items, folder_path }) => {
            use crate::plex::CachedFolder;
            let folder_title = folder_path.clone().unwrap_or_default();

            // Update the cache with fresh data and new timestamp
            state.folder_contents_cache.insert(folder_key.clone(), CachedFolder::with_path(items.clone(), folder_path));
            state.cache_mgmt.dirty = true;
            tracing::info!("Refreshed subfolder cache: {} ({} items)", folder_key, items.len());

            // Update the currently displayed column if it matches
            if let Some(ref mut folder_state) = state.folder_state {
                for col in folder_state.columns.iter_mut() {
                    if col.key.as_ref() == Some(&folder_key) {
                        let old_selected = col.selected_index;
                        col.items = items.clone();
                        col.selected_index = old_selected.min(col.items.len().saturating_sub(1));
                        if !folder_title.is_empty() {
                            col.title = folder_title.clone();
                        }
                        break;
                    }
                }
            }
            state.set_status("Folder refreshed".to_string());
            vec![]
        }
        Event::Folder(FolderEvent::FolderPathDiscovered { folder_key, path }) => {
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
                    state.cache_mgmt.dirty = true;
                }
            }
            vec![]
        }
        Event::Artwork(ArtworkEvent::ArtworkCacheStats { count, total_bytes }) => {
            state.artwork.cache_stats = Some((count, total_bytes));
            vec![]
        }
        Event::Cache(CacheEvent::LibraryCacheStats { total_bytes, breakdown }) => {
            state.library_cache_stats = Some((total_bytes, breakdown));
            vec![]
        }
        Event::Cache(CacheEvent::WaveformCacheStats { count, total_bytes }) => {
            state.waveform_cache_stats = Some((count, total_bytes));
            vec![]
        }
        Event::Preload(PreloadEvent::ArtistsPreloaded { library_key, mut artists }) => {
            // Ignore if this is for a different library (race condition from library switch)
            if state.active_library.as_ref() != Some(&library_key) {
                tracing::debug!("Ignoring stale artists preload for library {}", library_key);
                return vec![];
            }
            if state.library.artists.is_empty() || !state.library.artists_loading {
                artists.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
                let count = artists.len();
                state.library.artists = artists;
                state.library.artists_total = count as u32;
                tracing::debug!("Artists preloaded: {} items", count);
                // Update Miller columns (preserves drill-down state)
                if !state.library.artists.is_empty() {
                    let items = state.build_artist_root_items();
                    state.artist_nav.update_root_items("artists", items);
                }
            }
            // Artists now loaded — try compilation detection (needs both artists + albums)
            helpers::maybe_detect_compilations(event_tx, state, client);
            state.cache_mgmt.preloads_in_progress.remove("Artists");
            if state.cache_mgmt.preloads_in_progress.is_empty() { state.cache_mgmt.preloads_total = 0; }
            vec![]
        }
        Event::Preload(PreloadEvent::AlbumsPreloaded { library_key, mut albums }) => {
            // Ignore if this is for a different library (race condition from library switch)
            if state.active_library.as_ref() != Some(&library_key) {
                tracing::debug!("Ignoring stale albums preload for library {}", library_key);
                return vec![];
            }
            if state.library.albums.is_empty() || !state.library.albums_loading {
                albums.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
                let count = albums.len();
                state.library.albums = albums;
                state.library.albums_total = count as u32;
                tracing::debug!("Albums preloaded: {} items", count);
            }

            // Spawn background compilation detection if not already done
            helpers::maybe_detect_compilations(event_tx, state, client);
            state.cache_mgmt.preloads_in_progress.remove("Albums");
            if state.cache_mgmt.preloads_in_progress.is_empty() { state.cache_mgmt.preloads_total = 0; }
            vec![]
        }
        Event::Preload(PreloadEvent::PlaylistsPreloaded { library_key, playlists }) => {
            // Ignore if this is for a different library (race condition from library switch)
            if state.active_library.as_ref() != Some(&library_key) {
                tracing::debug!("Ignoring stale playlists preload for library {}", library_key);
                return vec![];
            }
            if state.library.playlists.is_empty() || !state.library.playlists_loading {
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
                                let _ = tx.send(PreloadEvent::PlaylistTracksPreloaded {
                                    playlist_key: pk, tracks,
                                }.into()).await;
                            }
                        }
                    });
                }

                tracing::debug!("Playlists preloaded: {} items", count);
                if !playlists.is_empty() {
                    state.library.playlists = playlists;
                    let items = crate::app::state::BrowseItem::from_playlists(&state.library.playlists);
                    state.playlist_nav.update_root_items("playlists", items);
                } else if state.library.playlists.is_empty() {
                    // Only clear nav if we also have no cached playlists
                    state.library.playlists = playlists;
                    state.playlist_nav = crate::app::state::BrowseNavigationState::new();
                }
                // else: preload returned empty but we have cached data — keep cached
            }
            state.cache_mgmt.preloads_in_progress.remove("Playlists");
            if state.cache_mgmt.preloads_in_progress.is_empty() { state.cache_mgmt.preloads_total = 0; }
            vec![]
        }
        Event::Preload(PreloadEvent::ArtistGenresPreloaded { library_key, genres }) => {
            handle_tag_preload(state, &library_key, "Artist Genres",
                crate::app::state::BrowseCategory::ArtistGenres, genres)
        }
        Event::Preload(PreloadEvent::AlbumGenresPreloaded { library_key, genres }) => {
            handle_tag_preload(state, &library_key, "Album Genres",
                crate::app::state::BrowseCategory::AlbumGenres, genres)
        }
        Event::Preload(PreloadEvent::MoodsPreloaded { library_key, moods }) => {
            handle_tag_preload(state, &library_key, "Moods",
                crate::app::state::BrowseCategory::Moods, moods)
        }
        Event::Preload(PreloadEvent::StylesPreloaded { library_key, styles }) => {
            handle_tag_preload(state, &library_key, "Styles",
                crate::app::state::BrowseCategory::Styles, styles)
        }
        Event::Preload(PreloadEvent::TagListPreloaded { library_key, category, items }) => {
            use crate::app::state::{BrowseCategory, RefreshCategory};
            let (label, sec) = match category {
                RefreshCategory::Decades => ("Decades", BrowseCategory::Decades),
                RefreshCategory::Years => ("Years", BrowseCategory::Years),
                RefreshCategory::Collections => ("Collections", BrowseCategory::Collections),
                RefreshCategory::Countries => ("Countries", BrowseCategory::Countries),
                RefreshCategory::Labels => ("Labels", BrowseCategory::Labels),
                RefreshCategory::Formats => ("Formats", BrowseCategory::Formats),
                RefreshCategory::Studios => ("Studios", BrowseCategory::Studios),
                _ => return vec![],
            };
            handle_tag_preload(state, &library_key, label, sec, items)
        }
        Event::Preload(PreloadEvent::StationsPreloaded { library_key, mut stations }) => {
            // Ignore if this is for a different library (race condition from library switch)
            if state.active_library.as_ref() != Some(&library_key) {
                tracing::debug!("Ignoring stale stations preload for library {}", library_key);
                return vec![];
            }
            if state.stations.is_empty() && !state.stations_loading {
                let count = stations.len();
                helpers::append_station_action_items(&mut stations, state.queue.shuffle_undo_queue.is_some());
                state.stations = stations.clone();
                tracing::debug!("Stations preloaded: {} items", count);
                // Rebuild station Miller columns
                state.station_nav.columns.clear();
                state.station_nav.columns.push(crate::app::state::StationColumn::new(
                    None,
                    "Radio".to_string(),
                    stations,
                ));
                state.station_nav.focused_column = 0;
            }
            state.cache_mgmt.preloads_in_progress.remove("Stations");
            if state.cache_mgmt.preloads_in_progress.is_empty() { state.cache_mgmt.preloads_total = 0; }
            vec![]
        }
        Event::Preload(PreloadEvent::AllTracksPreloaded { library_key, tracks }) => {
            if state.active_library.as_ref() != Some(&library_key) {
                tracing::debug!("Ignoring stale all-tracks preload for library {}", library_key);
                return vec![];
            }
            let was_refresh = !state.library.all_tracks.is_empty();
            let count = tracks.len();
            state.library.all_tracks = tracks;
            tracing::debug!("All tracks {}: {} items", if was_refresh { "refreshed" } else { "preloaded" }, count);
            state.cache_mgmt.dirty = true;

            // Build track-level artist list and artist aliases
            state.build_track_artists();
            state.build_artist_aliases();

            if was_refresh {
                // On refresh: re-detect compilations (reset flag so maybe_detect runs)
                state.library.compilations.detected = false;
                helpers::maybe_detect_compilations(event_tx, state, client);

                // Re-render All Artists album column if active (so album_display_artist updates show)
                if state.artist_nav.columns.len() >= 2 {
                    if let Some(col0) = state.artist_nav.columns.first() {
                        let is_all_artists = col0.selected_item()
                            .map_or(false, |i| matches!(i, BrowseItem::AllArtists));
                        if is_all_artists {
                            let mut items: Vec<BrowseItem> = BrowseItem::from_albums(&state.library.albums, &state.library.album_display_artist);
                            items.push(BrowseItem::AllTracks {
                                scope: crate::app::state::AllTracksScope::Library,
                                thumb: None,
                            });
                            let old_idx = state.artist_nav.columns[1].selected_index;
                            state.artist_nav.columns[1].items = items;
                            state.artist_nav.columns[1].selected_index = old_idx.min(
                                state.artist_nav.columns[1].items.len().saturating_sub(1)
                            );
                        }
                    }
                }
            } else {
                // Initial preload: trigger compilation detection
                helpers::maybe_detect_compilations(event_tx, state, client);
            }

            // Fill any pending "tracks (loading...)" placeholder column
            if let Some(col) = state.artist_nav.focused_mut() {
                if col.items.is_empty() && col.title.starts_with("tracks (loading") {
                    let items = BrowseItem::from_tracks(&state.library.all_tracks);
                    col.title = format!("tracks ({})", state.library.all_tracks.len());
                    col.items = items;
                    col.tracks = state.library.all_tracks.clone();
                }
            }

            state.cache_mgmt.preloads_in_progress.remove("Tracks");
            if state.cache_mgmt.preloads_in_progress.is_empty() { state.cache_mgmt.preloads_total = 0; }
            vec![]
        }
        Event::Preload(PreloadEvent::PreloadFailed { category }) => {
            tracing::warn!("Preload failed for category: {}", category);
            state.cache_mgmt.preloads_in_progress.remove(&category);
            if state.cache_mgmt.preloads_in_progress.is_empty() { state.cache_mgmt.preloads_total = 0; }
            vec![]
        }
        Event::Preload(PreloadEvent::CompilationsDetected { library_key, albums, artist_only_keys, track_artist_keys, artist_compilation_map, single_artist_compilations }) => {
            if state.active_library.as_ref() != Some(&library_key) {
                return vec![];
            }
            state.library.compilations.albums = albums;
            state.library.compilations.artist_keys = artist_only_keys;
            state.library.compilations.track_artist_keys = track_artist_keys;
            state.library.compilations.artist_map = artist_compilation_map;
            state.library.compilations.single_artist = single_artist_compilations;
            state.library.compilations.detected = true;
            state.cache_mgmt.dirty = true;

            // Update artist root column if currently viewing Library
            if !state.library.compilations.albums.is_empty() || !state.library.compilations.artist_keys.is_empty() {
                let items = state.build_artist_root_items();
                state.artist_nav.update_root_items("artists", items);
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

            // Clear expired toasts (5 second display)
            if let Some(show_time) = state.notifications.toast_show_time {
                if show_time.elapsed() > Duration::from_secs(5) {
                    state.notifications.toast_message = None;
                    state.notifications.toast_show_time = None;
                }
            }

            // Clear expired status messages (5 second display)
            if let Some(show_time) = state.notifications.status_show_time {
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

            // Per-tick counter for animated "Loading..." text in
            // miller column placeholders. Wraps; consumers do `% 4`.
            state.loading_tick = state.loading_tick.wrapping_add(1);

            // Drain the audio backend's sample tap into the
            // vectorscope buffer. Done unconditionally (cheap) so
            // the visualizer is "warm" the moment the user opens
            // its tab. Buffer is capped at VECTORSCOPE_BUFFER_LEN
            // — older samples roll off the front of the deque.
            if let Some(tap) = state.vectorscope_tap.clone() {
                if let Ok(mut q) = tap.lock() {
                    while let Some(s) = q.pop_front() {
                        if state.vectorscope_buffer.len() >= crate::app::state::VECTORSCOPE_BUFFER_LEN {
                            state.vectorscope_buffer.pop_front();
                        }
                        state.vectorscope_buffer.push_back(s);
                    }
                }
            }

            // Lazy-art settle: if `suppress_loads` was raised by a
            // recent rapid-nav gesture and the user has been still for
            // `ART_LOAD_PAUSE_MS`, clear the gate and dispatch one
            // viewport-wide `LoadAlbumArt` batch.
            if let Some(actions) = super::lazy_art::settle(state) {
                if !actions.is_empty() {
                    return actions;
                }
            }

            // Track-details pane: lazy-fetch sonically-similar tracks
            // for whatever Track row is currently focused. Returned
            // as a follow-up `LoadTrackPaneSimilar` action so the
            // dispatcher (which has client access) actually fires
            // the API call. Idempotent — the dispatcher early-returns
            // when the entry is already loaded or in flight.
            if state.view == crate::app::state::View::Browse {
                if let Some(track) = state.focused_track() {
                    let key = track.rating_key.clone();
                    if !key.is_empty()
                        && !state.track_pane_similar.contains_key(&key)
                        && !state.track_pane_similar_loading.contains(&key)
                    {
                        return vec![crate::app::action::DataAction::LoadTrackPaneSimilar {
                            rating_key: key,
                        }.into()];
                    }
                }
            }

            // Album art loading: lazy-load for visible items across
            // EVERY column with `artwork_visible`, not just the
            // focused one. The user expects album artwork to start
            // populating the moment they pause on an artist row —
            // before they've drilled into the album column itself —
            // so the artist's albums column (focused +1) and any
            // already-open art columns to its right all get a turn.
            // Cap concurrent in-flight requests to avoid overwhelming
            // the Plex transcoder.
            if state.view == crate::app::state::View::Browse
                && state.artwork.grid_pending.len() < 4
            {
                let nav = match state.browse_category {
                    crate::app::state::BrowseCategory::Library => &state.artist_nav,
                    crate::app::state::BrowseCategory::Playlists => &state.playlist_nav,
                    cat if cat.is_tag_section() => &state.tag_nav,
                    _ => &state.artist_nav,
                };
                let max_batch = 4usize.saturating_sub(state.artwork.grid_pending.len());
                let mut to_load: Vec<(String, String)> = Vec::new();
                // Iterate art-visible columns starting from the
                // focused one and walking outward, so the column the
                // user is staring at gets first claim on the budget.
                let focused_idx = nav.focused_column;
                let mut col_order: Vec<usize> = Vec::with_capacity(nav.columns.len());
                col_order.push(focused_idx);
                for d in 1..=nav.columns.len() {
                    if focused_idx + d < nav.columns.len() {
                        col_order.push(focused_idx + d);
                    }
                    if focused_idx >= d {
                        col_order.push(focused_idx - d);
                    }
                }
                'cols: for ci in col_order {
                    let col = match nav.columns.get(ci) { Some(c) => c, None => continue };
                    if !col.artwork_visible { continue; }
                    let total_items = col.items.len();
                    if total_items == 0 { continue; }
                    // Same visible-window math as the renderer, used
                    // here as a heuristic for "what the user is most
                    // likely about to see". Keeping it consistent with
                    // the TUI's `render_album_art_grid` formula avoids
                    // wasted prefetches on rows that are off-screen.
                    let inner_height = state.terminal_height.saturating_sub(4) as usize;
                    let target_visible = 3usize.max((total_items).min(5));
                    let row_height = if target_visible > 0 { (inner_height / target_visible).max(3) } else { 3 };
                    let visible_rows = if row_height > 0 { (inner_height / row_height).max(1) } else { 1 };
                    let scroll_offset = crate::services::NavigationService::calc_scroll_offset(
                        col.selected_index, visible_rows, total_items,
                    );
                    let end = (scroll_offset + visible_rows).min(total_items);
                    for item in &col.items[scroll_offset..end] {
                        if to_load.len() >= max_batch { break 'cols; }
                        match item {
                            BrowseItem::Album { key, thumb: Some(thumb), .. } => {
                                if !state.artwork.grid_cache.contains_key(key)
                                    && !state.artwork.grid_pending.contains(key)
                                {
                                    to_load.push((key.clone(), thumb.clone()));
                                }
                            }
                            BrowseItem::AllTracks { scope, thumb: Some(thumb) } => {
                                if let Some(artist_key) = scope.artist_key() {
                                    if !state.artwork.grid_cache.contains_key(artist_key)
                                        && !state.artwork.grid_pending.contains(artist_key)
                                    {
                                        to_load.push((artist_key.to_string(), thumb.clone()));
                                    }
                                }
                            }
                            BrowseItem::Artist { key, thumb: Some(thumb), .. } => {
                                if !state.artwork.grid_cache.contains_key(key)
                                    && !state.artwork.grid_pending.contains(key)
                                {
                                    to_load.push((key.clone(), thumb.clone()));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                if !to_load.is_empty() {
                    return vec![SystemAction::LoadAlbumArt(to_load).into()];
                }
            }

            // Visualizer data safety net: ensure waveform/spectrogram are generated
            // when the user is on a view where the visualizer panel is rendered.
            // The GUI's Queue view now shows the visualizer always-on in its
            // bottom half, so we trip this on both View::NowPlaying and
            // View::Queue. Catches all edge cases (track change, re-entry,
            // failed downloads) without fragile event-based triggering.
            if matches!(state.view, View::NowPlaying | View::Queue) {
                if let Some(track) = state.current_track().cloned() {
                    let tk = &track.rating_key;

                    // Ensure track_key is set (handles track change while on this view)
                    if state.waveform.track_key.as_ref() != Some(tk) {
                        state.waveform = crate::app::state::WaveformState::default();
                        state.waveform.track_key = Some(tk.clone());
                        state.spectrogram = crate::app::state::SpectrogramState::default();
                        state.spectrogram.track_key = Some(tk.clone());
                    }

                    // Trigger waveform if needed (co-generates spectrogram)
                    if state.waveform.data.is_none() && !state.waveform.generating {
                        state.waveform.error = None;
                        return vec![SystemAction::LoadWaveform.into()];
                    }

                    // Trigger spectrogram independently if waveform is done but spectrogram isn't
                    if state.spectrogram.data.is_none()
                        && !state.spectrogram.generating
                        && state.waveform.data.is_some()
                    {
                        state.spectrogram.error = None;
                        return vec![SystemAction::LoadSpectrogram.into()];
                    }
                }
            }

            // Periodic cache save: save if dirty, idle for 30+ seconds, and 2+ minutes since last save
            helpers::maybe_save_cache_async(event_tx, state);

            // (Per-category staleness checks are now done on view navigation, not on tick)

            vec![]
        }
        Event::Preload(PreloadEvent::LibraryCacheLoaded { library_key, cached }) => {
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
                        state.cache_mgmt.category_timestamps.insert(cat, *ts);
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
                        state.cache_mgmt.category_timestamps.insert(*cat, ts);
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
                state.library.artists = cached.artists;
                state.library.artists.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
                state.library.artists_total = state.library.artists.len() as u32;
                let items = state.build_artist_root_items();
                state.artist_nav.reset("artists", items);
            }
            if !cached.albums.is_empty() {
                state.library.albums = cached.albums;
                state.library.albums.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
                state.library.albums_total = state.library.albums.len() as u32;
            }
            if !cached.playlists.is_empty() {
                let mut playlists = cached.playlists.clone();
                // Move "Recently Played" to top (matches PlaylistsLoaded behavior)
                if let Some(pos) = playlists.iter().position(|p| p.title == "Recently Played") {
                    if pos > 0 {
                        let rp = playlists.remove(pos);
                        playlists.insert(0, rp);
                    }
                }
                state.library.playlists = playlists;
                let items = BrowseItem::from_playlists(&state.library.playlists);
                state.playlist_nav.reset("playlists", items);
            }
            if !cached.playlist_tracks.is_empty() {
                state.playlist_tracks_cache = cached.playlist_tracks;
                tracing::debug!("Loaded {} cached playlist track lists", state.playlist_tracks_cache.len());
            }

            // Folders
            if !cached.root_folders.is_empty() {
                use crate::services::{FolderColumn, FolderNavigationState, FolderService};
                let folders = FolderService::filter_invalid(cached.root_folders);
                let root_column = FolderColumn::new(None, lib_name, folders);
                let mut fs = FolderNavigationState::with_root(library_key.clone(), root_column);
                fs.ensure_placeholder();
                state.folder_state = Some(fs);
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
            // Just store the data — genre_nav is populated lazily via DrillGenreCategory
            if !cached.genres.is_empty() { state.library.album_genres = cached.genres; }
            if !cached.artist_genres.is_empty() { state.library.artist_genres = cached.artist_genres; }
            if !cached.album_genres.is_empty() { state.library.album_genres = cached.album_genres; }
            if !cached.moods.is_empty() { state.library.moods = cached.moods; }
            if !cached.styles.is_empty() { state.library.styles = cached.styles; }

            // Stations — validate cached data is root stations (not corrupted drilled children)
            let stations_valid = !cached.stations.is_empty()
                && cached.stations.iter().any(|s| s.identifier.as_deref() == Some("library"));
            if stations_valid {
                let mut stations = cached.stations;
                helpers::append_station_action_items(&mut stations, state.queue.shuffle_undo_queue.is_some());
                state.stations = stations.clone();
                state.station_nav.columns.clear();
                state.station_nav.columns.push(crate::app::state::StationColumn::new(
                    None,
                    "Radio".to_string(),
                    stations,
                ));
                state.station_nav.focused_column = 0;
            } else if !cached.stations.is_empty() {
                tracing::warn!("Ignoring corrupted station cache ({} items, missing root identifiers)", cached.stations.len());
            }
            if !cached.station_children.is_empty() {
                state.station_children_cache = cached.station_children;
            }

            // All tracks + track-level artists
            if !cached.all_tracks.is_empty() {
                state.library.all_tracks = cached.all_tracks;
                tracing::debug!("Library switch: loaded {} cached tracks", state.library.all_tracks.len());
            }
            if !cached.track_artists.is_empty() {
                state.library.track_artists = cached.track_artists;
                tracing::debug!("Library switch: loaded {} cached track artists", state.library.track_artists.len());
            }

            // Artist aliases
            if !cached.artist_aliases.is_empty() {
                state.library.artist_aliases = cached.artist_aliases;
                state.library.album_display_artist = cached.album_display_artist;
                tracing::debug!("Library switch: loaded {} cached artist aliases", state.library.artist_aliases.len());
            } else if !state.library.all_tracks.is_empty() && !state.library.albums.is_empty() {
                state.build_artist_aliases();
            }

            // Compilation detection results
            if !cached.compilation_albums.is_empty() || !cached.compilation_artist_keys.is_empty() {
                state.library.compilations.albums = cached.compilation_albums;
                state.library.compilations.artist_keys = cached.compilation_artist_keys;
                state.library.compilations.track_artist_keys = cached.compilation_track_artist_keys;
                state.library.compilations.artist_map = cached.artist_compilation_map;
                state.library.compilations.single_artist = cached.single_artist_compilations;
                state.library.compilations.detected = true;
                // Re-build artist root items with compilation data
                let items = state.build_artist_root_items();
                state.artist_nav.update_root_items("artists", items);
            }

            // Trigger compilation detection from cached tracks if not already done
            helpers::maybe_detect_compilations(event_tx, state, client);

            // No auto-drill on launch — land the user on a clean
            // 2-column view (cat + artists root). They can drill
            // explicitly with Enter/Right and close cols with
            // Ctrl+W to come back to this layout cheaply.
            vec![]
        }
        Event::Preload(PreloadEvent::LibraryCacheLoadFailed { library_key }) => {
            state.library_loading = false;
            if state.active_library.as_ref() == Some(&library_key) {
                tracing::debug!("No cache for library {} - waiting for API preload", library_key);
            }
            vec![]
        }
        Event::Cache(CacheEvent::CacheSaved) => {
            state.cache_mgmt.save_in_progress = false;
            tracing::debug!("Periodic cache save completed");
            vec![]
        }
        Event::Cache(CacheEvent::CacheRefreshCompleted { category, changed }) => {
            state.cache_mgmt.background_refresh.remove(&category);
            state.cache_mgmt.dirty = true;
            // Data was refreshed from the server — update the per-category timestamp
            let now = crate::plex::CacheData::now();
            state.cache_mgmt.category_timestamps.insert(category, now);

            // Clear the "Refreshing X..." status message if it matches this category
            let refresh_msg = format!("Refreshing {}...", category.display_name());
            if state.notifications.status_message.as_ref() == Some(&refresh_msg) {
                state.clear_status();
            }

            if changed && helpers::is_viewing_category(&category, state) {
                state.set_toast(format!("{} updated", category.display_name()));
            }
            vec![]
        }
        #[cfg(feature = "tui")]
        Event::Mouse(mouse_event) => {
            super::mouse_input::handle_mouse(mouse_event, state)
        }
        Event::Visualizer(VisualizerEvent::WaveformGenerated { track_key, data }) => {
            if state.waveform.track_key.as_ref() == Some(&track_key) {
                state.waveform.data = Some(data);
                state.waveform.generating = false;
                state.waveform.error = None;
                tracing::debug!("Waveform generated for track: {}", track_key);
            }
            vec![]
        }
        Event::Visualizer(VisualizerEvent::WaveformFailed { track_key, error }) => {
            if state.waveform.track_key.as_ref() == Some(&track_key) {
                if state.waveform.retry_count < 3 {
                    // Silent retry: keep generating=true so UI shows "Generating..."
                    state.waveform.retry_count += 1;
                    let retry_num = state.waveform.retry_count;
                    let tx = event_tx.clone();
                    let tk = track_key.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(2 * retry_num as u64)).await;
                        let _ = tx.send(VisualizerEvent::WaveformRetry(tk).into()).await;
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
        Event::Visualizer(VisualizerEvent::WaveformCacheHit { track_key, data }) => {
            if state.waveform.track_key.as_ref() == Some(&track_key) {
                state.waveform.data = Some(data);
                state.waveform.generating = false;
                state.waveform.error = None;
                tracing::debug!("Waveform loaded from cache for track: {}", track_key);
            }
            vec![]
        }
        Event::Visualizer(VisualizerEvent::WaveformRetry(track_key)) => {
            // generating is still true from the failed attempt; clear it so
            // LoadWaveform's needs_generation check passes.
            if state.waveform.track_key.as_ref() == Some(&track_key)
                && state.waveform.data.is_none()
            {
                state.waveform.generating = false;
                vec![SystemAction::LoadWaveform.into()]
            } else {
                vec![]
            }
        }
        Event::Visualizer(VisualizerEvent::SpectrogramGenerated { track_key, data }) => {
            if state.spectrogram.track_key.as_ref() == Some(&track_key) {
                state.spectrogram.data = Some(data);
                state.spectrogram.generating = false;
                state.spectrogram.error = None;
                tracing::debug!("Spectrogram generated for track: {}", track_key);
            }
            vec![]
        }
        Event::Visualizer(VisualizerEvent::SpectrogramFailed { track_key, error }) => {
            if state.spectrogram.track_key.as_ref() == Some(&track_key) {
                state.spectrogram.generating = false;
                // Only set error for real failures, not empty signals from cache miss
                if !error.is_empty() {
                    state.spectrogram.error = Some(error.clone());
                    tracing::warn!("Spectrogram failed for {}: {}", track_key, error);
                }
            }
            vec![]
        }
        Event::Visualizer(VisualizerEvent::SpectrogramCacheHit { track_key, data }) => {
            if state.spectrogram.track_key.as_ref() == Some(&track_key) {
                state.spectrogram.data = Some(data);
                state.spectrogram.generating = false;
                state.spectrogram.error = None;
                tracing::debug!("Spectrogram loaded from cache for track: {}", track_key);
            }
            vec![]
        }
        Event::Radio(RadioEvent::StationTracksLoaded { station_key, station_title, tracks, time_travel_decades }) => {
            state.stations_loading = false;
            state.station_nav.loading = false;

            if tracks.is_empty() {
                state.set_error(format!("{}: returned no tracks (check server logs; Sonic Analysis may be required)", station_title));
            } else {
                // Start playing the station in Radio mode
                state.playback_mode = PlaybackMode::Radio;
                state.queue.selected.clear();
                state.radio.clear();
                state.radio.active_station = Some(crate::app::state::ActiveStation {
                    key: station_key,
                    title: station_title.clone(),
                });
                state.radio.tracks = tracks;
                state.radio.track_index = Some(0);

                // Capture ancestor keys for ♪ indicator on parent categories
                state.radio.playing_station_ancestors = state.station_nav.columns[1..]
                    .iter()
                    .filter_map(|col| col.key.clone())
                    .collect();

                // Time Travel Radio initialization
                if !time_travel_decades.is_empty() {
                    state.radio.time_travel_decades = time_travel_decades;
                    state.radio.time_travel_index = 3;
                    tracing::info!("Time Travel Radio: initialized with {} decades, next fetch from index 3",
                        state.radio.time_travel_decades.len());
                }

                state.list_state.queue_index = 0;
                state.set_view(View::Queue);
                state.set_status(format!("Playing {} ({} tracks)", station_title, state.radio.tracks.len()));
                return vec![RadioAction::PlayCurrentRadioTrack.into()];
            }
            vec![]
        }
        Event::Radio(RadioEvent::StationLoadFailed { station_key: _, error }) => {
            state.stations_loading = false;
            state.station_nav.loading = false;
            state.set_error(error);
            vec![]
        }
        Event::Radio(RadioEvent::StationChildrenLoaded { station_key, station_title, children }) => {
            state.stations_loading = false;
            state.station_nav.loading = false;

            // Cache children for instant loading next time
            state.station_children_cache.insert(station_key.clone(), children.clone());
            state.cache_mgmt.dirty = true;

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
        // Radio track fetching completed (background)
        Event::Radio(RadioEvent::RadioTracksLoaded { tracks, time_travel_index }) => {
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
                return vec![PlaybackAction::Next.into()];
            }

            vec![]
        }

        // Playlist tracks loaded (non-blocking)
        Event::Radio(RadioEvent::PlaylistTracksForMillerLoaded { playlist_key, tracks }) => {
            state.playlist_nav.loading = false;
            // Cache always — even stale results help future hits.
            state.playlist_tracks_cache.insert(
                playlist_key.clone(),
                crate::plex::CachedPlaylistTracks::new(tracks.clone()),
            );
            state.cache_mgmt.dirty = true;

            // Race guard: when the user clicks several playlists in
            // quick succession, multiple `LoadPlaylistTracksForMiller`
            // tokio tasks fly off in parallel and their replies can
            // land out-of-order. Without this check, each reply
            // calls `drill_column → push_column` and the right side
            // of the Miller stack accumulates "tracks for playlist B
            // / tracks for playlist C / tracks for playlist A", with
            // the column 0 highlight no longer matching column 1's
            // header. Drop the reply if the user has since selected
            // a different playlist.
            let selected_key: Option<String> = state.playlist_nav.columns.first()
                .and_then(|c| c.items.get(c.selected_index))
                .map(|item| item.key().to_string());
            if selected_key.as_deref() != Some(playlist_key.as_str()) {
                tracing::debug!(
                    "Playlist tracks reply for {} arrived after user navigated to {:?} — discarding",
                    playlist_key, selected_key,
                );
                return vec![];
            }

            // Always anchor the new tracks column at index 1 of the
            // playlist nav (root is column 0). Force focus to 0 first
            // so `push_column`'s `truncate_right` drops any stale
            // child columns from a previous selection before pushing.
            state.playlist_nav.focused_column = 0;

            let playlist_name = state.playlist_nav.focused()
                .and_then(|c| c.selected_item())
                .map(|item| item.title().to_string())
                .unwrap_or_default();
            // Header carries the track count (formerly part of each
            // playlist row's label, like "Soundbombing (59 tracks)").
            // Now playlist rows show just the title and the count
            // appears in the resulting tracks column header.
            let n = tracks.len();
            let count_str = if n == 1 { "1 track".to_string() } else { format!("{} tracks", n) };
            let title = if playlist_name.is_empty() {
                format!("tracks \u{2014} {}", count_str)
            } else {
                format!("{} \u{2014} {}", playlist_name, count_str)
            };
            let items = crate::app::state::BrowseItem::from_tracks(&tracks);
            let mut col = crate::app::state::BrowseColumn::new_with_tracks(title, items, tracks);
            // GUI: render a "Play Playlist" button alongside the
            // header (same affordance as Play Album on a tracks
            // column). play_all_label means "play `col.tracks`
            // directly" — exactly the right semantics for a playlist.
            col.play_all_label = Some("Play Playlist".to_string());
            state.playlist_nav.push_column(col);
            vec![]
        }
        Event::Radio(RadioEvent::PlaylistTracksForMillerFailed { playlist_key: _, error }) => {
            state.playlist_nav.loading = false;
            state.set_error(format!("Failed to load playlist: {}", error));
            vec![]
        }

        // First page of a lazy-loaded playlist column. Same as the
        // legacy "all tracks loaded" handler above, but stamps lazy
        // state on the column so `LoadMorePlaylistTracks` can fill in
        // the tail as the user scrolls.
        Event::Radio(RadioEvent::PlaylistFirstPageLoaded { playlist_key, tracks, total }) => {
            state.playlist_nav.loading = false;
            // Race guard: drop replies for a playlist the user has
            // already navigated away from (mirrors the original
            // PlaylistTracksForMillerLoaded path).
            let selected_key: Option<String> = state.playlist_nav.columns.first()
                .and_then(|c| c.items.get(c.selected_index))
                .map(|item| item.key().to_string());
            if selected_key.as_deref() != Some(playlist_key.as_str()) {
                tracing::debug!(
                    "Playlist first-page reply for {} arrived after user navigated to {:?} — discarding",
                    playlist_key, selected_key,
                );
                return vec![];
            }

            state.playlist_nav.focused_column = 0;

            let playlist_name = state.playlist_nav.focused()
                .and_then(|c| c.selected_item())
                .map(|item| item.title().to_string())
                .unwrap_or_default();
            // Header reports the SERVER total, not the partial in-memory
            // count, so the user knows what they'll eventually scroll to.
            let n_visible = tracks.len();
            let total_n = total.map(|t| t as usize).unwrap_or(n_visible);
            let count_str = if total_n == 1 {
                "1 track".to_string()
            } else if (total_n as usize) > n_visible {
                format!("{} tracks", total_n)
            } else {
                format!("{} tracks", n_visible)
            };
            let title = if playlist_name.is_empty() {
                format!("tracks \u{2014} {}", count_str)
            } else {
                format!("{} \u{2014} {}", playlist_name, count_str)
            };
            let items = crate::app::state::BrowseItem::from_tracks(&tracks);
            let mut col = crate::app::state::BrowseColumn::new_with_tracks(title, items, tracks);
            col.play_all_label = Some("Play Playlist".to_string());
            // Apply this playlist's saved view toggles, if any. The
            // mirror lives on `state.playlist_views` (sync'd from
            // `config.ui.library_view_settings` at boot and on every
            // SavePlaylistView). Defaults are no-grouping +
            // no-artwork — same as a fresh column.
            let saved_view = state.active_library.as_ref()
                .and_then(|lib| state.playlist_views.get(lib))
                .and_then(|m| m.get(&playlist_key))
                .copied()
                .unwrap_or_default();
            if saved_view.show_artwork {
                col.artwork_visible = true;
            }
            // Mark the column as lazy so the GUI scroll handler knows
            // to ask for more pages as the user scrolls down.
            col.lazy = Some(crate::app::state::LazyPlaylist {
                key: playlist_key.clone(),
                total,
                loading: false,
            });
            // Apply group-by-album AFTER the column is wired up.
            // group_by_album resets selected_index to 0 and rebuilds
            // items, so we have to set the column up first.
            if saved_view.group_by_album {
                col.group_by_album();
            }
            state.playlist_nav.push_column(col);

            // If artwork was restored ON, kick a full-list art batch
            // so every album cover loads (mirrors `toggle_artwork`'s
            // ON-path). The col was just pushed; read it back to
            // collect art keys.
            let mut follow_ups: Vec<Action> = Vec::new();
            if saved_view.show_artwork {
                let batch = super::dispatch_miller::collect_all_art_to_load(
                    state.playlist_nav.columns.last(),
                    &state.artwork.grid_cache,
                    &state.artwork.grid_pending,
                );
                if !batch.is_empty() {
                    follow_ups.push(SystemAction::LoadAlbumArt(batch).into());
                }
            }
            // If grouping was restored AND there are more pages, the
            // user expects all the albums to show — kick the next
            // page fetch so pagination can chain to the end.
            if saved_view.group_by_album {
                let next = state.playlist_nav.columns.last().and_then(|col| {
                    let lazy = col.lazy.as_ref()?;
                    let total = lazy.total? as usize;
                    if !lazy.loading && col.tracks.len() < total {
                        Some((lazy.key.clone(), col.tracks.len() as u32))
                    } else {
                        None
                    }
                });
                if let Some((pk, off)) = next {
                    follow_ups.push(crate::app::action::MillerAction::LoadMorePlaylistTracks {
                        playlist_key: pk, offset: off,
                    }.into());
                }
            }
            follow_ups
        }

        // Subsequent page — append to the existing column.
        Event::Radio(RadioEvent::PlaylistMorePageLoaded { playlist_key, tracks, total }) => {
            let Some(col) = state.playlist_nav.columns.iter_mut()
                .find(|c| c.lazy.as_ref().map(|l| l.key == *playlist_key).unwrap_or(false))
            else {
                tracing::debug!(
                    "Playlist more-page reply for {} arrived after column was replaced — discarding",
                    playlist_key,
                );
                return vec![];
            };
            // Refresh total in case the smart playlist shifted between
            // pages (rare but possible for "Recently Added" if a track
            // was added mid-scroll).
            if let Some(lazy) = col.lazy.as_mut() {
                if let Some(t) = total { lazy.total = Some(t); }
                lazy.loading = false;
            }
            // Append both the BrowseItem rows and the raw Track records
            // so the column stays consistent for both render and play.
            let new_items = crate::app::state::BrowseItem::from_tracks(&tracks);
            col.items.extend(new_items);
            col.tracks.extend(tracks);

            // If the user had grouping enabled when the toggle fired,
            // re-run the grouping over the now-larger track list so
            // the new tracks slot into the existing albums (or open
            // new album rows). `group_by_album` resets `selected_index`
            // to 0, which would visibly snap the user's selection back
            // to the top of the list every time a page lands on a
            // long playlist — capture the selected album's key first
            // so we can restore the selection by key after.
            let was_grouped = col.grouped_by_album;
            if was_grouped {
                let prev_key: Option<String> = col.items.get(col.selected_index)
                    .and_then(|it| match it {
                        crate::app::state::BrowseItem::Album { key, .. } => Some(key.clone()),
                        _ => None,
                    });
                col.ungroup_by_album();
                col.group_by_album();
                if let Some(prev_key) = prev_key {
                    if let Some(idx) = col.items.iter().position(|it| matches!(
                        it,
                        crate::app::state::BrowseItem::Album { key, .. } if key == &prev_key
                    )) {
                        col.selected_index = idx;
                    }
                }
            }
            let artwork_on = col.artwork_visible;
            // Continue paginating until the column is fully loaded so
            // grouping / artwork covers the whole playlist.
            let next_offset = col.lazy.as_ref().and_then(|lazy| {
                let total = lazy.total? as usize;
                if !lazy.loading && col.tracks.len() < total {
                    Some(col.tracks.len() as u32)
                } else {
                    None
                }
            });

            let mut follow_ups = vec![];
            if artwork_on {
                let batch = super::dispatch_miller::collect_all_art_to_load(
                    Some(&*col),
                    &state.artwork.grid_cache,
                    &state.artwork.grid_pending,
                );
                if !batch.is_empty() {
                    follow_ups.push(SystemAction::LoadAlbumArt(batch).into());
                }
            }
            if let Some(off) = next_offset {
                follow_ups.push(crate::app::action::MillerAction::LoadMorePlaylistTracks {
                    playlist_key, offset: off,
                }.into());
            }
            follow_ups
        }

        Event::Radio(RadioEvent::PlaylistMorePageFailed { playlist_key, error }) => {
            // Don't surface the error inline — the user has the
            // already-loaded portion of the playlist visible. Just
            // release the loading lock so a future scroll can retry.
            if let Some(lazy) = state.playlist_nav.columns.iter_mut()
                .filter_map(|c| c.lazy.as_mut())
                .find(|l| l.key == *playlist_key)
            {
                lazy.loading = false;
            }
            tracing::warn!("Playlist more-page fetch failed for {}: {}", playlist_key, error);
            vec![]
        }

        // Playlist tracks preloaded in background
        Event::Preload(PreloadEvent::PlaylistTracksPreloaded { playlist_key, tracks }) => {
            if !tracks.is_empty() {
                state.playlist_tracks_cache.insert(playlist_key, crate::plex::CachedPlaylistTracks::new(tracks));
                state.cache_mgmt.dirty = true;
            }
            vec![]
        }

        // DJ mode tracks ready (continuous modes)
        Event::Ui(UiEvent::DjTracksReady { tracks, insert_next, error }) => {
            vec![RadioAction::DjModeTracksReady(tracks, insert_next, error).into()]
        }
        // DJ mode batch ready (inserter modes)
        Event::Ui(UiEvent::DjBatchReady { inserts }) => {
            vec![RadioAction::DjModeBatchReady(inserts).into()]
        }
        // Queue remix batch ready
        Event::Ui(UiEvent::RemixBatchReady { inserts }) => {
            vec![QueueAction::RemixBatchReady(inserts).into()]
        }
        Event::Ui(UiEvent::RemixDoppelgangerReady { replacements }) => {
            vec![QueueAction::RemixDoppelgangerReady(replacements).into()]
        }

        // Multi-artist radio complete
        Event::Ui(UiEvent::ArtistRadioComplete { tracks }) => {
            vec![SettingsAction::ArtistRadioComplete(tracks).into()]
        }

        // Artist bio popup loaded
        Event::Ui(UiEvent::ArtistBioLoaded { artist_name, bio, thumb }) => {
            if let Some(ref mut popup) = state.popups.artist_bio {
                popup.loading = false;
                popup.artist_name = artist_name;
                popup.bio = bio;
                popup.scroll = 0;
                popup.artwork_thumb = thumb;
            }
            vec![]
        }

        // Artist bio artwork loaded
        Event::Ui(UiEvent::ArtistBioArtworkLoaded { data, thumb }) => {
            if let Some(ref mut popup) = state.popups.artist_bio {
                popup.artwork_data = Some(data);
                popup.artwork_thumb = Some(thumb);
            }
            vec![]
        }

        // Inline list filter completed
        Event::Ui(UiEvent::ListFilterCompleted { version, results }) => {
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
        Event::Remote(RemoteEvent::PlayersDiscovered(players)) => {
            state.remote.discovering = false;
            state.remote.players = players;
            let count = state.remote.players.len();
            if count > 0 {
                state.set_status(format!("Found {} player{}", count, if count == 1 { "" } else { "s" }));
            } else {
                state.set_status("No remote players found".to_string());
            }
            vec![]
        }
        Event::Remote(RemoteEvent::PlayerDiscoveryFailed(err)) => {
            state.remote.discovering = false;
            state.set_error(format!("Player discovery failed: {}", err));
            vec![]
        }
        Event::Remote(RemoteEvent::RemotePlayerStatus { session_found, playing, position_ms: _, track_key: _, finished }) => {
            state.remote.playback.last_poll = Some(std::time::Instant::now());

            if let crate::app::state::OutputTarget::Remote { .. } = &state.remote.output_target {
                if !session_found {
                    return vec![];
                }

                if finished {
                    return vec![PlaybackAction::Next.into()];
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
        Event::Remote(RemoteEvent::RemotePlayerError(err)) => {
            tracing::warn!("Remote player error: {}", err);
            vec![]
        }

        _ => vec![],
    }
}

/// Generic preload handler for tag-style sections. Stores the items in
/// the matching `library` field, clears the preload tracker, and — if
/// the user is currently sitting on this section with `tag_nav.loading`
/// — dispatches a fresh `RefreshTagView` so the empty list gets
/// replaced with the just-loaded data.
fn handle_tag_preload(
    state: &mut AppState,
    library_key: &str,
    preload_label: &str,
    section: BrowseCategory,
    items: Vec<crate::plex::models::Genre>,
) -> Vec<Action> {
    if state.active_library.as_deref() != Some(library_key) {
        tracing::debug!("Ignoring stale {} preload for library {}", preload_label, library_key);
        return vec![];
    }

    let already_present = !state.tag_list_for(section).is_empty();
    if !already_present {
        let count = items.len();
        match section {
            BrowseCategory::AlbumGenres => state.library.album_genres = items,
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
        tracing::debug!("{} preloaded: {} items", preload_label, count);
    }

    state.cache_mgmt.preloads_in_progress.remove(preload_label);
    if state.cache_mgmt.preloads_in_progress.is_empty() {
        state.cache_mgmt.preloads_total = 0;
    }

    // If the user is sitting on this section and the nav is empty/loading,
    // refresh the view so the just-loaded items appear.
    if state.browse_category == section && state.tag_nav.loading {
        return vec![BrowseAction::RefreshTagView.into()];
    }
    vec![]
}
