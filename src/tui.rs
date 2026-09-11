//! Main application event loop (musikcube-style).
//!
//! Handles input events, async task coordination, and state updates.

use crate::app::event::*;
use crate::app::handlers;
use crate::app::state::PlayStatus;
use crate::app::{Action, AppState, Event};
use crate::audio::AudioPlayer;
use crate::config::Config;

use crate::ui;

use anyhow::Result;
use crossterm::event::{self, DisableMouseCapture, Event as CrosstermEvent, MouseEventKind};
use crossterm::execute;
use ratatui::prelude::*;
use std::io::Stdout;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

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
                    let raw_event = match event::poll(Duration::from_millis(50)) {
                        Ok(false) => continue,
                        Ok(true) => event::read(),
                        Err(error) => Err(error),
                    };
                    let mapped = match raw_event {
                        Ok(CrosstermEvent::Key(key)) => Some(Event::Key(key)),
                        Ok(CrosstermEvent::Mouse(mouse)) => Some(Event::Mouse(mouse)),
                        Ok(CrosstermEvent::Resize(width, height)) => {
                            Some(Event::Resize(width, height))
                        }
                        Err(error) => Some(Event::InputError(error.to_string())),
                        _ => None,
                    };

                    let Some(mut pending) = mapped else {
                        continue;
                    };
                    let fatal = matches!(pending, Event::InputError(_));
                    loop {
                        let lossy = matches!(
                            &pending,
                            Event::Mouse(crossterm::event::MouseEvent {
                                kind: MouseEventKind::Moved | MouseEventKind::Drag(_),
                                ..
                            })
                        );
                        match event_tx.try_send(pending) {
                            Ok(()) if fatal => return,
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
    use crate::app::state::MarqueePhase;

    let library = &state.library;
    let marquee_active = [
        state.marquee.phase != MarqueePhase::Inactive,
        state.marquee_subtitle.phase != MarqueePhase::Inactive,
    ]
    .contains(&true);
    let library_loading = [
        state.library_loading,
        library.artists_loading,
        library.albums_loading,
        library.playlists_loading,
        library.artist_genres_loading,
        library.album_genres_loading,
        library.moods_loading,
        library.styles_loading,
        library.decades_loading,
        library.years_loading,
        library.collections_loading,
        library.countries_loading,
        library.labels_loading,
        library.formats_loading,
        library.studios_loading,
        library.right_panel_loading,
        state.stations_loading,
        false,
    ]
    .contains(&true);
    let popup_loading = [
        state
            .popups
            .artist_bio
            .as_ref()
            .is_some_and(|popup| popup.loading),
        state
            .popups
            .adventure_launcher
            .as_ref()
            .is_some_and(|popup| popup.loading),
    ]
    .contains(&true);

    [
        matches!(
            state.playback.status,
            PlayStatus::Playing | PlayStatus::Buffering
        ),
        marquee_active,
        library_loading,
        popup_loading,
        state.similar.loading,
        state.related.loading,
        state.artwork.loading,
        !state.artwork.grid_pending.is_empty(),
        state.waveform.generating,
        state.spectrogram.generating,
        state.station_starting.is_some(),
        state.radio.refill != crate::app::state::RadioRefill::Idle,
        state.list_filter.loading,
        state.notifications.toast_show_time.is_some(),
        state.notifications.status_show_time.is_some(),
        state.alt_bar_until.is_some(),
    ]
    .contains(&true)
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

        audio: &mut AudioPlayer,
    ) -> Result<()> {
        let session = crate::app::tasks::TaskSession::new(self.event_tx.clone());
        session.run(self.run_session(terminal, state, audio)).await
    }

    async fn run_session(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        state: &mut AppState,

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

        // Apply the configured theme before the first frame.
        state.theme = crate::app::theme::ThemeName::from_config(&self.config.ui.theme);
        crate::ui::theme::set_theme(state.theme);

        // Restore cover art view preference
        state.artwork.default_visible = self.config.ui.cover_art_view;

        // Restore artwork mode preference
        state.artwork.mode =
            crate::app::state::ArtworkMode::from_config(&self.config.ui.artwork_mode);

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
        state.external_search.spotify = self.config.ui.enable_spotify_search;
        state.external_search.youtube = self.config.ui.enable_youtube_search;

        state.hidden_sections = self.config.ui.hidden_sections.clone();
        state.hidden_collections = self.config.ui.hidden_collections.clone();

        // Mirror saved per-playlist view toggles (group-by-album,
        // show-artwork) so the playlist-tracks-column event handlers
        // can apply them without a config handle.
        state.playlist_views.clear();
        for (lib, settings) in &self.config.ui.library_view_settings {
            state
                .playlist_views
                .insert(lib.clone(), settings.playlists.clone());
        }

        // Restore normal startup; directory discovery is independent of the
        // active library and never opens an overlay or starts playback.
        crate::app::sources::manager::initialize(state, &self.config);

        if let Some(choice) = crate::app::sources::manager::startup_choice(&self.config) {
            crate::app::dispatch::dispatch_action(
                choice.action(),
                state,
                audio,
                &mut self.config,
                &self.event_tx,
            )
            .await?;
        } else {
            state.view = crate::app::state::View::Browse;
            state.set_status("Add a library in Settings (F2)".into());
        }

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
                let mut feedback = None;
                terminal.draw(|f| {
                    feedback = Some(ui::render(f, state));
                })?;
                if let Some(feedback) = feedback {
                    state.apply_render_feedback(feedback);
                }
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
                        // Tick: update playback position
                        if state.playback.status == PlayStatus::Playing {
                            // Reset the one-shot end guard for a new playback attempt.
                            if state.playback.playback_started_at != last_playback_started {
                                track_ended_sent = false;
                                last_playback_started = state.playback.playback_started_at;
                            }

                            {
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

                        direct_events.extend(audio.take_failures().into_iter().map(Event::from));

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
                self.dispatch(crate::app::action::SystemAction::Quit.into(), state, audio)
                    .await?;
                dirty = true;
            }

            for event in direct_events {
                if let Event::InputError(error) | Event::WorkerFailed(error) = &event {
                    anyhow::bail!("{error}");
                }
                let event_needs_render =
                    !matches!(&event, Event::Tick) || tick_requires_render(state);
                let actions = self.handle_event(event, state);
                for action in actions {
                    self.dispatch(action, state, audio).await?;
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
    fn handle_event(&self, event: Event, state: &mut AppState) -> Vec<Action> {
        match event {
            Event::Key(key) => {
                state.cache_mgmt.last_input_time = std::time::Instant::now();
                // One shared router owns quit, popup capture and view shortcuts.
                handlers::key_input::handle_key(key, state, &self.config)
            }
            Event::Resize(w, h) => {
                state.terminal_width = w;
                state.terminal_height = h;
                state.seek_drag = None;
                state.volume_drag = false;
                vec![]
            }
            other => crate::app::dispatch::handle_core_event(other, state, &self.event_tx),
        }
    }

    /// Dispatch an action to modify state or trigger side effects.
    async fn dispatch(
        &mut self,
        action: Action,
        state: &mut AppState,

        audio: &mut AudioPlayer,
    ) -> Result<()> {
        crate::app::dispatch::dispatch_action(
            action,
            state,
            audio,
            &mut self.config,
            &self.event_tx,
        )
        .await
    }
}
