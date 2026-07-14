//! Main application event loop (musikcube-style).
//!
//! Handles input events, async task coordination, and state updates.

use crate::app::event::*;
use super::{Action, AppState, Event};
use super::state::{ConnectionState, PlayStatus};
use super::handlers;
use crate::plex::PlexClient;
use crate::audio::AudioPlayer;
use crate::config::Config;
use crate::ui;

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, Event as CrosstermEvent, KeyCode, KeyModifiers,
    MouseEventKind,
};
use crossterm::execute;
use ratatui::prelude::*;
use std::io::Stdout;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Joinable owner of crossterm's blocking input API.
///
/// Crossterm permits only one reader. Keeping the handle ensures shutdown
/// never races a second drain loop against a still-running reader.
struct InputReader {
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl InputReader {
    fn spawn(event_tx: mpsc::Sender<Event>, shutdown: Arc<AtomicBool>) -> std::io::Result<Self> {
        let thread_shutdown = shutdown.clone();
        let handle = std::thread::Builder::new()
            .name("textamp-terminal-input".to_string())
            .spawn(move || {
                while !thread_shutdown.load(Ordering::Acquire) {
                    if !event::poll(Duration::from_millis(50)).unwrap_or(false) {
                        continue;
                    }

                    let Ok(raw_event) = event::read() else {
                        continue;
                    };
                    let mapped = match raw_event {
                        CrosstermEvent::Key(key) => Some(Event::Key(key)),
                        CrosstermEvent::Mouse(mouse) => Some(Event::Mouse(mouse)),
                        CrosstermEvent::Resize(width, height) => {
                            Some(Event::Resize(width, height))
                        }
                        _ => None,
                    };

                    let Some(mut pending) = mapped else {
                        continue;
                    };
                    loop {
                        let lossy = matches!(
                            &pending,
                            Event::Resize(_, _)
                                | Event::Mouse(crossterm::event::MouseEvent {
                                    kind: MouseEventKind::Moved | MouseEventKind::Drag(_),
                                    ..
                                })
                        );
                        match event_tx.try_send(pending) {
                            Ok(()) => break,
                            Err(mpsc::error::TrySendError::Closed(_)) => return,
                            Err(mpsc::error::TrySendError::Full(event)) => {
                                if lossy {
                                    break;
                                }
                                if thread_shutdown.load(Ordering::Acquire) {
                                    return;
                                }
                                pending = event;
                                std::thread::sleep(Duration::from_millis(2));
                            }
                        }
                    }
                }
            })?;

        Ok(Self {
            shutdown,
            handle: Some(handle),
        })
    }

    fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            if handle.join().is_err() {
                tracing::warn!("Terminal input thread panicked during shutdown");
            }
        }
    }
}

