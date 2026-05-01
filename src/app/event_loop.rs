//! Main application event loop (musikcube-style).
//!
//! Handles input events, async task coordination, and state updates.

use crate::app::event::*;
use super::{Action, AppState, Event};
use super::state::{ConnectionState, PlayStatus};
use super::handlers;
use super::handlers::helpers;
use crate::plex::{PlexAuth, PlexClient};
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

// `PreloadType` now lives in `crate::app::handlers::helpers::preload` so
// both the TUI event loop and the GUI dispatch share it verbatim. See that
// module for the enum definition.

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

        // Restore cover art view preference
        state.artwork.default_visible = self.config.ui.cover_art_view;

        // Restore artwork mode preference
        state.artwork.mode = crate::app::state::ArtworkMode::from_config(&self.config.ui.artwork_mode);

        // Restore Miller column layout preference. Default = Shrinking.
        state.miller_layout = self.config.ui.miller_layout;

        // Restore tall-mode preference.
        state.tall_mode = self.config.ui.tall_mode;

        // Mirror the per-service "external search enabled" toggles
        // from config onto AppState so the palette / context menus /
        // menu bar can gate their entries without threading Config
        // through every render path. dispatch_settings updates these
        // when the user flips a Settings toggle.
        state.external_search.apple_music = self.config.ui.enable_apple_music_search;
        state.external_search.spotify     = self.config.ui.enable_spotify_search;
        state.external_search.youtube     = self.config.ui.enable_youtube_search;

        state.hidden_sections = self.config.ui.hidden_sections.clone();

        // Mirror saved per-playlist view toggles (group-by-album,
        // show-artwork) so the playlist-tracks-column event handlers
        // can apply them without a config handle.
        state.playlist_views.clear();
        for (lib, settings) in &self.config.ui.library_view_settings {
            state.playlist_views.insert(lib.clone(), settings.playlists.clone());
        }

        // Start authentication in background (same logic as GUI).
        state.connection = ConnectionState::Authenticating;
        state.auth_state.step = super::state::AuthStep::Checking;
        crate::app::dispatch::spawn_auth_task(self.event_tx.clone());

        // Clone event_tx for tick handler (avoids borrow conflicts with self in select!)
        let tick_event_tx = self.event_tx.clone();

        // Guard: only send one TrackEnded per track. Set when TrackEnded is sent,
        // cleared when a new track starts (playback_started_at changes).
        let mut track_ended_sent = false;
        let mut last_playback_started: Option<Instant> = None;

        // Main loop
        loop {
            // Update remote playback position from local clock before every render
            // (not just on tick) so the bottom bar progress stays smooth.
            if state.playback.status == PlayStatus::Playing {
                if let crate::app::state::OutputTarget::Remote { .. } = &state.remote.output_target {
                    if let Some(started) = state.playback.playback_started_at {
                        let pos = started.elapsed().as_millis() as u64;
                        state.playback.position_ms = pos;

                        // Detect track end locally — don't rely solely on server poll
                        if state.playback.duration_ms > 0 && pos >= state.playback.duration_ms {
                            let action = Action::Playback(super::action::PlaybackAction::Next);
                            self.dispatch(action, state, client, audio).await?;
                        }
                    }
                }
            }

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
                        // Remote mode: poll for state transitions regardless of play/pause
                        // (the remote player may resume after a seek while we think it's paused)
                        if let crate::app::state::OutputTarget::Remote { ref player_id, ref player_uri, .. } = state.remote.output_target {
                            let should_poll = state.remote.playback.last_poll
                                .map(|t| t.elapsed() >= Duration::from_secs(2))
                                .unwrap_or(true);

                            if should_poll {
                                state.remote.playback.last_poll = Some(Instant::now());
                                let target_id = player_id.clone();
                                let p_uri = player_uri.clone();
                                let token = client.token().map(|s| s.to_string()).unwrap_or_default();
                                let client_id = client.client_identifier().to_string();
                                let server_url = client.server_url().unwrap_or("").to_string();
                                let machine_id = state.available_servers.first()
                                    .map(|s| s.client_identifier.clone()).unwrap_or_default();
                                let tx = tick_event_tx.clone();

                                tokio::spawn(async move {
                                    let rc = crate::plex::RemotePlayerClient::new(
                                        token, client_id, target_id, server_url, machine_id, p_uri,
                                    );
                                    match rc.poll_timeline().await {
                                        Ok(status) => {
                                            let _ = tx.send(RemoteEvent::RemotePlayerStatus {
                                                session_found: status.session_found,
                                                playing: status.playing,
                                                position_ms: status.position_ms,
                                                track_key: status.track_key,
                                                finished: status.finished,
                                            }.into()).await;
                                        }
                                        Err(e) => {
                                            let _ = tx.send(RemoteEvent::RemotePlayerError(e.to_string()).into()).await;
                                        }
                                    }
                                });
                            }
                        }

                        // Tick: update playback position
                        if state.playback.status == PlayStatus::Playing {
                            if let crate::app::state::OutputTarget::Remote { .. } = state.remote.output_target {
                                // Position handled before render (above). Nothing else needed here.
                            } else {
                                // Local mode: existing position tracking and end detection
                                state.playback.position_ms += tick_rate.as_millis() as u64;

                                // Reset track_ended_sent when a new track starts
                                if state.playback.playback_started_at != last_playback_started {
                                    track_ended_sent = false;
                                    last_playback_started = state.playback.playback_started_at;
                                }

                                // Deferred error counter reset: only clear after 5s of
                                // sustained playback, confirming the track is truly playing
                                if state.consecutive_playback_errors > 0 {
                                    if let Some(started) = state.playback.playback_started_at {
                                        if started.elapsed() >= Duration::from_secs(5) {
                                            state.consecutive_playback_errors = 0;
                                        }
                                    }
                                }

                                // Detect track end: audio backend reports sink empty.
                                // Grace period: ignore is_finished() for the first second after
                                // playback starts to avoid spurious TrackEnded during cold-start
                                // (sink initialization, network buffering, decoder warmup).
                                // Only send once per track to prevent duplicate events.
                                let playing_long_enough = state.playback.playback_started_at
                                    .map(|t| t.elapsed() >= Duration::from_secs(1))
                                    .unwrap_or(false);
                                if playing_long_enough && audio.is_finished() && !track_ended_sent {
                                    track_ended_sent = true;

                                    // Duration-based guard: verify the track actually played
                                    // its expected duration before treating as natural end
                                    let actual_pos_ms = audio.position()
                                        .map(|d| d.as_millis() as u64)
                                        .unwrap_or(state.playback.position_ms);
                                    let expected_ms = state.playback.duration_ms;

                                    // Natural completion: no known duration, played >=90%,
                                    // or within 5s of end
                                    let completed_normally = expected_ms == 0
                                        || actual_pos_ms >= expected_ms * 90 / 100
                                        || (expected_ms > 5000 && actual_pos_ms >= expected_ms.saturating_sub(5000));

                                    if completed_normally {
                                        let _ = tick_event_tx.send(PlaybackEvent::TrackEnded.into()).await;
                                    } else {
                                        tracing::warn!(
                                            "Premature track end detected: played {}ms of {}ms expected",
                                            actual_pos_ms, expected_ms
                                        );
                                        let _ = tick_event_tx.send(PlaybackEvent::PlaybackError(
                                            "Track ended prematurely".to_string()
                                        ).into()).await;
                                    }
                                }
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
    #[allow(dead_code)]
    fn _start_auth_task_old(&self, state: &mut AppState) {
        use super::state::AuthStep;
        state.connection = ConnectionState::Authenticating;
        state.auth_state.step = AuthStep::Checking;

        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            // Try stored token (primary authentication method)
            if let Some(stored) = PlexAuth::load_token() {
                tracing::info!("Loaded stored auth: client_identifier={}, server_url={:?}",
                    stored.client_identifier, stored.server_url);

                // Fast path: if we have stored server_url + username, trust them
                // and proceed immediately. This gives instant startup.
                // A background task validates the URL and recovers if it's stale.
                if let (Some(ref server_url), Some(ref username)) = (&stored.server_url, &stored.username) {
                    tracing::info!("Fast path: using stored credentials (instant startup)");
                    let _ = event_tx.send(AuthEvent::AuthSuccess {
                        token: stored.token.clone(),
                        username: username.clone(),
                        server_url: server_url.clone(),
                        servers: vec![],
                        client_identifier: stored.client_identifier.clone(),
                        has_plex_pass: stored.has_plex_pass,
                    }.into()).await;

                    // Background: discover servers and validate stored URL.
                    // If the stored URL is stale, find a working one and update.
                    let event_tx_bg = event_tx.clone();
                    let stored_bg = stored.clone();
                    tokio::spawn(async move {
                        let auth = PlexAuth::from_stored_auth(&stored_bg);
                        let token = &stored_bg.token;
                        let client_id = &stored_bg.client_identifier;

                        // Step 1: Discover servers from plex.tv (for settings + recovery)
                        let servers = match auth.get_servers(token).await {
                            Ok(s) => {
                                let _ = event_tx_bg.send(AuthEvent::ServersDiscovered(s.clone()).into()).await;
                                s
                            }
                            Err(e) => {
                                tracing::warn!("Background server discovery failed: {}", e);
                                return;
                            }
                        };

                        // Step 2: Always try to find the best connection (prefer local over relay).
                        // Even if the stored URL passes a small API test, relay connections
                        // can't handle large audio streams (IncompleteBody errors).
                        // find_working_connection prioritizes: local > remote > relay.
                        let working_url = if let Some(ref stored_id) = stored_bg.server_identifier {
                            if let Some(server) = servers.iter().find(|s| &s.client_identifier == stored_id) {
                                tracing::info!("Testing connections for stored server: {}", server.name);
                                helpers::find_working_connection(server, token, client_id).await
                            } else {
                                helpers::find_working_connection_from_servers(&servers, token, client_id).await
                            }
                        } else {
                            helpers::find_working_connection_from_servers(&servers, token, client_id).await
                        };

                        if let Some(url) = working_url {
                            tracing::info!("Background: found working URL: {}", url);
                            let server_name = servers.iter()
                                .find(|s| s.connections.iter().any(|c| c.uri == url))
                                .map(|s| s.name.clone())
                                .unwrap_or_else(|| "Server".to_string());
                            let _ = event_tx_bg.send(AuthEvent::ServerConnectionSucceeded {
                                server_name,
                                url,
                            }.into()).await;
                        } else {
                            tracing::warn!("Background: no working connections found for any server");
                            let server_name = stored_bg.server_name.clone()
                                .or_else(|| servers.first().map(|s| s.name.clone()))
                                .unwrap_or_else(|| "Server".to_string());
                            let _ = event_tx_bg.send(AuthEvent::ServerConnectionFailed {
                                server_name,
                            }.into()).await;
                        }
                    });
                    return;
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
                            let _ = event_tx.send(AuthEvent::AuthSuccess {
                                token: stored.token,
                                username: user.username,
                                server_url: url,
                                servers,
                                client_identifier: stored.client_identifier,
                                has_plex_pass,
                            }.into()).await;
                            return;
                        }
                    }
                    Err(_) => {}
                }
            }

            // No valid stored token - show login form
            let _ = event_tx.send(AuthEvent::AuthShowLogin.into()).await;
        });
    }

    /// Handle an incoming event and return actions to dispatch.
    fn handle_event(&self, event: Event, state: &mut AppState, client: &mut PlexClient) -> Vec<Action> {
        match event {
            Event::Key(key) => {
                state.cache_mgmt.last_input_time = std::time::Instant::now();
                // Command-palette overlay swallows every key while
                // it's open. Open / close transitions:
                //   - `:` from any normal context  → open palette
                //   - Esc                          → cancel
                //   - Enter on a row               → execute and close
                if state.palette.open {
                    use crate::ui::command_palette::{handle_key as palette_key, run as palette_run, PaletteOutcome};
                    return match palette_key(state, key) {
                        PaletteOutcome::Continue => vec![],
                        PaletteOutcome::Cancel => {
                            state.palette.close();
                            vec![]
                        }
                        PaletteOutcome::Execute(cmd) => {
                            state.palette.close();
                            palette_run(cmd, state)
                        }
                    };
                }
                // `:` (open palette) and `/` (open filter) are now
                // handled inside the shared `key_input::handle_key`
                // so both the TUI and GUI honour them identically.
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
        let follow_ups = match action {
            Action::System(a) => {
                handlers::dispatch_system::dispatch(&self.event_tx, &mut self.config, a, state, client).await?
            }
            Action::Navigation(a) => {
                handlers::dispatch_navigation::dispatch(&self.event_tx, a, state, client).await?
            }
            Action::Data(a) => {
                handlers::dispatch_data::dispatch(&self.event_tx, &self.config, a, state, client).await?
            }
            Action::Miller(a) => {
                handlers::dispatch_miller::dispatch(&self.event_tx, a, state, client, audio).await?
            }
            Action::Playback(a) => {
                handlers::dispatch_playback::dispatch(&self.event_tx, a, state, client, audio).await?
            }
            Action::Queue(a) => {
                handlers::dispatch_queue::dispatch(&self.event_tx, a, state, client, audio).await?
            }
            Action::Search(a) => {
                handlers::dispatch_search::dispatch(&self.event_tx, a, state, client).await?
            }
            Action::Browse(a) => {
                handlers::dispatch_browse::dispatch(&self.event_tx, a, state, client).await?
            }
            Action::Folders(a) => {
                handlers::dispatch_folders::dispatch(&self.event_tx, a, state, client, audio).await?
            }
            Action::Radio(a) => {
                handlers::dispatch_radio::dispatch(&self.event_tx, a, state, client, audio).await?
            }
            Action::Settings(a) => {
                handlers::dispatch_settings::dispatch(&self.event_tx, &mut self.config, a, state, client, audio).await?
            }
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

