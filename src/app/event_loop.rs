//! Main application event loop (musikcube-style).
//!
//! Handles input events, async task coordination, and state updates.

use super::{Action, AppState, Event};
use super::state::{ConnectionState, PlayStatus};
use super::handlers;
use super::handlers::helpers;
use crate::api::{PlexAuth, PlexClient};
use crate::audio::AudioPlayer;
use crate::config::Config;
use crate::ui;

use anyhow::Result;
use crossterm::event::{self, Event as CrosstermEvent, DisableMouseCapture};
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

        // Apply configured theme immediately (before auth screen renders)
        state.theme = crate::ui::theme::ThemeName::from_config(&self.config.ui.theme);
        crate::ui::theme::set_theme(state.theme);

        // Start authentication in background
        self.start_auth_task(state);

        // Clone event_tx for tick handler (avoids borrow conflicts with self in select!)
        let tick_event_tx = self.event_tx.clone();

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

                            // Detect track end: audio backend reports sink empty
                            if audio.is_finished() {
                                let _ = tick_event_tx.send(Event::TrackEnded).await;
                            }
                        }

                        // Process tick event (status/toast expiry, cache saves, etc.)
                        let actions = self.handle_event(Event::Tick, state, client);
                        for action in actions {
                            self.dispatch(action, state, client, audio).await?;
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

                // Fast path: if we have stored server_url + username, test it directly.
                // This skips two round-trips to plex.tv (verify_token + get_servers)
                // and makes startup near-instant for local servers.
                if let (Some(ref server_url), Some(ref username)) = (&stored.server_url, &stored.username) {
                    tracing::info!("Fast path: testing stored server URL directly");
                    match crate::plex::test_connection(server_url, &stored.token, &stored.client_identifier).await {
                        Ok(()) => {
                            tracing::info!("Fast path succeeded: {} reachable", server_url);
                            let _ = event_tx.send(Event::AuthSuccess {
                                token: stored.token.clone(),
                                username: username.clone(),
                                server_url: server_url.clone(),
                                servers: vec![], // Populated by background refresh below
                                client_identifier: stored.client_identifier.clone(),
                                has_plex_pass: stored.has_plex_pass,
                            }).await;

                            // Background: refresh server list (needed for settings)
                            let event_tx_bg = event_tx.clone();
                            let token = stored.token.clone();
                            let auth = PlexAuth::from_stored_auth(&stored);
                            tokio::spawn(async move {
                                if let Ok(servers) = auth.get_servers(&token).await {
                                    let _ = event_tx_bg.send(Event::ServersDiscovered(servers)).await;
                                }
                            });
                            return;
                        }
                        Err(e) => {
                            tracing::info!("Fast path failed ({}), falling back to full auth", e);
                        }
                    }
                }

                // Slow path: verify token with plex.tv, discover servers, test connections
                let auth = PlexAuth::from_stored_auth(&stored);
                match auth.verify_token(&stored.token).await {
                    Ok(user) => {
                        let servers = auth.get_servers(&stored.token).await.unwrap_or_default();

                        let final_server_url = if let Some(stored_id) = &stored.server_identifier {
                            if let Some(server) = servers.iter().find(|s| &s.client_identifier == stored_id) {
                                tracing::info!("Testing connections for stored server: {}", server.name);
                                helpers::find_working_connection(server, &stored.token, &stored.client_identifier).await
                            } else {
                                tracing::warn!("Stored server no longer available, testing all servers");
                                helpers::find_working_connection_from_servers(&servers, &stored.token, &stored.client_identifier).await
                            }
                        } else {
                            helpers::find_working_connection_from_servers(&servers, &stored.token, &stored.client_identifier).await
                        };

                        if let Some(url) = final_server_url {
                            let has_plex_pass = user.has_plex_pass();
                            let _ = event_tx.send(Event::AuthSuccess {
                                token: stored.token,
                                username: user.username,
                                server_url: url,
                                servers,
                                client_identifier: stored.client_identifier,
                                has_plex_pass,
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
                handlers::key_input::handle_key(key, state, &self.config)
            }
            Event::Resize(w, h) => {
                state.terminal_width = w;
                state.terminal_height = h;
                vec![]
            }
            other => handlers::events::handle_app_event(other, state, client, &self.event_tx),
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
        use Action::*;

        let follow_ups = match action {
            // System
            Quit | ShowError(_) | ClearError | SetStatus(_) | ClearStatus
            | RefreshCategory(_) | CycleTheme | LoadArtwork | LoadWaveform => {
                handlers::dispatch_system::dispatch(&self.event_tx, &mut self.config, action, state, client).await?
            }

            // Navigation
            SetView(_) | NextView | PrevView | NextMode | PrevMode
            | SetCategory(_) | ToggleFocus => {
                handlers::dispatch_navigation::dispatch(&self.event_tx, action, state, client).await?
            }

            // Data loading
            LoadInitialData | LoadLibraries | LoadArtists | LoadAlbums | LoadPlaylists
            | LoadArtistAlbums | LoadArtistAllTracks | LoadSelectedAlbumTracks
            | LoadAlbumTracks { .. } | LoadCategoryTracks | GoBackInRightPanel
            | LoadSimilarAlbums { .. } | LoadSimilarTracks { .. }
            | ListUp | ListDown | ListPageUp | ListPageDown | ListTop | ListBottom => {
                handlers::dispatch_data::dispatch(&self.event_tx, &self.config, action, state, client).await?
            }

            // Miller columns
            LoadArtistAlbumsForMiller { .. } | LoadAlbumTracksForMiller { .. }
            | LoadArtistAllTracksForMiller { .. } | PlayTrackFromMiller { .. }
            | LoadGenreAlbumsForMiller { .. } | LoadGenreTracksForMiller { .. }
            | PlayGenreTrackFromMiller { .. } | LoadPlaylistTracksForMiller { .. }
            | LoadAlbumTracksForPlaylistMiller { .. }
            | PlayPlaylistTrackFromMiller { .. } => {
                handlers::dispatch_miller::dispatch(&self.event_tx, action, state, client, audio).await?
            }

            // Playback control
            TogglePlayPause | Pause | Play | Stop | Next | Previous
            | VolumeUp | VolumeDown | ToggleMute | Seek(_) | SeekRelative(_)
            | CycleRepeat | StartPendingPlayback => {
                handlers::dispatch_playback::dispatch(&self.event_tx, action, state, client, audio).await?
            }

            // Queue operations
            PlayTrack(_) | PlayTrackFromCategory(_) | PlayAlbum { .. }
            | EnqueueAlbum { .. } | ClearQueue | RemoveFromQueue(_)
            | JumpToQueueIndex(_) | PlayRecentlyPlayedAlbum(_)
            | EnqueueSelection | PromptSavePlaylist | SaveQueueAsPlaylist(_)
            | ToggleQueueShuffle => {
                handlers::dispatch_queue::dispatch(&self.event_tx, action, state, client, audio).await?
            }

            // Search and filter
            ExecuteSearch | ClearSearch | ExecuteFilterSearch | SelectFilterResult
            | ActivateListFilter | DeactivateListFilter | FilteredListUp | FilteredListDown
            | SelectFilteredItem | AppendListFilterChar(_) | DeleteListFilterChar
            | ClearListFilter | ExecuteListFilter | OpenSearchPopup | CloseSearchPopup
            | OpenLibraryPicker | CloseLibraryPicker => {
                handlers::dispatch_search::dispatch(&self.event_tx, action, state, client).await?
            }

            // Browse modes
            LoadStations | LoadGenres | LoadArtistGenres | LoadAlbumGenres
            | LoadMoods | LoadStyles | LoadGenreAlbums | LoadArtistGenreAlbums
            | LoadAlbumGenreAlbums | LoadMoodAlbums | LoadStyleAlbums
            | CycleGenreContentType | RefreshGenreView
            | CycleArtistViewMode | RefreshArtistView | CycleNowPlayingMode
            | RefreshNowPlayingView | LoadRecentlyPlayedAlbums
            | CyclePlaylistsMode | RefreshPlaylistsView | LoadRecentlyAddedAlbums => {
                handlers::dispatch_browse::dispatch(&self.event_tx, action, state, client).await?
            }

            // Folder navigation
            LoadFolderRoot | NavigateIntoFolder(_) | NavigateUpFolder
            | RefreshSubfolder(_) | PlayFolderTracks => {
                handlers::dispatch_folders::dispatch(&self.event_tx, action, state, client, audio).await?
            }

            // Radio and stations
            StartTrackRadio { .. } | StartAlbumRadio { .. } | StartArtistRadio { .. }
            | StopRadio | JumpToRadioTrack(_) | FetchMoreRadioTracks
            | PlayCurrentRadioTrack
            | PlayStation(_) | DrillIntoStation(_, _) | NavigateStationsBack => {
                handlers::dispatch_radio::dispatch(&self.event_tx, action, state, client, audio).await?
            }

            // Settings, auth, adventure
            Logout | AuthSignIn | AuthSelectServer | OpenSettings | SaveCredentials
            | SettingsSelect | SettingsSignIn | SettingsDiscoverServers
            | SelectServer(_) | SelectLibrary(_) | SaveSettings | ClearCache
            | StartAdventure | SetAdventureStart(_) | SetAdventureEnd(_)
            | SetAdventureLength(_) | CancelAdventure | AdventureComplete(_)
            | AdventureError(_) => {
                handlers::dispatch_settings::dispatch(&self.event_tx, &mut self.config, action, state, client, audio).await?
            }

            _ => vec![],
        };

        // Process follow-up actions
        for follow_up in follow_ups {
            Box::pin(self.dispatch(follow_up, state, client, audio)).await?;
        }

        Ok(())
    }

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
            ConnectionState::Connected { username, .. } => write!(f, "Connected as {}", username),
            ConnectionState::Error(e) => write!(f, "Error: {}", e),
        }
    }
}

