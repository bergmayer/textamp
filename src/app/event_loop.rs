//! Main application event loop (musikcube-style).
//!
//! Handles input events, async task coordination, and state updates.

use super::{Action, AppState, Event};
use super::state::{ConnectionState, View, BrowseCategory, Focus, PlayStatus, SearchSection, SearchTab, RightPanelMode, SettingsSection, PlaybackMode};
use crate::api::{PlexAuth, PlexClient};
use crate::api::models::Track;
use crate::audio::AudioPlayer;
use crate::cache::LibraryCache;
use crate::config::Config;
use crate::plex::CachedFolder;
use crate::services::{CacheService, FolderService, FolderNavigationState};
use crate::ui;
use crate::util::truncate_str;

use anyhow::Result;
use crossterm::event::{self, Event as CrosstermEvent, KeyCode, KeyModifiers, DisableMouseCapture};
use crossterm::execute;
use ratatui::prelude::*;
use std::io::Stdout;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Types of data that can be preloaded in the background.
///
/// This enum consolidates the 13 different preload operations into a single
/// type-safe representation. Each variant corresponds to a specific API call
/// and event result.
#[derive(Clone, Debug)]
pub enum PreloadType {
    Artists,
    Albums,
    Playlists,
    Genres,
    Moods,
    ArtistGenres,
    AlbumGenres,
    Styles,
    Stations,
    RecentlyAdded,
    RecentlyPlayed,
    /// Folders require additional lib_title for display.
    Folders { lib_title: String },
}

/// Main event loop.
pub struct EventLoop {
    event_tx: mpsc::Sender<Event>,
    event_rx: mpsc::Receiver<Event>,
    config: Config,
    shutdown: Arc<AtomicBool>,
}