impl Drop for InputReader {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A timer tick only requires a terminal redraw while it can change something
/// visible. Network/input completions always mark the frame dirty separately.
fn tick_requires_render(state: &AppState) -> bool {
    use super::state::MarqueePhase;

    let library = &state.library;
    let marquee_active = state.marquee.borrow().phase != MarqueePhase::Inactive
        || state.marquee_subtitle.borrow().phase != MarqueePhase::Inactive;
    let library_loading = state.library_loading
        || library.artists_loading
        || library.albums_loading
        || library.playlists_loading
        || library.artist_genres_loading
        || library.album_genres_loading
        || library.moods_loading
        || library.styles_loading
        || library.decades_loading
        || library.years_loading
        || library.collections_loading
        || library.countries_loading
        || library.labels_loading
        || library.formats_loading
        || library.studios_loading
        || library.right_panel_loading
        || state.stations_loading
        || !state.cache_mgmt.preloads_in_progress.is_empty();
    let popup_loading = state.popups.artist_bio.as_ref().is_some_and(|popup| popup.loading)
        || state.popups.radio_launcher.as_ref().is_some_and(|popup| popup.loading)
        || state.popups.adventure_launcher.as_ref().is_some_and(|popup| popup.loading);

    matches!(state.playback.status, PlayStatus::Playing | PlayStatus::Buffering)
        || marquee_active
        || library_loading
        || popup_loading
        || state.search.track_loading
        || state.similar.loading
        || state.related.loading
        || state.artwork.loading
        || !state.artwork.grid_pending.is_empty()
        || state.waveform.generating
        || state.spectrogram.generating
        || state.radio.fetching
        || state.radio_state.fetching
        || state.list_filter.loading
        || state.notifications.toast_show_time.is_some()
        || state.notifications.status_show_time.is_some()
        || state.alt_bar_until.is_some()
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let terminate = signal(SignalKind::terminate());
        match terminate {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

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
        let render_rate = Duration::from_millis(33);
        let mut last_tick = Instant::now();
        let mut last_render = Instant::now()
            .checked_sub(render_rate)
            .unwrap_or_else(Instant::now);
        let mut dirty = true;

        self.shutdown.store(false, Ordering::Release);
        let mut input_reader = InputReader::spawn(self.event_tx.clone(), self.shutdown.clone())?;
        let shutdown_requested = shutdown_signal();
        tokio::pin!(shutdown_requested);

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

        // Clone event_tx for remote polling tasks.
        let tick_event_tx = self.event_tx.clone();

        // Guard: only send one TrackEnded per track. Set when TrackEnded is sent,
        // cleared when a new track starts (playback_started_at changes).
        let mut track_ended_sent = false;
        let mut last_playback_started: Option<Instant> = None;

        // Main loop
        loop {
            // Coalesce bursts of input/network completions and cap terminal
            // writes at ~30 FPS. The app tick marks animated state dirty at
            // 10 FPS; idle screens perform no redraws at all.
            if dirty && last_render.elapsed() >= render_rate {
                terminal.draw(|f| ui::render(f, state))?;
                dirty = false;
                last_render = Instant::now();
            }

            // Wake for whichever is due first: app state maintenance or the
            // next allowed coalesced render.
            let tick_timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or(Duration::ZERO);
            let render_timeout = if dirty {
                render_rate
                    .checked_sub(last_render.elapsed())
                    .unwrap_or(Duration::ZERO)
            } else {
                tick_timeout
            };
            let timeout = tick_timeout.min(render_timeout);

            // Events generated by the event-loop task itself are reduced
            // directly after select. Sending them back into the bounded inbox
            // could deadlock when that inbox is full and this task is its sole
            // receiver.
            let mut direct_events = Vec::with_capacity(2);
            let mut signal_quit = false;

            tokio::select! {
                Some(event) = self.event_rx.recv() => {
                    direct_events.push(event);
                }
                _ = tokio::time::sleep(timeout) => {
                    if last_tick.elapsed() >= tick_rate {
                        // Remote mode: poll for state transitions regardless of play/pause
                        // (the remote player may resume after a seek while we think it's paused)
                        if let crate::app::state::OutputTarget::Remote { ref player_id, ref player_uri, .. } = state.remote.output_target {
                            let should_poll = state.remote.playback.last_poll
                                .map(|t| t.elapsed() >= Duration::from_secs(2))
                                .unwrap_or(true);

                            if should_poll && !state.remote.playback.poll_in_flight {
                                state.remote.playback.last_poll = Some(Instant::now());
                                state.remote.playback.poll_in_flight = true;
                                let target_id = player_id.clone();
                                let p_uri = player_uri.clone();
                                let token = client.shared_token_or_empty();
                                let client_id = client.client_identifier().to_string();
                                let server_url = client.server_url().unwrap_or("").to_string();
                                let machine_id = state.active_server_id.clone()
                                    .or_else(|| state.available_servers.first()
                                        .map(|server| server.client_identifier.clone()))
                                    .unwrap_or_default();
                                let tx = tick_event_tx.clone();
                                let library_generation = state.library_generation;

                                tokio::spawn(async move {
                                    let rc = crate::plex::RemotePlayerClient::new(
                                        token, client_id, target_id.clone(), server_url, machine_id, p_uri,
                                    );
                                    let result = match rc {
                                        Ok(rc) => rc.poll_timeline().await,
                                        Err(error) => Err(error),
                                    };
                                    match result {
                                        Ok(status) => {
                                            let event = RemoteEvent::RemotePlayerStatus {
                                                player_id: target_id,
                                                session_found: status.session_found,
                                                playing: status.playing,
                                                position_ms: status.position_ms,
                                                track_key: status.track_key,
                                                finished: status.finished,
                                            };
                                            let _ = tx.send(Event::for_library(
                                                library_generation,
                                                event,
                                            )).await;
                                        }
                                        Err(e) => {
                                            let event = RemoteEvent::RemotePlayerError {
                                                player_id: target_id,
                                                error: e.to_string(),
                                            };
                                            let _ = tx.send(Event::for_library(
                                                library_generation,
                                                event,
                                            )).await;
                                        }
                                    }
                                });
                            }
                        }

                        // Tick: update playback position
                        if state.playback.status == PlayStatus::Playing {
                            // Reset the one-shot end guard for both local and
                            // remote outputs when a new playback attempt starts.
                            if state.playback.playback_started_at != last_playback_started {
                                track_ended_sent = false;
                                last_playback_started = state.playback.playback_started_at;
                            }

                            if let crate::app::state::OutputTarget::Remote { .. } = state.remote.output_target {
                                if state.remote.playback.baseline_time.is_some() {
                                    let position = state.remote.playback.estimated_position();
                                    state.playback.position_ms = position;
                                    if state.playback.duration_ms > 0
                                        && position >= state.playback.duration_ms
                                        && !track_ended_sent
                                    {
                                        track_ended_sent = true;
                                        direct_events.push(PlaybackEvent::TrackEnded.into());
                                    }
                                }
                            } else {
                                // Local mode: existing position tracking and end detection
                                if let Some(position) = audio.position() {
                                    state.playback.position_ms = position.as_millis() as u64;
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
                                        direct_events.push(PlaybackEvent::TrackEnded.into());
                                    } else {
                                        tracing::warn!(
                                            "Premature track end detected: played {}ms of {}ms expected",
                                            actual_pos_ms, expected_ms
                                        );
                                        direct_events.push(PlaybackEvent::PlaybackError {
                                            playback_id: Some(state.playback.request_id),
                                            message: "Track ended prematurely".to_string(),
                                        }.into());
                                    }
                                }
                            }
                        }

                        for (playback_id, message) in audio.take_failures() {
                            direct_events.push(PlaybackEvent::PlaybackError {
                                playback_id: Some(playback_id),
                                message,
                            }.into());
                        }

                        // Process status/toast expiry, cache saves, etc.
                        direct_events.push(Event::Tick);

                        last_tick = Instant::now();
                    }
                }
                _ = &mut shutdown_requested => {
                    signal_quit = true;
                }
            }

            if signal_quit {
                self.dispatch(
                    super::action::SystemAction::Quit.into(),
                    state,
                    client,
                    audio,
                ).await?;
                dirty = true;
            }

            for event in direct_events {
                let event_needs_render = !matches!(&event, Event::Tick)
                    || tick_requires_render(state);
                let actions = self.handle_event(event, state, client);
                for action in actions {
                    self.dispatch(action, state, client, audio).await?;
                }
                dirty |= event_needs_render;
            }

            if state.should_quit {
                // Stop and join the sole terminal reader before touching the
                // input queue or restoring terminal modes.
                input_reader.stop();

                // Disable mouse capture IMMEDIATELY to prevent more mouse events
                // being queued in the terminal buffer
                let _ = execute!(std::io::stdout(), DisableMouseCapture);

                // Drain any remaining events from the terminal input buffer
                // This prevents escape sequences from being echoed after raw mode is disabled
                let drain_deadline = Instant::now() + Duration::from_millis(100);
                while Instant::now() < drain_deadline
                    && event::poll(Duration::from_millis(10)).unwrap_or(false)
                {
                    let _ = event::read();
                }

                break;
            }
        }

        Ok(())
    }

    /// Handle an incoming event and return actions to dispatch.
    fn handle_event(&self, event: Event, state: &mut AppState, client: &mut PlexClient) -> Vec<Action> {
        match event {
            Event::Key(key) => {
                state.cache_mgmt.last_input_time = std::time::Instant::now();
                // In raw mode Ctrl+C is commonly delivered as a key instead
                // of SIGINT. Treat it as an unconditional clean shutdown,
                // including while a popup owns normal text input.
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
                {
                    return vec![super::action::SystemAction::Quit.into()];
                }
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
            Event::LibraryResult { generation, event } => {
                if generation != state.library_generation {
                    tracing::debug!(
                        generation,
                        active_generation = state.library_generation,
                        "Ignoring stale library-scoped result"
                    );
                    vec![]
                } else {
                    self.handle_event(*event, state, client)
                }
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
            ConnectionState::Degraded { username, message, .. } => {
                write!(f, "Connected as {} (offline: {})", username, message)
            }
            ConnectionState::Error(e) => write!(f, "Error: {}", e),
        }
    }
}
