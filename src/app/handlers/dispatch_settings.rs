//! Settings dispatch handlers: Logout, AuthSignIn, AuthSelectServer, OpenSettings,
//! SaveCredentials, SettingsSelect, SettingsSignIn, SettingsDiscoverServers, SelectServer,
//! SelectLibrary, SaveSettings, ClearCache, and Adventure actions.

use crate::app::{Action, AppState, Event};
use crate::app::event_loop::PreloadType;
use crate::app::state::{BrowseItem, ConnectionState, PlayStatus, PlaybackMode, QueueSortMode, SettingsSection, View};
use crate::api::{PlexAuth, PlexClient};
use crate::audio::AudioPlayer;
use crate::cache::LibraryCache;
use crate::config::Config;
use crate::services::{CacheService, FolderColumn, FolderNavigationState};
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

            // Reset connection state
            state.connection = ConnectionState::Disconnected;
            state.active_library = None;

            // Clear all library and browse data
            state.libraries.clear();
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
            state.recently_played_albums.clear();
            state.selected_artist_albums.clear();
            state.selected_album_tracks.clear();
            state.genre_albums.clear();
            state.folder_state = None;
            state.folder_contents_cache.clear();
            state.available_servers.clear();

            // Clear playback state
            state.queue.clear();
            state.queue_index = None;
            state.queue_original.clear();

            // Clear navigation state
            state.station_nav.columns.clear();
            state.station_nav.focused_column = 0;
            state.list_state.reset();

            // Stop playback
            audio.stop();
            state.playback.status = PlayStatus::Stopped;

            // Clear all server-related config (keep app settings like theme/playback)
            config.plex = crate::config::PlexConfig::default();
            config.libraries.default_library = None;
            config.libraries.selected_server = None;
            config.general.default_library = None;
            if let Err(e) = crate::config::save_config(config) {
                tracing::warn!("Failed to save config after logout: {}", e);
            }

            state.set_status("Logged out. Restart to sign in again.".to_string());
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
                        // Signed in: 0=Clear Cache, 1=Sign Out
                        match state.settings_state.item_index {
                            0 => follow_ups.push(Action::ClearCache),
                            1 => follow_ups.push(Action::Logout),
                            _ => {}
                        }
                    } else {
                        // Not signed in: 0=Sign In
                        if state.settings_state.item_index == 0 {
                            state.settings_state.signing_in = true;
                            state.settings_state.item_index = 0;
                        }
                    }
                }
                SettingsSection::Libraries => {
                    // Activate selected library
                    if let Some(lib) = state.libraries.get(state.settings_state.item_index) {
                        let lib_key = lib.key.clone();
                        follow_ups.push(Action::SelectLibrary(lib_key));
                    }
                }
                SettingsSection::Playback => {
                    // No action yet for playback settings
                }
                SettingsSection::Interface => {
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
                SettingsSection::About => {
                    // No selectable items in About section
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

                                    // Find working server URL (tests connectivity)
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
                state.recently_played_albums.clear();
                state.selected_artist_albums.clear();
                state.selected_album_tracks.clear();
                state.folder_state = None;
                state.folder_contents_cache.clear();
                state.list_state.reset();

                // Clear Miller column navigation states
                state.artist_nav = crate::app::state::BrowseNavigationState::new();
                state.genre_nav = crate::app::state::BrowseNavigationState::new();
                state.playlist_nav = crate::app::state::BrowseNavigationState::new();
                state.station_nav = crate::app::state::StationNavigationState::new();

                // Stop playback and clear queue (tracks belong to the old library)
                audio.stop();
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

                // Load ALL cached data for instant display
                if let Some(cache) = LibraryCache::new() {
                    if let Some(cached) = cache.load(&lib_key) {
                        // Validate cache belongs to this library
                        if cached.library_key != lib_key {
                            tracing::warn!("Cache library_key mismatch: expected {}, got {} - ignoring cache",
                                lib_key, cached.library_key);
                        } else {
                            tracing::info!("Library switch: loading from cache: {} artists, {} albums, {} folders",
                                cached.artists.len(), cached.albums.len(), cached.root_folders.len());

                            // Core library data
                            // IMPORTANT: Always re-sort after loading from cache
                            if !cached.artists.is_empty() {
                                state.artists = cached.artists;
                                state.artists.sort_by(|a, b| helpers::sort_key(&a.title).cmp(&helpers::sort_key(&b.title)));
                                state.artists_total = state.artists.len() as u32;
                                // Initialize artist_nav for Miller columns
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
                                // Initialize playlist_nav for Miller columns
                                let items = BrowseItem::from_playlists(&state.playlists);
                                state.playlist_nav.reset("playlists", items);
                            }

                            // Folders
                            if !cached.root_folders.is_empty() {
                                let root_column = FolderColumn::new(None, lib_name.clone(), cached.root_folders);
                                state.folder_state = Some(FolderNavigationState {
                                    library_key: lib_key.clone(),
                                    columns: vec![root_column],
                                    focused_column: 0,
                                    loading: false,
                                });
                            }
                            // Load cached subfolder contents with staleness filtering
                            if !cached.folder_contents.is_empty() {
                                state.folder_contents_cache = cached.folder_contents;
                                let removed = CacheService::filter_stale_subfolders_default(&mut state.folder_contents_cache);
                                if removed > 0 {
                                    tracing::info!("Library switch: removed {} very stale subfolder caches", removed);
                                    state.cache_dirty = true;
                                }
                                tracing::debug!("Library switch: loaded {} cached subfolders", state.folder_contents_cache.len());
                            } else {
                                // New library has no subfolder cache - clear any old data
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

                            // Stations - populate both legacy and Miller columns
                            if !cached.stations.is_empty() {
                                state.stations = cached.stations.clone();
                                // Initialize Miller columns with root column
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
                            if !cached.recently_played_albums.is_empty() {
                                state.recently_played_albums = cached.recently_played_albums;
                            }
                        }
                    }
                }

                // Refresh from API in background
                helpers::preload_all_library_data(event_tx, &lib_key, &lib_name, client);

                state.set_status(format!("Switched to {}", lib_name));

                // Auto-save the default library
                follow_ups.push(Action::SaveSettings);
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
                            helpers::preload_data(event_tx, PreloadType::RecentlyPlayed, &lib_key, client);
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

        // Sonic Adventure actions
        Action::StartAdventure => {
            state.adventure = crate::app::state::AdventureState {
                active: true,
                start_track: None,
                end_track: None,
                requested_length: 20,
                generating: false,
            };
            state.set_status("Adventure: select START track (Alt+V)".to_string());
        }
        Action::SetAdventureStart(track) => {
            state.adventure.active = true;
            state.adventure.start_track = Some(track.clone());
            state.adventure.end_track = None;
            state.adventure.generating = false;
            state.set_status(format!("Adventure: {} → select END (Alt+V)", truncate_str(&track.title, 25)));
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
