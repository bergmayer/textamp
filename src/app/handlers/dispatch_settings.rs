//! Settings dispatch handlers: Logout, AuthSignIn, AuthSelectServer, OpenSettings,
//! SaveCredentials, SettingsSelect, SettingsSignIn, SettingsDiscoverServers, SelectServer,
//! SelectLibrary, SaveSettings, ClearCache, and Adventure actions.

use crate::app::{Action, AppState, Event};
use crate::app::event_loop::PreloadType;
use crate::app::state::{ConnectionState, PlayStatus, PlaybackMode, QueueSortMode, SettingsSection, View};
use crate::api::{PlexAuth, PlexClient};
use crate::audio::AudioPlayer;
use crate::cache::LibraryCache;
use crate::config::Config;
use crate::util::truncate_str;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Dispatch settings/auth/adventure actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    config: &mut Config,
    action: Action,
    state: &mut AppState,
    client: &mut PlexClient,
    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    let mut follow_ups = vec![];

    match action {
        Action::Logout => {
            // Clear auth token
            if let Err(e) = PlexAuth::delete_token() {
                tracing::warn!("Failed to delete auth token: {}", e);
            }

            // Clear all cached data
            if let Some(cache) = LibraryCache::new() {
                if let Err(e) = cache.clear_all() {
                    tracing::warn!("Failed to clear cache on logout: {}", e);
                }
            }

            // Reset connection and display state
            state.connection = ConnectionState::Disconnected;
            state.active_library = None;
            state.libraries.clear();
            state.available_servers.clear();
            state.connected_server_url = None;
            state.active_server_id = None;
            state.artwork_cache_stats = None;

            // Clear browse data
            state.artists.clear();
            state.albums.clear();
            state.playlists.clear();
            state.genres.clear();
            state.artist_genres.clear();
            state.album_genres.clear();
            state.moods.clear();
            state.styles.clear();
            state.stations.clear();
            state.recently_added_albums.clear();
            state.selected_artist_albums.clear();
            state.selected_album_tracks.clear();
            state.genre_albums.clear();
            state.folder_state = None;
            state.folder_contents_cache.clear();
            state.subfolder_preload_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            state.subfolder_preload_active = false;

            // Clear playback state
            state.queue.clear();
            state.queue_index = None;
            state.queue_original.clear();

            // Clear navigation state
            state.station_nav.columns.clear();
            state.station_nav.focused_column = 0;
            state.list_state.reset();

            // Clear session/runtime state
            state.category_timestamps.clear();
            state.background_refresh_in_progress.clear();
            state.plex_session_id = None;
            state.album_art_cache.clear();
            state.album_art_pending.clear();
            state.waveform = Default::default();
            state.search_results = None;
            state.playlist_tracks_cache.clear();

            // Stop playback and flush track cache
            audio.stop();
            audio.track_cache.flush();
            state.playback.status = PlayStatus::Stopped;

            // Clear all server-related config (keep app settings like theme/playback)
            config.plex = crate::config::PlexConfig::default();
            config.libraries.default_library = None;
            config.libraries.selected_server = None;
            config.general.default_library = None;
            if let Err(e) = crate::config::save_config(config) {
                tracing::warn!("Failed to save config after logout: {}", e);
            }

            state.set_status("Signed out.".to_string());
        }
        Action::AuthSignIn => {
            use crate::app::state::AuthStep;
            // Authenticate with username/password entered in auth screen login form
            let username = state.auth_state.username_input.clone();
            let password = state.auth_state.password_input.clone();

            if username.is_empty() || password.is_empty() {
                state.auth_state.error_message = Some("Please enter username and password".to_string());
            } else {
                state.auth_state.step = AuthStep::Authenticating;
                state.auth_state.error_message = None;
                let event_tx = event_tx.clone();

                tokio::spawn(async move {
                    let auth = PlexAuth::new();

                    match auth.authenticate_password(&username, &password).await {
                        Ok(token) => {
                            // Verify token and get user info
                            match auth.verify_token(&token).await {
                                Ok(user) => {
                                    // Get client_identifier BEFORE saving (save_token consumes it)
                                    let client_id = auth.client_identifier().to_string();

                                    // Save token (not password!)
                                    if let Err(e) = auth.save_token(&token, Some(&user)) {
                                        tracing::warn!("Failed to save token: {}", e);
                                    }

                                    // Get servers
                                    let servers = auth.get_servers(&token).await.unwrap_or_default();

                                    // Send servers ready event (will auto-select or show selection)
                                    let has_plex_pass = user.has_plex_pass();
                                    let _ = event_tx.send(Event::AuthServersReady {
                                        token,
                                        username: user.username,
                                        servers,
                                        client_identifier: client_id,
                                        has_plex_pass,
                                    }).await;
                                }
                                Err(e) => {
                                    let _ = event_tx.send(Event::AuthLoginFailed(
                                        format!("Token verification failed: {}", e)
                                    )).await;
                                }
                            }
                        }
                        Err(e) => {
                            let _ = event_tx.send(Event::AuthLoginFailed(
                                format!("Invalid username or password")
                            )).await;
                            tracing::error!("Auth error: {}", e);
                        }
                    }
                });

                // Clear password from memory immediately
                state.auth_state.password_input.clear();
            }
        }
        Action::AuthSelectServer => {
            use crate::app::state::AuthStep;
            // Select server from the server selection list
            if let Some(server) = state.available_servers.get(state.auth_state.server_index) {
                // Get the token and client_identifier that were stored when servers were received
                let token = client.token().map(|s| s.to_string());
                let client_id = client.client_identifier().to_string();

                if let Some(token) = token {
                    state.auth_state.step = AuthStep::Connecting;
                    let username = state.settings_state.username_input.clone();
                    let servers = state.available_servers.clone();
                    let server_clone = server.clone();
                    let event_tx = event_tx.clone();

                    // Find working connection URL (tests connectivity)
                    let has_plex_pass = state.auth_state.has_plex_pass;
                    tokio::spawn(async move {
                        if let Some(url) = helpers::find_working_connection(&server_clone, &token, &client_id).await {
                            let _ = event_tx.send(Event::AuthSuccess {
                                token,
                                username,
                                server_url: url,
                                servers,
                                client_identifier: client_id,
                                has_plex_pass,
                            }).await;
                        } else {
                            let _ = event_tx.send(Event::AuthFailed(
                                format!("Could not connect to {} - all connection attempts failed", server_clone.name)
                            )).await;
                        }
                    });
                } else {
                    state.auth_state.error_message = Some("Authentication token not found".to_string());
                    state.auth_state.step = AuthStep::Login;
                }
            }
        }
        Action::OpenSettings => {
            state.view = View::Settings;
            state.settings_state.section = SettingsSection::Account;
            state.settings_state.item_index = 0;
            state.settings_state.signing_in = false;

            // Get username from connection state first (most reliable), then StoredAuth, then config
            state.settings_state.username_input = match &state.connection {
                ConnectionState::Connected { username, .. } => username.clone(),
                _ => PlexAuth::load_token()
                    .and_then(|s| s.username)
                    .or_else(|| config.plex.username.clone())
                    .unwrap_or_default(),
            };

            // Password field no longer used - token-based auth only
            state.settings_state.password_input = String::new();
            state.settings_state.editing_credential = None;

            // If servers list is empty but we're connected, trigger discovery
            if state.available_servers.is_empty() {
                // Use stored auth to get the correct client_identifier
                if let Some(stored) = PlexAuth::load_token() {
                    state.settings_state.discovering_servers = true;
                    let event_tx = event_tx.clone();
                    tokio::spawn(async move {
                        let auth = PlexAuth::from_stored_auth(&stored);
                        match auth.get_servers(&stored.token).await {
                            Ok(servers) => {
                                let _ = event_tx.send(Event::ServersDiscovered(servers)).await;
                            }
                            Err(e) => {
                                let _ = event_tx.send(Event::ServerDiscoveryFailed(e.to_string())).await;
                            }
                        }
                    });
                }
            }
        }
        Action::SaveCredentials => {
            // Save username to config file (for display purposes only)
            // Authentication is handled via stored tokens, not passwords
            let mut updated_config = config.clone();
            updated_config.plex.username = if state.settings_state.username_input.is_empty() {
                None
            } else {
                Some(state.settings_state.username_input.clone())
            };
            if let Err(e) = crate::config::save_config(&updated_config) {
                state.set_error(format!("Failed to save: {}", e));
            } else {
                state.set_status("Username saved.".to_string());
            }
        }
        Action::SettingsSelect => {
            match state.settings_state.section {
                SettingsSection::Account => {
                    if state.settings_state.signing_in {
                        // In sign-in mode: 0=username, 1=password, 2=sign in, 3+=servers
                        let server_index = state.settings_state.item_index.saturating_sub(3);
                        if let Some(server) = state.available_servers.get(server_index) {
                            let server_id = server.client_identifier.clone();
                            tracing::info!("Selected server: {}", server.name);
                            follow_ups.push(Action::SelectServer(server_id));
                        }
                    } else if matches!(state.connection, ConnectionState::Connected { .. }) {
                        use crate::app::state::{ConfirmDialog, ConfirmAction};
                        let lib_count = state.libraries.len();
                        let idx = state.settings_state.item_index;
                        if idx < lib_count {
                            // Activate selected library
                            if let Some(lib) = state.libraries.get(idx) {
                                let lib_key = lib.key.clone();
                                follow_ups.push(Action::SelectLibrary(lib_key));
                            }
                        } else {
                            match idx - lib_count {
                                0 => {
                                    state.confirm_dialog = Some(ConfirmDialog {
                                        title: "Clear Library Cache".to_string(),
                                        message: "Clear all cached library data and reload from server?".to_string(),
                                        on_confirm: ConfirmAction::ClearLibraryCache,
                                    });
                                }
                                1 => {
                                    state.confirm_dialog = Some(ConfirmDialog {
                                        title: "Clear Artwork Cache".to_string(),
                                        message: "Delete all cached album artwork from disk?".to_string(),
                                        on_confirm: ConfirmAction::ClearArtworkCache,
                                    });
                                }
                                2 => {
                                    state.confirm_dialog = Some(ConfirmDialog {
                                        title: "Clear Subfolder Cache".to_string(),
                                        message: "Clear all cached subfolder contents?".to_string(),
                                        on_confirm: ConfirmAction::ClearSubfolderCache,
                                    });
                                }
                                3 => {
                                    // Toggle crawl: start if not running, stop if running
                                    if state.subfolder_preload_active {
                                        follow_ups.push(Action::StopSubfolderCrawl);
                                    } else {
                                        follow_ups.push(Action::StartSubfolderCrawl);
                                    }
                                }
                                4 => follow_ups.push(Action::Logout),
                                _ => {}
                            }
                        }
                    } else {
                        // Not signed in: 0=Sign In
                        if state.settings_state.item_index == 0 {
                            state.settings_state.signing_in = true;
                            state.settings_state.item_index = 0;
                        }
                    }
                }
                SettingsSection::About => {
                    // Apply selected theme
                    if let Some(theme_name) = crate::ui::theme::ThemeName::all().get(state.settings_state.item_index) {
                        state.theme = *theme_name;
                        crate::ui::theme::set_theme(state.theme);
                        state.set_status(format!("Theme: {}", state.theme.display_name()));

                        // Persist theme to config
                        config.ui.theme = state.theme.config_name().to_string();
                        if let Err(e) = crate::config::save_config(config) {
                            tracing::warn!("Failed to save theme preference: {}", e);
                        }
                    }
                }
            }
        }
        Action::SettingsSignIn => {
            // Authenticate with username/password entered in settings
            let username = state.settings_state.username_input.clone();
            let password = state.settings_state.password_input.clone();

            if username.is_empty() || password.is_empty() {
                state.set_error("Please enter username and password".to_string());
            } else if state.settings_state.discovering_servers {
                // Already signing in
            } else {
                state.settings_state.discovering_servers = true;
                let event_tx = event_tx.clone();
                let server_url = config.plex.server_url.clone();

                tokio::spawn(async move {
                    let auth = PlexAuth::new();

                    match auth.authenticate_password(&username, &password).await {
                        Ok(token) => {
                            // Verify token and get user info
                            match auth.verify_token(&token).await {
                                Ok(user) => {
                                    // Get client_identifier BEFORE saving
                                    let client_id = auth.client_identifier().to_string();

                                    // Save token (not password!)
                                    if let Err(e) = auth.save_token(&token, Some(&user)) {
                                        tracing::warn!("Failed to save token: {}", e);
                                    }

                                    // Get servers
                                    let servers = auth.get_servers(&token).await.unwrap_or_default();

                                    // Multiple servers and no configured URL: show server selection
                                    if server_url.is_empty() && servers.len() > 1 {
                                        let has_plex_pass = user.has_plex_pass();
                                        let _ = event_tx.send(Event::AuthServersReady {
                                            token,
                                            username: user.username,
                                            servers,
                                            client_identifier: client_id,
                                            has_plex_pass,
                                        }).await;
                                        return;
                                    }

                                    // Single server or configured URL: connect directly
                                    let final_url = if server_url.is_empty() {
                                        helpers::find_working_connection_from_servers(&servers, &token, &client_id).await
                                    } else {
                                        Some(server_url)
                                    };

                                    if let Some(url) = final_url {
                                        let has_plex_pass = user.has_plex_pass();
                                        let _ = event_tx.send(Event::AuthSuccess {
                                            token,
                                            username: user.username,
                                            server_url: url,
                                            servers,
                                            client_identifier: client_id,
                                            has_plex_pass,
                                        }).await;
                                    } else {
                                        // No working server connection available
                                        let _ = event_tx.send(Event::ServersDiscovered(servers)).await;
                                    }
                                }
                                Err(e) => {
                                    let _ = event_tx.send(Event::AuthFailed(
                                        format!("Token verification failed: {}", e)
                                    )).await;
                                }
                            }
                        }
                        Err(e) => {
                            let _ = event_tx.send(Event::AuthFailed(
                                format!("Authentication failed: {}", e)
                            )).await;
                        }
                    }
                });

                // Clear password immediately from memory (don't store it)
                state.settings_state.password_input.clear();
            }
        }
        Action::SettingsDiscoverServers => {
            // Use stored auth to get both token and client_identifier
            if let Some(stored) = PlexAuth::load_token() {
                state.settings_state.discovering_servers = true;
                let event_tx = event_tx.clone();
                tokio::spawn(async move {
                    let auth = PlexAuth::from_stored_auth(&stored);
                    match auth.get_servers(&stored.token).await {
                        Ok(servers) => {
                            let _ = event_tx.send(Event::ServersDiscovered(servers)).await;
                        }
                        Err(e) => {
                            let _ = event_tx.send(Event::ServerDiscoveryFailed(e.to_string())).await;
                        }
                    }
                });
            } else {
                state.set_error("No authentication token available".to_string());
            }
        }
        Action::SelectServer(server_id) => {
            // Find server and try to connect
            if let Some(server) = state.available_servers.iter().find(|s| s.client_identifier == server_id) {
                // Get token for connection testing
                let token = client.token().map(|s| s.to_string());

                if let Some(token) = token {
                    let server_clone = server.clone();
                    let event_tx = event_tx.clone();
                    let client_id = client.client_identifier().to_string();

                    // Find working connection URL (tests connectivity)
                    tokio::spawn(async move {
                        if let Some(url) = helpers::find_working_connection(&server_clone, &token, &client_id).await {
                            let _ = event_tx.send(Event::ServerConnectionSucceeded {
                                server_name: server_clone.name.clone(),
                                url,
                            }).await;
                        } else {
                            let _ = event_tx.send(Event::ServerConnectionFailed {
                                server_name: server_clone.name.clone(),
                            }).await;
                        }
                    });

                    state.set_status(format!("Testing connections to {}...", server.name));
                } else {
                    state.set_error("No authentication token available".to_string());
                }
            }
        }
        Action::SelectLibrary(lib_key) => {
            // Switch to the selected library
            if state.active_library.as_ref() != Some(&lib_key) {
                state.active_library = Some(lib_key.clone());

                // Clear all current data and UI state
                state.artists.clear();
                state.albums.clear();
                state.playlists.clear();
                state.genres.clear();
                state.artist_genres.clear();
                state.album_genres.clear();
                state.moods.clear();
                state.styles.clear();
                state.stations.clear();
                state.recently_added_albums.clear();
                state.selected_artist_albums.clear();
                state.selected_album_tracks.clear();
                state.folder_state = None;
                state.folder_contents_cache.clear();
                state.subfolder_preload_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                state.subfolder_preload_active = false;
                state.playlist_tracks_cache.clear();
                state.list_state.reset();

                // Clear cache timestamps (old library's values must not leak to new library)
                state.category_timestamps.clear();
                state.cache_dirty = false;

                // Clear Miller column navigation states
                state.artist_nav = crate::app::state::BrowseNavigationState::new();
                state.genre_nav = crate::app::state::BrowseNavigationState::new();
                state.playlist_nav = crate::app::state::BrowseNavigationState::new();
                state.station_nav = crate::app::state::StationNavigationState::new();

                // Report playback stop to Plex before switching libraries
                if state.playback.status != PlayStatus::Stopped {
                    if let Some(track) = state.current_track().cloned() {
                        helpers::report_playback_stop_to_plex(
                            &track, state.playback.position_ms, false,
                            state.plex_session_id.clone(), client,
                        );
                    }
                }

                // Stop playback, flush track cache, and clear queue (tracks belong to the old library)
                audio.stop();
                audio.track_cache.flush();
                state.playback.status = PlayStatus::Stopped;
                state.playback.position_ms = 0;
                state.playback.duration_ms = 0;
                state.queue.clear();
                state.queue_index = None;
                state.queue_original.clear();
                state.radio.clear();
                state.playback_mode = PlaybackMode::Queue;
                state.adventure = crate::app::state::AdventureState::default();

                // Find library name for status message
                let lib_name = state.libraries.iter()
                    .find(|l| l.key == lib_key)
                    .map(|l| l.title.clone())
                    .unwrap_or_else(|| lib_key.clone());

                // Show loading indicator and load cache in background
                state.library_loading = true;

                let tx = event_tx.clone();
                let bg_lib_key = lib_key.clone();
                tokio::task::spawn_blocking(move || {
                    let result = LibraryCache::new().and_then(|cache| cache.load(&bg_lib_key));
                    let rt = tokio::runtime::Handle::current();
                    match result {
                        Some(cached) => {
                            rt.block_on(async {
                                let _ = tx.send(Event::LibraryCacheLoaded {
                                    library_key: bg_lib_key,
                                    cached: Box::new(cached),
                                }).await;
                            });
                        }
                        None => {
                            rt.block_on(async {
                                let _ = tx.send(Event::LibraryCacheLoadFailed {
                                    library_key: bg_lib_key,
                                }).await;
                            });
                        }
                    }
                });

                // Refresh from API in background
                helpers::preload_all_library_data(event_tx, &lib_key, &lib_name, client);

                state.set_status(format!("Switched to {}", lib_name));

                // Auto-save the default library
                follow_ups.push(Action::SaveSettings);
            }
        }
        Action::SelectLibraryOnServer(lib_key, server_id) => {
            // Switch to a library on a different server
            // First, find the server and connect to it
            if let Some(server) = state.available_servers.iter().find(|s| s.client_identifier == server_id).cloned() {
                let token = client.token().map(|s| s.to_string());

                if let Some(token) = token {
                    // Clear all current data (same as SelectLibrary but more thorough)
                    state.artists.clear();
                    state.albums.clear();
                    state.playlists.clear();
                    state.genres.clear();
                    state.artist_genres.clear();
                    state.album_genres.clear();
                    state.moods.clear();
                    state.styles.clear();
                    state.stations.clear();
                    state.recently_added_albums.clear();
                    state.selected_artist_albums.clear();
                    state.selected_album_tracks.clear();
                    state.folder_state = None;
                    state.folder_contents_cache.clear();
                    state.subfolder_preload_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    state.subfolder_preload_active = false;
                    state.playlist_tracks_cache.clear();
                    state.list_state.reset();
                    state.category_timestamps.clear();
                    state.cache_dirty = false;
                    state.artist_nav = crate::app::state::BrowseNavigationState::new();
                    state.genre_nav = crate::app::state::BrowseNavigationState::new();
                    state.playlist_nav = crate::app::state::BrowseNavigationState::new();
                    state.station_nav = crate::app::state::StationNavigationState::new();

                    // Stop playback
                    if state.playback.status != PlayStatus::Stopped {
                        if let Some(track) = state.current_track().cloned() {
                            helpers::report_playback_stop_to_plex(
                                &track, state.playback.position_ms, false,
                                state.plex_session_id.clone(), client,
                            );
                        }
                    }
                    audio.stop();
                    audio.track_cache.flush();
                    state.playback.status = PlayStatus::Stopped;
                    state.playback.position_ms = 0;
                    state.playback.duration_ms = 0;
                    state.queue.clear();
                    state.queue_index = None;
                    state.queue_original.clear();
                    state.radio.clear();
                    state.playback_mode = PlaybackMode::Queue;
                    state.adventure = crate::app::state::AdventureState::default();

                    state.library_loading = true;
                    let server_name = server.name.clone();
                    state.set_status(format!("Connecting to {}...", server_name));

                    let client_id = client.client_identifier().to_string();
                    let event_tx = event_tx.clone();
                    let server_id_clone = server_id.clone();
                    let spawn_server_name = server_name.clone();

                    tokio::spawn(async move {
                        if let Some(url) = helpers::find_working_connection(&server, &token, &client_id).await {
                            let _ = event_tx.send(Event::ServerConnectionSucceeded {
                                server_name: spawn_server_name.clone(),
                                url: url.clone(),
                            }).await;

                            // Now load libraries from this server
                            let new_client = crate::api::PlexClient::new_with_url(&url, Some(&token), &client_id);
                            match new_client.get_libraries().await {
                                Ok(libs) => {
                                    let _ = event_tx.send(Event::LibrariesLoaded(libs)).await;
                                }
                                Err(e) => {
                                    tracing::error!("Failed to load libraries from {}: {}", spawn_server_name, e);
                                }
                            }
                        } else {
                            let _ = event_tx.send(Event::ServerConnectionFailed {
                                server_name: spawn_server_name,
                            }).await;
                        }
                    });

                    // Update server tracking
                    state.active_server_id = Some(server_id);
                    state.active_library = Some(lib_key.clone());

                    // Persist the new server info
                    let server_info = crate::plex::ServerInfo {
                        url: String::new(), // Will be updated by ServerConnectionSucceeded
                        identifier: server_id_clone,
                        name: server_name.clone(),
                    };
                    if let Err(e) = PlexAuth::update_server_info(&server_info) {
                        tracing::warn!("Failed to persist server info: {}", e);
                    }
                }
            } else {
                state.set_error("Server not found".to_string());
            }
        }
        Action::SaveSettings => {
            // Build updated config from current state
            let mut updated_config = config.clone();
            updated_config.libraries.default_library = state.active_library.clone();

            // Save config to disk
            if let Err(e) = crate::config::save_config(&updated_config) {
                state.set_error(format!("Failed to save settings: {}", e));
            } else {
                tracing::debug!("Settings saved");
            }
        }
        Action::ClearCache => {
            // Clear all cached library data
            if let Some(cache) = LibraryCache::new() {
                match cache.clear_all() {
                    Ok(count) => {
                        tracing::info!("Cleared {} cache files", count);

                        // Clear in-memory data
                        state.artists.clear();
                        state.albums.clear();
                        state.playlists.clear();
                        state.folder_state = None;
                        state.folder_contents_cache.clear();
                        state.subfolder_preload_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                        state.subfolder_preload_active = false;
                        state.playlist_tracks_cache.clear();
                        state.category_timestamps.clear();
                        state.cache_dirty = true;

                        // Reload from API
                        if let Some(lib_key) = &state.active_library {
                            let lib_key = lib_key.clone();
                            let lib_name = state.libraries.iter()
                                .find(|l| l.key == lib_key)
                                .map(|l| l.title.clone())
                                .unwrap_or_else(|| lib_key.clone());

                            // Preload all library data (same as preload_all_library_data)
                            helpers::preload_data(event_tx, PreloadType::Artists, &lib_key, client);
                            helpers::preload_data(event_tx, PreloadType::Albums, &lib_key, client);
                            helpers::preload_data(event_tx, PreloadType::Playlists, &lib_key, client);
                            helpers::preload_data(event_tx, PreloadType::Folders { lib_title: lib_name }, &lib_key, client);
                            helpers::preload_data(event_tx, PreloadType::Genres, &lib_key, client);
                            helpers::preload_data(event_tx, PreloadType::ArtistGenres, &lib_key, client);
                            helpers::preload_data(event_tx, PreloadType::AlbumGenres, &lib_key, client);
                            helpers::preload_data(event_tx, PreloadType::Moods, &lib_key, client);
                            helpers::preload_data(event_tx, PreloadType::Styles, &lib_key, client);
                            helpers::preload_data(event_tx, PreloadType::Stations, &lib_key, client);
                            helpers::preload_data(event_tx, PreloadType::RecentlyAdded, &lib_key, client);
                        }

                        state.set_status(format!("Cleared {} cache files, reloading...", count));
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to clear cache: {}", e));
                    }
                }
            } else {
                state.set_error("Cache not available".to_string());
            }
        }

        Action::ClearLibraryCache => {
            // Clear main library cache files and in-memory data (but not subfolders or artwork)
            if let Some(cache) = LibraryCache::new() {
                match cache.clear_all() {
                    Ok(count) => {
                        tracing::info!("Cleared {} library cache files", count);

                        // Clear in-memory library data
                        state.artists.clear();
                        state.albums.clear();
                        state.playlists.clear();
                        state.genres.clear();
                        state.artist_genres.clear();
                        state.album_genres.clear();
                        state.moods.clear();
                        state.styles.clear();
                        state.stations.clear();
                        state.recently_added_albums.clear();
                        state.playlist_tracks_cache.clear();
                        state.category_timestamps.clear();
                        state.cache_dirty = true;

                        // Reload from API
                        if let Some(lib_key) = &state.active_library {
                            let lib_key = lib_key.clone();
                            let lib_name = state.libraries.iter()
                                .find(|l| l.key == lib_key)
                                .map(|l| l.title.clone())
                                .unwrap_or_else(|| lib_key.clone());
                            helpers::preload_all_library_data(event_tx, &lib_key, &lib_name, client);
                        }

                        state.set_status(format!("Cleared {} library cache files, reloading...", count));
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to clear library cache: {}", e));
                    }
                }
            } else {
                state.set_error("Cache not available".to_string());
            }
        }
        Action::ClearArtworkCache => {
            let artwork_cache = crate::plex::ArtworkCache::default();
            let removed = artwork_cache.clear_all();
            tracing::info!("Cleared {} artwork cache files", removed);

            // Clear in-memory artwork
            state.album_art_cache.clear();
            state.album_art_pending.clear();
            state.artwork_cache_stats = Some((0, 0));

            state.set_status(format!("Cleared {} artwork cache files", removed));
        }
        Action::ClearSubfolderCache => {
            let count = state.folder_contents_cache.len();
            state.folder_contents_cache.clear();
            state.subfolder_preload_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            state.subfolder_preload_active = false;
            state.cache_dirty = true;

            tracing::info!("Cleared {} subfolder cache entries", count);
            state.set_status(format!("Cleared {} subfolder cache entries", count));
        }
        Action::StartSubfolderCrawl => {
            use crate::app::handlers::helpers::SubfolderPreloadResult;
            match helpers::maybe_start_subfolder_preload(event_tx, state, client) {
                SubfolderPreloadResult::Started => {
                    state.set_status("Subfolder crawl started".to_string());
                }
                SubfolderPreloadResult::AlreadyActive => {
                    state.set_status("Subfolder crawl already running".to_string());
                }
                SubfolderPreloadResult::AllCached { count } => {
                    state.set_status(format!("All {} folder listings already cached and fresh", count));
                }
                SubfolderPreloadResult::NoRootFolders => {
                    state.set_status("No root folders loaded yet".to_string());
                }
                SubfolderPreloadResult::NoSubfolders => {
                    state.set_status("No subfolders to crawl (root has only tracks)".to_string());
                }
                SubfolderPreloadResult::NoLibrary => {
                    state.set_status("No library selected".to_string());
                }
            }
        }
        Action::StopSubfolderCrawl => {
            state.subfolder_preload_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            state.subfolder_preload_active = false;
            state.set_status("Subfolder crawl stopped".to_string());
        }

        // Sonic Adventure actions
        Action::StartAdventure => {
            state.adventure = crate::app::state::AdventureState {
                active: true,
                start_track: None,
                end_track: None,
                requested_length: 20,
                generating: false,
            };
            state.set_status("Adventure: select START track (Alt+A)".to_string());
        }
        Action::SetAdventureStart(track) => {
            state.adventure.active = true;
            state.adventure.start_track = Some(track.clone());
            state.adventure.end_track = None;
            state.adventure.generating = false;
            state.set_status(format!("Adventure: {} → select END (Alt+A)", truncate_str(&track.title, 25)));
        }
        Action::SetAdventureEnd(track) => {
            state.adventure.end_track = Some(track);
            // Clear status message so transport shows normal info
            state.clear_status();
            // Show input dialog for length
            state.input_dialog = Some(crate::app::state::InputDialog {
                title: "Adventure Length (5-100)".to_string(),
                input: "20".to_string(),
                action_type: crate::app::state::InputDialogAction::AdventureLength,
            });
        }
        Action::SetAdventureLength(length) => {
            state.adventure.requested_length = length.clamp(5, 100);
            state.input_dialog = None;
            state.adventure.generating = true;
            state.set_status("Adventure: generating sonic bridge...".to_string());

            // Generate the adventure
            if let (Some(start), Some(end)) = (state.adventure.start_track.clone(), state.adventure.end_track.clone()) {
                let requested_length = state.adventure.requested_length;
                match crate::services::generate_adventure(client, &start, &end, requested_length).await {
                    Ok(tracks) => {
                        // Check if we got meaningful results (more than just start + end)
                        if tracks.len() <= 2 {
                            state.adventure = crate::app::state::AdventureState::default();
                            state.set_error("Adventure: no similar tracks found for these songs. Try different tracks with sonic analysis data.".to_string());
                            return Ok(vec![]);
                        }

                        // Clear adventure state
                        state.adventure = crate::app::state::AdventureState::default();

                        // Clear radio state if switching from radio mode
                        if state.playback_mode == PlaybackMode::Radio {
                            state.radio.clear();
                        }
                        // Replace queue with adventure
                        state.queue = tracks;
                        state.queue_index = Some(0);
                        state.queue_original.clear();
                        state.queue_sort_mode = QueueSortMode::QueueOrder;
                        state.playback_mode = PlaybackMode::Queue;
                        state.view = View::NowPlaying;

                        // Start playback
                        helpers::play_current_track(event_tx, state, client, audio).await;
                        state.set_status(format!("Adventure: {} tracks ready!", state.queue.len()));
                    }
                    Err(e) => {
                        // Fully reset adventure state on error
                        state.adventure = crate::app::state::AdventureState::default();
                        state.set_error(format!("Adventure failed: {}", e));
                    }
                }
            } else {
                // Fully reset adventure state
                state.adventure = crate::app::state::AdventureState::default();
                state.set_error("Adventure: missing start or end track".to_string());
            }
        }
        Action::CancelAdventure => {
            state.adventure = crate::app::state::AdventureState::default();
            state.input_dialog = None;
            state.clear_status();
        }
        Action::AdventureComplete(tracks) => {
            // This is handled inline in SetAdventureLength for simplicity
            state.adventure = crate::app::state::AdventureState::default();
            // Clear radio state if switching from radio mode
            if state.playback_mode == PlaybackMode::Radio {
                state.radio.clear();
            }
            state.queue = tracks;
            state.queue_index = Some(0);
            state.queue_original.clear();
            state.queue_sort_mode = QueueSortMode::QueueOrder;
            state.playback_mode = PlaybackMode::Queue;
            state.view = View::NowPlaying;
            helpers::play_current_track(event_tx, state, client, audio).await;
        }
        Action::AdventureError(msg) => {
            state.adventure.generating = false;
            state.set_error(format!("Adventure failed: {}", msg));
        }
        _ => unreachable!("dispatch_settings called with non-settings action: {:?}", action),
    }
    Ok(follow_ups)
}