impl EventLoop {
    pub fn new(config: Config) -> Self {
        let (event_tx, event_rx) = mpsc::channel(256);
        Self {
            event_tx,
            event_rx,
            config,
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Run the main event loop.
    pub async fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        state: &mut AppState,
        client: &mut PlexClient,
        audio: &mut AudioPlayer,
    ) -> Result<()> {
        let tick_rate = Duration::from_millis(100);
        let mut last_tick = Instant::now();

        // Spawn terminal event reader task
        let event_tx_input = self.event_tx.clone();
        let shutdown = self.shutdown.clone();
        tokio::spawn(async move {
            loop {
                // Check for shutdown signal
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }
                if event::poll(Duration::from_millis(50)).unwrap_or(false) {
                    if let Ok(evt) = event::read() {
                        let mapped = match evt {
                            CrosstermEvent::Key(k) => Some(Event::Key(k)),
                            CrosstermEvent::Mouse(m) => Some(Event::Mouse(m)),
                            CrosstermEvent::Resize(w, h) => Some(Event::Resize(w, h)),
                            _ => None,
                        };
                        if let Some(e) = mapped {
                            if event_tx_input.send(e).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        });

        // Start authentication in background
        self.start_auth_task(state);

        // Main loop
        loop {
            // Render
            terminal.draw(|f| ui::render(f, state))?;

            // Calculate timeout
            let timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or(Duration::ZERO);

            tokio::select! {
                Some(event) = self.event_rx.recv() => {
                    let actions = self.handle_event(event, state, client);
                    for action in actions {
                        self.dispatch(action, state, client, audio).await?;
                    }
                }
                _ = tokio::time::sleep(timeout) => {
                    if last_tick.elapsed() >= tick_rate {
                        // Tick: update playback position
                        if state.playback.status == PlayStatus::Playing {
                            state.playback.position_ms += tick_rate.as_millis() as u64;
                        }
                        last_tick = Instant::now();
                    }
                }
            }

            if state.should_quit {
                // Signal the input reader task to stop
                self.shutdown.store(true, Ordering::Relaxed);

                // Disable mouse capture IMMEDIATELY to prevent more mouse events
                // being queued in the terminal buffer
                let _ = execute!(std::io::stdout(), DisableMouseCapture);

                // Give the input task a moment to notice and exit
                tokio::time::sleep(Duration::from_millis(100)).await;

                // Drain any remaining events from the terminal input buffer
                // This prevents escape sequences from being echoed after raw mode is disabled
                while event::poll(Duration::from_millis(10)).unwrap_or(false) {
                    let _ = event::read();
                }

                break;
            }
        }

        Ok(())
    }

    /// Start authentication in background task.
    fn start_auth_task(&self, state: &mut AppState) {
        use super::state::AuthStep;
        state.connection = ConnectionState::Authenticating;
        state.auth_state.step = AuthStep::Checking;

        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            // Try stored token (primary authentication method)
            if let Some(stored) = PlexAuth::load_token() {
                tracing::info!("Loaded stored auth: client_identifier={}, server_url={:?}",
                    stored.client_identifier, stored.server_url);
                let auth = PlexAuth::from_stored_auth(&stored);
                match auth.verify_token(&stored.token).await {
                    Ok(user) => {
                        // Get servers for the authenticated user
                        let servers = auth.get_servers(&stored.token).await.unwrap_or_default();

                        // Find best working connection using parallel testing.
                        // This tests ALL connections simultaneously and picks the best one
                        // (local preferred over remote over relay). Much faster than sequential.
                        //
                        // If we have a stored server identifier, prefer that server's connections.
                        // Otherwise test all servers.
                        let final_server_url = if let Some(stored_id) = &stored.server_identifier {
                            // Find the stored server in the list
                            if let Some(server) = servers.iter().find(|s| &s.client_identifier == stored_id) {
                                tracing::info!("Testing connections for stored server: {}", server.name);
                                find_working_connection(server, &stored.token).await
                            } else {
                                tracing::warn!("Stored server no longer available, testing all servers");
                                find_working_connection_from_servers(&servers, &stored.token).await
                            }
                        } else {
                            // No stored server - find a working connection from available servers
                            find_working_connection_from_servers(&servers, &stored.token).await
                        };

                        if let Some(url) = final_server_url {
                            let _ = event_tx.send(Event::AuthSuccess {
                                token: stored.token,
                                username: user.username,
                                server_url: url,
                                servers,
                                client_identifier: stored.client_identifier,
                            }).await;
                            return;
                        }
                    }
                    Err(_) => {}
                }
            }

            // No valid stored token - show login form
            let _ = event_tx.send(Event::AuthShowLogin).await;
        });
    }

    /// Handle an incoming event and return actions to dispatch.
    fn handle_event(&self, event: Event, state: &mut AppState, client: &mut PlexClient) -> Vec<Action> {
        match event {
            Event::Key(key) => {
                state.last_input_time = std::time::Instant::now();
                self.handle_key(key, state)
            }
            Event::Resize(w, h) => {
                state.terminal_width = w;
                state.terminal_height = h;
                vec![]
            }
            Event::AuthSuccess { token, username, server_url, servers, client_identifier } => {
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

                state.available_servers = servers;
                state.connection = ConnectionState::Connected { username: username.clone() };
                state.settings_state.discovering_servers = false;
                state.settings_state.username_input = username;
                state.settings_state.password_input.clear(); // Never keep password in memory
                state.view = View::Browse;
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
                client.set_server(url);
                state.set_status(format!("Connected to {}", server_name));
                vec![]
            }
            Event::ServerConnectionFailed { server_name } => {
                tracing::warn!("All connection tests failed for server {}", server_name);
                state.set_error(format!("Could not connect to {} - all connections failed", server_name));
                vec![]
            }
            Event::AuthFailed(msg) => {
                use super::state::AuthStep;
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
                use super::state::AuthStep;
                tracing::info!("No stored credentials, showing login form");
                state.connection = ConnectionState::Disconnected;
                state.auth_state.step = AuthStep::Login;
                state.auth_state.field_index = 0;
                state.auth_state.editing = false;
                state.auth_state.error_message = None;
                vec![]
            }
            Event::AuthServersReady { token, username, servers, client_identifier } => {
                use super::state::AuthStep;
                tracing::info!("Login succeeded, {} servers available", servers.len());
                state.available_servers = servers.clone();

                // Auto-select if only one server, otherwise show selection
                if servers.len() == 1 {
                    state.auth_state.step = AuthStep::Connecting;
                    // Find working connection URL (tests connectivity)
                    let event_tx = self.event_tx.clone();
                    let token_clone = token.clone();
                    let username_clone = username.clone();
                    let servers_clone = servers.clone();
                    let client_id_clone = client_identifier.clone();
                    tokio::spawn(async move {
                        if let Some(url) = find_working_connection_from_servers(&servers_clone, &token_clone).await {
                            let _ = event_tx.send(Event::AuthSuccess {
                                token: token_clone,
                                username: username_clone,
                                server_url: url,
                                servers: servers_clone,
                                client_identifier: client_id_clone,
                            }).await;
                        } else {
                            let _ = event_tx.send(Event::AuthFailed(
                                "Could not connect to server - all connection attempts failed".to_string()
                            )).await;
                        }
                    });
                } else {
                    // Multiple servers - let user choose
                    state.auth_state.step = AuthStep::ServerSelect;
                    state.auth_state.server_index = 0;
                    // Store token temporarily for when server is selected
                    state.settings_state.username_input = username;
                    // Note: We need to pass the token through - store in a temp location
                    // We'll use the client's token holder for this
                    client.set_auth_token(token);
                    client.set_client_identifier(client_identifier);
                }
                vec![]
            }
            Event::AuthLoginFailed(msg) => {
                use super::state::AuthStep;
                tracing::error!("Login failed: {}", msg);
                state.auth_state.step = AuthStep::Login;
                state.auth_state.error_message = Some(msg);
                state.auth_state.password_input.clear();
                vec![]
            }
            Event::LibrariesLoaded(libs) => {
                tracing::info!("LibrariesLoaded: received {} libraries", libs.len());
                for lib in &libs {
                    tracing::debug!("  Library: {} (key={}, type={})", lib.title, lib.key, lib.library_type);
                }
                state.libraries = libs.into_iter().filter(|l| l.is_music()).collect();
                tracing::info!("After filtering: {} music libraries", state.libraries.len());
                if let Some(lib) = state.libraries.first() {
                    tracing::info!("Selected music library: {} (key={})", lib.title, lib.key);
                    state.active_library = Some(lib.key.clone());
                    // Now that we have an active library, load artists
                    return vec![Action::LoadArtists];
                } else {
                    tracing::warn!("No music libraries found after filtering!");
                }
                vec![]
            }
            Event::ArtistsLoaded(mut artists) => {
                // Sort by display title, ignoring "The " prefix
                artists.sort_by(|a, b| sort_key(&a.title).cmp(&sort_key(&b.title)));
                state.artists = artists;
                state.artists_loading = false;

                // Initialize artist_nav if we're in Artists category
                if state.browse_category == BrowseCategory::Artists && !state.artists.is_empty() {
                    let title = state.artist_view_mode.name();
                    let items = super::state::BrowseItem::from_artists(&state.artists);
                    state.artist_nav.reset(title, items);
                }
                vec![]
            }
            Event::AlbumsLoaded(mut albums) => {
                // Sort by display title, ignoring "The " prefix
                albums.sort_by(|a, b| sort_key(&a.title).cmp(&sort_key(&b.title)));
                state.albums = albums;
                state.albums_loading = false;
                vec![]
            }
            Event::PlaylistsLoaded(playlists) => {
                // Initialize playlist_nav with the playlists list
                let items = crate::app::state::BrowseItem::from_playlists(&playlists);
                state.playlist_nav.reset("playlists", items);
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
            Event::SimilarAlbumsLoaded(albums) => {
                state.similar_albums = albums;
                state.similar_mode = super::state::SimilarMode::Albums;
                state.similar_loading = false;
                state.list_state.similar_index = 0;
                vec![]
            }
            Event::SimilarTracksLoaded(tracks) => {
                state.similar_tracks = tracks;
                state.similar_mode = super::state::SimilarMode::Tracks;
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
                // Report stop to Plex when track ends naturally
                // continuing=true because we're about to play the next track
                if let Some(track) = state.current_track().cloned() {
                    // Use track duration as position (track finished)
                    let position = track.duration_ms();
                    Self::report_playback_stop_to_plex(&track, position, true, state.plex_session_id.clone(), client);
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
                // Check for 404/not found errors - offer to refresh cache
                if msg.contains("404") || msg.to_lowercase().contains("not found") {
                    state.confirm_dialog = Some(super::state::ConfirmDialog {
                        title: "Track Not Found".to_string(),
                        message: "This track may have been removed. Refresh cache?".to_string(),
                        on_confirm: super::state::ConfirmAction::RefreshCache,
                    });
                } else {
                    state.set_error(format!("Playback error: {}", msg));
                }
                state.playback.status = PlayStatus::Stopped;
                vec![]
            }
            Event::BufferingStart => {
                state.playback.status = PlayStatus::Buffering;
                vec![]
            }
            Event::BufferingEnd => {
                state.playback.status = PlayStatus::Playing;
                vec![]
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
            Event::FoldersPreloaded { library_key, folder_state } => {
                // Ignore if this is for a different library (race condition from library switch)
                if state.active_library.as_ref() != Some(&library_key) {
                    tracing::debug!("Ignoring stale folders preload for library {}", library_key);
                    return vec![];
                }
                // Only set if folders weren't already loaded (user might have navigated there)
                if state.folder_state.is_none() {
                    state.folder_state = Some(folder_state);
                    state.cache_dirty = true;
                    tracing::debug!("Folders preloaded and ready");
                }
                vec![]
            }
            Event::ArtistsPreloaded(mut artists) => {
                // Update artists if we haven't loaded them yet or if this is fresher data
                if state.artists.is_empty() || !state.artists_loading {
                    // Sort by display title, ignoring "The " prefix
                    artists.sort_by(|a, b| sort_key(&a.title).cmp(&sort_key(&b.title)));
                    let count = artists.len();
                    state.artists = artists;
                    state.artists_total = count as u32;
                    state.cache_dirty = true;
                    tracing::debug!("Artists preloaded: {} items", count);
                }
                vec![]
            }
            Event::AlbumsPreloaded(mut albums) => {
                // Update albums if we haven't loaded them yet
                if state.albums.is_empty() || !state.albums_loading {
                    // Sort by display title, ignoring "The " prefix
                    albums.sort_by(|a, b| sort_key(&a.title).cmp(&sort_key(&b.title)));
                    let count = albums.len();
                    state.albums = albums;
                    state.albums_total = count as u32;
                    state.cache_dirty = true;
                    tracing::debug!("Albums preloaded: {} items", count);
                }
                vec![]
            }
            Event::PlaylistsPreloaded(playlists) => {
                // Update playlists if we haven't loaded them yet
                if state.playlists.is_empty() || !state.playlists_loading {
                    let count = playlists.len();
                    state.playlists = playlists;
                    state.cache_dirty = true;
                    tracing::debug!("Playlists preloaded: {} items", count);
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
                    state.cache_dirty = true;
                    tracing::debug!("Genres preloaded: {} items", count);
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
                    state.cache_dirty = true;
                    tracing::debug!("Artist genres preloaded: {} items", count);
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
                    state.cache_dirty = true;
                    tracing::debug!("Album genres preloaded: {} items", count);
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
                    state.cache_dirty = true;
                    tracing::debug!("Moods preloaded: {} items", count);
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
                    state.cache_dirty = true;
                    tracing::debug!("Styles preloaded: {} items", count);
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
                    state.stations = stations;
                    state.cache_dirty = true;
                    tracing::debug!("Stations preloaded: {} items", count);
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
                    state.cache_dirty = true;
                    tracing::debug!("Recently added albums preloaded: {} items", count);
                }
                vec![]
            }
            Event::RecentlyPlayedPreloaded { library_key, albums } => {
                // Ignore if this is for a different library (race condition from library switch)
                if state.active_library.as_ref() != Some(&library_key) {
                    tracing::debug!("Ignoring stale recently played preload for library {}", library_key);
                    return vec![];
                }
                if state.recently_played_albums.is_empty() && !state.recently_played_loading {
                    let count = albums.len();
                    state.recently_played_albums = albums;
                    state.cache_dirty = true;
                    tracing::debug!("Recently played albums preloaded: {} items", count);
                }
                vec![]
            }
            Event::Tick => {
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

                // Periodic cache save: save if dirty, idle for 30+ seconds, and 2+ minutes since last save
                self.maybe_save_cache_async(state);

                // Very stale background refresh (32 days, 2min idle)
                self.maybe_refresh_very_stale(state, client);

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

                // Clear the "Refreshing X..." status message if it matches this category
                let refresh_msg = format!("Refreshing {}...", category.display_name());
                if state.status_message.as_ref() == Some(&refresh_msg) {
                    state.clear_status();
                }

                if changed && self.is_viewing_category(&category, state) {
                    state.set_toast(format!("{} updated", category.display_name()));
                }
                vec![]
            }
            Event::Mouse(mouse_event) => {
                self.handle_mouse(mouse_event, state)
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
                    state.waveform.error = Some(error.clone());
                    state.waveform.generating = false;
                    tracing::warn!("Waveform generation failed for {}: {}", track_key, error);
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
            Event::StationTracksLoaded { station_key, station_title, tracks } => {
                state.stations_loading = false;
                state.station_nav.loading = false;

                if tracks.is_empty() {
                    state.set_error("Station returned no tracks (is Sonic Analysis enabled in Plex settings?)".to_string());
                } else {
                    // Start playing the station
                    state.playback_mode = PlaybackMode::Radio;
                    state.radio.clear();
                    state.radio.active_station = Some(super::state::ActiveStation {
                        key: station_key,
                        title: station_title.clone(),
                    });
                    state.radio.tracks = tracks.clone();
                    state.radio.track_index = Some(0);
                    state.view = View::NowPlaying;
                    state.set_status(format!("Playing {} ({} tracks)", station_title, tracks.len()));

                    // Return action to play the first track
                    if let Some(track) = tracks.first().cloned() {
                        return vec![Action::PlayTrack(track)];
                    }
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
                state.station_nav.push_column(super::state::StationColumn::new(
                    Some(station_key),
                    station_title,
                    children.clone(),
                ));
                // Also update the legacy state for compatibility
                state.stations = children;
                state.stations_index = 0;
                state.clear_error();
                vec![]
            }
            Event::AlbumRadioTracksLoaded { tracks } => {
                state.radio_state.fetching = false;
                if !tracks.is_empty() {
                    state.queue.extend(tracks);
                    state.set_status(format!("Album radio: {} tracks queued", state.queue.len()));
                }
                vec![]
            }
            Event::AlbumRadioLoadFailed { error } => {
                state.radio_state.fetching = false;
                tracing::warn!("{}", error);
                // Don't show error to user - seed album is already playing
                vec![]
            }

            // Inline list filter completed
            Event::ListFilterCompleted { version, results } => {
                // Only apply if this is the most recent filter version
                if version == state.list_filter_version {
                    state.list_filter_loading = false;
                    state.list_filter_results = Some(results);
                }
                vec![]
            }
            _ => vec![],
        }
    }

    /// Handle mouse events.
    fn handle_mouse(&self, event: crossterm::event::MouseEvent, state: &mut AppState) -> Vec<Action> {
        use crossterm::event::{MouseEventKind, MouseButton};

        let click_row = event.row;
        let click_col = event.column;

        // Calculate layout regions
        let transport_row = state.terminal_height.saturating_sub(3);
        let shortcuts_row = state.terminal_height.saturating_sub(1);
        let left_panel_width = 30u16;

        match event.kind {
            // Left click
            MouseEventKind::Down(MouseButton::Left) => {
                // Check shortcut bar (bottom row)
                if click_row == shortcuts_row {
                    return self.handle_shortcut_bar_click(click_col, state);
                }

                // Check transport bar
                if click_row >= transport_row && click_row < shortcuts_row {
                    return self.handle_transport_down(click_col, state);
                }

                // Content area clicks depend on view
                match state.view {
                    View::Auth => {
                        return self.handle_auth_click(click_row, click_col, state);
                    }
                    View::Browse => {
                        // Left panel (categories)
                        if click_col < left_panel_width {
                            return self.handle_left_panel_click(click_row, state);
                        }
                        // Right panel (albums/tracks)
                        return self.handle_right_panel_click(click_row, click_col, state);
                    }
                    View::NowPlaying => {
                        return self.handle_now_playing_down(click_row, click_col, state);
                    }
                    View::Search => {
                        return self.handle_search_click(click_row, click_col, state);
                    }
                    View::Settings => {
                        return self.handle_settings_click(click_row, click_col, state);
                    }
                    View::Help => {
                        return self.handle_help_click(click_row, state);
                    }
                    _ => {}
                }
            }

            // Mouse drag - only seek if we started dragging on the indicator
            MouseEventKind::Drag(MouseButton::Left) => {
                if state.seeking_drag {
                    // When dragging, respond to either transport bar or visualizer area
                    // This allows smooth dragging even if mouse moves between areas

                    // Dragging in transport bar area
                    if click_row >= transport_row && click_row < shortcuts_row {
                        return self.handle_transport_drag(click_col, state);
                    }

                    // Dragging in visualizer seekbar (Now Playing view)
                    if state.view == View::NowPlaying {
                        if let super::state::NowPlayingMode::NowPlaying = state.now_playing_mode {
                            return self.handle_visualizer_drag(click_col, state);
                        }
                    }

                    // If dragging but mouse is in content area, still update based on horizontal position
                    // This makes seeking feel more responsive
                    if state.playback.duration_ms > 0 {
                        return self.handle_visualizer_drag(click_col, state);
                    }
                }
            }

            // Mouse up - clear drag state
            MouseEventKind::Up(MouseButton::Left) => {
                state.seeking_drag = false;
            }

            // Scroll wheel
            MouseEventKind::ScrollUp => {
                return self.handle_scroll(true, click_col, state);
            }
            MouseEventKind::ScrollDown => {
                return self.handle_scroll(false, click_col, state);
            }

            _ => {}
        }

        vec![]
    }

    /// Handle click on the shortcut bar at the bottom.
    fn handle_shortcut_bar_click(&self, click_col: u16, state: &AppState) -> Vec<Action> {
        // Shortcut bar items (same order as render_shortcuts):
        // ^A artists | ^P playlists | ^G genres | ^O folders | ^N queue | ^F search | F1 help | F2 settings
        //
        // These are centered, so we need to calculate positions based on terminal width.
        // Each item is roughly: " ^X label " with separators "|"
        //
        // Clicking an already-active item cycles its mode (like the keyboard shortcut does).
        // Note: Genres cycle includes Stations (via Ctrl+G).

        let shortcuts: [(&str, &str, usize); 8] = [
            ("^A", state.artist_view_mode.name(), 0),   // Artists
            ("^P", state.playlists_mode.name(), 1),     // Playlists
            ("^G", state.genre_content_type.name(), 2), // Genres (cycles through genres/moods/styles/stations)
            ("^O", "folders", 3),                       // Folders
            ("^N", state.now_playing_mode.name(), 4),   // Now Playing
            ("^F", "search", 5),                        // Search
            ("F1", "help", 6),                          // Help
            ("F2", "settings", 7),                      // Settings
        ];

        // Calculate total width of shortcut bar
        let mut total_width: u16 = 0;
        let mut item_ranges: Vec<(u16, u16, usize)> = Vec::new();

        for (i, (key, label, idx)) in shortcuts.iter().enumerate() {
            let separator_width = if i > 0 { 1 } else { 0 }; // "|"
            let item_width = 1 + key.len() as u16 + 1 + label.len() as u16 + 1; // " ^X label "

            let start = total_width + separator_width;
            let end = start + item_width;
            item_ranges.push((start, end, *idx));
            total_width = end;
        }

        // Center offset
        let center_offset = state.terminal_width.saturating_sub(total_width) / 2;

        // Find which item was clicked
        for (start, end, idx) in item_ranges {
            let abs_start = center_offset + start;
            let abs_end = center_offset + end;
            if click_col >= abs_start && click_col < abs_end {
                return self.shortcut_bar_action(idx, state);
            }
        }

        vec![]
    }

    /// Return the action for clicking a shortcut bar item (with cycling support).
    fn shortcut_bar_action(&self, idx: usize, state: &AppState) -> Vec<Action> {
        match idx {
            0 => {
                // Artists: cycle mode if already there, else switch
                if state.view == View::Browse && state.browse_category == BrowseCategory::Artists {
                    return vec![Action::CycleArtistViewMode];
                }
                vec![Action::SetCategory(BrowseCategory::Artists), Action::SetView(View::Browse)]
            }
            1 => {
                // Playlists: cycle mode if already there, else switch
                if state.view == View::Browse && state.browse_category == BrowseCategory::Playlists {
                    return vec![Action::CyclePlaylistsMode];
                }
                vec![Action::SetCategory(BrowseCategory::Playlists), Action::SetView(View::Browse)]
            }
            2 => {
                // Genres: cycle content type if already there, else switch (includes Stations)
                if state.view == View::Browse && state.browse_category == BrowseCategory::Genres {
                    return vec![Action::CycleGenreContentType];
                }
                vec![Action::SetCategory(BrowseCategory::Genres), Action::SetView(View::Browse)]
            }
            3 => {
                // Folders: just switch (no cycling)
                vec![Action::SetCategory(BrowseCategory::Folders), Action::SetView(View::Browse)]
            }
            4 => {
                // Now Playing: cycle mode if already there, else switch
                if state.view == View::NowPlaying {
                    return vec![Action::CycleNowPlayingMode];
                }
                vec![Action::SetView(View::NowPlaying)]
            }
            5 => {
                // Search: just switch (could cycle tabs but Tab key does that)
                vec![Action::SetView(View::Search)]
            }
            6 => {
                // Help
                vec![Action::SetView(View::Help)]
            }
            7 => {
                // Settings
                vec![Action::SetView(View::Settings)]
            }
            _ => vec![],
        }
    }

    /// Calculate scroll offset to keep selected item visible (same as UI rendering).
    fn calc_scroll_offset(selected: usize, viewport_height: usize, total_items: usize) -> usize {
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

    /// Handle mouse down on the transport bar.
    fn handle_transport_down(&self, click_col: u16, state: &mut AppState) -> Vec<Action> {
        // Transport bar layout (controls on left):
        // [⏸] [MM:SS] [━━━●───────] [MM:SS] [ │ ] [track info...] [...] [🔍]
        // ^0  ^2      ^8           ^28      ^34                          ^end
        //
        // Fixed positions at the start:
        // - Play/pause: cols 0-1 (icon + space)
        // - Position time: cols 2-6 (5 chars MM:SS)
        // - Space: col 7
        // - Seek bar: cols 8-27 (20 chars)
        // - Space: col 28
        // - Duration time: cols 29-33 (5 chars)
        // - Separator: cols 34-38 ("  │  ")
        // - Search emoji at the far right

        // Search emoji at far right (last 4 columns to account for emoji width)
        // Only activate filter in Browse view
        if state.view == View::Browse && click_col >= state.terminal_width.saturating_sub(4) {
            if state.list_filter_active {
                return vec![Action::DeactivateListFilter];
            } else {
                return vec![Action::ActivateListFilter];
            }
        }

        // Play/pause button at columns 0-1
        if click_col < 2 {
            return vec![Action::TogglePlayPause];
        }

        // Seek bar at columns 8-27 (20 chars)
        let seekbar_start = 8u16;
        let seekbar_end = 28u16;
        let seekable_width = 20u16;

        if state.playback.duration_ms > 0 && click_col >= seekbar_start && click_col < seekbar_end {
            let relative_pos = click_col - seekbar_start;

            // Calculate where the indicator currently is
            let progress = state.playback.position_ms as f64 / state.playback.duration_ms as f64;
            let indicator_pos = (progress * seekable_width as f64) as u16;

            // Check if click is on or near the indicator (within 1 char)
            let on_indicator = relative_pos >= indicator_pos.saturating_sub(1)
                && relative_pos <= indicator_pos.saturating_add(1);

            if on_indicator {
                // Start drag mode
                state.seeking_drag = true;
            }

            // Always seek on click
            let seek_progress = (relative_pos as f64 / seekable_width as f64).clamp(0.0, 1.0);
            let seek_ms = (seek_progress * state.playback.duration_ms as f64) as u64;
            return vec![Action::Seek(seek_ms)];
        }

        vec![]
    }

    /// Handle mouse drag on the transport bar (only when seeking_drag is true).
    fn handle_transport_drag(&self, click_col: u16, state: &AppState) -> Vec<Action> {
        if state.playback.duration_ms > 0 {
            let seekbar_start = 8u16;
            let seekable_width = 20u16;

            // Allow dragging slightly outside the bar bounds for smoother interaction
            let clamped_col = click_col.max(seekbar_start).min(seekbar_start + seekable_width);
            let relative_pos = clamped_col - seekbar_start;
            let progress = (relative_pos as f64 / seekable_width as f64).clamp(0.0, 1.0);
            let seek_ms = (progress * state.playback.duration_ms as f64) as u64;
            return vec![Action::Seek(seek_ms)];
        }
        vec![]
    }

    /// Handle click on the left panel (category list).
    fn handle_left_panel_click(&self, click_row: u16, state: &mut AppState) -> Vec<Action> {
        // Left panel has a 1-row border at top
        // Visual row within the list (0-indexed from first visible item)
        let visual_row = click_row.saturating_sub(1) as usize;

        // Calculate visible height for left panel (30 width, content area height minus borders)
        let content_height = state.terminal_height.saturating_sub(4) as usize;
        let visible_height = content_height.saturating_sub(1);

        // Set focus to left panel
        state.focus = Focus::Left;

        // Update the appropriate index based on category
        match state.browse_category {
            BrowseCategory::Artists => {
                let len = state.category_len();
                let selected = state.category_index();
                let scroll_offset = Self::calc_scroll_offset(selected, visible_height, len);
                let actual_idx = visual_row + scroll_offset;

                if actual_idx < len {
                    state.set_category_index(actual_idx);
                    return vec![Action::LoadArtistAlbums];
                }
            }
            BrowseCategory::Playlists => {
                let len = state.category_len();
                let selected = state.category_index();
                let scroll_offset = Self::calc_scroll_offset(selected, visible_height, len);
                let actual_idx = visual_row + scroll_offset;

                if actual_idx < len {
                    state.set_category_index(actual_idx);
                    return vec![Action::LoadCategoryTracks];
                }
            }
            BrowseCategory::Genres => {
                // Stations are now part of the genre content type cycle
                if state.genre_content_type == super::state::GenreContentType::Stations {
                    // Stations use station_nav - select item in focused column
                    if let Some(column) = state.station_nav.columns.get(state.station_nav.focused_column) {
                        let len = column.stations.len();
                        let selected = column.selected_index;
                        let scroll_offset = Self::calc_scroll_offset(selected, visible_height, len);
                        let actual_idx = visual_row + scroll_offset;

                        if actual_idx < len {
                            if let Some(col) = state.station_nav.columns.get_mut(state.station_nav.focused_column) {
                                col.selected_index = actual_idx;
                            }
                        }
                    }
                } else {
                    let len = state.current_genre_list().len();
                    let selected = state.genres_index;
                    let scroll_offset = Self::calc_scroll_offset(selected, visible_height, len);
                    let actual_idx = visual_row + scroll_offset;

                    if actual_idx < len {
                        state.genres_index = actual_idx;
                        // Load albums for this genre
                        return match state.genre_content_type {
                            super::state::GenreContentType::Genres => vec![Action::LoadGenreAlbums],
                            super::state::GenreContentType::ArtistGenres => vec![Action::LoadArtistGenreAlbums],
                            super::state::GenreContentType::AlbumGenres => vec![Action::LoadAlbumGenreAlbums],
                            super::state::GenreContentType::Moods => vec![Action::LoadMoodAlbums],
                            super::state::GenreContentType::Styles => vec![Action::LoadStyleAlbums],
                            super::state::GenreContentType::Stations => vec![], // Handled above
                        };
                    }
                }
            }
            BrowseCategory::Folders => {
                // Folders use folder_state
                if let Some(folder_state) = &mut state.folder_state {
                    if let Some(column) = folder_state.columns.get(folder_state.focused_column) {
                        let len = column.items.len();
                        let selected = column.selected_index;
                        let scroll_offset = Self::calc_scroll_offset(selected, visible_height, len);
                        let actual_idx = visual_row + scroll_offset;

                        if actual_idx < len {
                            if let Some(col) = folder_state.columns.get_mut(folder_state.focused_column) {
                                col.selected_index = actual_idx;
                            }
                        }
                    }
                }
            }
        }

        vec![]
    }

    /// Handle click on the right panel (albums/tracks).
    fn handle_right_panel_click(&self, click_row: u16, _click_col: u16, state: &mut AppState) -> Vec<Action> {
        // Right panel has a 1-row border at top
        // Visual row within the list (0-indexed from first visible item)
        let visual_row = click_row.saturating_sub(1) as usize;

        // Calculate visible height (content area minus transport and shortcuts, minus borders)
        let content_height = state.terminal_height.saturating_sub(4) as usize; // -3 for transport/shortcuts, -1 for top border
        let visible_height = content_height.saturating_sub(1); // Account for bottom border

        // Set focus to right panel
        state.focus = Focus::Right;

        // Handle based on current right panel mode
        match state.right_panel_mode {
            RightPanelMode::ArtistAlbums => {
                // Note: total includes "All Tracks" entry at index 0
                let len = state.selected_artist_albums.len() + 1;
                let selected = state.list_state.right_albums_index;
                let scroll_offset = Self::calc_scroll_offset(selected, visible_height, len);
                let actual_idx = visual_row + scroll_offset;

                if actual_idx < len {
                    let current = state.list_state.right_albums_index;
                    if current == actual_idx {
                        // Double-click behavior: drill into album or All Tracks
                        if actual_idx == 0 {
                            return vec![Action::LoadArtistAllTracks];
                        } else {
                            return vec![Action::LoadSelectedAlbumTracks];
                        }
                    }
                    state.list_state.right_albums_index = actual_idx;
                }
            }
            RightPanelMode::CategoryAlbums => {
                let len = state.genre_albums.len();
                let selected = state.genre_albums_index;
                let scroll_offset = Self::calc_scroll_offset(selected, visible_height, len);
                let actual_idx = visual_row + scroll_offset;

                if actual_idx < len {
                    let current = state.genre_albums_index;
                    if current == actual_idx {
                        // Double-click: drill into album
                        return vec![Action::LoadSelectedAlbumTracks];
                    }
                    state.genre_albums_index = actual_idx;
                }
            }
            RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                let len = state.selected_album_tracks.len();
                let selected = state.list_state.tracks_index;
                let scroll_offset = Self::calc_scroll_offset(selected, visible_height, len);
                let actual_idx = visual_row + scroll_offset;

                if actual_idx < len {
                    let current = state.list_state.tracks_index;
                    if current == actual_idx {
                        // Double-click: play track
                        return vec![Action::PlayTrackFromCategory(actual_idx)];
                    }
                    state.list_state.tracks_index = actual_idx;
                }
            }
            RightPanelMode::Empty => {}
        }

        vec![]
    }

    /// Handle mouse down in Now Playing view.
    fn handle_now_playing_down(&self, click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
        use super::state::NowPlayingMode;

        match state.now_playing_mode {
            NowPlayingMode::Queue => {
                // Queue mode layout: optional artwork (25 cols) + track list
                let artwork_width = if state.artwork_data.is_some() && state.terminal_width > 60 {
                    25u16
                } else {
                    0u16
                };

                // Track list starts after artwork
                if click_col >= artwork_width {
                    // Visual row (accounting for border)
                    let visual_row = click_row.saturating_sub(1) as usize;

                    // Calculate visible height
                    let content_height = state.terminal_height.saturating_sub(5) as usize;
                    let visible_height = content_height;

                    // Combined list: play_history + queue tracks
                    let history_len = state.play_history.len();
                    let tracks_len = if state.playback_mode == super::state::PlaybackMode::Radio {
                        state.radio.tracks.len()
                    } else {
                        state.queue.len()
                    };
                    let total_len = history_len + tracks_len;

                    let selected = state.list_state.queue_index;
                    let scroll_offset = Self::calc_scroll_offset(selected, visible_height, total_len);
                    let actual_idx = visual_row + scroll_offset;

                    if actual_idx < total_len {
                        let current = state.list_state.queue_index;
                        if current == actual_idx {
                            // Double-click: jump to this track in queue
                            if actual_idx >= history_len {
                                let queue_idx = actual_idx - history_len;
                                return vec![Action::JumpToQueueIndex(queue_idx)];
                            }
                        }
                        state.list_state.queue_index = actual_idx;
                    }
                }
            }
            NowPlayingMode::RecentlyPlayed => {
                // Recently played is a simple list with border
                let visual_row = click_row.saturating_sub(1) as usize;
                let content_height = state.terminal_height.saturating_sub(5) as usize;
                let visible_height = content_height;

                let len = state.recently_played_albums.len();
                let selected = state.list_state.recently_played_index;
                let scroll_offset = Self::calc_scroll_offset(selected, visible_height, len);
                let actual_idx = visual_row + scroll_offset;

                if actual_idx < len {
                    let current = state.list_state.recently_played_index;
                    if current == actual_idx {
                        // Double-click: play this album
                        return vec![Action::PlayRecentlyPlayedAlbum(actual_idx)];
                    }
                    state.list_state.recently_played_index = actual_idx;
                }
            }
            NowPlayingMode::NowPlaying => {
                // Visualizer mode: click to seek (enable drag only on indicator)
                let content_height = state.terminal_height.saturating_sub(3);
                let track_info_height = 5u16;
                let visualizer_top = track_info_height;
                let visualizer_bottom = content_height;
                let visualizer_inner_top = visualizer_top + 1;
                let visualizer_inner_bottom = visualizer_bottom.saturating_sub(1);
                let visualizer_inner_left = 1u16;
                let visualizer_inner_right = state.terminal_width.saturating_sub(1);

                // Check if click is within the visualizer inner area (for seeking)
                if click_row >= visualizer_inner_top
                    && click_row < visualizer_inner_bottom
                    && click_col >= visualizer_inner_left
                    && click_col < visualizer_inner_right
                    && state.playback.duration_ms > 0
                {
                    let inner_width = visualizer_inner_right - visualizer_inner_left;

                    // Calculate where the indicator currently is
                    let progress = state.playback.position_ms as f64 / state.playback.duration_ms as f64;
                    let indicator_col = visualizer_inner_left + (progress * inner_width as f64) as u16;

                    // Check if click is on or near the indicator (within 2 chars)
                    let on_indicator = click_col >= indicator_col.saturating_sub(2)
                        && click_col <= indicator_col.saturating_add(2);

                    if on_indicator {
                        // Enable drag mode
                        state.seeking_drag = true;
                    }

                    // Always seek on click
                    let relative_col = click_col - visualizer_inner_left;
                    let seek_progress = relative_col as f64 / inner_width as f64;
                    let seek_ms = (seek_progress * state.playback.duration_ms as f64) as u64;
                    return vec![Action::Seek(seek_ms)];
                }
            }
        }

        vec![]
    }

    /// Handle mouse drag on the visualizer seekbar.
    fn handle_visualizer_drag(&self, click_col: u16, state: &AppState) -> Vec<Action> {
        if state.playback.duration_ms > 0 {
            let visualizer_inner_left = 1u16;
            let visualizer_inner_right = state.terminal_width.saturating_sub(1);
            let inner_width = visualizer_inner_right.saturating_sub(visualizer_inner_left);

            if inner_width > 0 {
                // Clamp to valid range for smoother feel at edges
                let clamped_col = click_col.max(visualizer_inner_left).min(visualizer_inner_right);
                let relative_col = clamped_col - visualizer_inner_left;
                let progress = (relative_col as f64 / inner_width as f64).clamp(0.0, 1.0);
                let seek_ms = (progress * state.playback.duration_ms as f64) as u64;
                return vec![Action::Seek(seek_ms)];
            }
        }
        vec![]
    }

    /// Handle click in Search view.
    fn handle_search_click(&self, click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
        // Search view is a centered popup: 60% width, 70% height
        // Calculate popup bounds
        let content_height = state.terminal_height.saturating_sub(3); // Minus transport + shortcuts
        let content_width = state.terminal_width;

        let popup_height = content_height * 70 / 100;
        let popup_width = content_width * 60 / 100;
        let popup_top = (content_height - popup_height) / 2;
        let popup_left = (content_width - popup_width) / 2;

        // Check if click is within popup
        if click_row < popup_top || click_row >= popup_top + popup_height {
            return vec![];
        }
        if click_col < popup_left || click_col >= popup_left + popup_width {
            return vec![];
        }

        // Convert to popup-relative coordinates
        let rel_row = click_row - popup_top;
        let rel_col = click_col - popup_left;

        // Popup layout: border (1) + tabs (2 rows) + search input (3 rows) + results
        // Tabs are at rows 1-2 (after top border at row 0)
        let tabs_start_row = 1u16;
        let tabs_end_row = 3u16;

        if rel_row >= tabs_start_row && rel_row < tabs_end_row {
            // Click is in tabs area
            // Tabs rendered with Ratatui Tabs widget: "All | Artists | Album Artists | ..."
            // Format: "tab1 | tab2 | tab3" with spaces around separators
            let tab_names = ["All", "Artists", "Album Artists", "Albums", "Playlists", "Tracks", "Genres"];
            let tabs_with_enum = [
                SearchTab::Global,
                SearchTab::Artists,
                SearchTab::AlbumArtists,
                SearchTab::Albums,
                SearchTab::Playlists,
                SearchTab::Tracks,
                SearchTab::Genres,
            ];

            // Calculate tab positions (accounting for left border)
            let mut x: u16 = 1; // After left border
            for (i, name) in tab_names.iter().enumerate() {
                let tab_width = name.len() as u16;
                if rel_col >= x && rel_col < x + tab_width {
                    state.search_tab = tabs_with_enum[i];
                    return vec![];
                }
                x += tab_width;
                // Add separator width " | " = 3 chars
                if i < tab_names.len() - 1 {
                    x += 3;
                }
            }
            return vec![];
        }

        // Results area starts after tabs (3) + search input (3) + border (1) = row 7
        let results_start_row = 7u16;
        if rel_row >= results_start_row {
            let result_row = (rel_row - results_start_row) as usize;

            // Handle based on current tab
            match state.search_tab {
                SearchTab::Global => {
                    // 3-column layout: Artists | Albums | Tracks
                    // Each column is roughly 1/3 of the popup inner width
                    let inner_width = popup_width.saturating_sub(2); // Subtract borders
                    let col_width = inner_width / 3;
                    let rel_inner_col = rel_col.saturating_sub(1); // Subtract left border

                    if let Some(ref results) = state.filter_results {
                        if rel_inner_col < col_width {
                            // Artists column
                            if result_row < results.artists.len() {
                                state.list_state.search_section = super::state::SearchSection::Artists;
                                state.list_state.search_item_index = result_row;
                            }
                        } else if rel_inner_col < col_width * 2 {
                            // Albums column
                            if result_row < results.albums.len() {
                                state.list_state.search_section = super::state::SearchSection::Albums;
                                state.list_state.search_item_index = result_row;
                            }
                        } else {
                            // Tracks column
                            if result_row < results.tracks.len() {
                                state.list_state.search_section = super::state::SearchSection::Tracks;
                                state.list_state.search_item_index = result_row;
                            }
                        }
                    }
                }
                SearchTab::Artists | SearchTab::AlbumArtists => {
                    if let Some(ref results) = state.filter_results {
                        if result_row < results.artists.len() {
                            state.list_state.search_item_index = result_row;
                        }
                    }
                }
                SearchTab::Albums => {
                    if let Some(ref results) = state.filter_results {
                        if result_row < results.albums.len() {
                            state.list_state.search_item_index = result_row;
                        }
                    }
                }
                SearchTab::Playlists => {
                    if let Some(ref results) = state.filter_results {
                        if result_row < results.playlists.len() {
                            state.list_state.search_item_index = result_row;
                        }
                    }
                }
                SearchTab::Tracks => {
                    if let Some(ref results) = state.filter_results {
                        if result_row < results.tracks.len() {
                            state.list_state.search_item_index = result_row;
                        }
                    }
                }
                SearchTab::Genres => {
                    // Genres are filtered from state.genres
                    let query_lower = state.search_query.to_lowercase();
                    let filtered: Vec<_> = state.genres.iter()
                        .filter(|g| g.title.to_lowercase().contains(&query_lower))
                        .collect();
                    if result_row < filtered.len() {
                        state.list_state.search_item_index = result_row;
                    }
                }
            }
        }

        vec![]
    }

    /// Handle click in Auth view (login form, server selection).
    fn handle_auth_click(&self, click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
        use super::state::AuthStep;

        match state.auth_state.step {
            AuthStep::Login => {
                // Login form is centered, 50 chars wide, 12 rows tall
                let form_width = 50u16.min(state.terminal_width.saturating_sub(4));
                let form_height = 12u16;
                let form_x = (state.terminal_width.saturating_sub(form_width)) / 2;
                let form_y = (state.terminal_height.saturating_sub(form_height)) / 2;

                // Check if click is within form bounds
                if click_col >= form_x && click_col < form_x + form_width
                    && click_row >= form_y && click_row < form_y + form_height
                {
                    let rel_row = click_row - form_y;

                    // Form layout:
                    // 0-1: Title (2 rows)
                    // 2-4: Username field (3 rows)
                    // 5-7: Password field (3 rows)
                    // 8-9: Button (2 rows)

                    if rel_row >= 2 && rel_row < 5 {
                        // Username field clicked
                        state.auth_state.field_index = 0;
                        state.auth_state.editing = true;
                    } else if rel_row >= 5 && rel_row < 8 {
                        // Password field clicked
                        state.auth_state.field_index = 1;
                        state.auth_state.editing = true;
                    } else if rel_row >= 8 && rel_row < 10 {
                        // Sign In button clicked
                        state.auth_state.field_index = 2;
                        state.auth_state.editing = false;
                        // Trigger login action
                        return vec![Action::AuthSignIn];
                    }
                }
            }
            AuthStep::ServerSelect => {
                // Server list is centered, calculate bounds
                let list_width = 50u16.min(state.terminal_width.saturating_sub(4));
                let list_height = (state.available_servers.len() as u16).min(10) + 4;
                let list_x = (state.terminal_width.saturating_sub(list_width)) / 2;
                let list_y = (state.terminal_height.saturating_sub(list_height)) / 2;

                if click_col >= list_x && click_col < list_x + list_width
                    && click_row >= list_y && click_row < list_y + list_height
                {
                    let rel_row = click_row - list_y;

                    // Layout: 2 rows instruction, then server list, 1 row hint
                    if rel_row >= 2 && rel_row < list_height - 1 {
                        let server_index = (rel_row - 2) as usize;
                        if server_index < state.available_servers.len() {
                            state.auth_state.server_index = server_index;
                            // Double-click or single click to select
                            return vec![Action::AuthSelectServer];
                        }
                    }
                }
            }
            _ => {}
        }

        vec![]
    }

    /// Handle click in Settings view.
    fn handle_settings_click(&self, click_row: u16, click_col: u16, state: &mut AppState) -> Vec<Action> {
        // Settings layout: left panel (sections) | right panel (items)
        let left_panel_width = 20u16;

        // Account for top border
        let visual_row = click_row.saturating_sub(1) as usize;

        if click_col < left_panel_width {
            // Click on section list
            let sections = super::state::SettingsSection::all();
            if visual_row < sections.len() {
                state.settings_state.section = sections[visual_row];
                state.settings_state.item_index = 0;
                state.settings_state.focus = super::state::SettingsFocus::Sections;
            }
        } else {
            // Click on items in right panel
            state.settings_state.focus = super::state::SettingsFocus::Content;
            // Item count depends on section - just set the index if reasonable
            state.settings_state.item_index = visual_row;
        }

        vec![]
    }

    /// Handle click in Help view.
    fn handle_help_click(&self, click_row: u16, state: &mut AppState) -> Vec<Action> {
        // Help view is scrollable - clicking just sets focus, scroll wheel handles scrolling
        // Could implement click-to-scroll-to-position but for now just acknowledge the click
        let _ = click_row;
        let _ = state;
        vec![]
    }

    /// Handle scroll wheel events.
    fn handle_scroll(&self, up: bool, click_col: u16, state: &mut AppState) -> Vec<Action> {
        let delta: i32 = if up { -3 } else { 3 }; // Scroll 3 items at a time

        match state.view {
            View::Browse => {
                let left_panel_width = 30u16;
                if click_col < left_panel_width {
                    // Scroll left panel
                    let max = state.category_len().saturating_sub(1);
                    let current = state.category_index();
                    let new_idx = (current as i32 + delta).clamp(0, max as i32) as usize;
                    state.set_category_index(new_idx);
                } else {
                    // Scroll right panel
                    match state.right_panel_mode {
                        RightPanelMode::ArtistAlbums => {
                            let max = state.selected_artist_albums.len().saturating_sub(1);
                            let new_idx = (state.list_state.right_albums_index as i32 + delta).clamp(0, max as i32) as usize;
                            state.list_state.right_albums_index = new_idx;
                        }
                        RightPanelMode::CategoryAlbums => {
                            let max = state.genre_albums.len().saturating_sub(1);
                            let new_idx = (state.genre_albums_index as i32 + delta).clamp(0, max as i32) as usize;
                            state.genre_albums_index = new_idx;
                        }
                        RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                            let max = state.selected_album_tracks.len().saturating_sub(1);
                            let new_idx = (state.list_state.tracks_index as i32 + delta).clamp(0, max as i32) as usize;
                            state.list_state.tracks_index = new_idx;
                        }
                        RightPanelMode::Empty => {}
                    }
                }
            }
            View::NowPlaying => {
                // Scroll queue
                let max = state.queue.len().saturating_sub(1);
                let new_idx = (state.list_state.queue_index as i32 + delta).clamp(0, max as i32) as usize;
                state.list_state.queue_index = new_idx;
            }
            View::Search => {
                // Search scrolling handled via keyboard for now
                // (requires proper handling of optional filter_results)
            }
            View::Help => {
                // Scroll help content
                let new_scroll = (state.help_scroll as i32 + delta).max(0) as u16;
                state.help_scroll = new_scroll;
            }
            _ => {}
        }

        vec![]
    }

    /// Handle keyboard input (CUA-style with Ctrl shortcuts).
    fn handle_key(&self, key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
        // Track Alt key state for bottom bar display
        state.alt_held = key.modifiers.contains(KeyModifiers::ALT);

        // Clear error on any key
        if state.last_error.is_some() {
            state.clear_error();
            return vec![];
        }

        // Handle confirm dialog if active
        if state.confirm_dialog.is_some() {
            match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    state.confirm_dialog = None;
                    return self.refresh_current_view(state);
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    state.confirm_dialog = None;
                    return vec![];
                }
                _ => return vec![],
            }
        }

        // Handle input dialog if active
        if let Some(ref mut dialog) = state.input_dialog {
            match key.code {
                KeyCode::Esc => {
                    // Cancel dialog and adventure if it was for adventure length
                    let was_adventure = matches!(dialog.action_type, super::state::InputDialogAction::AdventureLength);
                    state.input_dialog = None;
                    if was_adventure {
                        return vec![Action::CancelAdventure];
                    }
                }
                KeyCode::Enter => {
                    // Confirm dialog
                    let input = dialog.input.clone();
                    let action_type = dialog.action_type.clone();
                    state.input_dialog = None;
                    match action_type {
                        super::state::InputDialogAction::SavePlaylist => {
                            return vec![Action::SaveQueueAsPlaylist(input)];
                        }
                        super::state::InputDialogAction::AdventureLength => {
                            // Parse the length (default to 20)
                            let length = input.parse::<usize>().unwrap_or(20).clamp(5, 100);
                            return vec![Action::SetAdventureLength(length)];
                        }
                    }
                }
                KeyCode::Backspace => {
                    dialog.input.pop();
                }
                KeyCode::Char(c) => {
                    // For adventure length, only allow digits
                    if matches!(dialog.action_type, super::state::InputDialogAction::AdventureLength) {
                        if c.is_ascii_digit() && dialog.input.len() < 3 {
                            dialog.input.push(c);
                        }
                    } else {
                        // Allow all printable characters for other dialogs
                        if dialog.input.len() < 100 {
                            dialog.input.push(c);
                        }
                    }
                }
                _ => {}
            }
            return vec![];
        }

        // Handle adventure mode Esc separately
        if state.adventure.active && !state.adventure.generating {
            if key.code == KeyCode::Esc {
                return vec![Action::CancelAdventure];
            }
        }

        // Global CUA shortcuts (work everywhere)
        match (key.modifiers, key.code) {
            // Quit: Ctrl+Q
            (KeyModifiers::CONTROL, KeyCode::Char('q')) => return vec![Action::Quit],
            // Also allow Ctrl+C/D for terminal convention
            (KeyModifiers::CONTROL, KeyCode::Char('c')) |
            (KeyModifiers::CONTROL, KeyCode::Char('d')) => return vec![Action::Quit],

            // Global navigation shortcuts
            (KeyModifiers::CONTROL, KeyCode::Char('f')) => {
                // Ctrl+F = Search/Filter popup (floating dialog)
                if state.search_popup_active {
                    return vec![Action::CloseSearchPopup];
                } else {
                    return vec![Action::OpenSearchPopup];
                }
            }
            (KeyModifiers::CONTROL, KeyCode::Char('g')) => {
                // Ctrl+G = Genres category, or cycle content type if already there
                if state.view == View::Browse && state.browse_category == BrowseCategory::Genres {
                    // Already in genres view - cycle content type
                    return vec![Action::CycleGenreContentType];
                }
                // Not in genres view - switch to it and reset right panel
                state.browse_category = BrowseCategory::Genres;
                Self::reset_right_panel(state);
                // Load the appropriate content based on current type
                let load_action = match state.genre_content_type {
                    crate::app::state::GenreContentType::Genres => Action::LoadGenres,
                    crate::app::state::GenreContentType::ArtistGenres => Action::LoadArtistGenres,
                    crate::app::state::GenreContentType::AlbumGenres => Action::LoadAlbumGenres,
                    crate::app::state::GenreContentType::Moods => Action::LoadMoods,
                    crate::app::state::GenreContentType::Styles => Action::LoadStyles,
                    crate::app::state::GenreContentType::Stations => Action::LoadStations,
                };
                return vec![load_action, Action::SetView(View::Browse)];
            }
            (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
                // Ctrl+N = Now Playing, or cycle mode if already there
                if state.view == View::NowPlaying {
                    // Already in Now Playing - cycle mode (Queue → Recently Played)
                    return vec![Action::CycleNowPlayingMode];
                }
                return vec![Action::SetView(View::NowPlaying)];
            }
            (KeyModifiers::CONTROL, KeyCode::Char('s')) => {
                // Ctrl+S = Save queue/radio as playlist (in Now Playing with tracks)
                if state.view == View::NowPlaying {
                    let has_tracks = !state.queue.is_empty() || !state.radio.tracks.is_empty();
                    if has_tracks {
                        return vec![Action::PromptSavePlaylist];
                    }
                }
            }
            (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
                // Ctrl+A = Artists category, or cycle view mode if already there
                if state.view == View::Browse && state.browse_category == BrowseCategory::Artists {
                    // Already in artists view - cycle view mode (Artist → Album Artist → Album)
                    return vec![Action::CycleArtistViewMode];
                }
                // Not in artists view - switch to it and reset right panel
                state.browse_category = BrowseCategory::Artists;
                Self::reset_right_panel(state);
                // Only load if data not already preloaded
                let needs_load = match state.artist_view_mode {
                    crate::app::state::ArtistViewMode::Artist |
                    crate::app::state::ArtistViewMode::AlbumArtist => state.artists.is_empty(),
                    crate::app::state::ArtistViewMode::Album => state.albums.is_empty(),
                };
                if needs_load {
                    let load_action = match state.artist_view_mode {
                        crate::app::state::ArtistViewMode::Artist |
                        crate::app::state::ArtistViewMode::AlbumArtist => Action::LoadArtists,
                        crate::app::state::ArtistViewMode::Album => Action::LoadAlbums,
                    };
                    return vec![load_action, Action::SetView(View::Browse)];
                }
                return vec![Action::SetView(View::Browse)];
            }
            (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
                // Ctrl+P = Playlists category, or cycle mode if already there
                if state.view == View::Browse && state.browse_category == BrowseCategory::Playlists {
                    // Already in Playlists - cycle mode (All → Recently Added → Recent)
                    return vec![Action::CyclePlaylistsMode];
                }
                // Not in Playlists - switch to it and reset right panel
                state.browse_category = BrowseCategory::Playlists;
                Self::reset_right_panel(state);
                if state.playlists.is_empty() {
                    return vec![Action::LoadPlaylists, Action::SetView(View::Browse)];
                }
                return vec![Action::SetView(View::Browse)];
            }
            (KeyModifiers::CONTROL, KeyCode::Char('o')) => {
                // Ctrl+O = Folders category
                state.browse_category = BrowseCategory::Folders;
                Self::reset_right_panel(state);
                if state.folder_state.is_none() {
                    return vec![Action::LoadFolderRoot, Action::SetView(View::Browse)];
                }
                return vec![Action::SetView(View::Browse)];
            }

            // Global function keys - work from any screen
            (_, KeyCode::F(1)) => {
                if state.view != View::Help {
                    return vec![Action::SetView(View::Help)];
                }
            }
            (_, KeyCode::F(2)) => {
                if state.view != View::Settings {
                    return vec![Action::OpenSettings];
                }
            }
            (_, KeyCode::F(5)) => {
                // F5 = Refresh current view
                return self.refresh_current_view(state);
            }

            // Playback controls with Ctrl
            (KeyModifiers::CONTROL, KeyCode::Char(' ')) |
            (_, KeyCode::Char(' ')) if state.view != View::Search => {
                return vec![Action::TogglePlayPause];
            }
            (KeyModifiers::CONTROL, KeyCode::Left) => return vec![Action::Previous],
            (KeyModifiers::CONTROL, KeyCode::Right) => return vec![Action::Next],
            (KeyModifiers::CONTROL, KeyCode::Up) => return vec![Action::VolumeUp],
            (KeyModifiers::CONTROL, KeyCode::Down) => return vec![Action::VolumeDown],
            // Shift+Left/Right for seeking (10 second skip)
            (KeyModifiers::SHIFT, KeyCode::Left) => return vec![Action::SeekRelative(-10000)],
            (KeyModifiers::SHIFT, KeyCode::Right) => return vec![Action::SeekRelative(10000)],

            // Alt key commands (global)
            (KeyModifiers::ALT, KeyCode::Char('r')) => {
                // Alt+R = Create radio from current selection
                return self.create_station_from_context(state);
            }
            (KeyModifiers::ALT, KeyCode::Char('e')) => {
                // Alt+E = Enqueue selection
                return vec![Action::EnqueueSelection];
            }
            (KeyModifiers::ALT, KeyCode::Char('s')) => {
                // Alt+S = Similar albums/tracks for current context
                return self.get_similar_action(state);
            }
            (KeyModifiers::ALT, KeyCode::Char('v')) => {
                // Alt+V = Sonic Adventure
                return self.handle_adventure_key(state);
            }
            (KeyModifiers::ALT, KeyCode::Char('o')) => {
                // Alt+O = Context-dependent cycling
                if state.view == View::Browse && state.browse_category == BrowseCategory::Genres {
                    // In Genres: cycle sort order
                    return vec![Action::CycleGenreSort];
                } else if state.view == View::Search {
                    // In Search: cycle through search tabs (same as Tab)
                    state.search_tab = state.search_tab.next();
                    state.list_state.search_item_index = 0;
                    state.list_state.search_section = SearchSection::Artists;
                    if !state.search_query.is_empty() {
                        if state.search_tab == super::state::SearchTab::Global {
                            return vec![Action::ExecuteSearch];
                        } else {
                            return vec![Action::ExecuteFilterSearch];
                        }
                    }
                    return vec![];
                }
                // Alt+O in Visualizer mode does nothing - only one visualizer style (waveform seekbar)
            }

            _ => {}
        }

        // Search popup handling (takes priority over view-specific handling)
        if state.search_popup_active {
            return self.handle_search_keys(key, state);
        }

        // View-specific handling
        match state.view {
            View::Auth => self.handle_auth_keys(key, state),
            View::Browse => self.handle_browse_keys(key, state),
            View::NowPlaying => self.handle_now_playing_keys(key, state),
            View::Search => self.handle_search_keys(key, state),
            View::Similar => self.handle_similar_keys(key, state),
            View::Help => self.handle_help_keys(key, state),
            View::Settings => self.handle_settings_keys(key, state),
        }
    }

    fn handle_auth_keys(&self, key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
        use super::state::AuthStep;

        match state.auth_state.step {
            AuthStep::Checking | AuthStep::Authenticating | AuthStep::Connecting => {
                // No input during these states
                vec![]
            }
            AuthStep::Login => {
                if state.auth_state.editing {
                    // Text input mode
                    match key.code {
                        KeyCode::Char(c) => {
                            if state.auth_state.field_index == 0 {
                                state.auth_state.username_input.push(c);
                            } else if state.auth_state.field_index == 1 {
                                state.auth_state.password_input.push(c);
                            }
                            vec![]
                        }
                        KeyCode::Backspace => {
                            if state.auth_state.field_index == 0 {
                                state.auth_state.username_input.pop();
                            } else if state.auth_state.field_index == 1 {
                                state.auth_state.password_input.pop();
                            }
                            vec![]
                        }
                        KeyCode::Enter => {
                            // Stop editing, move to next field or submit
                            state.auth_state.editing = false;
                            if state.auth_state.field_index < 2 {
                                state.auth_state.field_index += 1;
                            }
                            // If we're now on the sign in button, submit
                            if state.auth_state.field_index == 2 {
                                return vec![Action::AuthSignIn];
                            }
                            vec![]
                        }
                        KeyCode::Esc => {
                            state.auth_state.editing = false;
                            vec![]
                        }
                        KeyCode::Tab => {
                            // Move to next field while editing
                            state.auth_state.editing = false;
                            state.auth_state.field_index = (state.auth_state.field_index + 1) % 3;
                            vec![]
                        }
                        _ => vec![],
                    }
                } else {
                    // Navigation mode
                    match key.code {
                        KeyCode::Up => {
                            if state.auth_state.field_index > 0 {
                                state.auth_state.field_index -= 1;
                            }
                            vec![]
                        }
                        KeyCode::Down | KeyCode::Tab => {
                            if state.auth_state.field_index < 2 {
                                state.auth_state.field_index += 1;
                            }
                            vec![]
                        }
                        KeyCode::BackTab => {
                            if state.auth_state.field_index > 0 {
                                state.auth_state.field_index -= 1;
                            }
                            vec![]
                        }
                        KeyCode::Enter => {
                            if state.auth_state.field_index == 2 {
                                // Sign In button
                                vec![Action::AuthSignIn]
                            } else {
                                // Start editing the field
                                state.auth_state.editing = true;
                                vec![]
                            }
                        }
                        KeyCode::Char(c) => {
                            // Start editing and add the character (for username/password fields)
                            if state.auth_state.field_index < 2 {
                                state.auth_state.editing = true;
                                if state.auth_state.field_index == 0 {
                                    state.auth_state.username_input.push(c);
                                } else {
                                    state.auth_state.password_input.push(c);
                                }
                            }
                            vec![]
                        }
                        _ => vec![],
                    }
                }
            }
            AuthStep::ServerSelect => {
                match key.code {
                    KeyCode::Up => {
                        if state.auth_state.server_index > 0 {
                            state.auth_state.server_index -= 1;
                        }
                        vec![]
                    }
                    KeyCode::Down => {
                        if state.auth_state.server_index + 1 < state.available_servers.len() {
                            state.auth_state.server_index += 1;
                        }
                        vec![]
                    }
                    KeyCode::Enter => {
                        vec![Action::AuthSelectServer]
                    }
                    _ => vec![],
                }
            }
        }
    }

    /// Handle Browse view keys (CUA-style).
    fn handle_browse_keys(&self, key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
        // Inline list filter mode - handle filter-specific keys
        if state.list_filter_active {
            // Check if current focus is on the filter's target column
            use crate::app::state::GenreContentType;
            let focused_on_filter_column = match state.list_filter_category {
                BrowseCategory::Artists => state.artist_nav.focused_column == state.list_filter_column,
                BrowseCategory::Playlists => state.playlist_nav.focused_column == state.list_filter_column,
                BrowseCategory::Genres => {
                    if state.genre_content_type == GenreContentType::Stations {
                        state.station_nav.focused_column == state.list_filter_column
                    } else {
                        state.genre_nav.focused_column == state.list_filter_column
                    }
                }
                BrowseCategory::Folders => {
                    state.folder_state.as_ref()
                        .map(|fs| fs.focused_column == state.list_filter_column)
                        .unwrap_or(false)
                }
            };

            match key.code {
                // Esc always deactivates filter
                KeyCode::Esc => {
                    return vec![Action::DeactivateListFilter];
                }
                // Backspace deletes from filter query
                KeyCode::Backspace => {
                    return vec![Action::DeleteListFilterChar];
                }
                // Up/Down/Enter only intercept when focused on filter column
                KeyCode::Up if focused_on_filter_column => {
                    return vec![Action::FilteredListUp];
                }
                KeyCode::Down if focused_on_filter_column => {
                    return vec![Action::FilteredListDown];
                }
                KeyCode::Enter if focused_on_filter_column => {
                    return vec![Action::SelectFilteredItem];
                }
                // Typing appends to filter query (only unmodified chars)
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) => {
                    return vec![Action::AppendListFilterChar(c)];
                }
                // Other keys (arrows, etc.) fall through to normal handling
                _ => {}
            }
        }

        // Activate filter with / key (when not in filter mode)
        if key.code == KeyCode::Char('/') && !key.modifiers.contains(KeyModifiers::CONTROL) {
            return vec![Action::ActivateListFilter];
        }

        // Tab/Shift+Tab cycles through nav bar views (handle before category-specific handlers)
        // Shift+Up/Down cycles through modes within current category
        match key.code {
            KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                return vec![Action::PrevMode];
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                return vec![Action::NextMode];
            }
            KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
                return vec![Action::PrevView];
            }
            KeyCode::BackTab => {
                return vec![Action::PrevView];
            }
            KeyCode::Tab => {
                return vec![Action::NextView];
            }
            _ => {}
        }

        // Handle Folders category separately (Miller columns view)
        if state.browse_category == BrowseCategory::Folders {
            return self.handle_folder_browse_keys(key, state);
        }

        // Handle Artists category with Miller columns when artist_nav is populated
        if state.browse_category == BrowseCategory::Artists && !state.artist_nav.is_empty() {
            return self.handle_artist_browse_keys(key, state);
        }

        // Handle Playlists category with Miller columns when playlist_nav is populated
        if state.browse_category == BrowseCategory::Playlists && !state.playlist_nav.is_empty() {
            return self.handle_playlist_browse_keys(key, state);
        }

        // Handle Genres category with Miller columns (Genre | Albums | Tracks)
        // When GenreContentType::Stations is active, redirect to station handling
        if state.browse_category == BrowseCategory::Genres {
            if state.genre_content_type == super::state::GenreContentType::Stations {
                return self.handle_station_browse_keys(key, state);
            }
            return self.handle_genre_browse_keys(key, state);
        }

        // Ctrl+R = Create station from current selection (Browse-specific)
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('r') {
            return self.create_station_from_context(state);
        }

        match key.code {
            // Help
            KeyCode::F(1) | KeyCode::Char('?') => vec![Action::SetView(View::Help)],

            // Settings
            KeyCode::F(2) => vec![Action::OpenSettings],

            // Navigation (Tab is handled above, before category-specific handlers)
            KeyCode::Up => vec![Action::ListUp],
            KeyCode::Down => vec![Action::ListDown],
            KeyCode::PageUp => vec![Action::ListPageUp],
            KeyCode::PageDown => vec![Action::ListPageDown],
            KeyCode::Home => vec![Action::ListTop],
            KeyCode::End => vec![Action::ListBottom],

            // Selection/Action - depends on focus and current mode
            KeyCode::Enter | KeyCode::Right => {
                if state.focus == Focus::Left {
                    // Left panel: depends on category
                    match state.browse_category {
                        BrowseCategory::Artists => {
                            // Artist -> load their albums into right panel
                            vec![Action::LoadArtistAlbums]
                        }
                        BrowseCategory::Playlists => {
                            // Playlists -> load tracks directly
                            vec![Action::LoadCategoryTracks]
                        }
                        BrowseCategory::Genres => {
                            // Genre/Mood/Stations -> handled by genre browse keys
                            // (Stations are now part of genre content type cycle)
                            vec![Action::LoadGenreAlbums]
                        }
                        BrowseCategory::Folders => {
                            // Folders use folder navigation
                            vec![Action::LoadFolderRoot]
                        }
                    }
                } else {
                    // Right panel: depends on mode
                    match state.right_panel_mode {
                        RightPanelMode::ArtistAlbums => {
                            // Index 0 = "All Tracks", otherwise album
                            if state.list_state.right_albums_index == 0 {
                                vec![Action::LoadArtistAllTracks]
                            } else {
                                vec![Action::LoadSelectedAlbumTracks]
                            }
                        }
                        RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                            // Track selected -> play it
                            vec![Action::PlayTrackFromCategory(state.list_state.tracks_index)]
                        }
                        RightPanelMode::CategoryAlbums => {
                            // Album selected in genre view -> load album tracks
                            if let Some(album) = state.genre_albums.get(state.genre_albums_index).cloned() {
                                state.selected_album_title = album.title.clone();
                                state.pending_album_key = Some(album.rating_key.clone());
                                vec![Action::LoadAlbumTracks { rating_key: album.rating_key }]
                            } else {
                                vec![]
                            }
                        }
                        RightPanelMode::Empty => vec![],
                    }
                }
            }
            KeyCode::Left | KeyCode::Backspace | KeyCode::Esc => {
                if state.focus == Focus::Right {
                    // Check if we should go back to album list (from tracks view)
                    if state.right_panel_mode == RightPanelMode::AlbumTracks {
                        // If we came from a genre album, go back to CategoryAlbums
                        if state.browse_category == BrowseCategory::Genres {
                            state.right_panel_mode = RightPanelMode::CategoryAlbums;
                            state.selected_album_tracks.clear();
                            vec![]
                        } else {
                            vec![Action::GoBackInRightPanel]
                        }
                    } else {
                        vec![Action::ToggleFocus]
                    }
                } else if state.browse_category == BrowseCategory::Genres && state.genre_content_type == super::state::GenreContentType::Stations {
                    // In stations view (via Genres), go back in Miller columns
                    if state.station_nav.can_go_left() {
                        state.station_nav.focus_left();
                        // Update legacy state to match focused column
                        if let Some(col) = state.station_nav.focused() {
                            state.stations = col.stations.clone();
                            state.stations_index = col.selected_index;
                        }
                    }
                    vec![]
                } else {
                    vec![]
                }
            }

            // Alphabet jumping - jump to first item starting with letter
            // Allow with no modifiers or just SHIFT (for uppercase)
            KeyCode::Char(c) if c.is_ascii_alphabetic() && !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.jump_to_letter(state, c);
                vec![]
            }

            _ => vec![],
        }
    }

    /// Handle folder browsing mode keys (Miller columns style).
    fn handle_folder_browse_keys(&self, key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
        use crate::services::FolderItemType;

        match key.code {
            // Help
            KeyCode::F(1) | KeyCode::Char('?') => vec![Action::SetView(View::Help)],

            // Settings
            KeyCode::F(2) => vec![Action::OpenSettings],

            // Up/Down - navigate within current column
            // BUG FIX: Clear columns to the right when selection changes
            KeyCode::Up => {
                if let Some(ref mut folder_state) = state.folder_state {
                    folder_state.move_up();
                    // Clear columns to the right since selection changed
                    folder_state.truncate_right_columns();
                }
                vec![]
            }
            KeyCode::Down => {
                if let Some(ref mut folder_state) = state.folder_state {
                    folder_state.move_down();
                    // Clear columns to the right since selection changed
                    folder_state.truncate_right_columns();
                }
                vec![]
            }
            KeyCode::PageUp => {
                if let Some(ref mut folder_state) = state.folder_state {
                    if let Some(col) = folder_state.focused_mut() {
                        col.selected_index = col.selected_index.saturating_sub(10);
                    }
                    // Clear columns to the right since selection changed
                    folder_state.truncate_right_columns();
                }
                vec![]
            }
            KeyCode::PageDown => {
                if let Some(ref mut folder_state) = state.folder_state {
                    if let Some(col) = folder_state.focused_mut() {
                        let max = col.items.len().saturating_sub(1);
                        col.selected_index = (col.selected_index + 10).min(max);
                    }
                    // Clear columns to the right since selection changed
                    folder_state.truncate_right_columns();
                }
                vec![]
            }
            KeyCode::Home => {
                if let Some(ref mut folder_state) = state.folder_state {
                    if let Some(col) = folder_state.focused_mut() {
                        col.selected_index = 0;
                    }
                    // Clear columns to the right since selection changed
                    folder_state.truncate_right_columns();
                }
                vec![]
            }
            KeyCode::End => {
                if let Some(ref mut folder_state) = state.folder_state {
                    if let Some(col) = folder_state.focused_mut() {
                        col.selected_index = col.items.len().saturating_sub(1);
                    }
                    // Clear columns to the right since selection changed
                    folder_state.truncate_right_columns();
                }
                vec![]
            }

            // Right/Enter - go into selected folder or play track
            KeyCode::Enter | KeyCode::Right => {
                if let Some(ref mut folder_state) = state.folder_state {
                    // First check if there's already a column to the right we can move to
                    if folder_state.focus_right() {
                        return vec![];
                    }

                    // Otherwise, load the selected item
                    if let Some(item) = folder_state.selected_item().cloned() {
                        match item.item_type {
                            FolderItemType::Folder => {
                                return vec![Action::NavigateIntoFolder(item.key)];
                            }
                            FolderItemType::Track => {
                                return vec![Action::PlayFolderTracks];
                            }
                        }
                    }
                }
                vec![]
            }

            // Left/Backspace - move focus to previous column
            KeyCode::Left | KeyCode::Backspace => {
                if let Some(ref mut folder_state) = state.folder_state {
                    if folder_state.can_go_left() {
                        folder_state.focus_left();
                    }
                }
                vec![]
            }

            // Escape - go back or exit
            KeyCode::Esc => {
                if let Some(ref mut folder_state) = state.folder_state {
                    if folder_state.can_go_left() {
                        folder_state.focus_left();
                        return vec![];
                    }
                }
                vec![]
            }

            // Alphabet jumping in current column
            // Plain letter: jump to first item starting with that letter
            // Shift+letter: jump to first item where first char matches current item's first char
            //               AND second char matches the pressed letter
            KeyCode::Char(c) if c.is_ascii_alphabetic() && !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let letter_lower = c.to_ascii_lowercase();
                let use_second_char = key.modifiers.contains(KeyModifiers::SHIFT);
                if let Some(ref mut folder_state) = state.folder_state {
                    if let Some(col) = folder_state.focused_mut() {
                        if use_second_char {
                            // Get the first letter of the currently selected item
                            let first_letter = col.items.get(col.selected_index)
                                .map(|item| item.title.chars().next())
                                .flatten()
                                .map(|ch| ch.to_ascii_lowercase());

                            if let Some(first_letter) = first_letter {
                                // Find first item starting with that letter AND having pressed letter as second char
                                if let Some(idx) = col.items.iter().position(|item| {
                                    let mut chars = item.title.chars();
                                    let first = chars.next().map(|ch| ch.to_ascii_lowercase());
                                    let second = chars.next().map(|ch| ch.to_ascii_lowercase());
                                    first == Some(first_letter) && second == Some(letter_lower)
                                }) {
                                    col.selected_index = idx;
                                }
                            }
                        } else {
                            // Normal first-letter jump
                            if let Some(idx) = col.items.iter().position(|item| {
                                item.title.chars().next()
                                    .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                                    .unwrap_or(false)
                            }) {
                                col.selected_index = idx;
                            }
                        }
                    }
                    // Clear columns to the right since selection changed
                    folder_state.truncate_right_columns();
                }
                vec![]
            }

            _ => vec![],
        }
    }

    /// Handle Station browsing with Miller columns.
    fn handle_station_browse_keys(&self, key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
        match key.code {
            // Help
            KeyCode::F(1) | KeyCode::Char('?') => vec![Action::SetView(View::Help)],

            // Settings
            KeyCode::F(2) => vec![Action::OpenSettings],

            // Up/Down - navigate within current column
            KeyCode::Up => {
                state.station_nav.move_up();
                // Clear columns to the right since selection changed
                state.station_nav.truncate_right_columns();
                // Update legacy state
                if let Some(col) = state.station_nav.focused() {
                    state.stations_index = col.selected_index;
                }
                vec![]
            }
            KeyCode::Down => {
                state.station_nav.move_down();
                // Clear columns to the right since selection changed
                state.station_nav.truncate_right_columns();
                // Update legacy state
                if let Some(col) = state.station_nav.focused() {
                    state.stations_index = col.selected_index;
                }
                vec![]
            }
            KeyCode::PageUp => {
                if let Some(col) = state.station_nav.focused_mut() {
                    col.selected_index = col.selected_index.saturating_sub(10);
                }
                state.station_nav.truncate_right_columns();
                if let Some(col) = state.station_nav.focused() {
                    state.stations_index = col.selected_index;
                }
                vec![]
            }
            KeyCode::PageDown => {
                if let Some(col) = state.station_nav.focused_mut() {
                    let max = col.stations.len().saturating_sub(1);
                    col.selected_index = (col.selected_index + 10).min(max);
                }
                state.station_nav.truncate_right_columns();
                if let Some(col) = state.station_nav.focused() {
                    state.stations_index = col.selected_index;
                }
                vec![]
            }
            KeyCode::Home => {
                if let Some(col) = state.station_nav.focused_mut() {
                    col.selected_index = 0;
                }
                state.station_nav.truncate_right_columns();
                if let Some(col) = state.station_nav.focused() {
                    state.stations_index = col.selected_index;
                }
                vec![]
            }
            KeyCode::End => {
                if let Some(col) = state.station_nav.focused_mut() {
                    col.selected_index = col.stations.len().saturating_sub(1);
                }
                state.station_nav.truncate_right_columns();
                if let Some(col) = state.station_nav.focused() {
                    state.stations_index = col.selected_index;
                }
                vec![]
            }

            // Right/Enter - drill into category or play station
            KeyCode::Enter | KeyCode::Right => {
                // First check if there's already a column to the right we can move to
                if state.station_nav.focus_right() {
                    // Update legacy state
                    if let Some(col) = state.station_nav.focused() {
                        state.stations = col.stations.clone();
                        state.stations_index = col.selected_index;
                    }
                    return vec![];
                }

                // Otherwise, load the selected station
                if let Some(station) = state.station_nav.selected_station().cloned() {
                    if station.is_category() {
                        return vec![Action::DrillIntoStation(station.key.clone(), station.title.clone())];
                    } else {
                        return vec![Action::PlayStation(station.key.clone())];
                    }
                }
                vec![]
            }

            // Left/Backspace - move focus to previous column
            KeyCode::Left | KeyCode::Backspace => {
                if state.station_nav.can_go_left() {
                    state.station_nav.focus_left();
                    // Update legacy state
                    if let Some(col) = state.station_nav.focused() {
                        state.stations = col.stations.clone();
                        state.stations_index = col.selected_index;
                    }
                }
                vec![]
            }

            // Escape - go back or do nothing
            KeyCode::Esc => {
                if state.station_nav.can_go_left() {
                    state.station_nav.focus_left();
                    if let Some(col) = state.station_nav.focused() {
                        state.stations = col.stations.clone();
                        state.stations_index = col.selected_index;
                    }
                }
                vec![]
            }

            // Alphabet jumping in current column
            // Plain letter: jump to first item starting with that letter
            // Shift+letter: jump to first item where first char matches current item's first char
            //               AND second char matches the pressed letter
            KeyCode::Char(c) if c.is_ascii_alphabetic() && !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let letter_lower = c.to_ascii_lowercase();
                let use_second_char = key.modifiers.contains(KeyModifiers::SHIFT);
                if let Some(col) = state.station_nav.focused_mut() {
                    if use_second_char {
                        // Get the first letter of the currently selected item
                        let first_letter = col.stations.get(col.selected_index)
                            .and_then(|s| s.title.chars().next())
                            .map(|ch| ch.to_ascii_lowercase());

                        if let Some(first_letter) = first_letter {
                            // Find first item starting with that letter AND having pressed letter as second char
                            if let Some(idx) = col.stations.iter().position(|s| {
                                let mut chars = s.title.chars();
                                let first = chars.next().map(|ch| ch.to_ascii_lowercase());
                                let second = chars.next().map(|ch| ch.to_ascii_lowercase());
                                first == Some(first_letter) && second == Some(letter_lower)
                            }) {
                                col.selected_index = idx;
                            }
                        }
                    } else {
                        // Normal first-letter jump
                        if let Some(idx) = col.stations.iter().position(|s| {
                            s.title.chars().next()
                                .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                                .unwrap_or(false)
                        }) {
                            col.selected_index = idx;
                        }
                    }
                }
                state.station_nav.truncate_right_columns();
                if let Some(col) = state.station_nav.focused() {
                    state.stations_index = col.selected_index;
                }
                vec![]
            }

            _ => vec![],
        }
    }

    /// Handle Artist browsing with dynamic Miller columns.
    fn handle_artist_browse_keys(&self, key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
        use super::state::BrowseItem;

        // Handle common navigation keys
        if let Some(actions) = Self::handle_browse_nav_keys(key, &mut state.artist_nav) {
            return actions;
        }

        // Handle Enter/Right - drill down or play track
        if matches!(key.code, KeyCode::Enter | KeyCode::Right) {
            if let Some(item) = state.artist_nav.selected_item().cloned() {
                return match item {
                    BrowseItem::Artist { key, title } => {
                        state.selected_artist_name = title;
                        vec![Action::LoadArtistAlbumsForMiller { artist_key: key }]
                    }
                    BrowseItem::Album { key, title, .. } => {
                        state.selected_album_title = title;
                        vec![Action::LoadAlbumTracksForMiller { album_key: key }]
                    }
                    BrowseItem::AllTracks { artist_key, artist_name } => {
                        state.selected_album_title = format!("All tracks by {}", artist_name);
                        vec![Action::LoadArtistAllTracksForMiller { artist_key }]
                    }
                    BrowseItem::Track { .. } => {
                        if let Some(col) = state.artist_nav.focused() {
                            let idx = col.selected_index;
                            vec![Action::PlayTrackFromMiller { column_index: state.artist_nav.focused_column, track_index: idx }]
                        } else {
                            vec![]
                        }
                    }
                    _ => vec![],
                };
            }
        }

        vec![]
    }

    /// Handle Genre browsing with dynamic Miller columns
    fn handle_genre_browse_keys(&self, key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
        use super::state::BrowseItem;

        // Handle common navigation keys
        if let Some(actions) = Self::handle_browse_nav_keys(key, &mut state.genre_nav) {
            return actions;
        }

        // Handle Enter/Right - drill into selected item or play track
        if matches!(key.code, KeyCode::Enter | KeyCode::Right) {
            if let Some(item) = state.genre_nav.selected_item().cloned() {
                return match item {
                    BrowseItem::Genre { key, .. } => {
                        vec![Action::LoadGenreAlbumsForMiller { genre_key: key }]
                    }
                    BrowseItem::Album { key, .. } => {
                        vec![Action::LoadGenreTracksForMiller { album_key: key }]
                    }
                    BrowseItem::Track { .. } => {
                        if let Some(col) = state.genre_nav.focused() {
                            let idx = col.selected_index;
                            vec![Action::PlayGenreTrackFromMiller { column_index: state.genre_nav.focused_column, track_index: idx }]
                        } else {
                            vec![]
                        }
                    }
                    _ => vec![],
                };
            }
        }

        vec![]
    }

    /// Handle Playlist browsing with dynamic Miller columns
    /// Handle Playlist browsing with dynamic Miller columns
    fn handle_playlist_browse_keys(&self, key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
        use super::state::BrowseItem;

        // Handle common navigation keys
        if let Some(actions) = Self::handle_browse_nav_keys(key, &mut state.playlist_nav) {
            return actions;
        }

        // Handle Enter/Right - drill into playlist/album or play track
        if matches!(key.code, KeyCode::Enter | KeyCode::Right) {
            if let Some(item) = state.playlist_nav.selected_item().cloned() {
                return match item {
                    BrowseItem::Playlist { key, .. } => {
                        vec![Action::LoadPlaylistTracksForMiller { playlist_key: key }]
                    }
                    BrowseItem::Album { key, title, .. } => {
                        // For Recently Added mode - load album tracks
                        state.selected_album_title = title;
                        vec![Action::LoadAlbumTracksForPlaylistMiller { album_key: key }]
                    }
                    BrowseItem::Track { .. } => {
                        if let Some(col) = state.playlist_nav.focused() {
                            let idx = col.selected_index;
                            vec![Action::PlayPlaylistTrackFromMiller { column_index: state.playlist_nav.focused_column, track_index: idx }]
                        } else {
                            vec![]
                        }
                    }
                    _ => vec![],
                };
            }
        }

        vec![]
    }

    /// Get the similar albums/tracks action based on current context.
    fn get_similar_action(&self, state: &mut AppState) -> Vec<Action> {
        // Store current view so we can return to it
        state.previous_view = Some(state.view);

        // In Now Playing view, use the selected track
        if state.view == View::NowPlaying {
            let track = match state.playback_mode {
                PlaybackMode::Queue | PlaybackMode::None => {
                    state.queue.get(state.list_state.queue_index).cloned()
                }
                PlaybackMode::Radio => {
                    state.radio.tracks.get(state.list_state.queue_index).cloned()
                }
            };
            if let Some(track) = track {
                let title = format!("{} - {}", track.artist_name(), track.title);
                return vec![Action::LoadSimilarTracks {
                    rating_key: track.rating_key.clone(),
                    title,
                }];
            }
        }
        // When in right panel showing albums for an artist, use selected album
        // Index 0 is "All Tracks", so skip it for similar albums
        else if state.focus == Focus::Right && state.right_panel_mode == RightPanelMode::ArtistAlbums {
            let album_idx = state.list_state.right_albums_index.saturating_sub(1);
            if state.list_state.right_albums_index > 0 {
                if let Some(album) = state.selected_artist_albums.get(album_idx) {
                    let title = format!("{} - {}", album.artist_name(), album.title);
                    return vec![Action::LoadSimilarAlbums {
                        rating_key: album.rating_key.clone(),
                        title,
                    }];
                }
            }
        }
        // When in genre albums, use selected album
        else if state.focus == Focus::Right && state.right_panel_mode == RightPanelMode::CategoryAlbums {
            if let Some(album) = state.genre_albums.get(state.genre_albums_index) {
                let title = format!("{} - {}", album.artist_name(), album.title);
                return vec![Action::LoadSimilarAlbums {
                    rating_key: album.rating_key.clone(),
                    title,
                }];
            }
        }
        // When viewing tracks, use the selected track
        else if state.focus == Focus::Right && (state.right_panel_mode == RightPanelMode::AlbumTracks || state.right_panel_mode == RightPanelMode::CategoryTracks) {
            if let Some(track) = state.selected_album_tracks.get(state.list_state.tracks_index) {
                let title = format!("{} - {}", track.artist_name(), track.title);
                return vec![Action::LoadSimilarTracks {
                    rating_key: track.rating_key.clone(),
                    title,
                }];
            }
        }
        // Otherwise, use the now-playing track
        else if let Some(track) = state.current_track().cloned() {
            let title = format!("{} - {}", track.artist_name(), track.title);
            return vec![Action::LoadSimilarTracks {
                rating_key: track.rating_key.clone(),
                title,
            }];
        }
        vec![]
    }

    /// Reset right panel state when switching categories.
    /// Clears album/track selections and resets focus to left panel.
    fn reset_right_panel(state: &mut AppState) {
        state.right_panel_mode = RightPanelMode::Empty;
        state.focus = Focus::Left;
        state.selected_artist_albums.clear();
        state.selected_album_tracks.clear();
        state.genre_albums.clear();
        state.genre_albums_index = 0;
        state.selected_artist_name.clear();
        state.selected_album_title.clear();
    }

    /// Create a station from current context (artist, album, or track).
    /// Track selected -> Track radio (individual similar tracks)
    /// Album selected -> Album radio (similar albums played in order)
    /// Artist selected -> Artist radio
    fn create_station_from_context(&self, state: &AppState) -> Vec<Action> {
        // If viewing album tracks, create TRACK radio for the highlighted track
        if state.focus == Focus::Right && state.right_panel_mode == RightPanelMode::AlbumTracks {
            if let Some(track) = state.selected_album_tracks.get(state.list_state.tracks_index) {
                let title = format!("{} - {}", track.artist_name(), track.title);
                return vec![Action::StartTrackRadio {
                    track_key: track.rating_key.clone(),
                    title,
                }];
            }
        }
        // If viewing category tracks (playlist, etc), create TRACK radio
        else if state.focus == Focus::Right && state.right_panel_mode == RightPanelMode::CategoryTracks {
            if let Some(track) = state.selected_album_tracks.get(state.list_state.tracks_index) {
                let title = format!("{} - {}", track.artist_name(), track.title);
                return vec![Action::StartTrackRadio {
                    track_key: track.rating_key.clone(),
                    title,
                }];
            }
        }
        // If viewing artist albums, check what's selected
        else if state.focus == Focus::Right && state.right_panel_mode == RightPanelMode::ArtistAlbums {
            // Index 0 is "All Tracks" - create artist radio
            if state.list_state.right_albums_index == 0 {
                if let Some(artist) = state.artists.get(state.list_state.artists_index) {
                    return vec![Action::StartArtistRadio {
                        artist_key: artist.rating_key.clone(),
                        title: artist.title.clone(),
                    }];
                }
            }
            // Otherwise, create album radio for the selected album
            else if let Some(album) = state.selected_artist_albums.get(state.list_state.right_albums_index - 1) {
                return vec![Action::StartAlbumRadio {
                    album_key: album.rating_key.clone(),
                    title: album.title.clone(),
                }];
            }
        }
        // If viewing genre/mood albums, create album radio
        else if state.focus == Focus::Right && state.right_panel_mode == RightPanelMode::CategoryAlbums {
            if let Some(album) = state.genre_albums.get(state.genre_albums_index) {
                return vec![Action::StartAlbumRadio {
                    album_key: album.rating_key.clone(),
                    title: album.title.clone(),
                }];
            }
        }
        // If focused on left panel artist, create artist radio
        else if state.focus == Focus::Left && state.browse_category == BrowseCategory::Artists {
            if let Some(artist) = state.artists.get(state.list_state.artists_index) {
                return vec![Action::StartArtistRadio {
                    artist_key: artist.rating_key.clone(),
                    title: artist.title.clone(),
                }];
            }
        }
        // Otherwise, use the current playing track
        else if let Some(track) = state.current_track() {
            let title = format!("{} - {}", track.artist_name(), track.title);
            return vec![Action::StartTrackRadio {
                track_key: track.rating_key.clone(),
                title,
            }];
        }
        vec![]
    }

    /// Handle Alt+V for Sonic Adventure.
    fn handle_adventure_key(&self, state: &mut AppState) -> Vec<Action> {
        // Ignore if already generating
        if state.adventure.generating {
            return vec![];
        }

        // Get the currently selected/highlighted track
        let selected_track = self.get_selected_track(state);

        if !state.adventure.active {
            // Start adventure mode
            if let Some(track) = selected_track {
                return vec![Action::SetAdventureStart(track)];
            } else {
                return vec![Action::StartAdventure];
            }
        }

        // Adventure mode is active
        if state.adventure.start_track.is_some() && state.adventure.end_track.is_none() {
            // Set end track
            if let Some(track) = selected_track {
                return vec![Action::SetAdventureEnd(track)];
            }
        }

        vec![]
    }

    /// Get the currently selected/highlighted track based on context.
    /// Returns the track the user is highlighting in any view where tracks are visible.
    fn get_selected_track(&self, state: &AppState) -> Option<Track> {
        match state.view {
            // Search/Filter view - handle both Global search and tab-specific filters
            View::Search => {
                let idx = state.list_state.search_item_index;

                match state.search_tab {
                    // Global search - uses search_results with sections
                    SearchTab::Global => {
                        if state.list_state.search_section == SearchSection::Tracks {
                            if let Some(ref results) = state.search_results {
                                return results.tracks.get(idx).cloned();
                            }
                        }
                        None
                    }
                    // Tracks tab - uses filter_results
                    SearchTab::Tracks => {
                        if let Some(ref results) = state.filter_results {
                            return results.tracks.get(idx).cloned();
                        }
                        None
                    }
                    // Other tabs don't show tracks directly
                    _ => None
                }
            }

            // Now Playing view - get highlighted track from queue or radio
            View::NowPlaying => {
                let idx = state.list_state.queue_index;
                match state.playback_mode {
                    PlaybackMode::Queue | PlaybackMode::None => {
                        // Account for play history offset
                        let history_len = state.play_history.len();
                        if idx < history_len {
                            state.play_history.get(idx).cloned()
                        } else {
                            state.queue.get(idx - history_len).cloned()
                        }
                    }
                    PlaybackMode::Radio => {
                        state.radio.tracks.get(idx).cloned()
                    }
                }
            }

            // Browse view - check if tracks are showing in right panel
            View::Browse => {
                match state.right_panel_mode {
                    RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                        state.selected_album_tracks.get(state.list_state.tracks_index).cloned()
                    }
                    _ => None
                }
            }

            // Similar view - check if showing similar tracks
            View::Similar => {
                // Similar view shows albums by default, not individual tracks
                None
            }

            // Other views don't show selectable tracks
            _ => None
        }
    }

    /// Jump to first item in current list starting with given letter.
    /// Uses sort_key logic to match the sorting (ignores "The " prefix).
    fn jump_to_letter(&self, state: &mut AppState, letter: char) {
        let letter_lower = letter.to_ascii_lowercase();

        // Check if sort key starts with the given letter (matches sorting logic)
        let starts_with = |title: &str| -> bool {
            sort_key(title).chars().next()
                .map(|c| c.to_ascii_lowercase() == letter_lower)
                .unwrap_or(false)
        };

        if state.focus == Focus::Left {
            // Jump in category list
            match state.browse_category {
                BrowseCategory::Artists => {
                    if let Some(idx) = state.artists.iter().position(|a| starts_with(&a.title)) {
                        state.list_state.artists_index = idx;
                    }
                }
                BrowseCategory::Playlists => {
                    if let Some(idx) = state.playlists.iter().position(|p| starts_with(&p.title)) {
                        state.list_state.playlists_index = idx;
                    }
                }
                BrowseCategory::Genres => {
                    // Stations are now accessed via genre content type
                    if state.genre_content_type == super::state::GenreContentType::Stations {
                        if let Some(idx) = state.stations.iter().position(|s| starts_with(&s.title)) {
                            state.stations_index = idx;
                        }
                    } else if let Some(idx) = state.genres.iter().position(|g| starts_with(&g.title)) {
                        state.genres_index = idx;
                    }
                }
                BrowseCategory::Folders => {
                    // Handled separately in folder navigation
                }
            }
        } else {
            // Jump in right panel
            match state.right_panel_mode {
                RightPanelMode::ArtistAlbums => {
                    // +1 offset for "All Tracks" at index 0
                    if let Some(idx) = state.selected_artist_albums.iter().position(|a| starts_with(&a.title)) {
                        state.list_state.right_albums_index = idx + 1;
                    }
                }
                RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                    if let Some(idx) = state.selected_album_tracks.iter().position(|t| starts_with(&t.title)) {
                        state.list_state.tracks_index = idx;
                    }
                }
                RightPanelMode::CategoryAlbums => {
                    if let Some(idx) = state.genre_albums.iter().position(|a| starts_with(&a.title)) {
                        state.genre_albums_index = idx;
                    }
                }
                RightPanelMode::Empty => {}
            }
        }
    }

    /// Handle Queue view keys (CUA-style).
    /// Handle Now Playing view keys (unified queue/radio/playlist view).
    fn handle_now_playing_keys(&self, key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
        // Get the max index based on current mode
        let get_max_index = |state: &AppState| -> usize {
            match state.now_playing_mode {
                super::state::NowPlayingMode::RecentlyPlayed => {
                    state.recently_played_albums.len().saturating_sub(1)
                }
                _ => match state.playback_mode {
                    PlaybackMode::Queue | PlaybackMode::None => state.queue.len().saturating_sub(1),
                    PlaybackMode::Radio => state.radio.tracks.len().saturating_sub(1),
                }
            }
        };

        match key.code {
            KeyCode::Esc => vec![Action::SetView(View::Browse)],
            KeyCode::F(1) | KeyCode::Char('?') => vec![Action::SetView(View::Help)],

            // Tab/Shift+Tab cycles through nav bar views
            KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => vec![Action::PrevView],
            KeyCode::Tab => vec![Action::NextView],

            KeyCode::Up => {
                if state.list_state.queue_index > 0 {
                    state.list_state.queue_index -= 1;
                }
                vec![]
            }
            KeyCode::Down => {
                let max = get_max_index(state);
                state.list_state.queue_index = (state.list_state.queue_index + 1).min(max);
                vec![]
            }
            KeyCode::PageUp => {
                state.list_state.queue_index = state.list_state.queue_index.saturating_sub(10);
                vec![]
            }
            KeyCode::PageDown => {
                let max = get_max_index(state);
                state.list_state.queue_index = (state.list_state.queue_index + 10).min(max);
                vec![]
            }
            KeyCode::Home => {
                state.list_state.queue_index = 0;
                vec![]
            }
            KeyCode::End => {
                let max = get_max_index(state);
                state.list_state.queue_index = max;
                vec![]
            }

            KeyCode::Enter => {
                // Handle Recently Played mode first
                if state.now_playing_mode == super::state::NowPlayingMode::RecentlyPlayed {
                    // Play selected album from recently played
                    if let Some(album) = state.recently_played_albums.get(state.list_state.queue_index).cloned() {
                        return vec![Action::PlayAlbum { rating_key: album.rating_key }];
                    }
                    return vec![];
                }

                // Play selected item from queue or radio
                match state.playback_mode {
                    PlaybackMode::Queue | PlaybackMode::None => {
                        if let Some(track) = state.queue.get(state.list_state.queue_index).cloned() {
                            state.queue_index = Some(state.list_state.queue_index);
                            vec![Action::PlayTrack(track)]
                        } else {
                            vec![]
                        }
                    }
                    PlaybackMode::Radio => {
                        // Jump to selected radio track without clearing radio state
                        if state.list_state.queue_index < state.radio.tracks.len() {
                            vec![Action::JumpToRadioTrack(state.list_state.queue_index)]
                        } else {
                            vec![]
                        }
                    }
                }
            }

            KeyCode::Delete => {
                // Only allow delete in queue mode
                if state.playback_mode == PlaybackMode::Queue {
                    vec![Action::RemoveFromQueue(state.list_state.queue_index)]
                } else {
                    vec![]
                }
            }

            // Alt+O: Cycle queue sort mode (Queue Order -> Album -> Shuffle)
            KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::ALT) => {
                // Only applies in Queue mode
                if state.playback_mode == PlaybackMode::Queue && !state.queue.is_empty() {
                    use super::state::QueueSortMode;
                    use rand::seq::SliceRandom;

                    let new_mode = state.queue_sort_mode.next();

                    // Save original order if not already saved
                    if state.queue_original.is_empty() {
                        state.queue_original = state.queue.clone();
                    }

                    // Get current track key to preserve position
                    let current_track_key = state.queue_index
                        .and_then(|idx| state.queue.get(idx))
                        .map(|t| t.rating_key.clone());

                    match new_mode {
                        QueueSortMode::QueueOrder => {
                            // Restore original order
                            state.queue = state.queue_original.clone();
                        }
                        QueueSortMode::Album => {
                            // Sort by artist, then album, then track number
                            state.queue.sort_by(|a, b| {
                                // First compare by artist (grandparent title)
                                let artist_a = a.grandparent_title.as_deref().unwrap_or("");
                                let artist_b = b.grandparent_title.as_deref().unwrap_or("");
                                match sort_key(artist_a).cmp(&sort_key(artist_b)) {
                                    std::cmp::Ordering::Equal => {
                                        // Then by album
                                        match a.parent_rating_key.cmp(&b.parent_rating_key) {
                                            std::cmp::Ordering::Equal => a.index.cmp(&b.index),
                                            other => other,
                                        }
                                    }
                                    other => other,
                                }
                            });
                        }
                        QueueSortMode::Shuffle => {
                            // Shuffle the queue, keeping current track at top
                            if let Some(current_idx) = state.queue_index {
                                if let Some(current_track) = state.queue.get(current_idx).cloned() {
                                    // Remove current track, shuffle rest, put current at front
                                    state.queue.remove(current_idx);
                                    let mut rng = rand::rng();
                                    state.queue.shuffle(&mut rng);
                                    state.queue.insert(0, current_track);
                                    state.queue_index = Some(0);
                                }
                            } else {
                                // No current track, just shuffle
                                let mut rng = rand::rng();
                                state.queue.shuffle(&mut rng);
                            }
                        }
                    }

                    // Restore queue_index to point to same track (for non-shuffle modes)
                    if new_mode != QueueSortMode::Shuffle {
                        if let Some(key) = current_track_key {
                            state.queue_index = state.queue.iter().position(|t| t.rating_key == key);
                        }
                    }

                    // Update list selection to match
                    state.list_state.queue_index = state.queue_index.unwrap_or(0);
                    state.queue_sort_mode = new_mode;
                    state.set_status(format!("Queue: {}", new_mode.name()));
                }
                vec![]
            }

            // Left/Right arrow seeking in visualizer mode (1 second increments)
            KeyCode::Left if state.now_playing_mode == super::state::NowPlayingMode::NowPlaying => {
                vec![Action::SeekRelative(-1000)]
            }
            KeyCode::Right if state.now_playing_mode == super::state::NowPlayingMode::NowPlaying => {
                vec![Action::SeekRelative(1000)]
            }

            // Alphabet jumping
            KeyCode::Char(c) if c.is_ascii_alphabetic() && key.modifiers.is_empty() => {
                let letter_lower = c.to_ascii_lowercase();
                let tracks: &[Track] = match state.playback_mode {
                    PlaybackMode::Queue | PlaybackMode::None => &state.queue,
                    PlaybackMode::Radio => &state.radio.tracks,
                };
                if let Some(idx) = tracks.iter().position(|t| {
                    t.title.chars().next()
                        .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                        .unwrap_or(false)
                }) {
                    state.list_state.queue_index = idx;
                }
                vec![]
            }

            _ => vec![],
        }
    }

    /// Handle unified Search view keys (with tabs for Global/Artists/Playlists/Tracks/Genres).
    fn handle_search_keys(&self, key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
        use super::state::SearchTab;

        match key.code {
            KeyCode::Esc => {
                state.search_query.clear();
                state.search_results = None;
                state.filter_results = None;
                // Close popup if active, otherwise return to Browse view
                if state.search_popup_active {
                    vec![Action::CloseSearchPopup]
                } else {
                    vec![Action::SetView(View::Browse)]
                }
            }
            KeyCode::Enter => {
                match state.search_tab {
                    SearchTab::Global => {
                        if state.search_results.is_some() {
                            self.select_search_result(state)
                        } else if !state.search_query.is_empty() && !state.search_loading {
                            // Only trigger new search if not already loading
                            // (avoids discarding pending search results)
                            vec![Action::ExecuteSearch]
                        } else {
                            vec![]  // Wait for pending search to complete
                        }
                    }
                    _ => {
                        // Filter tabs - select filter result (only if not loading)
                        if !state.filter_loading {
                            vec![Action::SelectFilterResult]
                        } else {
                            vec![]  // Wait for pending filter to complete
                        }
                    }
                }
            }
            KeyCode::Backspace => {
                state.search_query.pop();
                state.list_state.search_item_index = 0;
                // Clear old results when modifying query
                state.search_results = None;
                state.filter_results = None;
                // Trigger search for all tabs (requires 2+ chars)
                if state.search_query.len() >= 2 {
                    match state.search_tab {
                        SearchTab::Global => vec![Action::ExecuteSearch],
                        _ => vec![Action::ExecuteFilterSearch],
                    }
                } else {
                    vec![]
                }
            }
            KeyCode::Up => {
                match state.search_tab {
                    SearchTab::Global => {
                        self.navigate_search_results(state, -1);
                        vec![]
                    }
                    _ => vec![Action::ListUp],
                }
            }
            KeyCode::Down => {
                match state.search_tab {
                    SearchTab::Global => {
                        self.navigate_search_results(state, 1);
                        vec![]
                    }
                    _ => vec![Action::ListDown],
                }
            }
            KeyCode::Tab => {
                // Tab always switches between search tabs
                state.search_tab = state.search_tab.next();
                state.list_state.search_item_index = 0;
                state.list_state.search_section = SearchSection::Artists;
                // Trigger appropriate search for new tab if we have a query
                if !state.search_query.is_empty() {
                    if state.search_tab == SearchTab::Global {
                        return vec![Action::ExecuteSearch];
                    } else {
                        return vec![Action::ExecuteFilterSearch];
                    }
                }
                vec![]
            }
            KeyCode::BackTab => {
                // Shift+Tab switches to previous tab
                state.search_tab = state.search_tab.prev();
                state.list_state.search_item_index = 0;
                state.list_state.search_section = SearchSection::Artists;
                if !state.search_query.is_empty() {
                    if state.search_tab == SearchTab::Global {
                        return vec![Action::ExecuteSearch];
                    } else {
                        return vec![Action::ExecuteFilterSearch];
                    }
                }
                vec![]
            }
            KeyCode::Left => {
                // Left arrow switches sections within Global search results
                if state.search_tab == SearchTab::Global && state.search_results.is_some() {
                    self.next_search_section(state, -1);
                }
                vec![]
            }
            KeyCode::Right => {
                // Right arrow switches sections within Global search results
                if state.search_tab == SearchTab::Global && state.search_results.is_some() {
                    self.next_search_section(state, 1);
                }
                vec![]
            }
            KeyCode::Char(c) => {
                state.search_query.push(c);
                state.list_state.search_item_index = 0;
                // Clear old results when typing new query
                state.search_results = None;
                state.filter_results = None;
                // Trigger search for all tabs (requires 2+ chars)
                if state.search_query.len() >= 2 {
                    match state.search_tab {
                        SearchTab::Global => vec![Action::ExecuteSearch],
                        _ => vec![Action::ExecuteFilterSearch],
                    }
                } else {
                    vec![]
                }
            }
            _ => vec![],
        }
    }

    /// Handle Similar view keys (CUA-style).
    fn handle_similar_keys(&self, key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
        use super::state::SimilarMode;

        match key.code {
            KeyCode::Esc => {
                // Return to previous view, or Browse if none
                let target = state.previous_view.take().unwrap_or(View::Browse);
                vec![Action::SetView(target)]
            }
            KeyCode::F(1) | KeyCode::Char('?') => vec![Action::SetView(View::Help)],

            KeyCode::Up => vec![Action::ListUp],
            KeyCode::Down => vec![Action::ListDown],
            KeyCode::PageUp => vec![Action::ListPageUp],
            KeyCode::PageDown => vec![Action::ListPageDown],
            KeyCode::Home => vec![Action::ListTop],
            KeyCode::End => vec![Action::ListBottom],

            KeyCode::Enter => {
                match state.similar_mode {
                    SimilarMode::Albums => {
                        // Navigate to selected similar album - show as artist's album view
                        if let Some(album) = state.similar_albums.get(state.list_state.similar_index).cloned() {
                            state.pending_album_key = Some(album.rating_key.clone());
                            state.selected_album_title = album.title.clone();
                            state.selected_artist_name = album.artist_name().to_string();
                            state.view = View::Browse;
                            state.browse_category = BrowseCategory::Artists;
                            if let Some(artist_key) = &album.parent_rating_key {
                                if let Some(idx) = state.artists.iter().position(|a| &a.rating_key == artist_key) {
                                    state.list_state.artists_index = idx;
                                }
                            }
                            vec![Action::LoadArtistAlbums]
                        } else {
                            vec![]
                        }
                    }
                    SimilarMode::Tracks => {
                        // Play selected track and queue remaining similar tracks
                        let idx = state.list_state.similar_index;
                        if idx < state.similar_tracks.len() {
                            state.queue = state.similar_tracks[idx..].to_vec();
                            state.queue_index = Some(0);
                            if let Some(track) = state.queue.first().cloned() {
                                vec![Action::PlayTrack(track)]
                            } else {
                                vec![]
                            }
                        } else {
                            vec![]
                        }
                    }
                }
            }

            // Alphabet jumping
            KeyCode::Char(c) if c.is_ascii_alphabetic() && key.modifiers.is_empty() => {
                let letter_lower = c.to_ascii_lowercase();
                match state.similar_mode {
                    SimilarMode::Albums => {
                        if let Some(idx) = state.similar_albums.iter().position(|a| {
                            a.title.chars().next()
                                .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                                .unwrap_or(false)
                        }) {
                            state.list_state.similar_index = idx;
                        }
                    }
                    SimilarMode::Tracks => {
                        if let Some(idx) = state.similar_tracks.iter().position(|t| {
                            t.title.chars().next()
                                .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                                .unwrap_or(false)
                        }) {
                            state.list_state.similar_index = idx;
                        }
                    }
                }
                vec![]
            }

            _ => vec![],
        }
    }

    /// Handle Help view keys.
    fn handle_help_keys(&self, key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
        match key.code {
            KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('?') => {
                state.help_scroll = 0;  // Reset scroll when closing
                vec![Action::SetView(View::Browse)]
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.help_scroll = state.help_scroll.saturating_sub(1);
                vec![]
            }
            KeyCode::Down | KeyCode::Char('j') => {
                // Cap at max reasonable scroll (help text is ~140 lines)
                let max_scroll = 140u16.saturating_sub(state.terminal_height.saturating_sub(4));
                state.help_scroll = state.help_scroll.saturating_add(1).min(max_scroll);
                vec![]
            }
            KeyCode::PageUp => {
                state.help_scroll = state.help_scroll.saturating_sub(20);
                vec![]
            }
            KeyCode::PageDown => {
                // Cap at max reasonable scroll (help text is ~140 lines)
                let max_scroll = 140u16.saturating_sub(state.terminal_height.saturating_sub(4));
                state.help_scroll = state.help_scroll.saturating_add(20).min(max_scroll);
                vec![]
            }
            KeyCode::Home => {
                state.help_scroll = 0;
                vec![]
            }
            KeyCode::End => {
                // Set to max scroll based on terminal height
                let max_scroll = 140u16.saturating_sub(state.terminal_height.saturating_sub(4));
                state.help_scroll = max_scroll;
                vec![]
            }
            _ => vec![],
        }
    }

    /// Handle Settings view keys.
    fn handle_settings_keys(&self, key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
        use crate::app::state::{CredentialField, SettingsFocus, SettingsSection};

        // Handle credential editing mode first
        if let Some(field) = state.settings_state.editing_credential {
            match key.code {
                KeyCode::Esc => {
                    // Cancel editing, restore original value
                    state.settings_state.editing_credential = None;
                    // Restore username from stored auth or config
                    state.settings_state.username_input = PlexAuth::load_token()
                        .and_then(|s| s.username)
                        .or_else(|| self.config.plex.username.clone())
                        .unwrap_or_default();
                    state.settings_state.password_input = String::new();
                    return vec![];
                }
                KeyCode::Enter => {
                    // Save credential and exit edit mode
                    state.settings_state.editing_credential = None;
                    return vec![Action::SaveCredentials];
                }
                KeyCode::Backspace => {
                    // Delete last character
                    match field {
                        CredentialField::Username => {
                            state.settings_state.username_input.pop();
                        }
                        CredentialField::Password => {
                            state.settings_state.password_input.pop();
                        }
                    }
                    return vec![];
                }
                KeyCode::Char(c) => {
                    // Add character to input
                    match field {
                        CredentialField::Username => {
                            state.settings_state.username_input.push(c);
                        }
                        CredentialField::Password => {
                            state.settings_state.password_input.push(c);
                        }
                    }
                    return vec![];
                }
                _ => return vec![],
            }
        }

        match key.code {
            KeyCode::Esc => {
                vec![Action::SetView(View::Browse)]
            }
            // Panel switching
            KeyCode::Tab | KeyCode::Right => {
                if state.settings_state.focus == SettingsFocus::Sections {
                    state.settings_state.focus = SettingsFocus::Content;
                    state.settings_state.item_index = 0;
                }
                vec![]
            }
            KeyCode::BackTab | KeyCode::Left => {
                if state.settings_state.focus == SettingsFocus::Content {
                    state.settings_state.focus = SettingsFocus::Sections;
                }
                vec![]
            }
            KeyCode::Up => {
                match state.settings_state.focus {
                    SettingsFocus::Sections => {
                        // Navigate sections
                        state.settings_state.section = state.settings_state.section.prev();
                        state.settings_state.item_index = 0;
                    }
                    SettingsFocus::Content => {
                        // Navigate items within section
                        if state.settings_state.item_index > 0 {
                            state.settings_state.item_index -= 1;
                        }
                    }
                }
                vec![]
            }
            KeyCode::Down => {
                match state.settings_state.focus {
                    SettingsFocus::Sections => {
                        // Navigate sections
                        state.settings_state.section = state.settings_state.section.next();
                        state.settings_state.item_index = 0;
                    }
                    SettingsFocus::Content => {
                        // Navigate items within section with bounds check
                        let max_index = match state.settings_state.section {
                            SettingsSection::Server => {
                                // username(0), password(1), sign in(2), then servers(3+)
                                2 + state.available_servers.len()
                            }
                            SettingsSection::Libraries => {
                                state.libraries.len().saturating_sub(1)
                            }
                            SettingsSection::Interface => {
                                crate::ui::theme::ThemeName::all().len().saturating_sub(1)
                            }
                            SettingsSection::Playback => 0,
                            SettingsSection::Data => 1, // Clear Cache, Sign Out
                            SettingsSection::About => 0, // No selectable items
                        };
                        if state.settings_state.item_index < max_index {
                            state.settings_state.item_index += 1;
                        }
                    }
                }
                vec![]
            }
            KeyCode::Enter => {
                if state.settings_state.focus == SettingsFocus::Sections {
                    // Enter on section -> move to content
                    state.settings_state.focus = SettingsFocus::Content;
                    state.settings_state.item_index = 0;
                    vec![]
                } else if state.settings_state.section == SettingsSection::Server {
                    // Handle credential fields vs sign in vs server selection
                    match state.settings_state.item_index {
                        0 => {
                            // Username field - start editing
                            state.settings_state.editing_credential = Some(CredentialField::Username);
                            vec![]
                        }
                        1 => {
                            // Password field - start editing
                            state.settings_state.editing_credential = Some(CredentialField::Password);
                            vec![]
                        }
                        2 => {
                            // Sign In button - authenticate with entered credentials
                            vec![Action::SettingsSignIn]
                        }
                        _ => {
                            // Server selection (index 3+)
                            vec![Action::SettingsSelect]
                        }
                    }
                } else {
                    // Enter on content -> select item
                    vec![Action::SettingsSelect]
                }
            }
            _ => vec![],
        }
    }

    fn select_search_result(&self, state: &mut AppState) -> Vec<Action> {
        if let Some(results) = &state.search_results {
            let section = state.list_state.search_section;
            let idx = state.list_state.search_item_index;

            match section {
                SearchSection::Artists => {
                    if let Some(artist) = results.artists.get(idx).cloned() {
                        // Store artist info for loading albums
                        state.selected_artist_name = artist.title.clone();
                        state.pending_filter_key = Some(artist.rating_key.clone());
                        // Set category directly - LoadArtistAlbums will load artists if needed
                        state.browse_category = BrowseCategory::Artists;
                        state.search_query.clear();
                        state.search_results = None;
                        state.view = View::Browse;
                        state.search_popup_active = false; // Close popup
                        return vec![Action::LoadArtistAlbums];
                    }
                }
                SearchSection::Albums => {
                    if let Some(album) = results.albums.get(idx).cloned() {
                        // Play album - close popup after playing
                        state.search_popup_active = false;
                        return vec![Action::PlayAlbum { rating_key: album.rating_key.clone() }];
                    }
                }
                SearchSection::Tracks => {
                    if let Some(track) = results.tracks.get(idx).cloned() {
                        // Play track - close popup after playing
                        state.search_popup_active = false;
                        return vec![Action::PlayTrack(track)];
                    }
                }
            }
        }
        vec![]
    }

    fn navigate_search_results(&self, state: &mut AppState, delta: i32) {
        if let Some(results) = &state.search_results {
            let section = state.list_state.search_section;
            let idx = state.list_state.search_item_index as i32;

            let section_len = match section {
                SearchSection::Artists => results.artists.len(),
                SearchSection::Albums => results.albums.len(),
                SearchSection::Tracks => results.tracks.len(),
            };

            if section_len == 0 {
                return;
            }

            let new_idx = idx + delta;

            if new_idx < 0 {
                self.next_search_section(state, -1);
                if let Some(results) = &state.search_results {
                    let new_len = match state.list_state.search_section {
                        SearchSection::Artists => results.artists.len(),
                        SearchSection::Albums => results.albums.len(),
                        SearchSection::Tracks => results.tracks.len(),
                    };
                    state.list_state.search_item_index = new_len.saturating_sub(1);
                }
            } else if new_idx >= section_len as i32 {
                self.next_search_section(state, 1);
                state.list_state.search_item_index = 0;
            } else {
                state.list_state.search_item_index = new_idx as usize;
            }
        }
    }

    fn next_search_section(&self, state: &mut AppState, direction: i32) {
        if let Some(results) = &state.search_results {
            let sections: Vec<SearchSection> = [
                (!results.artists.is_empty(), SearchSection::Artists),
                (!results.albums.is_empty(), SearchSection::Albums),
                (!results.tracks.is_empty(), SearchSection::Tracks),
            ]
            .iter()
            .filter(|(has_items, _)| *has_items)
            .map(|(_, section)| *section)
            .collect();

            if sections.is_empty() {
                return;
            }

            let current_idx = sections
                .iter()
                .position(|s| *s == state.list_state.search_section)
                .unwrap_or(0);

            let new_idx = if direction > 0 {
                (current_idx + 1) % sections.len()
            } else if current_idx == 0 {
                sections.len() - 1
            } else {
                current_idx - 1
            };

            state.list_state.search_section = sections[new_idx];
            state.list_state.search_item_index = 0;
        }
    }

    /// Dispatch an action to modify state or trigger side effects.
    async fn dispatch(
        &mut self,
        action: Action,
        state: &mut AppState,
        client: &mut PlexClient,
        audio: &mut AudioPlayer,
    ) -> Result<()> {
        match action {
            Action::Quit => {
                // Save cache to disk before quitting
                if let Some(lib_key) = &state.active_library {
                    use crate::cache::CacheData;

                    let mut cache_data = CacheData::new(lib_key);

                    // Core library data
                    cache_data.artists = state.artists.clone();
                    cache_data.albums = state.albums.clone();
                    cache_data.playlists = state.playlists.clone();

                    // Folder data - extract root folder items only if they belong to this library
                    if let Some(ref folder_state) = state.folder_state {
                        if folder_state.library_key == *lib_key {
                            if let Some(root_col) = folder_state.columns.first() {
                                cache_data.root_folders = root_col.items.clone();
                            }
                        } else {
                            tracing::debug!("Not saving folder_state on quit - belongs to different library (expected {}, got {})",
                                lib_key, folder_state.library_key);
                        }
                    }
                    // Save cached subfolder contents
                    cache_data.folder_contents = state.folder_contents_cache.clone();

                    // Genre/mood/style data
                    cache_data.genres = state.genres.clone();
                    cache_data.artist_genres = state.artist_genres.clone();
                    cache_data.album_genres = state.album_genres.clone();
                    cache_data.moods = state.moods.clone();
                    cache_data.styles = state.styles.clone();

                    // Stations
                    cache_data.stations = state.stations.clone();

                    // Recent content
                    cache_data.recently_added_albums = state.recently_added_albums.clone();
                    cache_data.recently_played_albums = state.recently_played_albums.clone();

                    if let Some(cache) = LibraryCache::new() {
                        if cache.save(&cache_data) {
                            tracing::info!("Cache saved on quit");
                        }
                    }
                }

                state.should_quit = true;
            }
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
                state.artists.clear();
                state.albums.clear();
                state.playlists.clear();
                state.folder_state = None;
                state.folder_contents_cache.clear();
                state.queue.clear();
                state.queue_index = None;

                // Stop playback
                audio.stop();
                state.playback.status = PlayStatus::Stopped;

                state.set_status("Logged out. Restart to sign in again.".to_string());
            }
            Action::AuthSignIn => {
                use super::state::AuthStep;
                // Authenticate with username/password entered in auth screen login form
                let username = state.auth_state.username_input.clone();
                let password = state.auth_state.password_input.clone();

                if username.is_empty() || password.is_empty() {
                    state.auth_state.error_message = Some("Please enter username and password".to_string());
                } else {
                    state.auth_state.step = AuthStep::Authenticating;
                    state.auth_state.error_message = None;
                    let event_tx = self.event_tx.clone();

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
                                        let _ = event_tx.send(Event::AuthServersReady {
                                            token,
                                            username: user.username,
                                            servers,
                                            client_identifier: client_id,
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
                use super::state::AuthStep;
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
                        let event_tx = self.event_tx.clone();

                        // Find working connection URL (tests connectivity)
                        tokio::spawn(async move {
                            if let Some(url) = find_working_connection(&server_clone, &token).await {
                                let _ = event_tx.send(Event::AuthSuccess {
                                    token,
                                    username,
                                    server_url: url,
                                    servers,
                                    client_identifier: client_id,
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
            Action::SetView(view) => {
                state.view = view;
            }
            Action::NextView => {
                // Tab: cycle through nav bar views
                // Order: Artists → Playlists → Genres → Folders → Now Playing → Artists
                if state.view == View::NowPlaying {
                    // From Now Playing, go to Artists
                    state.view = View::Browse;
                    Box::pin(self.dispatch(Action::SetCategory(BrowseCategory::Artists), state, client, audio)).await?;
                } else if state.view == View::Browse {
                    // Cycle through browse categories, then to Now Playing
                    match state.browse_category {
                        BrowseCategory::Artists => {
                            Box::pin(self.dispatch(Action::SetCategory(BrowseCategory::Playlists), state, client, audio)).await?;
                        }
                        BrowseCategory::Playlists => {
                            Box::pin(self.dispatch(Action::SetCategory(BrowseCategory::Genres), state, client, audio)).await?;
                        }
                        BrowseCategory::Genres => {
                            Box::pin(self.dispatch(Action::SetCategory(BrowseCategory::Folders), state, client, audio)).await?;
                        }
                        BrowseCategory::Folders => {
                            state.view = View::NowPlaying;
                        }
                    }
                } else {
                    // From other views (Help, Settings, Search, Similar), go to Browse
                    state.view = View::Browse;
                }
            }
            Action::PrevView => {
                // Shift+Tab: cycle backwards through nav bar views
                // Order: Artists ← Playlists ← Genres ← Folders ← Now Playing ← Artists
                if state.view == View::NowPlaying {
                    // From Now Playing, go to Folders
                    state.view = View::Browse;
                    Box::pin(self.dispatch(Action::SetCategory(BrowseCategory::Folders), state, client, audio)).await?;
                } else if state.view == View::Browse {
                    // Cycle backwards through browse categories, or to Now Playing
                    match state.browse_category {
                        BrowseCategory::Artists => {
                            state.view = View::NowPlaying;
                        }
                        BrowseCategory::Playlists => {
                            Box::pin(self.dispatch(Action::SetCategory(BrowseCategory::Artists), state, client, audio)).await?;
                        }
                        BrowseCategory::Genres => {
                            Box::pin(self.dispatch(Action::SetCategory(BrowseCategory::Playlists), state, client, audio)).await?;
                        }
                        BrowseCategory::Folders => {
                            Box::pin(self.dispatch(Action::SetCategory(BrowseCategory::Genres), state, client, audio)).await?;
                        }
                    }
                } else {
                    // From other views (Help, Settings, Search, Similar), go to Browse
                    state.view = View::Browse;
                }
            }
            Action::NextMode => {
                // Shift+Down: cycle modes within current category
                if state.view == View::NowPlaying {
                    state.now_playing_mode = state.now_playing_mode.next();
                    Box::pin(self.dispatch(Action::RefreshNowPlayingView, state, client, audio)).await?;
                } else if state.view == View::Browse {
                    match state.browse_category {
                        BrowseCategory::Artists => {
                            state.artist_view_mode = state.artist_view_mode.next();
                            Box::pin(self.dispatch(Action::RefreshArtistView, state, client, audio)).await?;
                        }
                        BrowseCategory::Playlists => {
                            state.playlists_mode = state.playlists_mode.next();
                            Box::pin(self.dispatch(Action::RefreshPlaylistsView, state, client, audio)).await?;
                        }
                        BrowseCategory::Genres => {
                            state.genre_content_type = state.genre_content_type.next();
                            Box::pin(self.dispatch(Action::RefreshGenreView, state, client, audio)).await?;
                        }
                        BrowseCategory::Folders => {
                            // Folders has no modes to cycle
                        }
                    }
                }
            }
            Action::PrevMode => {
                // Shift+Up: cycle modes backwards within current category
                if state.view == View::NowPlaying {
                    state.now_playing_mode = state.now_playing_mode.prev();
                    Box::pin(self.dispatch(Action::RefreshNowPlayingView, state, client, audio)).await?;
                } else if state.view == View::Browse {
                    match state.browse_category {
                        BrowseCategory::Artists => {
                            state.artist_view_mode = state.artist_view_mode.prev();
                            Box::pin(self.dispatch(Action::RefreshArtistView, state, client, audio)).await?;
                        }
                        BrowseCategory::Playlists => {
                            state.playlists_mode = state.playlists_mode.prev();
                            Box::pin(self.dispatch(Action::RefreshPlaylistsView, state, client, audio)).await?;
                        }
                        BrowseCategory::Genres => {
                            state.genre_content_type = state.genre_content_type.prev();
                            Box::pin(self.dispatch(Action::RefreshGenreView, state, client, audio)).await?;
                        }
                        BrowseCategory::Folders => {
                            // Folders has no modes to cycle
                        }
                    }
                }
            }
            Action::SetCategory(category) => {
                if state.browse_category != category {
                    state.browse_category = category;
                    state.focus = Focus::Left;
                    // Clear right panel
                    state.right_panel_mode = RightPanelMode::Empty;
                    state.selected_artist_albums.clear();
                    state.selected_album_tracks.clear();

                    // Load category data if needed (and not already loading)
                    match category {
                        BrowseCategory::Artists => {
                            if state.artists.is_empty() && !state.artists_loading {
                                self.load_artists(state, client);
                            } else if !state.artists.is_empty() {
                                // Initialize artist_nav with existing artists
                                let title = state.artist_view_mode.name();
                                let items = super::state::BrowseItem::from_artists(&state.artists);
                                state.artist_nav.reset(title, items);
                            }
                        }
                        BrowseCategory::Playlists => {
                            if state.playlists.is_empty() && !state.playlists_loading {
                                self.load_playlists(state, client);
                            }
                        }
                        BrowseCategory::Genres => {
                            // Load the appropriate content based on current genre content type
                            match state.genre_content_type {
                                super::state::GenreContentType::Genres => {
                                    if state.genres.is_empty() && !state.genres_loading {
                                        Box::pin(self.dispatch(Action::LoadGenres, state, client, audio)).await?;
                                    }
                                }
                                super::state::GenreContentType::ArtistGenres => {
                                    if state.artist_genres.is_empty() && !state.artist_genres_loading {
                                        Box::pin(self.dispatch(Action::LoadArtistGenres, state, client, audio)).await?;
                                    }
                                }
                                super::state::GenreContentType::AlbumGenres => {
                                    if state.album_genres.is_empty() && !state.album_genres_loading {
                                        Box::pin(self.dispatch(Action::LoadAlbumGenres, state, client, audio)).await?;
                                    }
                                }
                                super::state::GenreContentType::Moods => {
                                    if state.moods.is_empty() && !state.moods_loading {
                                        Box::pin(self.dispatch(Action::LoadMoods, state, client, audio)).await?;
                                    }
                                }
                                super::state::GenreContentType::Styles => {
                                    if state.styles.is_empty() && !state.styles_loading {
                                        Box::pin(self.dispatch(Action::LoadStyles, state, client, audio)).await?;
                                    }
                                }
                                super::state::GenreContentType::Stations => {
                                    if state.station_nav.columns.is_empty() && !state.stations_loading {
                                        Box::pin(self.dispatch(Action::LoadStations, state, client, audio)).await?;
                                    }
                                }
                            }
                        }
                        BrowseCategory::Folders => {
                            if state.folder_state.is_none() {
                                Box::pin(self.dispatch(Action::LoadFolderRoot, state, client, audio)).await?;
                            }
                        }
                    }
                }
            }
            Action::ToggleFocus => {
                state.focus = match state.focus {
                    Focus::Left => Focus::Right,
                    Focus::Right => Focus::Left,
                };
            }
            Action::LoadInitialData => {
                tracing::info!("Action::LoadInitialData - loading libraries and artists");

                // Load theme from config
                state.theme = crate::ui::theme::ThemeName::from_config(&self.config.ui.theme);
                crate::ui::theme::set_theme(state.theme);
                tracing::info!("Loaded theme: {}", state.theme.display_name());

                // Load libraries
                match client.get_libraries().await {
                    Ok(libs) => {
                        tracing::info!("Loaded {} total libraries", libs.len());
                        state.libraries = libs.into_iter().filter(|l| l.is_music()).collect();
                        tracing::info!("After filtering: {} music libraries", state.libraries.len());

                        // Try to use saved default library, or fall back to first
                        let lib = if let Some(ref default_key) = self.config.libraries.default_library {
                            state.libraries.iter()
                                .find(|l| &l.key == default_key)
                                .or_else(|| state.libraries.first())
                        } else {
                            state.libraries.first()
                        };

                        if let Some(lib) = lib {
                            tracing::info!("Selected music library: {} (key={})", lib.title, lib.key);
                            let lib_key = lib.key.clone();
                            let lib_title = lib.title.clone();
                            state.active_library = Some(lib_key.clone());

                            // Try to load ALL cached data for instant display
                            if let Some(cache) = LibraryCache::new() {
                                if let Some(cached) = cache.load(&lib_key) {
                                    // Validate cache belongs to this library
                                    if cached.library_key != lib_key {
                                        tracing::warn!("Cache library_key mismatch: expected {}, got {} - ignoring cache",
                                            lib_key, cached.library_key);
                                    } else {
                                        tracing::info!("Loading from cache: {} artists, {} albums, {} folders, {} genres",
                                            cached.artists.len(), cached.albums.len(), cached.root_folders.len(), cached.genres.len());

                                        // Core library data
                                        // IMPORTANT: Always re-sort after loading from cache
                                        // Cache may have been saved with API order, not alphabetical
                                        if !cached.artists.is_empty() {
                                            state.artists = cached.artists;
                                            state.artists.sort_by(|a, b| sort_key(&a.title).cmp(&sort_key(&b.title)));
                                            state.artists_total = state.artists.len() as u32;
                                            // Initialize artist_nav for Miller columns
                                            let items = super::state::BrowseItem::from_artists(&state.artists);
                                            state.artist_nav.reset(state.artist_view_mode.name(), items);
                                        }
                                        if !cached.albums.is_empty() {
                                            state.albums = cached.albums;
                                            state.albums.sort_by(|a, b| sort_key(&a.title).cmp(&sort_key(&b.title)));
                                            state.albums_total = state.albums.len() as u32;
                                        }
                                        if !cached.playlists.is_empty() {
                                            state.playlists = cached.playlists.clone();
                                            // Initialize playlist_nav for Miller columns
                                            let items = super::state::BrowseItem::from_playlists(&state.playlists);
                                            state.playlist_nav.reset("playlists", items);
                                        }

                                        // Folders
                                        if !cached.root_folders.is_empty() {
                                            use crate::services::{FolderColumn, FolderNavigationState};
                                            let root_column = FolderColumn::new(None, lib_title.clone(), cached.root_folders);
                                            state.folder_state = Some(FolderNavigationState {
                                                library_key: lib_key.clone(),
                                                columns: vec![root_column],
                                                focused_column: 0,
                                                loading: false,
                                            });
                                        }
                                        // Load cached subfolder contents with staleness filtering
                                        // Very stale entries (>32 days) are deleted, not refreshed
                                        if !cached.folder_contents.is_empty() {
                                            state.folder_contents_cache = cached.folder_contents;
                                            let removed = CacheService::filter_stale_subfolders_default(&mut state.folder_contents_cache);
                                            if removed > 0 {
                                                tracing::info!("Removed {} very stale subfolder caches on load", removed);
                                                state.cache_dirty = true;  // Save the filtered cache
                                            }
                                            tracing::debug!("Loaded {} cached subfolders", state.folder_contents_cache.len());
                                        }

                                        // Genres, artist genres, album genres, moods, styles
                                        if !cached.genres.is_empty() {
                                            state.genres = cached.genres;
                                            // Initialize genre_nav if this is the default genre type
                                            if state.genre_content_type == super::state::GenreContentType::Genres {
                                                let items = super::state::BrowseItem::from_genres(&state.genres);
                                                state.genre_nav.reset("genres", items);
                                            }
                                        }
                                        if !cached.artist_genres.is_empty() {
                                            state.artist_genres = cached.artist_genres;
                                            if state.genre_content_type == super::state::GenreContentType::ArtistGenres {
                                                let items = super::state::BrowseItem::from_genres(&state.artist_genres);
                                                state.genre_nav.reset("artist genres", items);
                                            }
                                        }
                                        if !cached.album_genres.is_empty() {
                                            state.album_genres = cached.album_genres;
                                            if state.genre_content_type == super::state::GenreContentType::AlbumGenres {
                                                let items = super::state::BrowseItem::from_genres(&state.album_genres);
                                                state.genre_nav.reset("album genres", items);
                                            }
                                        }
                                        if !cached.moods.is_empty() {
                                            state.moods = cached.moods;
                                            if state.genre_content_type == super::state::GenreContentType::Moods {
                                                let items = super::state::BrowseItem::from_genres(&state.moods);
                                                state.genre_nav.reset("moods", items);
                                            }
                                        }
                                        if !cached.styles.is_empty() {
                                            state.styles = cached.styles;
                                            if state.genre_content_type == super::state::GenreContentType::Styles {
                                                let items = super::state::BrowseItem::from_genres(&state.styles);
                                                state.genre_nav.reset("styles", items);
                                            }
                                        }

                                        // Stations - populate both legacy and Miller columns
                                        if !cached.stations.is_empty() {
                                            state.stations = cached.stations.clone();
                                            // Initialize Miller columns with root column
                                            state.station_nav.columns.clear();
                                            state.station_nav.columns.push(super::state::StationColumn::new(
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
                            self.preload_all_library_data(&lib_key, &lib_title, client);
                        } else {
                            tracing::warn!("No music libraries found!");
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to load libraries: {}", e);
                        state.set_error(format!("Failed to load libraries: {}", e));
                    }
                }
            }
            Action::LoadLibraries => {
                tracing::info!("Action::LoadLibraries - fetching libraries");
                match client.get_libraries().await {
                    Ok(libs) => {
                        tracing::info!("Fetched {} libraries", libs.len());
                        state.libraries = libs.into_iter().filter(|l| l.is_music()).collect();
                        if let Some(lib) = state.libraries.first() {
                            state.active_library = Some(lib.key.clone());
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to load libraries: {}", e);
                    }
                }
            }
            Action::LoadArtists => {
                tracing::info!("Action::LoadArtists - active_library={:?}", state.active_library);
                self.load_artists(state, client);
                tracing::info!("LoadArtists complete - loaded {} artists", state.artists.len());
            }
            Action::LoadAlbums => {
                self.load_albums(state, client);
            }
            Action::LoadPlaylists => {
                self.load_playlists(state, client);
            }
            Action::LoadArtistAlbums => {
                // Load albums for selected artist (right panel shows albums)
                // First check for pending filter key (from API search selection)
                let artist_key = if let Some(key) = state.pending_filter_key.take() {
                    // Coming from search - ensure artists are loaded first
                    if state.artists.is_empty() && !state.artists_loading {
                        state.artists_loading = true;
                        if let Some(lib_key) = &state.active_library {
                            match client.get_artists(lib_key).await {
                                Ok(mut artists) => {
                                    artists.sort_by(|a, b| sort_key(&a.title).cmp(&sort_key(&b.title)));
                                    state.artists = artists;
                                }
                                Err(e) => {
                                    tracing::error!("Failed to load artists: {}", e);
                                }
                            }
                        }
                        state.artists_loading = false;
                    }
                    // Find artist in loaded list and set correct index
                    if let Some(pos) = state.artists.iter().position(|a| a.rating_key == key) {
                        state.list_state.artists_index = pos;
                    }
                    key
                } else if let Some(artist) = state.artists.get(state.list_state.artists_index) {
                    state.selected_artist_name = artist.title.clone();
                    artist.rating_key.clone()
                } else {
                    return Ok(());
                };

                state.right_panel_loading = true;
                state.right_panel_mode = RightPanelMode::ArtistAlbums;
                state.selected_artist_albums.clear();
                state.list_state.right_albums_index = 0;

                match client.get_artist_albums(&artist_key).await {
                    Ok(albums) => {
                        state.selected_artist_albums = albums;
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load albums: {}", e));
                    }
                }
                state.right_panel_loading = false;
                state.focus = Focus::Right;

                // Check if we need to auto-select a specific album (e.g., from Similar view)
                if let Some(album_key) = state.pending_album_key.take() {
                    // Find the album in the list (+1 offset for "All Tracks" at index 0)
                    if let Some(album_idx) = state.selected_artist_albums.iter()
                        .position(|a| a.rating_key == album_key)
                    {
                        state.list_state.right_albums_index = album_idx + 1; // +1 for "All Tracks"
                        state.selected_album_title = state.selected_artist_albums[album_idx].title.clone();

                        // Auto-drill into the album to show tracks
                        state.right_panel_loading = true;
                        state.right_panel_mode = RightPanelMode::AlbumTracks;
                        state.selected_album_tracks.clear();
                        state.list_state.tracks_index = 0;

                        match client.get_album_tracks(&album_key).await {
                            Ok(tracks) => {
                                state.selected_album_tracks = tracks;
                            }
                            Err(e) => {
                                state.set_error(format!("Failed to load tracks: {}", e));
                            }
                        }
                        state.right_panel_loading = false;
                    }
                }
            }
            Action::LoadArtistAllTracks => {
                // Load all tracks by the selected artist
                if let Some(artist) = state.artists.get(state.list_state.artists_index) {
                    let artist_key = artist.rating_key.clone();
                    state.selected_album_title = format!("All tracks by {}", artist.title);
                    state.right_panel_loading = true;
                    state.right_panel_mode = RightPanelMode::AlbumTracks;
                    state.selected_album_tracks.clear();
                    state.list_state.tracks_index = 0;

                    match client.get_artist_all_tracks(&artist_key).await {
                        Ok(tracks) => {
                            state.selected_album_tracks = tracks;
                        }
                        Err(e) => {
                            state.set_error(format!("Failed to load tracks: {}", e));
                        }
                    }
                    state.right_panel_loading = false;
                }
            }
            Action::LoadSelectedAlbumTracks => {
                // Load tracks for selected album (drill down from artist albums)
                // Index 0 is "All Tracks", so actual albums start at index 1
                let album_idx = state.list_state.right_albums_index.saturating_sub(1);
                if let Some(album) = state.selected_artist_albums.get(album_idx) {
                    let album_key = album.rating_key.clone();
                    state.selected_album_title = album.title.clone();
                    state.right_panel_loading = true;
                    state.right_panel_mode = RightPanelMode::AlbumTracks;
                    state.selected_album_tracks.clear();
                    state.list_state.tracks_index = 0;

                    match client.get_album_tracks(&album_key).await {
                        Ok(tracks) => {
                            state.selected_album_tracks = tracks;
                        }
                        Err(e) => {
                            state.set_error(format!("Failed to load tracks: {}", e));
                        }
                    }
                    state.right_panel_loading = false;
                }
            }
            Action::LoadAlbumTracks { rating_key } => {
                // Load tracks for a specific album (used by genre albums)
                state.right_panel_loading = true;
                state.right_panel_mode = RightPanelMode::AlbumTracks;
                state.selected_album_tracks.clear();
                state.list_state.tracks_index = 0;

                match client.get_album_tracks(&rating_key).await {
                    Ok(tracks) => {
                        state.selected_album_tracks = tracks;
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load album tracks: {}", e));
                    }
                }
                state.right_panel_loading = false;
            }

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
                        let all_tracks = super::state::BrowseItem::AllTracks {
                            artist_key: artist_key.clone(),
                            artist_name: state.selected_artist_name.clone(),
                        };
                        // Then add albums
                        let mut items = vec![all_tracks];
                        items.extend(super::state::BrowseItem::from_albums(&albums));

                        let title = state.selected_artist_name.clone();
                        let col = super::state::BrowseColumn::new(title, items);
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
                        let items = super::state::BrowseItem::from_tracks(&tracks);
                        let title = state.selected_album_title.clone();
                        // Store full tracks for playback (includes media info)
                        let col = super::state::BrowseColumn::new_with_tracks(title, items, tracks);
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
                        let items = super::state::BrowseItem::from_tracks(&tracks);
                        let title = state.selected_album_title.clone();
                        // Store full tracks for playback (includes media info)
                        let col = super::state::BrowseColumn::new_with_tracks(title, items, tracks);
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
                    let tracks = Self::collect_tracks_from_column(col);
                    if !tracks.is_empty() {
                        state.queue.clear();
                        state.queue.extend(tracks);
                        state.queue_index = Some(track_index);
                        state.playback_mode = super::state::PlaybackMode::Queue;
                        state.view = View::NowPlaying;
                        self.play_current_track(state, client, audio).await;
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
                        super::state::GenreContentType::ArtistGenres => {
                            client.get_artist_genre_albums(lib_key, &genre_key).await
                        }
                        super::state::GenreContentType::AlbumGenres => {
                            client.get_album_genre_albums(lib_key, &genre_key).await
                        }
                        super::state::GenreContentType::Moods => {
                            client.get_mood_albums(lib_key, &genre_key).await
                        }
                        super::state::GenreContentType::Styles => {
                            client.get_style_albums(lib_key, &genre_key).await
                        }
                        _ => {
                            // Default genres use file-based tags
                            client.get_genre_albums(lib_key, &genre_key).await
                        }
                    };

                    match albums_result {
                        Ok(albums) => {
                            let items = super::state::BrowseItem::from_albums(&albums);
                            let col = super::state::BrowseColumn::new("albums", items);
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
                        let items = super::state::BrowseItem::from_tracks(&tracks);
                        // Store full tracks for playback (includes media info)
                        let col = super::state::BrowseColumn::new_with_tracks("tracks", items, tracks);
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
                    let tracks = Self::collect_tracks_from_column(col);
                    if !tracks.is_empty() {
                        state.queue.clear();
                        state.queue.extend(tracks);
                        state.queue_index = Some(track_index);
                        state.playback_mode = super::state::PlaybackMode::Queue;
                        state.view = View::NowPlaying;
                        self.play_current_track(state, client, audio).await;
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
                        let items = super::state::BrowseItem::from_tracks(&tracks);
                        // Store full tracks for playback (includes media info)
                        let col = super::state::BrowseColumn::new_with_tracks("tracks", items, tracks);
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
                        let items = super::state::BrowseItem::from_tracks(&tracks);
                        let title = state.selected_album_title.clone();
                        // Store full tracks for playback (includes media info)
                        let col = super::state::BrowseColumn::new_with_tracks(title, items, tracks);
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
                    let tracks = Self::collect_tracks_from_column(col);
                    if !tracks.is_empty() {
                        state.queue.clear();
                        state.queue.extend(tracks);
                        state.queue_index = Some(track_index);
                        state.playback_mode = super::state::PlaybackMode::Queue;
                        state.view = View::NowPlaying;
                        self.play_current_track(state, client, audio).await;
                    }
                }
            }

            Action::LoadCategoryTracks => {
                // Load tracks directly (for Playlists category)
                // Check for pending filter key first
                let pending_key = state.pending_filter_key.take();

                // Ensure category data is loaded first (synchronously)
                match state.browse_category {
                    BrowseCategory::Artists => {
                        if state.artists.is_empty() && !state.artists_loading {
                            state.artists_loading = true;
                            if let Some(lib_key) = &state.active_library {
                                match client.get_artists(lib_key).await {
                                    Ok(mut artists) => {
                                        artists.sort_by(|a, b| sort_key(&a.title).cmp(&sort_key(&b.title)));
                                        state.artists = artists;
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to load artists: {}", e);
                                    }
                                }
                            }
                            state.artists_loading = false;
                        }
                    }
                    BrowseCategory::Playlists => {
                        if state.playlists.is_empty() {
                            if let Ok(playlists) = client.get_playlists().await {
                                state.playlists = playlists;
                            }
                        }
                    }
                    BrowseCategory::Genres => {
                        // Stations are handled separately via station_nav
                        if state.genre_content_type == super::state::GenreContentType::Stations {
                            return Ok(());
                        }
                        if state.genres.is_empty() {
                            if let Some(lib_key) = &state.active_library {
                                match client.get_genres(lib_key).await {
                                    Ok(genres) => state.genres = genres,
                                    Err(e) => tracing::error!("Failed to load genres: {}", e),
                                }
                            }
                        }
                    }
                    BrowseCategory::Folders => {
                        // Folders don't use this load mechanism
                        return Ok(());
                    }
                }

                // Get rating key AFTER category data is loaded
                let rating_key = pending_key.or_else(|| state.selected_category_key());

                state.right_panel_mode = RightPanelMode::CategoryTracks;
                state.focus = Focus::Right;
                state.list_state.tracks_index = 0;

                if let Some(key) = rating_key {
                    state.right_panel_loading = true;
                    state.selected_album_tracks.clear();

                    let result = match state.browse_category {
                        BrowseCategory::Artists => client.get_artist_all_tracks(&key).await,
                        BrowseCategory::Playlists => {
                            // Check playlist mode - RecentlyAdded contains albums, not playlists
                            match state.playlists_mode {
                                crate::app::state::PlaylistsMode::RecentlyAdded => {
                                    // This is an album, not a playlist
                                    client.get_album_tracks(&key).await
                                }
                                _ => {
                                    // Check if this is a special smart playlist like "Recently Played"
                                    // that returns 500 errors - handle it specially
                                    let playlist_title = state.selected_category_title()
                                        .unwrap_or_default().to_lowercase();

                                    if playlist_title.contains("recently played") {
                                        // Use the working lastViewedAt approach instead
                                        if let Some(lib_key) = &state.active_library {
                                            match client.get_recently_played_albums(lib_key, 50).await {
                                                Ok(albums) => {
                                                    state.set_status("Showing recently played albums".to_string());
                                                    // Switch to showing albums instead of tracks
                                                    state.right_panel_mode = RightPanelMode::CategoryAlbums;
                                                    state.genre_albums = albums;
                                                    state.genre_albums_index = 0;
                                                    Ok(Vec::new())
                                                }
                                                Err(e) => Err(e)
                                            }
                                        } else {
                                            Err(crate::api::ApiError::NoServerSelected)
                                        }
                                    } else if playlist_title.contains("recently added") {
                                        // Use the recentlyAdded endpoint instead of smart playlist
                                        if let Some(lib_key) = &state.active_library {
                                            match client.get_recently_added_albums(lib_key, 50).await {
                                                Ok(albums) => {
                                                    state.set_status("Showing recently added albums".to_string());
                                                    state.right_panel_mode = RightPanelMode::CategoryAlbums;
                                                    state.genre_albums = albums;
                                                    state.genre_albums_index = 0;
                                                    Ok(Vec::new())
                                                }
                                                Err(e) => Err(e)
                                            }
                                        } else {
                                            Err(crate::api::ApiError::NoServerSelected)
                                        }
                                    } else {
                                        // Regular playlists or recent playlists
                                        client.get_playlist_tracks(&key).await
                                    }
                                }
                            }
                        }
                        BrowseCategory::Genres => {
                            if let Some(lib_key) = &state.active_library {
                                client.get_genre_tracks(lib_key, &key).await
                            } else {
                                Err(crate::api::ApiError::NoServerSelected)
                            }
                        }
                        BrowseCategory::Folders => return Ok(()), // Folders handled separately
                    };

                    match result {
                        Ok(tracks) => {
                            state.selected_album_tracks = tracks;
                            if let Some(first) = state.selected_album_tracks.first() {
                                if state.selected_album_title.is_empty() {
                                    state.selected_album_title = first.album_name().to_string();
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to load tracks for key {}: {}", key, e);
                            // Clean up error message - don't show raw HTML
                            let error_str = e.to_string();
                            let clean_error = if error_str.contains("<html>") || error_str.contains("500") {
                                "This playlist cannot be loaded (server error)".to_string()
                            } else {
                                format!("Failed to load tracks: {}", e)
                            };
                            state.set_error(clean_error);
                        }
                    }
                    state.right_panel_loading = false;
                } else {
                    // No key available
                    state.right_panel_loading = false;
                    state.selected_album_tracks.clear();
                }
            }
            Action::GoBackInRightPanel => {
                // Go from tracks back to albums view (for artist drill-down)
                if state.right_panel_mode == RightPanelMode::AlbumTracks {
                    state.right_panel_mode = RightPanelMode::ArtistAlbums;
                    state.selected_album_tracks.clear();
                }
            }
            Action::LoadSimilarAlbums { rating_key, title } => {
                state.similar_source_title = title;
                state.similar_loading = true;
                state.similar_albums.clear();
                state.list_state.similar_index = 0;
                state.view = View::Similar;

                match client.get_similar_albums(&rating_key, 50).await {
                    Ok(albums) => {
                        let _ = self.event_tx.send(Event::SimilarAlbumsLoaded(albums)).await;
                    }
                    Err(e) => {
                        state.similar_loading = false;
                        state.set_error(format!("Failed to load similar albums: {}", e));
                    }
                }
            }
            Action::LoadSimilarTracks { rating_key, title } => {
                state.similar_source_title = title;
                state.similar_loading = true;
                state.similar_tracks.clear();
                state.list_state.similar_index = 0;
                state.view = View::Similar;

                match client.get_similar_tracks(&rating_key, 50).await {
                    Ok(tracks) => {
                        let _ = self.event_tx.send(Event::SimilarTracksLoaded(tracks)).await;
                    }
                    Err(e) => {
                        state.similar_loading = false;
                        state.set_error(format!("Failed to load similar tracks: {}", e));
                    }
                }
            }
            Action::ListUp => {
                self.adjust_list_index(state, -1);
            }
            Action::ListDown => {
                self.adjust_list_index(state, 1);
                // Lazy load more if needed
                self.maybe_load_more(state, client).await;
            }
            Action::ListPageUp => {
                self.adjust_list_index(state, -10);
            }
            Action::ListPageDown => {
                self.adjust_list_index(state, 10);
                self.maybe_load_more(state, client).await;
            }
            Action::ListTop => {
                self.set_list_index(state, 0);
            }
            Action::ListBottom => {
                self.set_list_index(state, isize::MAX);
            }
            Action::PlayTrack(track) => {
                self.play_track(track, state, client, audio).await;
            }
            Action::PlayTrackFromCategory(idx) => {
                if idx < state.selected_album_tracks.len() {
                    // Report stop for currently playing track before switching
                    // continuing=true because we're switching to another track
                    if let Some(current) = state.current_track().cloned() {
                        Self::report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
                    }

                    // Generate new session ID for this playback context
                    state.plex_session_id = Some(Self::generate_plex_session_id());

                    // Clear radio state if switching from radio mode
                    if state.playback_mode == PlaybackMode::Radio {
                        state.radio.clear();
                    }
                    // Queue all tracks from current position
                    state.queue = state.selected_album_tracks[idx..].to_vec();
                    state.queue_index = Some(0);
                    state.queue_original.clear();
                    state.queue_sort_mode = super::state::QueueSortMode::QueueOrder;
                    state.playback_mode = PlaybackMode::Queue;
                    self.play_current_track(state, client, audio).await;
                }
            }
            Action::PlayAlbum { rating_key } => {
                // Load album tracks and play them
                match client.get_album_tracks(&rating_key).await {
                    Ok(tracks) => {
                        if !tracks.is_empty() {
                            // Report stop for currently playing track before switching
                            // continuing=true because we're switching to another album
                            if let Some(current) = state.current_track().cloned() {
                                Self::report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
                            }

                            // Generate new session ID for this playback context
                            state.plex_session_id = Some(Self::generate_plex_session_id());

                            // Clear radio state if switching from radio mode
                            if state.playback_mode == PlaybackMode::Radio {
                                state.radio.clear();
                            }
                            state.queue = tracks;
                            state.queue_index = Some(0);
                            state.queue_original.clear();
                            state.queue_sort_mode = super::state::QueueSortMode::QueueOrder;
                            state.playback_mode = PlaybackMode::Queue;
                            self.play_current_track(state, client, audio).await;
                        }
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load album: {}", e));
                    }
                }
            }
            Action::EnqueueAlbum { rating_key, title } => {
                // Load album tracks and add to queue
                match client.get_album_tracks(&rating_key).await {
                    Ok(tracks) => {
                        if !tracks.is_empty() {
                            // If radio is playing, convert to queue mode first
                            if state.playback_mode == PlaybackMode::Radio {
                                state.queue = state.radio.tracks.clone();
                                state.queue_index = state.radio.track_index;
                                state.playback_mode = PlaybackMode::Queue;
                                state.radio.clear();
                            }

                            // Add tracks to queue, respecting 500 track limit
                            const MAX_QUEUE_SIZE: usize = 500;
                            let mut added = 0;
                            for track in tracks {
                                if state.queue.len() < MAX_QUEUE_SIZE {
                                    state.queue.push(track);
                                    added += 1;
                                }
                            }
                            state.set_status(format!("Added {} tracks from \"{}\" to queue", added, title));
                        }
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load album: {}", e));
                    }
                }
            }
            Action::TogglePlayPause => {
                match state.playback.status {
                    PlayStatus::Playing => {
                        audio.pause();
                        state.playback.status = PlayStatus::Paused;
                    }
                    PlayStatus::Paused => {
                        audio.resume();
                        state.playback.status = PlayStatus::Playing;
                    }
                    PlayStatus::Stopped => {
                        if state.queue_index.is_some() {
                            self.play_current_track(state, client, audio).await;
                        }
                    }
                    _ => {}
                }
            }
            Action::Pause => {
                // Report stop to Plex before pausing
                // continuing=false because playback is pausing (not moving to next track)
                if let Some(track) = state.current_track().cloned() {
                    Self::report_playback_stop_to_plex(&track, state.playback.position_ms, false, state.plex_session_id.clone(), client);
                }
                audio.pause();
                state.playback.status = PlayStatus::Paused;
            }
            Action::Play => {
                audio.resume();
                state.playback.status = PlayStatus::Playing;
            }
            Action::Stop => {
                // Report stop to Plex before stopping
                // continuing=false because playback is truly stopping
                if let Some(track) = state.current_track().cloned() {
                    Self::report_playback_stop_to_plex(&track, state.playback.position_ms, false, state.plex_session_id.clone(), client);
                }
                audio.stop();
                state.playback.status = PlayStatus::Stopped;
                state.playback.position_ms = 0;
                // Clear session ID when playback truly stops
                state.plex_session_id = None;
            }
            Action::Next => {
                // Report stop for current track before switching
                // continuing=true because we're moving to the next track
                if let Some(track) = state.current_track().cloned() {
                    Self::report_playback_stop_to_plex(&track, state.playback.position_ms, true, state.plex_session_id.clone(), client);
                }

                match state.playback_mode {
                    PlaybackMode::Radio => {
                        // Radio mode: use radio.tracks and auto-fetch more
                        if let Some(idx) = state.radio.track_index {
                            if idx + 1 < state.radio.tracks.len() {
                                state.radio.track_index = Some(idx + 1);
                                self.play_current_track(state, client, audio).await;

                                // Auto-fetch more tracks when running low
                                let remaining = state.radio.tracks.len().saturating_sub(idx + 1);
                                if remaining < 5 && !state.radio.fetching {
                                    self.fetch_more_radio_tracks(state, client).await;
                                }
                            } else if !state.radio.fetching {
                                // At end, try to fetch more
                                self.fetch_more_radio_tracks(state, client).await;
                                // After fetching, try to advance
                                if idx + 1 < state.radio.tracks.len() {
                                    state.radio.track_index = Some(idx + 1);
                                    self.play_current_track(state, client, audio).await;
                                }
                            }
                        }
                    }
                    PlaybackMode::Queue | PlaybackMode::None => {
                        // Queue mode: use state.queue
                        if let Some(idx) = state.queue_index {
                            if idx + 1 < state.queue.len() {
                                state.queue_index = Some(idx + 1);
                                self.play_current_track(state, client, audio).await;
                            } else if state.playback.repeat_mode == super::state::RepeatMode::All {
                                state.queue_index = Some(0);
                                self.play_current_track(state, client, audio).await;
                            }
                            // else: stop at end of queue
                        }
                    }
                }
            }
            Action::Previous => {
                // If more than 3 seconds in, restart current track (no stop report needed)
                if state.playback.position_ms > 3000 {
                    state.playback.position_ms = 0;
                    self.play_current_track(state, client, audio).await;
                } else {
                    // Report stop for current track before going to previous
                    // continuing=true because we're moving to the previous track
                    if let Some(track) = state.current_track().cloned() {
                        Self::report_playback_stop_to_plex(&track, state.playback.position_ms, true, state.plex_session_id.clone(), client);
                    }

                    // Go to previous track based on playback mode
                    match state.playback_mode {
                        PlaybackMode::Radio => {
                            if let Some(idx) = state.radio.track_index {
                                if idx > 0 {
                                    state.radio.track_index = Some(idx - 1);
                                    self.play_current_track(state, client, audio).await;
                                }
                            }
                        }
                        PlaybackMode::Queue | PlaybackMode::None => {
                            if let Some(idx) = state.queue_index {
                                if idx > 0 {
                                    state.queue_index = Some(idx - 1);
                                    self.play_current_track(state, client, audio).await;
                                }
                            }
                        }
                    }
                }
            }
            Action::VolumeUp => {
                state.playback.volume = (state.playback.volume + 0.05).min(1.0);
                audio.set_volume(state.playback.volume);
            }
            Action::VolumeDown => {
                state.playback.volume = (state.playback.volume - 0.05).max(0.0);
                audio.set_volume(state.playback.volume);
            }
            Action::ToggleMute => {
                state.playback.muted = !state.playback.muted;
                audio.set_volume(if state.playback.muted { 0.0 } else { state.playback.volume });
            }
            Action::Seek(position_ms) => {
                // Seek to absolute position
                let position = std::time::Duration::from_millis(position_ms);
                if audio.try_seek(position) {
                    state.playback.position_ms = position_ms;
                }
            }
            Action::SeekRelative(delta_ms) => {
                // Seek relative to current position
                let current = state.playback.position_ms as i64;
                let duration = state.playback.duration_ms as i64;
                let new_pos = (current + delta_ms).clamp(0, duration) as u64;
                let position = std::time::Duration::from_millis(new_pos);
                if audio.try_seek(position) {
                    state.playback.position_ms = new_pos;
                }
            }
            Action::ToggleShuffle => {
                state.playback.shuffle = !state.playback.shuffle;
            }
            Action::CycleRepeat => {
                state.playback.repeat_mode = state.playback.repeat_mode.next();
            }
            Action::ExecuteSearch => {
                if state.search_query.len() >= 2 {
                    // Increment version to invalidate any pending searches
                    state.global_search_version = state.global_search_version.wrapping_add(1);
                    let version = state.global_search_version;
                    state.search_loading = true;

                    // Spawn search in background with debounce
                    let event_tx = self.event_tx.clone();
                    let query = state.search_query.clone();
                    let search_client = client.clone();

                    tokio::spawn(async move {
                        // Debounce: wait before searching
                        tokio::time::sleep(std::time::Duration::from_millis(350)).await;

                        // Execute search - stale results will be rejected by version check
                        match search_client.search(&query).await {
                            Ok(results) => {
                                let _ = event_tx.send(Event::GlobalSearchCompleted {
                                    version,
                                    results,
                                }).await;
                            }
                            Err(_) => {
                                let _ = event_tx.send(Event::GlobalSearchCompleted {
                                    version,
                                    results: Default::default(),
                                }).await;
                            }
                        }
                    });
                } else {
                    // Clear results for short queries
                    state.search_results = None;
                    state.search_loading = false;
                }
            }
            Action::ClearSearch => {
                state.search_query.clear();
                state.search_results = None;
            }
            Action::ExecuteFilterSearch => {
                if state.search_query.len() >= 2 {
                    // Increment version to invalidate any pending searches
                    state.filter_search_version = state.filter_search_version.wrapping_add(1);
                    let version = state.filter_search_version;
                    state.filter_loading = true;

                    // Spawn search in background with debounce
                    let event_tx = self.event_tx.clone();
                    let query = state.search_query.clone();
                    let search_client = client.clone();

                    tokio::spawn(async move {
                        // Debounce: wait before searching
                        tokio::time::sleep(std::time::Duration::from_millis(350)).await;

                        // Execute search - stale results will be rejected by version check
                        match search_client.search(&query).await {
                            Ok(results) => {
                                let _ = event_tx.send(Event::FilterSearchCompleted {
                                    version,
                                    results
                                }).await;
                            }
                            Err(_) => {
                                let _ = event_tx.send(Event::FilterSearchCompleted {
                                    version,
                                    results: Default::default()
                                }).await;
                            }
                        }
                    });
                } else {
                    // Clear filter results for short queries (use local filtering)
                    state.filter_results = None;
                    state.filter_loading = false;
                }
            }
            Action::SelectFilterResult => {
                let follow_up_actions = self.select_filter_result(state);
                for action in follow_up_actions {
                    // Recursively dispatch follow-up actions
                    Box::pin(self.dispatch(action, state, client, audio)).await?;
                }
            }
            Action::ClearQueue => {
                // Clear the appropriate queue based on playback mode
                match state.playback_mode {
                    PlaybackMode::Radio => {
                        state.radio.clear();
                    }
                    PlaybackMode::Queue | PlaybackMode::None => {
                        state.queue.clear();
                        state.queue_index = None;
                    }
                }
                audio.stop();
                state.playback.status = PlayStatus::Stopped;
            }
            Action::RemoveFromQueue(idx) => {
                if idx < state.queue.len() {
                    state.queue.remove(idx);
                    // Adjust queue_index if needed
                    if let Some(current) = state.queue_index {
                        if idx < current {
                            state.queue_index = Some(current - 1);
                        } else if idx == current && current >= state.queue.len() {
                            state.queue_index = if state.queue.is_empty() {
                                None
                            } else {
                                Some(state.queue.len() - 1)
                            };
                        }
                    }
                    // Adjust list selection
                    if state.list_state.queue_index >= state.queue.len() && !state.queue.is_empty() {
                        state.list_state.queue_index = state.queue.len() - 1;
                    }
                }
            }
            Action::JumpToQueueIndex(idx) => {
                // Jump to and play a specific track in the queue
                if idx < state.queue.len() {
                    state.queue_index = Some(idx);
                    state.list_state.queue_index = state.play_history.len() + idx;
                    if let Some(track) = state.queue.get(idx).cloned() {
                        Box::pin(self.dispatch(Action::PlayTrack(track), state, client, audio)).await?;
                    }
                }
            }
            Action::PlayRecentlyPlayedAlbum(idx) => {
                // Play album from recently played list
                if let Some(album) = state.recently_played_albums.get(idx).cloned() {
                    let rating_key = album.rating_key.clone();
                    Box::pin(self.dispatch(Action::PlayAlbum { rating_key }, state, client, audio)).await?;
                }
            }
            Action::ShowError(msg) => {
                state.set_error(msg);
            }
            Action::ClearError => {
                state.clear_error();
            }
            Action::SetStatus(msg) => {
                state.set_status(msg);
            }
            Action::ClearStatus => {
                state.clear_status();
            }
            Action::RefreshCategory(category) => {
                if let Some(lib_key) = &state.active_library {
                    let lib_key = lib_key.clone();
                    self.spawn_category_refresh(category, &lib_key, state, client);
                }
            }
            Action::CycleTheme => {
                state.theme = state.theme.next();
                crate::ui::theme::set_theme(state.theme);
                state.set_status(format!("Theme: {}", state.theme.display_name()));

                // Persist theme to config
                self.config.ui.theme = state.theme.config_name().to_string();
                if let Err(e) = crate::config::save_config(&self.config) {
                    tracing::warn!("Failed to save theme preference: {}", e);
                }
            }
            Action::OpenSettings => {
                state.view = View::Settings;
                state.settings_state.section = SettingsSection::Server;
                state.settings_state.item_index = 0;

                // Get username from connection state first (most reliable), then StoredAuth, then config
                state.settings_state.username_input = match &state.connection {
                    ConnectionState::Connected { username } => username.clone(),
                    _ => PlexAuth::load_token()
                        .and_then(|s| s.username)
                        .or_else(|| self.config.plex.username.clone())
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
                        let event_tx = self.event_tx.clone();
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
                let mut config = self.config.clone();
                config.plex.username = if state.settings_state.username_input.is_empty() {
                    None
                } else {
                    Some(state.settings_state.username_input.clone())
                };
                if let Err(e) = crate::config::save_config(&config) {
                    state.set_error(format!("Failed to save: {}", e));
                } else {
                    state.set_status("Username saved.".to_string());
                }
            }
            Action::SettingsSelect => {
                match state.settings_state.section {
                    SettingsSection::Server => {
                        // Select server (index adjusted: 0=username, 1=password, 2=sign in, 3+=servers)
                        let server_index = state.settings_state.item_index.saturating_sub(3);
                        if let Some(server) = state.available_servers.get(server_index) {
                            let server_id = server.client_identifier.clone();
                            tracing::info!("Selected server: {}", server.name);
                            Box::pin(self.dispatch(Action::SelectServer(server_id), state, client, audio)).await?;
                        }
                    }
                    SettingsSection::Libraries => {
                        // Activate selected library
                        if let Some(lib) = state.libraries.get(state.settings_state.item_index) {
                            let lib_key = lib.key.clone();
                            Box::pin(self.dispatch(Action::SelectLibrary(lib_key), state, client, audio)).await?;
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
                            self.config.ui.theme = state.theme.config_name().to_string();
                            if let Err(e) = crate::config::save_config(&self.config) {
                                tracing::warn!("Failed to save theme preference: {}", e);
                            }
                        }
                    }
                    SettingsSection::Data => {
                        // Handle data management options
                        match state.settings_state.item_index {
                            0 => {
                                // Clear Cache
                                Box::pin(self.dispatch(Action::ClearCache, state, client, audio)).await?;
                            }
                            1 => {
                                // Sign Out
                                Box::pin(self.dispatch(Action::Logout, state, client, audio)).await?;
                            }
                            _ => {}
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
                    let event_tx = self.event_tx.clone();
                    let server_url = self.config.plex.server_url.clone();

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
                                            find_working_connection_from_servers(&servers, &token).await
                                        } else {
                                            Some(server_url)
                                        };

                                        if let Some(url) = final_url {
                                            let _ = event_tx.send(Event::AuthSuccess {
                                                token,
                                                username: user.username,
                                                server_url: url,
                                                servers,
                                                client_identifier: client_id,
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
                    let event_tx = self.event_tx.clone();
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
                        let event_tx = self.event_tx.clone();

                        // Find working connection URL (tests connectivity)
                        tokio::spawn(async move {
                            if let Some(url) = find_working_connection(&server_clone, &token).await {
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
                    // Note: folder_contents_cache is NOT cleared here - it will be replaced
                    // with the new library's cached subfolders. Each library has its own
                    // cache file, so there's no cross-contamination.
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
                    state.list_state.reset();

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
                                    state.artists.sort_by(|a, b| sort_key(&a.title).cmp(&sort_key(&b.title)));
                                    state.artists_total = state.artists.len() as u32;
                                }
                                if !cached.albums.is_empty() {
                                    state.albums = cached.albums;
                                    state.albums.sort_by(|a, b| sort_key(&a.title).cmp(&sort_key(&b.title)));
                                    state.albums_total = state.albums.len() as u32;
                                }
                                if !cached.playlists.is_empty() {
                                    state.playlists = cached.playlists;
                                }

                                // Folders
                                if !cached.root_folders.is_empty() {
                                    use crate::services::{FolderColumn, FolderNavigationState};
                                    let root_column = FolderColumn::new(None, lib_name.clone(), cached.root_folders);
                                    state.folder_state = Some(FolderNavigationState {
                                        library_key: lib_key.clone(),
                                        columns: vec![root_column],
                                        focused_column: 0,
                                        loading: false,
                                    });
                                }
                                // Load cached subfolder contents with staleness filtering
                                // Very stale entries (>32 days) are deleted, not refreshed
                                if !cached.folder_contents.is_empty() {
                                    state.folder_contents_cache = cached.folder_contents;
                                    let removed = CacheService::filter_stale_subfolders_default(&mut state.folder_contents_cache);
                                    if removed > 0 {
                                        tracing::info!("Library switch: removed {} very stale subfolder caches", removed);
                                        state.cache_dirty = true;  // Save the filtered cache
                                    }
                                    tracing::debug!("Library switch: loaded {} cached subfolders", state.folder_contents_cache.len());
                                } else {
                                    // New library has no subfolder cache - clear any old data
                                    state.folder_contents_cache.clear();
                                }

                                // Genres, artist genres, album genres, moods, styles
                                if !cached.genres.is_empty() {
                                    state.genres = cached.genres;
                                }
                                if !cached.artist_genres.is_empty() {
                                    state.artist_genres = cached.artist_genres;
                                }
                                if !cached.album_genres.is_empty() {
                                    state.album_genres = cached.album_genres;
                                }
                                if !cached.moods.is_empty() {
                                    state.moods = cached.moods;
                                }
                                if !cached.styles.is_empty() {
                                    state.styles = cached.styles;
                                }

                                // Stations - populate both legacy and Miller columns
                                if !cached.stations.is_empty() {
                                    state.stations = cached.stations.clone();
                                    // Initialize Miller columns with root column
                                    state.station_nav.columns.clear();
                                    state.station_nav.columns.push(super::state::StationColumn::new(
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
                    self.preload_all_library_data(&lib_key, &lib_name, client);

                    state.set_status(format!("Switched to {}", lib_name));

                    // Auto-save the default library
                    Box::pin(self.dispatch(Action::SaveSettings, state, client, audio)).await?;
                }
            }
            Action::SaveSettings => {
                // Build updated config from current state
                let mut updated_config = self.config.clone();
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
                                self.preload_data(PreloadType::Artists, &lib_key, client);
                                self.preload_data(PreloadType::Albums, &lib_key, client);
                                self.preload_data(PreloadType::Playlists, &lib_key, client);
                                self.preload_data(PreloadType::Folders { lib_title: lib_name }, &lib_key, client);
                                self.preload_data(PreloadType::Genres, &lib_key, client);
                                self.preload_data(PreloadType::ArtistGenres, &lib_key, client);
                                self.preload_data(PreloadType::AlbumGenres, &lib_key, client);
                                self.preload_data(PreloadType::Moods, &lib_key, client);
                                self.preload_data(PreloadType::Styles, &lib_key, client);
                                self.preload_data(PreloadType::Stations, &lib_key, client);
                                self.preload_data(PreloadType::RecentlyAdded, &lib_key, client);
                                self.preload_data(PreloadType::RecentlyPlayed, &lib_key, client);
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
            Action::LoadArtwork => {
                // Get thumb path from current track (clone to avoid borrow)
                let thumb_path = state.current_track()
                    .and_then(|t| t.best_thumb().map(|s| s.to_string()));

                if let Some(thumb_path) = thumb_path {
                    // Check if we need to load new artwork
                    if state.artwork_thumb.as_deref() != Some(&thumb_path) {
                        state.artwork_loading = true;
                        match client.fetch_artwork(&thumb_path, 300).await {
                            Ok(data) => {
                                state.artwork_thumb = Some(thumb_path);
                                state.artwork_data = Some(data);
                            }
                            Err(e) => {
                                tracing::warn!("Failed to load artwork: {}", e);
                                state.artwork_thumb = None;
                                state.artwork_data = None;
                            }
                        }
                        state.artwork_loading = false;
                    }
                } else {
                    // No artwork available or no current track
                    state.artwork_thumb = None;
                    state.artwork_data = None;
                }
            }
            Action::LoadWaveform => {
                // Only generate waveform if we have a track and don't already have data
                if let Some(track) = state.current_track().cloned() {
                    let needs_generation = state.waveform.data.is_none()
                        && !state.waveform.generating
                        && state.waveform.track_key.as_ref() == Some(&track.rating_key);

                    if needs_generation {
                        state.waveform.generating = true;
                        let track_key = track.rating_key.clone();
                        let duration_ms = track.duration_ms();
                        let event_tx = self.event_tx.clone();

                        if let Ok(stream_url) = client.get_stream_url(&track) {
                            let token = client.token().map(|s| s.to_string());

                            tokio::spawn(async move {
                                // Check cache first
                                let cache_dir = dirs::cache_dir()
                                    .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                                    .join("textamp")
                                    .join("waveforms");
                                let cache = crate::services::WaveformCache::new(cache_dir);

                                if let Some(data) = cache.load(&track_key) {
                                    // Cache hit
                                    let _ = event_tx.send(Event::WaveformCacheHit {
                                        track_key,
                                        data,
                                    }).await;
                                    return;
                                }

                                // Cache miss - download and generate
                                let http_client = reqwest::Client::new();
                                let mut request = http_client.get(&stream_url);
                                if let Some(ref token) = token {
                                    request = request.header("X-Plex-Token", token);
                                }

                                match request.send().await {
                                    Ok(response) => {
                                        match response.bytes().await {
                                            Ok(audio_data) => {
                                                // Generate waveform
                                                match crate::services::generate_waveform(
                                                    track_key.clone(),
                                                    duration_ms,
                                                    audio_data.to_vec(),
                                                ) {
                                                    Ok(data) => {
                                                        // Save to cache
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
            }
            Action::LoadFolderRoot => {
                use crate::services::FolderColumn;

                if let Some(lib_key) = &state.active_library {
                    match client.get_library_folders(lib_key).await {
                        Ok(response) => {
                            let items = FolderService::from_response(&response);
                            let lib_title = state.libraries.iter()
                                .find(|l| &l.key == lib_key)
                                .map(|l| l.title.clone())
                                .unwrap_or_else(|| "Root".to_string());

                            let root_column = FolderColumn::new(None, lib_title, items);
                            state.folder_state = Some(FolderNavigationState {
                                library_key: lib_key.clone(),
                                columns: vec![root_column],
                                focused_column: 0,
                                loading: false,
                            });
                        }
                        Err(e) => {
                            state.set_error(format!("Failed to load folders: {}", e));
                        }
                    }
                }
            }
            Action::NavigateIntoFolder(folder_key) => {
                use crate::services::FolderColumn;

                // Check cache first for instant navigation
                if let Some(cached_folder) = state.folder_contents_cache.get(&folder_key) {
                    tracing::debug!("Folder cache hit: {} ({} items)", folder_key, cached_folder.items.len());
                    // Extract folder title from the key if possible, or use a generic title
                    let folder_title = folder_key.split('/').last().unwrap_or("Folder").to_string();
                    if let Some(ref mut folder_state) = state.folder_state {
                        let new_column = FolderColumn::new(Some(folder_key), folder_title, cached_folder.items.clone());
                        folder_state.push_column(new_column);
                    }
                } else {
                    // Not in cache - fetch from API
                    match client.get_folder_contents(&folder_key).await {
                        Ok(response) => {
                            let items = FolderService::from_response(&response);
                            let folder_title = response.media_container.title2.clone().unwrap_or_default();

                            // Store in cache with timestamp for future use
                            state.folder_contents_cache.insert(folder_key.clone(), CachedFolder::new(items.clone()));
                            state.cache_dirty = true;
                            tracing::debug!("Cached folder: {} ({} items)", folder_key, items.len());

                            if let Some(ref mut folder_state) = state.folder_state {
                                let new_column = FolderColumn::new(Some(folder_key), folder_title, items);
                                folder_state.push_column(new_column);
                            }
                        }
                        Err(e) => {
                            state.set_error(format!("Failed to load folder: {}", e));
                        }
                    }
                }
            }
            Action::NavigateUpFolder => {
                // In column view, going up just moves focus left
                if let Some(ref mut folder_state) = state.folder_state {
                    folder_state.focus_left();
                }
            }
            Action::RefreshSubfolder(folder_key) => {
                // Manual refresh of a specific subfolder (F5 when focused on subfolder)
                // This is the ONLY way subfolder caches get manually refreshed.

                match client.get_folder_contents(&folder_key).await {
                    Ok(response) => {
                        let items = FolderService::from_response(&response);
                        let folder_title = response.media_container.title2.clone().unwrap_or_default();

                        // Update the cache with fresh data and new timestamp
                        state.folder_contents_cache.insert(folder_key.clone(), CachedFolder::new(items.clone()));
                        state.cache_dirty = true;
                        tracing::info!("Refreshed subfolder cache: {} ({} items)", folder_key, items.len());

                        // Update the currently displayed column if it matches
                        if let Some(ref mut folder_state) = state.folder_state {
                            // Find the column that corresponds to this folder key and update it
                            for col in folder_state.columns.iter_mut() {
                                if col.key.as_ref() == Some(&folder_key) {
                                    let old_selected = col.selected_index;
                                    col.items = items.clone();
                                    // Preserve selection position if possible
                                    col.selected_index = old_selected.min(col.items.len().saturating_sub(1));
                                    col.title = folder_title.clone();
                                    break;
                                }
                            }
                        }

                        state.set_status("Folder refreshed".to_string());
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to refresh folder: {}", e));
                    }
                }
            }
            Action::PlayFolderTracks => {
                // Play tracks in the focused column's folder, starting from selected item
                if let Some(ref folder_state) = state.folder_state {
                    // Get the folder key and selected item from the focused column
                    let selected_key = folder_state.selected_item().map(|item| item.key.clone());
                    let selected_index = folder_state.focused().map(|col| col.selected_index).unwrap_or(0);

                    if let Some(col) = folder_state.focused() {
                        if let Some(ref folder_key) = col.key {
                            match client.get_folder_tracks(folder_key).await {
                                Ok(tracks) => {
                                    // Find the index of the selected track
                                    let start_idx = if let Some(ref sel_key) = selected_key {
                                        tracks.iter().position(|t| {
                                            // Match by rating_key or by position
                                            t.rating_key == *sel_key || t.key == *sel_key
                                        }).unwrap_or(selected_index.min(tracks.len().saturating_sub(1)))
                                    } else {
                                        0
                                    };

                                    state.queue = tracks;
                                    state.queue_index = Some(start_idx);
                                    state.playback_mode = PlaybackMode::Queue;
                                    if let Some(track) = state.queue.get(start_idx).cloned() {
                                        self.play_track(track, state, client, audio).await;
                                    }
                                }
                                Err(e) => {
                                    state.set_error(format!("Failed to load folder tracks: {}", e));
                                }
                            }
                        } else {
                            // Root folder - get all tracks from library root
                            if let Some(lib_key) = &state.active_library {
                                match client.get_library_root_tracks(lib_key).await {
                                    Ok(tracks) => {
                                        if !tracks.is_empty() {
                                            state.queue = tracks;
                                            state.queue_index = Some(0);
                                            state.playback_mode = PlaybackMode::Queue;
                                            if let Some(track) = state.queue.first().cloned() {
                                                self.play_track(track, state, client, audio).await;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        state.set_error(format!("Failed to load tracks: {}", e));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Action::StartTrackRadio { track_key, title } => {
                use crate::app::state::RadioMode;
                use rand::seq::SliceRandom;

                // Report stop for currently playing track before starting radio
                // continuing=true because we're starting new content
                if let Some(current) = state.current_track().cloned() {
                    Self::report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
                }

                // Generate new session ID for this playback context
                state.plex_session_id = Some(Self::generate_plex_session_id());

                // Start track radio - fetch similar tracks, shuffle to avoid album clustering
                state.radio_state.mode = RadioMode::Track;
                state.radio_state.seed_track_key = Some(track_key.clone());
                state.radio_state.seed_title = title.clone();
                state.radio_state.history.clear();
                state.radio_state.fetching = true;
                state.view = View::NowPlaying;
                state.playback_mode = PlaybackMode::Radio;

                // Get the seed track first (to start playing immediately)
                let seed_track = if let Some(track) = state.selected_album_tracks.iter()
                    .find(|t| t.rating_key == track_key)
                    .cloned() {
                    Some(track)
                } else if let Ok(tracks) = client.get_album_tracks(&track_key).await {
                    tracks.into_iter().find(|t| t.rating_key == track_key)
                } else {
                    None
                };

                // Clear queue and start with seed track if found
                state.queue.clear();
                if let Some(track) = seed_track {
                    state.queue.push(track);
                    state.radio_state.history.push(track_key.clone());
                }
                state.queue_index = Some(0);

                // Start playback immediately
                if !state.queue.is_empty() {
                    self.play_current_track(state, client, audio).await;
                }

                // Fetch similar tracks
                match client.get_similar_tracks(&track_key, 50).await {
                    Ok(mut tracks) => {
                        // Shuffle to break up album blocks and add diversity
                        let mut rng = rand::rng();
                        tracks.shuffle(&mut rng);

                        // Filter out seed track and duplicates
                        let new_tracks: Vec<_> = tracks.into_iter()
                            .filter(|t| !state.radio_state.history.contains(&t.rating_key))
                            .take(25)
                            .collect();

                        if !new_tracks.is_empty() {
                            // Add to history to avoid repeats
                            for track in &new_tracks {
                                state.radio_state.history.push(track.rating_key.clone());
                            }

                            // Extend queue with shuffled similar tracks
                            state.queue.extend(new_tracks.clone());

                            state.set_status(format!("{} radio: {} tracks", title, state.queue.len()));
                        } else if state.queue.is_empty() {
                            state.set_error("No similar tracks found".to_string());
                        }
                        state.radio_state.fetching = false;
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to fetch similar tracks: {}", e));
                        state.radio_state.fetching = false;
                    }
                }
            }
            Action::StartAlbumRadio { album_key, title } => {
                use crate::app::state::RadioMode;

                // Report stop for currently playing track before starting radio
                // continuing=true because we're starting new content
                if let Some(current) = state.current_track().cloned() {
                    Self::report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
                }

                // Generate new session ID for this playback context
                state.plex_session_id = Some(Self::generate_plex_session_id());

                // Start album radio - play album then similar albums
                state.radio_state.mode = RadioMode::Album;
                state.radio_state.seed_track_key = Some(album_key.clone());
                state.radio_state.seed_title = title;
                state.radio_state.history.clear();
                state.radio_state.fetching = true;
                state.view = View::NowPlaying;

                // First, load the album's tracks
                match client.get_album_tracks(&album_key).await {
                    Ok(tracks) => {
                        state.queue = tracks;
                        state.queue_index = Some(0);
                        self.play_current_track(state, client, audio).await;
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load album tracks: {}", e));
                    }
                }

                // Then fetch similar albums
                match client.get_similar_albums(&album_key, 10).await {
                    Ok(albums) => {
                        for album in albums {
                            if let Ok(tracks) = client.get_album_tracks(&album.rating_key).await {
                                state.queue.extend(tracks);
                            }
                        }
                        state.radio_state.fetching = false;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to fetch similar albums: {}", e);
                        state.radio_state.fetching = false;
                    }
                }
            }
            Action::StartArtistRadio { artist_key, title } => {
                use crate::app::state::RadioMode;

                // Report stop for currently playing track before starting radio
                // continuing=true because we're starting new content
                if let Some(current) = state.current_track().cloned() {
                    Self::report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
                }

                // Generate new session ID for this playback context
                state.plex_session_id = Some(Self::generate_plex_session_id());

                // Start artist radio - play artist's tracks then similar
                state.radio_state.mode = RadioMode::Artist;
                state.radio_state.seed_track_key = Some(artist_key.clone());
                state.radio_state.seed_title = title;
                state.radio_state.history.clear();
                state.radio_state.fetching = true;
                state.view = View::NowPlaying;

                // Load artist's tracks
                match client.get_artist_all_tracks(&artist_key).await {
                    Ok(tracks) => {
                        state.queue = tracks;
                        state.queue_index = Some(0);
                        self.play_current_track(state, client, audio).await;
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load artist tracks: {}", e));
                    }
                }
                state.radio_state.fetching = false;
            }
            Action::StopRadio => {
                use crate::app::state::RadioMode;

                state.radio_state.mode = RadioMode::Off;
                state.radio_state.seed_track_key = None;
                state.radio_state.seed_title.clear();
                state.radio_state.fetching = false;
                state.radio_state.history.clear();
            }
            Action::JumpToRadioTrack(idx) => {
                // Report stop for current track before jumping
                // continuing=true because we're jumping to another track
                if let Some(track) = state.current_track().cloned() {
                    Self::report_playback_stop_to_plex(&track, state.playback.position_ms, true, state.plex_session_id.clone(), client);
                }

                // Jump to track in radio queue without clearing radio state
                if idx < state.radio.tracks.len() {
                    state.radio.track_index = Some(idx);
                    state.list_state.queue_index = idx;
                    self.play_current_track(state, client, audio).await;
                }
            }
            Action::FetchMoreRadioTracks => {
                use crate::app::state::RadioMode;
                use rand::seq::SliceRandom;

                // Only fetch if in track radio mode and not already fetching
                if state.radio_state.mode == RadioMode::Track && !state.radio_state.fetching {
                    if let Some(ref seed_key) = state.radio_state.seed_track_key.clone() {
                        state.radio_state.fetching = true;

                        match client.get_similar_tracks(&seed_key, 30).await {
                            Ok(mut tracks) => {
                                // Shuffle to maintain diversity
                                let mut rng = rand::rng();
                                tracks.shuffle(&mut rng);

                                let new_tracks: Vec<_> = tracks.into_iter()
                                    .filter(|t| !state.radio_state.history.contains(&t.rating_key))
                                    .take(15)
                                    .collect();

                                for track in &new_tracks {
                                    state.radio_state.history.push(track.rating_key.clone());
                                }

                                state.queue.extend(new_tracks);
                                state.radio_state.fetching = false;
                            }
                            Err(e) => {
                                tracing::warn!("Failed to fetch more radio tracks: {}", e);
                                state.radio_state.fetching = false;
                            }
                        }
                    }
                }
            }
            Action::LoadStations => {
                if let Some(lib_key) = &state.active_library.clone() {
                    state.stations_loading = true;
                    state.station_nav.loading = true;
                    match client.get_stations(lib_key).await {
                        Ok(stations) => {
                            // Initialize with root column
                            state.station_nav.columns.clear();
                            state.station_nav.columns.push(super::state::StationColumn::new(
                                None,
                                "Stations".to_string(),
                                stations.clone(),
                            ));
                            state.station_nav.focused_column = 0;
                            state.station_nav.loading = false;
                            // Keep legacy state in sync
                            state.stations = stations;
                            state.stations_loading = false;
                            state.stations_index = 0;
                        }
                        Err(e) => {
                            state.set_error(format!("Failed to load stations: {}", e));
                            state.stations_loading = false;
                            state.station_nav.loading = false;
                        }
                    }
                }
            }
            Action::LoadGenres => {
                if let Some(lib_key) = &state.active_library.clone() {
                    state.genres_loading = true;
                    state.genre_nav.loading = true;
                    match client.get_genres(lib_key).await {
                        Ok(genres) => {
                            // Initialize genre_nav with the genres list
                            let items = crate::app::state::BrowseItem::from_genres(&genres);
                            state.genre_nav.reset("genres", items);
                            state.genres = genres;
                            state.genres_loading = false;
                            state.genres_index = 0;
                        }
                        Err(e) => {
                            state.set_error(format!("Failed to load genres: {}", e));
                            state.genres_loading = false;
                            state.genre_nav.loading = false;
                        }
                    }
                }
            }
            Action::LoadArtistGenres => {
                if let Some(lib_key) = &state.active_library.clone() {
                    state.artist_genres_loading = true;
                    state.genre_nav.loading = true;
                    match client.get_artist_genres(lib_key).await {
                        Ok(genres) => {
                            let items = crate::app::state::BrowseItem::from_genres(&genres);
                            state.genre_nav.reset("artist genres", items);
                            state.artist_genres = genres;
                            state.artist_genres_loading = false;
                            state.genres_index = 0;
                        }
                        Err(e) => {
                            state.set_error(format!("Failed to load artist genres: {}", e));
                            state.artist_genres_loading = false;
                            state.genre_nav.loading = false;
                        }
                    }
                }
            }
            Action::LoadAlbumGenres => {
                if let Some(lib_key) = &state.active_library.clone() {
                    state.album_genres_loading = true;
                    state.genre_nav.loading = true;
                    match client.get_album_genres(lib_key).await {
                        Ok(genres) => {
                            let items = crate::app::state::BrowseItem::from_genres(&genres);
                            state.genre_nav.reset("album genres", items);
                            state.album_genres = genres;
                            state.album_genres_loading = false;
                            state.genres_index = 0;
                        }
                        Err(e) => {
                            state.set_error(format!("Failed to load album genres: {}", e));
                            state.album_genres_loading = false;
                            state.genre_nav.loading = false;
                        }
                    }
                }
            }
            Action::LoadMoods => {
                if let Some(lib_key) = &state.active_library.clone() {
                    state.moods_loading = true;
                    state.genre_nav.loading = true;
                    match client.get_moods(lib_key).await {
                        Ok(moods) => {
                            let items = crate::app::state::BrowseItem::from_genres(&moods);
                            state.genre_nav.reset("moods", items);
                            state.moods = moods;
                            state.moods_loading = false;
                        }
                        Err(e) => {
                            state.set_error(format!("Failed to load moods: {}", e));
                            state.moods_loading = false;
                            state.genre_nav.loading = false;
                        }
                    }
                }
            }
            Action::LoadStyles => {
                if let Some(lib_key) = &state.active_library.clone() {
                    state.styles_loading = true;
                    state.genre_nav.loading = true;
                    match client.get_styles(lib_key).await {
                        Ok(styles) => {
                            let items = crate::app::state::BrowseItem::from_genres(&styles);
                            state.genre_nav.reset("styles", items);
                            state.styles = styles;
                            state.styles_loading = false;
                        }
                        Err(e) => {
                            state.set_error(format!("Failed to load styles: {}", e));
                            state.styles_loading = false;
                            state.genre_nav.loading = false;
                        }
                    }
                }
            }
            Action::LoadGenreAlbums => {
                // Get the selected genre based on current content type
                let genre = state.current_genre_list().get(state.genres_index).cloned();

                if let Some(genre) = genre {
                    let genre_key = genre.effective_key().to_string();
                    let lib_key = state.active_library.clone();
                    state.right_panel_loading = true;
                    state.genre_albums.clear();
                    state.genre_albums_index = 0;

                    if genre_key.is_empty() {
                        state.set_error("Genre/mood/style has no valid key".to_string());
                        state.right_panel_loading = false;
                    } else if let Some(lib_key) = lib_key {
                        let result = match state.genre_content_type {
                            super::state::GenreContentType::Genres => {
                                client.get_genre_albums(&lib_key, &genre_key).await
                            }
                            super::state::GenreContentType::ArtistGenres => {
                                client.get_artist_genre_albums(&lib_key, &genre_key).await
                            }
                            super::state::GenreContentType::AlbumGenres => {
                                client.get_album_genre_albums(&lib_key, &genre_key).await
                            }
                            super::state::GenreContentType::Moods => {
                                client.get_mood_albums(&lib_key, &genre_key).await
                            }
                            super::state::GenreContentType::Styles => {
                                client.get_style_albums(&lib_key, &genre_key).await
                            }
                            super::state::GenreContentType::Stations => {
                                // Stations don't have albums - this shouldn't be called
                                Ok(Vec::new())
                            }
                        };

                        match result {
                            Ok(mut albums) => {
                                // Sort albums based on current sort mode
                                match state.genre_sort_mode {
                                    super::state::GenreSortMode::Artist => {
                                        albums.sort_by(|a, b| {
                                            let a_artist = a.parent_title.as_deref().unwrap_or("").to_lowercase();
                                            let b_artist = b.parent_title.as_deref().unwrap_or("").to_lowercase();
                                            a_artist.cmp(&b_artist)
                                        });
                                    }
                                    super::state::GenreSortMode::AlbumArtist => {
                                        albums.sort_by(|a, b| {
                                            let a_artist = a.parent_title.as_deref().unwrap_or("").to_lowercase();
                                            let b_artist = b.parent_title.as_deref().unwrap_or("").to_lowercase();
                                            a_artist.cmp(&b_artist)
                                        });
                                    }
                                    super::state::GenreSortMode::AlbumTitle => {
                                        albums.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
                                    }
                                }
                                state.genre_albums = albums;
                            }
                            Err(e) => {
                                state.set_error(format!("Failed to load albums: {}", e));
                            }
                        }
                        state.right_panel_loading = false;
                    } else {
                        state.set_error("No library selected".to_string());
                        state.right_panel_loading = false;
                    }
                }
            }
            Action::LoadArtistGenreAlbums => {
                // Handled by LoadGenreAlbums - just dispatch to it
                Box::pin(self.dispatch(Action::LoadGenreAlbums, state, client, audio)).await?;
            }
            Action::LoadAlbumGenreAlbums => {
                // Handled by LoadGenreAlbums - just dispatch to it
                Box::pin(self.dispatch(Action::LoadGenreAlbums, state, client, audio)).await?;
            }
            Action::LoadMoodAlbums => {
                // Handled by LoadGenreAlbums - just dispatch to it
                Box::pin(self.dispatch(Action::LoadGenreAlbums, state, client, audio)).await?;
            }
            Action::LoadStyleAlbums => {
                // Handled by LoadGenreAlbums - just dispatch to it
                Box::pin(self.dispatch(Action::LoadGenreAlbums, state, client, audio)).await?;
            }
            Action::CycleGenreContentType => {
                state.genre_content_type = state.genre_content_type.next();
                Box::pin(self.dispatch(Action::RefreshGenreView, state, client, audio)).await?;
            }
            Action::RefreshGenreView => {
                state.genres_index = 0;
                state.genre_albums.clear();
                state.genre_albums_index = 0;

                // Reset genre_nav when cycling
                state.genre_nav = super::state::BrowseNavigationState::new();

                // Load the appropriate content based on current type
                match state.genre_content_type {
                    super::state::GenreContentType::Genres => {
                        if state.genres.is_empty() {
                            Box::pin(self.dispatch(Action::LoadGenres, state, client, audio)).await?;
                        } else {
                            // Initialize genre_nav from cached data
                            let items = super::state::BrowseItem::from_genres(&state.genres);
                            state.genre_nav.reset("genres", items);
                        }
                    }
                    super::state::GenreContentType::ArtistGenres => {
                        if state.artist_genres.is_empty() {
                            Box::pin(self.dispatch(Action::LoadArtistGenres, state, client, audio)).await?;
                        } else {
                            let items = super::state::BrowseItem::from_genres(&state.artist_genres);
                            state.genre_nav.reset("artist genres", items);
                        }
                    }
                    super::state::GenreContentType::AlbumGenres => {
                        if state.album_genres.is_empty() {
                            Box::pin(self.dispatch(Action::LoadAlbumGenres, state, client, audio)).await?;
                        } else {
                            let items = super::state::BrowseItem::from_genres(&state.album_genres);
                            state.genre_nav.reset("album genres", items);
                        }
                    }
                    super::state::GenreContentType::Moods => {
                        if state.moods.is_empty() {
                            Box::pin(self.dispatch(Action::LoadMoods, state, client, audio)).await?;
                        } else {
                            let items = super::state::BrowseItem::from_genres(&state.moods);
                            state.genre_nav.reset("moods", items);
                        }
                    }
                    super::state::GenreContentType::Styles => {
                        if state.styles.is_empty() {
                            Box::pin(self.dispatch(Action::LoadStyles, state, client, audio)).await?;
                        } else {
                            let items = super::state::BrowseItem::from_genres(&state.styles);
                            state.genre_nav.reset("styles", items);
                        }
                    }
                    super::state::GenreContentType::Stations => {
                        // Reset station navigation and load stations
                        state.station_nav.columns.clear();
                        state.station_nav.focused_column = 0;
                        Box::pin(self.dispatch(Action::LoadStations, state, client, audio)).await?;
                    }
                }
            }
            Action::CycleGenreSort => {
                state.genre_sort_mode = state.genre_sort_mode.next();
                // Re-sort the current albums
                if !state.genre_albums.is_empty() {
                    match state.genre_sort_mode {
                        // Artist and AlbumArtist both sort by parent_title (the album's artist)
                        super::state::GenreSortMode::Artist |
                        super::state::GenreSortMode::AlbumArtist => {
                            state.genre_albums.sort_by(|a, b| {
                                let a_artist = a.parent_title.as_deref().unwrap_or("").to_lowercase();
                                let b_artist = b.parent_title.as_deref().unwrap_or("").to_lowercase();
                                a_artist.cmp(&b_artist)
                            });
                        }
                        super::state::GenreSortMode::AlbumTitle => {
                            state.genre_albums.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
                        }
                    }
                }
            }
            Action::CycleArtistViewMode => {
                state.artist_view_mode = state.artist_view_mode.next();
                Box::pin(self.dispatch(Action::RefreshArtistView, state, client, audio)).await?;
            }
            Action::RefreshArtistView => {
                // Clear right panel state (drill-down state) but keep preloaded data
                state.selected_artist_albums.clear();
                state.selected_album_tracks.clear();
                state.list_state.artists_index = 0;
                state.list_state.albums_index = 0;
                state.right_panel_mode = RightPanelMode::Empty;
                state.focus = Focus::Left;

                // Only load if data is empty (not already preloaded)
                match state.artist_view_mode {
                    crate::app::state::ArtistViewMode::Artist |
                    crate::app::state::ArtistViewMode::AlbumArtist => {
                        if state.artists.is_empty() {
                            Box::pin(self.dispatch(Action::LoadArtists, state, client, audio)).await?;
                        }
                        // Reset artist_nav with new data
                        let title = state.artist_view_mode.name();
                        let items = super::state::BrowseItem::from_artists(&state.artists);
                        state.artist_nav.reset(title, items);
                    }
                    crate::app::state::ArtistViewMode::Album => {
                        if state.albums.is_empty() {
                            Box::pin(self.dispatch(Action::LoadAlbums, state, client, audio)).await?;
                        }
                        // Reset artist_nav with albums
                        let title = state.artist_view_mode.name();
                        let items = super::state::BrowseItem::from_albums(&state.albums);
                        state.artist_nav.reset(title, items);
                    }
                }
            }
            Action::CycleNowPlayingMode => {
                state.now_playing_mode = state.now_playing_mode.next();
                Box::pin(self.dispatch(Action::RefreshNowPlayingView, state, client, audio)).await?;
            }
            Action::RefreshNowPlayingView => {
                // Load data for the current mode if needed
                match state.now_playing_mode {
                    crate::app::state::NowPlayingMode::Queue => {
                        // Queue mode - nothing to load, already have queue
                    }
                    crate::app::state::NowPlayingMode::RecentlyPlayed => {
                        // Recently Played mode - load from Plex hubs
                        if state.recently_played_albums.is_empty() && !state.recently_played_loading {
                            Box::pin(self.dispatch(Action::LoadRecentlyPlayedAlbums, state, client, audio)).await?;
                        }
                    }
                    crate::app::state::NowPlayingMode::NowPlaying => {
                        // Visualizer mode - load waveform for all visualizer styles
                        Box::pin(self.dispatch(Action::LoadWaveform, state, client, audio)).await?;
                    }
                }
            }
            Action::LoadRecentlyPlayedAlbums => {
                if let Some(library_key) = &state.active_library {
                    state.recently_played_loading = true;
                    // Use the lastViewedAt sort approach - more reliable than hubs
                    match client.get_recently_played_albums(library_key, 50).await {
                        Ok(albums) => {
                            tracing::info!("Loaded {} recently played albums", albums.len());
                            state.recently_played_albums = albums;
                            state.recently_played_loading = false;
                        }
                        Err(e) => {
                            tracing::warn!("Failed to load recently played albums: {}", e);
                            state.recently_played_loading = false;
                        }
                    }
                }
            }
            Action::CyclePlaylistsMode => {
                state.playlists_mode = state.playlists_mode.next();
                Box::pin(self.dispatch(Action::RefreshPlaylistsView, state, client, audio)).await?;
            }
            Action::RefreshPlaylistsView => {
                // Load data for the current mode if needed and reset playlist_nav
                match state.playlists_mode {
                    crate::app::state::PlaylistsMode::All => {
                        // All playlists - reload if empty
                        if state.playlists.is_empty() {
                            Box::pin(self.dispatch(Action::LoadPlaylists, state, client, audio)).await?;
                        } else {
                            // Reset playlist_nav with playlists
                            let items = super::state::BrowseItem::from_playlists(&state.playlists);
                            state.playlist_nav.reset("playlists", items);
                        }
                    }
                    crate::app::state::PlaylistsMode::RecentlyAdded => {
                        // Recently added albums
                        if state.recently_added_albums.is_empty() {
                            Box::pin(self.dispatch(Action::LoadRecentlyAddedAlbums, state, client, audio)).await?;
                        }
                        // Reset playlist_nav with recently added albums
                        let items = super::state::BrowseItem::from_albums(&state.recently_added_albums);
                        state.playlist_nav.reset("recently added", items);
                    }
                }
            }
            Action::LoadRecentlyAddedAlbums => {
                if let Some(library_key) = &state.active_library {
                    state.recently_added_loading = true;
                    match client.get_recently_added_albums(library_key, 50).await {
                        Ok(albums) => {
                            state.recently_added_albums = albums;
                            state.recently_added_loading = false;
                            // Reset playlist_nav with recently added albums
                            let items = super::state::BrowseItem::from_albums(&state.recently_added_albums);
                            state.playlist_nav.reset("recently added", items);
                        }
                        Err(e) => {
                            tracing::warn!("Failed to load recently added albums: {}", e);
                            state.recently_added_loading = false;
                        }
                    }
                }
            }
            Action::EnqueueSelection => {
                // Check if we should enqueue an album instead of tracks
                let album_to_enqueue: Option<(String, String)> = match state.view {
                    View::Browse => {
                        match state.focus {
                            Focus::Left => {
                                // Check if we're in albums mode or have albums selected
                                match state.browse_category {
                                    BrowseCategory::Artists if state.artist_view_mode == super::state::ArtistViewMode::Album => {
                                        // Albums view on left - enqueue selected album
                                        state.albums.get(state.list_state.albums_index)
                                            .map(|a| (a.rating_key.clone(), a.title.clone()))
                                    }
                                    BrowseCategory::Playlists if state.playlists_mode == super::state::PlaylistsMode::RecentlyAdded => {
                                        // Recently added albums - enqueue selected album
                                        state.recently_added_albums.get(state.list_state.playlists_index)
                                            .map(|a| (a.rating_key.clone(), a.title.clone()))
                                    }
                                    _ => None,
                                }
                            }
                            Focus::Right => {
                                // Check if we're viewing albums (not tracks)
                                match state.right_panel_mode {
                                    super::state::RightPanelMode::ArtistAlbums => {
                                        // Artist's albums - enqueue selected album
                                        // Note: right_albums_index 0 is "All tracks", so actual albums start at 1
                                        if state.list_state.right_albums_index > 0 {
                                            let album_idx = state.list_state.right_albums_index - 1;
                                            state.selected_artist_albums.get(album_idx)
                                                .map(|a| (a.rating_key.clone(), a.title.clone()))
                                        } else {
                                            None
                                        }
                                    }
                                    super::state::RightPanelMode::CategoryAlbums => {
                                        // Genre/mood albums - enqueue selected album
                                        state.genre_albums.get(state.genre_albums_index)
                                            .map(|a| (a.rating_key.clone(), a.title.clone()))
                                    }
                                    _ => None,
                                }
                            }
                        }
                    }
                    View::Similar => {
                        match state.similar_mode {
                            super::state::SimilarMode::Albums => {
                                // Similar albums - enqueue selected album
                                state.similar_albums.get(state.list_state.similar_index)
                                    .map(|a| (a.rating_key.clone(), a.title.clone()))
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };

                // If we found an album to enqueue, do that instead
                if let Some((rating_key, title)) = album_to_enqueue {
                    // Use Box::pin for recursive async dispatch
                    return Box::pin(self.dispatch(
                        Action::EnqueueAlbum { rating_key, title },
                        state, client, audio
                    )).await;
                }

                // Otherwise, try to enqueue individual tracks
                let tracks_to_add: Vec<Track> = match state.view {
                    View::Browse => {
                        match state.focus {
                            Focus::Right => {
                                // Enqueue selected track
                                if !state.selected_album_tracks.is_empty() {
                                    vec![state.selected_album_tracks[state.list_state.tracks_index].clone()]
                                } else {
                                    vec![]
                                }
                            }
                            Focus::Left => {
                                // Left panel with no album selected - nothing to enqueue
                                vec![]
                            }
                        }
                    }
                    View::Similar => {
                        match state.similar_mode {
                            super::state::SimilarMode::Tracks => {
                                if let Some(track) = state.similar_tracks.get(state.list_state.similar_index) {
                                    vec![track.clone()]
                                } else {
                                    vec![]
                                }
                            }
                            _ => vec![],
                        }
                    }
                    View::NowPlaying => {
                        // Already in queue view - can't enqueue from here
                        vec![]
                    }
                    _ => vec![],
                };

                if !tracks_to_add.is_empty() {
                    // If radio is playing, convert to queue mode
                    if state.playback_mode == PlaybackMode::Radio {
                        // Convert current radio tracks to queue
                        state.queue = state.radio.tracks.clone();
                        state.queue_index = state.radio.track_index;
                        state.playback_mode = PlaybackMode::Queue;
                        state.radio.clear();
                    }

                    // Add tracks to queue, respecting 500 track limit
                    const MAX_QUEUE_SIZE: usize = 500;
                    for track in tracks_to_add {
                        if state.queue.len() < MAX_QUEUE_SIZE {
                            state.queue.push(track);
                        }
                    }
                    state.set_status(format!("Added to queue ({} tracks)", state.queue.len()));
                }
            }
            Action::PromptSavePlaylist => {
                // Show input dialog for playlist name
                // Use queue if available, otherwise use radio tracks
                let has_tracks = !state.queue.is_empty() || !state.radio.tracks.is_empty();
                if !has_tracks {
                    state.set_error("No tracks to save".to_string());
                } else {
                    let title = if !state.queue.is_empty() {
                        "Save Queue as Playlist"
                    } else {
                        "Save Station as Playlist"
                    };
                    state.input_dialog = Some(super::state::InputDialog {
                        title: title.to_string(),
                        input: String::new(),
                        action_type: super::state::InputDialogAction::SavePlaylist,
                    });
                }
            }
            Action::SaveQueueAsPlaylist(name) => {
                // Create playlist on Plex server
                // Use queue if available, otherwise use radio tracks
                let tracks: Vec<&Track> = if !state.queue.is_empty() {
                    state.queue.iter().collect()
                } else {
                    state.radio.tracks.iter().collect()
                };

                if tracks.is_empty() {
                    state.set_error("No tracks to save".to_string());
                } else if name.trim().is_empty() {
                    state.set_error("Playlist name cannot be empty".to_string());
                } else if let Some(ref library_key) = state.active_library {
                    let track_keys: Vec<String> = tracks.iter()
                        .map(|t| t.rating_key.clone())
                        .collect();
                    let track_count = track_keys.len();
                    let name_clone = name.clone();
                    let library_key_clone = library_key.clone();

                    state.set_status(format!("Saving playlist \"{}\"...", name));

                    match client.create_playlist(&name_clone, &track_keys, &library_key_clone).await {
                        Ok(()) => {
                            state.set_status(format!("Saved \"{}\" ({} tracks)", name_clone, track_count));
                            // Refresh playlists so the new one appears
                            state.playlists_loading = true;
                        }
                        Err(e) => {
                            state.set_error(format!("Failed to save playlist: {}", e));
                        }
                    }
                } else {
                    state.set_error("No library selected".to_string());
                }
            }
            Action::PlayStation(station_key) => {
                // Report stop for currently playing track before starting station
                // continuing=true because we're starting new content
                if let Some(current) = state.current_track().cloned() {
                    Self::report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
                }

                // Generate new session ID for this playback context
                state.plex_session_id = Some(Self::generate_plex_session_id());

                // Find station title from station_nav (Miller columns) or fall back to legacy state.stations
                let station_title = state.station_nav.selected_station()
                    .filter(|s| s.key == station_key)
                    .map(|s| s.title.clone())
                    .or_else(|| {
                        // Search all columns
                        state.station_nav.columns.iter()
                            .flat_map(|col| col.stations.iter())
                            .find(|s| s.key == station_key)
                            .map(|s| s.title.clone())
                    })
                    .or_else(|| {
                        // Fall back to legacy state.stations
                        state.stations.iter()
                            .find(|s| s.key == station_key)
                            .map(|s| s.title.clone())
                    })
                    .unwrap_or_else(|| "Radio".to_string());

                state.set_status(format!("Starting {}...", station_title));

                // IMPORTANT: Station queue creation has a 30-second timeout to prevent freezes.
                // If it takes longer, show an error rather than blocking the UI indefinitely.
                let queue_future = client.create_station_queue(&station_key);
                let timeout_duration = std::time::Duration::from_secs(30);

                match tokio::time::timeout(timeout_duration, queue_future).await {
                    Ok(Ok(tracks)) => {
                        if tracks.is_empty() {
                            state.set_error("Station returned no tracks".to_string());
                        } else {
                            state.playback_mode = PlaybackMode::Radio;
                            state.radio.clear();
                            state.radio.active_station = Some(super::state::ActiveStation {
                                key: station_key.clone(),
                                title: station_title.clone(),
                            });
                            state.radio.tracks = tracks;
                            state.radio.track_index = Some(0);

                            // For Time Travel Radio: initialize chronological continuation state
                            if station_key.contains("timeTravel") {
                                if let Some(lib_key) = &state.active_library {
                                    if let Ok(decades) = client.get_time_travel_decades(lib_key).await {
                                        state.radio.time_travel_decades = decades;
                                        // We started with first 3 decades, next fetch starts at index 3
                                        state.radio.time_travel_index = 3;
                                        tracing::info!("Time Travel Radio: initialized with {} decades, next fetch from index 3",
                                            state.radio.time_travel_decades.len());
                                    }
                                }
                            }

                            state.view = View::NowPlaying;
                            self.play_current_track(state, client, audio).await;
                            state.set_status(format!("Playing {} ({} tracks)", station_title, state.radio.tracks.len()));
                        }
                    }
                    Ok(Err(e)) => {
                        state.set_error(format!("Failed to start station: {}", e));
                    }
                    Err(_) => {
                        // Timeout - this prevents indefinite freezes
                        state.set_error("Station timed out - try a different station".to_string());
                        tracing::warn!("Station queue creation timed out after 30 seconds: {}", station_key);
                    }
                }
            }
            Action::DrillIntoStation(station_key, station_title) => {
                // Drill into a station category (e.g., Mood Radio -> sub-moods)
                state.stations_loading = true;
                state.station_nav.loading = true;
                state.set_status(format!("Loading {}...", station_title));

                match client.get_station_children(&station_key).await {
                    Ok(children) => {
                        if children.is_empty() {
                            // No children - treat as playable station
                            state.stations_loading = false;
                            state.station_nav.loading = false;
                            state.set_status(format!("Starting {}...", station_title));

                            // IMPORTANT: Station queue creation has a 30-second timeout to prevent freezes.
                            let queue_future = client.create_station_queue(&station_key);
                            let timeout_duration = std::time::Duration::from_secs(30);

                            match tokio::time::timeout(timeout_duration, queue_future).await {
                                Ok(Ok(tracks)) => {
                                    if tracks.is_empty() {
                                        state.set_error("Station returned no tracks".to_string());
                                    } else {
                                        state.playback_mode = PlaybackMode::Radio;
                                        state.radio.clear();
                                        state.radio.active_station = Some(super::state::ActiveStation {
                                            key: station_key.clone(),
                                            title: station_title.clone(),
                                        });
                                        state.radio.tracks = tracks;
                                        state.radio.track_index = Some(0);
                                        state.view = View::NowPlaying;
                                        self.play_current_track(state, client, audio).await;
                                        state.set_status(format!("Playing {} ({} tracks)", station_title, state.radio.tracks.len()));
                                    }
                                }
                                Ok(Err(e)) => {
                                    state.set_error(format!("Failed to start station: {}", e));
                                }
                                Err(_) => {
                                    state.set_error("Station timed out - try a different station".to_string());
                                    tracing::warn!("Station queue creation timed out after 30 seconds: {}", station_key);
                                }
                            }
                        } else {
                            // Push new column with children (Miller columns style)
                            state.station_nav.push_column(super::state::StationColumn::new(
                                Some(station_key.clone()),
                                station_title.clone(),
                                children.clone(),
                            ));
                            // Also update the legacy state for compatibility
                            state.stations = children;
                            state.stations_index = 0;
                            state.stations_loading = false;
                            state.station_nav.loading = false;
                            state.clear_error();
                        }
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to load station children: {}", e));
                        state.stations_loading = false;
                        state.station_nav.loading = false;
                    }
                }
            }
            Action::NavigateStationsBack => {
                // Go back in Miller columns (just move focus left - data already in memory)
                if state.station_nav.can_go_left() {
                    state.station_nav.focus_left();
                    // Update legacy state to match focused column
                    if let Some(col) = state.station_nav.focused() {
                        state.stations = col.stations.clone();
                        state.stations_index = col.selected_index;
                    }
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
                state.input_dialog = Some(super::state::InputDialog {
                    title: "Adventure Length (5-100)".to_string(),
                    input: "20".to_string(),
                    action_type: super::state::InputDialogAction::AdventureLength,
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
                                return Ok(());
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
                            state.queue_sort_mode = super::state::QueueSortMode::QueueOrder;
                            state.playback_mode = PlaybackMode::Queue;
                            state.view = View::NowPlaying;

                            // Start playback
                            self.play_current_track(state, client, audio).await;
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
                state.queue_sort_mode = super::state::QueueSortMode::QueueOrder;
                state.playback_mode = PlaybackMode::Queue;
                state.view = View::NowPlaying;
                self.play_current_track(state, client, audio).await;
            }
            Action::AdventureError(msg) => {
                state.adventure.generating = false;
                state.set_error(format!("Adventure failed: {}", msg));
            }

            // Inline list filter actions
            Action::ActivateListFilter => {
                use crate::app::state::GenreContentType;
                state.list_filter_active = true;
                state.list_filter_query.clear();
                state.list_filter_results = None;
                state.list_filter_loading = false;
                state.list_filter_selected = 0;
                // Capture which category and column the filter was activated on
                state.list_filter_category = state.browse_category;
                state.list_filter_column = match state.browse_category {
                    BrowseCategory::Artists => state.artist_nav.focused_column,
                    BrowseCategory::Playlists => state.playlist_nav.focused_column,
                    BrowseCategory::Genres => {
                        if state.genre_content_type == GenreContentType::Stations {
                            state.station_nav.focused_column
                        } else {
                            state.genre_nav.focused_column
                        }
                    }
                    BrowseCategory::Folders => {
                        state.folder_state.as_ref().map(|fs| fs.focused_column).unwrap_or(0)
                    }
                };
            }
            Action::DeactivateListFilter => {
                state.list_filter_active = false;
                state.list_filter_query.clear();
                state.list_filter_results = None;
                state.list_filter_loading = false;
                state.list_filter_selected = 0;
            }
            Action::FilteredListUp => {
                // Navigate up within filtered results and update the target column's selection
                if state.list_filter_selected > 0 {
                    state.list_filter_selected -= 1;
                    // Update the column's selected_index to match
                    if let Some(ref results) = state.list_filter_results {
                        if let Some(&item_idx) = results.matched_indices.get(state.list_filter_selected) {
                            self.update_filter_column_selection(state, item_idx);
                        }
                    }
                }
            }
            Action::FilteredListDown => {
                // Navigate down within filtered results and update the target column's selection
                if let Some(ref results) = state.list_filter_results {
                    if state.list_filter_selected + 1 < results.matched_indices.len() {
                        state.list_filter_selected += 1;
                        if let Some(&item_idx) = results.matched_indices.get(state.list_filter_selected) {
                            self.update_filter_column_selection(state, item_idx);
                        }
                    }
                }
            }
            Action::SelectFilteredItem => {
                // Select the currently highlighted filtered item and drill down
                // Filter stays active and continues to apply to the original column
                if let Some(ref results) = state.list_filter_results.clone() {
                    if let Some(&item_idx) = results.matched_indices.get(state.list_filter_selected) {
                        // Update the column's selected_index to point to this item
                        self.update_filter_column_selection(state, item_idx);

                        // Get and dispatch drill-down actions (filter stays active)
                        let drilldown_actions = self.get_filter_drilldown_actions(state);
                        for action in drilldown_actions {
                            Box::pin(self.dispatch(action, state, client, audio)).await?;
                        }
                    }
                }
            }
            Action::AppendListFilterChar(c) => {
                state.list_filter_query.push(c);
                // Trigger filter execution with debounce
                return self.execute_list_filter(state).await;
            }
            Action::DeleteListFilterChar => {
                state.list_filter_query.pop();
                if state.list_filter_query.is_empty() {
                    state.list_filter_results = None;
                    state.list_filter_loading = false;
                } else {
                    return self.execute_list_filter(state).await;
                }
            }
            Action::ClearListFilter => {
                state.list_filter_query.clear();
                state.list_filter_results = None;
                state.list_filter_loading = false;
            }
            Action::ExecuteListFilter => {
                return self.execute_list_filter(state).await;
            }

            // Search popup actions
            Action::OpenSearchPopup => {
                state.search_popup_active = true;
                // Clear previous search when opening
                state.search_query.clear();
                state.search_results = None;
                state.filter_results = None;
            }
            Action::CloseSearchPopup => {
                state.search_popup_active = false;
            }
            _ => {}
        }
        Ok(())
    }

    const PAGE_SIZE: u32 = 100;

    fn load_artists(&self, state: &mut AppState, client: &PlexClient) {
        if let Some(lib_key) = &state.active_library {
            tracing::info!("Loading all artists from library: {}", lib_key);
            state.artists_loading = true;

            // Spawn background task to load artists
            let event_tx = self.event_tx.clone();
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

    fn load_albums(&self, state: &mut AppState, client: &PlexClient) {
        if let Some(lib_key) = &state.active_library {
            tracing::info!("Loading all albums from library: {}", lib_key);
            state.albums_loading = true;

            // Spawn background task to load albums
            let event_tx = self.event_tx.clone();
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

    fn load_playlists(&self, state: &mut AppState, client: &PlexClient) {
        tracing::info!("Loading playlists");
        state.playlists_loading = true;

        // Spawn background task to load playlists
        let event_tx = self.event_tx.clone();
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

    async fn maybe_load_more(&self, state: &mut AppState, client: &PlexClient) {
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
                        if let Ok((more, _)) = client.get_artists_page(lib_key, offset, Self::PAGE_SIZE).await {
                            state.artists.extend(more);
                            // Re-sort after extending, ignoring "The " prefix
                            state.artists.sort_by(|a, b| sort_key(&a.title).cmp(&sort_key(&b.title)));
                        }
                        state.artists_loading = false;
                    }
                }
                _ => {}
            }
        }
    }

    fn select_filter_result(&self, state: &mut AppState) -> Vec<Action> {
        use super::state::SearchTab;

        let idx = state.list_state.search_item_index;
        let search_tab = state.search_tab;

        // Handle selection based on search tab
        match search_tab {
            SearchTab::Global => {
                // Global search is handled by select_search_result
                return vec![];
            }
            SearchTab::Artists => {
                // Check API results first
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
                // Fall back to local filter
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
                // Album artists filter - navigate to browse and select
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
                    // For album artists, just go to browse - no direct play
                    state.search_query.clear();
                    state.filter_results = None;
                    state.view = View::Browse;
                }
            }
            SearchTab::Albums => {
                // Check API results for album
                if let Some(ref results) = state.filter_results {
                    if let Some(album) = results.albums.get(idx).cloned() {
                        state.search_query.clear();
                        state.filter_results = None;
                        state.view = View::Browse;
                        // Play the album
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
                    // Load and play the playlist tracks
                    return vec![Action::LoadCategoryTracks];
                }
            }
            SearchTab::Tracks => {
                // Check API results for track
                if let Some(ref results) = state.filter_results {
                    if let Some(track) = results.tracks.get(idx).cloned() {
                        state.search_query.clear();
                        state.filter_results = None;
                        state.view = View::Browse;
                        // Play the track - add to queue and start
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

    fn adjust_list_index(&self, state: &mut AppState, delta: isize) {
        match state.view {
            View::Browse => {
                if state.focus == Focus::Left {
                    let len = state.category_len();
                    if len > 0 {
                        let idx = state.category_index() as isize + delta;
                        state.set_category_index(idx.clamp(0, len as isize - 1) as usize);
                    }
                } else {
                    // Right panel - depends on mode
                    match state.right_panel_mode {
                        RightPanelMode::ArtistAlbums => {
                            // +1 for "All Tracks" entry at index 0
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
                    super::state::SimilarMode::Albums => state.similar_albums.len(),
                    super::state::SimilarMode::Tracks => state.similar_tracks.len(),
                };
                if len > 0 {
                    let idx = state.list_state.similar_index as isize + delta;
                    state.list_state.similar_index = idx.clamp(0, len as isize - 1) as usize;
                }
            }
            View::Search => {
                use super::state::SearchTab;

                // Search uses search_item_index for filtered results
                // Check API search results first, fall back to local filtering
                let filtered_len = if let Some(ref results) = state.filter_results {
                    match state.search_tab {
                        SearchTab::Global => 0, // Global search navigates differently
                        SearchTab::Artists => results.artists.len(),
                        SearchTab::AlbumArtists => {
                            // Count unique album artists from loaded albums
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
                            // Playlists use local filtering
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
                    // Fall back to local filtering
                    let query = state.search_query.to_lowercase();
                    match state.search_tab {
                        SearchTab::Global => 0,
                        SearchTab::Artists => state.artists.iter()
                            .filter(|a| query.is_empty() || a.title.to_lowercase().contains(&query))
                            .count(),
                        SearchTab::AlbumArtists => {
                            // Count unique album artists from loaded albums
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

    fn set_list_index(&self, state: &mut AppState, index: isize) {
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
                    // Right panel - depends on mode
                    match state.right_panel_mode {
                        RightPanelMode::ArtistAlbums => {
                            // +1 for "All Tracks" entry at index 0
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
                    super::state::SimilarMode::Albums => state.similar_albums.len(),
                    super::state::SimilarMode::Tracks => state.similar_tracks.len(),
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

    async fn play_track(
        &self,
        track: crate::api::models::Track,
        state: &mut AppState,
        client: &PlexClient,
        audio: &mut AudioPlayer,
    ) {
        // Report stop for currently playing track before switching
        // continuing=true because we're switching to another track
        if let Some(current) = state.current_track().cloned() {
            Self::report_playback_stop_to_plex(&current, state.playback.position_ms, true, state.plex_session_id.clone(), client);
        }

        // Generate new session ID for this playback context
        state.plex_session_id = Some(Self::generate_plex_session_id());

        // BUG FIX: Don't replace queue when playing from views that set up their own queue
        // (NowPlaying has queue, Similar has similar_tracks queued)
        if state.view == View::NowPlaying || state.view == View::Similar {
            // Queue is already set up - just start playback
            // Clear radio state if switching from radio mode
            if state.playback_mode == PlaybackMode::Radio {
                state.radio.clear();
            }
            state.queue_original.clear();
            state.queue_sort_mode = super::state::QueueSortMode::QueueOrder;
            state.playback_mode = PlaybackMode::Queue;
            self.play_current_track(state, client, audio).await;
        } else {
            // Playing from elsewhere (search, browse, etc.) - create new queue
            // Clear radio state if switching from radio mode
            if state.playback_mode == PlaybackMode::Radio {
                state.radio.clear();
            }
            state.queue = vec![track];
            state.queue_index = Some(0);
            state.queue_original.clear();
            state.queue_sort_mode = super::state::QueueSortMode::QueueOrder;
            state.playback_mode = PlaybackMode::Queue;
            self.play_current_track(state, client, audio).await;
        }
    }

    /// Helper to collect tracks from a Miller column for playback.
    /// Uses stored full Track objects if available (for media info), otherwise creates stubs.
    fn collect_tracks_from_column(col: &super::state::BrowseColumn) -> Vec<Track> {
        // If we have full tracks stored, use them (they have media info for direct playback)
        if !col.tracks.is_empty() {
            return col.tracks.clone();
        }

        // Fallback: create Track stubs from BrowseItems (won't have media info)
        // This path should be avoided - direct playback will fail and transcode may 400 on relay servers
        let track_count = col.items.iter().filter(|item| matches!(item, super::state::BrowseItem::Track { .. })).count();
        if track_count > 0 {
            tracing::warn!(
                "collect_tracks_from_column fallback: creating {} track stubs without media info for column '{}'. Direct playback may fail.",
                track_count,
                col.title
            );
        }

        col.items.iter()
            .filter_map(|item| {
                if let super::state::BrowseItem::Track { key, title, duration_ms, track_number } = item {
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

    /// Handle common navigation keys for BrowseNavigationState-based views.
    /// Returns Some(actions) if the key was handled, None if not.
    fn handle_browse_nav_keys(
        key: event::KeyEvent,
        nav: &mut super::state::BrowseNavigationState,
    ) -> Option<Vec<Action>> {
        match key.code {
            // Help
            KeyCode::F(1) | KeyCode::Char('?') => Some(vec![Action::SetView(View::Help)]),

            // Settings
            KeyCode::F(2) => Some(vec![Action::OpenSettings]),

            // Up - move selection up, truncate columns to the right
            KeyCode::Up => {
                nav.move_up();
                nav.truncate_right();
                Some(vec![])
            }

            // Down - move selection down, truncate columns to the right
            KeyCode::Down => {
                nav.move_down();
                nav.truncate_right();
                Some(vec![])
            }

            // Page Up
            KeyCode::PageUp => {
                if let Some(col) = nav.focused_mut() {
                    col.selected_index = col.selected_index.saturating_sub(10);
                }
                nav.truncate_right();
                Some(vec![])
            }

            // Page Down
            KeyCode::PageDown => {
                if let Some(col) = nav.focused_mut() {
                    let max_idx = col.items.len().saturating_sub(1);
                    col.selected_index = (col.selected_index + 10).min(max_idx);
                }
                nav.truncate_right();
                Some(vec![])
            }

            // Home - go to first item
            KeyCode::Home => {
                if let Some(col) = nav.focused_mut() {
                    col.selected_index = 0;
                }
                nav.truncate_right();
                Some(vec![])
            }

            // End - go to last item
            KeyCode::End => {
                if let Some(col) = nav.focused_mut() {
                    col.selected_index = col.items.len().saturating_sub(1);
                }
                nav.truncate_right();
                Some(vec![])
            }

            // Left/Backspace/Esc - focus previous column
            KeyCode::Left | KeyCode::Backspace | KeyCode::Esc => {
                if nav.can_go_left() {
                    nav.focus_left();
                }
                Some(vec![])
            }

            // Tab is NOT handled here - it's handled globally to cycle between views
            // (Artists → Playlists → Genres → Folders → Now Playing)

            // Alphabet jumping in current column
            // Plain letter: jump to first item starting with that letter
            // Shift+letter: jump to first item where first char matches current item's first char
            //               AND second char matches the pressed letter
            KeyCode::Char(c) if c.is_ascii_alphabetic() && !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(col) = nav.focused_mut() {
                    let letter_lower = c.to_ascii_lowercase();
                    let use_second_char = key.modifiers.contains(KeyModifiers::SHIFT);

                    if use_second_char {
                        // Get the first letter of the currently selected item
                        let first_letter = col.items.get(col.selected_index)
                            .and_then(|item| item.title().chars().next())
                            .map(|ch| ch.to_ascii_lowercase());

                        if let Some(first_letter) = first_letter {
                            // Find first item starting with that letter AND having pressed letter as second char
                            if let Some(idx) = col.items.iter().position(|item| {
                                let mut chars = item.title().chars();
                                let first = chars.next().map(|ch| ch.to_ascii_lowercase());
                                let second = chars.next().map(|ch| ch.to_ascii_lowercase());
                                first == Some(first_letter) && second == Some(letter_lower)
                            }) {
                                col.selected_index = idx;
                            }
                        }
                    } else {
                        // Normal first-letter jump
                        if let Some(idx) = col.items.iter().position(|item| {
                            item.title().chars().next()
                                .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                                .unwrap_or(false)
                        }) {
                            col.selected_index = idx;
                        }
                    }
                }
                nav.truncate_right();
                Some(vec![])
            }

            // Not handled by common navigation
            _ => None,
        }
    }

    /// Update the column's selected_index for the filter's target category/column.
    fn update_filter_column_selection(&self, state: &mut AppState, item_idx: usize) {
        use crate::app::state::GenreContentType;
        let category = state.list_filter_category;
        let column = state.list_filter_column;

        match category {
            BrowseCategory::Artists => {
                if let Some(col) = state.artist_nav.columns.get_mut(column) {
                    col.selected_index = item_idx;
                }
            }
            BrowseCategory::Playlists => {
                if let Some(col) = state.playlist_nav.columns.get_mut(column) {
                    col.selected_index = item_idx;
                }
            }
            BrowseCategory::Genres => {
                if state.genre_content_type == GenreContentType::Stations {
                    if let Some(col) = state.station_nav.columns.get_mut(column) {
                        col.selected_index = item_idx;
                    }
                } else {
                    if let Some(col) = state.genre_nav.columns.get_mut(column) {
                        col.selected_index = item_idx;
                    }
                }
            }
            BrowseCategory::Folders => {
                if let Some(ref mut folder_state) = state.folder_state {
                    if let Some(col) = folder_state.columns.get_mut(column) {
                        col.selected_index = item_idx;
                    }
                }
            }
        }
    }

    /// Get the drill-down actions for the selected filtered item.
    /// This simulates pressing Enter on the selected item to drill into it.
    fn get_filter_drilldown_actions(&self, state: &mut AppState) -> Vec<Action> {
        use crate::app::state::GenreContentType;
        let category = state.list_filter_category;

        // Get the appropriate drill-down action based on category
        match category {
            BrowseCategory::Artists => {
                // Use the artist_nav enter key logic
                self.handle_artist_browse_keys(
                    crossterm::event::KeyEvent::new(
                        crossterm::event::KeyCode::Enter,
                        crossterm::event::KeyModifiers::NONE,
                    ),
                    state,
                )
            }
            BrowseCategory::Playlists => {
                self.handle_playlist_browse_keys(
                    crossterm::event::KeyEvent::new(
                        crossterm::event::KeyCode::Enter,
                        crossterm::event::KeyModifiers::NONE,
                    ),
                    state,
                )
            }
            BrowseCategory::Genres => {
                if state.genre_content_type == GenreContentType::Stations {
                    self.handle_station_browse_keys(
                        crossterm::event::KeyEvent::new(
                            crossterm::event::KeyCode::Enter,
                            crossterm::event::KeyModifiers::NONE,
                        ),
                        state,
                    )
                } else {
                    self.handle_genre_browse_keys(
                        crossterm::event::KeyEvent::new(
                            crossterm::event::KeyCode::Enter,
                            crossterm::event::KeyModifiers::NONE,
                        ),
                        state,
                    )
                }
            }
            BrowseCategory::Folders => {
                self.handle_folder_browse_keys(
                    crossterm::event::KeyEvent::new(
                        crossterm::event::KeyCode::Enter,
                        crossterm::event::KeyModifiers::NONE,
                    ),
                    state,
                )
            }
        }
    }

    /// Execute inline list filter with debounce.
    /// Collects filterable items from the filter's target column (captured when filter was activated).
    async fn execute_list_filter(&self, state: &mut AppState) -> Result<()> {
        use crate::app::state::GenreContentType;
        use crate::services::{filter_browse_items, filter_folder_items, filter_stations, DEFAULT_MAX_RESULTS};

        // Increment version for debouncing
        state.list_filter_version = state.list_filter_version.wrapping_add(1);
        let version = state.list_filter_version;
        let query = state.list_filter_query.clone();

        if query.is_empty() {
            state.list_filter_results = None;
            state.list_filter_loading = false;
            return Ok(());
        }

        state.list_filter_loading = true;

        // Use the filter's captured category and column (not the currently focused one)
        let event_tx = self.event_tx.clone();
        let category = state.list_filter_category;
        let column = state.list_filter_column;

        match category {
            BrowseCategory::Artists => {
                // Filter items in the captured column of artist_nav
                if let Some(col) = state.artist_nav.columns.get(column) {
                    let items: Vec<_> = col.items.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                        let results = filter_browse_items(&items, &query, DEFAULT_MAX_RESULTS);
                        let _ = event_tx.send(Event::ListFilterCompleted { version, results }).await;
                    });
                }
            }
            BrowseCategory::Playlists => {
                // Filter items in the captured column of playlist_nav
                if let Some(col) = state.playlist_nav.columns.get(column) {
                    let items: Vec<_> = col.items.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                        let results = filter_browse_items(&items, &query, DEFAULT_MAX_RESULTS);
                        let _ = event_tx.send(Event::ListFilterCompleted { version, results }).await;
                    });
                }
            }
            BrowseCategory::Genres => {
                if state.genre_content_type == GenreContentType::Stations {
                    // Filter stations in the captured column
                    if let Some(col) = state.station_nav.columns.get(column) {
                        let items: Vec<_> = col.stations.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                            let results = filter_stations(&items, &query, DEFAULT_MAX_RESULTS);
                            let _ = event_tx.send(Event::ListFilterCompleted { version, results }).await;
                        });
                    }
                } else {
                    // Filter items in the captured column of genre_nav
                    if let Some(col) = state.genre_nav.columns.get(column) {
                        let items: Vec<_> = col.items.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                            let results = filter_browse_items(&items, &query, DEFAULT_MAX_RESULTS);
                            let _ = event_tx.send(Event::ListFilterCompleted { version, results }).await;
                        });
                    }
                }
            }
            BrowseCategory::Folders => {
                // Filter folder items in the captured column
                if let Some(ref folder_state) = state.folder_state {
                    if let Some(col) = folder_state.columns.get(column) {
                        let items: Vec<_> = col.items.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                            let results = filter_folder_items(&items, &query, DEFAULT_MAX_RESULTS);
                            let _ = event_tx.send(Event::ListFilterCompleted { version, results }).await;
                        });
                    }
                }
            }
        }

        Ok(())
    }

    async fn play_current_track(
        &self,
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
                    && state.now_playing_mode == super::state::NowPlayingMode::NowPlaying
                {
                    // Trigger waveform generation
                    if let Ok(stream_url) = client.get_stream_url(&track) {
                        state.waveform.generating = true;
                        let track_key = track.rating_key.clone();
                        let duration_ms = track.duration_ms();
                        let event_tx = self.event_tx.clone();
                        let token = client.token().map(|s| s.to_string());

                        tokio::spawn(async move {
                            // Check cache first
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

                            // Cache miss - download and generate
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
                    // Only spawn if we have a server URL
                    if let Some(server_url) = client.server_url() {
                        state.artwork_loading = true;
                        let thumb_path_owned = thumb_path.to_string();
                        let event_tx = self.event_tx.clone();
                        let server_url = server_url.to_string();
                        let token = client.token().map(|s| s.to_string());
                        let client_id = client.client_identifier().to_string();

                        // Spawn artwork loading in background task
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
                        // No server URL available - can't load artwork
                        state.artwork_loading = false;
                        state.artwork_data = None;
                    }
                } else {
                    // Artwork is same as before - no need to reload
                    state.artwork_loading = false;
                }
            } else {
                // Track has no artwork
                state.artwork_thumb = None;
                state.artwork_data = None;
                state.artwork_loading = false;
            }

            // Try direct playback first
            match client.get_stream_url(&track) {
                Ok(url) => {
                    tracing::debug!("Direct stream URL: {}", url);
                    match audio.play_url(&url).await {
                        Ok(()) => {
                            state.playback.status = PlayStatus::Playing;
                            // Report playback to Plex (scrobble + timeline) in background
                            self.report_playback_to_plex(&track, state.plex_session_id.clone(), client);
                            // Update local recently played list immediately
                            Self::update_local_recently_played(state, &track);
                            return;
                        }
                        Err(e) => {
                            tracing::warn!("Direct playback failed, trying transcode: {}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!("Cannot get direct stream URL (track has {} media items): {}",
                        track.media.len(), e);
                }
            }

            // Fall back to transcoded stream (Plex converts to MP3)
            if let Ok(url) = client.get_transcoded_stream_url(&track) {
                // Log URL (redact token for security)
                let redacted = url.split("X-Plex-Token=").next().unwrap_or(&url);
                tracing::info!("Using transcoded stream for: {} - URL: {}...", track.title, redacted);
                match audio.play_url(&url).await {
                    Ok(()) => {
                        state.playback.status = PlayStatus::Playing;
                        // Report playback to Plex (scrobble + timeline) in background
                        self.report_playback_to_plex(&track, state.plex_session_id.clone(), client);
                        // Update local recently played list immediately
                        Self::update_local_recently_played(state, &track);
                        return;
                    }
                    Err(e) => {
                        tracing::error!("Transcoded playback also failed: {}", e);
                        state.set_error(format!("Playback failed: {}", e));
                        state.playback.status = PlayStatus::Stopped;
                    }
                }
            } else {
                state.set_error("Failed to get stream URL".to_string());
                state.playback.status = PlayStatus::Stopped;
            }
        }
    }

    /// Report playback to Plex server (scrobble and timeline) in background.
    fn report_playback_to_plex(&self, track: &crate::api::models::Track, session_id: Option<String>, client: &PlexClient) {
        if let Some(server_url) = client.server_url() {
            let rating_key = track.rating_key.clone();
            let track_clone = track.clone();
            let server_url = server_url.to_string();
            let token = client.token().map(|s| s.to_string());
            let client_id = client.client_identifier().to_string();

            tokio::spawn(async move {
                let client = crate::api::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);

                // Report playback start (for "Continue Listening" etc.)
                if let Err(e) = client.report_playback_start(&track_clone, 0, session_id.as_deref()).await {
                    tracing::debug!("Failed to report playback start: {}", e);
                }

                // Scrobble (mark as played for play history)
                if let Err(e) = client.scrobble(&rating_key).await {
                    tracing::debug!("Failed to scrobble: {}", e);
                } else {
                    tracing::debug!("Scrobbled track: {}", rating_key);
                }
            });
        }
    }

    /// Report playback stop to Plex server in background.
    ///
    /// - `continuing=true`: Moving to another track (don't clear from Now Playing)
    /// - `continuing=false`: Truly stopping playback (clear from Now Playing)
    fn report_playback_stop_to_plex(
        track: &crate::api::models::Track,
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

    /// Generate a new Plex session ID for timeline reporting.
    fn generate_plex_session_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// Update local recently played albums list when a track starts playing.
    /// This provides immediate feedback without waiting for server refresh.
    fn update_local_recently_played(state: &mut AppState, track: &crate::api::models::Track) {
        use crate::api::models::Album;

        // Create album stub from track info
        if let Some(album) = Album::from_track(track) {
            let album_key = album.rating_key.clone();

            // Remove any existing entry for this album (we'll add it to front)
            state.recently_played_albums.retain(|a| a.rating_key != album_key);

            // Add to front of list
            state.recently_played_albums.insert(0, album);

            // Keep list capped at 50 items
            state.recently_played_albums.truncate(50);

            // Mark cache as dirty for eventual save
            state.cache_dirty = true;

            tracing::debug!("Updated local recently played: {} items", state.recently_played_albums.len());
        }
    }

    /// Preload data in background for faster access.
    ///
    /// This is a unified preload function that handles all data types. Each preload type
    /// spawns an async task that fetches data from the Plex API and sends the result
    /// back via the event channel.
    ///
    /// # Arguments
    /// * `preload_type` - The type of data to preload
    /// * `lib_key` - The library key (may be unused for some preload types like Playlists)
    /// * `client` - The Plex client to clone connection info from
    fn preload_data(&self, preload_type: PreloadType, lib_key: &str, client: &PlexClient) {
        use crate::services::{FolderColumn, FolderNavigationState, FolderService};

        let Some(server_url) = client.server_url() else { return };
        let server_url = server_url.to_string();
        let token = client.token().map(|s| s.to_string());
        let client_id = client.client_identifier().to_string();
        let lib_key = lib_key.to_string();
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            let client = crate::api::PlexClient::new_with_url(&server_url, token.as_deref(), &client_id);
            let lib_key_ref = lib_key.as_str();

            match preload_type {
                PreloadType::Artists => {
                    tracing::debug!("Preloading artists for library: {}", lib_key);
                    if let Ok(data) = client.get_artists(lib_key_ref).await {
                        tracing::debug!("Artists preloaded: {} items", data.len());
                        let _ = event_tx.send(Event::ArtistsPreloaded(data)).await;
                    }
                }
                PreloadType::Albums => {
                    tracing::debug!("Preloading albums for library: {}", lib_key);
                    if let Ok(data) = client.get_albums(lib_key_ref).await {
                        tracing::debug!("Albums preloaded: {} items", data.len());
                        let _ = event_tx.send(Event::AlbumsPreloaded(data)).await;
                    }
                }
                PreloadType::Playlists => {
                    tracing::debug!("Preloading playlists");
                    if let Ok(data) = client.get_playlists().await {
                        tracing::debug!("Playlists preloaded: {} items", data.len());
                        let _ = event_tx.send(Event::PlaylistsPreloaded(data)).await;
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

    /// Preload all library data in background for a fresh library.
    ///
    /// This initiates background fetches for all data types: artists, albums, playlists,
    /// genres, moods, styles, stations, folders, and recent content. Each fetch runs
    /// concurrently and sends its result via the event channel when complete.
    ///
    /// # Arguments
    /// * `lib_key` - The library key to preload data for
    /// * `lib_title` - The library title for display in folder columns
    /// * `client` - The Plex client to clone connection info from
    fn preload_all_library_data(&self, lib_key: &str, lib_title: &str, client: &PlexClient) {
        self.preload_data(PreloadType::Artists, lib_key, client);
        self.preload_data(PreloadType::Folders { lib_title: lib_title.to_string() }, lib_key, client);
        self.preload_data(PreloadType::Albums, lib_key, client);
        self.preload_data(PreloadType::Genres, lib_key, client);
        self.preload_data(PreloadType::ArtistGenres, lib_key, client);
        self.preload_data(PreloadType::AlbumGenres, lib_key, client);
        self.preload_data(PreloadType::Moods, lib_key, client);
        self.preload_data(PreloadType::Styles, lib_key, client);
        self.preload_data(PreloadType::Stations, lib_key, client);
        self.preload_data(PreloadType::Playlists, lib_key, client);
        self.preload_data(PreloadType::RecentlyAdded, lib_key, client);
        self.preload_data(PreloadType::RecentlyPlayed, lib_key, client);
    }

    /// Fetch more tracks for the current radio station.
    async fn fetch_more_radio_tracks(&self, state: &mut AppState, client: &PlexClient) {
        if state.radio.fetching {
            return; // Already fetching
        }

        // Only fetch if we have an active station
        if let Some(ref station) = state.radio.active_station {
            state.radio.fetching = true;

            // Special handling for Time Travel Radio: continue chronologically
            if station.key.contains("timeTravel") && !state.radio.time_travel_decades.is_empty() {
                if let Some(lib_key) = &state.active_library.clone() {
                    let decades = state.radio.time_travel_decades.clone();
                    let current_index = state.radio.time_travel_index;

                    tracing::info!("Time Travel Radio: fetching more tracks starting from decade index {} ({})",
                        current_index % decades.len(),
                        decades.get(current_index % decades.len()).unwrap_or(&"?".to_string()));

                    match client.fetch_time_travel_tracks_from_index(lib_key, &decades, current_index).await {
                        Ok(new_tracks) => {
                            // Filter out tracks we already have
                            let existing_keys: std::collections::HashSet<_> = state.radio.tracks
                                .iter()
                                .map(|t| t.rating_key.clone())
                                .collect();

                            let unique_tracks: Vec<_> = new_tracks
                                .into_iter()
                                .filter(|t| !existing_keys.contains(&t.rating_key))
                                .collect();

                            tracing::info!("Time Travel Radio: adding {} new unique tracks", unique_tracks.len());
                            state.radio.tracks.extend(unique_tracks);

                            // Advance index for next fetch (wraps around via modulo in fetch function)
                            state.radio.time_travel_index = current_index + 3; // We fetch 3 decades at a time
                        }
                        Err(e) => {
                            tracing::warn!("Time Travel Radio: failed to fetch more tracks: {}", e);
                        }
                    }
                    state.radio.fetching = false;
                    return;
                }
            }

            // Standard station fetch
            tracing::info!("Fetching more tracks for station: {}", station.title);

            match client.create_station_queue(&station.key).await {
                Ok(new_tracks) => {
                    // Filter out tracks we already have
                    let existing_keys: std::collections::HashSet<_> = state.radio.tracks
                        .iter()
                        .map(|t| t.rating_key.clone())
                        .collect();

                    let unique_tracks: Vec<_> = new_tracks
                        .into_iter()
                        .filter(|t| !existing_keys.contains(&t.rating_key))
                        .collect();

                    tracing::info!("Adding {} new unique tracks", unique_tracks.len());
                    state.radio.tracks.extend(unique_tracks);
                    state.radio.fetching = false;
                }
                Err(e) => {
                    tracing::warn!("Failed to fetch more radio tracks: {}", e);
                    state.radio.fetching = false;
                }
            }
        } else {
            state.radio.fetching = false;
        }
    }

    /// Check if we should save the cache and spawn async save if conditions are met.
    /// Conditions: cache is dirty, user is idle (30+ sec), 2+ min since last save, not already saving.
    fn maybe_save_cache_async(&self, state: &mut AppState) {
        // Skip if not dirty or already saving
        if !state.cache_dirty || state.cache_save_in_progress {
            return;
        }

        // Skip if no active library
        let lib_key = match &state.active_library {
            Some(k) => k.clone(),
            None => return,
        };

        // Check idle time (30 seconds)
        let idle_threshold = std::time::Duration::from_secs(30);
        if state.last_input_time.elapsed() < idle_threshold {
            return;
        }

        // Check save interval (2 minutes)
        let save_interval = std::time::Duration::from_secs(120);
        if state.last_cache_save.elapsed() < save_interval {
            return;
        }

        // Mark as in progress and reset dirty flag
        state.cache_save_in_progress = true;
        state.cache_dirty = false;
        state.last_cache_save = std::time::Instant::now();

        // Build cache data from current state
        use crate::cache::CacheData;
        let mut cache_data = CacheData::new(&lib_key);
        cache_data.artists = state.artists.clone();
        cache_data.albums = state.albums.clone();
        cache_data.playlists = state.playlists.clone();
        // Only save folders if they belong to this library
        if let Some(ref folder_state) = state.folder_state {
            if folder_state.library_key == lib_key {
                if let Some(root_col) = folder_state.columns.first() {
                    cache_data.root_folders = root_col.items.clone();
                }
            } else {
                tracing::debug!("Not saving folder_state (periodic) - belongs to different library (expected {}, got {})",
                    lib_key, folder_state.library_key);
            }
        }
        // Save cached subfolder contents
        cache_data.folder_contents = state.folder_contents_cache.clone();
        cache_data.genres = state.genres.clone();
        cache_data.artist_genres = state.artist_genres.clone();
        cache_data.album_genres = state.album_genres.clone();
        cache_data.moods = state.moods.clone();
        cache_data.styles = state.styles.clone();
        cache_data.stations = state.stations.clone();
        cache_data.recently_added_albums = state.recently_added_albums.clone();
        cache_data.recently_played_albums = state.recently_played_albums.clone();

        // Spawn async task to save cache (non-blocking)
        let event_tx = self.event_tx.clone();
        tokio::spawn(async move {
            // Use tokio::fs for async file I/O
            if let Some(cache) = LibraryCache::new() {
                match serde_json::to_string(&cache_data) {
                    Ok(contents) => {
                        let path = cache.cache_path(&lib_key);
                        let temp_path = path.with_extension("json.tmp");

                        // Write atomically using async I/O
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

            // Signal completion to reset in_progress flag
            let _ = event_tx.send(Event::CacheSaved).await;
        });
    }

    /// Refresh the current view's category and return actions.
    fn refresh_current_view(&self, state: &mut AppState) -> Vec<Action> {
        use super::state::{RefreshCategory, ArtistViewMode, PlaylistsMode, GenreContentType};

        // Special handling for Folders: refresh the focused column (subfolder or root)
        if state.view == View::Browse && state.browse_category == BrowseCategory::Folders {
            // Extract subfolder key first to avoid borrow conflict
            let subfolder_key = state.folder_state.as_ref().and_then(|folder_state| {
                if folder_state.focused_column > 0 {
                    folder_state.columns.get(folder_state.focused_column)
                        .and_then(|col| col.key.clone())
                } else {
                    None
                }
            });

            if let Some(folder_key) = subfolder_key {
                // User is focused on a subfolder - refresh that specific subfolder
                state.set_status("Refreshing folder...".to_string());
                return vec![Action::RefreshSubfolder(folder_key)];
            }
            // Fall through to refresh root folders if focused_column == 0 or no subfolder key
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
                    PlaylistsMode::RecentlyAdded => Some(RefreshCategory::RecentlyAdded),
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
    fn is_viewing_category(&self, category: &super::state::RefreshCategory, state: &AppState) -> bool {
        use super::state::{RefreshCategory, ArtistViewMode, PlaylistsMode, GenreContentType};

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
    fn maybe_refresh_very_stale(&self, state: &mut AppState, client: &PlexClient) {
        use super::state::RefreshCategory;

        // Only when idle for 2+ minutes
        if state.last_input_time.elapsed() < Duration::from_secs(120) {
            return;
        }

        // Only one refresh at a time
        if !state.background_refresh_in_progress.is_empty() {
            return;
        }

        // Need active library
        let lib_key = match &state.active_library {
            Some(k) => k.clone(),
            None => return,
        };

        // Check each category for very stale (32+ days)
        for category in RefreshCategory::all() {
            // Skip categories we're currently viewing to avoid disruption
            if self.is_viewing_category(category, state) {
                continue;
            }

            if self.is_category_very_stale(*category, state) {
                tracing::info!("Very stale background refresh: {:?}", category);
                self.spawn_category_refresh(*category, &lib_key, state, client);
                break; // One at a time
            }
        }
    }

    /// Check if a category's data is very stale (32+ days old).
    fn is_category_very_stale(&self, category: super::state::RefreshCategory, state: &AppState) -> bool {
        use super::state::RefreshCategory;

        // Load cache to check timestamp
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

                // Check if the specific category has data and is very stale
                let has_data = match category {
                    RefreshCategory::Artists | RefreshCategory::AlbumArtists => !data.artists.is_empty(),
                    RefreshCategory::Albums => !data.albums.is_empty(),
                    RefreshCategory::Playlists => !data.playlists.is_empty(),
                    RefreshCategory::RecentlyAdded => !data.recently_added_albums.is_empty(),
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
    fn spawn_category_refresh(
        &self,
        category: super::state::RefreshCategory,
        lib_key: &str,
        state: &mut AppState,
        client: &PlexClient,
    ) {
        use super::state::RefreshCategory;

        // Mark as in progress
        state.background_refresh_in_progress.insert(category);

        // Capture current data count for change detection
        let old_count = match category {
            RefreshCategory::Artists | RefreshCategory::AlbumArtists => state.artists.len(),
            RefreshCategory::Albums => state.albums.len(),
            RefreshCategory::Playlists => state.playlists.len(),
            RefreshCategory::RecentlyAdded => state.recently_added_albums.len(),
            RefreshCategory::Genres => state.genres.len(),
            RefreshCategory::ArtistGenres => state.artist_genres.len(),
            RefreshCategory::AlbumGenres => state.album_genres.len(),
            RefreshCategory::Moods => state.moods.len(),
            RefreshCategory::Styles => state.styles.len(),
            RefreshCategory::Stations => state.stations.len(),
            RefreshCategory::Folders => state.folder_state.as_ref().map(|f| f.columns.first().map(|c| c.items.len()).unwrap_or(0)).unwrap_or(0),
        };

        let event_tx = self.event_tx.clone();
        let lib_key = lib_key.to_string();
        let client = client.clone();

        tokio::spawn(async move {
            let changed = match category {
                RefreshCategory::Artists | RefreshCategory::AlbumArtists => {
                    match client.get_artists(&lib_key).await {
                        Ok(artists) => {
                            let new_count = artists.len();
                            let _ = event_tx.send(Event::ArtistsPreloaded(artists)).await;
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
                            let _ = event_tx.send(Event::AlbumsPreloaded(albums)).await;
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
                            let _ = event_tx.send(Event::PlaylistsPreloaded(playlists)).await;
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

            // Signal completion
            let _ = event_tx.send(Event::CacheRefreshCompleted { category, changed }).await;
        });
    }
}

/// Find the first working server connection by testing ALL connections in PARALLEL.
/// Priority: local non-relay > non-relay > relay
///
/// Tests all connections simultaneously and returns the best working one
/// (preferring local over remote over relay). This is much faster than
/// sequential testing when some connections are unreachable.
async fn find_working_connection(
    server: &crate::api::models::PlexServer,
    token: &str,
) -> Option<String> {
    use futures::future::join_all;

    // Build prioritized list of connections with their priority level
    // Priority 0 = local non-relay, 1 = remote non-relay, 2 = relay
    let mut prioritized: Vec<(usize, &str)> = Vec::new();

    for conn in &server.connections {
        let priority = if conn.local && !conn.relay {
            0 // Local, non-relay - highest priority
        } else if !conn.relay {
            1 // Remote, non-relay
        } else {
            2 // Relay - lowest priority
        };
        prioritized.push((priority, conn.uri.as_str()));
    }

    if prioritized.is_empty() {
        tracing::warn!("No connections available for server {}", server.name);
        return None;
    }

    // Test ALL connections in parallel
    let token_str = token.to_string();
    let futures = prioritized.iter().map(|(priority, uri)| {
        let uri = uri.to_string();
        let token = token_str.clone();
        let prio = *priority;
        async move {
            match crate::plex::test_connection(&uri, &token).await {
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

    // Wait for ALL tests to complete
    let results: Vec<Option<(usize, String)>> = join_all(futures).await;

    // Filter successful results and sort by priority (lowest = best)
    let mut successes: Vec<(usize, String)> = results.into_iter().flatten().collect();
    successes.sort_by_key(|(prio, _)| *prio);

    // Return the best (lowest priority number = highest preference)
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
/// Tests each server's connections in priority order.
async fn find_working_connection_from_servers(
    servers: &[crate::api::models::PlexServer],
    token: &str,
) -> Option<String> {
    for server in servers {
        if let Some(url) = find_working_connection(server, token).await {
            return Some(url);
        }
    }
    None
}

impl std::fmt::Display for ConnectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectionState::Disconnected => write!(f, "Disconnected"),
            ConnectionState::Authenticating => write!(f, "Authenticating..."),
            ConnectionState::AuthPending { pin_code, .. } => {
                write!(f, "Enter PIN: {}", pin_code)
            }
            ConnectionState::Connecting => write!(f, "Connecting..."),
            ConnectionState::Connected { username } => write!(f, "Connected as {}", username),
            ConnectionState::Error(e) => write!(f, "Error: {}", e),
        }
    }
}

/// Generate a sort key for a title, ignoring "The " prefix.
fn sort_key(title: &str) -> String {
    let lower = title.to_lowercase();
    if lower.starts_with("the ") {
        lower[4..].to_string()
    } else {
        lower
    }
}
